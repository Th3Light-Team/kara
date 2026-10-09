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
        RemotePath(String::from("/"))
    }

    /// Parses and normalises a path (cb_01, cb_02, cb_03).
    pub fn parse(s: &str) -> Result<RemotePath, RemotePathError> {
        if s.is_empty() {
            return Err(RemotePathError::Empty);
        }
        if s.contains('\0') {
            return Err(RemotePathError::ContainsNul);
        }
        if !s.starts_with('/') {
            return Err(RemotePathError::NotAbsolute { raw: s.to_owned() });
        }
        let mut canonical = String::with_capacity(s.len());
        for segment in s.split('/').filter(|segment| !segment.is_empty()) {
            if segment == "." || segment == ".." {
                return Err(RemotePathError::DotSegment { raw: s.to_owned() });
            }
            canonical.push('/');
            canonical.push_str(segment);
        }
        if canonical.is_empty() {
            return Ok(RemotePath::root());
        }
        Ok(RemotePath(canonical))
    }

    /// Like [`RemotePath::parse`], from raw bytes that must be UTF-8.
    pub fn from_bytes(bytes: &[u8]) -> Result<RemotePath, RemotePathError> {
        if bytes.is_empty() {
            return Err(RemotePathError::Empty);
        }
        // A NUL is reported as such even when the rest is not UTF-8.
        if bytes.contains(&0) {
            return Err(RemotePathError::ContainsNul);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| RemotePathError::NotUtf8)?;
        RemotePath::parse(text)
    }

    /// The canonical string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is the drive root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0 == "/"
    }

    /// The containing directory; `None` for the root.
    #[must_use]
    pub fn parent(&self) -> Option<RemotePath> {
        if self.is_root() {
            return None;
        }
        let cut = self.0.rfind('/')?;
        if cut == 0 {
            return Some(RemotePath::root());
        }
        self.0.get(..cut).map(|head| RemotePath(head.to_owned()))
    }

    /// The last segment; `None` for the root.
    #[must_use]
    pub fn file_name(&self) -> Option<&str> {
        if self.is_root() {
            return None;
        }
        let cut = self.0.rfind('/')?;
        self.0.get(cut + 1..)
    }

    /// Appends exactly one segment (cb_04).
    pub fn join(&self, segment: &str) -> Result<RemotePath, RemotePathError> {
        if segment.contains('\0') {
            return Err(RemotePathError::ContainsNul);
        }
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains('/') {
            return Err(RemotePathError::InvalidSegment {
                segment: segment.to_owned(),
            });
        }
        let mut joined = String::with_capacity(self.0.len() + 1 + segment.len());
        if !self.is_root() {
            joined.push_str(&self.0);
        }
        joined.push('/');
        joined.push_str(segment);
        Ok(RemotePath(joined))
    }

    /// The segments, root first; empty for the root.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|segment| !segment.is_empty())
    }

    /// Segment-aware prefix test: `/ab` does not start with `/a`.
    #[must_use]
    pub fn starts_with(&self, base: &RemotePath) -> bool {
        if base.is_root() {
            return true;
        }
        match self.0.strip_prefix(base.0.as_str()) {
            Some(rest) => rest.is_empty() || rest.starts_with('/'),
            None => false,
        }
    }
}

impl fmt::Display for RemotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for RemotePath {
    type Err = RemotePathError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RemotePath::parse(s)
    }
}
