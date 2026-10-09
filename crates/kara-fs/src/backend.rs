//! `LocalBackend`: the local filesystem behind [`kara_vfs::Backend`].
//!
//! Contract slug: `localbackend-kara-fs-implements-kara-vfs-backend-step-2`.
//! Design: `docs/remote-backends.md`, «Order of work», step 2.
//!
//! The existing code stays the implementation: listing goes through
//! [`crate::listing::describe`], the no-replace rename is the one the trash
//! already uses, and `remove_tree` is [`crate::trash::delete_permanently`].
//! Writes go to a hidden temporary sibling that is fsynced and renamed.
//!
//! Mapping between local paths and [`RemotePath`] is lexical: the backend has a
//! root (`"/"` for [`LocalBackend::system`]) and a `RemotePath` names the root
//! joined with its segments, byte for byte.

use std::io::Read;
use std::path::{Path, PathBuf};

use kara_core::FileEntry;
use kara_vfs::{
    Backend, BackendError, Cancel, Capabilities, Listing, RemotePath, RemotePathError, WriteSession,
};

/// The local filesystem as a [`Backend`], rooted at a directory.
#[derive(Debug, Clone)]
pub struct LocalBackend {
    root: PathBuf,
}

/// Why a local path has no [`RemotePath`] in a given [`LocalBackend`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocalPathError {
    #[error("local path is not absolute")]
    NotAbsolute,
    #[error("local path is outside the backend root")]
    OutsideRoot,
    #[error(transparent)]
    Path(#[from] RemotePathError),
}

impl LocalBackend {
    /// What a local POSIX volume offers (contract cb_02). `trash` is true even
    /// though the trait has no trash method: `kara-ops` routes Supr on a local
    /// location to `kara_fs::trash` (contract dec_01).
    pub const CAPABILITIES: Capabilities = Capabilities {
        trash: true,
        atomic_rename: true,
        server_side_copy: false,
        real_directories: true,
        posix_permissions: true,
        symlinks: true,
        watch: true,
        undo_rename: true,
        undo_move: true,
    };

    /// The whole system, rooted at `"/"`. No I/O, cannot fail.
    #[must_use]
    pub fn system() -> LocalBackend {
        todo!("LocalBackend::system")
    }

    /// A backend rooted at `root`, which must be an existing directory.
    pub fn with_root(root: &Path) -> Result<LocalBackend, BackendError> {
        let _ = root;
        todo!("LocalBackend::with_root")
    }

    /// The root, exactly as given.
    #[must_use]
    pub fn root(&self) -> &Path {
        todo!("LocalBackend::root")
    }

    /// The local path a [`RemotePath`] names: lexical, no I/O.
    #[must_use]
    pub fn to_local(&self, path: &RemotePath) -> PathBuf {
        let _ = (&self.root, path);
        todo!("LocalBackend::to_local")
    }

    /// The [`RemotePath`] of a local path under the root: lexical, no I/O.
    pub fn to_remote(&self, local: &Path) -> Result<RemotePath, LocalPathError> {
        let _ = local;
        todo!("LocalBackend::to_remote")
    }
}

impl Backend for LocalBackend {
    fn capabilities(&self) -> Capabilities {
        todo!("LocalBackend::capabilities")
    }

    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        let _ = (dir, cancel);
        todo!("LocalBackend::list")
    }

    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        let _ = path;
        todo!("LocalBackend::stat")
    }

    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn Read + Send>, BackendError> {
        let _ = (path, from);
        todo!("LocalBackend::open_read")
    }

    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        let _ = (path, size_hint, replace);
        todo!("LocalBackend::begin_write")
    }

    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        let _ = path;
        todo!("LocalBackend::create_dir")
    }

    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        let _ = (from, to);
        todo!("LocalBackend::rename")
    }

    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        let _ = path;
        todo!("LocalBackend::remove")
    }

    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        let _ = (path, cancel);
        todo!("LocalBackend::remove_tree")
    }

    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        let _ = (from, to);
        todo!("LocalBackend::copy_within")
    }
}
