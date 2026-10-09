//! The `Backend` trait, its `WriteSession`, `Listing` and `Cancel`.

use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use kara_core::FileEntry;

use crate::capabilities::Capabilities;
use crate::error::BackendError;
use crate::path::RemotePath;

/// Chunk size of transfers: the suite's «file larger than one chunk» and the
/// granularity `kara-ops` uses for cancel and pause.
pub const TRANSFER_CHUNK: usize = 1 << 20;

/// Cooperative cancellation flag. Clones share state; `cancel` is idempotent.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A fresh, not cancelled token.
    #[must_use]
    pub fn new() -> Cancel {
        todo!("Cancel::new")
    }

    /// Requests cancellation. There is no way back.
    pub fn cancel(&self) {
        todo!("Cancel::cancel")
    }

    /// Whether cancellation was requested on this token or any clone.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        todo!("Cancel::is_cancelled")
    }
}

/// A directory listing: unsorted entries plus per-entry failures.
#[derive(Debug, Default)]
pub struct Listing {
    pub entries: Vec<FileEntry>,
    pub errors: Vec<BackendError>,
}

/// A remote drive. Blocking, `Send + Sync`, object safe.
pub trait Backend: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError>;
    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError>;

    fn open_read(&self, path: &RemotePath, from: u64)
    -> Result<Box<dyn Read + Send>, BackendError>;
    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError>;

    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError>;
    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError>;
    /// File or empty directory.
    fn remove(&self, path: &RemotePath) -> Result<(), BackendError>;
    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError>;

    /// Only called if `capabilities().server_side_copy`.
    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError>;
}

/// An upload in progress. Nothing is visible under the final name until
/// [`WriteSession::finish`]; dropping the session aborts it.
pub trait WriteSession: Write + Send {
    /// Commits the bytes written under the final name.
    fn finish(self: Box<Self>) -> Result<(), BackendError>;
    /// Discards the session, leaving nothing behind (contract dec_02).
    fn abort(self: Box<Self>) -> Result<(), BackendError>;
}
