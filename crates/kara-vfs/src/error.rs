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
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("BackendErrorKind Display")
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
    pub fn new(_kind: BackendErrorKind, _path: Option<RemotePath>) -> BackendError {
        todo!("BackendError::new")
    }

    /// Attaches the underlying cause.
    #[must_use]
    pub fn with_source(
        self,
        _source: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    ) -> BackendError {
        todo!("BackendError::with_source")
    }

    /// The kind.
    #[must_use]
    pub fn kind(&self) -> BackendErrorKind {
        todo!("BackendError::kind")
    }

    /// Recovers a wrapped `BackendError` unchanged, or maps a foreign
    /// `io::Error` by kind and raw errno (cb_13, cb_14).
    #[must_use]
    pub fn from_io(_error: io::Error, _path: Option<&RemotePath>) -> BackendError {
        todo!("BackendError::from_io")
    }
}

impl From<BackendError> for io::Error {
    fn from(_error: BackendError) -> io::Error {
        todo!("From<BackendError> for io::Error")
    }
}
