//! `BackendError`: a closed set of kinds that always names the path.

use std::fmt;
use std::io;

use crate::path::RemotePath;

/// What went wrong, in terms `kara-ops` maps onto its own failure kinds.
///
/// Closed and exhaustive on purpose (contract dec_05): adding a kind must force
/// every mapping to be revisited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendErrorKind {
    NotFound,
    AlreadyExists,
    PermissionDenied,
    NoSpace,
    Unavailable,
    AuthRequired,
    Unsupported,
    Cancelled,
    Other,
}

impl fmt::Display for BackendErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let phrase = match self {
            BackendErrorKind::NotFound => "not found",
            BackendErrorKind::AlreadyExists => "already exists",
            BackendErrorKind::PermissionDenied => "permission denied",
            BackendErrorKind::NoSpace => "no space left",
            BackendErrorKind::Unavailable => "unavailable",
            BackendErrorKind::AuthRequired => "authentication required",
            BackendErrorKind::Unsupported => "unsupported",
            BackendErrorKind::Cancelled => "cancelled",
            BackendErrorKind::Other => "other error",
        };
        f.write_str(phrase)
    }
}

/// An error from a [`crate::Backend`] operation.
#[derive(Debug, thiserror::Error)]
#[error("{kind}{}", path.as_ref().map(|p| format!(": {p}")).unwrap_or_default())]
pub struct BackendError {
    pub kind: BackendErrorKind,
    pub path: Option<RemotePath>,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl BackendError {
    /// An error of `kind` about `path`, with no source.
    #[must_use]
    pub fn new(kind: BackendErrorKind, path: Option<RemotePath>) -> BackendError {
        BackendError {
            kind,
            path,
            source: None,
        }
    }

    /// Attaches the underlying cause.
    #[must_use]
    pub fn with_source(
        mut self,
        source: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    ) -> BackendError {
        self.source = Some(source.into());
        self
    }

    /// The kind.
    #[must_use]
    pub fn kind(&self) -> BackendErrorKind {
        self.kind
    }

    /// Recovers a wrapped `BackendError` unchanged, or maps a foreign
    /// `io::Error` by kind and raw errno (cb_13, cb_14).
    #[must_use]
    pub fn from_io(error: io::Error, path: Option<&RemotePath>) -> BackendError {
        let wraps_backend_error = error
            .get_ref()
            .is_some_and(|inner| inner.is::<BackendError>());
        if wraps_backend_error {
            // Take the inner error out of the io::Error. If the downcast were to
            // fail after all, there is nothing left to keep but the kind.
            return match error.into_inner() {
                Some(inner) => match inner.downcast::<BackendError>() {
                    Ok(original) => *original,
                    Err(other) => {
                        BackendError::new(BackendErrorKind::Other, path.cloned()).with_source(other)
                    }
                },
                None => BackendError::new(BackendErrorKind::Other, path.cloned()),
            };
        }
        let kind = kind_of_io(&error);
        BackendError::new(kind, path.cloned()).with_source(error)
    }
}

/// Raw errnos of vanished media win over the generic kind: EIO, ENXIO, ENODEV
/// and ENOMEDIUM mean the device went away, so retrying cannot help.
fn kind_of_io(error: &io::Error) -> BackendErrorKind {
    if matches!(error.raw_os_error(), Some(5 | 6 | 19 | 123)) {
        return BackendErrorKind::Unavailable;
    }
    match error.kind() {
        io::ErrorKind::NotFound => BackendErrorKind::NotFound,
        io::ErrorKind::AlreadyExists => BackendErrorKind::AlreadyExists,
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            BackendErrorKind::PermissionDenied
        }
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => BackendErrorKind::NoSpace,
        io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::NotConnected
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::TimedOut
        | io::ErrorKind::HostUnreachable
        | io::ErrorKind::NetworkUnreachable
        | io::ErrorKind::NetworkDown
        | io::ErrorKind::StaleNetworkFileHandle => BackendErrorKind::Unavailable,
        io::ErrorKind::Unsupported => BackendErrorKind::Unsupported,
        _ => BackendErrorKind::Other,
    }
}

impl From<BackendError> for io::Error {
    fn from(error: BackendError) -> io::Error {
        // Cancelled is never Interrupted: io::copy and friends retry that forever.
        let kind = match error.kind {
            BackendErrorKind::NotFound => io::ErrorKind::NotFound,
            BackendErrorKind::AlreadyExists => io::ErrorKind::AlreadyExists,
            BackendErrorKind::PermissionDenied | BackendErrorKind::AuthRequired => {
                io::ErrorKind::PermissionDenied
            }
            BackendErrorKind::NoSpace => io::ErrorKind::StorageFull,
            BackendErrorKind::Unavailable => io::ErrorKind::NotConnected,
            BackendErrorKind::Unsupported => io::ErrorKind::Unsupported,
            BackendErrorKind::Cancelled | BackendErrorKind::Other => io::ErrorKind::Other,
        };
        io::Error::new(kind, error)
    }
}
