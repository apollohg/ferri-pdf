use crate::resources::ResourceError;
#[cfg(file_system)]
use std::path::{Path, PathBuf};
use url::Url;

#[cfg(file_system)]
const CURRENT_DIRECTORY: &str = ".";
/// The default base, which names no location: relative references do not resolve against it
/// and it gives a render no folder to read local files in.
const BLANK_URL: &str = "about:blank";
pub(crate) const FILE_SCHEME: &str = "file";
#[cfg(file_system)]
const URL_PATH_SEPARATOR: char = '/';
/// The relative reference to the folder that holds a URL's last path segment.
#[cfg(not(file_system))]
const CONTAINING_DIRECTORY: &str = ".";

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
    /// The URL of the containing document or resource; `None` for a directory and for the
    /// default base, which name no document.
    pub fn url(&self) -> Option<&Url> {
        match self {
            Self::Url(url) if url.as_str() != BLANK_URL => Some(url),
            Self::Url(_) | Self::Directory(_) => None,
        }
    }

    /// Whether content with this base is neither a local folder, a `file:` document nor
    /// content given without a base. Such content reads no local files.
    pub fn is_remote(&self) -> bool {
        match self {
            Self::Url(url) => url.scheme() != FILE_SCHEME && url.as_str() != BLANK_URL,
            #[cfg(not(file_system))]
            Self::Directory(url) => url.scheme() != FILE_SCHEME,
            #[cfg(file_system)]
            Self::Directory(_) => false,
        }
    }

    /// The folder a render of a document with this base reads local files in: the
    /// directory itself, or the one holding a `file:` document. `None` for every other base.
    #[cfg(file_system)]
    pub fn local_directory(&self) -> Option<PathBuf> {
        match self {
            Self::Directory(directory) => Some(named_directory(directory).to_path_buf()),
            Self::Url(url) if url.scheme() == FILE_SCHEME => {
                let path = url.to_file_path().ok()?;
                if url.path().ends_with(URL_PATH_SEPARATOR) {
                    Some(path)
                } else {
                    path.parent().map(Path::to_path_buf)
                }
            }
            Self::Url(_) => None,
        }
    }

    /// The folder a render of a document with this base reads local files in, as a `file:`
    /// URL: the directory itself, or the one holding a `file:` document. `None` for every
    /// other base.
    #[cfg(not(file_system))]
    pub fn local_directory(&self) -> Option<Url> {
        let (Self::Directory(url) | Self::Url(url)) = self;
        (url.scheme() == FILE_SCHEME)
            .then(|| url.join(CONTAINING_DIRECTORY).ok())
            .flatten()
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
                let directory = named_directory(directory);
                let path =
                    std::path::absolute(directory).map_err(|source| ResourceError::Read {
                        path: directory.to_path_buf(),
                        source,
                    })?;
                Url::from_directory_path(path)
                    .map_err(|()| ResourceError::InvalidBase(directory.to_path_buf()))?
            }
        };
        match base.join(reference) {
            Ok(url) => Ok(url),
            // The empty reference names the containing document itself, wherever it is.
            Err(url::ParseError::RelativeUrlWithCannotBeABaseBase) if reference.is_empty() => {
                Ok(base)
            }
            Err(url::ParseError::RelativeUrlWithCannotBeABaseBase) => {
                Err(ResourceError::NoBase(reference.to_owned()))
            }
            Err(source) => Err(ResourceError::Url {
                reference: reference.to_owned(),
                source,
            }),
        }
    }
}

/// `directory`, or the current directory for the empty parent of a bare file name.
#[cfg(file_system)]
fn named_directory(directory: &Path) -> &Path {
    if directory.as_os_str().is_empty() {
        Path::new(CURRENT_DIRECTORY)
    } else {
        directory
    }
}
