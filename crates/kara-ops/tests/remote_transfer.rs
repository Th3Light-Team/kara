//! Milestone 2: copies and moves that involve a remote drive. Streaming,
//! server-side copy, size verification, abort, cancel, retry / skip / cancel.

mod remote_common;

use std::fs;
use std::sync::Arc;
use std::thread;

use kara_ops::runner::Op;
use kara_ops::{Action, ErrorDecision, FailureKind};
use kara_vfs::memory::{Fault, FaultEffect, MemoryBackend, Op as MemOp};
use kara_vfs::{Backend, BackendErrorKind, Location};
use remote_common::*;

const BIG: usize = 700 * 1024; // more than two chunks of the runner

fn fail(op: MemOp, path: Option<&str>, after: u64, kind: BackendErrorKind, times: Option<u32>) -> Fault {
    Fault {
        op,
        path: path.map(rpath),
        after,
        effect: FaultEffect::Fail(kind),
        times,
    }
}

fn two_drives() -> (Arc<MemoryBackend>, Arc<MemoryBackend>, kara_ops::BackendResolver) {
    let a = Arc::new(MemoryBackend::posix_like());
    let b = Arc::new(MemoryBackend::posix_like());
    let resolver = resolver(vec![(drive("a"), a.clone()), (drive("b"), b.clone())]);
    (a, b, resolver)
}

// ---------------------------------------------------------------------------
// Plain transfers.

#[test]
fn a_local_file_is_streamed_onto_a_remote_drive() {
    let (_root, src, _) = local_world();
    let data = bytes(BIG);
    write(&src.join("big.bin"), &data);
    let memory = Arc::new(MemoryBackend::posix_like());
    mkdir(memory.as_ref(), "/in");
    let id = drive("nas");
    let resolver = resolver(vec![(id.clone(), memory.clone())]);

    let (outcome, transcript) = run(
        request(Op::Copy, vec![Location::Local(src.join("big.bin"))], rloc(&id, "/in")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(content(&memory, "/in/big.bin"), Some(data));
    assert_eq!(transcript.started, Some((BIG as u64, 1)));
    assert_eq!(transcript.progress.last(), Some(&(BIG as u64, 1)));
    match outcome.actions.as_slice() {
        [Action::RemoteCopied { created }] => assert_eq!(created, &rloc(&id, "/in/big.bin")),
        other => panic!("expected one RemoteCopied, got {other:?}"),
    }
    assert!(src.join("big.bin").exists(), "a copy keeps its source");
}

#[test]
fn a_remote_tree_is_copied_to_a_local_folder_and_measured_first() {
    let (_root, _, dst) = local_world();
    let memory = Arc::new(MemoryBackend::object_store_like());
    put(memory.as_ref(), "/d/a", b"aa");
    put(memory.as_ref(), "/d/e/b", b"bbb");
    let id = drive("s3");
    let resolver = resolver(vec![(id.clone(), memory.clone())]);

    let (outcome, transcript) = run(
        request(Op::Copy, vec![rloc(&id, "/d")], Location::Local(dst.clone())),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(fs::read(dst.join("d/a")).unwrap(), b"aa");
    assert_eq!(fs::read(dst.join("d/e/b")).unwrap(), b"bbb");
    assert_eq!(transcript.started, Some((5, 4)), "d, d/a, d/e, d/e/b");
    assert!(matches!(outcome.actions.as_slice(), [Action::Copied { created }] if *created == dst.join("d")));
}

#[test]
fn a_tree_moves_between_two_drives_and_the_source_goes_only_at_the_end() {
    let (a, b, resolver) = two_drives();
    put(a.as_ref(), "/d/x", b"x");
    put(a.as_ref(), "/d/e/y", &bytes(BIG));
    mkdir(b.as_ref(), "/t");

    let (outcome, _) = run(
        request(Op::Move, vec![rloc(&drive("a"), "/d")], rloc(&drive("b"), "/t")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(content(&b, "/t/d/x"), Some(b"x".to_vec()));
    assert_eq!(content(&b, "/t/d/e/y"), Some(bytes(BIG)));
    assert!(keys(&a).is_empty(), "the source tree is gone: {:?}", keys(&a));
    match outcome.actions.as_slice() {
        [Action::NotUndoable { label, reason, .. }] => {
            assert_eq!(*label, "mover");
            assert!(reason.contains("unidades distintas"));
        }
        other => panic!("a move across drives is not undoable, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Failures mid-copy.

#[test]
fn a_failure_mid_copy_leaves_no_destination_and_keeps_the_source_of_a_move() {
    let (a, b, resolver) = two_drives();
    put(a.as_ref(), "/f.bin", &bytes(BIG));
    b.inject(fail(MemOp::Write, Some("/f.bin"), 300 * 1024, BackendErrorKind::NoSpace, None))
        .unwrap();

    let (outcome, transcript) = run(
        request(Op::Move, vec![rloc(&drive("a"), "/f.bin")], rloc(&drive("b"), "/")),
        &resolver,
        &Script::errors(ErrorDecision::Skip),
    );

    assert_eq!(content(&a, "/f.bin"), Some(bytes(BIG)), "the source is untouched");
    assert!(keys(&b).is_empty(), "nothing under the final name: {:?}", keys(&b));
    assert_eq!(b.stats().unwrap().open_sessions, 0);
    assert_eq!(transcript.failures.len(), 1);
    let prompt = &transcript.failures[0];
    assert_eq!(prompt.kind, FailureKind::NoSpace);
    assert_eq!(prompt.path, uri(&drive("a"), "/f.bin"), "the prompt names the URI");
    assert_eq!(outcome.report.skipped, vec![uri(&drive("a"), "/f.bin")]);
    assert!(outcome.actions.is_empty());
}

#[test]
fn a_failed_copy_aborts_its_session() {
    let a = Arc::new(MemoryBackend::posix_like());
    let b = Arc::new(MemoryBackend::posix_like());
    let spy = Arc::new(Spy::new(b.clone()));
    put(a.as_ref(), "/f", &bytes(BIG));
    b.inject(fail(MemOp::Write, None, 1024, BackendErrorKind::Other, None)).unwrap();
    let resolver = resolver(vec![(drive("a"), a.clone()), (drive("b"), spy.clone())]);

    run(
        request(Op::Copy, vec![rloc(&drive("a"), "/f")], rloc(&drive("b"), "/")),
        &resolver,
        &Script::errors(ErrorDecision::Skip),
    );

    assert_eq!(spy.called("abort"), 1, "{:?}", spy.calls.lock().unwrap());
    assert_eq!(spy.called("finish"), 0);
    assert_eq!(spy.called("dropped"), 0, "a session is aborted, not just dropped");
}

#[test]
fn cancelling_mid_copy_aborts_the_session_and_keeps_the_source() {
    let a = Arc::new(MemoryBackend::posix_like());
    let b = Arc::new(MemoryBackend::posix_like());
    put(a.as_ref(), "/f", &bytes(BIG));
    let (gated, gate) = Gated::new(a.clone(), 256 * 1024);
    let spy = Arc::new(Spy::new(b.clone()));
    let resolver = resolver(vec![(drive("a"), Arc::new(gated)), (drive("b"), spy.clone())]);

    let job = start(
        request(Op::Move, vec![rloc(&drive("a"), "/f")], rloc(&drive("b"), "/")),
        resolver,
    );
    gate.wait();
    job.handle.cancel();
    gate.release();
    let (outcome, _) = job.finish(&Script::errors(ErrorDecision::Cancel));

    assert!(outcome.cancelled);
    assert!(outcome.report.cancelled);
    assert_eq!(spy.called("abort"), 1, "{:?}", spy.calls.lock().unwrap());
    assert_eq!(spy.called("finish"), 0);
    assert!(keys(&b).is_empty(), "{:?}", keys(&b));
    assert_eq!(content(&a, "/f"), Some(bytes(BIG)));
    assert!(outcome.actions.is_empty());
}

#[test]
fn a_paused_copy_waits_at_a_chunk_boundary_and_a_cancel_still_works() {
    let a = Arc::new(MemoryBackend::posix_like());
    let b = Arc::new(MemoryBackend::posix_like());
    put(a.as_ref(), "/f", &bytes(BIG));
    let (gated, gate) = Gated::new(a.clone(), 256 * 1024);
    let resolver = resolver(vec![(drive("a"), Arc::new(gated)), (drive("b"), b.clone())]);

    let job = start(
        request(Op::Copy, vec![rloc(&drive("a"), "/f")], rloc(&drive("b"), "/")),
        resolver,
    );
    gate.wait();
    job.handle.pause();
    assert!(job.handle.is_paused());
    gate.release();
    thread::sleep(std::time::Duration::from_millis(300));
    let written = b.stats().unwrap().bytes_written;
    assert!(written < BIG as u64, "paused: the copy did not run to the end ({written})");
    thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!(b.stats().unwrap().bytes_written, written, "nothing moves while paused");
    job.handle.cancel();
    let (outcome, _) = job.finish(&Script::errors(ErrorDecision::Cancel));

    assert!(outcome.cancelled);
    assert!(keys(&b).is_empty());
}

#[test]
fn a_size_mismatch_keeps_the_source_and_removes_the_bad_copy() {
    let a = Arc::new(MemoryBackend::posix_like());
    let b = Arc::new(MemoryBackend::posix_like());
    put(a.as_ref(), "/f", &bytes(BIG));
    let liar = LyingStat {
        inner: b.clone(),
        lie_under: rpath("/"),
        delta: 1,
    };
    let resolver = resolver(vec![(drive("a"), a.clone()), (drive("b"), Arc::new(liar))]);

    let (outcome, transcript) = run(
        request(Op::Move, vec![rloc(&drive("a"), "/f")], rloc(&drive("b"), "/")),
        &resolver,
        &Script::errors(ErrorDecision::Skip),
    );

    assert_eq!(content(&a, "/f"), Some(bytes(BIG)), "the source survives a mismatch");
    assert!(keys(&b).is_empty(), "the mismatched copy is removed: {:?}", keys(&b));
    assert_eq!(transcript.failures.len(), 1);
    assert!(transcript.failures[0].reason.contains("tamaño"));
    assert!(outcome.actions.is_empty());
}

#[test]
fn a_size_mismatch_after_a_server_side_copy_keeps_the_source_too() {
    let memory = Arc::new(MemoryBackend::object_store_like());
    put(memory.as_ref(), "/a/f", &bytes(1000));
    let liar = LyingStat {
        inner: memory.clone(),
        lie_under: rpath("/b"),
        delta: 7,
    };
    let id = drive("s3");
    let resolver = resolver(vec![(id.clone(), Arc::new(liar))]);

    let (outcome, _) = run(
        request(Op::Move, vec![rloc(&id, "/a/f")], rloc(&id, "/b")),
        &resolver,
        &Script::errors(ErrorDecision::Skip),
    );

    assert_eq!(content(&memory, "/a/f"), Some(bytes(1000)));
    assert_eq!(content(&memory, "/b/f"), None);
    assert_eq!(outcome.report.failures.len(), 1);
}

#[test]
fn one_failing_child_keeps_the_whole_source_tree_of_a_move() {
    let (a, b, resolver) = two_drives();
    put(a.as_ref(), "/d/1", b"one");
    put(a.as_ref(), "/d/2", b"two");
    put(a.as_ref(), "/d/3", b"three");
    b.inject(fail(MemOp::BeginWrite, Some("/d/2"), 0, BackendErrorKind::PermissionDenied, None))
        .unwrap();

    let (outcome, transcript) = run(
        request(Op::Move, vec![rloc(&drive("a"), "/d")], rloc(&drive("b"), "/")),
        &resolver,
        &Script::errors(ErrorDecision::Skip),
    );

    for name in ["/d/1", "/d/2", "/d/3"] {
        assert!(exists(a.as_ref(), name), "{name} must stay in the source");
    }
    assert_eq!(content(&b, "/d/1"), Some(b"one".to_vec()));
    assert_eq!(content(&b, "/d/3"), Some(b"three".to_vec()));
    assert!(!exists(b.as_ref(), "/d/2"));
    assert_eq!(transcript.failures.len(), 1);
    assert_eq!(transcript.failures[0].kind, FailureKind::PermissionDenied);
    assert_eq!(outcome.report.skipped, vec![uri(&drive("a"), "/d/2")]);
}

#[test]
fn retry_after_a_transient_failure_copies_once_and_counts_bytes_once() {
    let (a, b, resolver) = two_drives();
    put(a.as_ref(), "/f", &bytes(BIG));
    b.inject(fail(MemOp::Write, Some("/f"), 400 * 1024, BackendErrorKind::Other, Some(1)))
        .unwrap();

    let (outcome, transcript) = run(
        request(Op::Move, vec![rloc(&drive("a"), "/f")], rloc(&drive("b"), "/")),
        &resolver,
        &Script::errors(ErrorDecision::Retry),
    );

    assert_eq!(transcript.failures.len(), 1, "asked once");
    assert!(outcome.report.failures.is_empty(), "{:?}", outcome.report);
    assert_eq!(content(&b, "/f"), Some(bytes(BIG)));
    assert!(!exists(a.as_ref(), "/f"));
    assert_eq!(transcript.progress.last(), Some(&(BIG as u64, 1)));
}

#[test]
fn cancel_from_a_failure_prompt_stops_the_batch() {
    let (a, b, resolver) = two_drives();
    put(a.as_ref(), "/1", b"1");
    put(a.as_ref(), "/2", b"2");
    b.inject(fail(MemOp::BeginWrite, Some("/1"), 0, BackendErrorKind::Other, None)).unwrap();

    let (outcome, _) = run(
        request(
            Op::Copy,
            vec![rloc(&drive("a"), "/1"), rloc(&drive("a"), "/2")],
            rloc(&drive("b"), "/"),
        ),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.cancelled);
    assert!(!exists(b.as_ref(), "/2"), "nothing after the cancel");
}

#[test]
fn skip_all_does_not_ask_again() {
    let (a, b, resolver) = two_drives();
    for name in ["/1", "/2", "/3"] {
        put(a.as_ref(), name, b"x");
    }
    b.inject(fail(MemOp::BeginWrite, None, 0, BackendErrorKind::Other, None)).unwrap();

    let (outcome, transcript) = run(
        request(
            Op::Copy,
            vec![rloc(&drive("a"), "/1"), rloc(&drive("a"), "/2"), rloc(&drive("a"), "/3")],
            rloc(&drive("b"), "/"),
        ),
        &resolver,
        &Script::errors(ErrorDecision::SkipAll),
    );

    assert_eq!(transcript.failures.len(), 1);
    assert_eq!(outcome.report.skipped.len(), 3);
    assert!(!outcome.cancelled);
}

#[test]
fn a_lost_connection_is_media_gone_and_a_blanket_retry_does_not_loop() {
    let (a, b, resolver) = two_drives();
    put(a.as_ref(), "/1", b"1");
    put(a.as_ref(), "/2", b"2");
    b.disconnect().unwrap();

    // Retry once (a blanket retry), then the user skips: if the policy
    // retried MediaGone on its own this would never finish.
    let job = start(
        request(Op::Copy, vec![rloc(&drive("a"), "/1"), rloc(&drive("a"), "/2")], rloc(&drive("b"), "/")),
        resolver,
    );
    let mut prompts = Vec::new();
    let outcome = loop {
        match job.next() {
            kara_ops::runner::Event::Failure(prompt) => {
                let decision = if prompts.is_empty() {
                    ErrorDecision::Retry
                } else {
                    ErrorDecision::SkipAll
                };
                prompts.push(prompt);
                job.handle.answer(kara_ops::runner::Answer::Error(decision));
            }
            kara_ops::runner::Event::Finished(outcome) => break outcome,
            _ => {}
        }
    };
    assert!(prompts.iter().all(|p| p.kind == FailureKind::MediaGone), "{prompts:?}");
    assert_eq!(prompts.len(), 2, "a retry asks again instead of looping");
    assert_eq!(outcome.report.skipped.len(), 2);
}

// ---------------------------------------------------------------------------
// Which route.

#[test]
fn a_copy_inside_a_drive_with_server_side_copy_moves_no_bytes() {
    let memory = Arc::new(MemoryBackend::object_store_like());
    put(memory.as_ref(), "/a/f", &bytes(BIG));
    put(memory.as_ref(), "/a/d/g", &bytes(10));
    let spy = Arc::new(Spy::new(memory.clone()));
    let id = drive("s3");
    let resolver = resolver(vec![(id.clone(), spy.clone())]);
    let before = memory.stats().unwrap();

    let (outcome, transcript) = run(
        request(Op::Copy, vec![rloc(&id, "/a/f"), rloc(&id, "/a/d")], rloc(&id, "/b")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    let after = memory.stats().unwrap();
    assert_eq!(after.bytes_read, before.bytes_read, "no byte read");
    assert_eq!(after.bytes_written, before.bytes_written, "no byte written");
    assert_eq!(spy.called("copy_within"), 2);
    assert_eq!(spy.called("open_read"), 0);
    assert_eq!(content(&memory, "/b/f"), Some(bytes(BIG)));
    assert_eq!(content(&memory, "/b/d/g"), Some(bytes(10)));
    assert_eq!(transcript.progress.last(), Some(&((BIG + 10) as u64, 3)));
}

#[test]
fn a_copy_inside_a_drive_without_server_side_copy_is_streamed() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/a/f", &bytes(BIG));
    let spy = Arc::new(Spy::new(memory.clone()));
    let id = drive("nas");
    let resolver = resolver(vec![(id.clone(), spy.clone())]);
    let before = memory.stats().unwrap();

    run(
        request(Op::Copy, vec![rloc(&id, "/a/f")], rloc(&id, "/")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    let after = memory.stats().unwrap();
    assert_eq!(after.bytes_read - before.bytes_read, BIG as u64);
    assert_eq!(after.bytes_written - before.bytes_written, BIG as u64);
    assert_eq!(spy.called("copy_within"), 0);
    assert_eq!(content(&memory, "/f"), Some(bytes(BIG)));
}

#[test]
fn a_move_inside_a_drive_with_atomic_rename_is_one_rename() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/a/d/f", &bytes(BIG));
    put(memory.as_ref(), "/a/g", b"g");
    mkdir(memory.as_ref(), "/b");
    let spy = Arc::new(Spy::new(memory.clone()));
    let id = drive("nas");
    let resolver = resolver(vec![(id.clone(), spy.clone())]);
    let before = memory.stats().unwrap();

    let (outcome, _) = run(
        request(Op::Move, vec![rloc(&id, "/a/d"), rloc(&id, "/a/g")], rloc(&id, "/b")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(spy.called("rename"), 2);
    assert_eq!(memory.stats().unwrap(), before, "no byte moved");
    assert_eq!(content(&memory, "/b/d/f"), Some(bytes(BIG)));
    assert!(!exists(memory.as_ref(), "/a/d"));
    match outcome.actions.as_slice() {
        [Action::RemoteMoved { from, to }, Action::RemoteMoved { .. }] => {
            assert_eq!(from, &rloc(&id, "/a/d"));
            assert_eq!(to, &rloc(&id, "/b/d"));
        }
        other => panic!("expected two RemoteMoved, got {other:?}"),
    }
}

#[test]
fn a_move_inside_an_object_store_is_copy_verify_delete() {
    let memory = Arc::new(MemoryBackend::object_store_like());
    put(memory.as_ref(), "/a/d/f", &bytes(100));
    put(memory.as_ref(), "/a/g", b"g");
    let spy = Arc::new(Spy::new(memory.clone()));
    let id = drive("s3");
    let resolver = resolver(vec![(id.clone(), spy.clone())]);

    let (outcome, _) = run(
        request(Op::Move, vec![rloc(&id, "/a/d"), rloc(&id, "/a/g")], rloc(&id, "/b")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(spy.called("rename"), 0, "no atomic rename on an object store");
    assert_eq!(spy.called("copy_within"), 2);
    assert_eq!(content(&memory, "/b/d/f"), Some(bytes(100)));
    assert_eq!(content(&memory, "/b/g"), Some(b"g".to_vec()));
    assert!(!exists(memory.as_ref(), "/a/g"));
    assert!(!exists(memory.as_ref(), "/a/d/f"));
    assert!(
        outcome.actions.iter().all(|a| matches!(a, Action::NotUndoable { label: "mover", .. })),
        "{:?}",
        outcome.actions
    );
}

#[test]
fn a_symlink_is_not_turned_into_a_copy_of_its_target() {
    let (a, b, resolver) = two_drives();
    put(a.as_ref(), "/d/real", b"r");
    a.create_symlink(&rpath("/d/link"), "real").unwrap();

    let (outcome, _) = run(
        request(Op::Move, vec![rloc(&drive("a"), "/d")], rloc(&drive("b"), "/")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(!exists(b.as_ref(), "/d/link"));
    assert_eq!(content(&b, "/d/real"), Some(b"r".to_vec()));
    assert!(exists(a.as_ref(), "/d/link"), "a partial move keeps the source");
    assert!(exists(a.as_ref(), "/d/real"));
    assert_eq!(outcome.report.failures.len(), 1);
    assert_eq!(outcome.report.failures[0].path, uri(&drive("a"), "/d/link"));
}

#[test]
fn copying_a_remote_folder_into_itself_is_refused() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/d/f", b"f");
    let id = drive("nas");
    let resolver = resolver(vec![(id.clone(), memory.clone())]);

    let (outcome, _) = run(
        request(Op::Copy, vec![rloc(&id, "/d")], rloc(&id, "/d")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert_eq!(keys(&memory), vec!["/d".to_string(), "/d/f".to_string()]);
    assert_eq!(outcome.report.failures.len(), 1);
    assert!(outcome.report.failures[0].reason.contains("dentro de sí misma"));
}

#[test]
fn pasting_a_copy_where_it_already_is_makes_a_numbered_sibling() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/d/a.txt", b"a");
    let id = drive("nas");
    let resolver = resolver(vec![(id.clone(), memory.clone())]);

    let (outcome, transcript) = run(
        request(Op::Copy, vec![rloc(&id, "/d/a.txt")], rloc(&id, "/d")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(transcript.conflicts.is_empty());
    assert!(outcome.report.is_clean());
    assert_eq!(content(&memory, "/d/a (2).txt"), Some(b"a".to_vec()));
}

#[test]
fn a_local_source_with_a_name_the_drive_cannot_take_is_reported_not_guessed() {
    use std::os::unix::ffi::OsStrExt;
    let (_root, src, _) = local_world();
    let odd = src.join(std::ffi::OsStr::from_bytes(b"bad\xff"));
    write(&odd, b"x");
    let memory = Arc::new(MemoryBackend::posix_like());
    let id = drive("nas");
    let resolver = resolver(vec![(id.clone(), memory.clone())]);

    let (outcome, _) = run(
        request(Op::Copy, vec![Location::Local(odd.clone())], rloc(&id, "/")),
        &resolver,
        &Script::errors(ErrorDecision::Skip),
    );

    assert!(keys(&memory).is_empty());
    assert_eq!(outcome.report.skipped, vec![odd]);
    let _ = Backend::capabilities(memory.as_ref());
}

#[test]
fn a_file_that_appears_in_the_source_during_a_folder_move_is_not_deleted() {
    let a = Arc::new(MemoryBackend::posix_like());
    let b = Arc::new(MemoryBackend::posix_like());
    put(a.as_ref(), "/d/a", &bytes(BIG));
    put(a.as_ref(), "/d/b", b"b");
    let (gated, gate) = Gated::new(a.clone(), 256 * 1024);
    let resolver = resolver(vec![(drive("a"), Arc::new(gated)), (drive("b"), b.clone())]);

    let job = start(
        request(Op::Move, vec![rloc(&drive("a"), "/d")], rloc(&drive("b"), "/")),
        resolver,
    );
    gate.wait();
    // Written by someone else after the folder was listed: never copied.
    put(a.as_ref(), "/d/zz", b"recien llegado");
    gate.release();
    let (outcome, _) = job.finish(&Script::errors(ErrorDecision::Cancel));

    assert!(!outcome.cancelled);
    assert_eq!(content(&b, "/d/a"), Some(bytes(BIG)));
    assert_eq!(content(&b, "/d/b"), Some(b"b".to_vec()));
    assert_eq!(content(&a, "/d/zz"), Some(b"recien llegado".to_vec()), "never copied, never deleted");
    assert!(!exists(a.as_ref(), "/d/a") && !exists(a.as_ref(), "/d/b"), "{:?}", keys(&a));
}

#[test]
fn a_tree_moves_out_of_an_object_store_and_leaves_no_prefix_behind() {
    let a = Arc::new(MemoryBackend::object_store_like());
    let b = Arc::new(MemoryBackend::posix_like());
    put(a.as_ref(), "/d/x", b"x");
    put(a.as_ref(), "/d/e/y", b"y");
    let resolver = resolver(vec![(drive("a"), a.clone()), (drive("b"), b.clone())]);

    let (outcome, transcript) = run(
        request(Op::Move, vec![rloc(&drive("a"), "/d")], rloc(&drive("b"), "/")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(transcript.failures.is_empty(), "{:?}", transcript.failures);
    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(content(&b, "/d/e/y"), Some(b"y".to_vec()));
    assert!(keys(&a).is_empty(), "{:?}", keys(&a));
}

#[test]
fn a_local_folder_moved_to_a_drive_is_removed_locally_only_after_it_arrived() {
    let (_root, src, _) = local_world();
    fs::create_dir_all(src.join("d/e")).unwrap();
    write(&src.join("d/1"), b"1");
    write(&src.join("d/e/2"), b"2");
    let memory = Arc::new(MemoryBackend::posix_like());
    let id = drive("nas");
    let resolver = resolver(vec![(id.clone(), memory.clone())]);

    let (outcome, _) = run(
        request(Op::Move, vec![Location::Local(src.join("d"))], rloc(&id, "/")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(content(&memory, "/d/e/2"), Some(b"2".to_vec()));
    assert!(!src.join("d").exists());
}
