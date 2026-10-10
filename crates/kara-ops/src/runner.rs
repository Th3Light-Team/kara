//! Running a copy or a move on a worker thread, asking the user when it has to.
//!
//! Reference conveniences: `ground/spec/05-operaciones.md` — «Diálogo de
//! progreso», «Resolución de conflictos», «Aplicar la misma acción a todos» and
//! «Manejo de errores con reintentar / omitir / cancelar».
//!
//! # Shape
//!
//! [`spawn`] starts a thread and returns a [`Handle`]. The thread talks to the
//! outside through a sink closure ([`Event`]s going out) and the handle
//! ([`Answer`]s coming back). Nothing here knows about Qt: the UI's sink queues
//! each event onto the GUI thread, and the tests' sink is a channel.
//!
//! # Decisions
//!
//! - **«Calculando…» comes first.** The tree is measured before a byte moves, so
//!   the progress has a total and an honest ETA from the start.
//! - **Replace never destroys.** The thing being replaced goes to the FreeDesktop
//!   trash first and the undo stack remembers it, so even a confirmed
//!   «Reemplazar» can be taken back.
//! - **A failure never aborts the batch.** It asks (retry / skip / cancel); skip
//!   moves on and the final [`Outcome`] lists everything that did not happen.
//! - **A move that copied only part is never finished by deleting the source.**
//!   Across volumes every file is copied, then removed, one at a time; a file
//!   that failed to copy stays where it was.
//! - **No `unwrap`/`expect` on the way.** Every syscall result is handled.
//!
//! # Locations
//!
//! [`spawn_with`] takes a [`LocationRequest`], whose items may live on remote
//! drives. An all-local request is handed to [`spawn`] unchanged; anything
//! else runs in `remote`, with the same events, questions, batch policy and
//! undo records. See [`crate::location`].

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use kara_fs::trash::{Flow as WalkFlow, TrashError, TrashObserver, delete_permanently, trash_one};

use crate::batch::{BatchPolicy, BatchReport, ErrorDecision, Failure, FailureKind};
use crate::clock::trash_policy;
use crate::conflict::{ConflictDecisions, ConflictKind, Resolution, ResolutionCounts};
use crate::location::{BackendResolver, LocationRequest, RequestError};
use crate::undo::Action;

mod remote;

/// Chunk size of the byte loop. Big enough for throughput, small enough that a
/// cancel is felt within a few milliseconds even on a slow disk.
const CHUNK: usize = 256 * 1024;

/// Minimum gap between two progress events.
const PROGRESS_EVERY: Duration = Duration::from_millis(80);

/// How often a worker blocked on a question looks at the cancel flag.
const ANSWER_POLL: Duration = Duration::from_millis(100);

/// How often a paused worker looks at the pause and cancel flags.
const PAUSE_POLL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Copy,
    Move,
    /// Permanent deletion: no destination, nothing to undo.
    Delete,
}

impl Op {
    /// The verb for a title: «Copiando», «Moviendo».
    #[must_use]
    pub fn gerund(self) -> &'static str {
        match self {
            Self::Copy => "Copiando",
            Self::Move => "Moviendo",
            Self::Delete => "Eliminando",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub op: Op,
    pub sources: Vec<PathBuf>,
    pub dest_dir: PathBuf,
}

/// What the worker tells the outside.
#[derive(Debug, Clone)]
pub enum Event {
    /// Measuring the sources. Nothing has been touched yet.
    Calculating,
    Started {
        total_bytes: u64,
        total_items: u64,
    },
    Progress {
        current: String,
        bytes_done: u64,
        items_done: u64,
    },
    /// The worker is blocked until [`Handle::answer`] gets an
    /// [`Answer::Conflict`].
    Conflict(ConflictPrompt),
    /// The worker is blocked until it gets an [`Answer::Error`].
    Failure(FailurePrompt),
    Finished(Box<Outcome>),
}

#[derive(Debug, Clone)]
pub struct ConflictPrompt {
    pub kind: ConflictKind,
    pub name: String,
    pub source: PathBuf,
    pub destination: PathBuf,
}

#[derive(Debug, Clone)]
pub struct FailurePrompt {
    pub path: PathBuf,
    pub reason: String,
    pub kind: FailureKind,
}

/// What the outside tells the worker.
#[derive(Debug, Clone)]
pub enum Answer {
    Conflict {
        resolution: Resolution,
        apply_to_all: bool,
    },
    Error(ErrorDecision),
}

/// Everything that happened, for the final summary and the undo stack.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub op: Op,
    /// Oldest first. Replacing pushes the trashing before the transfer, so
    /// undoing walks back through them in the right order.
    pub actions: Vec<Action>,
    pub report: BatchReport,
    pub resolutions: ResolutionCounts,
    pub cancelled: bool,
}

/// The caller's side of a running job.
#[derive(Debug, Clone)]
pub struct Handle {
    answers: Sender<Answer>,
    cancel: Arc<AtomicBool>,
    /// The same cancel, as the token a backend call watches.
    token: kara_vfs::Cancel,
    paused: Arc<AtomicBool>,
}

impl Handle {
    /// Replies to the question the worker is blocked on. A reply of the wrong
    /// kind is treated as a cancel by the worker rather than guessed at.
    pub fn answer(&self, answer: Answer) {
        let _ = self.answers.send(answer);
    }

    /// Stops after the file in flight, leaving what was already done as it is.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.token.cancel();
    }

    /// Holds the transfer at the next chunk boundary. A cancel still works
    /// while paused.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
}

/// Builds the handle and the worker's ends of it.
fn channels() -> (Handle, Receiver<Answer>) {
    let (answers_tx, answers_rx) = unbounded();
    let handle = Handle {
        answers: answers_tx,
        cancel: Arc::new(AtomicBool::new(false)),
        token: kara_vfs::Cancel::new(),
        paused: Arc::new(AtomicBool::new(false)),
    };
    (handle, answers_rx)
}

/// Starts the job. Every event goes through `sink`, on the worker thread.
pub fn spawn(request: Request, sink: impl Fn(Event) + Send + 'static) -> Handle {
    let (handle, answers_rx) = channels();
    let worker_handle = handle.clone();

    std::thread::spawn(move || {
        let mut worker = Worker::new(request, Box::new(sink), answers_rx, &worker_handle);
        worker.run();
    });
    handle
}

/// Starts a job over [`kara_vfs::Location`]s. Every event goes through `sink`,
/// on the worker thread, exactly as with [`spawn`].
///
/// Refused before anything is touched when a delete is not confirmed
/// ([`LocationRequest::confirmed_permanent`]) or a drive does not resolve. An
/// all-local request is run by [`spawn`] itself, so its behaviour is the old
/// one bit for bit.
pub fn spawn_with(
    request: LocationRequest,
    resolver: BackendResolver,
    sink: impl Fn(Event) + Send + 'static,
) -> Result<Handle, RequestError> {
    if request.op == Op::Delete && !request.confirmed_permanent {
        return Err(RequestError::PermanentDeleteNotConfirmed);
    }
    let destination = (request.op != Op::Delete).then_some(&request.dest_dir);
    for location in request.sources.iter().chain(destination) {
        if let Some(drive) = location.drive()
            && resolver(drive).is_none()
        {
            return Err(RequestError::DriveUnavailable(drive.clone()));
        }
    }
    if let Some(local) = request.to_local() {
        return Ok(spawn(local, sink));
    }

    let (handle, answers_rx) = channels();
    let worker_handle = handle.clone();
    std::thread::spawn(move || {
        let legacy = Request {
            op: request.op,
            sources: Vec::new(),
            dest_dir: PathBuf::new(),
        };
        let mut worker = Worker::new(legacy, Box::new(sink), answers_rx, &worker_handle);
        worker.job = Some(remote::LocationJob {
            sources: request.sources,
            dest_dir: request.dest_dir,
            resolver,
        });
        worker.run();
    });
    Ok(handle)
}

/// Carries a [`delete_permanently`] walk's progress and cancel into the worker.
struct DeleteObserver<'a> {
    worker: &'a mut Worker,
    base: u64,
    name: String,
}

impl TrashObserver for DeleteObserver<'_> {
    fn on_bytes(&mut self, removed: u64, _total: Option<u64>) -> WalkFlow {
        self.worker.items_done = self.base + removed;
        self.worker.tick(self.name.clone(), false);
        if self.worker.cancelled() {
            WalkFlow::Cancel
        } else {
            WalkFlow::Continue
        }
    }
}

/// A trash error as the `io::Error` the failure policy understands, keeping the
/// kind so a permission problem is not classified as «other».
fn trash_error_to_io(error: TrashError) -> io::Error {
    use io::ErrorKind;
    let kind = match &error {
        TrashError::PermissionDenied { .. } => ErrorKind::PermissionDenied,
        TrashError::NotFound { .. } => ErrorKind::NotFound,
        TrashError::Io { source, .. }
        | TrashError::InfoWrite { source, .. } => source.kind(),
        TrashError::Cancelled => ErrorKind::Interrupted,
        _ => ErrorKind::Other,
    };
    io::Error::new(kind, error.to_string())
}

/// A node that stops the walk: only a cancel does.
struct Cancelled;

/// How a node ended, when it did not cancel.
#[derive(PartialEq, Eq)]
enum Flow {
    /// Everything under it is in place.
    Done,
    /// Something under it was skipped, so a move must not remove the source.
    Partial,
}

type Step = Result<Flow, Cancelled>;

struct Worker {
    op: Op,
    sources: Vec<PathBuf>,
    dest_dir: PathBuf,
    sink: Box<dyn Fn(Event) + Send>,
    answers: Receiver<Answer>,
    cancel: Arc<AtomicBool>,
    decisions: ConflictDecisions,
    policy: BatchPolicy,
    actions: Vec<Action>,
    bytes_done: u64,
    items_done: u64,
    last_emit: Instant,
    /// Set by [`spawn_with`] for a request that involves a remote drive.
    job: Option<remote::LocationJob>,
    token: kara_vfs::Cancel,
    paused: Arc<AtomicBool>,
    /// Sources already copied by a folder move across backends, children
    /// before parents, waiting for the whole folder to arrive. The flag says
    /// whether it is a folder.
    copied_sources: Vec<(crate::location::Endpoint, bool)>,
}

impl Worker {
    fn new(
        request: Request,
        sink: Box<dyn Fn(Event) + Send>,
        answers: Receiver<Answer>,
        handle: &Handle,
    ) -> Self {
        Self {
            op: request.op,
            sources: request.sources,
            dest_dir: request.dest_dir,
            sink,
            answers,
            cancel: Arc::clone(&handle.cancel),
            decisions: ConflictDecisions::new(),
            policy: BatchPolicy::new(),
            actions: Vec::new(),
            bytes_done: 0,
            items_done: 0,
            last_emit: Instant::now(),
            job: None,
            token: handle.token.clone(),
            paused: Arc::clone(&handle.paused),
            copied_sources: Vec::new(),
        }
    }

    fn emit(&self, event: Event) {
        (self.sink)(event);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Blocks while the job is paused, until it is resumed or cancelled.
    fn wait_while_paused(&self) {
        while self.paused.load(Ordering::SeqCst) && !self.cancelled() {
            std::thread::sleep(PAUSE_POLL);
        }
    }

    fn run(&mut self) {
        if let Some(job) = self.job.take() {
            self.run_locations(job);
            return;
        }
        self.emit(Event::Calculating);

        let mut total_bytes = 0;
        let mut total_items = 0;
        for source in self.sources.clone() {
            if self.cancelled() {
                break;
            }
            match measure(&source) {
                Ok((items, bytes)) => {
                    total_items += items;
                    total_bytes += bytes;
                }
                // A source that cannot even be measured is reported when its
                // turn comes; the totals just come out a little short.
                Err(_) => total_items += 1,
            }
        }
        self.emit(Event::Started {
            total_bytes,
            total_items,
        });

        for source in self.sources.clone() {
            if self.cancelled() {
                break;
            }
            let Some(name) = source.file_name() else {
                let failure = Failure {
                    path: source.clone(),
                    kind: FailureKind::Other,
                    reason: "no tiene un nombre usable".to_string(),
                };
                self.policy.record(failure, ErrorDecision::Skip);
                continue;
            };
            let outcome = if self.op == Op::Delete {
                self.delete_node(&source)
            } else {
                let destination = self.dest_dir.join(name);
                self.node(&source, destination, true)
            };
            if outcome.is_err() {
                // Whatever stopped it, the summary has to say it stopped.
                self.cancel.store(true, Ordering::SeqCst);
                break;
            }
        }

        self.finish();
    }

    /// The last progress and the summary.
    fn finish(&mut self) {
        self.tick(String::new(), true);
        let cancelled = self.cancelled() || self.policy.is_cancelled();
        let report = BatchReport {
            cancelled,
            ..self.policy.report()
        };
        let outcome = Outcome {
            op: self.op,
            actions: std::mem::take(&mut self.actions),
            report,
            resolutions: self.decisions.counts(),
            cancelled,
        };
        self.emit(Event::Finished(Box::new(outcome)));
    }

    /// Registers a finished copy or move for undo. Deleting registers nothing.
    fn record_transfer(&mut self, source: &Path, destination: &Path) {
        match self.op {
            Op::Copy => self.actions.push(Action::Copied {
                created: destination.to_path_buf(),
            }),
            Op::Move => self.actions.push(Action::Moved {
                from: source.to_path_buf(),
                to: destination.to_path_buf(),
            }),
            Op::Delete => {}
        }
    }

    /// Permanently deletes one top-level item, with a live count and a cancel.
    ///
    /// The refusals live in `kara_fs::trash::delete_permanently` (the root and
    /// mount points); this only drives it. A failure asks, like everywhere
    /// else, and a skipped item stays exactly where it was.
    fn delete_node(&mut self, path: &Path) -> Step {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let before = self.items_done;
        let removed = self.guarded(path, |worker| {
            let mut observer = DeleteObserver {
                worker,
                base: before,
                name: name.clone(),
            };
            delete_permanently(path, &mut observer).map_err(trash_error_to_io)
        })?;
        match removed {
            Some(count) => {
                self.items_done = before + count;
                Ok(Flow::Done)
            }
            None => {
                self.count_skipped(path);
                Ok(Flow::Partial)
            }
        }
    }

    /// Emits progress, at most every [`PROGRESS_EVERY`] unless `force`.
    fn tick(&mut self, current: String, force: bool) {
        if !force && self.last_emit.elapsed() < PROGRESS_EVERY {
            return;
        }
        self.last_emit = Instant::now();
        self.emit(Event::Progress {
            current,
            bytes_done: self.bytes_done,
            items_done: self.items_done,
        });
    }

    /// Blocks for the answer to a question, giving up when the job is
    /// cancelled or nobody can answer any more.
    fn wait_for_answer(&self) -> Option<Answer> {
        loop {
            if self.cancelled() {
                return None;
            }
            match self.answers.recv_timeout(ANSWER_POLL) {
                Ok(answer) => return Some(answer),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    /// Runs `attempt` until it succeeds or the user stops asking for retries.
    ///
    /// `Ok(true)` — it worked. `Ok(false)` — skipped, and the batch goes on.
    /// `Err(Cancelled)` — stop.
    fn guarded<T>(
        &mut self,
        path: &Path,
        mut attempt: impl FnMut(&mut Self) -> io::Result<T>,
    ) -> Result<Option<T>, Cancelled> {
        loop {
            if self.cancelled() {
                return Err(Cancelled);
            }
            let error = match attempt(self) {
                Ok(value) => {
                    self.policy.record_success();
                    return Ok(Some(value));
                }
                Err(error) => error,
            };
            // An error that is only the cancel being noticed is not a failure
            // to ask about.
            if self.cancelled() {
                return Err(Cancelled);
            }
            let failure = Failure {
                path: path.to_path_buf(),
                kind: classify(&error),
                reason: error.to_string(),
            };
            if !self.settle(failure)? {
                return Ok(None);
            }
        }
    }

    /// Asks (or applies the standing decision) about one failure.
    ///
    /// `Ok(true)` — retry. `Ok(false)` — skipped, recorded. `Err(Cancelled)` —
    /// stop, recorded.
    fn settle(&mut self, failure: Failure) -> Result<bool, Cancelled> {
        let decision = match self.policy.decide(&failure) {
            Some(decision) => decision,
            None => {
                self.emit(Event::Failure(FailurePrompt {
                    path: failure.path.clone(),
                    reason: failure.reason.clone(),
                    kind: failure.kind,
                }));
                match self.wait_for_answer() {
                    Some(Answer::Error(decision)) => decision,
                    _ => ErrorDecision::Cancel,
                }
            }
        };
        if decision == ErrorDecision::SkipAll {
            self.policy.apply_to_all(decision);
        }
        match decision {
            ErrorDecision::Retry => Ok(true),
            ErrorDecision::Skip | ErrorDecision::SkipAll => {
                self.policy.record(failure, decision);
                Ok(false)
            }
            ErrorDecision::Cancel => {
                self.policy.record(failure, decision);
                Err(Cancelled)
            }
        }
    }

    /// Counts a subtree as done without touching it: what a skip does to the
    /// progress, so the bar still reaches the end.
    fn count_skipped(&mut self, source: &Path) {
        if let Ok((items, bytes)) = measure(source) {
            self.items_done += items;
            self.bytes_done += bytes;
        } else {
            self.items_done += 1;
        }
    }

    /// Copies or moves one node and everything under it onto `destination`.
    ///
    /// `record` says whether a freshly created node is registered for undo. It
    /// is `false` below a node that was itself created fresh: undoing the
    /// parent already takes the children with it.
    fn node(&mut self, source: &Path, mut destination: PathBuf, record: bool) -> Step {
        let Some(meta) = self.guarded(source, |_| fs::symlink_metadata(source))? else {
            self.count_skipped(source);
            return Ok(Flow::Partial);
        };
        let source_is_dir = meta.is_dir();
        let name = source
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());

        if into_itself(source, &destination, source_is_dir) {
            let failure = Failure {
                path: source.to_path_buf(),
                kind: FailureKind::Other,
                reason: "una carpeta no se puede copiar ni mover dentro de sí misma".to_string(),
            };
            self.policy.record(failure, ErrorDecision::Skip);
            self.count_skipped(source);
            return Ok(Flow::Partial);
        }

        // Pasting where the item already is: a copy becomes «name (2)», a move
        // has nothing to do.
        if source == destination {
            if self.op == Op::Move {
                self.count_skipped(source);
                return Ok(Flow::Done);
            }
            destination = sibling_name(&destination);
        }

        let mut merge = false;
        let mut already_there = true;
        while already_there {
            let Some(dest_meta) = fs::symlink_metadata(&destination).ok() else {
                already_there = false;
                continue;
            };
            let kind = match (source_is_dir, dest_meta.is_dir()) {
                (false, false) => ConflictKind::FileOverFile,
                (false, true) => ConflictKind::FileOverDirectory,
                (true, false) => ConflictKind::DirectoryOverFile,
                (true, true) => ConflictKind::DirectoryOverDirectory,
            };
            let resolution = match self.decisions.decide(kind) {
                Some(known) => known.clone(),
                None => {
                    self.emit(Event::Conflict(ConflictPrompt {
                        kind,
                        name: name.clone(),
                        source: source.to_path_buf(),
                        destination: destination.clone(),
                    }));
                    match self.wait_for_answer() {
                        Some(Answer::Conflict {
                            resolution,
                            apply_to_all,
                        }) => {
                            if apply_to_all {
                                self.decisions.apply_to_all(kind, resolution.clone());
                            }
                            resolution
                        }
                        _ => return Err(Cancelled),
                    }
                }
            };
            if !kind.allows(&resolution) {
                // The dialog never offers it; a stale or wrong answer is not
                // worth guessing about.
                return Err(Cancelled);
            }
            self.decisions.record(&resolution);

            match resolution {
                Resolution::Skip => {
                    self.count_skipped(source);
                    return Ok(Flow::Partial);
                }
                Resolution::KeepBoth => {
                    destination = sibling_name(&destination);
                    already_there = false;
                }
                Resolution::RenameTo(new_name) => {
                    destination = destination.with_file_name(new_name);
                }
                Resolution::Merge => {
                    merge = true;
                    already_there = false;
                }
                Resolution::Replace => {
                    let target = destination.clone();
                    let trashed = self.guarded(&target, |_| {
                        trash_one(&target, &trash_policy()).map_err(trash_error_to_io)
                    })?;
                    match trashed {
                        Some(item) => {
                            self.actions.push(Action::Trashed {
                                item: Box::new(item),
                            });
                            already_there = false;
                        }
                        None => {
                            self.count_skipped(source);
                            return Ok(Flow::Partial);
                        }
                    }
                }
            }
        }

        self.tick(name, false);

        if source_is_dir {
            self.directory(source, &destination, merge, record)
        } else {
            self.leaf(source, &destination, &meta, record)
        }
    }

    /// A folder: created (or merged into), then its children, then — for a
    /// move — removed from the source if nothing was left behind.
    fn directory(&mut self, source: &Path, destination: &Path, merge: bool, record: bool) -> Step {
        // Same volume: one rename moves the whole tree, children and all.
        if self.op == Op::Move && !merge && same_volume(source, destination) {
            let renamed = self.guarded(source, |_| fs::rename(source, destination));
            match renamed {
                Ok(Some(())) => {
                    self.count_skipped(source);
                    if record {
                        self.actions.push(Action::Moved {
                            from: source.to_path_buf(),
                            to: destination.to_path_buf(),
                        });
                    }
                    return Ok(Flow::Done);
                }
                Ok(None) => {
                    self.count_skipped(source);
                    return Ok(Flow::Partial);
                }
                Err(cancel) => return Err(cancel),
            }
        }

        if !merge {
            let made = self.guarded(destination, |_| fs::create_dir(destination))?;
            if made.is_none() {
                self.count_skipped(source);
                return Ok(Flow::Partial);
            }
            if record {
                self.record_transfer(source, destination);
            }
        }
        self.items_done += 1;

        let children = match self.guarded(source, |_| {
            fs::read_dir(source)?
                .map(|entry| entry.map(|e| e.file_name()))
                .collect::<io::Result<Vec<_>>>()
        })? {
            Some(mut names) => {
                names.sort();
                names
            }
            None => return Ok(Flow::Partial),
        };

        let mut whole = Flow::Done;
        for child in children {
            // Under a merge each child may be new to the destination, so each
            // is registered; under a fresh folder the folder covers them.
            let flow = self.node(&source.join(&child), destination.join(&child), merge)?;
            if flow == Flow::Partial {
                whole = Flow::Partial;
            }
        }

        if self.op == Op::Move && whole == Flow::Done {
            // Empty by now unless something else wrote into it meanwhile; a
            // failure to remove it loses nothing.
            let _ = fs::remove_dir(source);
        }
        Ok(whole)
    }

    /// A file or a symlink.
    fn leaf(&mut self, source: &Path, destination: &Path, meta: &fs::Metadata, record: bool) -> Step {
        let size = meta.len();
        let is_link = meta.file_type().is_symlink();

        if self.op == Op::Move && same_volume(source, destination) {
            let renamed = self.guarded(source, |_| fs::rename(source, destination));
            match renamed {
                Ok(Some(())) => {
                    self.items_done += 1;
                    self.bytes_done += size;
                    if record {
                        self.actions.push(Action::Moved {
                            from: source.to_path_buf(),
                            to: destination.to_path_buf(),
                        });
                    }
                    return Ok(Flow::Done);
                }
                Ok(None) => {
                    self.count_skipped(source);
                    return Ok(Flow::Partial);
                }
                Err(cancel) => return Err(cancel),
            }
        }

        let copied = if is_link {
            self.guarded(source, |_| {
                let target = fs::read_link(source)?;
                std::os::unix::fs::symlink(target, destination)
            })?
        } else {
            self.guarded(source, |worker| worker.copy_bytes(source, destination, meta))?
        };
        if copied.is_none() {
            self.count_skipped(source);
            return Ok(Flow::Partial);
        }
        self.items_done += 1;
        if is_link {
            self.bytes_done += size;
        }

        if record {
            self.record_transfer(source, destination);
        }

        if self.op == Op::Move {
            // Only now, with the copy complete. If removing the original fails
            // the user has two copies, which is the safe way to be wrong.
            let removed = self.guarded(source, |_| fs::remove_file(source))?;
            if removed.is_none() {
                return Ok(Flow::Partial);
            }
        }
        Ok(Flow::Done)
    }

    /// Copies a file chunk by chunk so progress moves and a cancel is heard.
    /// The destination is created exclusively: if something appeared there
    /// since the conflict check, the copy fails instead of overwriting it.
    fn copy_bytes(&mut self, source: &Path, destination: &Path, meta: &fs::Metadata) -> io::Result<()> {
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;

        let before = self.bytes_done;
        let mut buffer = vec![0u8; CHUNK];
        let result = (|| -> io::Result<()> {
            loop {
                self.wait_while_paused();
                if self.cancelled() {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelado"));
                }
                let read = input.read(&mut buffer)?;
                if read == 0 {
                    return Ok(());
                }
                output.write_all(&buffer[..read])?;
                self.bytes_done += read as u64;
                self.tick(
                    source
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                    false,
                );
            }
        })();

        match result {
            Ok(()) => {
                output.set_permissions(meta.permissions())?;
                if let Ok(modified) = meta.modified() {
                    // Not worth failing a finished copy over.
                    let _ = output.set_modified(modified);
                }
                Ok(())
            }
            Err(error) => {
                // A half-written file is worse than none. Retrying starts over,
                // so the counter goes back too.
                drop(output);
                let _ = fs::remove_file(destination);
                self.bytes_done = before;
                Err(error)
            }
        }
    }
}

/// `(nodes, bytes)` under `path`, links not followed.
fn measure(path: &Path) -> io::Result<(u64, u64)> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return Ok((1, meta.len()));
    }
    let mut items = 1;
    let mut bytes = 0;
    for entry in fs::read_dir(path)? {
        // One unreadable child must not hide the rest of the total.
        if let Ok(entry) = entry
            && let Ok((i, b)) = measure(&entry.path())
        {
            items += i;
            bytes += b;
        }
    }
    Ok((items, bytes))
}

fn classify(error: &io::Error) -> FailureKind {
    match error.kind() {
        io::ErrorKind::PermissionDenied => FailureKind::PermissionDenied,
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => FailureKind::NoSpace,
        io::ErrorKind::ResourceBusy | io::ErrorKind::ExecutableFileBusy => FailureKind::InUse,
        _ => FailureKind::Other,
    }
}

/// `dir/name` → `dir/name (2)`, the first free one.
fn sibling_name(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let free = kara_core::unique_name(&name, |candidate| {
        fs::symlink_metadata(parent.join(candidate)).is_ok()
    });
    parent.join(free)
}

/// Whether `destination` lies inside the directory `source`.
fn into_itself(source: &Path, destination: &Path, source_is_dir: bool) -> bool {
    if !source_is_dir {
        return false;
    }
    let (Ok(from), Some(parent)) = (source.canonicalize(), destination.parent()) else {
        return false;
    };
    parent
        .canonicalize()
        .is_ok_and(|to| to == from || to.starts_with(&from))
}

/// Whether a rename from `source` to `destination` stays on one device. The
/// destination does not exist yet, so its parent is the one asked.
fn same_volume(source: &Path, destination: &Path) -> bool {
    let (Ok(from), Some(parent)) = (fs::symlink_metadata(source), destination.parent()) else {
        return false;
    };
    fs::metadata(parent).is_ok_and(|to| to.dev() == from.dev())
}
