//! Resource loading policy is separate from document styling and layout.
mod base;
mod data_url;
#[cfg(file_system)]
mod file;
#[cfg(feature = "http")]
mod http;
mod mime;
pub(crate) mod sniffing;
pub(crate) use base::FILE_SCHEME;
pub use base::ResourceBase;
pub use data_url::DataUrlError;
#[cfg(file_system)]
pub use file::FileResolver;
#[cfg(feature = "http")]
pub use http::{LoaderOptions, UrlResolver};
use std::{io, path::PathBuf};
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
    #[error("relative reference {0} has no base URL to resolve against")]
    NoBase(String),
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
    #[error("local file is outside the allowed folder: {0}")]
    OutsideRoot(PathBuf),
    /// The render has no folder to read local files in.
    #[error("local file {0} is not read: the render has no local base or file root")]
    NoFileRoot(Url),
    /// Remote content named a local file.
    #[error("local file {0} is not read: the content that names it is not local")]
    LocalFromRemote(Url),
    #[error("not a regular file: {0}")]
    NotRegularFile(PathBuf),
    #[error("the file changed while it was being opened: {0}")]
    Changed(PathBuf),
    #[error("unsupported resource scheme: {0}")]
    UnsupportedScheme(String),
    #[error("invalid data URL: {0}")]
    InvalidDataUrl(#[from] DataUrlError),
    /// A caller-supplied resolver could not supply the resource, for its own reason.
    #[error("resolver could not supply {url}: {reason}")]
    Unavailable { url: Url, reason: Box<str> },
}
impl ResourceError {
    /// Whether the renderer's own limits left the resource out; a resolver's caps, such as
    /// its byte and redirect limits, are the caller's policy and leave it unavailable.
    pub fn loses_content(&self) -> bool {
        match self {
            #[cfg(feature = "http")]
            Self::Http { .. }
            | Self::HttpStatus { .. }
            | Self::RedirectLocation { .. }
            | Self::RedirectLimit { .. }
            | Self::RemoteTooLarge { .. }
            | Self::Timeout(_) => false,
            Self::InvalidBase(_)
            | Self::Url { .. }
            | Self::InvalidFileUrl(_)
            | Self::Denied(_)
            | Self::Read { .. }
            | Self::TooLarge { .. }
            | Self::NoBase(_)
            | Self::OutsideRoot(_)
            | Self::NoFileRoot(_)
            | Self::LocalFromRemote(_)
            | Self::NotRegularFile(_)
            | Self::Changed(_)
            | Self::UnsupportedScheme(_)
            | Self::InvalidDataUrl(_)
            | Self::Unavailable { .. } => false,
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

    /// Whether the render should stop with [`crate::Error::Interrupted`]; asked once the
    /// style sheets have loaded and again after layout. A resolver that only records what it
    /// lacks, for its caller to acquire before rendering again, spares the stages the
    /// repeated render redoes.
    fn interrupts(&self) -> bool {
        false
    }
}

impl<T: ResourceResolver + ?Sized> ResourceResolver for &T {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        (**self).resolve(reference, base)
    }

    fn resolve_with_metadata(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<ResolvedResource, ResourceError> {
        (**self).resolve_with_metadata(reference, base)
    }

    fn interrupts(&self) -> bool {
        (**self).interrupts()
    }
}

/// Decodes `data:` URLs, which need no transport; every other reference is unsupported.
#[derive(Clone, Debug)]
pub struct DataResolver {
    pub max_resource_bytes: usize,
}
impl Default for DataResolver {
    fn default() -> Self {
        Self {
            max_resource_bytes: DEFAULT_MAX_RESOURCE_BYTES,
        }
    }
}
impl DataResolver {
    /// The resource `reference` holds if it is a `data:` URL; its relative references
    /// resolve against `base`.
    pub fn resolve_data_url(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Option<Result<ResolvedResource, ResourceError>> {
        let data = reference.get(data_url::SCHEME.len()..).filter(|_| {
            reference[..data_url::SCHEME.len()].eq_ignore_ascii_case(data_url::SCHEME)
        })?;
        Some(
            data_url::decode(data, self.max_resource_bytes).and_then(|bytes| {
                Ok(ResolvedResource {
                    resource: Resource {
                        bytes,
                        base: base.clone(),
                    },
                    metadata: data_url::metadata(data)?,
                })
            }),
        )
    }
}
impl ResourceResolver for DataResolver {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        self.resolve_with_metadata(reference, base)
            .map(|resolved| resolved.resource)
    }

    fn resolve_with_metadata(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<ResolvedResource, ResourceError> {
        self.resolve_data_url(reference, base).unwrap_or_else(|| {
            Err(ResourceError::UnsupportedScheme(
                base.resolve(reference)?.scheme().into(),
            ))
        })
    }
}

/// The resolver for resources that need no network: local files and `data:` URLs, or only
/// the latter where there is no file system.
#[cfg(file_system)]
pub(crate) type LocalResolver = FileResolver;
#[cfg(not(file_system))]
pub(crate) type LocalResolver = DataResolver;

#[cfg(test)]
mod tests;
