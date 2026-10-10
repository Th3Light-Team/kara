//! What can go wrong on the wire, and how it becomes a [`BackendError`].
//!
//! [`Fail`] is the adapter's internal error: an SFTP status with its code and
//! message, a lost or timed-out session, a cancellation, or anything else.
//! Callers turn it into a [`BackendError`] naming **the caller's** path with
//! [`Fail::at`]; the server path or the name of a temporary never appears in an
//! error.
//!
//! Status mapping (SFTP v3, what OpenSSH speaks):
//!
//! | SFTP | `BackendErrorKind` |
//! |---|---|
//! | `NO_SUCH_FILE` | `NotFound` |
//! | `PERMISSION_DENIED` | `PermissionDenied` |
//! | `FAILURE` whose message says the disk or a quota is full | `NoSpace` |
//! | `FAILURE` otherwise, `BAD_MESSAGE`, `EOF` | `Other` |
//! | `OP_UNSUPPORTED` | `Unsupported` |
//! | `NO_CONNECTION`, `CONNECTION_LOST`, closed channel, reset, timeout | `Unavailable` |
//!
//! OpenSSH answers every `errno` it has no code for with a bare `FAILURE`
//! («Failure»), so an existing target, a non-empty directory and a full disk
//! look alike: the backend tells `AlreadyExists` apart by looking (`lstat`)
//! after the failure, and only reports `NoSpace` when the server's message says
//! so (servers that send `strerror` text do).

use std::fmt;
use std::io;

use kara_vfs::{BackendError, BackendErrorKind, RemotePath};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::StatusCode;

/// An SFTP-level failure, before it is attached to a path.
#[derive(Debug, Clone)]
pub(crate) enum Fail {
    /// The server answered with a status other than `OK`.
    Status { code: StatusCode, message: String },
    /// The session, the channel or the connection is gone.
    Lost(String),
    /// No answer in time.
    TimedOut,
    /// The caller's token was cancelled.
    Cancelled,
    /// Anything else: a malformed reply, a limit, an invalid argument.
    Other(String),
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fail::Status { code, message } => write!(f, "sftp status {code}: {message}"),
            Fail::Lost(reason) => write!(f, "connection lost: {reason}"),
            Fail::TimedOut => f.write_str("the server did not answer in time"),
            Fail::Cancelled => f.write_str("cancelled"),
            Fail::Other(reason) => f.write_str(reason),
        }
    }
}

impl From<SftpError> for Fail {
    fn from(error: SftpError) -> Fail {
        match error {
            SftpError::Status(status) => match status.status_code {
                StatusCode::NoConnection | StatusCode::ConnectionLost => {
                    Fail::Lost(status.error_message)
                }
                code => Fail::Status {
                    code,
                    message: status.error_message,
                },
            },
            SftpError::Timeout => Fail::TimedOut,
            // An I/O error under the channel, or the session machinery saying
            // the transport stopped ("session closed", "sender dropped", ...).
            SftpError::IO(reason) | SftpError::UnexpectedBehavior(reason) => Fail::Lost(reason),
            SftpError::Limited(reason) => Fail::Other(reason),
            SftpError::UnexpectedPacket => Fail::Other(String::from("unexpected reply packet")),
        }
    }
}

impl Fail {
    /// Whether this is a status with `code`.
    pub(crate) fn is(&self, code: StatusCode) -> bool {
        matches!(self, Fail::Status { code: got, .. } if *got == code)
    }

    /// The kind this failure maps to.
    pub(crate) fn kind(&self) -> BackendErrorKind {
        match self {
            Fail::Status { code, message } => match code {
                StatusCode::NoSuchFile => BackendErrorKind::NotFound,
                StatusCode::PermissionDenied => BackendErrorKind::PermissionDenied,
                StatusCode::Failure if says_no_space(message) => BackendErrorKind::NoSpace,
                StatusCode::OpUnsupported => BackendErrorKind::Unsupported,
                StatusCode::NoConnection | StatusCode::ConnectionLost => {
                    BackendErrorKind::Unavailable
                }
                StatusCode::Ok
                | StatusCode::Eof
                | StatusCode::Failure
                | StatusCode::BadMessage => BackendErrorKind::Other,
            },
            Fail::Lost(_) | Fail::TimedOut => BackendErrorKind::Unavailable,
            Fail::Cancelled => BackendErrorKind::Cancelled,
            Fail::Other(_) => BackendErrorKind::Other,
        }
    }

    /// The error about `path` (the caller's path, never a server path).
    pub(crate) fn at(self, path: &RemotePath) -> BackendError {
        self.with_kind(self.kind(), path)
    }

    /// The error about `path` with an explicit kind, keeping this failure as source.
    pub(crate) fn with_kind(&self, kind: BackendErrorKind, path: &RemotePath) -> BackendError {
        let io_kind = match self {
            Fail::TimedOut => io::ErrorKind::TimedOut,
            Fail::Lost(_) => io::ErrorKind::NotConnected,
            _ => io::ErrorKind::Other,
        };
        BackendError::new(kind, Some(path.clone()))
            .with_source(io::Error::new(io_kind, self.to_string()))
    }
}

/// Whether a `FAILURE` message says the disk or a quota is full. OpenSSH sends
/// only «Failure»; servers that pass `strerror` text through say more.
fn says_no_space(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    ["no space", "disk full", "quota", "not enough space", "insufficient space"]
        .iter()
        .any(|needle| lower.contains(needle))
}

/// An error of `kind` about `path`, with a plain io error of `source` as cause.
pub(crate) fn error(kind: BackendErrorKind, path: &RemotePath, source: io::ErrorKind) -> BackendError {
    BackendError::new(kind, Some(path.clone())).with_source(io::Error::from(source))
}
