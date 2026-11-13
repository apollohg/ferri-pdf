//! Local files, read only inside one folder.
use crate::resources::{
    DEFAULT_MAX_RESOURCE_BYTES, DataResolver, FILE_SCHEME, ResolvedResource, Resource,
    ResourceBase, ResourceError, ResourceMetadata, ResourceResolver, Url,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
};

/// Loads `data:` URLs and the local files inside `root`. Content whose base is remote reads
/// no local file, whatever the root.
#[derive(Clone, Debug)]
pub struct FileResolver {
    /// The only folder files are read in, links and `..` resolved; `None` reads no file.
    pub root: Option<PathBuf>,
    pub max_resource_bytes: usize,
}
/// Reads no local file.
impl Default for FileResolver {
    fn default() -> Self {
        Self {
            root: None,
            max_resource_bytes: DEFAULT_MAX_RESOURCE_BYTES,
        }
    }
}
impl FileResolver {
    /// Reads inside the folder of a document with `base`, [`ResourceBase::local_directory`].
    pub fn for_base(base: &ResourceBase) -> Self {
        Self {
            root: base.local_directory(),
            ..Self::default()
        }
    }

    /// Opens the regular file `path` names if its real path is inside `root`'s.
    fn open(&self, root: &Path, path: &Path) -> Result<(File, PathBuf), ResourceError> {
        let canonical = path.canonicalize().map_err(read_error(path))?;
        let root = root.canonicalize().map_err(read_error(root))?;
        if !canonical.starts_with(&root) {
            return Err(ResourceError::OutsideRoot(canonical));
        }
        Ok((self.open_checked(&canonical)?, canonical))
    }

    /// Opens the regular file within the byte limit that `canonical`, a real path, names.
    fn open_checked(&self, canonical: &Path) -> Result<File, ResourceError> {
        let unread = read_error(canonical);
        // Checked by name first, so that only a file put in its place since is opened unseen.
        if !fs::symlink_metadata(canonical).map_err(&unread)?.is_file() {
            return Err(ResourceError::NotRegularFile(canonical.to_path_buf()));
        }
        let file = unfollowed().open(canonical).map_err(&unread)?;
        // The path was checked and then opened by name, so the open file must still be the
        // one the checked path names; everything after this reads the handle, not the name.
        if canonical.canonicalize().map_err(&unread)? != canonical
            || !names(canonical, &file).map_err(&unread)?
        {
            return Err(ResourceError::Changed(canonical.to_path_buf()));
        }
        let metadata = file.metadata().map_err(&unread)?;
        if !metadata.is_file() {
            return Err(ResourceError::NotRegularFile(canonical.to_path_buf()));
        }
        if metadata.len() > self.max_resource_bytes as u64 {
            return Err(self.too_large(canonical.to_path_buf()));
        }
        Ok(file)
    }

    fn too_large(&self, path: PathBuf) -> ResourceError {
        ResourceError::TooLarge {
            path,
            limit: self.max_resource_bytes,
        }
    }
}

fn read_error(path: &Path) -> impl Fn(io::Error) -> ResourceError {
    move |source| ResourceError::Read {
        path: path.to_path_buf(),
        source,
    }
}

/// Options that open a FIFO without waiting for its writer and refuse a link put in the
/// file's place.
#[cfg(unix)]
fn unfollowed() -> OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    options
}
/// Without such flags, the checks on the open handle stand alone.
#[cfg(not(unix))]
fn unfollowed() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true);
    options
}

/// Whether `path`, without following a link, names the open `file`.
#[cfg(unix)]
fn names(path: &Path, file: &File) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let (named, opened) = (fs::symlink_metadata(path)?, file.metadata()?);
    Ok(named.dev() == opened.dev() && named.ino() == opened.ino())
}
#[cfg(not(unix))]
fn names(path: &Path, file: &File) -> io::Result<bool> {
    use same_file::Handle;
    Ok(Handle::from_path(path)? == Handle::from_file(file.try_clone()?)?)
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
        let data = DataResolver {
            max_resource_bytes: self.max_resource_bytes,
        };
        if let Some(resolved) = data.resolve_data_url(reference, base) {
            return resolved;
        }
        let url = base.resolve(reference)?;
        if url.scheme() != FILE_SCHEME {
            return Err(ResourceError::UnsupportedScheme(url.scheme().into()));
        }
        if base.is_remote() {
            return Err(ResourceError::LocalFromRemote(url));
        }
        let Some(root) = &self.root else {
            return Err(ResourceError::NoFileRoot(url));
        };
        let path = url
            .to_file_path()
            .map_err(|()| ResourceError::InvalidFileUrl(url))?;
        let (file, canonical) = self.open(root, &path)?;
        // One byte more than the limit shows a file that grew past it since its size was read.
        let mut bytes = Vec::new();
        file.take(self.max_resource_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(read_error(&canonical))?;
        if bytes.len() > self.max_resource_bytes {
            return Err(self.too_large(canonical));
        }
        Ok(ResolvedResource {
            resource: Resource {
                bytes,
                base: Url::from_file_path(&canonical)
                    .map_err(|()| ResourceError::InvalidBase(canonical.clone()))?
                    .into(),
            },
            metadata: ResourceMetadata::default(),
        })
    }
}
