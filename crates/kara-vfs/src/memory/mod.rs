//! `MemoryBackend`: an in-memory drive with an SFTP-like and an S3-like
//! profile, plus fault injection for tests (contract dec_01).
//!
//! Everything lives in one flat map from canonical path to node, behind a single
//! mutex that is only held inside a call: open readers and write sessions keep an
//! `Arc` to the state, never the lock, so one slow caller cannot block another.
//! With `real_directories` a directory is a `Dir` node and its parent must
//! exist; without it a directory is a `DirMarker` object or, more often, just the
//! common prefix of other keys, and it vanishes with its last object.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::ops::Bound;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use kara_core::{EntryKind, FileEntry, MetadataBag};

use crate::backend::{Backend, Cancel, Listing, WriteSession};
use crate::capabilities::Capabilities;
use crate::error::{BackendError, BackendErrorKind};
use crate::path::RemotePath;

/// How many symbolic links are followed before a chain counts as broken.
const MAX_LINK_HOPS: usize = 8;
/// Links followed by one resolution, however they nest.
const LINK_BUDGET: usize = MAX_LINK_HOPS * 4;

/// A stored object. Times live here, not in [`MemoryNode`], so snapshots compare
/// by value.
#[derive(Debug, Clone)]
enum Node {
    File {
        content: Arc<Vec<u8>>,
        modified: SystemTime,
    },
    Dir {
        modified: SystemTime,
    },
    Marker {
        modified: SystemTime,
    },
    Symlink {
        target: String,
        modified: SystemTime,
    },
}

/// What a path currently is.
enum Lookup<'a> {
    Missing,
    Root,
    /// An object-store prefix: no node of its own, but keys below it.
    Implicit,
    Node(&'a Node),
}

impl Lookup<'_> {
    fn is_directory(&self) -> bool {
        matches!(
            self,
            Lookup::Root | Lookup::Implicit | Lookup::Node(Node::Dir { .. } | Node::Marker { .. })
        )
    }
}

#[derive(Debug)]
struct ActiveFault {
    fault: Fault,
    remaining: Option<u32>,
}

/// Internal state. Private: its shape is the implementation's business.
#[derive(Debug)]
struct State {
    caps: Capabilities,
    nodes: BTreeMap<String, Node>,
    faults: Vec<ActiveFault>,
    capacity: Option<u64>,
    connected: bool,
    stats: MemoryStats,
    /// Bytes buffered by open write sessions, counted against the capacity.
    session_bytes: u64,
    /// Bumped by every `disconnect`: sessions and readers opened before it are dead.
    epoch: u64,
}

/// An in-memory [`Backend`]. Its behaviour follows `real_directories`,
/// `atomic_rename`, `server_side_copy` and `symlinks` of its capabilities.
#[derive(Debug)]
pub struct MemoryBackend {
    caps: Capabilities,
    state: Arc<Mutex<State>>,
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

// ---------------------------------------------------------------------------
// Small helpers.

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    // A panic in another thread must not turn every later call into one.
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn to_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn err(kind: BackendErrorKind, path: &RemotePath) -> BackendError {
    BackendError::new(kind, Some(path.clone()))
}

/// `Other` with the precise `io::ErrorKind` as source (contract dec_05).
fn other(io_kind: io::ErrorKind, path: &RemotePath) -> BackendError {
    err(BackendErrorKind::Other, path).with_source(io::Error::from(io_kind))
}

fn effect_error(effect: FaultEffect, path: &RemotePath) -> BackendError {
    match effect {
        FaultEffect::Fail(kind) => err(kind, path),
        FaultEffect::Cancel(token) => {
            token.cancel();
            err(BackendErrorKind::Cancelled, path)
        }
    }
}

/// `"/a/b"` -> `"/a/b/"`; the root stays `"/"`.
fn child_prefix(path: &RemotePath) -> String {
    if path.is_root() {
        String::from("/")
    } else {
        format!("{}/", path.as_str())
    }
}

/// Every key that starts with `prefix` (which ends in `/`) lies in
/// `[prefix, prefix-with-'0'-for-'/')`, because `'0'` follows `'/'`.
fn subtree_bounds(prefix: &str) -> (Bound<String>, Bound<String>) {
    let head = prefix.strip_suffix('/').unwrap_or(prefix);
    (
        Bound::Included(prefix.to_owned()),
        Bound::Excluded(format!("{head}0")),
    )
}

fn path_of(key: &str, fallback: &RemotePath) -> RemotePath {
    RemotePath::parse(key).unwrap_or_else(|_| fallback.clone())
}

/// Resolves a link target against the directory of the link.
///
/// `.` and `..` are folded here, segment by segment: `RemotePath::parse` rejects
/// them, and relative links such as `../t/x` are the common case on POSIX servers.
fn absolutize(link: &RemotePath, target: &str) -> Option<RemotePath> {
    let parent = link.parent()?;
    let mut segments: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        parent.segments().collect()
    };
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            name => segments.push(name),
        }
    }
    RemotePath::parse(&format!("/{}", segments.join("/"))).ok()
}

/// Puts back the path the caller used in an error that names the resolved one.
fn named(mut error: BackendError, used: &RemotePath, real: &RemotePath) -> BackendError {
    if error.path.as_ref() == Some(real) {
        error.path = Some(used.clone());
    }
    error
}

fn entry(
    name: &str,
    kind: EntryKind,
    size: Option<u64>,
    modified: Option<SystemTime>,
    link: Option<bool>,
) -> FileEntry {
    FileEntry {
        name: OsString::from(name),
        display: name.to_owned(),
        kind,
        is_symlink: link.is_some(),
        symlink_broken: link == Some(true),
        is_hidden: name.starts_with('.'),
        size,
        modified,
        created: None,
        accessed: None,
        type_label: None,
        location: None,
        extra: MetadataBag::new(),
    }
}

// ---------------------------------------------------------------------------
// State.

impl State {
    fn new(caps: Capabilities) -> State {
        State {
            caps,
            nodes: BTreeMap::new(),
            faults: Vec::new(),
            capacity: None,
            connected: true,
            stats: MemoryStats::default(),
            session_bytes: 0,
            epoch: 0,
        }
    }

    fn ensure_connected(&self, path: &RemotePath) -> Result<(), BackendError> {
        if self.connected {
            Ok(())
        } else {
            Err(err(BackendErrorKind::Unavailable, path))
        }
    }

    fn has_children(&self, path: &RemotePath) -> bool {
        self.nodes
            .range(subtree_bounds(&child_prefix(path)))
            .next()
            .is_some()
    }

    fn lookup(&self, path: &RemotePath) -> Lookup<'_> {
        if path.is_root() {
            return Lookup::Root;
        }
        if let Some(node) = self.nodes.get(path.as_str()) {
            return Lookup::Node(node);
        }
        if !self.caps.real_directories && self.has_children(path) {
            Lookup::Implicit
        } else {
            Lookup::Missing
        }
    }

    /// Where a link ends up: the first path that is not itself a link, if it exists.
    /// Links on the way to the target are followed too.
    fn final_path(&self, link: &RemotePath, target: &str) -> Option<RemotePath> {
        self.final_path_within(link, target, &mut { LINK_BUDGET })
    }

    /// `budget` is shared by the whole resolution, so a cycle of links ends.
    /// It is spent before the target's own path is resolved: a link whose
    /// target runs through itself (`/l -> l/x`) recurses through this call.
    fn final_path_within(
        &self,
        link: &RemotePath,
        target: &str,
        budget: &mut usize,
    ) -> Option<RemotePath> {
        *budget = budget.checked_sub(1)?;
        let mut current = self.canon_within(&absolutize(link, target)?, budget);
        loop {
            *budget = budget.checked_sub(1)?;
            match self.lookup(&current) {
                Lookup::Missing => return None,
                Lookup::Node(Node::Symlink { target, .. }) => {
                    current = self.canon_within(&absolutize(&current, target)?, budget);
                }
                _ => return Some(current),
            }
        }
    }

    /// Follows a path through links to the node it finally names.
    fn resolve(&self, path: &RemotePath) -> Option<RemotePath> {
        match self.lookup(path) {
            Lookup::Node(Node::Symlink { target, .. }) => self.final_path(path, target),
            Lookup::Missing => None,
            _ => Some(path.clone()),
        }
    }

    /// `path` with every link on the way to it followed. The last component is
    /// never followed, so a destructive operation acts on the link itself.
    fn canon(&self, path: &RemotePath) -> RemotePath {
        self.canon_within(path, &mut { LINK_BUDGET })
    }

    fn canon_within(&self, path: &RemotePath, budget: &mut usize) -> RemotePath {
        match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => self
                .canon_dir_within(&parent, budget)
                .join(name)
                .unwrap_or_else(|_| path.clone()),
            _ => path.clone(),
        }
    }

    /// Like [`State::canon_within`], but also follows the last component.
    fn canon_dir_within(&self, path: &RemotePath, budget: &mut usize) -> RemotePath {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return path.clone();
        };
        let candidate = self
            .canon_dir_within(&parent, budget)
            .join(name)
            .unwrap_or_else(|_| path.clone());
        match self.lookup(&candidate) {
            Lookup::Node(Node::Symlink { target, .. }) => self
                .final_path_within(&candidate, target, budget)
                .unwrap_or(candidate),
            _ => candidate,
        }
    }

    fn entry_for(&self, path: &RemotePath, name: &str, node: Option<&Node>) -> FileEntry {
        match node {
            None => entry(name, EntryKind::Directory, None, None, None),
            Some(Node::File { content, modified }) => entry(
                name,
                EntryKind::File,
                Some(to_u64(content.len())),
                Some(*modified),
                None,
            ),
            Some(Node::Dir { modified } | Node::Marker { modified }) => {
                entry(name, EntryKind::Directory, None, Some(*modified), None)
            }
            Some(Node::Symlink { target, modified }) => {
                let resolved = self.final_path(path, target);
                match resolved.as_ref().map(|r| self.lookup(r)) {
                    Some(l) if l.is_directory() => entry(
                        name,
                        EntryKind::Directory,
                        None,
                        Some(*modified),
                        Some(false),
                    ),
                    Some(Lookup::Node(Node::File { .. })) => entry(
                        name,
                        EntryKind::File,
                        Some(to_u64(target.len())),
                        Some(*modified),
                        Some(false),
                    ),
                    _ => entry(
                        name,
                        EntryKind::File,
                        Some(to_u64(target.len())),
                        Some(*modified),
                        Some(true),
                    ),
                }
            }
        }
    }

    fn stat_entry(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        match self.lookup(path) {
            Lookup::Root => Ok(entry("/", EntryKind::Directory, None, None, None)),
            Lookup::Implicit => Ok(self.entry_for(path, path.file_name().unwrap_or("/"), None)),
            Lookup::Node(node) => {
                Ok(self.entry_for(path, path.file_name().unwrap_or("/"), Some(node)))
            }
            Lookup::Missing => Err(err(BackendErrorKind::NotFound, path)),
        }
    }

    /// The parent of `path` must be able to hold it.
    fn check_parent(&self, path: &RemotePath) -> Result<(), BackendError> {
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        if self.caps.real_directories {
            return match self.lookup(&parent) {
                l if l.is_directory() => Ok(()),
                // `parent` comes out of `canon`, which follows every link that
                // leads somewhere: a link still standing here is dangling, so
                // the parent does not exist (POSIX: ENOENT).
                Lookup::Missing | Lookup::Node(Node::Symlink { .. }) => {
                    Err(err(BackendErrorKind::NotFound, path))
                }
                _ => Err(other(io::ErrorKind::NotADirectory, path)),
            };
        }
        // Object store: any prefix is fine, as long as no ancestor is an object.
        let mut ancestor = Some(parent);
        while let Some(dir) = ancestor {
            if matches!(
                self.lookup(&dir),
                Lookup::Node(Node::File { .. } | Node::Symlink { .. })
            ) {
                return Err(other(io::ErrorKind::NotADirectory, path));
            }
            ancestor = dir.parent();
        }
        Ok(())
    }

    /// Whether a file may be committed under `path` right now.
    fn check_target(&self, path: &RemotePath, replace: bool) -> Result<(), BackendError> {
        match self.lookup(path) {
            Lookup::Root
            | Lookup::Implicit
            | Lookup::Node(Node::Dir { .. } | Node::Marker { .. }) => {
                return Err(err(BackendErrorKind::AlreadyExists, path));
            }
            Lookup::Node(Node::File { .. } | Node::Symlink { .. }) if !replace => {
                return Err(err(BackendErrorKind::AlreadyExists, path));
            }
            _ => {}
        }
        self.check_parent(path)
    }

    /// `path` itself (if it is a node) followed by everything below it, ascending.
    fn subtree_keys(&self, path: &RemotePath) -> Vec<String> {
        let mut keys = Vec::new();
        if self.nodes.contains_key(path.as_str()) {
            keys.push(path.as_str().to_owned());
        }
        keys.extend(
            self.nodes
                .range(subtree_bounds(&child_prefix(path)))
                .map(|(key, _)| key.clone()),
        );
        keys
    }

    fn committed_bytes(&self) -> u64 {
        self.nodes
            .values()
            .map(|node| match node {
                Node::File { content, .. } => to_u64(content.len()),
                _ => 0,
            })
            .fold(0, u64::saturating_add)
    }

    /// The size of the file stored at `path`; 0 for anything else.
    fn file_bytes(&self, path: &RemotePath) -> u64 {
        match self.nodes.get(path.as_str()) {
            Some(Node::File { content, .. }) => to_u64(content.len()),
            _ => 0,
        }
    }

    /// Fires the first matching fault whose `after` has been reached, consuming one
    /// of its `times`. Operations that ignore `after` pass `u64::MAX`.
    fn take_fault(&mut self, op: Op, paths: &[&RemotePath], progress: u64) -> Option<FaultEffect> {
        let index = self.faults.iter().position(|active| {
            active.fault.op == op
                && progress >= active.fault.after
                && active
                    .fault
                    .path
                    .as_ref()
                    .is_none_or(|wanted| paths.contains(&wanted))
        })?;
        let active = self.faults.get_mut(index)?;
        let effect = active.fault.effect.clone();
        if let Some(remaining) = active.remaining.as_mut() {
            *remaining = remaining.saturating_sub(1);
            if *remaining == 0 {
                self.faults.remove(index);
            }
        }
        Some(effect)
    }

    /// The nearest not yet reached `after` of a fault on this stream, so a
    /// transfer can stop exactly there.
    fn next_fault_after(&self, op: Op, path: &RemotePath, progress: u64) -> Option<u64> {
        self.faults
            .iter()
            .filter(|active| {
                active.fault.op == op
                    && active.fault.after > progress
                    && active
                        .fault
                        .path
                        .as_ref()
                        .is_none_or(|wanted| wanted == path)
            })
            .map(|active| active.fault.after)
            .min()
    }

    /// Connection check then the "start of operation" fault.
    fn begin_op(&mut self, op: Op, path: &RemotePath) -> Result<(), BackendError> {
        self.ensure_connected(path)?;
        match self.take_fault(op, &[path], u64::MAX) {
            Some(effect) => Err(effect_error(effect, path)),
            None => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Public controls.

impl MemoryBackend {
    /// SFTP column of the capability table.
    #[must_use]
    pub fn posix_like() -> MemoryBackend {
        MemoryBackend::with_capabilities(Capabilities {
            trash: false,
            atomic_rename: true,
            server_side_copy: false,
            real_directories: true,
            posix_permissions: true,
            symlinks: true,
            watch: false,
            undo_rename: true,
            undo_move: true,
        })
    }

    /// S3 column of the capability table.
    #[must_use]
    pub fn object_store_like() -> MemoryBackend {
        MemoryBackend::with_capabilities(Capabilities {
            trash: false,
            atomic_rename: false,
            server_side_copy: true,
            real_directories: false,
            posix_permissions: false,
            symlinks: false,
            watch: false,
            undo_rename: false,
            undo_move: false,
        })
    }

    /// A backend that declares, and behaves according to, `caps`.
    #[must_use]
    pub fn with_capabilities(caps: Capabilities) -> MemoryBackend {
        MemoryBackend {
            caps,
            state: Arc::new(Mutex::new(State::new(caps))),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    pub fn inject(&self, fault: Fault) -> Result<(), BackendError> {
        let remaining = fault.times;
        self.lock().faults.push(ActiveFault { fault, remaining });
        Ok(())
    }

    pub fn clear_faults(&self) -> Result<(), BackendError> {
        self.lock().faults.clear();
        Ok(())
    }

    /// Limits the bytes stored (committed plus open sessions); `None` is unlimited.
    pub fn set_capacity(&self, bytes: Option<u64>) -> Result<(), BackendError> {
        self.lock().capacity = bytes;
        Ok(())
    }

    pub fn disconnect(&self) -> Result<(), BackendError> {
        let mut st = self.lock();
        st.connected = false;
        st.epoch = st.epoch.saturating_add(1);
        Ok(())
    }

    pub fn reconnect(&self) -> Result<(), BackendError> {
        self.lock().connected = true;
        Ok(())
    }

    /// `Unsupported` unless the backend declares `symlinks`.
    pub fn create_symlink(&self, link: &RemotePath, target: &str) -> Result<(), BackendError> {
        let mut st = self.lock();
        if !st.caps.symlinks {
            return Err(err(BackendErrorKind::Unsupported, link));
        }
        let real = st.canon(link);
        if !matches!(st.lookup(&real), Lookup::Missing) {
            return Err(err(BackendErrorKind::AlreadyExists, link));
        }
        st.check_parent(&real).map_err(|e| named(e, link, &real))?;
        st.nodes.insert(
            real.as_str().to_owned(),
            Node::Symlink {
                target: target.to_owned(),
                modified: SystemTime::now(),
            },
        );
        Ok(())
    }

    pub fn snapshot(&self) -> Result<MemorySnapshot, BackendError> {
        let st = self.lock();
        let mut nodes = BTreeMap::new();
        for (key, node) in &st.nodes {
            let path = RemotePath::parse(key)
                .map_err(|e| BackendError::new(BackendErrorKind::Other, None).with_source(e))?;
            let node = match node {
                Node::File { content, .. } => MemoryNode::File {
                    content: content.as_ref().clone(),
                },
                Node::Dir { .. } => MemoryNode::Dir,
                Node::Marker { .. } => MemoryNode::DirMarker,
                Node::Symlink { target, .. } => MemoryNode::Symlink {
                    target: target.clone(),
                },
            };
            nodes.insert(path, node);
        }
        Ok(MemorySnapshot { nodes })
    }

    pub fn stats(&self) -> Result<MemoryStats, BackendError> {
        Ok(self.lock().stats)
    }
}

// ---------------------------------------------------------------------------
// Reader and write session.

/// A reader over the content as it was when it was opened.
struct MemoryReader {
    state: Arc<Mutex<State>>,
    path: RemotePath,
    data: Arc<Vec<u8>>,
    position: usize,
    transferred: u64,
    epoch: u64,
}

impl MemoryReader {
    fn fail(&self, error: BackendError) -> io::Error {
        io::Error::from(error)
    }
}

impl Read for MemoryReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut st = lock(&self.state);
        st.ensure_connected(&self.path).map_err(|e| self.fail(e))?;
        if st.epoch != self.epoch {
            return Err(self.fail(err(BackendErrorKind::Unavailable, &self.path)));
        }
        if let Some(effect) = st.take_fault(Op::Read, &[&self.path], self.transferred) {
            return Err(self.fail(effect_error(effect, &self.path)));
        }
        let rest = self.data.get(self.position..).unwrap_or_default();
        let mut n = rest.len().min(buf.len());
        if let Some(limit) = st.next_fault_after(Op::Read, &self.path, self.transferred) {
            let allowed =
                usize::try_from(limit.saturating_sub(self.transferred)).unwrap_or(usize::MAX);
            n = n.min(allowed);
        }
        if let (Some(src), Some(dst)) = (rest.get(..n), buf.get_mut(..n)) {
            dst.copy_from_slice(src);
        }
        self.position = self.position.saturating_add(n);
        self.transferred = self.transferred.saturating_add(to_u64(n));
        st.stats.bytes_read = st.stats.bytes_read.saturating_add(to_u64(n));
        Ok(n)
    }
}

/// An upload in progress: bytes are buffered and only `finish` makes them visible.
struct MemorySession {
    state: Arc<Mutex<State>>,
    /// The path the caller used: faults and errors name this one.
    path: RemotePath,
    /// Where the object lands, with links on the way followed.
    real: RemotePath,
    epoch: u64,
    replace: bool,
    buffer: Vec<u8>,
    /// Kind of the first failure; the session is poisoned from then on.
    poison: Option<BackendErrorKind>,
    open: bool,
}

impl MemorySession {
    fn counted(&self) -> u64 {
        to_u64(self.buffer.len())
    }

    /// Gives the buffered bytes back to the capacity and marks the session closed.
    fn close(&mut self, st: &mut State) {
        if self.open {
            self.open = false;
            st.stats.open_sessions = st.stats.open_sessions.saturating_sub(1);
        }
        st.session_bytes = st.session_bytes.saturating_sub(self.counted());
        self.buffer = Vec::new();
    }

    fn poisoned(&mut self, st: &mut State, kind: BackendErrorKind) -> io::Error {
        self.poison = Some(kind);
        st.session_bytes = st.session_bytes.saturating_sub(self.counted());
        self.buffer = Vec::new();
        io::Error::from(err(kind, &self.path))
    }

    fn commit(&mut self, st: &mut State) -> Result<(), BackendError> {
        if let Some(kind) = self.poison {
            return Err(err(kind, &self.path));
        }
        st.ensure_connected(&self.path)?;
        if st.epoch != self.epoch {
            return Err(err(BackendErrorKind::Unavailable, &self.path));
        }
        if let Some(effect) = st.take_fault(Op::Finish, &[&self.path], u64::MAX) {
            return Err(effect_error(effect, &self.path));
        }
        st.check_target(&self.real, self.replace)
            .map_err(|e| named(e, &self.path, &self.real))?;
        let content = std::mem::take(&mut self.buffer);
        // The bytes move from "open session" to "committed".
        st.session_bytes = st.session_bytes.saturating_sub(to_u64(content.len()));
        st.nodes.insert(
            self.real.as_str().to_owned(),
            Node::File {
                content: Arc::new(content),
                modified: SystemTime::now(),
            },
        );
        Ok(())
    }
}

impl Write for MemorySession {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Some(kind) = self.poison {
            return Err(io::Error::from(err(kind, &self.path)));
        }
        if data.is_empty() {
            return Ok(0);
        }
        let state = Arc::clone(&self.state);
        let mut st = lock(&state);
        if !st.connected || st.epoch != self.epoch {
            return Err(self.poisoned(&mut st, BackendErrorKind::Unavailable));
        }
        if let Some(effect) = st.take_fault(Op::Write, &[&self.path], self.counted()) {
            let kind = effect_error(effect, &self.path).kind;
            return Err(self.poisoned(&mut st, kind));
        }
        let mut n = data.len();
        if let Some(limit) = st.next_fault_after(Op::Write, &self.path, self.counted()) {
            let allowed =
                usize::try_from(limit.saturating_sub(self.counted())).unwrap_or(usize::MAX);
            n = n.min(allowed);
        }
        if let Some(capacity) = st.capacity {
            // A replacing session is measured against the drive as it will be
            // after `finish`: the object it replaces is not counted twice.
            let replaced = if self.replace {
                st.file_bytes(&self.real)
            } else {
                0
            };
            let used = st
                .committed_bytes()
                .saturating_add(st.session_bytes)
                .saturating_sub(replaced);
            if used.saturating_add(to_u64(n)) > capacity {
                return Err(self.poisoned(&mut st, BackendErrorKind::NoSpace));
            }
        }
        if let Some(chunk) = data.get(..n) {
            self.buffer.extend_from_slice(chunk);
        }
        st.session_bytes = st.session_bytes.saturating_add(to_u64(n));
        st.stats.bytes_written = st.stats.bytes_written.saturating_add(to_u64(n));
        Ok(n)
    }

    /// Flushing never commits.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl WriteSession for MemorySession {
    fn finish(mut self: Box<Self>) -> Result<(), BackendError> {
        let state = Arc::clone(&self.state);
        let mut st = lock(&state);
        let outcome = self.commit(&mut st);
        self.close(&mut st);
        outcome
    }

    fn abort(mut self: Box<Self>) -> Result<(), BackendError> {
        let state = Arc::clone(&self.state);
        let mut st = lock(&state);
        let reachable = st.ensure_connected(&self.path);
        self.close(&mut st);
        reachable
    }
}

impl Drop for MemorySession {
    fn drop(&mut self) {
        if self.open {
            let state = Arc::clone(&self.state);
            let mut st = lock(&state);
            self.close(&mut st);
        }
    }
}

// ---------------------------------------------------------------------------
// The trait.

impl Backend for MemoryBackend {
    fn capabilities(&self) -> Capabilities {
        self.caps
    }

    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        let mut st = self.lock();
        st.ensure_connected(dir)?;
        if cancel.is_cancelled() {
            return Err(err(BackendErrorKind::Cancelled, dir));
        }
        let real = st.canon(dir);
        let target = match st.lookup(&real) {
            Lookup::Missing => return Err(err(BackendErrorKind::NotFound, dir)),
            Lookup::Node(Node::File { .. }) => {
                return Err(other(io::ErrorKind::NotADirectory, dir));
            }
            Lookup::Node(Node::Symlink { .. }) => match st.resolve(&real) {
                Some(final_path) if st.lookup(&final_path).is_directory() => final_path,
                Some(_) => return Err(other(io::ErrorKind::NotADirectory, dir)),
                None => return Err(err(BackendErrorKind::NotFound, dir)),
            },
            _ => real,
        };

        let prefix = child_prefix(&target);
        let (_, upper) = subtree_bounds(&prefix);
        let mut cursor = Bound::Included(prefix.clone());
        let mut entries = Vec::new();
        loop {
            let next = st
                .nodes
                .range((cursor.clone(), upper.clone()))
                .next()
                .map(|(key, node)| (key.clone(), node.clone()));
            let Some((key, node)) = next else {
                // An empty directory must honour `after: 0` as well.
                if entries.is_empty()
                    && let Some(effect) = st.take_fault(Op::List, &[dir], 0)
                {
                    return Err(effect_error(effect, dir));
                }
                break;
            };

            let index = to_u64(entries.len());
            if let Some(effect) = st.take_fault(Op::List, &[dir], index) {
                return Err(effect_error(effect, dir));
            }
            if cancel.is_cancelled() {
                return Err(err(BackendErrorKind::Cancelled, dir));
            }

            let rest = key.get(prefix.len()..).unwrap_or_default();
            match rest.split_once('/') {
                None => {
                    let child = path_of(&key, dir);
                    entries.push(st.entry_for(&child, rest, Some(&node)));
                    cursor = Bound::Excluded(key);
                }
                Some((name, _)) => {
                    // Everything below this child is skipped in one jump. An
                    // explicit node of the same name was listed on its own.
                    let own_key = format!("{prefix}{name}");
                    if !st.nodes.contains_key(&own_key) {
                        let child = path_of(&own_key, dir);
                        entries.push(st.entry_for(&child, name, None));
                    }
                    cursor = Bound::Included(format!("{own_key}0"));
                }
            }
        }
        Ok(Listing {
            entries,
            errors: Vec::new(),
        })
    }

    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        let mut st = self.lock();
        st.begin_op(Op::Stat, path)?;
        let real = st.canon(path);
        st.stat_entry(&real).map_err(|e| named(e, path, &real))
    }

    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn Read + Send>, BackendError> {
        let mut st = self.lock();
        st.begin_op(Op::OpenRead, path)?;
        let real = st.canon(path);
        let resolved = st
            .resolve(&real)
            .ok_or_else(|| err(BackendErrorKind::NotFound, path))?;
        let data = match st.lookup(&resolved) {
            Lookup::Node(Node::File { content, .. }) => Arc::clone(content),
            _ => return Err(other(io::ErrorKind::IsADirectory, path)),
        };
        let position = usize::try_from(from)
            .ok()
            .filter(|position| *position <= data.len())
            .ok_or_else(|| other(io::ErrorKind::InvalidInput, path))?;
        Ok(Box::new(MemoryReader {
            state: Arc::clone(&self.state),
            path: path.clone(),
            data,
            position,
            transferred: 0,
            epoch: st.epoch,
        }))
    }

    fn begin_write(
        &self,
        path: &RemotePath,
        _size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        let mut st = self.lock();
        st.begin_op(Op::BeginWrite, path)?;
        let real = st.canon(path);
        st.check_target(&real, replace)
            .map_err(|e| named(e, path, &real))?;
        st.stats.open_sessions = st.stats.open_sessions.saturating_add(1);
        Ok(Box::new(MemorySession {
            state: Arc::clone(&self.state),
            path: path.clone(),
            real,
            epoch: st.epoch,
            replace,
            buffer: Vec::new(),
            poison: None,
            open: true,
        }))
    }

    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        let mut st = self.lock();
        st.begin_op(Op::CreateDir, path)?;
        let real = st.canon(path);
        if !matches!(st.lookup(&real), Lookup::Missing) {
            return Err(err(BackendErrorKind::AlreadyExists, path));
        }
        st.check_parent(&real).map_err(|e| named(e, path, &real))?;
        let modified = SystemTime::now();
        let node = if st.caps.real_directories {
            Node::Dir { modified }
        } else {
            Node::Marker { modified }
        };
        st.nodes.insert(real.as_str().to_owned(), node);
        Ok(())
    }

    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        let mut st = self.lock();
        st.ensure_connected(from)?;
        if from.is_root() {
            return Err(other(io::ErrorKind::InvalidInput, from));
        }
        if to.is_root() {
            return Err(other(io::ErrorKind::InvalidInput, to));
        }
        let (src, dst) = (st.canon(from), st.canon(to));
        if matches!(st.lookup(&src), Lookup::Missing) {
            return Err(err(BackendErrorKind::NotFound, from));
        }
        if src == dst {
            return Ok(());
        }
        // Only a directory can be moved into itself, and only to a place that
        // exists: below a file, or below a missing or dangling parent, there
        // is no parent at all, which the parent check reports (POSIX: ENOTDIR
        // or ENOENT, not EINVAL). An object store has a parent everywhere.
        let parent_exists = !st.caps.real_directories
            || dst.parent().is_some_and(|parent| st.lookup(&parent).is_directory());
        if dst.starts_with(&src) && st.lookup(&src).is_directory() && parent_exists {
            return Err(other(io::ErrorKind::InvalidInput, to));
        }
        if !matches!(st.lookup(&dst), Lookup::Missing) {
            return Err(err(BackendErrorKind::AlreadyExists, to));
        }
        st.check_parent(&dst).map_err(|e| named(e, to, &dst))?;

        let keys = st.subtree_keys(&src);
        let atomic = st.caps.atomic_rename;
        // An atomic rename fails as a whole, so a fault fires whatever its `after`.
        if atomic && let Some(effect) = st.take_fault(Op::Rename, &[from, to], u64::MAX) {
            return Err(effect_error(effect, from));
        }
        for (index, key) in keys.iter().enumerate() {
            // A non-atomic rename is copy then delete, one object at a time: a
            // failure between two objects leaves each one under exactly one name.
            if !atomic && let Some(effect) = st.take_fault(Op::Rename, &[from, to], to_u64(index)) {
                return Err(effect_error(effect, &path_of(key, from)));
            }
            let suffix = key.get(src.as_str().len()..).unwrap_or_default();
            if let Some(node) = st.nodes.remove(key) {
                st.nodes.insert(format!("{}{suffix}", dst.as_str()), node);
            }
        }
        Ok(())
    }

    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        let mut st = self.lock();
        st.begin_op(Op::Remove, path)?;
        if path.is_root() {
            return Err(other(io::ErrorKind::InvalidInput, path));
        }
        let real = st.canon(path);
        match st.lookup(&real) {
            Lookup::Missing => return Err(err(BackendErrorKind::NotFound, path)),
            Lookup::Implicit => return Err(other(io::ErrorKind::DirectoryNotEmpty, path)),
            Lookup::Node(Node::Dir { .. } | Node::Marker { .. }) if st.has_children(&real) => {
                return Err(other(io::ErrorKind::DirectoryNotEmpty, path));
            }
            _ => {}
        }
        st.nodes.remove(real.as_str());
        Ok(())
    }

    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        let mut st = self.lock();
        st.ensure_connected(path)?;
        if cancel.is_cancelled() {
            return Err(err(BackendErrorKind::Cancelled, path));
        }
        if path.is_root() {
            return Err(other(io::ErrorKind::InvalidInput, path));
        }
        let real = st.canon(path);
        if matches!(st.lookup(&real), Lookup::Missing) {
            return Err(err(BackendErrorKind::NotFound, path));
        }
        // Children before parents, so a failure never leaves a child whose
        // parent is gone. Links are removed as links, never followed.
        let mut keys = st.subtree_keys(&real);
        keys.reverse();
        for (index, key) in keys.iter().enumerate() {
            let object = path_of(key, path);
            if let Some(effect) = st.take_fault(Op::RemoveTree, &[path, &object], to_u64(index)) {
                let at = match &effect {
                    FaultEffect::Fail(_) => &object,
                    FaultEffect::Cancel(_) => path,
                };
                return Err(effect_error(effect, at));
            }
            if cancel.is_cancelled() {
                return Err(err(BackendErrorKind::Cancelled, path));
            }
            st.nodes.remove(key);
        }
        Ok(())
    }

    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        let mut st = self.lock();
        st.begin_op(Op::CopyWithin, from)?;
        if !st.caps.server_side_copy {
            return Err(err(BackendErrorKind::Unsupported, from));
        }
        let (src, dst) = (st.canon(from), st.canon(to));
        let content = match st.lookup(&src) {
            Lookup::Missing => return Err(err(BackendErrorKind::NotFound, from)),
            Lookup::Node(Node::File { content, .. }) => Arc::clone(content),
            _ => return Err(err(BackendErrorKind::Unsupported, from)),
        };
        if !matches!(st.lookup(&dst), Lookup::Missing) {
            return Err(err(BackendErrorKind::AlreadyExists, to));
        }
        st.check_parent(&dst).map_err(|e| named(e, to, &dst))?;
        if let Some(capacity) = st.capacity {
            let used = st.committed_bytes().saturating_add(st.session_bytes);
            if used.saturating_add(to_u64(content.len())) > capacity {
                return Err(err(BackendErrorKind::NoSpace, to));
            }
        }
        st.nodes.insert(
            dst.as_str().to_owned(),
            Node::File {
                content,
                modified: SystemTime::now(),
            },
        );
        Ok(())
    }
}
