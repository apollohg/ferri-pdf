//! Resource loading policy is separate from document styling and layout.
mod base;
mod data_url;
#[cfg(feature = "http")]
mod http;
mod mime;
pub use base::ResourceBase;
pub use data_url::DataUrlError;
#[cfg(feature = "http")]
pub use http::{LoaderOptions, UrlResolver};
use std::{
    fs::File,
    io::{self, Read},
    path::PathBuf,
};
pub use url::Url;

const DEFAULT_MAX_RESOURCE_BYTES: usize = 64 * 1024 * 1024;
pub const CSS_MEDIA_TYPE: &str = "text/css";

/// Bytes and the base used to resolve this resource's own relative references.
pub struct Resource {
    pub bytes: Vec<u8>,
    pub base: ResourceBase,
}

/// Transport declarations, absent when the resolver has no metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceMetadata {
    /// Lowercase MIME type essence, without parameters.
    pub media_type: Option<String>,
    /// Declared charset label; decoding and label normalization belong to the consumer.
    pub charset: Option<String>,
}

impl ResourceMetadata {
    pub fn from_content_type(value: &str) -> Self {
        mime::parse_content_type(value.as_bytes())
    }
}

/// Resource bytes and transport metadata obtained by the same resolution.
pub struct ResolvedResource {
    pub resource: Resource,
    pub metadata: ResourceMetadata,
}

#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    #[error("invalid resource base directory: {0}")]
    InvalidBase(PathBuf),
    #[error("invalid resource URL {reference}: {source}")]
    Url {
        reference: String,
        #[source]
        source: url::ParseError,
    },
    #[error("invalid local file URL: {0}")]
    InvalidFileUrl(Url),
    #[error("resource URL rejected by policy: {0}")]
    Denied(Url),
    #[cfg(feature = "http")]
    #[error("HTTP request for {url}: {source}")]
    Http {
        url: Url,
        #[source]
        source: Box<ureq::Error>,
    },
    #[cfg(feature = "http")]
    #[error("HTTP {status} for {url}")]
    HttpStatus { url: Url, status: u16 },
    #[cfg(feature = "http")]
    #[error("redirect from {url} has no valid Location header")]
    RedirectLocation { url: Url },
    #[cfg(feature = "http")]
    #[error("resource exceeds {limit} redirects: {url}")]
    RedirectLimit { url: Url, limit: u32 },
    #[cfg(feature = "http")]
    #[error("remote resource exceeds {limit} bytes: {url}")]
    RemoteTooLarge { url: Url, limit: usize },
    #[cfg(feature = "http")]
    #[error("resource request timed out: {0}")]
    Timeout(Url),
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("resource exceeds {limit} bytes: {path}")]
    TooLarge { path: PathBuf, limit: usize },
    #[error("resource escapes the allowed root: {0}")]
    OutsideRoot(PathBuf),
    #[error("unsupported resource scheme: {0}")]
    UnsupportedScheme(String),
    #[error("invalid data URL: {0}")]
    InvalidDataUrl(#[from] DataUrlError),
}

impl ResourceError {
    /// The resource could not be obtained, as opposed to being malformed or denied by policy.
    pub fn is_unavailable(&self) -> bool {
        match self {
            Self::Read { .. } | Self::UnsupportedScheme(_) => true,
            #[cfg(feature = "http")]
            Self::Http { .. } | Self::HttpStatus { .. } | Self::Timeout(_) => true,
            _ => false,
        }
    }
}

/// Resolve a reference against its containing document or resource.
pub trait ResourceResolver {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError>;

    fn resolve_with_metadata(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<ResolvedResource, ResourceError> {
        self.resolve(reference, base)
            .map(|resource| ResolvedResource {
                resource,
                metadata: ResourceMetadata::default(),
            })
    }
}

/// Loads local files, optionally constrained to a canonical filesystem root.
#[derive(Clone, Debug)]
pub struct FileResolver {
    pub root: Option<PathBuf>,
    pub max_resource_bytes: usize,
}
impl Default for FileResolver {
    fn default() -> Self {
        Self {
            root: None,
            max_resource_bytes: DEFAULT_MAX_RESOURCE_BYTES,
        }
    }
}
impl ResourceResolver for FileResolver {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        self.resolve_with_metadata(reference, base)
            .map(|resolved| resolved.resource)
    }

    fn resolve_with_metadata(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<ResolvedResource, ResourceError> {
        if let Some(data) = reference.get(data_url::SCHEME.len()..)
            && reference[..data_url::SCHEME.len()].eq_ignore_ascii_case(data_url::SCHEME)
        {
            return Ok(ResolvedResource {
                resource: Resource {
                    bytes: data_url::decode(data, self.max_resource_bytes)?,
                    base: base.clone(),
                },
                metadata: data_url::metadata(data)?,
            });
        }
        let url = base.resolve(reference)?;
        if url.scheme() != "file" {
            return Err(ResourceError::UnsupportedScheme(url.scheme().into()));
        }
        let path = url
            .to_file_path()
            .map_err(|_| ResourceError::InvalidFileUrl(url))?;
        let canonical = path.canonicalize().map_err(|source| ResourceError::Read {
            path: path.clone(),
            source,
        })?;
        if let Some(root) = &self.root {
            let root = root.canonicalize().map_err(|source| ResourceError::Read {
                path: root.clone(),
                source,
            })?;
            if !canonical.starts_with(&root) {
                return Err(ResourceError::OutsideRoot(canonical));
            }
        }
        let mut file = File::open(&canonical)
            .map_err(|source| ResourceError::Read {
                path: canonical.clone(),
                source,
            })?
            .take(self.max_resource_bytes.saturating_add(1) as u64);
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|source| ResourceError::Read {
                path: canonical.clone(),
                source,
            })?;
        if bytes.len() > self.max_resource_bytes {
            return Err(ResourceError::TooLarge {
                path: canonical,
                limit: self.max_resource_bytes,
            });
        }
        Ok(ResolvedResource {
            resource: Resource {
                bytes,
                base: Url::from_file_path(&canonical)
                    .map_err(|_| ResourceError::InvalidBase(canonical.clone()))?
                    .into(),
            },
            metadata: ResourceMetadata::default(),
        })
    }
}
