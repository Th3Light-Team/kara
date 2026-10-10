//! `ObjectStoreBackend` when the service misbehaves: failed and stalled
//! parts, failed completions, aborts, lost connections, 5xx, full storage,
//! denied requests, cancel mid-list / mid-delete, a rename that fails
//! half-way, the no-overwrite rules under races, and the placeholder.

mod objstore_support;

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use kara_remote::objstore::{ObjectStoreOptions, PLACEHOLDER};
use kara_vfs::{Backend, BackendError, BackendErrorKind, Cancel, RemotePath};
use objstore_support::{Effect, Fault, Faulty, Op, backend_over, backend_with_parts, faulty_backend, rp};

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(131) >> 3) as u8).collect()
}

fn put(backend: &dyn Backend, path: &RemotePath, data: &[u8]) -> io::Result<()> {
    let mut session = backend.begin_write(path, None, false).map_err(io::Error::other)?;
    session.write_all(data)?;
    session.finish().map_err(io::Error::other)
}

fn get(backend: &dyn Backend, path: &RemotePath) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    backend
        .open_read(path, 0)
        .map_err(io::Error::other)?
        .read_to_end(&mut out)?;
    Ok(out)
}

fn kind_and_path(result: Result<(), BackendError>) -> Option<(BackendErrorKind, Option<RemotePath>)> {
    result.err().map(|e| (e.kind, e.path))
}

/// Writes `data` and finishes; the first error, from a write or from finish.
fn upload(backend: &dyn Backend, path: &RemotePath, data: &[u8], replace: bool) -> Result<(), BackendError> {
    let mut session = backend.begin_write(path, None, replace)?;
    if let Err(error) = session.write_all(data) {
        return Err(BackendError::from_io(error, Some(path)));
    }
    session.finish()
}

// ---------------------------------------------------------------------------
// Uploads.

#[test]
fn a_failed_part_leaves_no_object_and_aborts_the_upload() -> io::Result<()> {
    let store = Faulty::new();
    let backend = backend_with_parts(&store, 1024, 2)?;
    store.inject(Fault::on(Op::Part, Effect::Refused).after(3));
    let path = rp("/up/video.mkv")?;
    let result = upload(&backend, &path, &pattern(20_000), false);
    assert_eq!(kind_and_path(result), Some((BackendErrorKind::Unavailable, Some(path.clone()))));
    assert!(store.keys().is_empty(), "{:?}", store.keys());
    assert_eq!(store.open_uploads(), 0, "the multipart upload was aborted");
    assert!(store.counts.get(Op::Abort) >= 1);
    assert_eq!(store.counts.get(Op::Complete), 0);
    Ok(())
}

#[test]
fn a_failed_completion_leaves_no_object_and_aborts_the_upload() -> io::Result<()> {
    let store = Faulty::new();
    let backend = backend_with_parts(&store, 1024, 2)?;
    store.inject(Fault::on(Op::Complete, Effect::ServerError));
    let path = rp("/big.bin")?;
    let result = upload(&backend, &path, &pattern(10_000), true);
    assert_eq!(kind_and_path(result), Some((BackendErrorKind::Unavailable, Some(path))));
    assert!(store.keys().is_empty());
    assert_eq!(store.open_uploads(), 0);
    Ok(())
}

#[test]
fn a_failed_single_put_leaves_no_object() -> io::Result<()> {
    let store = Faulty::new();
    let backend = faulty_backend(&store)?;
    store.inject(Fault::on(Op::Put, Effect::Full));
    let path = rp("/small.txt")?;
    let result = upload(&backend, &path, b"hello", false);
    assert_eq!(kind_and_path(result), Some((BackendErrorKind::NoSpace, Some(path))));
    assert!(store.keys().is_empty());
    Ok(())
}

#[test]
fn abort_and_drop_abort_the_multipart_upload() -> io::Result<()> {
    let store = Faulty::new();
    let backend = backend_with_parts(&store, 1024, 2)?;
    let path = rp("/x.bin")?;
    let mut session = backend.begin_write(&path, None, false).map_err(io::Error::other)?;
    session.write_all(&pattern(5000))?;
    assert_eq!(store.open_uploads(), 1, "the upload started with the first full part");
    session.abort().map_err(io::Error::other)?;
    assert_eq!(store.open_uploads(), 0);

    let mut session = backend.begin_write(&path, None, false).map_err(io::Error::other)?;
    session.write_all(&pattern(5000))?;
    assert_eq!(store.open_uploads(), 1);
    drop(session);
    assert_eq!(store.open_uploads(), 0, "dropping aborts too");
    assert_eq!(store.counts.get(Op::Abort), 2);
    assert!(store.keys().is_empty());
    Ok(())
}

#[test]
fn finish_completes_only_after_every_part_arrived() -> io::Result<()> {
    let store = Faulty::new();
    let backend = backend_with_parts(&store, 1024, 4)?;
    // Every part takes a while: finish must still wait for all of them.
    store.inject(Fault::on(Op::Part, Effect::Stall(Duration::from_millis(150))).times(12));
    let path = rp("/slow.bin")?;
    let data = pattern(12 * 1024 + 100);
    upload(&backend, &path, &data, false).map_err(io::Error::other)?;
    assert_eq!(store.bytes("slow.bin"), Some(data));
    assert_eq!(store.counts.get(Op::Part), 13);
    Ok(())
}

#[test]
fn the_loser_of_a_multipart_race_gets_already_exists() -> io::Result<()> {
    for conditional in [true, false] {
        let store = Faulty::new();
        store.conditional_put.store(conditional, Ordering::SeqCst);
        let backend = backend_with_parts(&store, 1024, 2)?;
        let path = rp("/race.bin")?;
        let mut first = backend.begin_write(&path, None, false).map_err(io::Error::other)?;
        let mut second = backend.begin_write(&path, None, false).map_err(io::Error::other)?;
        first.write_all(&pattern(4000))?;
        second.write_all(&[7u8; 3000])?;
        second.finish().map_err(io::Error::other)?;
        let lost = first.finish();
        assert_eq!(
            kind_and_path(lost),
            Some((BackendErrorKind::AlreadyExists, Some(path.clone()))),
            "conditional put {conditional}"
        );
        assert_eq!(store.bytes("race.bin"), Some(vec![7u8; 3000]));
        assert_eq!(store.open_uploads(), 0, "the loser's upload was aborted");
    }
    Ok(())
}

#[test]
fn a_name_taken_between_the_check_and_the_put_is_not_overwritten() -> io::Result<()> {
    // The check before the PUT passes; another client writes the name while
    // the PUT is on its way: only the conditional put can still refuse.
    let store = Faulty::new();
    let backend = Arc::new(faulty_backend(&store)?);
    let path = rp("/contract.pdf")?;
    let mut session = backend.begin_write(&path, None, false).map_err(io::Error::other)?;
    session.write_all(b"mine")?;
    store.inject(Fault::on(Op::Put, Effect::Stall(Duration::from_millis(400))));
    let other = {
        let store = Arc::clone(&store);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            store.put_raw("contract.pdf", b"theirs")
        })
    };
    let result = session.finish();
    other.join().map_err(|_| io::Error::other("join"))??;
    assert_eq!(kind_and_path(result), Some((BackendErrorKind::AlreadyExists, Some(path))));
    assert_eq!(store.bytes("contract.pdf"), Some(b"theirs".to_vec()));
    Ok(())
}

#[test]
fn replace_true_overwrites_and_replace_false_never_does() -> io::Result<()> {
    let store = Faulty::new();
    let backend = backend_with_parts(&store, 1024, 2)?;
    let path = rp("/doc")?;
    put(&backend, &path, b"old")?;
    assert_eq!(
        backend.begin_write(&path, None, false).err().map(|e| (e.kind, e.path)),
        Some((BackendErrorKind::AlreadyExists, Some(path.clone())))
    );
    for size in [10usize, 5000] {
        upload(&backend, &path, &pattern(size), true).map_err(io::Error::other)?;
        assert_eq!(store.bytes("doc"), Some(pattern(size)));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reads.

#[test]
fn reads_start_at_the_offset_and_a_lost_body_is_unavailable() -> io::Result<()> {
    let store = Faulty::new();
    let backend = faulty_backend(&store)?;
    let path = rp("/data.bin")?;
    let data = pattern(300_000);
    put(&backend, &path, &data)?;
    for from in [0usize, 1, 65_536, 200_001, 299_999, 300_000] {
        let mut out = Vec::new();
        backend
            .open_read(&path, from as u64)
            .map_err(io::Error::other)?
            .read_to_end(&mut out)?;
        assert_eq!(out, data[from..], "from {from}");
    }
    let past = backend.open_read(&path, 300_001).err().map(|e| (e.kind, e.path));
    assert_eq!(past, Some((BackendErrorKind::Other, Some(path.clone()))));

    // The connection drops after two pieces of the body.
    store.inject(Fault::on(Op::Body, Effect::Refused).after(1));
    let mut reader = backend.open_read(&path, 0).map_err(io::Error::other)?;
    let mut out = Vec::new();
    let error = reader.read_to_end(&mut out).err().ok_or_else(|| io::Error::other("read it all"))?;
    let error = BackendError::from_io(error, None);
    assert_eq!((error.kind, error.path), (BackendErrorKind::Unavailable, Some(path.clone())));

    // The server closes the body early without an error: never a short file.
    store.truncate_body_after.store(2, Ordering::SeqCst);
    let mut reader = backend.open_read(&path, 0).map_err(io::Error::other)?;
    let mut out = Vec::new();
    let error = reader.read_to_end(&mut out).err().ok_or_else(|| io::Error::other("short read accepted"))?;
    let error = BackendError::from_io(error, None);
    assert_eq!((error.kind, error.path), (BackendErrorKind::Unavailable, Some(path)));
    Ok(())
}

// ---------------------------------------------------------------------------
// Error kinds and paths.

#[test]
fn errors_map_by_kind_and_name_the_callers_path_not_the_key() -> io::Result<()> {
    let store = Faulty::new();
    let backend = backend_over(
        &store,
        ObjectStoreOptions {
            prefix: String::from("tenant-7/home"),
            ..ObjectStoreOptions::default()
        },
    )?;
    let path = rp("/docs/report.pdf")?;
    put(&backend, &path, b"x")?;
    for (effect, kind) in [
        (Effect::Refused, BackendErrorKind::Unavailable),
        (Effect::ServerError, BackendErrorKind::Unavailable),
        (Effect::Denied, BackendErrorKind::PermissionDenied),
        (Effect::Unauthenticated, BackendErrorKind::PermissionDenied),
        (Effect::Full, BackendErrorKind::NoSpace),
    ] {
        store.inject(Fault::on(Op::Head, effect.clone()));
        let error = backend.stat(&path).err().ok_or_else(|| io::Error::other("stat worked"))?;
        assert_eq!(error.kind, kind, "{effect:?}");
        assert_eq!(error.path.as_ref(), Some(&path));
        let text = format!("{error} {:?}", error.source);
        assert!(!text.contains("tenant-7"), "the key leaked: {text}");
    }
    Ok(())
}

#[test]
fn an_unplugged_service_answers_unavailable_at_once_for_every_call() -> io::Result<()> {
    let store = Faulty::new();
    let backend = faulty_backend(&store)?;
    let file = rp("/a/f")?;
    put(&backend, &file, b"x")?;
    store.unplugged.store(true, Ordering::SeqCst);
    let started = Instant::now();
    let dir = rp("/a")?;
    let other = rp("/b")?;
    let results = [
        backend.stat(&file).map(|_| ()),
        backend.list(&dir, &Cancel::new()).map(|_| ()),
        backend.open_read(&file, 0).map(|_| ()),
        backend.begin_write(&other, None, false).map(|_| ()),
        backend.create_dir(&other),
        backend.rename(&file, &other),
        backend.remove(&file),
        backend.remove_tree(&dir, &Cancel::new()),
        backend.copy_within(&file, &other),
    ];
    for result in results {
        let error = result.err().ok_or_else(|| io::Error::other("a call worked"))?;
        assert_eq!(error.kind, BackendErrorKind::Unavailable, "{error}");
        assert!(error.path.is_some());
    }
    assert!(started.elapsed() < Duration::from_secs(1));
    Ok(())
}

#[test]
fn a_hung_request_is_cut_by_the_drive_timeout() -> io::Result<()> {
    let store = Faulty::new();
    let backend = backend_over(
        &store,
        ObjectStoreOptions {
            timeout: Duration::from_millis(500),
            ..ObjectStoreOptions::default()
        },
    )?;
    store.inject(Fault::on(Op::Head, Effect::Stall(Duration::from_secs(60))));
    let started = Instant::now();
    let error = backend.stat(&rp("/x")?).err().ok_or_else(|| io::Error::other("answered"))?;
    assert_eq!(error.kind, BackendErrorKind::Unavailable);
    assert!(started.elapsed() < Duration::from_secs(8), "{:?}", started.elapsed());
    Ok(())
}

// ---------------------------------------------------------------------------
// Cancel.

fn many_files(store: &Arc<Faulty>, dir: &str, count: usize) -> io::Result<()> {
    for i in 0..count {
        store.put_raw(&format!("{dir}/f{i:05}"), b"x")?;
    }
    Ok(())
}

#[test]
fn cancel_stops_a_listing_between_pages() -> io::Result<()> {
    let store = Faulty::new();
    many_files(&store, "big", 400)?;
    store.page_size.store(10, Ordering::SeqCst);
    store.list_delay_ms.store(20, Ordering::SeqCst);
    let backend = faulty_backend(&store)?;
    let cancel = Cancel::new();
    let canceller = {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            cancel.cancel();
            Instant::now()
        })
    };
    let dir = rp("/big")?;
    let result = backend.list(&dir, &cancel);
    let ended = Instant::now();
    let cancelled_at = canceller.join().map_err(|_| io::Error::other("join"))?;
    let error = result.err().ok_or_else(|| io::Error::other("listed everything"))?;
    assert_eq!((error.kind, error.path), (BackendErrorKind::Cancelled, Some(dir)));
    assert!(ended.saturating_duration_since(cancelled_at) < Duration::from_millis(300));
    assert!(store.counts.get(Op::List) < 40, "it went on listing");
    Ok(())
}

#[test]
fn cancel_stops_a_listing_while_a_page_is_on_its_way() -> io::Result<()> {
    let store = Faulty::new();
    many_files(&store, "d", 3)?;
    let backend = faulty_backend(&store)?;
    store.inject(Fault::on(Op::List, Effect::Stall(Duration::from_secs(20))));
    let cancel = Cancel::new();
    let canceller = {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            cancel.cancel();
        })
    };
    let started = Instant::now();
    let result = backend.list(&rp("/d")?, &cancel);
    let _ = canceller.join();
    assert_eq!(result.err().map(|e| e.kind), Some(BackendErrorKind::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(2));
    Ok(())
}

#[test]
fn cancel_stops_remove_tree_midway_and_no_child_outlives_its_placeholder() -> io::Result<()> {
    let store = Faulty::new();
    let backend = faulty_backend(&store)?;
    backend.create_dir(&rp("/t")?).map_err(io::Error::other)?;
    for sub in ["a", "b", "c"] {
        backend.create_dir(&rp(&format!("/t/{sub}"))?).map_err(io::Error::other)?;
        many_files(&store, &format!("t/{sub}"), 100)?;
    }
    store.page_size.store(20, Ordering::SeqCst);
    store.delete_delay_ms.store(3, Ordering::SeqCst);
    let cancel = Cancel::new();
    let canceller = {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(120));
            cancel.cancel();
        })
    };
    let root = rp("/t")?;
    let result = backend.remove_tree(&root, &cancel);
    let _ = canceller.join();
    let error = result.err().ok_or_else(|| io::Error::other("removed everything"))?;
    assert_eq!((error.kind, error.path), (BackendErrorKind::Cancelled, Some(root.clone())));
    let keys = store.keys();
    assert!(keys.iter().any(|k| k.contains("/f")), "something must remain: {keys:?}");
    // Every folder that still has a child still has its placeholder.
    for sub in ["t", "t/a", "t/b", "t/c"] {
        let has_child = keys
            .iter()
            .any(|k| k.starts_with(&format!("{sub}/")) && !k.ends_with(PLACEHOLDER));
        if has_child {
            assert!(keys.contains(&format!("{sub}/{PLACEHOLDER}")), "{sub} lost its placeholder: {keys:?}");
        }
    }
    // Finishing the job leaves nothing, placeholders included.
    backend.remove_tree(&root, &Cancel::new()).map_err(io::Error::other)?;
    assert!(store.keys().is_empty(), "{:?}", store.keys());
    Ok(())
}

#[test]
fn a_failed_delete_in_remove_tree_keeps_the_placeholders_and_names_the_object() -> io::Result<()> {
    let store = Faulty::new();
    let backend = faulty_backend(&store)?;
    backend.create_dir(&rp("/p")?).map_err(io::Error::other)?;
    backend.create_dir(&rp("/p/q")?).map_err(io::Error::other)?;
    for name in ["/p/one", "/p/q/two", "/p/q/three"] {
        put(&backend, &rp(name)?, b"x")?;
    }
    store.inject(Fault::on(Op::Delete, Effect::Denied).after(1));
    let error = backend.remove_tree(&rp("/p")?, &Cancel::new()).err().ok_or_else(|| io::Error::other("removed"))?;
    assert_eq!(error.kind, BackendErrorKind::PermissionDenied);
    let named = error.path.ok_or_else(|| io::Error::other("no path"))?;
    assert!(named.starts_with(&rp("/p")?), "{named}");
    let keys = store.keys();
    assert!(keys.contains(&format!("p/{PLACEHOLDER}")), "{keys:?}");
    assert!(keys.contains(&format!("p/q/{PLACEHOLDER}")), "{keys:?}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Rename.

fn tree(backend: &dyn Backend, store: &Arc<Faulty>, files: usize) -> io::Result<Vec<String>> {
    backend.create_dir(&rp("/src")?).map_err(io::Error::other)?;
    backend.create_dir(&rp("/src/empty")?).map_err(io::Error::other)?;
    for i in 0..files {
        put(backend, &rp(&format!("/src/d{}/f{i}", i % 3))?, &pattern(100 + i))?;
    }
    Ok(store.keys())
}

/// Every object of `before` (under `src/`) is now under exactly one of the two
/// names, or under `dst/` only when `moved`.
fn each_under_one_name(store: &Faulty, before: &[String]) -> (usize, usize, usize) {
    let after = store.keys();
    let (mut old, mut new, mut both) = (0, 0, 0);
    for key in before {
        let moved = key.replacen("src/", "dst/", 1);
        match (after.contains(key), after.contains(&moved)) {
            (true, true) => both += 1,
            (true, false) => old += 1,
            (false, true) => new += 1,
            (false, false) => panic!("{key} was lost: {after:?}"),
        }
    }
    (old, new, both)
}

#[test]
fn a_rename_that_fails_while_copying_leaves_everything_under_the_old_name() -> io::Result<()> {
    for at in [0usize, 7, 19] {
        let store = Faulty::new();
        let backend = faulty_backend(&store)?;
        let before = tree(&backend, &store, 20)?;
        store.inject(Fault::on(Op::Copy, Effect::Refused).after(at));
        let error = backend.rename(&rp("/src")?, &rp("/dst")?).err().ok_or_else(|| io::Error::other("renamed"))?;
        assert_eq!(error.kind, BackendErrorKind::Unavailable);
        assert!(error.path.as_ref().is_some_and(|p| p.starts_with(&rp("/src").unwrap_or_else(|_| RemotePath::root()))));
        let (old, new, both) = each_under_one_name(&store, &before);
        assert_eq!((old, new, both), (before.len(), 0, 0), "failure at copy {at}");
        // Copies run in batches: the others of the failing batch did arrive,
        // and exactly those are taken back.
        assert_eq!(store.counts.get(Op::Delete), store.counts.get(Op::Copy) - 1, "only the copies were deleted");
    }
    Ok(())
}

#[test]
fn a_rename_whose_rollback_fails_keeps_every_object_somewhere() -> io::Result<()> {
    let store = Faulty::new();
    let backend = faulty_backend(&store)?;
    let before = tree(&backend, &store, 12)?;
    store.inject(Fault::on(Op::Copy, Effect::Refused).after(8));
    store.inject(Fault::on(Op::Delete, Effect::Refused).always());
    assert!(backend.rename(&rp("/src")?, &rp("/dst")?).is_err());
    let (old, new, _both) = each_under_one_name(&store, &before);
    assert_eq!(old + new + _both, before.len());
    assert_eq!(new, 0, "no source is deleted while copying");
    Ok(())
}

#[test]
fn sources_are_deleted_only_after_every_copy_and_a_late_failure_loses_nothing() -> io::Result<()> {
    let store = Faulty::new();
    let backend = faulty_backend(&store)?;
    let before = tree(&backend, &store, 15)?;
    store.inject(Fault::on(Op::Delete, Effect::ServerError).after(4));
    let error = backend.rename(&rp("/src")?, &rp("/dst")?).err().ok_or_else(|| io::Error::other("renamed"))?;
    assert_eq!(error.kind, BackendErrorKind::Unavailable);
    assert_eq!(store.counts.get(Op::Copy), before.len(), "every copy came first");
    let (old, new, both) = each_under_one_name(&store, &before);
    assert_eq!(old, 0, "every object reached the new name");
    assert_eq!(new + both, before.len());
    assert!(both > 0);
    // The placeholders were the last to go: the old folders still show.
    let after = store.keys();
    assert!(after.contains(&format!("src/{PLACEHOLDER}")), "{after:?}");
    Ok(())
}

#[test]
fn rename_never_overwrites_and_moves_the_whole_prefix() -> io::Result<()> {
    for copy_create in [true, false] {
        let store = Faulty::new();
        store.copy_create.store(copy_create, Ordering::SeqCst);
        let backend = faulty_backend(&store)?;
        let before = tree(&backend, &store, 6)?;
        put(&backend, &rp("/taken")?, b"t")?;
        let refused = backend.rename(&rp("/src")?, &rp("/taken")?).err().map(|e| (e.kind, e.path));
        assert_eq!(refused, Some((BackendErrorKind::AlreadyExists, Some(rp("/taken")?))));
        backend.rename(&rp("/src")?, &rp("/dst")?).map_err(io::Error::other)?;
        let (old, new, both) = each_under_one_name(&store, &before);
        assert_eq!((old, new, both), (0, before.len(), 0));
        assert!(store.keys().contains(&format!("dst/empty/{PLACEHOLDER}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// copy_within.

#[test]
fn copy_within_is_server_side_and_never_overwrites() -> io::Result<()> {
    for copy_create in [true, false] {
        let store = Faulty::new();
        store.copy_create.store(copy_create, Ordering::SeqCst);
        let backend = faulty_backend(&store)?;
        put(&backend, &rp("/a")?, &pattern(5000))?;
        let gets = store.counts.get(Op::Get);
        backend.copy_within(&rp("/a")?, &rp("/b")?).map_err(io::Error::other)?;
        assert_eq!(store.counts.get(Op::Get), gets, "no byte went through the client");
        assert_eq!(store.bytes("b"), Some(pattern(5000)));
        put(&backend, &rp("/c")?, b"keep")?;
        let refused = backend.copy_within(&rp("/a")?, &rp("/c")?).err().map(|e| (e.kind, e.path));
        assert_eq!(refused, Some((BackendErrorKind::AlreadyExists, Some(rp("/c")?))));
        assert_eq!(store.bytes("c"), Some(b"keep".to_vec()));
    }
    Ok(())
}

#[test]
fn a_copy_target_taken_during_the_copy_is_not_overwritten() -> io::Result<()> {
    let store = Faulty::new();
    let backend = faulty_backend(&store)?;
    put(&backend, &rp("/a")?, b"source")?;
    store.inject(Fault::on(Op::Copy, Effect::Stall(Duration::from_millis(400))));
    let other = {
        let store = Arc::clone(&store);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            store.put_raw("b", b"theirs")
        })
    };
    let result = backend.copy_within(&rp("/a")?, &rp("/b")?);
    other.join().map_err(|_| io::Error::other("join"))??;
    assert_eq!(kind_and_path(result), Some((BackendErrorKind::AlreadyExists, Some(rp("/b")?))));
    assert_eq!(store.bytes("b"), Some(b"theirs".to_vec()));
    Ok(())
}

#[test]
fn objects_above_the_server_copy_limit_are_streamed() -> io::Result<()> {
    let store = Faulty::new();
    let backend = backend_over(
        &store,
        ObjectStoreOptions {
            max_server_copy: Some(1000),
            ..ObjectStoreOptions::default()
        },
    )?;
    put(&backend, &rp("/big")?, &pattern(5000))?;
    put(&backend, &rp("/small")?, &pattern(500))?;
    backend.copy_within(&rp("/big")?, &rp("/big2")?).map_err(io::Error::other)?;
    backend.copy_within(&rp("/small")?, &rp("/small2")?).map_err(io::Error::other)?;
    assert_eq!(store.counts.get(Op::Copy), 1, "only the small one was copied by the service");
    assert_eq!(store.bytes("big2"), Some(pattern(5000)));
    backend.rename(&rp("/big")?, &rp("/big3")?).map_err(io::Error::other)?;
    assert_eq!(store.bytes("big3"), Some(pattern(5000)));
    assert!(store.bytes("big").is_none());
    Ok(())
}

// ---------------------------------------------------------------------------
// The placeholder and foreign layouts.

#[test]
fn the_placeholder_is_never_listed_never_reachable_and_never_survives() -> io::Result<()> {
    let store = Faulty::new();
    let backend = faulty_backend(&store)?;
    backend.create_dir(&rp("/d")?).map_err(io::Error::other)?;
    backend.create_dir(&rp("/d/e")?).map_err(io::Error::other)?;
    put(&backend, &rp("/d/e/f")?, b"f")?;
    assert!(store.keys().contains(&format!("d/{PLACEHOLDER}")));
    let names = |dir: &str| -> io::Result<Vec<String>> {
        let listing = backend.list(&rp(dir)?, &Cancel::new()).map_err(io::Error::other)?;
        assert!(listing.errors.is_empty(), "{:?}", listing.errors);
        Ok(listing.entries.into_iter().map(|e| e.display).collect())
    };
    assert_eq!(names("/")?, vec![String::from("d")]);
    assert_eq!(names("/d")?, vec![String::from("e")]);
    assert_eq!(names("/d/e")?, vec![String::from("f")]);

    let marker = rp(&format!("/d/{PLACEHOLDER}"))?;
    assert_eq!(backend.stat(&marker).err().map(|e| e.kind), Some(BackendErrorKind::NotFound));
    assert_eq!(backend.open_read(&marker, 0).err().map(|e| e.kind), Some(BackendErrorKind::NotFound));
    assert_eq!(backend.remove(&marker).err().map(|e| e.kind), Some(BackendErrorKind::NotFound));
    for refused in [
        backend.begin_write(&marker, None, true).err(),
        backend.create_dir(&marker).err(),
        backend.begin_write(&rp(&format!("/d/{PLACEHOLDER}/x"))?, None, false).err(),
        backend.rename(&rp("/d/e/f")?, &marker).err(),
        backend.copy_within(&rp("/d/e/f")?, &marker).err(),
    ] {
        let error = refused.ok_or_else(|| io::Error::other("the reserved name was accepted"))?;
        assert_eq!(error.kind, BackendErrorKind::Other, "{error}");
    }
    backend.remove_tree(&rp("/d")?, &Cancel::new()).map_err(io::Error::other)?;
    assert!(store.keys().is_empty(), "{:?}", store.keys());
    Ok(())
}

#[test]
fn an_empty_folder_is_removed_with_remove_and_a_full_one_is_refused() -> io::Result<()> {
    let store = Faulty::new();
    let backend = faulty_backend(&store)?;
    backend.create_dir(&rp("/d")?).map_err(io::Error::other)?;
    backend.create_dir(&rp("/d/sub")?).map_err(io::Error::other)?;
    let refused = backend.remove(&rp("/d")?).err().map(|e| (e.kind, e.path));
    assert_eq!(refused, Some((BackendErrorKind::Other, Some(rp("/d")?))), "an empty subfolder is content");
    backend.remove(&rp("/d/sub")?).map_err(io::Error::other)?;
    backend.remove(&rp("/d")?).map_err(io::Error::other)?;
    assert!(store.keys().is_empty());
    Ok(())
}

#[test]
fn an_object_and_a_folder_with_one_name_the_object_wins_and_the_folder_is_reported() -> io::Result<()> {
    let store = Faulty::new();
    store.put_raw("photos", b"a file")?;
    store.put_raw("photos/2024.jpg", b"jpeg")?;
    store.put_raw("other.txt", b"o")?;
    let backend = faulty_backend(&store)?;
    let listing = backend.list(&RemotePath::root(), &Cancel::new()).map_err(io::Error::other)?;
    let mut names: Vec<(String, bool)> = listing
        .entries
        .iter()
        .map(|e| (e.display.clone(), e.kind == kara_core::EntryKind::Directory))
        .collect();
    names.sort();
    assert_eq!(names, vec![(String::from("other.txt"), false), (String::from("photos"), false)]);
    assert_eq!(listing.errors.len(), 1);
    assert_eq!(listing.errors.first().and_then(|e| e.path.clone()), Some(rp("/photos")?));
    assert_eq!(backend.stat(&rp("/photos")?).map_err(io::Error::other)?.size, Some(6));
    let not_a_folder = backend.list(&rp("/photos")?, &Cancel::new()).err().map(|e| e.kind);
    assert_eq!(not_a_folder, Some(BackendErrorKind::Other));
    Ok(())
}

#[test]
fn names_that_are_not_keys_cannot_be_created_and_do_not_exist() -> io::Result<()> {
    let store = Faulty::new();
    let backend = faulty_backend(&store)?;
    let odd = rp("/line\nbreak")?;
    assert_eq!(backend.stat(&odd).err().map(|e| e.kind), Some(BackendErrorKind::NotFound));
    assert_eq!(backend.begin_write(&odd, None, false).err().map(|e| e.kind), Some(BackendErrorKind::Other));
    assert_eq!(backend.create_dir(&odd).err().map(|e| e.kind), Some(BackendErrorKind::Other));
    // Characters other S3 tools write literally are written literally.
    for name in ["100%.txt", "#1.txt", "a?b", "[x]", "sp ace", "ü"] {
        put(&backend, &rp(&format!("/{name}"))?, name.as_bytes())?;
        assert_eq!(store.bytes(name), Some(name.as_bytes().to_vec()), "{name}");
        assert_eq!(get(&backend, &rp(&format!("/{name}"))?)?, name.as_bytes());
    }
    Ok(())
}
