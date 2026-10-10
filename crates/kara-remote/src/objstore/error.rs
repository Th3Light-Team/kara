//! What can go wrong talking to an object store, and how it becomes a
//! [`BackendError`].
//!
//! | `object_store::Error` | `BackendErrorKind` |
//! |---|---|
//! | `NotFound` | `NotFound` |
//! | `AlreadyExists`, `Precondition` (a conditional put or copy lost) | `AlreadyExists` |
//! | `PermissionDenied` (403), `Unauthenticated` (401) | `PermissionDenied` |
//! | `NotSupported`, `NotImplemented` | `Unsupported` |
//! | anything whose cause says the service is out of space or quota (HTTP 507, MinIO's `XMinioStorageFull`, `QuotaExceeded`) | `NoSpace` |
//! | a connection, request or timeout failure of the HTTP client, or a 5xx answer, once the client's retries are exhausted | `Unavailable` |
//! | our own bound on a call running out | `Unavailable` |
//! | anything else | `Other` |
//!
//! `NoSpace` is reported only when the service says so; S3 itself never runs
//! out of space, so on AWS this never happens. The error always names **the
//! caller's** path; the text of the cause is kept as the source with the
//! object key replaced by that path.

use std::error::Error as StdError;
use std::fmt;
use std::io;

use kara_vfs::{BackendError, BackendErrorKind, RemotePath};
#[cfg(any(feature = "s3", feature = "gcs"))]
use object_store::client::{HttpError, HttpErrorKind};
use object_store::path::Path;

/// An object-store failure, before it is attached to a path.
#[derive(Debug)]
pub(crate) enum Fail {
    /// The store answered with an error.
    Store(object_store::Error),
    /// Our own bound on the call ran out.
    TimedOut,
    /// The caller's token was cancelled.
    Cancelled,
    /// Anything else: a broken invariant, a closed drive.
    Other(String),
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fail::Store(error) => write!(f, "{error}"),
            Fail::TimedOut => f.write_str("the service did not answer in time"),
            Fail::Cancelled => f.write_str("cancelled"),
            Fail::Other(reason) => f.write_str(reason),
        }
    }
}

impl From<object_store::Error> for Fail {
    fn from(error: object_store::Error) -> Fail {
        Fail::Store(error)
    }
}

/// The whole chain of causes of `error`, as text, for classification.
pub(crate) fn chain_text(error: &(dyn StdError + 'static)) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(inner) = cause {
        text.push_str(" | ");
        text.push_str(&inner.to_string());
        cause = inner.source();
    }
    text
}

/// Whether any cause is an HTTP failure that means the service is unreachable.
fn http_unreachable(error: &(dyn StdError + 'static)) -> bool {
    let mut cause: Option<&(dyn StdError + 'static)> = Some(error);
    while let Some(inner) = cause {
        #[cfg(any(feature = "s3", feature = "gcs"))]
        if let Some(http) = inner.downcast_ref::<HttpError>() {
            return matches!(
                http.kind(),
                HttpErrorKind::Connect
                    | HttpErrorKind::Request
                    | HttpErrorKind::Timeout
                    | HttpErrorKind::Interrupted
            );
        }
        if let Some(io) = inner.downcast_ref::<io::Error>()
            && matches!(
                io.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::TimedOut
                    | io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::HostUnreachable
                    | io::ErrorKind::NetworkUnreachable
                    | io::ErrorKind::NetworkDown
            )
        {
            return true;
        }
        cause = inner.source();
    }
    false
}

/// Whether the service said it is out of space or over a quota.
pub(crate) fn says_no_space(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "status code: 507",
        "insufficient storage",
        "xminiostoragefull",
        "storage full",
        "no space left",
        "quotaexceeded",
        "quota exceeded",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Whether the text says the service answered 401 or 403.
pub(crate) fn says_denied(text: &str) -> bool {
    text.contains("status code: 401") || text.contains("status code: 403")
}

/// Whether the text says the service answered 5xx (after the client's retries).
fn says_server_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("status code: 5")
        || lower.contains("serviceunavailable")
        || lower.contains("slowdown")
        || lower.contains("internalerror")
}

/// The kind an `object_store` error maps to.
pub(crate) fn kind_of(error: &object_store::Error) -> BackendErrorKind {
    use object_store::Error as E;
    match error {
        E::NotFound { .. } => BackendErrorKind::NotFound,
        E::AlreadyExists { .. } | E::Precondition { .. } => BackendErrorKind::AlreadyExists,
        E::PermissionDenied { .. } | E::Unauthenticated { .. } => {
            BackendErrorKind::PermissionDenied
        }
        E::NotSupported { .. } | E::NotImplemented { .. } => BackendErrorKind::Unsupported,
        other => {
            // Some requests (listing, bulk delete) wrap every HTTP failure in
            // `Generic`: the status is only in the text.
            let text = chain_text(other);
            if says_no_space(&text) {
                BackendErrorKind::NoSpace
            } else if says_denied(&text) {
                BackendErrorKind::PermissionDenied
            } else if http_unreachable(other) || says_server_error(&text) {
                BackendErrorKind::Unavailable
            } else {
                BackendErrorKind::Other
            }
        }
    }
}

impl Fail {
    /// The text of the failure and all its causes.
    pub(crate) fn full_text(&self) -> String {
        match self {
            Fail::Store(error) => chain_text(error),
            other => other.to_string(),
        }
    }

    /// The kind this failure maps to.
    pub(crate) fn kind(&self) -> BackendErrorKind {
        match self {
            Fail::Store(error) => kind_of(error),
            Fail::TimedOut => BackendErrorKind::Unavailable,
            Fail::Cancelled => BackendErrorKind::Cancelled,
            Fail::Other(_) => BackendErrorKind::Other,
        }
    }

    /// Whether the store said the object is not there.
    pub(crate) fn is_not_found(&self) -> bool {
        matches!(self, Fail::Store(object_store::Error::NotFound { .. }))
    }

    /// The error about `path`, the caller's path. `key`, the object key the
    /// request was about, is replaced by `path` in the text of the cause.
    pub(crate) fn at(&self, path: &RemotePath, key: Option<&Path>) -> BackendError {
        self.with_kind(self.kind(), path, key)
    }

    /// Like [`Fail::at`] with an explicit kind.
    pub(crate) fn with_kind(
        &self,
        kind: BackendErrorKind,
        path: &RemotePath,
        key: Option<&Path>,
    ) -> BackendError {
        let mut text = self.to_string();
        if let Some(key) = key.map(AsRef::<str>::as_ref).filter(|k| !k.is_empty()) {
            text = text.replace(key, path.as_str());
        }
        let io_kind = match self {
            Fail::TimedOut => io::ErrorKind::TimedOut,
            Fail::Cancelled => io::ErrorKind::Other,
            _ if kind == BackendErrorKind::Unavailable => io::ErrorKind::NotConnected,
            _ => io::ErrorKind::Other,
        };
        BackendError::new(kind, Some(path.clone())).with_source(io::Error::new(io_kind, text))
    }
}

/// An error of `kind` about `path`, with a plain io error of `source` as cause.
pub(crate) fn error(kind: BackendErrorKind, path: &RemotePath, source: io::ErrorKind) -> BackendError {
    BackendError::new(kind, Some(path.clone())).with_source(io::Error::from(source))
}

/// `Other` with the precise `io::ErrorKind` as cause (as `MemoryBackend` does).
pub(crate) fn other(path: &RemotePath, source: io::ErrorKind) -> BackendError {
    error(BackendErrorKind::Other, path, source)
}
