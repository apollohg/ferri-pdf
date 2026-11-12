use crate::resources::ResourceError;
#[cfg(file_system)]
use std::path::{Path, PathBuf};
use url::Url;

#[cfg(file_system)]
const CURRENT_DIRECTORY: &str = ".";
/// The default base where there is no file system; relative references do not resolve
/// against it.
#[cfg(not(file_system))]
const BLANK_URL: &str = "about:blank";

/// A directory or the full URL of the containing document/resource.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResourceBase {
    /// A local directory.
    #[cfg(file_system)]
    Directory(PathBuf),
    /// Where there is no file system, the URL that relative references resolve against;
    /// like a local directory, it names no document.
    #[cfg(not(file_system))]
    Directory(Url),
    Url(Url),
}
#[cfg(file_system)]
impl Default for ResourceBase {
    fn default() -> Self {
        Self::Directory(PathBuf::from(CURRENT_DIRECTORY))
    }
}
#[cfg(not(file_system))]
impl Default for ResourceBase {
    fn default() -> Self {
        Self::Url(Url::parse(BLANK_URL).expect("a constant URL is valid"))
    }
}
/// String conversions are local directories; use `Url` for remote bases.
#[cfg(file_system)]
impl From<&str> for ResourceBase {
    fn from(path: &str) -> Self {
        Self::Directory(PathBuf::from(path))
    }
}
#[cfg(file_system)]
impl From<PathBuf> for ResourceBase {
    fn from(path: PathBuf) -> Self {
        Self::Directory(path)
    }
}
#[cfg(file_system)]
impl From<&Path> for ResourceBase {
    fn from(path: &Path) -> Self {
        Self::Directory(path.to_owned())
    }
}
impl From<Url> for ResourceBase {
    fn from(url: Url) -> Self {
        Self::Url(url)
    }
}
impl ResourceBase {
    /// The URL of the containing document or resource; `None` for a directory.
    pub fn url(&self) -> Option<&Url> {
        match self {
            Self::Url(url) => Some(url),
            Self::Directory(_) => None,
        }
    }

    pub fn resolve(&self, reference: &str) -> Result<Url, ResourceError> {
        if let Ok(url) = Url::parse(reference) {
            return Ok(url);
        }
        let base = match self {
            Self::Url(url) => url.clone(),
            #[cfg(not(file_system))]
            Self::Directory(url) => url.clone(),
            #[cfg(file_system)]
            Self::Directory(directory) => {
                // The empty parent of a bare file name is the current directory.
                let directory = if directory.as_os_str().is_empty() {
                    Path::new(CURRENT_DIRECTORY)
                } else {
                    directory
                };
                let path =
                    std::path::absolute(directory).map_err(|source| ResourceError::Read {
                        path: directory.to_path_buf(),
                        source,
                    })?;
                Url::from_directory_path(path)
                    .map_err(|()| ResourceError::InvalidBase(directory.to_path_buf()))?
            }
        };
        base.join(reference).map_err(|source| ResourceError::Url {
            reference: reference.to_owned(),
            source,
        })
    }
}
