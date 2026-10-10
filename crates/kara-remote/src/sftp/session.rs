//! One live SFTP session: the private runtime, the SSH connection and the
//! SFTP channel, behind blocking calls.
//!
//! Every call first checks that the connection is still up (a dead one answers
//! `Unavailable` at once, without touching the network) and then waits at most
//! the configured timeout. Nothing async leaves this module except through
//! [`Session::spawn`], which the reader and the write session use to keep
//! several requests in flight.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use kara_vfs::Cancel;
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, Packet, StatusCode};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

use super::connect::Client;
use super::error::Fail;

/// Extra time the outer wait gives the per-request timeout before giving up
/// itself: the request's own timeout is the one that normally fires.
const GRACE: Duration = Duration::from_secs(2);
/// How often a cancellable wait looks at the token.
const CANCEL_POLL: Duration = Duration::from_millis(20);

/// A file as `readdir` describes it.
pub(crate) struct DirEntry {
    pub name: String,
    pub attrs: FileAttributes,
}

/// The connection and what the server offered, fixed at connect time.
pub(crate) struct Session {
    runtime: Option<Runtime>,
    pub(crate) sftp: Arc<RawSftpSession>,
    ssh: russh::client::Handle<Client>,
    timeout: Duration,
    pub(crate) posix_rename: bool,
    pub(crate) fsync: bool,
}

impl Session {
    pub(crate) fn new(
        runtime: Runtime,
        sftp: RawSftpSession,
        ssh: russh::client::Handle<Client>,
        timeout: Duration,
        posix_rename: bool,
        fsync: bool,
    ) -> Session {
        Session {
            runtime: Some(runtime),
            sftp: Arc::new(sftp),
            ssh,
            timeout,
            posix_rename,
            fsync,
        }
    }

    /// Whether the SSH connection or the SFTP channel is gone.
    pub(crate) fn is_closed(&self) -> bool {
        self.ssh.is_closed()
    }

    /// Runs `future` to completion on the private runtime, bounded by the timeout.
    pub(crate) fn block_on<F: Future>(&self, future: F) -> Result<F::Output, Fail> {
        // Blocking inside an async runtime would panic in tokio: refuse instead.
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(Fail::Other(String::from(
                "an SFTP drive was called from inside an async runtime",
            )));
        }
        let Some(runtime) = self.runtime.as_ref() else {
            return Err(Fail::Lost(String::from("the session is closed")));
        };
        if self.is_closed() {
            return Err(Fail::Lost(String::from("the connection is closed")));
        }
        let limit = self.timeout + GRACE;
        // The timer is created inside the runtime: tokio needs its context for it.
        runtime
            .block_on(async move { tokio::time::timeout(limit, future).await })
            .map_err(|_| Fail::TimedOut)
    }

    /// Like [`Session::block_on`], but gives up as soon as `cancel` is set.
    pub(crate) fn block_on_cancellable<F: Future>(
        &self,
        future: F,
        cancel: &Cancel,
    ) -> Result<F::Output, Fail> {
        let watch = async {
            while !cancel.is_cancelled() {
                tokio::time::sleep(CANCEL_POLL).await;
            }
        };
        self.block_on(async {
            tokio::select! {
                out = future => Ok(out),
                () = watch => Err(Fail::Cancelled),
            }
        })?
    }

    /// One request, its failure turned into a [`Fail`].
    pub(crate) fn call<T>(&self, future: impl Future<Output = Result<T, SftpError>>) -> Result<T, Fail> {
        self.block_on(future)?.map_err(Fail::from)
    }

    /// Starts `future` on the runtime without waiting for it. `None` once the
    /// session is closed.
    pub(crate) fn spawn<F>(&self, future: F) -> Option<JoinHandle<F::Output>>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let runtime = self.runtime.as_ref()?;
        if self.is_closed() {
            return None;
        }
        Some(runtime.spawn(future))
    }

    /// Waits for a task started by [`Session::spawn`].
    pub(crate) fn join<T>(&self, task: JoinHandle<Result<T, SftpError>>) -> Result<T, Fail> {
        match self.block_on(task)? {
            Ok(result) => result.map_err(Fail::from),
            Err(join) => Err(Fail::Lost(join.to_string())),
        }
    }

    // -- requests ------------------------------------------------------------

    pub(crate) fn lstat(&self, path: &str) -> Result<FileAttributes, Fail> {
        self.call(self.sftp.lstat(path)).map(|attrs| attrs.attrs)
    }

    pub(crate) fn stat(&self, path: &str) -> Result<FileAttributes, Fail> {
        self.call(self.sftp.stat(path)).map(|attrs| attrs.attrs)
    }

    pub(crate) fn realpath(&self, path: &str) -> Result<String, Fail> {
        let name = self.call(self.sftp.realpath(path))?;
        name.files
            .into_iter()
            .next()
            .map(|file| file.filename)
            .ok_or_else(|| Fail::Other(String::from("realpath returned no name")))
    }

    pub(crate) fn open(&self, path: &str, flags: OpenFlags) -> Result<String, Fail> {
        self.call(self.sftp.open(path, flags, FileAttributes::empty()))
            .map(|handle| handle.handle)
    }

    pub(crate) fn close(&self, handle: &str) -> Result<(), Fail> {
        self.call(self.sftp.close(handle)).map(|_| ())
    }

    /// Closes a handle without waiting: for `Drop`, where nobody reads the answer.
    pub(crate) fn close_later(&self, handle: String) {
        let sftp = Arc::clone(&self.sftp);
        let _ = self.spawn(async move { sftp.close(handle).await });
    }

    pub(crate) fn remove(&self, path: &str) -> Result<(), Fail> {
        self.call(self.sftp.remove(path)).map(|_| ())
    }

    pub(crate) fn rmdir(&self, path: &str) -> Result<(), Fail> {
        self.call(self.sftp.rmdir(path)).map(|_| ())
    }

    pub(crate) fn mkdir(&self, path: &str) -> Result<(), Fail> {
        self.call(self.sftp.mkdir(path, FileAttributes::empty()))
            .map(|_| ())
    }

    /// Plain `SSH_FXP_RENAME`: never replaces an existing target on OpenSSH.
    pub(crate) fn rename(&self, from: &str, to: &str) -> Result<(), Fail> {
        self.call(self.sftp.rename(from, to)).map(|_| ())
    }

    /// `posix-rename@openssh.com`: rename(2), which replaces a file atomically.
    pub(crate) fn posix_rename(&self, from: &str, to: &str) -> Result<(), Fail> {
        let mut data = Vec::with_capacity(8 + from.len() + to.len());
        for text in [from, to] {
            let len = u32::try_from(text.len())
                .map_err(|_| Fail::Other(String::from("path too long")))?;
            data.extend_from_slice(&len.to_be_bytes());
            data.extend_from_slice(text.as_bytes());
        }
        let reply = self.call(self.sftp.extended("posix-rename@openssh.com", data))?;
        status_reply(reply)
    }

    /// `fsync@openssh.com` on an open handle.
    pub(crate) fn fsync(&self, handle: &str) -> Result<(), Fail> {
        self.call(self.sftp.fsync(handle)).map(|_| ())
    }

    /// Every entry of a directory but `.` and `..`, page by page; `cancel` is
    /// looked at between pages and while a page is on its way.
    pub(crate) fn read_dir(&self, path: &str, cancel: &Cancel) -> Result<Vec<DirEntry>, Fail> {
        if cancel.is_cancelled() {
            return Err(Fail::Cancelled);
        }
        let handle = self
            .block_on_cancellable(self.sftp.opendir(path), cancel)?
            .map_err(Fail::from)?
            .handle;
        let mut entries = Vec::new();
        let outcome = loop {
            if cancel.is_cancelled() {
                break Err(Fail::Cancelled);
            }
            let page = match self.block_on_cancellable(self.sftp.readdir(handle.as_str()), cancel) {
                Ok(page) => page.map_err(Fail::from),
                Err(fail) => Err(fail),
            };
            match page {
                Ok(name) => entries.extend(
                    name.files
                        .into_iter()
                        .filter(|file| file.filename != "." && file.filename != "..")
                        .map(|file| DirEntry {
                            name: file.filename,
                            attrs: file.attrs,
                        }),
                ),
                Err(fail) if fail.is(StatusCode::Eof) => break Ok(()),
                Err(fail) => break Err(fail),
            }
        };
        match outcome {
            Ok(()) => {
                self.close(&handle)?;
                Ok(entries)
            }
            Err(fail) => {
                self.close_later(handle);
                Err(fail)
            }
        }
    }
}

/// An extended request answered with a status: `OK` or a failure.
fn status_reply(reply: Packet) -> Result<(), Fail> {
    match reply {
        Packet::Status(status) if status.status_code == StatusCode::Ok => Ok(()),
        Packet::Status(status) => Err(Fail::from(SftpError::Status(status))),
        _ => Err(Fail::Other(String::from("unexpected reply to an extended request"))),
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        // Say goodbye if the connection is still there, without waiting long.
        if tokio::runtime::Handle::try_current().is_err() && !self.ssh.is_closed() {
            let _ = self.sftp.close_session();
            let ssh = &self.ssh;
            let _ = runtime.block_on(async move {
                tokio::time::timeout(
                    Duration::from_millis(500),
                    ssh.disconnect(russh::Disconnect::ByApplication, "", "en"),
                )
                .await
            });
        }
        runtime.shutdown_background();
    }
}
