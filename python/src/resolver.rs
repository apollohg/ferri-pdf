//! Resource loading for a render: the built-in loaders, or a Python `fetch` callback for
//! every URL that is not a local file or a data URL.

use std::cell::RefCell;

use ferri_pdf::{
    FileResolver, LoaderOptions, ResolvedResource, Resource as FetchedResource, ResourceBase,
    ResourceError, ResourceMetadata, ResourceResolver, Url, UrlResolver,
};
use pyo3::exceptions::{PyException, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

const FILE_SCHEME: &str = "file";
const DATA_URL_PREFIX: &str = "data:";
const NONE_RETURNED: &str = "fetch returned None";
const ABANDONED: &str = "fetch was not called after an earlier call failed";
const LOCAL_LOCATION: &str = "fetch places a remote resource at a local file";

/// A fetched resource; `url` is where it ended up after redirects, for its relative URLs.
#[pyclass(frozen, module = "ferri_pdf")]
pub(crate) struct Resource {
    data: Py<PyBytes>,
    content_type: Option<String>,
    url: Option<Url>,
}

#[pymethods]
impl Resource {
    #[new]
    #[pyo3(signature = (data, *, content_type = None, url = None))]
    fn new(data: Py<PyBytes>, content_type: Option<String>, url: Option<&str>) -> PyResult<Self> {
        let url = url
            .map(|url| {
                Url::parse(url)
                    .map_err(|error| PyValueError::new_err(format!("url {url:?}: {error}")))
            })
            .transpose()?;
        Ok(Self {
            data,
            content_type,
            url,
        })
    }
}

/// A Python callable that fetches URLs. An `Exception` it raises leaves that resource
/// unavailable, like a failed request; anything else, such as `KeyboardInterrupt` or a
/// result of the wrong type, stops further calls and is raised once rendering returns.
pub(crate) struct FetchCallback {
    fetch: Py<PyAny>,
    fatal: RefCell<Option<PyErr>>,
}

impl FetchCallback {
    fn fetch(&self, url: Url, max_bytes: usize) -> Result<ResolvedResource, ResourceError> {
        if self.fatal.borrow().is_some() {
            return Err(unavailable(url, ABANDONED.to_owned()));
        }
        Python::attach(|py| match self.fetch.call1(py, (url.as_str(),)) {
            Ok(value) => self.fetched(url, value.bind(py), max_bytes),
            Err(error) if error.is_instance_of::<PyException>(py) => {
                Err(unavailable(url, error.to_string()))
            }
            Err(error) => {
                let reason = error.to_string();
                self.fatal.replace(Some(error));
                Err(unavailable(url, reason))
            }
        })
    }

    fn fetched(
        &self,
        url: Url,
        value: &Bound<'_, PyAny>,
        max_bytes: usize,
    ) -> Result<ResolvedResource, ResourceError> {
        if value.is_none() {
            return Err(unavailable(url, NONE_RETURNED.to_owned()));
        }
        let (data, content_type, location) = if let Ok(bytes) = value.cast::<PyBytes>() {
            (bytes.as_bytes(), None, None)
        } else if let Ok(resource) = value.cast::<Resource>() {
            let resource = resource.get();
            (
                resource.data.as_bytes(value.py()),
                resource.content_type.as_deref(),
                resource.url.clone(),
            )
        } else {
            let error = PyTypeError::new_err(format!(
                "fetch must return bytes, Resource or None, not {}",
                value
                    .get_type()
                    .name()
                    .map_or_else(|_| "an unnamed type".to_owned(), |name| name.to_string())
            ));
            let reason = error.to_string();
            self.fatal.replace(Some(error));
            return Err(unavailable(url, reason));
        };
        // As a redirect to a local file would, it lets remote content name local files.
        if location
            .as_ref()
            .is_some_and(|location| location.scheme() == FILE_SCHEME)
        {
            return Err(unavailable(url, LOCAL_LOCATION.to_owned()));
        }
        if data.len() > max_bytes {
            return Err(ResourceError::RemoteTooLarge {
                url,
                limit: max_bytes,
            });
        }
        Ok(ResolvedResource {
            resource: FetchedResource {
                bytes: data.to_vec(),
                base: ResourceBase::Url(location.unwrap_or(url)),
            },
            metadata: content_type
                .map(ResourceMetadata::from_content_type)
                .unwrap_or_default(),
        })
    }
}

/// Reads local files and data URLs itself and sends every other URL to the callback.
pub(crate) struct FetchResolver<'a> {
    files: FileResolver,
    callback: &'a FetchCallback,
}

fn unavailable(url: Url, reason: String) -> ResourceError {
    ResourceError::Unavailable {
        url,
        reason: reason.into(),
    }
}

impl ResourceResolver for FetchResolver<'_> {
    fn resolve(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<FetchedResource, ResourceError> {
        self.resolve_with_metadata(reference, base)
            .map(|resolved| resolved.resource)
    }

    fn resolve_with_metadata(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<ResolvedResource, ResourceError> {
        if reference
            .get(..DATA_URL_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(DATA_URL_PREFIX))
        {
            return self.files.resolve_with_metadata(reference, base);
        }
        let url = base.resolve(reference)?;
        if url.scheme() == FILE_SCHEME {
            return self.files.resolve_with_metadata(reference, base);
        }
        self.callback.fetch(url, self.files.max_resource_bytes)
    }
}

/// What loads the resources of a render that are not local files or data URLs.
pub(crate) enum Loader {
    Nothing,
    Http,
    Fetch(FetchCallback),
}

impl Loader {
    /// `fetch`, when given, replaces the built-in HTTP client whatever `allow_http` says.
    pub(crate) fn new(allow_http: bool, fetch: Option<Py<PyAny>>) -> Self {
        match fetch {
            Some(fetch) => Self::Fetch(FetchCallback {
                fetch,
                fatal: RefCell::new(None),
            }),
            None if allow_http => Self::Http,
            None => Self::Nothing,
        }
    }

    /// The resolver for a document whose local files `files` reads.
    pub(crate) fn resolver(&self, files: FileResolver) -> Resolver<'_> {
        match self {
            Self::Nothing => Resolver::Files(files),
            Self::Http => Resolver::Urls(UrlResolver::new(LoaderOptions {
                files,
                ..LoaderOptions::default()
            })),
            Self::Fetch(callback) => Resolver::Fetch(FetchResolver { files, callback }),
        }
    }

    /// The error that stopped a `fetch` callback, which the render must raise instead.
    pub(crate) fn into_fatal(self) -> Option<PyErr> {
        match self {
            Self::Fetch(callback) => callback.fatal.into_inner(),
            Self::Nothing | Self::Http => None,
        }
    }
}

pub(crate) enum Resolver<'a> {
    Files(FileResolver),
    Urls(UrlResolver),
    Fetch(FetchResolver<'a>),
}

impl Resolver<'_> {
    fn loader(&self) -> &dyn ResourceResolver {
        match self {
            Self::Files(resolver) => resolver,
            Self::Urls(resolver) => resolver,
            Self::Fetch(resolver) => resolver,
        }
    }
}

impl ResourceResolver for Resolver<'_> {
    fn resolve(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<FetchedResource, ResourceError> {
        self.loader().resolve(reference, base)
    }

    fn resolve_with_metadata(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<ResolvedResource, ResourceError> {
        self.loader().resolve_with_metadata(reference, base)
    }
}
