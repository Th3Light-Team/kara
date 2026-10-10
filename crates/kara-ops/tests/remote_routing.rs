//! Milestone 1: the Location API, its refusals, and the guarantee that an
//! all-local job behaves exactly like the old path-based one.

mod remote_common;

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kara_ops::runner::{Op, Outcome, Request, spawn_with};
use kara_ops::{
    Action, ErrorDecision, LocationRequest, RequestError, Resolution, display_path, no_drives,
    parse_display_path,
};
use kara_vfs::Location;
use kara_vfs::memory::MemoryBackend;
use remote_common::*;

// ---------------------------------------------------------------------------
// Differential: old API vs Location API on the same local scenarios.

type Build = fn(&Path, &Path) -> (Op, Vec<PathBuf>);

/// One scenario: builds the fixture under (src, dst) and says what to run.
struct Scenario {
    name: &'static str,
    build: Build,
    script: Script,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "copy one file",
            build: |src, _| {
                write(&src.join("a.txt"), b"hola");
                (Op::Copy, vec![src.join("a.txt")])
            },
            script: Script::errors(ErrorDecision::Cancel),
        },
        Scenario {
            name: "copy a tree",
            build: |src, _| {
                fs::create_dir_all(src.join("d/e")).unwrap();
                write(&src.join("d/1"), b"uno");
                write(&src.join("d/e/2"), b"dos");
                (Op::Copy, vec![src.join("d")])
            },
            script: Script::errors(ErrorDecision::Cancel),
        },
        Scenario {
            name: "move a file and a tree",
            build: |src, _| {
                fs::create_dir_all(src.join("d/e")).unwrap();
                write(&src.join("d/e/2"), b"dos");
                write(&src.join("f"), b"f");
                (Op::Move, vec![src.join("d"), src.join("f")])
            },
            script: Script::errors(ErrorDecision::Cancel),
        },
        Scenario {
            name: "replace",
            build: |src, dst| {
                write(&src.join("a"), b"nuevo");
                write(&dst.join("a"), b"viejo");
                (Op::Copy, vec![src.join("a")])
            },
            script: Script::conflicts(Resolution::Replace),
        },
        Scenario {
            name: "keep both",
            build: |src, dst| {
                write(&src.join("a.txt"), b"nuevo");
                write(&dst.join("a.txt"), b"viejo");
                (Op::Copy, vec![src.join("a.txt")])
            },
            script: Script::conflicts(Resolution::KeepBoth),
        },
        Scenario {
            name: "skip",
            build: |src, dst| {
                write(&src.join("a"), b"nuevo");
                write(&dst.join("a"), b"viejo");
                (Op::Move, vec![src.join("a")])
            },
            script: Script::conflicts(Resolution::Skip),
        },
        Scenario {
            name: "merge",
            build: |src, dst| {
                fs::create_dir_all(src.join("d")).unwrap();
                fs::create_dir_all(dst.join("d")).unwrap();
                write(&src.join("d/new"), b"n");
                write(&dst.join("d/old"), b"o");
                (Op::Copy, vec![src.join("d")])
            },
            script: Script::conflicts(Resolution::Merge),
        },
        Scenario {
            name: "into itself",
            build: |src, _| {
                fs::create_dir_all(src.join("d")).unwrap();
                (Op::Copy, vec![src.join("d")])
            },
            script: Script::errors(ErrorDecision::Cancel),
        },
        Scenario {
            name: "missing source is skipped",
            build: |src, _| {
                write(&src.join("ok"), b"ok");
                (Op::Copy, vec![src.join("gone"), src.join("ok")])
            },
            script: Script::errors(ErrorDecision::Skip),
        },
        Scenario {
            name: "permanent delete",
            build: |src, _| {
                fs::create_dir_all(src.join("d/e")).unwrap();
                write(&src.join("d/e/x"), b"x");
                (Op::Delete, vec![src.join("d")])
            },
            script: Script::errors(ErrorDecision::Cancel),
        },
    ]
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map_or_else(|_| path.display().to_string(), |rest| rest.display().to_string())
}

/// The outcome without what legitimately differs between two runs: the
/// tempdir and the trash file names.
fn summary(outcome: &Outcome, root: &Path) -> String {
    let actions: Vec<String> = outcome
        .actions
        .iter()
        .map(|action| match action {
            Action::Copied { created } => format!("Copied({})", relative(created, root)),
            Action::Moved { from, to } => {
                format!("Moved({} -> {})", relative(from, root), relative(to, root))
            }
            Action::Trashed { item } => format!("Trashed({})", relative(&item.original_path, root)),
            other => format!("{other:?}"),
        })
        .collect();
    let failures: Vec<String> = outcome
        .report
        .failures
        .iter()
        .map(|f| format!("{}|{:?}|{}", relative(&f.path, root), f.kind, f.reason))
        .collect();
    let skipped: Vec<String> = outcome
        .report
        .skipped
        .iter()
        .map(|p| relative(p, root))
        .collect();
    format!(
        "op={:?} actions={actions:?} succeeded={} skipped={skipped:?} failures={failures:?} \
         cancelled={} {} resolutions={:?}",
        outcome.op,
        outcome.report.succeeded,
        outcome.report.cancelled,
        outcome.cancelled,
        outcome.resolutions,
    )
}

/// Every path under `root` with its content, for comparing trees.
fn tree(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                out.push(format!("{}/", relative(&path, root)));
                walk(&path, root, out);
            } else {
                out.push(format!(
                    "{}={:?}",
                    relative(&path, root),
                    fs::read(&path).unwrap()
                ));
            }
        }
    }
    walk(root, root, &mut out);
    out
}

#[test]
fn every_local_scenario_gives_the_same_outcome_through_both_apis() {
    for scenario in scenarios() {
        let (root_old, src_old, dst_old) = local_world();
        let (op, sources) = (scenario.build)(&src_old, &dst_old);
        let dest = if scenario.name == "into itself" {
            sources[0].clone()
        } else {
            dst_old.clone()
        };
        let (old, _) = start_old(Request {
            op,
            sources,
            dest_dir: dest,
        })
        .finish(&scenario.script);

        let (root_new, src_new, dst_new) = local_world();
        let (op, sources) = (scenario.build)(&src_new, &dst_new);
        let dest = if scenario.name == "into itself" {
            sources[0].clone()
        } else {
            dst_new.clone()
        };
        let converted = LocationRequest::from(Request {
            op,
            sources,
            dest_dir: dest,
        });
        let (new, _) = run(converted, &no_drives(), &scenario.script);

        assert_eq!(
            summary(&old, root_old.path()),
            summary(&new, root_new.path()),
            "scenario «{}»: the outcome differs",
            scenario.name
        );
        assert_eq!(
            tree(root_old.path()),
            tree(root_new.path()),
            "scenario «{}»: the resulting tree differs",
            scenario.name
        );
    }
}

// ---------------------------------------------------------------------------
// The local fast path is still the one taken.

#[test]
fn a_local_folder_moved_on_one_volume_is_renamed_not_copied() {
    let (_root, src, dst) = local_world();
    fs::create_dir_all(src.join("d/e")).unwrap();
    write(&src.join("d/e/x"), b"x");
    let inode = fs::metadata(src.join("d")).unwrap().ino();

    let (outcome, _) = run(
        request(Op::Move, vec![Location::Local(src.join("d"))], Location::Local(dst.clone())),
        &no_drives(),
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    let moved = fs::metadata(dst.join("d")).unwrap().ino();
    assert_eq!(moved, inode, "rename(2) keeps the inode; a copy would not");
    assert!(matches!(outcome.actions.as_slice(), [Action::Moved { .. }]));
}

#[test]
fn a_local_item_in_a_mixed_job_still_goes_through_rename() {
    let (_root, src, dst) = local_world();
    fs::create_dir_all(src.join("d")).unwrap();
    write(&src.join("d/x"), b"x");
    let inode = fs::metadata(src.join("d")).unwrap().ino();
    let id = drive("nas");
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/r.txt", b"remote");
    let resolver = resolver(vec![(id.clone(), memory.clone())]);

    let (outcome, _) = run(
        request(
            Op::Move,
            vec![Location::Local(src.join("d")), rloc(&id, "/r.txt")],
            Location::Local(dst.clone()),
        ),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(fs::metadata(dst.join("d")).unwrap().ino(), inode);
    assert_eq!(fs::read(dst.join("r.txt")).unwrap(), b"remote");
    assert!(!exists(memory.as_ref(), "/r.txt"), "the remote source was moved");
}

// ---------------------------------------------------------------------------
// Refusals: nothing is touched.

#[test]
fn an_unconfirmed_delete_is_refused_before_anything_is_touched() {
    let id = drive("nas");
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/a", b"a");
    let resolver = resolver(vec![(id.clone(), memory.clone())]);
    let (_root, src, _) = local_world();
    write(&src.join("l"), b"l");

    for source in [rloc(&id, "/a"), Location::Local(src.join("l"))] {
        let refused = spawn_with(
            LocationRequest {
                op: Op::Delete,
                sources: vec![source],
                dest_dir: rloc(&id, "/"),
                confirmed_permanent: false,
            },
            Arc::clone(&resolver),
            |_| panic!("a refused request must not emit anything"),
        );
        assert!(matches!(refused, Err(RequestError::PermanentDeleteNotConfirmed)));
    }
    assert!(exists(memory.as_ref(), "/a"));
    assert!(src.join("l").exists());
}

#[test]
fn a_drive_that_does_not_resolve_is_refused_up_front() {
    let id = drive("gone");
    let (_root, src, _) = local_world();
    write(&src.join("a"), b"a");
    let refused = spawn_with(
        request(Op::Copy, vec![Location::Local(src.join("a"))], rloc(&id, "/")),
        no_drives(),
        |_| panic!("a refused request must not emit anything"),
    );
    assert_eq!(refused.err(), Some(RequestError::DriveUnavailable(id)));
}

#[test]
fn converting_an_old_request_keeps_everything_local() {
    let converted = LocationRequest::from(Request {
        op: Op::Copy,
        sources: vec![PathBuf::from("/a"), PathBuf::from("/b")],
        dest_dir: PathBuf::from("/c"),
    });
    assert!(converted.is_all_local());
    let back = converted.to_local().expect("all local");
    assert_eq!(back.sources, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    assert_eq!(back.dest_dir, PathBuf::from("/c"));
    assert!(!converted.confirmed_permanent, "only a delete is confirmed by conversion");
}

#[test]
fn a_remote_item_is_named_by_its_uri_and_parses_back() {
    let id = drive("nas");
    let location = rloc(&id, "/docs/a b.txt");
    let shown = display_path(&location);
    assert_eq!(shown, PathBuf::from("kara+mem://nas/docs/a%20b.txt"));
    assert_eq!(parse_display_path(&shown), location);
    let local = Location::Local(PathBuf::from("/home/x/a b.txt"));
    assert_eq!(display_path(&local), PathBuf::from("/home/x/a b.txt"));
    assert_eq!(parse_display_path(&display_path(&local)), local);
}
