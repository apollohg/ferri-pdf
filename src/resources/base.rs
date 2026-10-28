use crate::resources::ResourceError;
use std::path::{Path, PathBuf};
use url::Url;

const CURRENT_DIRECTORY: &str = ".";

/// A local directory or the full URL of the containing document/resource.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResourceBase {
    Directory(PathBuf),
    Url(Url),
}
impl Default for ResourceBase {
    fn default() -> Self {
        Self::Directory(PathBuf::from(CURRENT_DIRECTORY))
    }
}
/// String conversions are local directories; use `Url` for remote bases.
impl From<&str> for ResourceBase {
    fn from(path: &str) -> Self {
        Self::Directory(PathBuf::from(path))
    }
}
impl From<PathBuf> for ResourceBase {
    fn from(path: PathBuf) -> Self {
        Self::Directory(path)
    }
}
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
    pub fn resolve(&self, reference: &str) -> Result<Url, ResourceError> {
        if let Ok(url) = Url::parse(reference) {
            return Ok(url);
        }
        let base = match self {
            Self::Url(url) => url.clone(),
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
