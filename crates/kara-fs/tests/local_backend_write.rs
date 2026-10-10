//! cb_14..cb_19: begin_write, finish, abort and drop of LocalBackend.

mod local_backend_support;

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::Path;

use kara_vfs::{Backend, BackendError, BackendErrorKind, Cancel, RemotePath};
use local_backend_support::{
    assert_err, assert_err_io, backend_source, chmod, code_lines, err_of, get, mkfifo,
    permissions_enforced, put, rooted, rp, snapshot, temp_names, tmp_tempdir,
};

fn session_temp(dir: &Path, before: &[OsString]) -> io::Result<Vec<OsString>> {
    Ok(local_backend_support::names(dir)?
        .into_iter()
        .filter(|name| !before.contains(name))
        .collect())
}

// ---------------------------------------------------------------------------
// cb_14

#[test]
fn cb_14_begin_write_creates_a_hidden_sibling_temp_and_never_the_final_name() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::create_dir(tmp.path().join("d"))?;
    fs::write(tmp.path().join("d/other"), b"o")?;
    let before = local_backend_support::names(&tmp.path().join("d"))?;
    let target = rp("/d/n")?;

    let mut session = backend.begin_write(&target, None, false)?;
    // Created at once, before any byte is written.
    let extra = session_temp(&tmp.path().join("d"), &before)?;
    assert_eq!(
        extra.len(),
        1,
        "exactly one temporary sibling, got {extra:?}"
    );
    session.write_all(b"payload")?;

    let error = err_of(backend.stat(&target), "stat during the session");
    assert_err(
        &error,
        BackendErrorKind::NotFound,
        &target,
        "invisible before finish",
    );

    let listing = backend.list(&rp("/d")?, &Cancel::new())?;
    assert_eq!(listing.entries.len(), 2, "{:?}", listing.entries);
    let temp = match listing.entries.iter().find(|e| e.name != "other") {
        Some(entry) => entry,
        None => panic!("the temporary is not listed"),
    };
    assert!(temp.is_hidden);
    assert!(temp.display.starts_with('.'), "{}", temp.display);
    assert!(temp.display.ends_with(".kara-part"), "{}", temp.display);
    assert!(temp.name.len() <= 255);
    assert_eq!(temp.name, extra[0]);
    assert!(
        fs::symlink_metadata(tmp.path().join("n")).is_err(),
        "nothing is created outside the target's directory"
    );

    session.finish()?;
    assert_eq!(get(&backend, &target, 0)?, b"payload");
    assert_eq!(
        local_backend_support::names(&tmp.path().join("d"))?,
        vec![OsString::from("n"), OsString::from("other")]
    );
    Ok(())
}

#[test]
fn cb_14_two_sessions_on_one_target_use_different_temps() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let target = rp("/t")?;
    let first = backend.begin_write(&target, None, true)?;
    let second = backend.begin_write(&target, None, true)?;
    let temps = temp_names(tmp.path())?;
    assert_eq!(temps.len(), 2, "two different temporaries, got {temps:?}");
    assert_ne!(temps[0], temps[1]);
    first.abort()?;
    second.abort()?;
    assert!(temp_names(tmp.path())?.is_empty());
    Ok(())
}

fn long_name_round_trip(name: &str) -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let target = RemotePath::root().join(name).map_err(io::Error::other)?;
    let mut session = backend.begin_write(&target, None, false)?;
    let temps = temp_names(tmp.path())?;
    assert_eq!(temps.len(), 1, "{temps:?}");
    assert!(
        temps[0].len() <= 255,
        "temp name of {} bytes",
        temps[0].len()
    );
    assert!(
        temps[0].to_str().is_some(),
        "the target part of the temp name is cut on a char boundary"
    );
    session.write_all(b"long")?;
    session.finish()?;
    assert_eq!(get(&backend, &target, 0)?, b"long");
    assert_eq!(fs::read(tmp.path().join(name))?, b"long");
    assert!(temp_names(tmp.path())?.is_empty());
    Ok(())
}

#[test]
fn cb_14_a_255_byte_ascii_target_name_writes_and_reads_back() -> io::Result<()> {
    long_name_round_trip(&"a".repeat(255))
}

#[test]
fn cb_14_a_254_byte_multibyte_target_name_writes_and_reads_back() -> io::Result<()> {
    long_name_round_trip(&"\u{e9}".repeat(127))
}

// ---------------------------------------------------------------------------
// cb_15

fn assert_refused(
    tmp: &Path,
    target: &str,
    replace: bool,
    kind: BackendErrorKind,
    source: Option<io::ErrorKind>,
) -> io::Result<()> {
    let backend = rooted(tmp)?;
    let path = rp(target)?;
    let before = snapshot(tmp)?;
    let error = err_of(
        backend.begin_write(&path, None, replace),
        &format!("begin_write({target}, replace={replace})"),
    );
    let what = format!("begin_write({target}, replace={replace})");
    match source {
        Some(io_kind) => assert_err_io(&error, kind, &path, io_kind, &what),
        None => assert_err(&error, kind, &path, &what),
    }
    assert!(
        !error.to_string().contains(".kara-part"),
        "{what}: the error names the temp: {error}"
    );
    assert_eq!(
        snapshot(tmp)?,
        before,
        "{what}: a refused begin_write changed the tree"
    );
    Ok(())
}

#[test]
fn cb_15_begin_write_onto_a_directory_is_already_exists_even_with_replace() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    fs::create_dir(tmp.path().join("d"))?;
    fs::write(tmp.path().join("d/child"), b"c")?;
    for replace in [false, true] {
        assert_refused(
            tmp.path(),
            "/d",
            replace,
            BackendErrorKind::AlreadyExists,
            None,
        )?;
        assert_refused(
            tmp.path(),
            "/",
            replace,
            BackendErrorKind::AlreadyExists,
            None,
        )?;
    }
    Ok(())
}

#[test]
fn cb_15_begin_write_without_replace_refuses_anything_existing() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    fs::write(tmp.path().join("file"), b"f")?;
    symlink("file", tmp.path().join("link"))?;
    symlink("nowhere", tmp.path().join("broken"))?;
    mkfifo(&tmp.path().join("fifo"))?;
    for target in ["/file", "/link", "/broken", "/fifo"] {
        assert_refused(
            tmp.path(),
            target,
            false,
            BackendErrorKind::AlreadyExists,
            None,
        )?;
    }
    Ok(())
}

#[test]
fn cb_15_begin_write_into_a_missing_parent_is_not_found_naming_the_target() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    for replace in [false, true] {
        assert_refused(
            tmp.path(),
            "/missing/t",
            replace,
            BackendErrorKind::NotFound,
            None,
        )?;
    }
    Ok(())
}

#[test]
fn cb_15_begin_write_under_a_file_is_other_not_a_directory() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    fs::write(tmp.path().join("f"), b"f")?;
    for replace in [false, true] {
        assert_refused(
            tmp.path(),
            "/f/t",
            replace,
            BackendErrorKind::Other,
            Some(io::ErrorKind::NotADirectory),
        )?;
    }
    Ok(())
}

#[test]
fn cb_15_begin_write_into_a_read_only_dir_is_permission_denied() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let ro = tmp.path().join("ro");
    fs::create_dir(&ro)?;
    chmod(&ro, 0o555)?;
    if !permissions_enforced(&ro) {
        println!("cb_15: running as root, permission checks do not apply; skipped");
        chmod(&ro, 0o755)?;
        return Ok(());
    }
    let result = assert_refused(
        tmp.path(),
        "/ro/t",
        false,
        BackendErrorKind::PermissionDenied,
        None,
    );
    chmod(&ro, 0o755)?;
    result
}

// ---------------------------------------------------------------------------
// cb_16

#[test]
fn cb_16_the_loser_of_a_no_replace_race_gets_already_exists() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let target = rp("/t")?;
    let mut first = backend.begin_write(&target, None, false)?;
    first.write_all(b"first")?;
    let mut second = backend.begin_write(&target, None, false)?;
    second.write_all(b"second")?;
    second.finish()?;
    let error = err_of(first.finish(), "finish of the loser");
    assert_err(
        &error,
        BackendErrorKind::AlreadyExists,
        &target,
        "race at finish",
    );
    assert_eq!(fs::read(tmp.path().join("t"))?, b"second");
    assert_eq!(
        local_backend_support::names(tmp.path())?,
        vec![OsString::from("t")],
        "no .kara-part may be left"
    );
    Ok(())
}

#[test]
fn cb_16_a_broken_symlink_appearing_at_the_target_wins_over_no_replace() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let target = rp("/t")?;
    let mut session = backend.begin_write(&target, None, false)?;
    session.write_all(b"mine")?;
    symlink("nowhere", tmp.path().join("t"))?;
    let error = err_of(session.finish(), "finish onto a broken link");
    assert_err(
        &error,
        BackendErrorKind::AlreadyExists,
        &target,
        "broken link at finish",
    );
    assert_eq!(fs::read_link(tmp.path().join("t"))?, Path::new("nowhere"));
    assert!(temp_names(tmp.path())?.is_empty());
    Ok(())
}

#[test]
fn cb_16_a_target_that_became_a_directory_is_not_replaced() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let local = tmp.path().join("t");
    fs::write(&local, b"old")?;
    let target = rp("/t")?;
    let mut session = backend.begin_write(&target, None, true)?;
    session.write_all(b"new")?;
    fs::remove_file(&local)?;
    fs::create_dir(&local)?;
    fs::write(local.join("child"), b"kept")?;
    let error = err_of(session.finish(), "finish onto a directory");
    assert_err(
        &error,
        BackendErrorKind::AlreadyExists,
        &target,
        "dir at finish",
    );
    assert!(fs::symlink_metadata(&local)?.is_dir());
    assert_eq!(fs::read(local.join("child"))?, b"kept");
    assert!(
        temp_names(tmp.path())?.is_empty(),
        "the temp must be removed"
    );
    Ok(())
}

#[test]
fn cb_16_finish_syncs_the_data_before_the_rename() -> io::Result<()> {
    let source = backend_source()?;
    let code = code_lines(&source).join("\n");
    assert!(
        code.contains("sync_all(") || code.contains("sync_data("),
        "finish must fsync the temp before renaming it"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_17

#[test]
fn cb_17_replace_swaps_atomically_and_open_readers_keep_the_old_bytes() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let local = tmp.path().join("t");
    fs::write(&local, b"old content")?;
    let target = rp("/t")?;
    let mut session = backend.begin_write(&target, Some(3), true)?;
    session.write_all(b"NEW")?;
    assert_eq!(backend.stat(&target)?.size, Some(11));
    assert_eq!(get(&backend, &target, 0)?, b"old content");
    let mut reader = backend.open_read(&target, 0)?;
    session.finish()?;
    assert_eq!(get(&backend, &target, 0)?, b"NEW");
    assert_eq!(backend.stat(&target)?.size, Some(3));
    let mut old = Vec::new();
    reader.read_to_end(&mut old)?;
    assert_eq!(
        old, b"old content",
        "a reader opened before finish sees the old inode"
    );
    assert!(temp_names(tmp.path())?.is_empty());
    Ok(())
}

#[test]
fn cb_17_replacing_a_symlink_replaces_the_link_not_its_target() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"keep")?;
    symlink("f", tmp.path().join("l"))?;
    put(&backend, &rp("/l")?, b"new bytes", true)?;
    let meta = fs::symlink_metadata(tmp.path().join("l"))?;
    assert!(meta.is_file(), "the link became a regular file");
    assert_eq!(fs::read(tmp.path().join("l"))?, b"new bytes");
    assert_eq!(fs::read(tmp.path().join("f"))?, b"keep");
    Ok(())
}

#[test]
fn cb_17_replacing_a_hard_linked_file_detaches_it() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("t"), b"old")?;
    fs::hard_link(tmp.path().join("t"), tmp.path().join("h"))?;
    put(&backend, &rp("/t")?, b"new", true)?;
    assert_eq!(fs::read(tmp.path().join("t"))?, b"new");
    assert_eq!(fs::read(tmp.path().join("h"))?, b"old");
    Ok(())
}

#[test]
fn cb_17_the_new_file_gets_the_default_creation_mode() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let local = tmp.path().join("t");
    fs::write(&local, b"old")?;
    chmod(&local, 0o700)?;
    put(&backend, &rp("/t")?, b"new", true)?;
    fs::File::create(tmp.path().join("reference"))?;
    let reference = fs::metadata(tmp.path().join("reference"))?.mode() & 0o7777;
    assert_eq!(fs::metadata(&local)?.mode() & 0o7777, reference);
    put(&backend, &rp("/fresh")?, b"x", false)?;
    assert_eq!(
        fs::metadata(tmp.path().join("fresh"))?.mode() & 0o7777,
        reference
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_18

fn ends_leave_nothing(
    end: fn(Box<dyn kara_vfs::WriteSession>) -> io::Result<()>,
) -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("old"), b"old content")?;
    let before = snapshot(tmp.path())?;

    let mut fresh = backend.begin_write(&rp("/new")?, None, false)?;
    fresh.write_all(&[7u8; 5000])?;
    end(fresh)?;
    assert_eq!(snapshot(tmp.path())?, before, "a new target left something");

    let mut replacing = backend.begin_write(&rp("/old")?, None, true)?;
    replacing.write_all(b"replacement")?;
    end(replacing)?;
    assert_eq!(snapshot(tmp.path())?, before, "a replace target changed");
    Ok(())
}

#[test]
fn cb_18_abort_leaves_the_directory_as_it_was() -> io::Result<()> {
    ends_leave_nothing(|session| session.abort().map_err(io::Error::from))
}

#[test]
fn cb_18_dropping_a_session_leaves_the_directory_as_it_was() -> io::Result<()> {
    ends_leave_nothing(|session| {
        drop(session);
        Ok(())
    })
}

#[test]
fn cb_18_abort_of_a_temp_already_gone_is_ok() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let mut session = backend.begin_write(&rp("/t")?, None, false)?;
    session.write_all(b"x")?;
    for temp in temp_names(tmp.path())? {
        fs::remove_file(tmp.path().join(temp))?;
    }
    session.abort()?;
    assert!(local_backend_support::names(tmp.path())?.is_empty());
    Ok(())
}

/// Writes 1 MiB chunks until the filesystem refuses one; `None` if 8 GiB went in.
fn fill_until_error(session: &mut Box<dyn kara_vfs::WriteSession>) -> Option<io::Error> {
    let chunk = vec![0x5au8; 1 << 20];
    for _ in 0..8192 {
        if let Err(error) = session.write_all(&chunk) {
            return Some(error);
        }
    }
    None
}

#[test]
fn cb_18_a_failed_write_poisons_the_session() -> io::Result<()> {
    let Some(full) = std::env::var_os("KARA_TEST_FULL_FS") else {
        println!("cb_18: KARA_TEST_FULL_FS is not set, the poisoning case is skipped");
        return Ok(());
    };
    let tmp = tempfile::tempdir_in(full)?;
    let backend = rooted(tmp.path())?;
    let target = rp("/big")?;
    let mut session = match backend.begin_write(&target, None, false) {
        Ok(session) => session,
        Err(error) => {
            assert_err(
                &error,
                BackendErrorKind::NoSpace,
                &target,
                "begin on a full fs",
            );
            assert!(local_backend_support::names(tmp.path())?.is_empty());
            return Ok(());
        }
    };
    let Some(first) = fill_until_error(&mut session) else {
        println!("cb_18: the filesystem took 8 GiB without failing; skipped");
        drop(session);
        return Ok(());
    };
    assert_eq!(
        BackendError::from_io(first, None).kind,
        BackendErrorKind::NoSpace
    );
    let again = match session.write_all(b"x") {
        Ok(()) => panic!("a poisoned session accepted a later write"),
        Err(error) => error,
    };
    assert_eq!(
        BackendError::from_io(again, None).kind,
        BackendErrorKind::NoSpace
    );
    let error = err_of(session.finish(), "finish of a poisoned session");
    assert_err(
        &error,
        BackendErrorKind::NoSpace,
        &target,
        "poisoned finish",
    );
    assert!(
        local_backend_support::names(tmp.path())?.is_empty(),
        "neither target nor temp may remain"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_19

#[test]
fn cb_19_write_errors_keep_kind_and_name_the_target() -> io::Result<()> {
    let Some(full) = std::env::var_os("KARA_TEST_FULL_FS") else {
        println!("cb_19: KARA_TEST_FULL_FS is not set, the full-filesystem case is skipped");
        return Ok(());
    };
    let tmp = tempfile::tempdir_in(full)?;
    let backend = rooted(tmp.path())?;
    let target = rp("/big")?;
    let Ok(mut session) = backend.begin_write(&target, None, false) else {
        println!("cb_19: begin_write already failed on the full filesystem; skipped");
        return Ok(());
    };
    let Some(error) = fill_until_error(&mut session) else {
        println!("cb_19: the filesystem took 8 GiB without failing; skipped");
        return Ok(());
    };
    let mapped = BackendError::from_io(error, None);
    assert_eq!(mapped.kind, BackendErrorKind::NoSpace, "{mapped:?}");
    assert_eq!(mapped.path, Some(target), "the target, never the temp");
    assert!(!mapped.to_string().contains(".kara-part"));
    Ok(())
}

#[test]
fn cb_19_write_errors_go_through_backend_error() -> io::Result<()> {
    let source = backend_source()?;
    let code = code_lines(&source).join("\n");
    assert!(
        code.contains("BackendError::from_io"),
        "write/flush errors must be io::Error::from(BackendError::from_io(e, Some(target)))"
    );
    Ok(())
}

#[test]
fn cb_19_errors_of_a_session_name_the_target_never_the_temp() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let target = rp("/t")?;
    let mut session = backend.begin_write(&target, None, false)?;
    session.write_all(b"x")?;
    fs::write(tmp.path().join("t"), b"winner")?;
    let error = err_of(session.finish(), "finish loses");
    assert_err(
        &error,
        BackendErrorKind::AlreadyExists,
        &target,
        "finish loses",
    );
    let shown = error.to_string();
    assert!(!shown.contains(".kara-part"), "{shown}");
    assert!(!shown.contains(&*tmp.path().to_string_lossy()), "{shown}");
    assert_eq!(fs::read(tmp.path().join("t"))?, b"winner");
    assert!(temp_names(tmp.path())?.is_empty());
    Ok(())
}
