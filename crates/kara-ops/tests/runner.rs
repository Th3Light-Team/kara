//! The paste engine, driven the way the UI drives it: events out, answers in.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use kara_ops::runner::{Answer, Event, Handle, Op, Outcome, Request, spawn};
use kara_ops::{Action, ConflictKind, ErrorDecision, Resolution};
use tempfile::TempDir;

struct Run {
    handle: Handle,
    events: mpsc::Receiver<Event>,
}

fn start(op: Op, sources: &[&Path], dest: &Path) -> Run {
    let (tx, events) = mpsc::channel();
    let handle = spawn(
        Request {
            op,
            sources: sources.iter().map(|p| p.to_path_buf()).collect(),
            dest_dir: dest.to_path_buf(),
        },
        move |event| {
            let _ = tx.send(event);
        },
    );
    Run { handle, events }
}

impl Run {
    fn next(&self) -> Event {
        self.events
            .recv_timeout(Duration::from_secs(10))
            .expect("the worker went quiet")
    }

    /// Answers every conflict with `resolution`, every failure with
    /// `on_error`, and returns the outcome.
    fn finish(&self, resolution: Option<Resolution>, on_error: ErrorDecision) -> (Outcome, usize) {
        let mut conflicts = 0;
        loop {
            match self.next() {
                Event::Conflict(_) => {
                    conflicts += 1;
                    self.handle.answer(Answer::Conflict {
                        resolution: resolution.clone().expect("unexpected conflict"),
                        apply_to_all: false,
                    });
                }
                Event::Failure(_) => self.handle.answer(Answer::Error(on_error)),
                Event::Finished(outcome) => return (*outcome, conflicts),
                _ => {}
            }
        }
    }
}

fn write(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
}

/// Points the trash at a private folder. Replacing sends the old file to the
/// trash, and on this machine /tmp is its own volume: without this the tests
/// would leave `/tmp/.Trash-<uid>` behind, which another suite refuses to run
/// next to.
fn isolate_trash() {
    use std::sync::OnceLock;
    static HOME: OnceLock<TempDir> = OnceLock::new();
    HOME.get_or_init(|| {
        let dir = TempDir::new().unwrap();
        // Set once, before any worker thread reads it: every test calls
        // `world()` first.
        unsafe { std::env::set_var("XDG_DATA_HOME", dir.path()) };
        dir
    });
}

fn world() -> (TempDir, PathBuf, PathBuf) {
    isolate_trash();
    let root = TempDir::new().unwrap();
    let src = root.path().join("src");
    let dst = root.path().join("dst");
    fs::create_dir(&src).unwrap();
    fs::create_dir(&dst).unwrap();
    (root, src, dst)
}

#[test]
fn it_says_it_is_calculating_before_anything_else() {
    let (_root, src, dst) = world();
    write(&src.join("a.txt"), "a");
    let run = start(Op::Copy, &[&src.join("a.txt")], &dst);
    assert!(matches!(run.next(), Event::Calculating));
    match run.next() {
        Event::Started {
            total_bytes,
            total_items,
        } => assert_eq!((total_bytes, total_items), (1, 1)),
        other => panic!("expected Started, got {other:?}"),
    }
}

#[test]
fn copy_leaves_the_source_and_registers_one_undoable_action() {
    let (_root, src, dst) = world();
    write(&src.join("a.txt"), "hello");
    let (outcome, _) = start(Op::Copy, &[&src.join("a.txt")], &dst).finish(None, ErrorDecision::Cancel);

    assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
    assert!(src.join("a.txt").exists());
    assert!(outcome.report.is_clean());
    assert!(matches!(outcome.actions.as_slice(), [Action::Copied { .. }]));
}

#[test]
fn copying_a_tree_registers_the_folder_once_not_every_file() {
    let (_root, src, dst) = world();
    let tree = src.join("tree");
    fs::create_dir_all(tree.join("inner")).unwrap();
    write(&tree.join("one"), "1");
    write(&tree.join("inner/two"), "2");
    let (outcome, _) = start(Op::Copy, &[&tree], &dst).finish(None, ErrorDecision::Cancel);

    assert_eq!(fs::read_to_string(dst.join("tree/inner/two")).unwrap(), "2");
    assert_eq!(outcome.actions.len(), 1);
}

#[test]
fn move_on_one_volume_renames_and_is_undoable() {
    let (_root, src, dst) = world();
    write(&src.join("a.txt"), "x");
    let (outcome, _) = start(Op::Move, &[&src.join("a.txt")], &dst).finish(None, ErrorDecision::Cancel);

    assert!(!src.join("a.txt").exists());
    assert!(dst.join("a.txt").exists());
    assert!(matches!(outcome.actions.as_slice(), [Action::Moved { .. }]));
}

#[test]
fn a_conflict_asks_and_skip_leaves_the_destination_alone() {
    let (_root, src, dst) = world();
    write(&src.join("a.txt"), "new");
    write(&dst.join("a.txt"), "old");
    let (outcome, asked) =
        start(Op::Copy, &[&src.join("a.txt")], &dst).finish(Some(Resolution::Skip), ErrorDecision::Cancel);

    assert_eq!(asked, 1);
    assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "old");
    assert!(outcome.actions.is_empty());
    assert_eq!(outcome.resolutions.skipped, 1);
}

#[test]
fn keep_both_writes_beside_it_and_never_overwrites() {
    let (_root, src, dst) = world();
    write(&src.join("a.txt"), "new");
    write(&dst.join("a.txt"), "old");
    start(Op::Copy, &[&src.join("a.txt")], &dst).finish(Some(Resolution::KeepBoth), ErrorDecision::Cancel);

    assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "old");
    let copies: Vec<_> = fs::read_dir(&dst).unwrap().collect();
    assert_eq!(copies.len(), 2);
}

#[test]
fn replace_trashes_the_old_one_so_it_can_be_taken_back() {
    let (_root, src, dst) = world();
    write(&src.join("a.txt"), "new");
    write(&dst.join("a.txt"), "old");
    let (outcome, _) =
        start(Op::Copy, &[&src.join("a.txt")], &dst).finish(Some(Resolution::Replace), ErrorDecision::Cancel);

    assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "new");
    // The trashing first, then the copy: undo walks them back in reverse.
    assert!(matches!(
        outcome.actions.as_slice(),
        [Action::Trashed { .. }, Action::Copied { .. }]
    ));
}

#[test]
fn apply_to_all_asks_once_per_kind() {
    let (_root, src, dst) = world();
    for name in ["a", "b", "c"] {
        write(&src.join(name), "new");
        write(&dst.join(name), "old");
    }
    let sources = [src.join("a"), src.join("b"), src.join("c")];
    let refs: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
    let run = start(Op::Copy, &refs, &dst);

    let mut asked = 0;
    let outcome = loop {
        match run.next() {
            Event::Conflict(prompt) => {
                asked += 1;
                assert_eq!(prompt.kind, ConflictKind::FileOverFile);
                run.handle.answer(Answer::Conflict {
                    resolution: Resolution::Skip,
                    apply_to_all: true,
                });
            }
            Event::Finished(outcome) => break outcome,
            _ => {}
        }
    };
    assert_eq!(asked, 1);
    assert_eq!(outcome.resolutions.skipped, 3);
}

#[test]
fn merge_combines_two_folders_and_keeps_what_was_only_there() {
    let (_root, src, dst) = world();
    fs::create_dir(src.join("d")).unwrap();
    fs::create_dir(dst.join("d")).unwrap();
    write(&src.join("d/new"), "n");
    write(&dst.join("d/kept"), "k");
    start(Op::Copy, &[&src.join("d")], &dst).finish(Some(Resolution::Merge), ErrorDecision::Cancel);

    assert!(dst.join("d/new").exists());
    assert_eq!(fs::read_to_string(dst.join("d/kept")).unwrap(), "k");
}

#[test]
fn copying_into_the_same_folder_makes_a_numbered_copy_without_asking() {
    let (_root, src, _dst) = world();
    write(&src.join("a.txt"), "x");
    let (outcome, asked) = start(Op::Copy, &[&src.join("a.txt")], &src).finish(None, ErrorDecision::Cancel);

    assert_eq!(asked, 0);
    assert_eq!(fs::read_dir(&src).unwrap().count(), 2);
    assert!(outcome.report.is_clean());
}

#[test]
fn moving_into_the_same_folder_does_nothing() {
    let (_root, src, _dst) = world();
    write(&src.join("a.txt"), "x");
    let (outcome, _) = start(Op::Move, &[&src.join("a.txt")], &src).finish(None, ErrorDecision::Cancel);

    assert_eq!(fs::read_dir(&src).unwrap().count(), 1);
    assert!(outcome.actions.is_empty());
}

#[test]
fn a_folder_cannot_go_inside_itself_and_the_batch_carries_on() {
    let (_root, src, dst) = world();
    let tree = src.join("tree");
    fs::create_dir(&tree).unwrap();
    write(&src.join("ok.txt"), "x");
    let (outcome, _) =
        start(Op::Copy, &[&tree, &src.join("ok.txt")], &tree).finish(None, ErrorDecision::Cancel);

    assert_eq!(outcome.report.failures.len(), 1);
    assert!(tree.join("ok.txt").exists());
    drop(dst);
}

#[test]
fn a_failure_asks_and_skip_does_not_abort_the_batch() {
    let (_root, src, dst) = world();
    write(&src.join("good.txt"), "g");
    let missing = src.join("missing.txt");
    let run = start(Op::Copy, &[&missing, &src.join("good.txt")], &dst);

    let (outcome, _) = run.finish(None, ErrorDecision::Skip);
    assert!(dst.join("good.txt").exists());
    assert_eq!(outcome.report.failures.len(), 1);
    assert_eq!(outcome.report.failures[0].path, missing);
    assert!(!outcome.cancelled);
}

#[test]
fn cancel_on_a_failure_stops_and_says_so() {
    let (_root, src, dst) = world();
    write(&src.join("later.txt"), "l");
    let (outcome, _) = start(Op::Copy, &[&src.join("missing.txt"), &src.join("later.txt")], &dst)
        .finish(None, ErrorDecision::Cancel);

    assert!(outcome.cancelled);
    assert!(!dst.join("later.txt").exists());
}

#[test]
fn cancelling_while_a_question_is_open_ends_the_job() {
    let (_root, src, dst) = world();
    write(&src.join("a.txt"), "new");
    write(&dst.join("a.txt"), "old");
    let run = start(Op::Copy, &[&src.join("a.txt")], &dst);
    loop {
        if matches!(run.next(), Event::Conflict(_)) {
            break;
        }
    }
    run.handle.cancel();
    let outcome = loop {
        if let Event::Finished(outcome) = run.next() {
            break outcome;
        }
    };
    assert!(outcome.cancelled);
    assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "old");
}

#[test]
fn progress_reaches_the_total() {
    let (_root, src, dst) = world();
    fs::write(src.join("big"), vec![7u8; 1_500_000]).unwrap();
    let run = start(Op::Copy, &[&src.join("big")], &dst);
    let mut last = (0, 0);
    loop {
        match run.next() {
            Event::Progress {
                bytes_done,
                items_done,
                ..
            } => last = (bytes_done, items_done),
            Event::Finished(_) => break,
            _ => {}
        }
    }
    assert_eq!(last, (1_500_000, 1));
}

#[test]
fn a_symlink_is_recreated_as_a_link() {
    let (_root, src, dst) = world();
    write(&src.join("target"), "t");
    std::os::unix::fs::symlink("target", src.join("link")).unwrap();
    start(Op::Copy, &[&src.join("link")], &dst).finish(None, ErrorDecision::Cancel);

    assert!(fs::symlink_metadata(dst.join("link")).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_link(dst.join("link")).unwrap(), Path::new("target"));
}

#[test]
fn skip_then_keep_both_then_undo_walks_back_through_everything() {
    use kara_ops::UndoStack;

    let (_root, src, dst) = world();
    write(&src.join("a.txt"), "new");
    write(&dst.join("a.txt"), "old");
    let mut undo = UndoStack::new();

    // First paste: asked, skipped, nothing registered.
    let (skipped, asked) =
        start(Op::Copy, &[&src.join("a.txt")], &dst).finish(Some(Resolution::Skip), ErrorDecision::Cancel);
    assert_eq!(asked, 1);
    assert!(skipped.actions.is_empty());

    // Second paste of the same thing, a new job: keep both.
    let (kept, _) =
        start(Op::Copy, &[&src.join("a.txt")], &dst).finish(Some(Resolution::KeepBoth), ErrorDecision::Cancel);
    assert_eq!(fs::read_dir(&dst).unwrap().count(), 2);
    for action in kept.actions {
        undo.push(action);
    }

    // Undo takes the copy back and the original is untouched.
    undo.undo().expect("the copy can be undone");
    assert_eq!(fs::read_dir(&dst).unwrap().count(), 1);
    assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "old");
}

fn delete(sources: &[&Path]) -> Run {
    start(Op::Delete, sources, Path::new("/"))
}

#[test]
fn delete_removes_files_and_trees_for_good_and_registers_nothing_to_undo() {
    let (_root, src, _dst) = world();
    write(&src.join("a.txt"), "x");
    let tree = src.join("tree");
    fs::create_dir_all(tree.join("inner")).unwrap();
    write(&tree.join("inner/b"), "y");

    let (outcome, _) = delete(&[&src.join("a.txt"), &tree]).finish(None, ErrorDecision::Cancel);

    assert!(!src.join("a.txt").exists());
    assert!(!tree.exists());
    assert!(outcome.report.is_clean());
    // Irreversible by nature: nothing for Ctrl+Z to find.
    assert!(outcome.actions.is_empty());
}

#[test]
fn delete_never_follows_a_symlink_into_its_target() {
    let (_root, src, _dst) = world();
    let precious = src.join("precious");
    fs::create_dir(&precious).unwrap();
    write(&precious.join("keep.txt"), "k");
    std::os::unix::fs::symlink(&precious, src.join("link")).unwrap();

    delete(&[&src.join("link")]).finish(None, ErrorDecision::Cancel);

    assert!(fs::symlink_metadata(src.join("link")).is_err());
    assert!(precious.join("keep.txt").exists(), "the target must survive");
}

#[test]
fn delete_of_something_already_gone_asks_and_skip_carries_on() {
    let (_root, src, _dst) = world();
    write(&src.join("later.txt"), "l");
    let (outcome, _) =
        delete(&[&src.join("gone.txt"), &src.join("later.txt")]).finish(None, ErrorDecision::Skip);

    assert!(!src.join("later.txt").exists());
    assert_eq!(outcome.report.failures.len(), 1);
}

#[test]
fn delete_cancelled_on_a_failure_leaves_the_rest_alone() {
    let (_root, src, _dst) = world();
    write(&src.join("later.txt"), "l");
    let (outcome, _) =
        delete(&[&src.join("gone.txt"), &src.join("later.txt")]).finish(None, ErrorDecision::Cancel);

    assert!(outcome.cancelled);
    assert!(src.join("later.txt").exists());
}

#[test]
fn a_cancel_while_copying_ends_quietly_without_a_failure_prompt() {
    let (_root, src, dst) = world();
    fs::write(src.join("big"), vec![1u8; 40_000_000]).unwrap();
    let run = start(Op::Copy, &[&src.join("big")], &dst);
    loop {
        if let Event::Progress { bytes_done, .. } = run.next()
            && bytes_done > 0
        {
            break;
        }
    }
    run.handle.cancel();
    let outcome = loop {
        match run.next() {
            Event::Failure(_) => panic!("a cancel must not turn into a failure question"),
            Event::Finished(outcome) => break outcome,
            _ => {}
        }
    };
    // The copy may have beaten the cancel on a fast disk; what must never
    // happen is a cancel that leaves half a file behind.
    if outcome.cancelled {
        assert!(!dst.join("big").exists(), "a half-written copy is removed");
    } else {
        assert_eq!(fs::metadata(dst.join("big")).unwrap().len(), 40_000_000);
    }
}
