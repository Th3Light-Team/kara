//! Milestone 4: undo and redo of remote actions, by capability.

mod remote_common;

use std::fs;
use std::sync::Arc;

use kara_ops::runner::Op;
use kara_ops::{Action, ErrorDecision, UndoError, UndoStack, no_drives, rename_at};
use kara_vfs::memory::MemoryBackend;
use kara_vfs::{Backend, Capabilities, Location};
use remote_common::*;

fn drive_with(memory: &Arc<MemoryBackend>) -> (kara_vfs::DriveId, kara_ops::BackendResolver) {
    let id = drive("nas");
    let resolver = resolver(vec![(id.clone(), memory.clone() as Arc<dyn Backend>)]);
    (id, resolver)
}

#[test]
fn undoing_a_remote_copy_removes_the_copy_and_only_the_copy() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/src/d/f", b"f");
    let (id, resolver) = drive_with(&memory);

    let (outcome, _) = run(
        request(Op::Copy, vec![rloc(&id, "/src/d")], rloc(&id, "/")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );
    let mut stack = UndoStack::new();
    for action in outcome.actions {
        stack.push(action);
    }
    assert!(stack.can_undo());
    assert_eq!(stack.undo_label(), Some("copiar"));

    stack.undo_with(&resolver).unwrap();

    assert!(!exists(memory.as_ref(), "/d"));
    assert_eq!(content(&memory, "/src/d/f"), Some(b"f".to_vec()));
    assert!(stack.can_redo());
    assert!(matches!(stack.redo_with(&resolver), Err(UndoError::NotUndoable(_))));
}

#[test]
fn undo_and_redo_of_a_move_inside_a_drive_with_undo_move() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/a/f", b"f");
    mkdir(memory.as_ref(), "/b");
    let (id, resolver) = drive_with(&memory);

    let (outcome, _) = run(
        request(Op::Move, vec![rloc(&id, "/a/f")], rloc(&id, "/b")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );
    let mut stack = UndoStack::new();
    for action in outcome.actions {
        stack.push(action);
    }
    assert_eq!(stack.undo_label(), Some("mover"));

    stack.undo_with(&resolver).unwrap();
    assert_eq!(content(&memory, "/a/f"), Some(b"f".to_vec()));
    assert!(!exists(memory.as_ref(), "/b/f"));

    stack.redo_with(&resolver).unwrap();
    assert_eq!(content(&memory, "/b/f"), Some(b"f".to_vec()));
    assert!(!exists(memory.as_ref(), "/a/f"));
}

#[test]
fn a_move_on_a_drive_without_undo_move_is_recorded_and_disabled_with_a_reason() {
    let memory = Arc::new(MemoryBackend::object_store_like());
    put(memory.as_ref(), "/a/f", b"f");
    let (id, resolver) = drive_with(&memory);

    let (outcome, _) = run(
        request(Op::Move, vec![rloc(&id, "/a/f")], rloc(&id, "/b")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );
    let mut stack = UndoStack::new();
    stack.push(Action::Renamed {
        from: "/x".into(),
        to: "/y".into(),
    });
    for action in outcome.actions {
        stack.push(action);
    }

    assert!(!stack.can_undo(), "the menu entry is disabled");
    assert_eq!(stack.undo_label(), Some("mover"));
    let reason = stack.undo_disabled_reason().expect("a reason").to_string();
    assert!(reason.contains("deshacer"), "{reason}");
    let depth = stack.depth();
    assert!(matches!(stack.undo_with(&resolver), Err(UndoError::NotUndoable(r)) if r == reason));
    assert_eq!(stack.depth(), depth, "the record stays");
    assert_eq!(content(&memory, "/b/f"), Some(b"f".to_vec()), "nothing was touched");
}

#[test]
fn a_recorded_remote_move_is_not_undone_if_the_drive_no_longer_allows_it() {
    // The record says it was undoable when it happened; the drive serving the
    // id now is a different one that declares no undo_move.
    let memory = Arc::new(MemoryBackend::with_capabilities(Capabilities {
        atomic_rename: true,
        real_directories: true,
        ..Capabilities::default()
    }));
    put(memory.as_ref(), "/b/f", b"f");
    mkdir(memory.as_ref(), "/a");
    let (id, resolver) = drive_with(&memory);
    let mut stack = UndoStack::new();
    stack.push(Action::RemoteMoved {
        from: rloc(&id, "/a/f"),
        to: rloc(&id, "/b/f"),
    });

    assert!(matches!(stack.undo_with(&resolver), Err(UndoError::NotUndoable(_))));
    assert_eq!(content(&memory, "/b/f"), Some(b"f".to_vec()));
    assert_eq!(stack.depth(), 1);
}

#[test]
fn undo_of_a_remote_rename_puts_the_old_name_back() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/a", b"a");
    let (id, resolver) = drive_with(&memory);
    let (_, action) = rename_at(&rloc(&id, "/a"), "b", &resolver).unwrap();
    let mut stack = UndoStack::new();
    stack.push(action);

    stack.undo_with(&resolver).unwrap();
    assert_eq!(content(&memory, "/a"), Some(b"a".to_vec()));
    stack.redo_with(&resolver).unwrap();
    assert_eq!(content(&memory, "/b"), Some(b"a".to_vec()));
}

#[test]
fn an_undo_never_overwrites_what_took_the_old_place() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/a", b"a");
    let (id, resolver) = drive_with(&memory);
    let (_, action) = rename_at(&rloc(&id, "/a"), "b", &resolver).unwrap();
    put(memory.as_ref(), "/a", b"newcomer");
    let mut stack = UndoStack::new();
    stack.push(action);

    assert!(matches!(stack.undo_with(&resolver), Err(UndoError::Remote { .. })));
    assert_eq!(content(&memory, "/a"), Some(b"newcomer".to_vec()));
    assert_eq!(content(&memory, "/b"), Some(b"a".to_vec()));
    assert_eq!(stack.depth(), 1);
}

#[test]
fn undo_of_a_vanished_copy_says_so_by_uri_and_keeps_the_record() {
    let memory = Arc::new(MemoryBackend::posix_like());
    let (id, resolver) = drive_with(&memory);
    let mut stack = UndoStack::new();
    stack.push(Action::RemoteCopied {
        created: rloc(&id, "/gone"),
    });

    match stack.undo_with(&resolver) {
        Err(UndoError::Vanished(path)) => assert_eq!(path, uri(&id, "/gone")),
        other => panic!("expected Vanished, got {other:?}"),
    }
    assert_eq!(stack.depth(), 1);
}

#[test]
fn undo_of_a_remote_new_folder_removes_it_only_while_empty() {
    let memory = Arc::new(MemoryBackend::posix_like());
    let (id, resolver) = drive_with(&memory);
    let (folder, action) = kara_ops::create_dir_at(&rloc(&id, "/"), "N", &resolver).unwrap();
    assert_eq!(folder, rloc(&id, "/N"));
    put(memory.as_ref(), "/N/later", b"l");
    let mut stack = UndoStack::new();
    stack.push(action);

    assert!(stack.undo_with(&resolver).is_err(), "not empty any more");
    assert_eq!(content(&memory, "/N/later"), Some(b"l".to_vec()));

    memory.remove(&rpath("/N/later")).unwrap();
    stack.undo_with(&resolver).unwrap();
    assert!(!exists(memory.as_ref(), "/N"));
}

#[test]
fn a_remote_undo_without_the_drive_fails_and_keeps_the_record() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/c", b"c");
    let (id, _) = drive_with(&memory);
    let mut stack = UndoStack::new();
    stack.push(Action::RemoteCopied {
        created: rloc(&id, "/c"),
    });

    assert!(matches!(stack.undo(), Err(UndoError::DriveUnavailable(_))));
    assert!(matches!(stack.undo_with(&no_drives()), Err(UndoError::DriveUnavailable(_))));
    assert_eq!(content(&memory, "/c"), Some(b"c".to_vec()));
    assert!(stack.can_undo());
}

#[test]
fn local_records_under_remote_ones_still_undo_as_before() {
    let (_root, src, dst) = local_world();
    write(&src.join("a"), b"a");
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/c", b"c");
    let (id, resolver) = drive_with(&memory);

    let (outcome, _) = run(
        request(Op::Move, vec![Location::Local(src.join("a"))], Location::Local(dst.clone())),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );
    let mut stack = UndoStack::new();
    for action in outcome.actions {
        stack.push(action);
    }
    stack.push(Action::RemoteCopied {
        created: rloc(&id, "/c"),
    });

    stack.undo_with(&resolver).unwrap();
    assert!(!exists(memory.as_ref(), "/c"));
    stack.undo_with(&resolver).unwrap();
    assert_eq!(fs::read(src.join("a")).unwrap(), b"a");
    assert!(!dst.join("a").exists());
}

#[test]
fn a_move_across_backends_and_a_replacing_copy_are_never_undoable() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/x", b"old");
    let (id, resolver) = drive_with(&memory);
    let (_root, src, _) = local_world();
    write(&src.join("x"), b"new");
    write(&src.join("y"), b"y");

    let (copied, _) = run(
        request(Op::Copy, vec![Location::Local(src.join("x"))], rloc(&id, "/")),
        &resolver,
        &Script::conflicts(kara_ops::Resolution::Replace),
    );
    let (moved, _) = run(
        request(Op::Move, vec![Location::Local(src.join("y"))], rloc(&id, "/")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    for action in copied.actions.iter().chain(moved.actions.iter()) {
        assert!(!action.is_undoable(), "{action:?}");
        let mut stack = UndoStack::new();
        stack.push(action.clone());
        assert!(!stack.can_undo());
        assert!(stack.undo_with(&resolver).is_err());
    }
    assert_eq!(content(&memory, "/x"), Some(b"new".to_vec()));
    assert_eq!(content(&memory, "/y"), Some(b"y".to_vec()));
}
