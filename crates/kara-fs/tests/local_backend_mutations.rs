//! cb_20..cb_26: create_dir, rename, remove, remove_tree and copy_within of
//! LocalBackend.

mod local_backend_support;

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::Path;

use kara_fs::LocalBackend;
use kara_vfs::{Backend, BackendErrorKind, Cancel, RemotePath};
use local_backend_support::{
    assert_err, assert_err_io, chmod, disk_tempdir, err_of, permissions_enforced, rooted, rp,
    snapshot, tmp_tempdir,
};

// ---------------------------------------------------------------------------
// cb_20

#[test]
fn cb_20_create_dir_makes_exactly_one_level_with_the_default_mode() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    backend.create_dir(&rp("/new")?)?;
    let meta = fs::symlink_metadata(tmp.path().join("new"))?;
    assert!(meta.is_dir());
    fs::create_dir(tmp.path().join("reference"))?;
    let reference = fs::metadata(tmp.path().join("reference"))?.mode() & 0o7777;
    assert_eq!(meta.mode() & 0o7777, reference, "mode 0o777 before umask");
    assert_eq!(fs::read_dir(tmp.path().join("new"))?.count(), 0);
    Ok(())
}

#[test]
fn cb_20_create_dir_with_a_missing_parent_creates_nothing() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let path = rp("/a/b")?;
    let error = err_of(backend.create_dir(&path), "create_dir /a/b");
    assert_err(&error, BackendErrorKind::NotFound, &path, "missing parent");
    assert!(
        fs::symlink_metadata(tmp.path().join("a")).is_err(),
        "/a must not be created"
    );
    Ok(())
}

#[test]
fn cb_20_create_dir_never_lands_on_anything_existing() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("file"), b"f")?;
    fs::create_dir(tmp.path().join("dir"))?;
    fs::write(tmp.path().join("dir/child"), b"c")?;
    symlink("dir", tmp.path().join("link"))?;
    symlink("nowhere", tmp.path().join("broken"))?;
    let before = snapshot(tmp.path())?;
    for target in ["/file", "/dir", "/link", "/broken", "/"] {
        let path = rp(target)?;
        let error = err_of(backend.create_dir(&path), target);
        assert_err(&error, BackendErrorKind::AlreadyExists, &path, target);
    }
    assert_eq!(snapshot(tmp.path())?, before);
    Ok(())
}

#[test]
fn cb_20_create_dir_under_a_file_is_other_not_a_directory() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"f")?;
    let before = snapshot(tmp.path())?;
    let path = rp("/f/d")?;
    let error = err_of(backend.create_dir(&path), "create_dir /f/d");
    assert_err_io(
        &error,
        BackendErrorKind::Other,
        &path,
        io::ErrorKind::NotADirectory,
        "create_dir under a file",
    );
    assert_eq!(snapshot(tmp.path())?, before);
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_21

/// Runs a rename that must fail and checks that nothing changed.
fn refused_rename(
    tmp: &Path,
    from: &str,
    to: &str,
    kind: BackendErrorKind,
    expected_path: &str,
    source: Option<io::ErrorKind>,
) -> io::Result<()> {
    let backend = rooted(tmp)?;
    let before = snapshot(tmp)?;
    let what = format!("rename({from}, {to})");
    let error = err_of(backend.rename(&rp(from)?, &rp(to)?), &what);
    let path = rp(expected_path)?;
    match source {
        Some(io_kind) => assert_err_io(&error, kind, &path, io_kind, &what),
        None => assert_err(&error, kind, &path, &what),
    }
    assert_eq!(snapshot(tmp)?, before, "{what} changed the tree");
    Ok(())
}

#[test]
fn cb_21_rename_refuses_the_root_on_either_side() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    fs::write(tmp.path().join("x"), b"x")?;
    let invalid = Some(io::ErrorKind::InvalidInput);
    refused_rename(tmp.path(), "/", "/y", BackendErrorKind::Other, "/", invalid)?;
    refused_rename(tmp.path(), "/x", "/", BackendErrorKind::Other, "/", invalid)?;
    refused_rename(
        tmp.path(),
        "/missing",
        "/",
        BackendErrorKind::Other,
        "/",
        invalid,
    )?;
    Ok(())
}

#[test]
fn cb_21_rename_of_a_missing_source_is_not_found_naming_from() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    fs::write(tmp.path().join("exists"), b"e")?;
    fs::write(tmp.path().join("f"), b"f")?;
    refused_rename(
        tmp.path(),
        "/missing",
        "/y",
        BackendErrorKind::NotFound,
        "/missing",
        None,
    )?;
    // Checked before the destination: an existing `to` does not change the answer.
    refused_rename(
        tmp.path(),
        "/missing",
        "/exists",
        BackendErrorKind::NotFound,
        "/missing",
        None,
    )?;
    // A source under a file does not exist either (ENOTDIR -> NotFound).
    refused_rename(
        tmp.path(),
        "/f/x",
        "/y",
        BackendErrorKind::NotFound,
        "/f/x",
        None,
    )?;
    Ok(())
}

#[test]
fn cb_21_rename_onto_itself_is_a_no_op() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("x"), b"content")?;
    fs::create_dir(tmp.path().join("d"))?;
    fs::write(tmp.path().join("d/child"), b"c")?;
    let before = snapshot(tmp.path())?;
    let dir_meta = fs::metadata(tmp.path())?;
    backend.rename(&rp("/x")?, &rp("/x")?)?;
    backend.rename(&rp("/d")?, &rp("/d")?)?;
    assert_eq!(snapshot(tmp.path())?, before);
    let after = fs::metadata(tmp.path())?;
    assert_eq!(
        (dir_meta.mtime(), dir_meta.mtime_nsec()),
        (after.mtime(), after.mtime_nsec()),
        "nothing that changes the directory may run"
    );
    Ok(())
}

#[test]
fn cb_21_rename_into_its_own_subtree_is_invalid_input_naming_to() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    fs::create_dir(tmp.path().join("d"))?;
    fs::write(tmp.path().join("d/existing"), b"e")?;
    let invalid = Some(io::ErrorKind::InvalidInput);
    refused_rename(
        tmp.path(),
        "/d",
        "/d/sub",
        BackendErrorKind::Other,
        "/d/sub",
        invalid,
    )?;
    // Checked before "to exists".
    refused_rename(
        tmp.path(),
        "/d",
        "/d/existing",
        BackendErrorKind::Other,
        "/d/existing",
        invalid,
    )?;
    Ok(())
}

#[test]
fn cb_21_rename_never_overwrites_anything() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    fs::write(tmp.path().join("src"), b"source")?;
    fs::write(tmp.path().join("file"), b"f")?;
    fs::create_dir(tmp.path().join("dir"))?;
    symlink("file", tmp.path().join("link"))?;
    symlink("nowhere", tmp.path().join("broken"))?;
    for to in ["/file", "/dir", "/link", "/broken"] {
        refused_rename(
            tmp.path(),
            "/src",
            to,
            BackendErrorKind::AlreadyExists,
            to,
            None,
        )?;
    }
    fs::create_dir(tmp.path().join("srcdir"))?;
    refused_rename(
        tmp.path(),
        "/srcdir",
        "/dir",
        BackendErrorKind::AlreadyExists,
        "/dir",
        None,
    )?;
    Ok(())
}

#[test]
fn cb_21_rename_into_a_missing_or_file_parent_names_to() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    fs::write(tmp.path().join("src"), b"source")?;
    fs::write(tmp.path().join("f"), b"f")?;
    refused_rename(
        tmp.path(),
        "/src",
        "/missing/x",
        BackendErrorKind::NotFound,
        "/missing/x",
        None,
    )?;
    refused_rename(
        tmp.path(),
        "/src",
        "/f/x",
        BackendErrorKind::Other,
        "/f/x",
        Some(io::ErrorKind::NotADirectory),
    )?;
    Ok(())
}

#[test]
fn cb_21_rename_moves_a_symlink_as_a_link() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::create_dir(tmp.path().join("target"))?;
    fs::write(tmp.path().join("target/child"), b"c")?;
    symlink("target", tmp.path().join("link"))?;
    backend.rename(&rp("/link")?, &rp("/l2")?)?;
    let meta = fs::symlink_metadata(tmp.path().join("l2"))?;
    assert!(meta.file_type().is_symlink());
    assert_eq!(fs::read_link(tmp.path().join("l2"))?, Path::new("target"));
    assert!(fs::symlink_metadata(tmp.path().join("link")).is_err());
    assert_eq!(fs::read(tmp.path().join("target/child"))?, b"c");
    Ok(())
}

#[test]
fn cb_21_rename_moves_files_and_subtrees() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::create_dir_all(tmp.path().join("d/inner"))?;
    fs::write(tmp.path().join("d/inner/f"), b"deep")?;
    let inode = fs::metadata(tmp.path().join("d/inner/f"))?.ino();
    fs::create_dir(tmp.path().join("other"))?;
    backend.rename(&rp("/d")?, &rp("/other/moved")?)?;
    let moved = tmp.path().join("other/moved/inner/f");
    assert_eq!(fs::read(&moved)?, b"deep");
    assert_eq!(fs::metadata(&moved)?.ino(), inode, "a rename, not a copy");
    assert!(fs::symlink_metadata(tmp.path().join("d")).is_err());
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_22

#[test]
fn cb_22_rename_across_filesystems_fails_instead_of_copying() -> io::Result<()> {
    let disk = disk_tempdir()?;
    let disk_dev = fs::metadata(disk.path())?.dev();
    let mut other = None;
    for base in ["/tmp", "/dev/shm"] {
        let Ok(meta) = fs::metadata(base) else {
            continue;
        };
        if meta.dev() != disk_dev {
            if let Ok(dir) = tempfile::tempdir_in(base) {
                other = Some(dir);
                break;
            }
        }
    }
    let Some(other) = other else {
        println!("cb_22: no second filesystem next to CARGO_TARGET_TMPDIR; skipped");
        return Ok(());
    };
    let system = LocalBackend::system();
    let from_local = disk.path().join("f");
    fs::write(&from_local, b"stays")?;
    let to_local = other.path().join("f");
    let from = system.to_remote(&from_local).map_err(io::Error::other)?;
    let to = system.to_remote(&to_local).map_err(io::Error::other)?;
    let error = err_of(system.rename(&from, &to), "rename across devices");
    assert_err_io(
        &error,
        BackendErrorKind::Other,
        &from,
        io::ErrorKind::CrossesDevices,
        "EXDEV",
    );
    assert_eq!(fs::read(&from_local)?, b"stays");
    assert!(
        fs::symlink_metadata(&to_local).is_err(),
        "nothing may be copied"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_23

#[test]
fn cb_23_remove_takes_files_links_and_empty_dirs() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"f")?;
    fs::create_dir(tmp.path().join("empty"))?;
    symlink("nowhere", tmp.path().join("broken"))?;
    for target in ["/f", "/empty", "/broken"] {
        backend.remove(&rp(target)?)?;
    }
    assert_eq!(fs::read_dir(tmp.path())?.count(), 0);
    Ok(())
}

#[test]
fn cb_23_remove_of_a_link_to_a_dir_removes_only_the_link() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::create_dir(tmp.path().join("target"))?;
    fs::write(tmp.path().join("target/child"), b"kept")?;
    symlink("target", tmp.path().join("link"))?;
    backend.remove(&rp("/link")?)?;
    assert!(fs::symlink_metadata(tmp.path().join("link")).is_err());
    assert_eq!(fs::read(tmp.path().join("target/child"))?, b"kept");
    Ok(())
}

#[test]
fn cb_23_remove_of_a_non_empty_dir_removes_nothing() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::create_dir(tmp.path().join("d"))?;
    fs::write(tmp.path().join("d/child"), b"readable")?;
    let before = snapshot(tmp.path())?;
    let path = rp("/d")?;
    let error = err_of(backend.remove(&path), "remove non-empty dir");
    assert_err_io(
        &error,
        BackendErrorKind::Other,
        &path,
        io::ErrorKind::DirectoryNotEmpty,
        "remove /d",
    );
    assert_eq!(snapshot(tmp.path())?, before);
    assert_eq!(fs::read(tmp.path().join("d/child"))?, b"readable");
    Ok(())
}

#[test]
fn cb_23_remove_of_missing_or_under_a_file_is_not_found() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"f")?;
    let before = snapshot(tmp.path())?;
    for target in ["/missing", "/f/x"] {
        let path = rp(target)?;
        let error = err_of(backend.remove(&path), target);
        assert_err(&error, BackendErrorKind::NotFound, &path, target);
    }
    assert_eq!(snapshot(tmp.path())?, before);
    Ok(())
}

#[test]
fn cb_23_remove_of_the_root_is_refused() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let root = tmp.path().join("r");
    fs::create_dir(&root)?;
    let backend = rooted(&root)?;
    let before = snapshot(tmp.path())?;
    let error = err_of(backend.remove(&RemotePath::root()), "remove root");
    assert_err_io(
        &error,
        BackendErrorKind::Other,
        &RemotePath::root(),
        io::ErrorKind::InvalidInput,
        "remove root",
    );
    assert_eq!(snapshot(tmp.path())?, before);
    assert!(root.is_dir(), "an empty root must survive remove(\"/\")");
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_24

#[test]
fn cb_24_remove_tree_unlinks_symlinks_and_never_follows_them() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let root = tmp.path().join("root");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(root.join("tree/sub"))?;
    fs::create_dir(&outside)?;
    fs::write(outside.join("precious"), b"keep")?;
    fs::write(root.join("tree/sub/f"), b"f")?;
    symlink(&outside, root.join("tree/sub/link"))?;
    let backend = rooted(&root)?;
    backend.remove_tree(&rp("/tree")?, &Cancel::new())?;
    assert!(fs::symlink_metadata(root.join("tree")).is_err());
    assert_eq!(fs::read(outside.join("precious"))?, b"keep");
    Ok(())
}

#[test]
fn cb_24_remove_tree_of_a_single_file_and_of_missing() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"f")?;
    backend.remove_tree(&rp("/f")?, &Cancel::new())?;
    assert!(fs::symlink_metadata(tmp.path().join("f")).is_err());
    let missing = rp("/missing")?;
    let error = err_of(backend.remove_tree(&missing, &Cancel::new()), "missing");
    assert_err(
        &error,
        BackendErrorKind::NotFound,
        &missing,
        "remove_tree missing",
    );
    Ok(())
}

#[test]
fn cb_24_remove_tree_precancelled_removes_nothing() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"f")?;
    fs::create_dir(tmp.path().join("d"))?;
    fs::write(tmp.path().join("d/child"), b"c")?;
    let before = snapshot(tmp.path())?;
    let cancel = Cancel::new();
    cancel.cancel();
    for target in ["/f", "/d", "/missing"] {
        let path = rp(target)?;
        let error = err_of(backend.remove_tree(&path, &cancel), target);
        assert_err(&error, BackendErrorKind::Cancelled, &path, target);
    }
    assert_eq!(snapshot(tmp.path())?, before);
    Ok(())
}

#[test]
fn cb_24_remove_tree_of_the_root_is_refused() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let root = tmp.path().join("r");
    fs::create_dir(&root)?;
    fs::write(root.join("f"), b"f")?;
    let backend = rooted(&root)?;
    let before = snapshot(tmp.path())?;
    let error = err_of(
        backend.remove_tree(&RemotePath::root(), &Cancel::new()),
        "remove_tree root",
    );
    assert_err_io(
        &error,
        BackendErrorKind::Other,
        &RemotePath::root(),
        io::ErrorKind::InvalidInput,
        "remove_tree root",
    );
    assert_eq!(snapshot(tmp.path())?, before);
    Ok(())
}

#[test]
fn cb_24_remove_tree_cancelled_midway_is_never_ok_with_the_root_present() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let local = tmp.path().join("big");
    for group in 0..50 {
        let dir = local.join(format!("g{group:02}"));
        fs::create_dir_all(&dir)?;
        for index in 0..100 {
            fs::write(dir.join(format!("f{index:03}")), b"x")?;
        }
    }
    let path = rp("/big")?;
    let cancel = Cancel::new();
    let worker = {
        let backend = backend.clone();
        let cancel = cancel.clone();
        let path = path.clone();
        std::thread::spawn(move || backend.remove_tree(&path, &cancel))
    };
    cancel.cancel();
    let result = match worker.join() {
        Ok(result) => result,
        Err(_) => panic!("remove_tree panicked"),
    };
    match result {
        Ok(()) => assert!(
            fs::symlink_metadata(&local).is_err(),
            "remove_tree returned Ok with the tree still present"
        ),
        Err(error) => {
            assert_err(
                &error,
                BackendErrorKind::Cancelled,
                &path,
                "cancelled remove_tree",
            );
        }
    }
    Ok(())
}

#[test]
fn cb_24_remove_tree_reports_the_descendant_that_failed() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let locked = tmp.path().join("tree/locked");
    fs::create_dir_all(&locked)?;
    fs::write(locked.join("file"), b"f")?;
    chmod(&locked, 0o555)?;
    if !permissions_enforced(&locked) {
        println!("cb_24: running as root, permission checks do not apply; skipped");
        chmod(&locked, 0o755)?;
        return Ok(());
    }
    let result = backend.remove_tree(&rp("/tree")?, &Cancel::new());
    chmod(&locked, 0o755)?;
    let error = err_of(result, "remove_tree with a read-only child");
    assert_err(
        &error,
        BackendErrorKind::PermissionDenied,
        &rp("/tree/locked/file")?,
        "the descendant that could not be removed",
    );
    assert_eq!(fs::read(locked.join("file"))?, b"f");
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_25

#[test]
fn cb_25_remove_tree_refuses_a_mount_point() -> io::Result<()> {
    let root_dev = fs::metadata("/")?.dev();
    let proc_dev = match fs::metadata("/proc") {
        Ok(meta) => meta.dev(),
        Err(_) => root_dev,
    };
    if proc_dev == root_dev {
        println!("cb_25: /proc is not a separate mount here; skipped");
        return Ok(());
    }
    let system = LocalBackend::system();
    let path = rp("/proc")?;
    let error = err_of(
        system.remove_tree(&path, &Cancel::new()),
        "remove_tree /proc",
    );
    assert_err_io(
        &error,
        BackendErrorKind::Other,
        &path,
        io::ErrorKind::InvalidInput,
        "mount point",
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_26

#[test]
fn cb_26_copy_within_is_unsupported_and_touches_nothing() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("existing"), b"e")?;
    let before = snapshot(tmp.path())?;
    let from = rp("/existing")?;
    let error = err_of(backend.copy_within(&from, &rp("/new")?), "copy_within");
    assert_err(&error, BackendErrorKind::Unsupported, &from, "copy_within");
    let missing = rp("/missing")?;
    let error = err_of(
        backend.copy_within(&missing, &rp("/x")?),
        "copy_within missing",
    );
    assert_err(
        &error,
        BackendErrorKind::Unsupported,
        &missing,
        "not NotFound",
    );
    assert_eq!(snapshot(tmp.path())?, before);
    assert!(!backend.capabilities().server_side_copy);
    Ok(())
}
