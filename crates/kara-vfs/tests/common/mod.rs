//! Shared helpers for the kara-vfs integration tests.
//!
//! Tests may panic (they are the harness); `src/` may not (cb_15).

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::io::{self, Read, Write};

use kara_core::{EntryKind, FileEntry};
use kara_vfs::memory::{MemoryBackend, MemoryNode, MemorySnapshot, MemoryStats};
use kara_vfs::{Backend, BackendError, BackendErrorKind, Cancel, Capabilities, RemotePath};

/// Parses a test path; panics on a typo in the test itself.
pub fn p(s: &str) -> RemotePath {
    RemotePath::parse(s).unwrap_or_else(|e| panic!("test path {s:?} does not parse: {e}"))
}

pub fn posix_caps() -> Capabilities {
    Capabilities {
        trash: false,
        atomic_rename: true,
        server_side_copy: false,
        real_directories: true,
        posix_permissions: true,
        symlinks: true,
        watch: false,
        undo_rename: true,
        undo_move: true,
    }
}

pub fn object_caps() -> Capabilities {
    Capabilities {
        trash: false,
        atomic_rename: false,
        server_side_copy: true,
        real_directories: false,
        posix_permissions: false,
        symlinks: false,
        watch: false,
        undo_rename: false,
        undo_move: false,
    }
}

/// Both MemoryBackend profiles, labelled for assertion messages.
pub fn profiles() -> Vec<(&'static str, MemoryBackend)> {
    vec![
        ("posix_like", MemoryBackend::posix_like()),
        ("object_store_like", MemoryBackend::object_store_like()),
    ]
}

/// Writes a new file (replace = false) and commits it.
pub fn write(b: &dyn Backend, path: &str, content: &[u8]) {
    write_with(b, path, content, false);
}

/// Writes a file with the given replace flag and commits it.
pub fn write_with(b: &dyn Backend, path: &str, content: &[u8], replace: bool) {
    let mut s = b
        .begin_write(&p(path), Some(content.len() as u64), replace)
        .unwrap_or_else(|e| panic!("begin_write({path}) failed: {e:?}"));
    s.write_all(content)
        .unwrap_or_else(|e| panic!("write({path}) failed: {e:?}"));
    s.finish()
        .unwrap_or_else(|e| panic!("finish({path}) failed: {e:?}"));
}

pub fn mkdir(b: &dyn Backend, path: &str) {
    b.create_dir(&p(path))
        .unwrap_or_else(|e| panic!("create_dir({path}) failed: {e:?}"));
}

/// Reads a whole file from offset 0; panics on any error.
pub fn read(b: &dyn Backend, path: &str) -> Vec<u8> {
    try_read(b, &p(path)).unwrap_or_else(|e| panic!("read({path}) failed: {e:?}"))
}

/// Reads a whole file from offset 0; read errors are mapped back with from_io.
pub fn try_read(b: &dyn Backend, path: &RemotePath) -> Result<Vec<u8>, BackendError> {
    let mut r = b.open_read(path, 0)?;
    let mut out = Vec::new();
    r.read_to_end(&mut out)
        .map_err(|e| BackendError::from_io(e, Some(path)))?;
    Ok(out)
}

/// The names of a directory's entries, as a set.
pub fn names(b: &dyn Backend, dir: &str) -> BTreeSet<String> {
    let listing = b
        .list(&p(dir), &Cancel::new())
        .unwrap_or_else(|e| panic!("list({dir}) failed: {e:?}"));
    assert!(
        listing.errors.is_empty(),
        "list({dir}) reported errors: {:?}",
        listing.errors
    );
    listing.entries.iter().map(|e| e.display.clone()).collect()
}

/// The entry named `name` in `dir`.
pub fn entry(b: &dyn Backend, dir: &str, name: &str) -> FileEntry {
    let listing = b
        .list(&p(dir), &Cancel::new())
        .unwrap_or_else(|e| panic!("list({dir}) failed: {e:?}"));
    listing
        .entries
        .into_iter()
        .find(|e| e.display == name)
        .unwrap_or_else(|| panic!("{name} not listed in {dir}"))
}

pub fn snap(m: &MemoryBackend) -> MemorySnapshot {
    m.snapshot()
        .unwrap_or_else(|e| panic!("snapshot failed: {e:?}"))
}

pub fn stats(m: &MemoryBackend) -> MemoryStats {
    m.stats().unwrap_or_else(|e| panic!("stats failed: {e:?}"))
}

/// The error of a result that must have failed.
pub fn err<T>(r: Result<T, BackendError>, what: &str) -> BackendError {
    match r {
        Ok(_) => panic!("{what}: expected an error, got Ok"),
        Err(e) => e,
    }
}

/// Asserts an error has `kind` and `path`.
pub fn assert_err(e: &BackendError, kind: BackendErrorKind, path: &str, what: &str) {
    assert_eq!(e.kind, kind, "{what}: wrong kind in {e:?}");
    assert_eq!(
        e.kind(),
        kind,
        "{what}: kind() disagrees with the field in {e:?}"
    );
    assert_eq!(e.path, Some(p(path)), "{what}: wrong path in {e:?}");
}

/// The io::ErrorKind of an error's source, when the source is an io::Error.
pub fn io_source_kind(e: &BackendError) -> Option<io::ErrorKind> {
    e.source
        .as_ref()
        .and_then(|s| s.downcast_ref::<io::Error>())
        .map(io::Error::kind)
}

/// Asserts `Other` with an io::Error source of `io_kind` (contract dec_05).
pub fn assert_other_with(e: &BackendError, io_kind: io::ErrorKind, what: &str) {
    assert_eq!(
        e.kind,
        BackendErrorKind::Other,
        "{what}: wrong kind in {e:?}"
    );
    assert_eq!(
        io_source_kind(e),
        Some(io_kind),
        "{what}: wrong io source in {e:?}"
    );
}

pub fn is_dir(e: &FileEntry) -> bool {
    e.kind == EntryKind::Directory
}

pub fn file(content: &[u8]) -> MemoryNode {
    MemoryNode::File {
        content: content.to_vec(),
    }
}

/// Deterministic, non-repeating-at-any-small-period byte pattern.
pub fn pattern(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x9E37_79B9;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect()
}
