//! The vocabulary the view hears from the file watcher, without a real watcher:
//! how raw `notify` events become changes, and the rule that anything unclear
//! becomes a rescan rather than a guess.

use std::path::PathBuf;

use kara_index::{Cancel, Change};
use kara_index::watch::translate;
use notify::event::{CreateKind, DataChange, EventKind, ModifyKind, RemoveKind, RenameMode};
use notify::Event;

fn event(kind: EventKind, paths: &[&str]) -> Event {
    let mut event = Event::new(kind);
    for path in paths {
        event = event.add_path(PathBuf::from(path));
    }
    event
}

#[test]
fn creating_and_removing_name_every_path_involved() {
    assert_eq!(
        translate(&event(EventKind::Create(CreateKind::File), &["/d/a", "/d/b"])),
        [
            Change::Appeared("/d/a".into()),
            Change::Appeared("/d/b".into())
        ]
    );
    assert_eq!(
        translate(&event(EventKind::Remove(RemoveKind::Folder), &["/d/sub"])),
        [Change::Vanished("/d/sub".into())]
    );
}

#[test]
fn a_rename_that_arrives_whole_is_one_change_with_both_ends() {
    let both = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
        &["/d/old", "/d/new"],
    );
    assert_eq!(
        translate(&both),
        [Change::Renamed {
            from: "/d/old".into(),
            to: "/d/new".into()
        }]
    );
}

#[test]
fn half_a_rename_is_a_rescan_because_pairing_it_would_be_a_guess() {
    for mode in [RenameMode::From, RenameMode::To, RenameMode::Any] {
        let half = event(EventKind::Modify(ModifyKind::Name(mode)), &["/d/x"]);
        assert_eq!(translate(&half), [Change::Rescan], "{mode:?}");
    }
    // `Both` with a path count that does not fit is no better.
    let malformed = event(EventKind::Modify(ModifyKind::Name(RenameMode::Both)), &["/d/only-one"]);
    assert_eq!(translate(&malformed), [Change::Rescan]);
}

#[test]
fn a_content_change_touches_the_file() {
    let written = event(EventKind::Modify(ModifyKind::Data(DataChange::Content)), &["/d/a"]);
    assert_eq!(translate(&written), [Change::Touched("/d/a".into())]);
}

#[test]
fn events_that_say_nothing_useful_are_a_rescan() {
    assert_eq!(translate(&event(EventKind::Other, &[])), [Change::Rescan]);
    assert_eq!(translate(&event(EventKind::Any, &["/d"])), [Change::Rescan]);
}

#[test]
fn a_create_with_no_paths_says_nothing_rather_than_inventing_one() {
    assert!(translate(&event(EventKind::Create(CreateKind::Any), &[])).is_empty());
}

#[test]
fn a_cancel_token_is_shared_by_its_clones() {
    let token = Cancel::new();
    let held_by_the_walker = token.clone();
    assert!(!held_by_the_walker.is_cancelled());

    token.cancel();
    assert!(held_by_the_walker.is_cancelled(), "the walker must see the user's cancel");
    token.cancel();
    assert!(token.is_cancelled(), "cancelling twice is harmless");
}
