//! cb_06..cb_13: list, stat and open_read of LocalBackend.

mod local_backend_support;

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::time::{Duration, Instant};

use kara_core::{EntryKind, FileEntry};
use kara_fs::{LocalBackend, describe};
use kara_vfs::{Backend, BackendErrorKind, Cancel, RemotePath};
use local_backend_support::{
    assert_err, assert_err_io, backend_source, code_lines, err_of, get, io_kind, mkfifo, rooted,
    rp, tmp_tempdir, within,
};

/// The entry without `accessed`: following a symlink to describe it updates
/// the link's own atime, so that one field moves between two reads.
fn comparable(entry: &FileEntry) -> FileEntry {
    let mut copy = entry.clone();
    copy.accessed = None;
    copy
}

fn entry_named<'a>(entries: &'a [FileEntry], name: &str) -> &'a FileEntry {
    match entries.iter().find(|e| e.name == OsStr::new(name)) {
        Some(entry) => entry,
        None => panic!("no entry named {name:?} in {entries:?}"),
    }
}

// ---------------------------------------------------------------------------
// cb_06

#[test]
fn cb_06_non_utf8_names_are_listed_flagged_and_never_aliased() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let dir_local = tmp.path().join("d");
    fs::create_dir(&dir_local)?;
    let raw = dir_local.join(OsStr::from_bytes(b"\xff"));
    fs::write(&raw, b"raw")?;
    let dir = rp("/d")?;

    let listing = backend.list(&dir, &Cancel::new())?;
    assert_eq!(listing.entries.len(), 1, "{:?}", listing.entries);
    let entry = &listing.entries[0];
    assert_eq!(entry.name.as_bytes(), b"\xff");
    assert_eq!(entry.display, "\u{FFFD}");
    assert_eq!(listing.errors.len(), 1, "{:?}", listing.errors);
    assert_err_io(
        &listing.errors[0],
        BackendErrorKind::Other,
        &dir,
        io::ErrorKind::InvalidData,
        "per-entry error of a non-UTF-8 name",
    );

    // The lossy display names the literal U+FFFD bytes: a different file.
    let lossy = rp("/d/\u{FFFD}")?;
    let error = err_of(backend.stat(&lossy), "stat of the lossy name");
    assert_err(&error, BackendErrorKind::NotFound, &lossy, "stat lossy");
    let error = err_of(backend.remove(&lossy), "remove of the lossy name");
    assert_err(&error, BackendErrorKind::NotFound, &lossy, "remove lossy");
    assert_eq!(fs::read(&raw)?, b"raw", "the raw file must be untouched");

    backend.remove_tree(&dir, &Cancel::new())?;
    assert!(
        fs::symlink_metadata(&dir_local).is_err(),
        "remove_tree must remove a dir holding non-UTF-8 names"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_07

#[test]
fn cb_07_list_entries_are_exactly_what_describe_returns() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let dir = tmp.path().join("d");
    fs::create_dir(&dir)?;
    fs::write(dir.join(".dot"), b"dot")?;
    fs::write(dir.join("f"), b"12345")?;
    fs::create_dir(dir.join("sub"))?;
    symlink("f", dir.join("link"))?;
    symlink("nowhere", dir.join("broken"))?;
    fs::write(dir.join(".hidden"), b"f\n")?;

    let listing = backend.list(&rp("/d")?, &Cancel::new())?;
    assert!(listing.errors.is_empty(), "{:?}", listing.errors);
    let mut got: Vec<&OsStr> = listing.entries.iter().map(|e| e.name.as_os_str()).collect();
    got.sort();
    let expected: Vec<&OsStr> = [".dot", ".hidden", "broken", "f", "link", "sub"]
        .iter()
        .map(OsStr::new)
        .collect();
    assert_eq!(
        got, expected,
        ".hidden is listed and not applied; no '.'/'..'"
    );

    let entries = &listing.entries;
    assert!(entry_named(entries, ".dot").is_hidden);
    let f = entry_named(entries, "f");
    assert_eq!(f.size, Some(5));
    assert!(
        !f.is_hidden,
        ".hidden is the UI's filter, not the backend's"
    );
    assert_eq!(f.kind, EntryKind::File);
    let sub = entry_named(entries, "sub");
    assert_eq!(sub.size, None);
    assert_eq!(sub.kind, EntryKind::Directory);
    let link = entry_named(entries, "link");
    assert!(link.is_symlink && !link.symlink_broken);
    assert_eq!(link.kind, EntryKind::File);
    let broken = entry_named(entries, "broken");
    assert!(broken.is_symlink && broken.symlink_broken);
    assert_eq!(broken.kind, EntryKind::File);

    for entry in entries {
        assert_eq!(entry.type_label, None);
        let described = match describe(&dir.join(&entry.name)) {
            Ok(described) => described,
            Err(error) => panic!("describe failed: {error}"),
        };
        assert_eq!(
            comparable(entry),
            comparable(&described),
            "list must reuse describe for {:?}",
            entry.name
        );
        assert_eq!(entry.location.as_deref(), Some(dir.as_path()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_08

#[test]
fn cb_08_list_of_a_missing_dir_is_not_found() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let missing = rp("/missing")?;
    let error = err_of(backend.list(&missing, &Cancel::new()), "list missing");
    assert_err(&error, BackendErrorKind::NotFound, &missing, "list missing");
    Ok(())
}

#[test]
fn cb_08_list_of_a_file_is_other_not_a_directory() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"x")?;
    let path = rp("/f")?;
    let error = err_of(backend.list(&path, &Cancel::new()), "list file");
    assert_err_io(
        &error,
        BackendErrorKind::Other,
        &path,
        io::ErrorKind::NotADirectory,
        "list file",
    );
    Ok(())
}

#[test]
fn cb_08_list_of_a_path_under_a_file_is_not_found() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"x")?;
    let path = rp("/f/x")?;
    let error = err_of(backend.list(&path, &Cancel::new()), "list under a file");
    assert_err(&error, BackendErrorKind::NotFound, &path, "list /f/x");
    Ok(())
}

#[test]
fn cb_08_list_of_a_fifo_is_not_a_directory_and_does_not_block() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    mkfifo(&tmp.path().join("p"))?;
    let path = rp("/p")?;
    let probe = backend.clone();
    let asked = path.clone();
    let outcome = within(Duration::from_secs(2), move || {
        probe.list(&asked, &Cancel::new()).map(|l| l.entries.len())
    });
    let Some(result) = outcome else {
        panic!("list of a FIFO blocked for more than 2 s");
    };
    let error = err_of(result, "list fifo");
    assert_err_io(
        &error,
        BackendErrorKind::Other,
        &path,
        io::ErrorKind::NotADirectory,
        "list fifo",
    );
    Ok(())
}

#[test]
fn cb_08_list_of_an_empty_root_is_ok_and_empty() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let listing = backend.list(&RemotePath::root(), &Cancel::new())?;
    assert!(listing.entries.is_empty() && listing.errors.is_empty());
    Ok(())
}

#[test]
fn cb_08_list_of_a_root_removed_after_construction_is_not_found() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let root = tmp.path().join("r");
    fs::create_dir(&root)?;
    let backend = rooted(&root)?;
    fs::remove_dir(&root)?;
    let error = err_of(
        backend.list(&RemotePath::root(), &Cancel::new()),
        "list gone root",
    );
    assert_err(
        &error,
        BackendErrorKind::NotFound,
        &RemotePath::root(),
        "gone root",
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_09

#[test]
fn cb_09_list_precancelled_is_cancelled_with_the_dir_path() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let dir = tmp.path().join("d");
    fs::create_dir(&dir)?;
    for name in ["a", "b", "c"] {
        fs::write(dir.join(name), name)?;
    }
    let cancel = Cancel::new();
    cancel.cancel();
    let path = rp("/d")?;
    let error = err_of(backend.list(&path, &cancel), "precancelled list");
    assert_err(
        &error,
        BackendErrorKind::Cancelled,
        &path,
        "precancelled list",
    );
    Ok(())
}

#[test]
fn cb_09_list_cancelled_midway_never_returns_a_partial_ok() -> io::Result<()> {
    const COUNT: usize = 20_000;
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let dir = tmp.path().join("many");
    fs::create_dir(&dir)?;
    for index in 0..COUNT {
        fs::File::create(dir.join(format!("f{index:05}")))?;
    }
    let path = rp("/many")?;
    for run in 0..5 {
        let cancel = Cancel::new();
        let worker = {
            let backend = backend.clone();
            let cancel = cancel.clone();
            let path = path.clone();
            std::thread::spawn(move || backend.list(&path, &cancel))
        };
        if run > 0 {
            std::thread::sleep(Duration::from_millis(run * 2));
        }
        cancel.cancel();
        let result = match worker.join() {
            Ok(result) => result,
            Err(_) => panic!("the listing thread panicked"),
        };
        match result {
            Ok(listing) => assert_eq!(
                listing.entries.len(),
                COUNT,
                "run {run}: a cancelled list returned Ok with a partial listing"
            ),
            Err(error) => {
                assert_err(&error, BackendErrorKind::Cancelled, &path, "cancelled list");
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_10

#[test]
fn cb_10_stat_agrees_with_the_list_entry() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"hello")?;
    fs::create_dir(tmp.path().join("sub"))?;
    symlink("f", tmp.path().join("link"))?;
    fs::write(tmp.path().join(".dot"), b"")?;
    let listing = backend.list(&RemotePath::root(), &Cancel::new())?;
    for entry in &listing.entries {
        let path = RemotePath::root()
            .join(&entry.display)
            .map_err(io::Error::other)?;
        let stat = backend.stat(&path)?;
        assert_eq!(
            comparable(&stat),
            comparable(entry),
            "stat({path}) differs from its list entry"
        );
    }
    assert_eq!(listing.entries.len(), 4);
    Ok(())
}

fn assert_is_drive_root(entry: &FileEntry) {
    assert_eq!(entry.name, OsStr::new("/"));
    assert_eq!(entry.display, "/");
    assert_eq!(entry.kind, EntryKind::Directory);
    assert!(!entry.is_hidden, "the drive root is never hidden");
    assert_eq!(entry.location, None);
}

#[test]
fn cb_10_stat_of_the_root_is_named_slash_for_any_root() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let root = tmp.path().join(".hiddenroot");
    fs::create_dir(&root)?;
    let backend = rooted(&root)?;
    assert_is_drive_root(&backend.stat(&RemotePath::root())?);
    assert_is_drive_root(&LocalBackend::system().stat(&RemotePath::root())?);
    Ok(())
}

#[test]
fn cb_10_stat_of_missing_and_of_a_path_under_a_file_is_not_found() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"x")?;
    let missing = rp("/missing")?;
    let error = err_of(backend.stat(&missing), "stat missing");
    assert_err(&error, BackendErrorKind::NotFound, &missing, "stat missing");
    let under = rp("/f/x")?;
    let error = err_of(backend.stat(&under), "stat /f/x");
    assert_err(
        &error,
        BackendErrorKind::NotFound,
        &under,
        "ENOTDIR is NotFound for stat",
    );
    Ok(())
}

#[test]
fn cb_10_stat_of_a_symlink_loop_is_a_broken_link() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    symlink("loop", tmp.path().join("loop"))?;
    let entry = backend.stat(&rp("/loop")?)?;
    assert!(entry.is_symlink);
    assert!(entry.symlink_broken);
    assert_eq!(entry.kind, EntryKind::File);
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_11

#[test]
fn cb_11_open_read_honours_offsets_up_to_the_length() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("ten"), b"0123456789")?;
    let path = rp("/ten")?;
    assert_eq!(get(&backend, &path, 0)?, b"0123456789");
    assert_eq!(get(&backend, &path, 4)?, b"456789");
    assert_eq!(get(&backend, &path, 10)?, b"");
    Ok(())
}

#[test]
fn cb_11_open_read_past_the_end_fails_at_open() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("ten"), b"0123456789")?;
    let path = rp("/ten")?;
    for from in [11, u64::MAX] {
        let error = err_of(backend.open_read(&path, from), "open_read past the end");
        assert_err_io(
            &error,
            BackendErrorKind::Other,
            &path,
            io::ErrorKind::InvalidInput,
            &format!("open_read from {from}"),
        );
    }
    Ok(())
}

#[test]
fn cb_11_open_read_of_a_directory_fails_at_open() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::create_dir(tmp.path().join("d"))?;
    let path = rp("/d")?;
    let error = err_of(backend.open_read(&path, 0), "open_read dir");
    assert_err_io(
        &error,
        BackendErrorKind::Other,
        &path,
        io::ErrorKind::IsADirectory,
        "open_read dir",
    );
    let error = err_of(backend.open_read(&RemotePath::root(), 0), "open_read root");
    assert_eq!(
        io_kind(&error),
        Some(io::ErrorKind::IsADirectory),
        "{error:?}"
    );
    Ok(())
}

#[test]
fn cb_11_open_read_of_missing_and_under_a_file_is_not_found() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"x")?;
    for text in ["/missing", "/f/x"] {
        let path = rp(text)?;
        let error = err_of(backend.open_read(&path, 0), text);
        assert_err(&error, BackendErrorKind::NotFound, &path, text);
    }
    Ok(())
}

#[test]
fn cb_11_open_read_follows_a_symlink_to_its_target() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    fs::write(tmp.path().join("f"), b"target bytes")?;
    symlink("f", tmp.path().join("link"))?;
    assert_eq!(get(&backend, &rp("/link")?, 0)?, b"target bytes");
    assert_eq!(get(&backend, &rp("/link")?, 7)?, b"bytes");
    Ok(())
}

#[test]
fn cb_11_open_read_seeks_instead_of_reading_and_discarding() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let local = tmp.path().join("sparse");
    let file = fs::File::create(&local)?;
    const LEN: u64 = 64 << 30;
    if let Err(error) = file.set_len(LEN) {
        println!("cb_11: cannot create a 64 GiB sparse file here ({error}); skipped");
        return Ok(());
    }
    {
        use std::os::unix::fs::FileExt;
        file.write_all_at(b"end", LEN - 3)?;
    }
    drop(file);
    let started = Instant::now();
    let tail = get(&backend, &rp("/sparse")?, LEN - 3)?;
    assert_eq!(tail, b"end");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "reading the last 3 bytes of a sparse file took {:?}: the offset is not a seek",
        started.elapsed()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_12

#[test]
fn cb_12_open_read_of_a_fifo_is_unsupported_and_does_not_block() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    mkfifo(&tmp.path().join("p"))?;
    symlink("p", tmp.path().join("to-p"))?;
    for text in ["/p", "/to-p"] {
        let path = rp(text)?;
        let probe = backend.clone();
        let asked = path.clone();
        let outcome = within(Duration::from_secs(2), move || {
            probe.open_read(&asked, 0).map(|_| ())
        });
        let Some(result) = outcome else {
            panic!("open_read({text}) blocked on a FIFO for more than 2 s");
        };
        let error = err_of(result, text);
        assert_err(&error, BackendErrorKind::Unsupported, &path, text);
    }
    Ok(())
}

#[test]
fn cb_12_open_read_of_dev_zero_is_unsupported() -> io::Result<()> {
    let system = LocalBackend::system();
    let path = rp("/dev/zero")?;
    let error = err_of(system.open_read(&path, 0), "/dev/zero");
    assert_err(&error, BackendErrorKind::Unsupported, &path, "/dev/zero");
    Ok(())
}

#[test]
fn cb_12_open_read_of_a_socket_is_unsupported() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let _listener = std::os::unix::net::UnixListener::bind(tmp.path().join("sock"))?;
    let path = rp("/sock")?;
    let error = err_of(backend.open_read(&path, 0), "socket");
    assert_err(&error, BackendErrorKind::Unsupported, &path, "socket");
    Ok(())
}

// ---------------------------------------------------------------------------
// cb_13

#[test]
fn cb_13_dropping_a_reader_midway_changes_nothing() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let content: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    let local = tmp.path().join("big");
    fs::write(&local, &content)?;
    let path = rp("/big")?;
    {
        let mut reader = backend.open_read(&path, 0)?;
        let mut half = vec![0u8; 5_000];
        reader.read_exact(&mut half)?;
        assert_eq!(half, content[..5_000]);
    }
    assert_eq!(backend.stat(&path)?.size, Some(10_000));
    assert_eq!(fs::read(&local)?, content);
    Ok(())
}

#[test]
fn cb_13_reader_and_session_errors_are_built_from_backend_errors() -> io::Result<()> {
    let source = backend_source()?;
    let code = code_lines(&source).join("\n");
    assert!(
        code.contains("BackendError::from_io"),
        "reader and session io errors must go through BackendError::from_io"
    );
    assert!(
        code.contains("impl Read for") || code.contains("io::Read for"),
        "the backend module must define its own reader"
    );
    assert!(
        code.contains("impl Write for") || code.contains("io::Write for"),
        "the backend module must define its own write session"
    );
    Ok(())
}
