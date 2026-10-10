//! Helpers shared by the `local_backend_*` test files (contract
//! `localbackend-kara-fs-implements-kara-vfs-backend-step-2`).
//!
//! Every test works inside its own tempdir: `tempfile::tempdir()` (under /tmp)
//! or [`disk_tempdir`] (under `CARGO_TARGET_TMPDIR`). Nothing here trashes
//! anything or touches a path outside those directories.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use kara_fs::LocalBackend;
use kara_vfs::{Backend, BackendError, BackendErrorKind, RemotePath};

/// A tempdir on the disk filesystem that holds the build (`CARGO_TARGET_TMPDIR`).
pub fn disk_tempdir() -> io::Result<tempfile::TempDir> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(&base)?;
    tempfile::tempdir_in(base)
}

/// A tempdir under /tmp (tmpfs on CI).
pub fn tmp_tempdir() -> io::Result<tempfile::TempDir> {
    tempfile::tempdir()
}

/// Parses a RemotePath, turning a refusal into an io::Error.
pub fn rp(text: &str) -> io::Result<RemotePath> {
    RemotePath::parse(text)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("{text:?}: {e}")))
}

/// A backend rooted at `dir`.
pub fn rooted(dir: &Path) -> io::Result<LocalBackend> {
    LocalBackend::with_root(dir).map_err(io::Error::from)
}

/// The error of `result`, or a panic naming what was expected.
pub fn err_of<T>(result: Result<T, BackendError>, what: &str) -> BackendError {
    match result {
        Ok(_) => panic!("{what}: expected an error, got Ok"),
        Err(error) => error,
    }
}

/// The io::ErrorKind of the error's source, when the source is an io::Error.
pub fn io_kind(error: &BackendError) -> Option<io::ErrorKind> {
    let source = error.source.as_ref()?;
    source.downcast_ref::<io::Error>().map(io::Error::kind)
}

/// Asserts kind and path of an error.
pub fn assert_err(error: &BackendError, kind: BackendErrorKind, path: &RemotePath, what: &str) {
    assert_eq!(error.kind, kind, "{what}: wrong kind in {error:?}");
    assert_eq!(
        error.path.as_ref(),
        Some(path),
        "{what}: wrong path in {error:?}"
    );
}

/// Asserts kind, path and the io::ErrorKind of the source.
pub fn assert_err_io(
    error: &BackendError,
    kind: BackendErrorKind,
    path: &RemotePath,
    source: io::ErrorKind,
    what: &str,
) {
    assert_err(error, kind, path, what);
    assert_eq!(
        io_kind(error),
        Some(source),
        "{what}: the source should be an io::Error of kind {source:?}, got {error:?}"
    );
}

/// What a node of a local tree is, for before/after comparisons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    File { bytes: Vec<u8>, ino: u64, mode: u32 },
    Dir { mode: u32 },
    Link { target: PathBuf },
    Special,
}

/// Recursive snapshot of `dir` (relative raw paths), never following links.
pub fn snapshot(dir: &Path) -> io::Result<BTreeMap<PathBuf, Node>> {
    let mut out = BTreeMap::new();
    walk(dir, Path::new(""), &mut out)?;
    Ok(out)
}

fn walk(base: &Path, rel: &Path, out: &mut BTreeMap<PathBuf, Node>) -> io::Result<()> {
    for item in fs::read_dir(base.join(rel))? {
        let item = item?;
        let rel_child = rel.join(item.file_name());
        let full = base.join(&rel_child);
        let meta = fs::symlink_metadata(&full)?;
        let file_type = meta.file_type();
        let node = if file_type.is_symlink() {
            Node::Link {
                target: fs::read_link(&full)?,
            }
        } else if file_type.is_dir() {
            walk(base, &rel_child, out)?;
            Node::Dir {
                mode: meta.mode() & 0o7777,
            }
        } else if file_type.is_file() {
            Node::File {
                bytes: fs::read(&full)?,
                ino: meta.ino(),
                mode: meta.mode() & 0o7777,
            }
        } else {
            Node::Special
        };
        out.insert(rel_child, node);
    }
    Ok(())
}

/// Raw names directly inside `dir`, sorted.
pub fn names(dir: &Path) -> io::Result<Vec<OsString>> {
    let mut found = Vec::new();
    for item in fs::read_dir(dir)? {
        found.push(item?.file_name());
    }
    found.sort();
    Ok(found)
}

/// Names inside `dir` that look like a LocalBackend temporary.
pub fn temp_names(dir: &Path) -> io::Result<Vec<OsString>> {
    Ok(names(dir)?
        .into_iter()
        .filter(|name| {
            let bytes = name.as_bytes();
            bytes.starts_with(b".") && bytes.ends_with(b".kara-part")
        })
        .collect())
}

/// Writes a whole file through the backend.
pub fn put(
    backend: &dyn Backend,
    path: &RemotePath,
    bytes: &[u8],
    replace: bool,
) -> io::Result<()> {
    let mut session = backend.begin_write(path, Some(bytes.len() as u64), replace)?;
    session.write_all(bytes)?;
    session.finish()?;
    Ok(())
}

/// Reads a whole file through the backend.
pub fn get(backend: &dyn Backend, path: &RemotePath, from: u64) -> io::Result<Vec<u8>> {
    let mut reader = backend.open_read(path, from)?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Makes a FIFO at `path`.
pub fn mkfifo(path: &Path) -> io::Result<()> {
    rustix::fs::mknodat(
        rustix::fs::CWD,
        path,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )
    .map_err(io::Error::from)
}

/// Runs `work` on a thread and waits at most `limit` for it. `None` means it
/// did not finish in time (the thread is left behind, blocked).
pub fn within<T: Send + 'static>(
    limit: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(limit).ok()
}

/// Whether the current user is subject to permission checks. Creates and
/// removes one probe file inside `read_only_dir`, which must already be 0o555.
pub fn permissions_enforced(read_only_dir: &Path) -> bool {
    let probe = read_only_dir.join("probe");
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            false
        }
        Err(_) => true,
    }
}

/// Sets the permission bits of `path`.
pub fn chmod(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// The text of every `.rs` file of the backend module (src/backend.rs and/or
/// src/backend/**), concatenated.
pub fn backend_source() -> io::Result<String> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut text = String::new();
    let single = src.join("backend.rs");
    if single.is_file() {
        text.push_str(&fs::read_to_string(&single)?);
    }
    let folder = src.join("backend");
    if folder.is_dir() {
        let mut stack = vec![folder];
        while let Some(dir) = stack.pop() {
            for item in fs::read_dir(&dir)? {
                let path = item?.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    text.push_str(&fs::read_to_string(&path)?);
                    text.push('\n');
                }
            }
        }
    }
    if text.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no backend module under src/",
        ));
    }
    Ok(text)
}

/// The source without comment lines (`//`, `///`, `//!`).
pub fn code_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect()
}

/// Whether `path` names a FIFO.
pub fn is_fifo(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_fifo())
}
