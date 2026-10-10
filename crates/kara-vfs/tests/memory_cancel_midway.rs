//! A cancel token flipped while `MemoryBackend::remove_tree` is walking must
//! stop the walk. The fault-driven cancel test (cb_38) cancels from inside the
//! backend, so it never exercised the per-object check of the token itself; a
//! mutation that removed that check survived. This test flips the token from
//! another thread, on a drive big enough that the walk is still running.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use kara_vfs::memory::MemoryBackend;
use kara_vfs::{Backend, BackendErrorKind, Cancel, RemotePath};

const OBJECTS: usize = 100_000;

fn p(text: &str) -> RemotePath {
    match RemotePath::parse(text) {
        Ok(path) => path,
        Err(error) => panic!("{text:?}: {error}"),
    }
}

fn big_drive() -> MemoryBackend {
    let m = MemoryBackend::posix_like();
    if let Err(error) = m.create_dir(&p("/t")) {
        panic!("create /t: {error:?}");
    }
    for index in 0..OBJECTS {
        if let Err(error) = m.create_dir(&p(&format!("/t/d{index:06}"))) {
            panic!("create_dir: {error:?}");
        }
    }
    m
}

#[test]
fn a_token_cancelled_mid_walk_stops_remove_tree() {
    // How long an uncancelled walk of this drive takes here.
    let reference = big_drive();
    let started = Instant::now();
    if let Err(error) = reference.remove_tree(&p("/t"), &Cancel::new()) {
        panic!("uncancelled remove_tree: {error:?}");
    }
    let whole = started.elapsed();

    for delay in [Duration::from_millis(1), Duration::from_millis(2), Duration::from_millis(4)] {
        if whole < delay * 5 {
            eprintln!("skipped {delay:?}: the whole walk takes only {whole:?} here");
            continue;
        }
        let m = Arc::new(big_drive());
        let cancel = Cancel::new();
        let worker = {
            let m = Arc::clone(&m);
            let cancel = cancel.clone();
            thread::spawn(move || m.remove_tree(&p("/t"), &cancel))
        };
        thread::sleep(delay);
        cancel.cancel();
        let outcome = match worker.join() {
            Ok(outcome) => outcome,
            Err(_) => panic!("the remove_tree thread panicked"),
        };
        match outcome {
            Ok(()) => panic!(
                "cancelled {delay:?} into a {whole:?} walk, remove_tree removed everything"
            ),
            Err(error) => {
                assert_eq!(error.kind, BackendErrorKind::Cancelled, "{error:?}");
                assert_eq!(error.path, Some(p("/t")), "{error:?}");
            }
        }
        let snapshot = match m.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => panic!("snapshot: {error:?}"),
        };
        assert!(
            snapshot.nodes.contains_key(&p("/t")),
            "children go before parents: the root of a cancelled walk stays"
        );
    }
}
