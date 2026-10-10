//! Reading and writing file contents over SFTP.
//!
//! Both keep several requests in flight ([`IN_FLIGHT`] × [`CHUNK`] bytes), which
//! is what makes SFTP usable over a link with latency.
//!
//! **Writes** go to a hidden temporary sibling, `.<name>.<pid>.<n>.kara-part`,
//! created with `CREATE | EXCLUDE`. [`WriteSession::finish`] waits until the
//! server has acknowledged **every** byte, asks for `fsync@openssh.com` when
//! the server offers it, closes the handle, and only then renames:
//!
//! - `replace = false`: plain `SSH_FXP_RENAME`, which OpenSSH implements as
//!   `link` + `unlink` for files, so it never replaces anything, even in a race;
//!   a target that appeared meanwhile is reported as `AlreadyExists`;
//! - `replace = true`: `posix-rename@openssh.com` (rename(2), atomic) when the
//!   server offers it; otherwise the old file is removed and the temporary
//!   renamed. **That fallback is not atomic**: for a moment the name is
//!   missing, and a crash in between leaves the old file deleted and the new
//!   one under its temporary name.
//!
//! `abort`, `Drop` and a failed `finish` remove the temporary. A half-written
//! destination is never visible under its final name. When the connection is
//! gone the temporary cannot be removed and stays on the server, hidden.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kara_vfs::{BackendError, BackendErrorKind, RemotePath, WriteSession};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{Data, Status, StatusCode};
use tokio::task::JoinHandle;

use super::error::Fail;
use super::session::Session;

/// Bytes per request: the size every SFTP server must accept.
pub(crate) const CHUNK: usize = 32 * 1024;
/// Requests kept in flight per reader or writer.
pub(crate) const IN_FLIGHT: usize = 16;
/// The longest file name the server is assumed to accept, in bytes.
const NAME_MAX: usize = 255;
/// Suffix that marks a write in progress; the name also starts with a dot.
const PART_SUFFIX: &str = ".kara-part";

/// Makes every temporary name of the process unique, together with the pid.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `.<name>.<pid>.<serial>.kara-part`, with `name` cut to fit `NAME_MAX`.
pub(crate) fn temp_name(name: &str) -> String {
    let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tail = format!(".{}.{serial}{PART_SUFFIX}", std::process::id());
    let room = NAME_MAX.saturating_sub(1 + tail.len());
    let mut cut = name.len().min(room);
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    let kept = name.get(..cut).unwrap_or("");
    format!(".{kept}{tail}")
}

/// Wraps an error so that `BackendError::from_io` gets it back unchanged.
fn io_error(error: BackendError) -> io::Error {
    io::Error::from(error)
}

// ---------------------------------------------------------------------------
// Reading.

type ReadTask = JoinHandle<Result<Data, SftpError>>;

/// A file opened for reading, with read-ahead.
pub(crate) struct SftpReader {
    session: Arc<Session>,
    handle: Option<String>,
    path: RemotePath,
    /// Where the next request starts.
    next: u64,
    /// Requests on their way, in offset order: (offset, length, task).
    queue: VecDeque<(u64, usize, ReadTask)>,
    buf: Vec<u8>,
    pos: usize,
    eof: bool,
    failed: Option<BackendErrorKind>,
}

impl SftpReader {
    pub(crate) fn new(session: Arc<Session>, handle: String, from: u64, path: RemotePath) -> SftpReader {
        SftpReader {
            session,
            handle: Some(handle),
            path,
            next: from,
            queue: VecDeque::new(),
            buf: Vec::new(),
            pos: 0,
            eof: false,
            failed: None,
        }
    }

    fn fail(&mut self, fail: Fail) -> io::Error {
        let error = fail.at(&self.path);
        self.failed = Some(error.kind);
        self.drop_queue();
        io_error(error)
    }

    fn drop_queue(&mut self) {
        for (_, _, task) in self.queue.drain(..) {
            task.abort();
        }
    }

    fn fill_queue(&mut self) -> Result<(), Fail> {
        let Some(handle) = self.handle.clone() else {
            return Err(Fail::Other(String::from("the file is closed")));
        };
        while !self.eof && self.queue.len() < IN_FLIGHT {
            let offset = self.next;
            let sftp = Arc::clone(&self.session.sftp);
            let handle = handle.clone();
            let len = u32::try_from(CHUNK).unwrap_or(u32::MAX);
            let task = self
                .session
                .spawn(async move { sftp.read(handle, offset, len).await })
                .ok_or_else(|| Fail::Lost(String::from("the connection is closed")))?;
            self.queue.push_back((offset, CHUNK, task));
            self.next = offset.saturating_add(CHUNK as u64);
        }
        Ok(())
    }
}

impl Read for SftpReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.pos < self.buf.len() {
                let available = self.buf.get(self.pos..).unwrap_or_default();
                let n = available.len().min(out.len());
                if let (Some(dst), Some(src)) = (out.get_mut(..n), available.get(..n)) {
                    dst.copy_from_slice(src);
                }
                self.pos += n;
                return Ok(n);
            }
            if let Some(kind) = self.failed {
                return Err(io_error(BackendError::new(kind, Some(self.path.clone()))));
            }
            if self.eof && self.queue.is_empty() {
                return Ok(0);
            }
            if let Err(fail) = self.fill_queue() {
                return Err(self.fail(fail));
            }
            let Some((offset, asked, task)) = self.queue.pop_front() else {
                return Ok(0);
            };
            match self.session.join(task) {
                Ok(data) if data.data.is_empty() => {
                    self.eof = true;
                    self.drop_queue();
                }
                Ok(data) => {
                    let got = data.data.len();
                    if got < asked {
                        // A short read: what was asked after it starts at the
                        // wrong offset. Ask again from where this one ended.
                        self.drop_queue();
                        self.next = offset.saturating_add(got as u64);
                    }
                    self.buf = data.data;
                    self.pos = 0;
                }
                Err(fail) if fail.is(StatusCode::Eof) => {
                    self.eof = true;
                    self.drop_queue();
                }
                Err(fail) => return Err(self.fail(fail)),
            }
        }
    }
}

impl Drop for SftpReader {
    fn drop(&mut self) {
        self.drop_queue();
        if let Some(handle) = self.handle.take() {
            self.session.close_later(handle);
        }
    }
}

// ---------------------------------------------------------------------------
// Writing.

type WriteTask = JoinHandle<Result<Status, SftpError>>;

/// An upload in progress. See the module documentation.
pub(crate) struct SftpWriteSession {
    session: Arc<Session>,
    target: RemotePath,
    /// Server paths of the temporary and of the final name.
    temp: String,
    destination: String,
    handle: Option<String>,
    replace: bool,
    /// Bytes not sent yet (less than one chunk).
    pending: Vec<u8>,
    /// Where the next chunk goes.
    offset: u64,
    in_flight: VecDeque<WriteTask>,
    poisoned: Option<BackendErrorKind>,
    /// The temporary is gone (renamed or removed): `Drop` has nothing to do.
    done: bool,
}

impl SftpWriteSession {
    pub(crate) fn new(
        session: Arc<Session>,
        target: RemotePath,
        temp: String,
        destination: String,
        handle: String,
        replace: bool,
    ) -> SftpWriteSession {
        SftpWriteSession {
            session,
            target,
            temp,
            destination,
            handle: Some(handle),
            replace,
            pending: Vec::new(),
            offset: 0,
            in_flight: VecDeque::new(),
            poisoned: None,
            done: false,
        }
    }

    fn error(&self, kind: BackendErrorKind) -> BackendError {
        BackendError::new(kind, Some(self.target.clone()))
    }

    /// Records the failure; every later call answers with the same kind.
    fn poison(&mut self, fail: Fail) -> BackendError {
        let error = fail.at(&self.target);
        self.poisoned = Some(error.kind);
        error
    }

    /// Sends `bytes` at the current offset without waiting for the answer;
    /// waits for the oldest request when too many are in flight.
    fn send(&mut self, bytes: Vec<u8>) -> Result<(), BackendError> {
        let Some(handle) = self.handle.clone() else {
            return Err(self.error(BackendErrorKind::Other));
        };
        let len = bytes.len() as u64;
        let offset = self.offset;
        let sftp = Arc::clone(&self.session.sftp);
        let Some(task) = self
            .session
            .spawn(async move { sftp.write(handle, offset, bytes).await })
        else {
            return Err(self.poison(Fail::Lost(String::from("the connection is closed"))));
        };
        self.in_flight.push_back(task);
        self.offset = self.offset.saturating_add(len);
        while self.in_flight.len() > IN_FLIGHT {
            self.wait_oldest()?;
        }
        Ok(())
    }

    fn wait_oldest(&mut self) -> Result<(), BackendError> {
        let Some(task) = self.in_flight.pop_front() else {
            return Ok(());
        };
        match self.session.join(task) {
            Ok(_) => Ok(()),
            Err(fail) => Err(self.poison(fail)),
        }
    }

    /// Sends what is buffered and waits until the server acknowledged every byte.
    fn drain(&mut self) -> Result<(), BackendError> {
        if let Some(kind) = self.poisoned {
            return Err(self.error(kind));
        }
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            self.send(rest)?;
        }
        while !self.in_flight.is_empty() {
            self.wait_oldest()?;
        }
        Ok(())
    }

    fn commit(&mut self) -> Result<(), BackendError> {
        self.drain()?;
        let Some(handle) = self.handle.take() else {
            return Err(self.error(BackendErrorKind::Other));
        };
        if self.session.fsync
            && let Err(fail) = self.session.fsync(&handle)
        {
            self.session.close_later(handle);
            return Err(fail.at(&self.target));
        }
        // Some servers only report a failed write when the handle is closed.
        self.session.close(&handle).map_err(|fail| fail.at(&self.target))?;
        self.rename_into_place()?;
        self.done = true;
        Ok(())
    }

    fn rename_into_place(&self) -> Result<(), BackendError> {
        let session = &self.session;
        let taken = || session.lstat(&self.destination).is_ok();
        let taken_by_directory = || {
            session
                .lstat(&self.destination)
                .is_ok_and(|attrs| attrs.is_dir())
        };
        if !self.replace {
            if taken() {
                return Err(self.error(BackendErrorKind::AlreadyExists));
            }
            return match session.rename(&self.temp, &self.destination) {
                Ok(()) => Ok(()),
                Err(fail) if taken() => Err(fail.with_kind(BackendErrorKind::AlreadyExists, &self.target)),
                Err(fail) => Err(fail.at(&self.target)),
            };
        }
        if session.posix_rename {
            return match session.posix_rename(&self.temp, &self.destination) {
                Ok(()) => Ok(()),
                Err(fail) if taken_by_directory() => {
                    Err(fail.with_kind(BackendErrorKind::AlreadyExists, &self.target))
                }
                Err(fail) => Err(fail.at(&self.target)),
            };
        }
        // No atomic replace on this server: remove, then rename (see module docs).
        match session.lstat(&self.destination) {
            Ok(attrs) if attrs.is_dir() => return Err(self.error(BackendErrorKind::AlreadyExists)),
            Ok(_) => match session.remove(&self.destination) {
                Ok(()) => {}
                Err(fail) if fail.kind() == BackendErrorKind::NotFound => {}
                Err(fail) => return Err(fail.at(&self.target)),
            },
            Err(fail) if fail.kind() == BackendErrorKind::NotFound => {}
            Err(fail) => return Err(fail.at(&self.target)),
        }
        match session.rename(&self.temp, &self.destination) {
            Ok(()) => Ok(()),
            Err(fail) if taken() => Err(fail.with_kind(BackendErrorKind::AlreadyExists, &self.target)),
            Err(fail) => Err(fail.at(&self.target)),
        }
    }

    /// Closes the handle and removes the temporary. `done` only once it is
    /// really gone, so `Drop` tries again after a failed `abort`.
    fn discard(&mut self) -> Result<(), BackendError> {
        for task in self.in_flight.drain(..) {
            task.abort();
        }
        self.pending.clear();
        if self.done {
            return Ok(());
        }
        if let Some(handle) = self.handle.take() {
            // Best effort: what matters is the removal.
            let _ = self.session.close(&handle);
        }
        match self.session.remove(&self.temp) {
            Ok(()) => {
                self.done = true;
                Ok(())
            }
            Err(fail) if fail.kind() == BackendErrorKind::NotFound => {
                self.done = true;
                Ok(())
            }
            Err(fail) => Err(fail.at(&self.target)),
        }
    }
}

impl Write for SftpWriteSession {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(kind) = self.poisoned {
            return Err(io_error(self.error(kind)));
        }
        if self.handle.is_none() {
            return Err(io_error(self.error(BackendErrorKind::Other)));
        }
        self.pending.extend_from_slice(bytes);
        while self.pending.len() >= CHUNK {
            let chunk: Vec<u8> = self.pending.drain(..CHUNK).collect();
            self.send(chunk).map_err(io_error)?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain().map_err(io_error)
    }
}

impl WriteSession for SftpWriteSession {
    fn finish(mut self: Box<Self>) -> Result<(), BackendError> {
        let outcome = self.commit();
        if outcome.is_err() {
            let _ = self.discard();
        }
        outcome
    }

    fn abort(mut self: Box<Self>) -> Result<(), BackendError> {
        self.discard()
    }
}

impl Drop for SftpWriteSession {
    fn drop(&mut self) {
        let _ = self.discard();
    }
}
