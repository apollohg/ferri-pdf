use crate::{
    FileResolver, ResolvedResource, Resource, ResourceBase, ResourceError, ResourceResolver, Url,
    mime::parse_content_type,
};
use std::{
    io::Read,
    sync::Arc,
    time::{Duration, Instant},
};
use ureq::{
    Agent,
    http::{
        StatusCode,
        header::{CONTENT_TYPE, LOCATION},
    },
    tls::TlsConfig,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_REDIRECT_LIMIT: u32 = 10;
type UrlFilter = dyn Fn(&Url) -> bool + Send + Sync;

#[derive(Clone, Debug)]
pub struct LoaderOptions {
    /// Local loader settings; its byte limit also caps remote transfers and decoded bodies.
    pub files: FileResolver,
    /// Total timeout across the request, redirects and response body.
    pub timeout: Duration,
    pub max_redirects: u32,
}
impl Default for LoaderOptions {
    fn default() -> Self {
        Self {
            files: FileResolver::default(),
            timeout: DEFAULT_TIMEOUT,
            max_redirects: DEFAULT_REDIRECT_LIMIT,
        }
    }
}

/// Local/data/HTTP(S) loading with policy checks before every request and redirect.
pub struct UrlResolver {
    options: LoaderOptions,
    agent: Agent,
    filter: Option<Box<UrlFilter>>,
}
impl Default for UrlResolver {
    fn default() -> Self {
        Self::new(LoaderOptions::default())
    }
}
impl UrlResolver {
    pub fn new(options: LoaderOptions) -> Self {
        let tls = TlsConfig::builder()
            .unversioned_rustls_crypto_provider(Arc::new(rustls_rustcrypto::provider()))
            .build();
        let agent = Agent::config_builder()
            .tls_config(tls)
            .max_redirects(0)
            .http_status_as_error(false)
            .proxy(None)
            .build()
            .into();
        Self {
            options,
            agent,
            filter: None,
        }
    }
    /// Reject a URL before access, including redirect targets and nested resources.
    pub fn with_url_filter(
        mut self,
        filter: impl Fn(&Url) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.filter = Some(Box::new(filter));
        self
    }
    fn check(&self, url: &Url) -> Result<(), ResourceError> {
        if self.filter.as_ref().is_some_and(|filter| !filter(url)) {
            return Err(ResourceError::Denied(url.clone()));
        }
        Ok(())
    }
    fn request_error(&self, url: &Url, source: ureq::Error) -> ResourceError {
        match source {
            ureq::Error::Timeout(_) => ResourceError::Timeout(url.clone()),
            ureq::Error::BodyExceedsLimit(_) => ResourceError::RemoteTooLarge {
                url: url.clone(),
                limit: self.options.files.max_resource_bytes,
            },
            source => ResourceError::Http {
                url: url.clone(),
                source: Box::new(source),
            },
        }
    }
    fn fetch(&self, mut url: Url) -> Result<ResolvedResource, ResourceError> {
        let started = Instant::now();
        let mut redirects = 0;
        loop {
            self.check(&url)?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(ResourceError::UnsupportedScheme(url.scheme().into()));
            }
            let remaining = self
                .options
                .timeout
                .checked_sub(started.elapsed())
                .filter(|v| !v.is_zero())
                .ok_or_else(|| ResourceError::Timeout(url.clone()))?;
            let mut response = self
                .agent
                .get(url.as_str())
                .config()
                .timeout_global(Some(remaining))
                .build()
                .call()
                .map_err(|source| self.request_error(&url, source))?;
            let status = response.status();
            if matches!(
                status,
                StatusCode::MOVED_PERMANENTLY
                    | StatusCode::FOUND
                    | StatusCode::SEE_OTHER
                    | StatusCode::TEMPORARY_REDIRECT
                    | StatusCode::PERMANENT_REDIRECT
            ) {
                if redirects == self.options.max_redirects {
                    return Err(ResourceError::RedirectLimit {
                        url,
                        limit: self.options.max_redirects,
                    });
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| ResourceError::RedirectLocation { url: url.clone() })?;
                url = url.join(location).map_err(|source| ResourceError::Url {
                    reference: location.to_owned(),
                    source,
                })?;
                url.set_fragment(None);
                redirects += 1;
                continue;
            }
            if !status.is_success() {
                return Err(ResourceError::HttpStatus {
                    url,
                    status: status.as_u16(),
                });
            }
            let metadata = response
                .headers()
                .get(CONTENT_TYPE)
                .map(|value| parse_content_type(value.as_bytes()))
                .unwrap_or_default();
            let limit = self.options.files.max_resource_bytes;
            let mut bytes = Vec::new();
            response
                .body_mut()
                .with_config()
                .limit(limit.saturating_add(1) as u64)
                .reader()
                .take(limit.saturating_add(1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|source| self.request_error(&url, source.into()))?;
            if bytes.len() > limit {
                return Err(ResourceError::RemoteTooLarge { url, limit });
            }
            return Ok(ResolvedResource {
                resource: Resource {
                    bytes,
                    base: ResourceBase::Url(url),
                },
                metadata,
            });
        }
    }
}
impl ResourceResolver for UrlResolver {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        self.resolve_with_metadata(reference, base)
            .map(|resolved| resolved.resource)
    }

    fn resolve_with_metadata(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<ResolvedResource, ResourceError> {
        let mut url = base.resolve(reference)?;
        url.set_fragment(None);
        if matches!(url.scheme(), "http" | "https") {
            return self.fetch(url);
        }
        self.check(&url)?;
        self.options.files.resolve_with_metadata(reference, base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::{
        ServerConfig, ServerConnection, StreamOwned,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    const CERT: &[u8] = include_bytes!("../tests/assets/localhost.der");
    const KEY: &[u8] = include_bytes!("../tests/assets/localhost-key.der");
    const ROOT: &[u8] = include_bytes!("../tests/assets/root.der");

    fn server(
        version: &'static rustls::SupportedProtocolVersion,
    ) -> (Url, thread::JoinHandle<std::io::Result<()>>) {
        let config = ServerConfig::builder_with_provider(Arc::new(rustls_rustcrypto::provider()))
            .with_protocol_versions(&[version])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(CERT.to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY.to_vec())),
            )
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!(
            "https://localhost:{}/image",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let worker = thread::spawn(move || {
            let (socket, _) = listener.accept()?;
            socket.set_read_timeout(Some(Duration::from_secs(2)))?;
            socket.set_write_timeout(Some(Duration::from_secs(2)))?;
            let connection = ServerConnection::new(Arc::new(config)).unwrap();
            let mut stream = StreamOwned::new(connection, socket);
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte)?;
                header.push(byte[0]);
                assert!(header.len() < 8192, "bounded test request header");
            }
            stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nsafe",
            )?;
            stream.conn.send_close_notify();
            stream.flush()
        });
        (url, worker)
    }
    fn trusting_test_root() -> UrlResolver {
        let mut resolver = UrlResolver::default();
        let tls = TlsConfig::builder()
            .unversioned_rustls_crypto_provider(Arc::new(rustls_rustcrypto::provider()))
            .root_certs(ureq::tls::RootCerts::new_with_certs(&[
                ureq::tls::Certificate::from_der(ROOT),
            ]))
            .build();
        resolver.agent = Agent::config_builder()
            .tls_config(tls)
            .proxy(None)
            .build()
            .into();
        resolver
    }
    #[test]
    fn pure_rust_tls_verifies_trusted_chains_for_tls12_and_tls13() {
        for version in [&rustls::version::TLS12, &rustls::version::TLS13] {
            let (url, worker) = server(version);
            let result = trusting_test_root().resolve(url.as_str(), &ResourceBase::default());
            worker.join().unwrap().unwrap();
            assert_eq!(result.unwrap().bytes, b"safe", "{version:?}");
        }
    }
    #[test]
    fn https_rejects_unknown_issuers_and_hostname_mismatches() {
        for trust_root in [false, true] {
            let (mut url, worker) = server(&rustls::version::TLS13);
            let resolver = if trust_root {
                url.set_host(Some("127.0.0.1")).unwrap();
                trusting_test_root()
            } else {
                UrlResolver::default()
            };
            let result = resolver.resolve(url.as_str(), &ResourceBase::default());
            assert!(
                worker.join().unwrap().is_err(),
                "untrusted TLS handshake must be rejected"
            );
            let error = result.err().expect("invalid certificate must fail");
            let tls = match &error {
                ResourceError::Http { source, .. } => match source.as_ref() {
                    ureq::Error::Rustls(error) => Some(error),
                    ureq::Error::Io(error) => error
                        .get_ref()
                        .and_then(|error| error.downcast_ref::<rustls::Error>()),
                    _ => None,
                },
                _ => None,
            };
            if trust_root {
                assert!(
                    matches!(
                        tls,
                        Some(rustls::Error::InvalidCertificate(
                            rustls::CertificateError::NotValidForName
                                | rustls::CertificateError::NotValidForNameContext { .. }
                        ))
                    ),
                    "hostname mismatch: {error:?}"
                );
            } else {
                assert!(
                    matches!(
                        tls,
                        Some(rustls::Error::InvalidCertificate(
                            rustls::CertificateError::UnknownIssuer
                        ))
                    ),
                    "unknown issuer: {error:?}"
                );
            }
        }
    }
}
