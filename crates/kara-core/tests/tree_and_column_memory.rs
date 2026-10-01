//! The navigation tree's bookkeeping (what is open, what has been read, what to
//! reveal) and the bounded per-folder memory of column layouts.

use std::path::{Path, PathBuf};

use kara_core::columns::{ColumnLayout, ColumnMemory};
use kara_core::{Branch, Expandable, RowKind, Section, SectionId, Tree};

fn tree() -> Tree {
    Tree::new(vec![
        Section {
            id: SectionId::QuickAccess,
            roots: vec![Branch::at("/home/ana")],
        },
        Section {
            id: SectionId::ThisComputer,
            roots: vec![Branch::at("/"), Branch::at("/mnt/usb")],
        },
    ])
}

fn folder_paths(tree: &Tree) -> Vec<PathBuf> {
    tree.rows()
        .into_iter()
        .filter_map(|row| match row.kind {
            RowKind::Folder { path, .. } => Some(path),
            RowKind::Section(_) => None,
        })
        .collect()
}

// ---- open / read bookkeeping ----------------------------------------------------

#[test]
fn opening_an_unread_branch_asks_for_it_to_be_read_and_an_unread_one_is_unknown() {
    let mut tree = tree();
    let home = Path::new("/home/ana");
    assert_eq!(tree.expandable(home), Expandable::Unknown);
    assert!(tree.expand(home), "nothing read yet: the caller must read it");
    assert!(tree.is_expanded(home));
}

#[test]
fn a_branch_that_was_read_is_not_asked_for_again() {
    let mut tree = tree();
    let home = Path::new("/home/ana");
    tree.set_children(home, vec![Branch::at("/home/ana/docs")]);
    assert_eq!(tree.expandable(home), Expandable::Yes);
    assert!(!tree.expand(home));
}

#[test]
fn a_folder_with_no_subfolders_loses_its_arrow() {
    let mut tree = tree();
    tree.set_children(Path::new("/mnt/usb"), Vec::new());
    assert_eq!(tree.expandable(Path::new("/mnt/usb")), Expandable::No);
}

#[test]
fn toggling_opens_then_closes_and_only_the_opening_may_need_a_read() {
    let mut tree = tree();
    let home = Path::new("/home/ana");
    assert!(tree.toggle(home), "first toggle opens an unread branch");
    assert!(!tree.toggle(home), "second toggle closes it: nothing to read");
    assert!(!tree.is_expanded(home));
}

#[test]
fn closing_keeps_what_was_read_and_forgetting_keeps_it_open_but_unread() {
    let mut tree = tree();
    let home = Path::new("/home/ana");
    tree.set_children(home, vec![Branch::at("/home/ana/docs")]);
    tree.expand(home);

    tree.collapse(home);
    assert_eq!(tree.expandable(home), Expandable::Yes, "closing does not unread");

    tree.expand(home);
    tree.forget(home);
    assert!(tree.is_expanded(home), "a refresh must not fold what the user opened");
    assert_eq!(tree.expandable(home), Expandable::Unknown);
    assert!(tree.expand(home), "and it is read again next time");
}

#[test]
fn replacing_the_sections_keeps_what_was_open_and_read() {
    let mut tree = tree();
    let home = Path::new("/home/ana");
    tree.set_children(home, vec![Branch::at("/home/ana/docs")]);
    tree.expand(home);

    // A volume was mounted: the roots change, the user's state does not.
    tree.set_sections(vec![Section {
        id: SectionId::QuickAccess,
        roots: vec![Branch::at("/home/ana"), Branch::at("/media/new")],
    }]);

    assert_eq!(tree.sections().len(), 1);
    assert!(tree.is_expanded(home));
    assert_eq!(tree.expandable(home), Expandable::Yes);
}

// ---- flattening -------------------------------------------------------------------

#[test]
fn only_open_branches_show_their_children_and_depth_grows_with_nesting() {
    let mut tree = tree();
    let home = Path::new("/home/ana");
    tree.set_children(home, vec![Branch::at("/home/ana/docs")]);
    assert_eq!(folder_paths(&tree).len(), 3, "home, / and /mnt/usb");

    tree.expand(home);
    let rows = tree.rows();
    let child = rows
        .iter()
        .find(|row| matches!(&row.kind, RowKind::Folder { path, .. } if path == Path::new("/home/ana/docs")))
        .expect("the child is listed once its parent is open");
    let parent = rows
        .iter()
        .find(|row| matches!(&row.kind, RowKind::Folder { path, .. } if path == home))
        .expect("parent");
    assert_eq!(child.depth, parent.depth + 1);
}

#[test]
fn a_cycle_in_the_children_cannot_make_flattening_run_forever() {
    // A symlink pointing at its own ancestor makes a folder its own child.
    let mut tree = Tree::new(vec![Section {
        id: SectionId::ThisComputer,
        roots: vec![Branch::at("/loop")],
    }]);
    let looped = Path::new("/loop");
    tree.set_children(looped, vec![Branch::at("/loop")]);
    tree.expand(looped);

    let rows = tree.rows();
    assert!(rows.len() < 200, "stopped at the depth cap, got {} rows", rows.len());
}

// ---- revealing the current folder -----------------------------------------------------

#[test]
fn the_reveal_starts_at_the_deepest_root_that_contains_the_folder() {
    let tree = tree();
    let chain = tree.path_to_reveal(Path::new("/home/ana/docs/work/q3"));
    // From home, not from `/`: that is where the user lives.
    assert_eq!(
        chain,
        [
            PathBuf::from("/home/ana"),
            PathBuf::from("/home/ana/docs"),
            PathBuf::from("/home/ana/docs/work"),
        ]
    );
}

#[test]
fn the_folder_itself_and_a_root_need_nothing_opened() {
    let tree = tree();
    assert!(tree.path_to_reveal(Path::new("/home/ana")).is_empty());
    assert_eq!(
        tree.path_to_reveal(Path::new("/home/ana/docs")),
        [PathBuf::from("/home/ana")],
        "the target is shown, never opened"
    );
}

#[test]
fn a_folder_under_no_root_reveals_nothing() {
    let tree = Tree::new(vec![Section {
        id: SectionId::QuickAccess,
        roots: vec![Branch::at("/home/ana")],
    }]);
    assert!(tree.path_to_reveal(Path::new("/srv/data")).is_empty());
}

#[test]
fn the_filesystem_root_is_used_when_no_nearer_root_applies() {
    let tree = tree();
    assert_eq!(
        tree.path_to_reveal(Path::new("/etc/ssh")),
        [PathBuf::from("/"), PathBuf::from("/etc")]
    );
}

// ---- column layout memory ------------------------------------------------------------------

fn narrower() -> ColumnLayout {
    let mut layout = ColumnLayout::default();
    let first = layout.columns()[0].id.clone();
    layout.set_width(&first, 321);
    layout
}

#[test]
fn an_unconfigured_folder_uses_the_global_layout() {
    let memory = ColumnMemory::default();
    assert!(memory.is_empty());
    assert_eq!(
        memory.layout_for(Path::new("/a")).columns().len(),
        memory.fallback().columns().len()
    );
}

#[test]
fn a_remembered_layout_wins_for_its_folder_and_only_there() {
    let mut memory = ColumnMemory::default();
    memory.remember(Path::new("/a"), narrower());

    let first = memory.fallback().columns()[0].id.clone();
    let width = |layout: ColumnLayout| {
        layout
            .columns()
            .iter()
            .find(|c| c.id == first)
            .map(|c| c.width)
            .expect("column")
    };
    assert_eq!(width(memory.layout_for(Path::new("/a"))), 321);
    assert_ne!(width(memory.layout_for(Path::new("/b"))), 321);
}

#[test]
fn changing_the_global_layout_moves_only_folders_that_never_chose() {
    let mut memory = ColumnMemory::default();
    memory.remember(Path::new("/chose"), narrower());

    let mut other = ColumnLayout::default();
    let extra = other.available_to_add().into_iter().next().expect("something to add");
    other.add(extra);
    let wider = other.columns().len();
    memory.set_fallback(other);

    assert_eq!(memory.layout_for(Path::new("/never")).columns().len(), wider);
    assert_ne!(memory.layout_for(Path::new("/chose")).columns().len(), wider);
}

#[test]
fn the_column_memory_is_bounded_and_forgets_the_oldest() {
    let mut memory = ColumnMemory::new(ColumnLayout::default(), 2);
    memory.remember(Path::new("/a"), narrower());
    memory.remember(Path::new("/b"), narrower());
    memory.remember(Path::new("/c"), narrower());
    assert_eq!(memory.len(), 2);

    memory.forget(Path::new("/b"));
    assert_eq!(memory.len(), 1);
    memory.forget(Path::new("/never-there"));
    assert_eq!(memory.len(), 1, "forgetting the unknown is a no-op");
}

#[test]
fn a_column_memory_with_no_room_stores_nothing() {
    let mut memory = ColumnMemory::new(ColumnLayout::default(), 0);
    memory.remember(Path::new("/a"), narrower());
    assert!(memory.is_empty());
}
