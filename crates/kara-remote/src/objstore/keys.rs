//! How a [`RemotePath`] becomes an object key and back.
//!
//! A drive is a bucket plus an optional key prefix. `RemotePath` "/" is the
//! prefix itself; `/a/b` is `<prefix>/a/b`. Keys are built with
//! `object_store::path::Path::parse`, which **does not percent-encode**: a
//! file called `100%.txt` is stored under the key `100%.txt`, exactly as other
//! S3 tools would store it. `Path::from` would have written `100%25.txt`.
//!
//! Some names cannot be keys: `object_store` refuses ASCII control characters
//! in a segment. Reading such a name answers `NotFound` (it cannot exist in
//! the drive); creating one answers `Other` (`InvalidInput`).
//!
//! # The directory placeholder
//!
//! An object store has no directories, only key prefixes, and a prefix with
//! no objects under it does not exist. «Nueva carpeta» therefore writes an
//! empty object named [`PLACEHOLDER`] inside the new folder
//! (`<folder>/.kara-dir`). The backend hides it from every listing, `stat`
//! and `open_read` treat it as missing, it is removed with its folder, and the
//! name is reserved: nothing else can be written under it. Other tools (the
//! AWS console, `aws s3 ls`, `gsutil`) do see it as a small file.
//!
//! The S3 console's own convention, an empty object whose key ends in `/`,
//! cannot be written through `object_store` (its paths never end in `/`). One
//! written by another tool still counts: it makes its folder exist and is
//! never listed as an entry.

use kara_vfs::RemotePath;
use object_store::path::Path;

/// Name of the empty object that keeps an empty folder alive.
pub const PLACEHOLDER: &str = ".kara-dir";

/// Maps drive paths to keys under the drive's prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Keys {
    /// The prefix, without leading or trailing `/`; empty for the bucket root.
    prefix: String,
}

/// Why a path has no key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoKey {
    /// A segment `object_store` cannot represent (control characters).
    Invalid,
    /// The last segment is [`PLACEHOLDER`].
    Reserved,
}

impl Keys {
    /// `prefix` is lexically normalised: empty segments are dropped, `.` and
    /// `..` are refused (as is anything `object_store` cannot represent).
    pub(crate) fn new(prefix: &str) -> Result<Keys, String> {
        let mut parts = Vec::new();
        for segment in prefix.split('/') {
            match segment {
                "" => {}
                "." | ".." => return Err(format!("the prefix {prefix:?} has a {segment:?} segment")),
                other => parts.push(other),
            }
        }
        let joined = parts.join("/");
        if !joined.is_empty() {
            Path::parse(&joined).map_err(|e| format!("the prefix {prefix:?} is not a valid key: {e}"))?;
        }
        Ok(Keys { prefix: joined })
    }

    /// The drive's prefix as written in keys (empty for the bucket root).
    pub(crate) fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The key of `path`, whatever its last segment. The root is the prefix.
    pub(crate) fn raw(&self, path: &RemotePath) -> Result<Path, NoKey> {
        let rel = path.as_str().trim_start_matches('/');
        let full = match (self.prefix.is_empty(), rel.is_empty()) {
            (_, true) => self.prefix.clone(),
            (true, false) => rel.to_owned(),
            (false, false) => format!("{}/{rel}", self.prefix),
        };
        Path::parse(&full).map_err(|_| NoKey::Invalid)
    }

    /// The key of `path`, unless a segment is the reserved [`PLACEHOLDER`].
    pub(crate) fn key(&self, path: &RemotePath) -> Result<Path, NoKey> {
        if path.segments().any(|segment| segment == PLACEHOLDER) {
            return Err(NoKey::Reserved);
        }
        self.raw(path)
    }

    /// The listing prefix of the folder whose key is `key`: `"key/"`, or the
    /// empty string for the root of a bucket.
    pub(crate) fn dir_prefix(key: &Path) -> String {
        let raw = key.as_ref();
        if raw.is_empty() {
            String::new()
        } else {
            format!("{raw}/")
        }
    }

    /// The drive path of `key`, if it lies under the prefix and every segment
    /// is a valid `RemotePath` segment. The prefix itself is the root.
    pub(crate) fn path_of(&self, key: &Path) -> Option<RemotePath> {
        let raw = key.as_ref();
        let rel = if self.prefix.is_empty() {
            raw
        } else if raw == self.prefix {
            ""
        } else {
            raw.strip_prefix(self.prefix.as_str())?.strip_prefix('/')?
        };
        let mut path = RemotePath::root();
        if rel.is_empty() {
            return Some(path);
        }
        for segment in rel.split('/') {
            path = path.join(segment).ok()?;
        }
        Some(path)
    }
}

/// `key/name`, or `name` under the empty (root) key.
pub(crate) fn join(key: &Path, name: &str) -> Path {
    let raw = key.as_ref();
    let full = if raw.is_empty() {
        name.to_owned()
    } else {
        format!("{raw}/{name}")
    };
    // `name` is a single valid segment here; parse only fails on input we
    // never produce, and then the unencoded form is still the right key.
    Path::parse(&full).unwrap_or_else(|_| Path::from_iter(full.split('/')))
}

/// The last segment of a key.
pub(crate) fn last_segment(key: &Path) -> &str {
    let raw = key.as_ref();
    raw.rsplit('/').next().unwrap_or(raw)
}

/// Whether `key` is the placeholder of some folder.
pub(crate) fn is_placeholder(key: &Path) -> bool {
    last_segment(key) == PLACEHOLDER
}
