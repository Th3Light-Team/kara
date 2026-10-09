//! Regressions for the independent review of LocalBackend: a read-only file is
//! refused instead of being swapped for a new one.

mod local_backend_support;

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;

use kara_vfs::{Backend, BackendErrorKind};
use local_backend_support::{chmod, err_of, get, rooted, rp, tmp_tempdir};

fn mode_of(path: &std::path::Path) -> io::Result<u32> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o7777)
}

#[test]
fn a_read_only_file_is_not_replaced() -> io::Result<()> {
    let dir = tmp_tempdir()?;
    let backend = rooted(dir.path())?;
    let target = dir.path().join("ro");
    fs::write(&target, b"keep")?;
    chmod(&target, 0o444)?;

    let path = rp("/ro")?;
    let error = err_of(backend.begin_write(&path, None, true), "begin_write over 0444");
    assert_eq!(error.kind, BackendErrorKind::PermissionDenied);
    assert_eq!(error.path, Some(path.clone()));
    assert_eq!(get(&backend, &path, 0)?, b"keep");
    assert_eq!(mode_of(&target)?, 0o444);
    Ok(())
}
