//! `RemotePath`: a normalised, `/`-separated, UTF-8 path inside a drive.
//!
//! Not a `PathBuf`: S3 keys are not filesystem paths and must not inherit their
//! semantics, so `.` and `..` are rejected instead of resolved.

use std::fmt;
use std::str::FromStr;

/// A canonical absolute path inside a remote drive.
///
/// Canonical form: starts with `/`, segments separated by exactly one `/`, no
/// trailing `/` except the root `"/"`, no `.` or `..` segment, no NUL, valid
/// UTF-8. No Unicode normalisation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RemotePath(String);

/// Why a string is not a valid [`RemotePath`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemotePathError {
    #[error("empty path")]
    Empty,
    #[error("path is not absolute: {raw:?}")]
    NotAbsolute { raw: String },
    #[error("path has a '.' or '..' segment: {raw:?}")]
    DotSegment { raw: String },
    #[error("path contains a NUL byte")]
    ContainsNul,
    #[error("path is not valid UTF-8")]
    NotUtf8,
    #[error("invalid path segment: {segment:?}")]
    InvalidSegment { segment: String },
}

impl RemotePath {
    /// The drive root, `"/"`.
    #[must_use]
    pub fn root() -> RemotePath {
        todo!("RemotePath::root")
    }

    /// Parses and normalises a path (cb_01, cb_02, cb_03).
    pub fn parse(_s: &str) -> Result<RemotePath, RemotePathError> {
        todo!("RemotePath::parse")
    }

    /// Like [`RemotePath::parse`], from raw bytes that must be UTF-8.
    pub fn from_bytes(_bytes: &[u8]) -> Result<RemotePath, RemotePathError> {
        todo!("RemotePath::from_bytes")
    }

    /// The canonical string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        todo!("RemotePath::as_str")
    }

    /// Whether this is the drive root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        todo!("RemotePath::is_root")
    }

    /// The containing directory; `None` for the root.
    #[must_use]
    pub fn parent(&self) -> Option<RemotePath> {
        todo!("RemotePath::parent")
    }

    /// The last segment; `None` for the root.
    #[must_use]
    pub fn file_name(&self) -> Option<&str> {
        todo!("RemotePath::file_name")
    }

    /// Appends exactly one segment (cb_04).
    pub fn join(&self, _segment: &str) -> Result<RemotePath, RemotePathError> {
        todo!("RemotePath::join")
    }

    /// The segments, root first; empty for the root.
    #[allow(unreachable_code)]
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        todo!("RemotePath::segments");
        std::iter::empty()
    }

    /// Segment-aware prefix test: `/ab` does not start with `/a`.
    #[must_use]
    pub fn starts_with(&self, _base: &RemotePath) -> bool {
        todo!("RemotePath::starts_with")
    }
}

impl fmt::Display for RemotePath {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("RemotePath Display")
    }
}

impl FromStr for RemotePath {
    type Err = RemotePathError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        todo!("RemotePath FromStr")
    }
}
