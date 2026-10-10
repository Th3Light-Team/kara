//! Milestone 3: permanent delete on a drive, rename and new folder, conflict
//! resolution against a remote destination, and the error mapping.

mod remote_common;

use std::fs;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kara_core::FileEntry;
use kara_ops::runner::{Answer, Event, Op};
use kara_ops::{
    Action, ConflictKind, ErrorDecision, FailureKind, LocationOpError, Resolution, create_dir_at,
    failure_kind, rename_at,
};
use kara_vfs::memory::{Fault, FaultEffect, MemoryBackend, Op as MemOp};
use kara_vfs::{
    Backend, BackendError, BackendErrorKind, Cancel, Capabilities, Listing, Location, RemotePath,
    WriteSession,
};
use remote_common::*;

fn nas() -> (Arc<MemoryBackend>, kara_vfs::DriveId, kara_ops::BackendResolver) {
    let memory = Arc::new(MemoryBackend::posix_like());
    let id = drive("nas");
    let resolver = resolver(vec![(id.clone(), memory.clone() as Arc<dyn Backend>)]);
    (memory, id, resolver)
}

// ---------------------------------------------------------------------------
// Error mapping.

#[test]
fn every_backend_error_kind_maps_onto_the_failure_kinds() {
    use BackendErrorKind as B;
    let table = [
        (B::NotFound, FailureKind::Other),
        (B::AlreadyExists, FailureKind::Other),
        (B::PermissionDenied, FailureKind::PermissionDenied),
        (B::NoSpace, FailureKind::NoSpace),
        (B::Unavailable, FailureKind::MediaGone),
        (B::AuthRequired, FailureKind::PermissionDenied),
        (B::Unsupported, FailureKind::Other),
        (B::Cancelled, FailureKind::Other),
        (B::Other, FailureKind::Other),
    ];
    for (kind, expected) in table {
        assert_eq!(failure_kind(kind), expected, "{kind:?}");
    }
    assert!(!failure_kind(B::Unavailable).retry_may_help(), "no endless retry");
    assert!(failure_kind(B::AuthRequired).needs_user_action_first());
}

#[test]
fn a_failure_prompt_carries_the_mapped_kind_of_each_backend_error() {
    for (kind, expected) in [
        (BackendErrorKind::AuthRequired, FailureKind::PermissionDenied),
        (BackendErrorKind::NoSpace, FailureKind::NoSpace),
        (BackendErrorKind::Unavailable, FailureKind::MediaGone),
        (BackendErrorKind::Unsupported, FailureKind::Other),
    ] {
        let (memory, id, resolver) = nas();
        put(memory.as_ref(), "/f", b"f");
        memory
            .inject(Fault {
                op: MemOp::OpenRead,
                path: None,
                after: 0,
                effect: FaultEffect::Fail(kind),
                times: None,
            })
            .unwrap();
        let (_root, _, dst) = local_world();
        let (_, transcript) = run(
            request(Op::Copy, vec![rloc(&id, "/f")], Location::Local(dst.clone())),
            &resolver,
            &Script::errors(ErrorDecision::Skip),
        );
        assert_eq!(transcript.failures.len(), 1, "{kind:?}");
        assert_eq!(transcript.failures[0].kind, expected, "{kind:?}");
        assert_eq!(transcript.failures[0].path, uri(&id, "/f"));
        assert!(!dst.join("f").exists(), "nothing half-written locally");
    }
}

// ---------------------------------------------------------------------------
// Permanent delete.

#[test]
fn a_confirmed_remote_delete_removes_the_tree_and_records_nothing() {
    let (memory, id, resolver) = nas();
    put(memory.as_ref(), "/d/e/f", b"f");
    put(memory.as_ref(), "/d/g", b"g");
    put(memory.as_ref(), "/keep", b"k");

    let (outcome, transcript) = run(
        request(Op::Delete, vec![rloc(&id, "/d")], rloc(&id, "/")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(keys(&memory), vec!["/keep".to_string()]);
    assert!(outcome.actions.is_empty(), "a permanent delete cannot be undone");
    assert_eq!(transcript.progress.last().map(|p| p.1), Some(4), "d, d/e, d/e/f, d/g");
}

#[test]
fn a_failing_remote_delete_asks_and_skip_leaves_it() {
    let (memory, id, resolver) = nas();
    put(memory.as_ref(), "/a/x", b"x");
    put(memory.as_ref(), "/b", b"b");
    memory
        .inject(Fault {
            op: MemOp::RemoveTree,
            path: Some(rpath("/a")),
            after: 0,
            effect: FaultEffect::Fail(BackendErrorKind::PermissionDenied),
            times: None,
        })
        .unwrap();

    let (outcome, transcript) = run(
        request(Op::Delete, vec![rloc(&id, "/a"), rloc(&id, "/b")], rloc(&id, "/")),
        &resolver,
        &Script::errors(ErrorDecision::Skip),
    );

    assert_eq!(transcript.failures.len(), 1);
    assert_eq!(transcript.failures[0].kind, FailureKind::PermissionDenied);
    assert_eq!(outcome.report.skipped, vec![uri(&id, "/a")]);
    assert!(exists(memory.as_ref(), "/a/x"));
    assert!(!exists(memory.as_ref(), "/b"), "the batch went on");
}

/// `remove_tree` that waits for its token: the job's cancel must reach it.
struct WaitsForCancel {
    inner: Arc<MemoryBackend>,
    reached: Mutex<mpsc::Sender<()>>,
}

impl Backend for WaitsForCancel {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        self.inner.list(dir, cancel)
    }
    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        self.inner.stat(path)
    }
    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn std::io::Read + Send>, BackendError> {
        self.inner.open_read(path, from)
    }
    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        self.inner.begin_write(path, size_hint, replace)
    }
    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.inner.remove(path)
    }
    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        let _ = self.reached.lock().unwrap().send(());
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if cancel.is_cancelled() {
                return Err(BackendError::new(BackendErrorKind::Cancelled, Some(path.clone())));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // The token never fired: behave as a delete that ran to the end.
        self.inner.remove_tree(path, &Cancel::new())
    }
    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.inner.copy_within(from, to)
    }
}

#[test]
fn cancelling_a_remote_delete_reaches_remove_tree_through_its_token() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/d/f", b"f");
    let (reached_tx, reached) = mpsc::channel();
    let backend = WaitsForCancel {
        inner: memory.clone(),
        reached: Mutex::new(reached_tx),
    };
    let id = drive("nas");
    let resolver = resolver(vec![(id.clone(), Arc::new(backend) as Arc<dyn Backend>)]);

    let job = start(request(Op::Delete, vec![rloc(&id, "/d")], rloc(&id, "/")), resolver);
    reached.recv_timeout(Duration::from_secs(10)).unwrap();
    job.handle.cancel();
    let (outcome, transcript) = job.finish(&Script::errors(ErrorDecision::Cancel));

    assert!(outcome.cancelled);
    assert!(transcript.failures.is_empty(), "a cancel is not a failure to ask about");
    assert!(exists(memory.as_ref(), "/d/f"), "the token stopped it before it removed anything");
}

#[test]
fn a_mixed_delete_removes_local_items_through_the_old_path_and_remote_ones_for_good() {
    let (memory, id, resolver) = nas();
    put(memory.as_ref(), "/r", b"r");
    let (_root, src, _) = local_world();
    fs::create_dir_all(src.join("d")).unwrap();
    write(&src.join("d/x"), b"x");

    let (outcome, _) = run(
        request(Op::Delete, vec![Location::Local(src.join("d")), rloc(&id, "/r")], rloc(&id, "/")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert!(!src.join("d").exists());
    assert!(!exists(memory.as_ref(), "/r"));
    assert!(outcome.actions.is_empty());
}

// ---------------------------------------------------------------------------
// Rename and new folder.

#[test]
fn renaming_on_a_drive_with_undo_rename_records_an_undoable_rename() {
    let (memory, id, resolver) = nas();
    put(memory.as_ref(), "/d/a.txt", b"a");

    let (landed, action) = rename_at(&rloc(&id, "/d/a.txt"), "b.txt", &resolver).unwrap();

    assert_eq!(landed, rloc(&id, "/d/b.txt"));
    assert_eq!(content(&memory, "/d/b.txt"), Some(b"a".to_vec()));
    assert!(matches!(action, Action::RemoteRenamed { ref from, ref to }
        if *from == rloc(&id, "/d/a.txt") && *to == rloc(&id, "/d/b.txt")));
}

#[test]
fn renaming_never_overwrites_and_rejects_names_that_are_not_one_name() {
    let (memory, id, resolver) = nas();
    put(memory.as_ref(), "/a", b"a");
    put(memory.as_ref(), "/b", b"b");

    let taken = rename_at(&rloc(&id, "/a"), "b", &resolver);
    assert!(matches!(taken, Err(LocationOpError::AlreadyExists(ref p)) if *p == uri(&id, "/b")));
    for bad in ["x/y", "..", "", "."] {
        assert!(
            matches!(rename_at(&rloc(&id, "/a"), bad, &resolver), Err(LocationOpError::InvalidName(_))),
            "{bad:?}"
        );
    }
    assert_eq!(content(&memory, "/a"), Some(b"a".to_vec()));
    assert_eq!(content(&memory, "/b"), Some(b"b".to_vec()));
}

#[test]
fn renaming_on_an_object_store_is_recorded_as_not_undoable() {
    let memory = Arc::new(MemoryBackend::object_store_like());
    put(memory.as_ref(), "/a", b"a");
    let id = drive("s3");
    let resolver = resolver(vec![(id.clone(), memory.clone() as Arc<dyn Backend>)]);

    let (_, action) = rename_at(&rloc(&id, "/a"), "b", &resolver).unwrap();

    assert!(!action.is_undoable());
    assert_eq!(action.label(), "renombrar");
    assert!(action.not_undoable_reason().is_some_and(|r| r.contains("renombrado")));
}

#[test]
fn a_rename_on_a_drive_that_is_gone_says_so() {
    let (_, id, _) = nas();
    let error = rename_at(&rloc(&id, "/a"), "b", &kara_ops::no_drives()).unwrap_err();
    assert!(matches!(error, LocationOpError::DriveUnavailable(ref d) if *d == id));

    let (memory, id, resolver) = nas();
    put(memory.as_ref(), "/a", b"a");
    memory.disconnect().unwrap();
    match rename_at(&rloc(&id, "/a"), "b", &resolver) {
        Err(LocationOpError::Failed { kind, path, .. }) => {
            assert_eq!(kind, FailureKind::MediaGone);
            assert!(path.to_string_lossy().starts_with("kara+mem://nas/"), "{path:?}");
        }
        other => panic!("expected a MediaGone failure, got {other:?}"),
    }
}

#[test]
fn local_rename_and_new_folder_go_through_kara_fs_as_before() {
    let (_root, src, _) = local_world();
    write(&src.join("a"), b"a");
    let none = kara_ops::no_drives();

    let (landed, action) = rename_at(&Location::Local(src.join("a")), "b", &none).unwrap();
    assert_eq!(landed, Location::Local(src.join("b")));
    assert!(matches!(action, Action::Renamed { .. }));

    let (made, action) = create_dir_at(&Location::Local(src.clone()), "Nueva carpeta", &none).unwrap();
    let (again, _) = create_dir_at(&Location::Local(src.clone()), "Nueva carpeta", &none).unwrap();
    assert_eq!(made, Location::Local(src.join("Nueva carpeta")));
    assert_eq!(again, Location::Local(src.join("Nueva carpeta (2)")));
    assert!(matches!(action, Action::DirectoryCreated { .. }));
}

#[test]
fn a_new_remote_folder_takes_the_next_free_name() {
    for memory in [MemoryBackend::posix_like(), MemoryBackend::object_store_like()] {
        let memory = Arc::new(memory);
        let id = drive("nas");
        let resolver = resolver(vec![(id.clone(), memory.clone() as Arc<dyn Backend>)]);

        let (first, action) = create_dir_at(&rloc(&id, "/"), "Nueva carpeta", &resolver).unwrap();
        let (second, _) = create_dir_at(&rloc(&id, "/"), "Nueva carpeta", &resolver).unwrap();

        assert_eq!(first, rloc(&id, "/Nueva carpeta"));
        assert_eq!(second, rloc(&id, "/Nueva carpeta (2)"));
        assert!(exists(memory.as_ref(), "/Nueva carpeta (2)"));
        assert!(matches!(action, Action::RemoteDirectoryCreated { ref path } if *path == first));
    }
}

// ---------------------------------------------------------------------------
// Conflicts against a remote destination.

#[test]
fn replace_on_a_remote_file_writes_with_replace_and_is_not_undoable() {
    let (memory, id, _) = nas();
    put(memory.as_ref(), "/a.txt", b"viejo");
    let spy = Arc::new(Spy::new(memory.clone()));
    let resolver = resolver(vec![(id.clone(), spy.clone() as Arc<dyn Backend>)]);
    let (_root, src, _) = local_world();
    write(&src.join("a.txt"), b"nuevo");

    let (outcome, transcript) = run(
        request(Op::Copy, vec![Location::Local(src.join("a.txt"))], rloc(&id, "/")),
        &resolver,
        &Script::conflicts(Resolution::Replace),
    );

    assert_eq!(transcript.conflicts.len(), 1);
    let prompt = &transcript.conflicts[0];
    assert_eq!(prompt.kind, ConflictKind::FileOverFile);
    assert_eq!(prompt.destination, uri(&id, "/a.txt"), "the prompt names the URI");
    assert_eq!(prompt.source, src.join("a.txt"));
    assert_eq!(spy.called("begin_write_replace"), 1);
    assert_eq!(content(&memory, "/a.txt"), Some(b"nuevo".to_vec()));
    assert_eq!(outcome.resolutions.replaced, 1);
    assert!(matches!(outcome.actions.as_slice(), [Action::NotUndoable { label: "copiar", .. }]));
}

#[test]
fn keep_both_on_a_remote_drive_uses_the_local_naming_rule() {
    let (memory, id, resolver) = nas();
    put(memory.as_ref(), "/in/a.tar.gz", b"viejo");
    put(memory.as_ref(), "/in/a (2).tar.gz", b"otro");
    let (_root, src, _) = local_world();
    write(&src.join("a.tar.gz"), b"nuevo");

    let (outcome, _) = run(
        request(Op::Copy, vec![Location::Local(src.join("a.tar.gz"))], rloc(&id, "/in")),
        &resolver,
        &Script::conflicts(Resolution::KeepBoth),
    );

    assert_eq!(content(&memory, "/in/a.tar.gz"), Some(b"viejo".to_vec()));
    assert_eq!(content(&memory, "/in/a (2).tar.gz"), Some(b"otro".to_vec()));
    assert_eq!(content(&memory, "/in/a (3).tar.gz"), Some(b"nuevo".to_vec()));
    assert_eq!(outcome.resolutions.kept_both, 1);
    assert!(matches!(outcome.actions.as_slice(), [Action::RemoteCopied { created }]
        if *created == rloc(&id, "/in/a (3).tar.gz")));
}

#[test]
fn skip_keeps_both_sides_and_the_source_of_a_move() {
    let a = Arc::new(MemoryBackend::posix_like());
    let b = Arc::new(MemoryBackend::posix_like());
    let resolver = resolver(vec![
        (drive("a"), a.clone() as Arc<dyn Backend>),
        (drive("b"), b.clone() as Arc<dyn Backend>),
    ]);
    put(a.as_ref(), "/f", b"nuevo");
    put(b.as_ref(), "/f", b"viejo");

    let (outcome, _) = run(
        request(Op::Move, vec![rloc(&drive("a"), "/f")], rloc(&drive("b"), "/")),
        &resolver,
        &Script::conflicts(Resolution::Skip),
    );

    assert_eq!(content(&a, "/f"), Some(b"nuevo".to_vec()));
    assert_eq!(content(&b, "/f"), Some(b"viejo".to_vec()));
    assert_eq!(outcome.resolutions.skipped, 1);
    assert!(outcome.actions.is_empty());
}

#[test]
fn apply_to_all_answers_the_rest_of_the_same_kind_without_asking() {
    let (memory, id, resolver) = nas();
    let (_root, src, _) = local_world();
    for name in ["1", "2", "3"] {
        put(memory.as_ref(), &format!("/{name}"), b"viejo");
        write(&src.join(name), b"nuevo");
    }
    let script = Script {
        conflict: Some(Resolution::Replace),
        apply_to_all: true,
        on_error: ErrorDecision::Cancel,
    };

    let (outcome, transcript) = run(
        request(
            Op::Copy,
            ["1", "2", "3"].iter().map(|n| Location::Local(src.join(n))).collect(),
            rloc(&id, "/"),
        ),
        &resolver,
        &script,
    );

    assert_eq!(transcript.conflicts.len(), 1);
    assert_eq!(outcome.resolutions.replaced, 3);
    for name in ["/1", "/2", "/3"] {
        assert_eq!(content(&memory, name), Some(b"nuevo".to_vec()));
    }
}

#[test]
fn merge_into_a_remote_folder_asks_again_for_what_collides_inside() {
    let (memory, id, resolver) = nas();
    put(memory.as_ref(), "/d/old", b"o");
    put(memory.as_ref(), "/d/both", b"remoto");
    let (_root, src, _) = local_world();
    fs::create_dir(src.join("d")).unwrap();
    write(&src.join("d/new"), b"n");
    write(&src.join("d/both"), b"local");

    let job = start(request(Op::Copy, vec![Location::Local(src.join("d"))], rloc(&id, "/")), resolver);
    let mut kinds = Vec::new();
    let outcome = loop {
        match job.next() {
            Event::Conflict(prompt) => {
                kinds.push(prompt.kind);
                let resolution = match prompt.kind {
                    ConflictKind::DirectoryOverDirectory => Resolution::Merge,
                    _ => Resolution::KeepBoth,
                };
                job.handle.answer(Answer::Conflict {
                    resolution,
                    apply_to_all: false,
                });
            }
            Event::Finished(outcome) => break outcome,
            _ => {}
        }
    };

    assert_eq!(kinds, vec![ConflictKind::DirectoryOverDirectory, ConflictKind::FileOverFile]);
    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(content(&memory, "/d/old"), Some(b"o".to_vec()));
    assert_eq!(content(&memory, "/d/new"), Some(b"n".to_vec()));
    assert_eq!(content(&memory, "/d/both"), Some(b"remoto".to_vec()));
    assert_eq!(content(&memory, "/d/both (2)"), Some(b"local".to_vec()));
}

#[test]
fn replace_never_destroys_a_remote_folder_or_puts_a_folder_over_a_file() {
    let (memory, id, resolver) = nas();
    put(memory.as_ref(), "/x/inside", b"i");
    put(memory.as_ref(), "/y", b"file");
    let (_root, src, _) = local_world();
    write(&src.join("x"), b"a file over a folder");
    fs::create_dir(src.join("y")).unwrap();
    write(&src.join("y/z"), b"z");

    let (outcome, transcript) = run(
        request(
            Op::Move,
            vec![Location::Local(src.join("x")), Location::Local(src.join("y"))],
            rloc(&id, "/"),
        ),
        &resolver,
        &Script::conflicts(Resolution::Replace),
    );

    assert_eq!(
        transcript.conflicts.iter().map(|c| c.kind).collect::<Vec<_>>(),
        vec![ConflictKind::FileOverDirectory, ConflictKind::DirectoryOverFile]
    );
    assert_eq!(content(&memory, "/x/inside"), Some(b"i".to_vec()));
    assert_eq!(content(&memory, "/y"), Some(b"file".to_vec()));
    assert!(src.join("x").exists() && src.join("y/z").exists(), "sources stay");
    assert_eq!(outcome.report.failures.len(), 2);
    assert!(outcome.actions.is_empty());
}

#[test]
fn replace_on_a_local_destination_still_goes_through_the_trash() {
    let (memory, id, resolver) = nas();
    put(memory.as_ref(), "/a", b"nuevo");
    let (_root, _, dst) = local_world();
    write(&dst.join("a"), b"viejo");

    let (outcome, _) = run(
        request(Op::Copy, vec![rloc(&id, "/a")], Location::Local(dst.clone())),
        &resolver,
        &Script::conflicts(Resolution::Replace),
    );

    assert_eq!(fs::read(dst.join("a")).unwrap(), b"nuevo");
    match outcome.actions.as_slice() {
        [Action::Trashed { item }, Action::Copied { created }] => {
            assert_eq!(item.original_path, dst.join("a"));
            assert_eq!(fs::read(&item.trashed_path).unwrap(), b"viejo");
            assert_eq!(*created, dst.join("a"));
        }
        other => panic!("expected Trashed then Copied, got {other:?}"),
    }
}

#[test]
fn rename_to_from_the_dialog_lands_under_the_typed_name() {
    let (memory, id, resolver) = nas();
    put(memory.as_ref(), "/a", b"viejo");
    let (_root, src, _) = local_world();
    write(&src.join("a"), b"nuevo");

    run(
        request(Op::Copy, vec![Location::Local(src.join("a"))], rloc(&id, "/")),
        &resolver,
        &Script::conflicts(Resolution::RenameTo("otro".into())),
    );

    assert_eq!(content(&memory, "/a"), Some(b"viejo".to_vec()));
    assert_eq!(content(&memory, "/otro"), Some(b"nuevo".to_vec()));
}

/// A stat that hides one path: the name is taken after the conflict check.
struct Blind {
    inner: Arc<MemoryBackend>,
    hidden: RemotePath,
}

impl Backend for Blind {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        self.inner.list(dir, cancel)
    }
    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        if *path == self.hidden {
            return Err(BackendError::new(BackendErrorKind::NotFound, Some(path.clone())));
        }
        self.inner.stat(path)
    }
    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn std::io::Read + Send>, BackendError> {
        self.inner.open_read(path, from)
    }
    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        self.inner.begin_write(path, size_hint, replace)
    }
    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.inner.remove(path)
    }
    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        self.inner.remove_tree(path, cancel)
    }
    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.inner.copy_within(from, to)
    }
}

#[test]
fn a_name_taken_behind_the_conflict_check_is_never_overwritten() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/a", b"de otro");
    let id = drive("nas");
    let blind = Blind {
        inner: memory.clone(),
        hidden: rpath("/a"),
    };
    let resolver = resolver(vec![(id.clone(), Arc::new(blind) as Arc<dyn Backend>)]);
    let (_root, src, _) = local_world();
    write(&src.join("a"), b"mio");

    let (outcome, transcript) = run(
        request(Op::Move, vec![Location::Local(src.join("a"))], rloc(&id, "/")),
        &resolver,
        &Script::errors(ErrorDecision::Skip),
    );

    assert!(transcript.conflicts.is_empty(), "the check could not see it");
    assert_eq!(content(&memory, "/a"), Some(b"de otro".to_vec()), "never overwritten");
    assert_eq!(fs::read(src.join("a")).unwrap(), b"mio", "the source stays");
    assert_eq!(outcome.report.failures.len(), 1);
}
