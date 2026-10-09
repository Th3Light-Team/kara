//! `MemoryBackend`: an in-memory drive with an SFTP-like and an S3-like
//! profile, plus fault injection for tests (contract dec_01).

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Mutex;

use kara_core::FileEntry;

use crate::backend::{Backend, Cancel, Listing, WriteSession};
use crate::capabilities::Capabilities;
use crate::error::{BackendError, BackendErrorKind};
use crate::path::RemotePath;

/// Internal state. Private: its shape is the implementation's business.
#[derive(Debug, Default)]
struct State {}

/// An in-memory [`Backend`]. Its behaviour follows `real_directories`,
/// `atomic_rename`, `server_side_copy` and `symlinks` of its capabilities.
#[derive(Debug)]
pub struct MemoryBackend {
    state: Mutex<State>,
}

/// The operation a [`Fault`] targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    List,
    Stat,
    OpenRead,
    Read,
    BeginWrite,
    Write,
    Finish,
    CreateDir,
    Rename,
    Remove,
    RemoveTree,
    CopyWithin,
}

/// What an injected fault does when it fires.
#[derive(Debug, Clone)]
pub enum FaultEffect {
    /// The operation fails with this kind.
    Fail(BackendErrorKind),
    /// The token is cancelled; the operation then observes it.
    Cancel(Cancel),
}

/// An injected fault.
///
/// `path: None` matches any path. `after` counts entries yielded (List), bytes
/// transferred (Read/Write) or objects processed (Rename/RemoveTree) before the
/// fault fires, and is ignored otherwise. `times: None` fires until
/// [`MemoryBackend::clear_faults`].
#[derive(Debug, Clone)]
pub struct Fault {
    pub op: Op,
    pub path: Option<RemotePath>,
    pub after: u64,
    pub effect: FaultEffect,
    pub times: Option<u32>,
}

/// A committed node. Holds no times, so snapshots compare by value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryNode {
    File {
        content: Vec<u8>,
    },
    Dir,
    /// The object-store `key/` marker.
    DirMarker,
    Symlink {
        target: String,
    },
}

/// Committed state only; never contains the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub nodes: BTreeMap<RemotePath, MemoryNode>,
}

/// Bytes moved through readers and write sessions (not by `copy_within`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryStats {
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub open_sessions: usize,
}

impl MemoryBackend {
    /// SFTP column of the capability table.
    #[must_use]
    pub fn posix_like() -> MemoryBackend {
        todo!("MemoryBackend::posix_like")
    }

    /// S3 column of the capability table.
    #[must_use]
    pub fn object_store_like() -> MemoryBackend {
        todo!("MemoryBackend::object_store_like")
    }

    /// A backend that declares, and behaves according to, `caps`.
    #[must_use]
    pub fn with_capabilities(_caps: Capabilities) -> MemoryBackend {
        todo!("MemoryBackend::with_capabilities")
    }

    pub fn inject(&self, _fault: Fault) -> Result<(), BackendError> {
        todo!("MemoryBackend::inject")
    }

    pub fn clear_faults(&self) -> Result<(), BackendError> {
        todo!("MemoryBackend::clear_faults")
    }

    /// Limits the bytes stored (committed plus open sessions); `None` is unlimited.
    pub fn set_capacity(&self, _bytes: Option<u64>) -> Result<(), BackendError> {
        todo!("MemoryBackend::set_capacity")
    }

    pub fn disconnect(&self) -> Result<(), BackendError> {
        todo!("MemoryBackend::disconnect")
    }

    pub fn reconnect(&self) -> Result<(), BackendError> {
        todo!("MemoryBackend::reconnect")
    }

    /// `Unsupported` unless the backend declares `symlinks`.
    pub fn create_symlink(&self, _link: &RemotePath, _target: &str) -> Result<(), BackendError> {
        todo!("MemoryBackend::create_symlink")
    }

    pub fn snapshot(&self) -> Result<MemorySnapshot, BackendError> {
        todo!("MemoryBackend::snapshot")
    }

    pub fn stats(&self) -> Result<MemoryStats, BackendError> {
        todo!("MemoryBackend::stats")
    }
}

impl Backend for MemoryBackend {
    fn capabilities(&self) -> Capabilities {
        todo!("MemoryBackend::capabilities")
    }

    fn list(&self, _dir: &RemotePath, _cancel: &Cancel) -> Result<Listing, BackendError> {
        todo!("MemoryBackend::list")
    }

    fn stat(&self, _path: &RemotePath) -> Result<FileEntry, BackendError> {
        todo!("MemoryBackend::stat")
    }

    fn open_read(
        &self,
        _path: &RemotePath,
        _from: u64,
    ) -> Result<Box<dyn Read + Send>, BackendError> {
        todo!("MemoryBackend::open_read")
    }

    fn begin_write(
        &self,
        _path: &RemotePath,
        _size_hint: Option<u64>,
        _replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        todo!("MemoryBackend::begin_write")
    }

    fn create_dir(&self, _path: &RemotePath) -> Result<(), BackendError> {
        todo!("MemoryBackend::create_dir")
    }

    fn rename(&self, _from: &RemotePath, _to: &RemotePath) -> Result<(), BackendError> {
        todo!("MemoryBackend::rename")
    }

    fn remove(&self, _path: &RemotePath) -> Result<(), BackendError> {
        todo!("MemoryBackend::remove")
    }

    fn remove_tree(&self, _path: &RemotePath, _cancel: &Cancel) -> Result<(), BackendError> {
        todo!("MemoryBackend::remove_tree")
    }

    fn copy_within(&self, _from: &RemotePath, _to: &RemotePath) -> Result<(), BackendError> {
        todo!("MemoryBackend::copy_within")
    }
}
