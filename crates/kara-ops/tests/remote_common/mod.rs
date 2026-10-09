//! Shared support of the `remote_*` test files: a job driven like the UI
//! drives it, memory drives behind a resolver, and a few fixtures.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use kara_core::FileEntry;
use kara_ops::runner::{
    Answer, ConflictPrompt, Event, FailurePrompt, Handle, Op, Outcome, Request, spawn, spawn_with,
};
use kara_ops::{BackendResolver, ErrorDecision, LocationRequest, Resolution};
use kara_vfs::memory::{MemoryBackend, MemoryNode};
use kara_vfs::{
    Backend, BackendError, Cancel, Capabilities, DriveId, Listing, Location, RemotePath,
    WriteSession,
};
use tempfile::TempDir;

pub struct Run {
    pub handle: Handle,
    pub events: mpsc::Receiver<Event>,
}

/// What the test answers to every question.
#[derive(Clone)]
pub struct Script {
    pub conflict: Option<Resolution>,
    pub apply_to_all: bool,
    pub on_error: ErrorDecision,
}

impl Script {
    pub fn errors(on_error: ErrorDecision) -> Script {
        Script {
            conflict: None,
            apply_to_all: false,
            on_error,
        }
    }

    pub fn conflicts(resolution: Resolution) -> Script {
        Script {
            conflict: Some(resolution),
            apply_to_all: false,
            on_error: ErrorDecision::Cancel,
        }
    }
}

/// Everything a run said, besides the outcome.
#[derive(Default)]
pub struct Transcript {
    pub conflicts: Vec<ConflictPrompt>,
    pub failures: Vec<FailurePrompt>,
    pub progress: Vec<(u64, u64)>,
    pub started: Option<(u64, u64)>,
}

impl Run {
    pub fn next(&self) -> Event {
        self.events
            .recv_timeout(Duration::from_secs(20))
            .expect("the worker went quiet")
    }

    pub fn finish(&self, script: &Script) -> (Outcome, Transcript) {
        let mut transcript = Transcript::default();
        loop {
            match self.next() {
                Event::Conflict(prompt) => {
                    transcript.conflicts.push(prompt);
                    self.handle.answer(Answer::Conflict {
                        resolution: script.conflict.clone().expect("unexpected conflict"),
                        apply_to_all: script.apply_to_all,
                    });
                }
                Event::Failure(prompt) => {
                    transcript.failures.push(prompt);
                    self.handle.answer(Answer::Error(script.on_error));
                }
                Event::Progress {
                    bytes_done,
                    items_done,
                    ..
                } => transcript.progress.push((bytes_done, items_done)),
                Event::Started {
                    total_bytes,
                    total_items,
                } => transcript.started = Some((total_bytes, total_items)),
                Event::Finished(outcome) => return (*outcome, transcript),
                Event::Calculating => {}
            }
        }
    }
}

pub fn sink() -> (mpsc::Sender<Event>, mpsc::Receiver<Event>) {
    mpsc::channel()
}

pub fn start_old(request: Request) -> Run {
    let (tx, events) = sink();
    let handle = spawn(request, move |event| {
        let _ = tx.send(event);
    });
    Run { handle, events }
}

pub fn start(request: LocationRequest, resolver: BackendResolver) -> Run {
    let (tx, events) = sink();
    let handle = spawn_with(request, resolver, move |event| {
        let _ = tx.send(event);
    })
    .expect("the request was refused");
    Run { handle, events }
}

pub fn request(op: Op, sources: Vec<Location>, dest_dir: Location) -> LocationRequest {
    LocationRequest {
        op,
        sources,
        dest_dir,
        confirmed_permanent: op == Op::Delete,
    }
}

/// Runs a request to the end with `script`.
pub fn run(request: LocationRequest, resolver: &BackendResolver, script: &Script) -> (Outcome, Transcript) {
    start(request, Arc::clone(resolver)).finish(script)
}

pub fn drive(name: &str) -> DriveId {
    DriveId::new("mem", name).expect("valid drive id")
}

pub fn rpath(path: &str) -> RemotePath {
    RemotePath::parse(path).expect("valid remote path")
}

pub fn rloc(drive: &DriveId, path: &str) -> Location {
    Location::Remote {
        drive: drive.clone(),
        path: rpath(path),
    }
}

pub fn uri(drive: &DriveId, path: &str) -> PathBuf {
    PathBuf::from(rloc(drive, path).to_uri().expect("uri"))
}

/// A resolver serving the given drives.
pub fn resolver(drives: Vec<(DriveId, Arc<dyn Backend>)>) -> BackendResolver {
    let map: BTreeMap<DriveId, Arc<dyn Backend>> = drives.into_iter().collect();
    Arc::new(move |id: &DriveId| map.get(id).cloned())
}

/// Writes a file, creating its parent folders on a drive that has real ones.
pub fn put(backend: &dyn Backend, path: &str, content: &[u8]) {
    if backend.capabilities().real_directories {
        let mut dir = rpath(path).parent();
        let mut missing = Vec::new();
        while let Some(parent) = dir {
            if parent.is_root() || backend.stat(&parent).is_ok() {
                break;
            }
            dir = parent.parent();
            missing.push(parent);
        }
        for parent in missing.into_iter().rev() {
            backend.create_dir(&parent).expect("create parent");
        }
    }
    let mut session = backend
        .begin_write(&rpath(path), Some(content.len() as u64), false)
        .expect("begin_write");
    session.write_all(content).expect("write");
    session.finish().expect("finish");
}

pub fn mkdir(backend: &dyn Backend, path: &str) {
    backend.create_dir(&rpath(path)).expect("create_dir");
}

pub fn content(memory: &MemoryBackend, path: &str) -> Option<Vec<u8>> {
    match memory.snapshot().expect("snapshot").nodes.get(&rpath(path)) {
        Some(MemoryNode::File { content }) => Some(content.clone()),
        _ => None,
    }
}

pub fn exists(backend: &dyn Backend, path: &str) -> bool {
    backend.stat(&rpath(path)).is_ok()
}

/// Every committed key of a memory drive.
pub fn keys(memory: &MemoryBackend) -> Vec<String> {
    memory
        .snapshot()
        .expect("snapshot")
        .nodes
        .keys()
        .map(|key| key.as_str().to_string())
        .collect()
}

/// Bytes of `n` that are not all the same, so a truncation shows.
pub fn bytes(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

/// Points the trash at a private folder (see `tests/runner.rs`). Set once,
/// before any worker thread reads it.
pub fn isolate_trash() {
    use std::sync::OnceLock;
    static HOME: OnceLock<TempDir> = OnceLock::new();
    HOME.get_or_init(|| {
        let dir = TempDir::new().expect("tempdir");
        unsafe { std::env::set_var("XDG_DATA_HOME", dir.path()) };
        dir
    });
}

pub fn local_world() -> (TempDir, PathBuf, PathBuf) {
    isolate_trash();
    let root = TempDir::new().expect("tempdir");
    let src = root.path().join("src");
    let dst = root.path().join("dst");
    fs::create_dir(&src).expect("src");
    fs::create_dir(&dst).expect("dst");
    (root, src, dst)
}

pub fn write(path: &Path, content: &[u8]) {
    fs::write(path, content).expect("write");
}

/// A backend that lies about the size `stat` reports for paths under `lie_under`.
pub struct LyingStat {
    pub inner: Arc<MemoryBackend>,
    pub lie_under: RemotePath,
    pub delta: u64,
}

impl Backend for LyingStat {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        self.inner.list(dir, cancel)
    }
    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        let mut entry = self.inner.stat(path)?;
        if path.starts_with(&self.lie_under) {
            entry.size = entry.size.map(|size| size + self.delta);
        }
        Ok(entry)
    }
    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn std::io::Read + Send>, BackendError> {
        self.inner.open_read(path, from)
    }
    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        self.inner.begin_write(path, size_hint, replace)
    }
    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.inner.remove(path)
    }
    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        self.inner.remove_tree(path, cancel)
    }
    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.inner.copy_within(from, to)
    }
}

/// Records which backend methods were called, by name, around a memory drive.
/// Sessions are wrapped too: they log `finish`, `abort` and `dropped` (a
/// session dropped without either).
pub struct Spy {
    pub inner: Arc<dyn Backend>,
    pub calls: Arc<Mutex<Vec<&'static str>>>,
}

impl Spy {
    pub fn new(inner: Arc<dyn Backend>) -> Spy {
        Spy {
            inner,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn note(&self, name: &'static str) {
        self.calls.lock().expect("spy").push(name);
    }

    pub fn called(&self, name: &str) -> usize {
        self.calls
            .lock()
            .expect("spy")
            .iter()
            .filter(|call| **call == name)
            .count()
    }
}

struct SpySession {
    inner: Option<Box<dyn WriteSession>>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl std::io::Write for SpySession {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.inner.as_mut() {
            Some(inner) => inner.write(buf),
            None => Err(std::io::Error::other("closed")),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl WriteSession for SpySession {
    fn finish(mut self: Box<Self>) -> Result<(), BackendError> {
        self.calls.lock().expect("spy").push("finish");
        match self.inner.take() {
            Some(inner) => inner.finish(),
            None => Ok(()),
        }
    }
    fn abort(mut self: Box<Self>) -> Result<(), BackendError> {
        self.calls.lock().expect("spy").push("abort");
        match self.inner.take() {
            Some(inner) => inner.abort(),
            None => Ok(()),
        }
    }
}

impl Drop for SpySession {
    fn drop(&mut self) {
        if self.inner.is_some()
            && let Ok(mut calls) = self.calls.lock()
        {
            calls.push("dropped");
        }
    }
}

impl Backend for Spy {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        self.note("list");
        self.inner.list(dir, cancel)
    }
    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        self.note("stat");
        self.inner.stat(path)
    }
    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn std::io::Read + Send>, BackendError> {
        self.note("open_read");
        self.inner.open_read(path, from)
    }
    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        self.note(if replace { "begin_write_replace" } else { "begin_write" });
        let inner = self.inner.begin_write(path, size_hint, replace)?;
        Ok(Box::new(SpySession {
            inner: Some(inner),
            calls: Arc::clone(&self.calls),
        }))
    }
    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.note("create_dir");
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.note("rename");
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.note("remove");
        self.inner.remove(path)
    }
    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        self.note("remove_tree");
        self.inner.remove_tree(path, cancel)
    }
    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.note("copy_within");
        self.inner.copy_within(from, to)
    }
}

/// A backend whose readers stop once, after `after` bytes, until the test
/// opens the gate: a copy can be caught exactly in the middle.
pub struct Gated {
    pub inner: Arc<dyn Backend>,
    pub after: u64,
    reached: Mutex<mpsc::Sender<()>>,
    open: Arc<Mutex<mpsc::Receiver<()>>>,
}

/// The test's side of a [`Gated`] backend.
pub struct Gate {
    pub reached: mpsc::Receiver<()>,
    pub open: mpsc::Sender<()>,
}

impl Gated {
    pub fn new(inner: Arc<dyn Backend>, after: u64) -> (Gated, Gate) {
        let (reached_tx, reached_rx) = mpsc::channel();
        let (open_tx, open_rx) = mpsc::channel();
        (
            Gated {
                inner,
                after,
                reached: Mutex::new(reached_tx),
                open: Arc::new(Mutex::new(open_rx)),
            },
            Gate {
                reached: reached_rx,
                open: open_tx,
            },
        )
    }
}

impl Gate {
    /// Waits until a reader is held at the gate.
    pub fn wait(&self) {
        self.reached
            .recv_timeout(Duration::from_secs(20))
            .expect("no reader reached the gate");
    }

    pub fn release(&self) {
        let _ = self.open.send(());
    }
}

struct GatedReader {
    inner: Box<dyn std::io::Read + Send>,
    left: Option<u64>,
    reached: mpsc::Sender<()>,
    open: Arc<Mutex<mpsc::Receiver<()>>>,
}

impl std::io::Read for GatedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.left {
            Some(0) => {
                self.left = None;
                let _ = self.reached.send(());
                let _ = self
                    .open
                    .lock()
                    .expect("gate")
                    .recv_timeout(Duration::from_secs(20));
                self.inner.read(buf)
            }
            Some(left) => {
                let cap = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
                let n = self.inner.read(&mut buf[..cap])?;
                self.left = Some(left - n as u64);
                Ok(n)
            }
            None => self.inner.read(buf),
        }
    }
}

impl Backend for Gated {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        self.inner.list(dir, cancel)
    }
    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        self.inner.stat(path)
    }
    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn std::io::Read + Send>, BackendError> {
        let inner = self.inner.open_read(path, from)?;
        Ok(Box::new(GatedReader {
            inner,
            left: Some(self.after),
            reached: self.reached.lock().expect("gate").clone(),
            open: Arc::clone(&self.open),
        }))
    }
    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        self.inner.begin_write(path, size_hint, replace)
    }
    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.inner.remove(path)
    }
    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        self.inner.remove_tree(path, cancel)
    }
    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.inner.copy_within(from, to)
    }
}
