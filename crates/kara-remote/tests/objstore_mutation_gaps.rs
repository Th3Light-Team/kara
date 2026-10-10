//! Tests added after the mutation pass over `src/objstore` (see
//! `docs/remote-backends-testing.md`): each one fails under a mutation the
//! first pass did not catch, or pins a fix made after it.

mod objstore_support;
mod s3_mock_support;

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use kara_remote::objstore::object_store::ObjectStore;
use kara_remote::objstore::object_store::list::PaginatedListStore;
use kara_remote::objstore::{ObjectStoreBackend, ObjectStoreOptions, S3Factory};
use kara_remote::{ConnectError, DriveConfig, Secret};
use kara_vfs::{Backend, BackendErrorKind, Cancel};
use objstore_support::{Effect, Fault, Faulty, Op, faulty_backend, rp, s3_config};

/// O14: the token is watched while a page of a `remove_tree` is on its way,
/// not only between pages.
#[test]
fn cancel_stops_remove_tree_while_a_page_is_on_its_way() -> io::Result<()> {
    let store = Faulty::new();
    for i in 0..5 {
        store.put_raw(&format!("t/f{i}"), b"x")?;
    }
    let backend = faulty_backend(&store)?;
    // The head check goes through; the first page hangs.
    store.inject(Fault::on(Op::List, Effect::Stall(Duration::from_secs(20))));
    let cancel = Cancel::new();
    let canceller = {
        let cancel = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            cancel.cancel();
        })
    };
    let started = Instant::now();
    let result = backend.remove_tree(&rp("/t")?, &cancel);
    let _ = canceller.join();
    assert_eq!(result.err().map(|e| e.kind), Some(BackendErrorKind::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    assert_eq!(store.keys().len(), 5, "nothing was deleted");
    Ok(())
}

/// O19: with no secret an S3 drive says `AuthRequired` before talking to
/// the service, so the registry asks for the secret instead of failing.
#[test]
fn a_missing_s3_secret_is_auth_required_before_any_request() -> io::Result<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Faulty::new();
    let factory = {
        let calls = Arc::clone(&calls);
        let store = Arc::clone(&store);
        S3Factory::with_connector(Arc::new(move |_params, _secret| {
            calls.fetch_add(1, Ordering::SeqCst);
            let pager: Arc<dyn PaginatedListStore> = Arc::clone(&store) as _;
            let os: Arc<dyn ObjectStore> = Arc::clone(&store) as _;
            Ok((os, Some(pager)))
        }))
    };
    let config = s3_config("b", &[("bucket", "b"), ("access_key_id", "AKIA1")])?;
    assert_eq!(factory.open(&config, None, &Cancel::new()).err(), Some(ConnectError::AuthRequired));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(store.counts.get(Op::List), 0);
    // With the ambient chain no secret is needed.
    let env = s3_config("b", &[("bucket", "b"), ("credentials", "env")])?;
    factory.open(&env, None, &Cancel::new()).map_err(io::Error::other)?;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    Ok(())
}

/// After the pass: a part is given time for a slow uplink, not the short
/// bound of metadata calls (which would abort uploads on a slow link).
#[test]
fn a_slow_part_is_given_more_time_than_a_metadata_call() -> io::Result<()> {
    let store = Faulty::new();
    let pager: Arc<dyn PaginatedListStore> = Arc::clone(&store) as _;
    let os: Arc<dyn ObjectStore> = Arc::clone(&store) as _;
    let backend = ObjectStoreBackend::new_with_small_parts(
        os,
        ObjectStoreOptions {
            pager: Some(pager),
            // Metadata calls are cut after 4 × 0.2 + 5 = 5.8 s.
            timeout: Duration::from_millis(200),
            part_size: 4 * 1024 * 1024,
            upload_concurrency: 1,
            ..ObjectStoreOptions::default()
        },
    )
    .map_err(io::Error::other)?;
    store.inject(Fault::on(Op::Part, Effect::Stall(Duration::from_secs(7))));
    let path = rp("/slow-uplink.bin")?;
    let data: Vec<u8> = (0..4 * 1024 * 1024 + 10).map(|i| (i % 251) as u8).collect();
    let mut session = backend.begin_write(&path, None, false).map_err(io::Error::other)?;
    session.write_all(&data)?;
    session.finish().map_err(io::Error::other)?;
    assert_eq!(store.bytes("slow-uplink.bin"), Some(data));
    Ok(())
}

/// After the pass: a download longer than four times the timeout is not cut
/// as long as data keeps coming (`object_store`'s default bounds the whole
/// request, body included, to 30 s).
#[test]
fn a_long_download_that_keeps_moving_is_not_cut() -> io::Result<()> {
    let mock = s3_mock_support::S3Mock::start()?;
    let data: Vec<u8> = (0..1_000_000).map(|i| (i % 253) as u8).collect();
    mock.state.put_raw("long.bin", &data);
    let config = DriveConfig::new(
        "s3",
        "mock",
        "Mock",
        [
            ("bucket", s3_mock_support::BUCKET),
            ("endpoint", mock.endpoint().as_str()),
            ("allow_http", "true"),
            ("access_key_id", s3_mock_support::ACCESS_KEY),
            ("timeout_s", "1"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned())),
    )
    .map_err(|e| io::Error::other(e.to_string()))?;
    let backend = S3Factory::new()
        .open(&config, Some(&Secret::new(s3_mock_support::SECRET_KEY)), &Cancel::new())
        .map_err(io::Error::other)?;
    // 16 pieces, 400 ms apart: 6 s in all, never silent for 1 s.
    mock.state.trickle_ms.store(400, Ordering::SeqCst);
    let started = Instant::now();
    let mut out = Vec::new();
    backend
        .open_read(&rp("/long.bin")?, 0)
        .map_err(io::Error::other)?
        .read_to_end(&mut out)?;
    assert!(started.elapsed() > Duration::from_secs(5), "{:?}", started.elapsed());
    assert_eq!(out, data);
    Ok(())
}
