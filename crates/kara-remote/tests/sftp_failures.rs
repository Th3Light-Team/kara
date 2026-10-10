//! What the SFTP backend does when the server or the network misbehaves:
//! dropped connections, stalls, keepalive, full disks, denied paths, cancel.

mod support;

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use kara_vfs::{Backend, BackendError, BackendErrorKind, Cancel, RemotePath};
use support::{Fixture, ServerOptions, rp};

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(31) ^ (i >> 7)) as u8).collect()
}

fn kind_of(error: io::Error) -> (BackendErrorKind, Option<RemotePath>) {
    let error = BackendError::from_io(error, None);
    (error.kind, error.path)
}

/// Waits (up to `limit`) until the backend notices the connection is gone.
fn wait_disconnected(backend: &support::SftpBackend, limit: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if !backend.is_connected() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn a_connection_dropped_mid_write_never_leaves_the_final_name() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    fixture
        .server
        .faults
        .kill_after_written
        .store(200 * 1024, Ordering::SeqCst);
    let target = rp("/upload.bin")?;
    let mut session = fixture
        .backend
        .begin_write(&target, None, false)
        .map_err(io::Error::other)?;
    let data = pattern(2 * 1024 * 1024);
    let written = data
        .chunks(64 * 1024)
        .try_for_each(|chunk| session.write_all(chunk));
    let outcome = match written {
        Err(error) => Err(BackendError::from_io(error, None)),
        Ok(()) => session.finish(),
    };
    let error = outcome.err().ok_or_else(|| io::Error::other("the upload succeeded"))?;
    assert_eq!(error.kind, BackendErrorKind::Unavailable, "{error}");
    assert_eq!(error.path.as_ref(), Some(&target));
    assert!(!fixture.server.root().join("upload.bin").exists());
    // Whatever stayed behind is the hidden temporary, never the final name.
    for entry in fs::read_dir(fixture.server.root())?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(name.starts_with('.') && name.ends_with(".kara-part"), "{name}");
    }
    Ok(())
}

#[test]
fn abort_and_drop_remove_the_temporary_on_the_server() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    for (i, by_drop) in [false, true].into_iter().enumerate() {
        let target = rp(&format!("/f{i}"))?;
        let mut session = fixture
            .backend
            .begin_write(&target, None, false)
            .map_err(io::Error::other)?;
        session.write_all(&pattern(300 * 1024))?;
        assert_eq!(fixture.server.temporaries().len(), 1, "the temporary is where it writes");
        if by_drop {
            drop(session);
        } else {
            session.abort().map_err(io::Error::other)?;
        }
        assert!(fixture.server.temporaries().is_empty(), "left: {:?}", fixture.server.temporaries());
        assert!(!fixture.server.root().join(format!("f{i}")).exists());
    }
    Ok(())
}

#[test]
fn a_connection_dropped_mid_read_is_unavailable_naming_the_file() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    fs::write(fixture.server.root().join("big.bin"), pattern(3 * 1024 * 1024))?;
    fixture
        .server
        .faults
        .kill_after_read
        .store(256 * 1024, Ordering::SeqCst);
    let path = rp("/big.bin")?;
    let mut reader = fixture.backend.open_read(&path, 0).map_err(io::Error::other)?;
    let mut out = Vec::new();
    let error = reader
        .read_to_end(&mut out)
        .err()
        .ok_or_else(|| io::Error::other("the read succeeded"))?;
    assert_eq!(kind_of(error), (BackendErrorKind::Unavailable, Some(path)));
    assert!(out.len() < 3 * 1024 * 1024);
    Ok(())
}

#[test]
fn a_stalled_server_times_out_instead_of_hanging() -> io::Result<()> {
    let fixture = Fixture::new(ServerOptions::default(), &[("timeout_s", "1")])?;
    fixture.server.faults.stall.store(true, Ordering::SeqCst);
    let started = Instant::now();
    let path = rp("/anything")?;
    let error = fixture
        .backend
        .stat(&path)
        .err()
        .ok_or_else(|| io::Error::other("stat answered"))?;
    let took = started.elapsed();
    fixture.server.faults.stall.store(false, Ordering::SeqCst);
    assert_eq!(error.kind, BackendErrorKind::Unavailable);
    assert_eq!(error.path, Some(path));
    // The per-request timeout (1 s) answers, not the outer safety net (3 s).
    assert!(took < Duration::from_millis(2500), "took {took:?}");
    assert!(took >= Duration::from_millis(900), "answered before the timeout: {took:?}");
    Ok(())
}

#[test]
fn after_the_connection_is_gone_every_call_is_unavailable_at_once() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    fs::write(fixture.server.root().join("f"), b"x")?;
    fs::create_dir(fixture.server.root().join("d"))?;
    let caps = fixture.backend.capabilities();
    fixture.server.kill_connections();
    assert!(wait_disconnected(&fixture.backend, Duration::from_secs(5)));
    let b = fixture.backend.as_ref();
    let (f, d, n) = (rp("/f")?, rp("/d")?, rp("/n")?);
    let started = Instant::now();
    let results: Vec<(&str, Result<(), BackendError>, RemotePath)> = vec![
        ("list", b.list(&d, &Cancel::new()).map(|_| ()), d.clone()),
        ("stat", b.stat(&f).map(|_| ()), f.clone()),
        ("open_read", b.open_read(&f, 0).map(|_| ()), f.clone()),
        ("begin_write", b.begin_write(&n, None, false).map(|_| ()), n.clone()),
        ("create_dir", b.create_dir(&n), n.clone()),
        ("rename", b.rename(&f, &n), f.clone()),
        ("remove", b.remove(&f), f.clone()),
        ("remove_tree", b.remove_tree(&d, &Cancel::new()), d.clone()),
    ];
    assert!(started.elapsed() < Duration::from_secs(1), "took {:?}", started.elapsed());
    for (what, result, path) in results {
        let error = result.err().ok_or_else(|| io::Error::other(format!("{what} succeeded")))?;
        assert_eq!(error.kind, BackendErrorKind::Unavailable, "{what}: {error}");
        assert_eq!(error.path, Some(path), "{what}");
    }
    assert_eq!(b.capabilities(), caps, "capabilities never change after connect");
    Ok(())
}

#[test]
fn keepalive_notices_a_silent_server_without_any_call() -> io::Result<()> {
    let fixture = Fixture::new(ServerOptions::default(), &[("keepalive_s", "1"), ("timeout_s", "60")])?;
    fixture.server.freeze();
    // Three unanswered keepalives one second apart close the session.
    assert!(
        wait_disconnected(&fixture.backend, Duration::from_secs(10)),
        "the session never noticed the server went silent"
    );
    let started = Instant::now();
    let error = fixture
        .backend
        .stat(&rp("/")?)
        .err()
        .ok_or_else(|| io::Error::other("stat answered"))?;
    assert_eq!(error.kind, BackendErrorKind::Unavailable);
    assert!(started.elapsed() < Duration::from_secs(1));
    Ok(())
}

#[test]
fn a_full_disk_is_no_space_and_leaves_nothing_behind() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    fixture
        .server
        .faults
        .no_space_after
        .store(64 * 1024, Ordering::SeqCst);
    let target = rp("/full.bin")?;
    // 100 KiB: the third chunk fails, after write() already returned.
    let mut session = fixture
        .backend
        .begin_write(&target, None, false)
        .map_err(io::Error::other)?;
    session.write_all(&pattern(100 * 1024))?;
    let error = session.finish().err().ok_or_else(|| io::Error::other("finish succeeded"))?;
    assert_eq!(error.kind, BackendErrorKind::NoSpace, "{error}");
    assert_eq!(error.path, Some(target));
    assert!(!fixture.server.root().join("full.bin").exists());
    assert!(fixture.server.temporaries().is_empty());
    assert!(fixture.backend.is_connected(), "a full disk is not a lost connection");
    Ok(())
}

#[test]
fn a_denied_path_is_permission_denied_naming_the_callers_path() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    fs::create_dir_all(fixture.server.root().join("home/locked"))?;
    let fixture = Fixture {
        backend: fixture.reconnect(&[("root", "/home")])?,
        ..fixture
    };
    fs::write(fixture.server.root().join("home/locked/f"), b"x")?;
    fixture.server.deny("/home/locked");
    let b = fixture.backend.as_ref();
    let (dir, file, new) = (rp("/locked")?, rp("/locked/f")?, rp("/locked/new")?);
    for (what, result, path) in [
        ("stat", b.stat(&file).map(|_| ()), &file),
        ("list", b.list(&dir, &Cancel::new()).map(|_| ()), &dir),
        ("create_dir", b.create_dir(&new), &new),
        ("begin_write", b.begin_write(&new, None, false).map(|_| ()), &new),
        ("open_read", b.open_read(&file, 0).map(|_| ()), &file),
    ] {
        let error = result.err().ok_or_else(|| io::Error::other(format!("{what} succeeded")))?;
        assert_eq!(error.kind, BackendErrorKind::PermissionDenied, "{what}");
        assert_eq!(error.path.as_ref(), Some(path), "{what}");
        let text = format!("{error} {error:?}");
        assert!(!text.contains("/home"), "{what} leaks the server path: {text}");
    }
    Ok(())
}

#[test]
fn errors_never_name_the_temporary_or_the_server_path() -> io::Result<()> {
    let server = support::TestServer::start(ServerOptions::default())?;
    fs::create_dir_all(server.root().join("home/kara"))?;
    let client = tempfile::tempdir()?;
    let known = support::trusting_known_hosts(&server, client.path())?;
    let backend = support::connect(&server, &known, &[("root", "/home/kara")])?;
    server.faults.no_space_after.store(10, Ordering::SeqCst);
    let target = rp("/doc.txt")?;
    let mut session = backend
        .begin_write(&target, None, false)
        .map_err(io::Error::other)?;
    session.write_all(b"more than ten bytes")?;
    let error = session.finish().err().ok_or_else(|| io::Error::other("finish succeeded"))?;
    assert_eq!(error.path, Some(target));
    let text = format!("{error} {error:?}");
    assert!(!text.contains(".kara-part"), "{text}");
    assert!(!text.contains("home/kara"), "{text}");
    Ok(())
}

#[test]
fn a_read_only_file_is_not_replaced() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    let local = fixture.server.root().join("ro.txt");
    fs::write(&local, b"keep")?;
    fs::set_permissions(&local, fs::Permissions::from_mode(0o444))?;
    let error = fixture
        .backend
        .begin_write(&rp("/ro.txt")?, None, true)
        .err()
        .ok_or_else(|| io::Error::other("begin_write succeeded"))?;
    assert_eq!(error.kind, BackendErrorKind::PermissionDenied);
    assert_eq!(fs::read(&local)?, b"keep");
    Ok(())
}

#[test]
fn offset_reads_start_exactly_there() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    let content = pattern(300 * 1024 + 7);
    fs::write(fixture.server.root().join("o.bin"), &content)?;
    let path = rp("/o.bin")?;
    for from in [0usize, 1, 32 * 1024 - 1, 32 * 1024, 100_000, content.len() - 1, content.len()] {
        let mut reader = fixture
            .backend
            .open_read(&path, from as u64)
            .map_err(io::Error::other)?;
        let mut got = Vec::new();
        reader.read_to_end(&mut got)?;
        assert!(got == content[from..], "from {from}: {} bytes", got.len());
    }
    let error = fixture
        .backend
        .open_read(&path, content.len() as u64 + 1)
        .err()
        .ok_or_else(|| io::Error::other("opened past the end"))?;
    assert_eq!(error.kind, BackendErrorKind::Other);
    Ok(())
}

#[test]
fn a_short_read_from_the_server_does_not_lose_or_repeat_bytes() -> io::Result<()> {
    // Every answer is shorter than the 32 KiB asked: the requests already in
    // flight after a short one start at the wrong offset and must be redone.
    let fixture = Fixture::standard()?;
    let content = pattern(1024 * 1024 + 12_345);
    fs::write(fixture.server.root().join("s.bin"), &content)?;
    fixture.server.faults.read_cap.store(10_000, Ordering::SeqCst);
    let mut reader = fixture
        .backend
        .open_read(&rp("/s.bin")?, 0)
        .map_err(io::Error::other)?;
    let mut got = Vec::new();
    let mut buf = vec![0u8; 7_777];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    assert!(got == content, "{} bytes, expected {}", got.len(), content.len());
    Ok(())
}

#[test]
fn remove_tree_removes_links_and_never_what_they_point_to() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    let root = fixture.server.root();
    fs::create_dir_all(root.join("outside/deep"))?;
    fs::write(root.join("outside/keep.txt"), b"keep")?;
    fs::write(root.join("outside/deep/keep2.txt"), b"keep")?;
    fs::create_dir_all(root.join("t/sub"))?;
    fs::write(root.join("t/sub/f"), b"f")?;
    std::os::unix::fs::symlink(root.join("outside"), root.join("t/to-dir"))?;
    std::os::unix::fs::symlink("../outside", root.join("t/sub/relative"))?;
    std::os::unix::fs::symlink(root.join("outside/keep.txt"), root.join("t/to-file"))?;
    std::os::unix::fs::symlink("nowhere", root.join("t/broken"))?;

    fixture
        .backend
        .remove_tree(&rp("/t")?, &Cancel::new())
        .map_err(io::Error::other)?;
    assert!(!root.join("t").exists());
    assert_eq!(fs::read(root.join("outside/keep.txt"))?, b"keep");
    assert_eq!(fs::read(root.join("outside/deep/keep2.txt"))?, b"keep");

    // A link given directly is removed as a link.
    std::os::unix::fs::symlink(root.join("outside"), root.join("direct"))?;
    fixture
        .backend
        .remove_tree(&rp("/direct")?, &Cancel::new())
        .map_err(io::Error::other)?;
    assert!(fs::symlink_metadata(root.join("direct")).is_err());
    assert_eq!(fs::read(root.join("outside/keep.txt"))?, b"keep");
    Ok(())
}

#[test]
fn cancel_stops_a_slow_listing_promptly() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    let dir = fixture.server.root().join("many");
    fs::create_dir(&dir)?;
    for i in 0..3000 {
        fs::write(dir.join(format!("f{i:05}")), b"")?;
    }
    // 30 pages of 100 at 100 ms each: three seconds uncancelled.
    fixture.server.faults.delay_ms.store(100, Ordering::SeqCst);
    let cancel = Cancel::new();
    let trigger = cancel.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(400));
        trigger.cancel();
        Instant::now()
    });
    let path = rp("/many")?;
    let outcome = fixture.backend.list(&path, &cancel);
    let returned = Instant::now();
    let cancelled_at = canceller.join().map_err(|_| io::Error::other("join"))?;
    let error = outcome.err().ok_or_else(|| io::Error::other("the listing completed"))?;
    assert_eq!(error.kind, BackendErrorKind::Cancelled);
    assert_eq!(error.path, Some(path));
    let late = returned.saturating_duration_since(cancelled_at);
    assert!(late < Duration::from_millis(300), "returned {late:?} after the cancel");
    Ok(())
}

#[test]
fn cancel_stops_remove_tree_midway() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    let dir = fixture.server.root().join("tree");
    fs::create_dir(&dir)?;
    for i in 0..400 {
        fs::write(dir.join(format!("f{i:04}")), b"x")?;
    }
    fixture.server.faults.delay_ms.store(10, Ordering::SeqCst);
    let cancel = Cancel::new();
    let trigger = cancel.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        trigger.cancel();
    });
    let path = rp("/tree")?;
    let error = fixture
        .backend
        .remove_tree(&path, &cancel)
        .err()
        .ok_or_else(|| io::Error::other("remove_tree finished"))?;
    let _ = canceller.join();
    assert_eq!(error.kind, BackendErrorKind::Cancelled);
    assert_eq!(error.path, Some(path));
    let left = fs::read_dir(&dir)?.count();
    assert!(left > 200, "only {left} files left: the cancel came late");
    assert!(left < 400, "nothing was removed before the cancel");
    Ok(())
}

#[test]
fn a_listing_reports_links_like_the_local_backend() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    let root = fixture.server.root();
    fs::create_dir(root.join("d"))?;
    fs::write(root.join("d/file"), b"12345")?;
    fs::create_dir(root.join("d/sub"))?;
    std::os::unix::fs::symlink("file", root.join("d/to-file"))?;
    std::os::unix::fs::symlink("sub", root.join("d/to-sub"))?;
    std::os::unix::fs::symlink("missing", root.join("d/broken"))?;
    fs::write(root.join("d/.hidden"), b"")?;
    let local = kara_fs::LocalBackend::with_root(root).map_err(io::Error::other)?;
    let d = rp("/d")?;
    let summary = |b: &dyn Backend| -> io::Result<Vec<String>> {
        let listing = b.list(&d, &Cancel::new()).map_err(io::Error::other)?;
        assert!(listing.errors.is_empty());
        let mut seen: Vec<String> = listing
            .entries
            .iter()
            .map(|e| {
                format!(
                    "{} {:?} link={} broken={} hidden={} size={:?}",
                    e.display, e.kind, e.is_symlink, e.symlink_broken, e.is_hidden, e.size
                )
            })
            .collect();
        seen.sort();
        Ok(seen)
    };
    assert_eq!(summary(fixture.backend.as_ref())?, summary(&local)?);
    // And stat agrees with the listing, entry by entry.
    for name in ["file", "sub", "to-file", "to-sub", "broken", ".hidden"] {
        let path = d.join(name).map_err(io::Error::other)?;
        let remote = fixture.backend.stat(&path).map_err(io::Error::other)?;
        let here = local.stat(&path).map_err(io::Error::other)?;
        assert_eq!(
            (remote.kind, remote.is_symlink, remote.symlink_broken, remote.size, remote.modified),
            (here.kind, here.is_symlink, here.symlink_broken, here.size, here.modified.map(|t| {
                // SFTP v3 carries whole seconds.
                let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                std::time::UNIX_EPOCH + Duration::from_secs(secs)
            })),
            "{name}"
        );
    }
    Ok(())
}
