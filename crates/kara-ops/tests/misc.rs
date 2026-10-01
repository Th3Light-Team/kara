//! The small public surface that the larger suites only touch in passing:
//! labels, resolution menus, failure classes, the queue's combined figures.

use std::path::PathBuf;
use std::time::Duration;

use kara_ops::runner::Op;
use kara_ops::{
    Action, ConflictKind, Concurrency, Eta, FailureKind, JobId, JobState, Kind, Meter, Queue,
    Resolution, UndoError, UndoStack, humanize,
};

// ---- conflict menus --------------------------------------------------------

#[test]
fn only_folders_over_folders_can_be_merged() {
    for kind in [
        ConflictKind::FileOverFile,
        ConflictKind::FileOverDirectory,
        ConflictKind::DirectoryOverFile,
    ] {
        assert!(!kind.offers().contains(&Resolution::Merge), "{kind:?}");
        assert!(!kind.allows(&Resolution::Merge), "{kind:?}");
    }
    assert!(ConflictKind::DirectoryOverDirectory.offers().contains(&Resolution::Merge));
}

#[test]
fn every_conflict_offers_skip_keep_both_and_replace_and_never_defaults_to_replace() {
    for kind in [
        ConflictKind::FileOverFile,
        ConflictKind::FileOverDirectory,
        ConflictKind::DirectoryOverFile,
        ConflictKind::DirectoryOverDirectory,
    ] {
        let offered = kind.offers();
        for needed in [Resolution::Skip, Resolution::KeepBoth, Resolution::Replace] {
            assert!(offered.contains(&needed), "{kind:?} should offer {needed:?}");
        }
        assert_ne!(kind.default_resolution(), Resolution::Replace, "{kind:?}");
        // The focused option must be one of the choices on the dialog.
        assert!(offered.contains(&kind.default_resolution()), "{kind:?}");
    }
}

#[test]
fn a_typed_name_is_always_acceptable() {
    assert!(ConflictKind::FileOverDirectory.allows(&Resolution::RenameTo("x".into())));
}

// ---- failure classes -----------------------------------------------------------

#[test]
fn retrying_helps_only_when_the_cause_can_go_away_by_itself() {
    assert!(FailureKind::InUse.retry_may_help());
    assert!(FailureKind::Other.retry_may_help());
    assert!(!FailureKind::PermissionDenied.retry_may_help());
    assert!(!FailureKind::NoSpace.retry_may_help());
    assert!(!FailureKind::MediaGone.retry_may_help());
}

#[test]
fn permission_and_space_need_the_user_before_anything_else() {
    assert!(FailureKind::PermissionDenied.needs_user_action_first());
    assert!(FailureKind::NoSpace.needs_user_action_first());
    assert!(!FailureKind::InUse.needs_user_action_first());
}

// ---- labels ----------------------------------------------------------------------

#[test]
fn the_verbs_are_spanish_gerunds_and_agree_between_the_runner_and_the_queue() {
    assert_eq!(Op::Copy.gerund(), "Copiando");
    assert_eq!(Op::Move.gerund(), "Moviendo");
    assert_eq!(Op::Delete.gerund(), "Eliminando");
    assert_eq!(Op::Copy.gerund(), Kind::Copy.label());
    assert_eq!(Op::Move.gerund(), Kind::Move.label());
    assert_eq!(Op::Delete.gerund(), Kind::Delete.label());
}

#[test]
fn undo_says_what_it_would_undo_and_redo_what_it_would_redo() {
    let dir = tempfile::tempdir().expect("tempdir");
    let from = dir.path().join("a");
    let to = dir.path().join("b");
    std::fs::write(&to, "x").expect("write");

    let mut stack = UndoStack::new();
    assert_eq!(stack.undo_label(), None);
    assert_eq!(stack.redo_label(), None);

    stack.push(Action::Renamed { from, to });
    assert_eq!(stack.undo_label(), Some("renombrar"));
    stack.undo().expect("rename is undone");
    assert_eq!(stack.undo_label(), None);
    assert_eq!(stack.redo_label(), Some("renombrar"));
    assert!(stack.can_redo());
}

#[test]
fn a_new_action_forgets_what_could_have_been_redone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::write(&b, "x").expect("write");

    let mut stack = UndoStack::new();
    stack.push(Action::Renamed { from: a.clone(), to: b.clone() });
    stack.undo().expect("undo");
    assert!(stack.can_redo());

    stack.push(Action::DirectoryCreated { path: dir.path().to_path_buf() });
    assert!(!stack.can_redo(), "like browsing after going back");
}

#[test]
fn undoing_something_that_is_gone_fails_and_keeps_it_on_the_stack() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut stack = UndoStack::new();
    stack.push(Action::Renamed {
        from: dir.path().join("a"),
        to: dir.path().join("never-existed"),
    });

    match stack.undo() {
        Err(UndoError::Vanished(path)) => assert!(path.ends_with("never-existed")),
        other => panic!("expected Vanished, got {other:?}"),
    }
    assert_eq!(stack.depth(), 1, "a failed undo must not lose the record");
    assert!(stack.can_undo());
}

#[test]
fn undo_on_an_empty_stack_is_an_error_not_a_panic() {
    let mut stack = UndoStack::new();
    assert!(matches!(stack.undo(), Err(UndoError::Empty)));
    assert!(matches!(stack.redo(), Err(UndoError::Empty)));
}

// ---- the queue -----------------------------------------------------------------------

fn files(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(PathBuf::from).collect()
}

#[test]
fn a_job_title_names_the_item_or_counts_them() {
    let mut queue = Queue::new(Concurrency::Serial);
    let one = queue.push(Kind::Copy, files(&["/a/report.txt"]), Some("/b".into()));
    let many = queue.push(Kind::Delete, files(&["/a/x", "/a/y", "/a/z"]), None);

    assert_eq!(queue.get(one).expect("job").title(), "Copiando report.txt");
    assert_eq!(queue.get(many).expect("job").title(), "Eliminando 3 elementos");
}

#[test]
fn the_summary_follows_how_many_are_pending() {
    let mut queue = Queue::new(Concurrency::Serial);
    assert_eq!(queue.summary(), "Sin operaciones");
    assert_eq!(queue.pending(), 0);

    let first = queue.push(Kind::Move, files(&["/a/x"]), Some("/b".into()));
    assert_eq!(queue.summary(), "Moviendo x");
    queue.push(Kind::Copy, files(&["/a/y"]), Some("/b".into()));
    assert_eq!(queue.summary(), "2 operaciones en curso");
    assert_eq!(queue.pending(), 2);

    queue.finish(first);
    assert_eq!(queue.pending(), 1);
    assert_eq!(queue.summary(), "Copiando y");
}

#[test]
fn serial_runs_one_at_a_time_and_parallel_up_to_its_limit() {
    let mut serial = Queue::new(Concurrency::Serial);
    let a = serial.push(Kind::Copy, files(&["/x"]), None);
    serial.push(Kind::Copy, files(&["/y"]), None);
    assert_eq!(serial.next_runnable(), Some(a));
    serial.start(a);
    assert_eq!(serial.next_runnable(), None, "the second waits its turn");

    let mut parallel = Queue::new(Concurrency::Parallel(2));
    let p1 = parallel.push(Kind::Copy, files(&["/x"]), None);
    let p2 = parallel.push(Kind::Copy, files(&["/y"]), None);
    let p3 = parallel.push(Kind::Copy, files(&["/z"]), None);
    parallel.start(p1);
    assert_eq!(parallel.next_runnable(), Some(p2));
    parallel.start(p2);
    assert_eq!(parallel.next_runnable(), None);
    assert_eq!(parallel.get(p3).expect("job").state, JobState::Queued);
}

#[test]
fn cancelling_one_leaves_the_others_alone_and_prune_forgets_the_finished() {
    let mut queue = Queue::new(Concurrency::Parallel(3));
    let a = queue.push(Kind::Copy, files(&["/a"]), None);
    let b = queue.push(Kind::Copy, files(&["/b"]), None);
    let c = queue.push(Kind::Copy, files(&["/c"]), None);
    queue.start(a);
    queue.start(b);

    queue.cancel(a);
    queue.finish(b);
    assert_eq!(queue.get(a).expect("job").state, JobState::Cancelled);
    assert_eq!(queue.get(c).expect("job").state, JobState::Queued);

    queue.prune();
    assert!(queue.get(a).is_none() && queue.get(b).is_none());
    assert!(queue.get(c).is_some());
}

#[test]
fn a_finished_job_cannot_be_cancelled_into_a_lie() {
    let mut queue = Queue::new(Concurrency::Serial);
    let id = queue.push(Kind::Copy, files(&["/a"]), None);
    queue.finish(id);
    queue.cancel(id);
    assert_eq!(queue.get(id).expect("job").state, JobState::Done);
}

/// A meter that has been running long enough to promise a time left.
fn running_meter(total_bytes: u64, bytes_per_second: u64, done: u64) -> Meter {
    let mut meter = Meter::measuring();
    meter.start(Some(total_bytes), 10);
    for step in 0..5u64 {
        meter.sample(step as f64, step * bytes_per_second, step);
    }
    meter.sample(5.0, done, 5);
    meter
}

#[test]
fn the_combined_eta_ignores_jobs_that_are_already_over() {
    let mut queue = Queue::new(Concurrency::Parallel(2));
    let finished = queue.push(Kind::Copy, files(&["/old"]), None);
    let running = queue.push(Kind::Copy, files(&["/new"]), None);
    queue.start(finished);
    queue.start(running);

    *queue.meter_mut(running).expect("meter") = running_meter(1000, 100, 500);
    queue.finish(finished);

    // A finished job has no time left to give: it must not turn the whole queue's
    // estimate into «Calculando…».
    match queue.combined_eta() {
        Eta::Remaining(left) => assert!(left > Duration::ZERO),
        other => panic!("the running job's estimate should win, got {other:?}"),
    }
}

#[test]
fn the_combined_eta_is_the_longest_of_the_pending_ones() {
    let mut queue = Queue::new(Concurrency::Parallel(2));
    let quick = queue.push(Kind::Copy, files(&["/quick"]), None);
    let slow = queue.push(Kind::Copy, files(&["/slow"]), None);
    queue.start(quick);
    queue.start(slow);
    *queue.meter_mut(quick).expect("meter") = running_meter(1000, 100, 900);
    *queue.meter_mut(slow).expect("meter") = running_meter(100_000, 100, 500);

    let (Eta::Remaining(combined), Some(Eta::Remaining(slowest))) = (
        queue.combined_eta(),
        queue.get(slow).map(|job| job.meter.eta()),
    ) else {
        panic!("both jobs should have an estimate");
    };
    assert_eq!(combined, slowest);
}

#[test]
fn a_job_still_measuring_makes_the_combined_figures_unknown() {
    let mut queue = Queue::new(Concurrency::Serial);
    queue.push(Kind::Copy, files(&["/a"]), None);
    assert_eq!(queue.combined_fraction(), None);
    assert!(matches!(queue.combined_eta(), Eta::Unknown));
    assert_eq!(humanize(Eta::Unknown), "Calculando…");
}

#[test]
fn job_ids_are_never_reused() {
    let mut queue = Queue::new(Concurrency::Serial);
    let a = queue.push(Kind::Copy, files(&["/a"]), None);
    queue.finish(a);
    queue.prune();
    let b = queue.push(Kind::Copy, files(&["/b"]), None);
    assert_ne!(a, b);
    assert_eq!(b, JobId(a.0 + 1));
}

// ---- the meter ------------------------------------------------------------------------

#[test]
fn the_meter_remembers_what_it_is_working_on() {
    let mut meter = Meter::measuring();
    assert_eq!(meter.current(), None);
    meter.set_current("holiday.mp4");
    assert_eq!(meter.current(), Some("holiday.mp4"));
    meter.set_current(String::from("next.mp4"));
    assert_eq!(meter.current(), Some("next.mp4"));
}
