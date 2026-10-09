//! MemoryBackend mutations: create_dir, rename, remove, remove_tree,
//! copy_within, symlinks, implicit directories, error paths.
//! Edge cases cb_12, cb_32 to cb_41.

mod common;

use std::collections::BTreeMap;
use std::io;

use common::{
    assert_err, assert_other_with, err, file, is_dir, mkdir, names, p, profiles, read, snap, stats,
    try_read, write,
};
use kara_vfs::memory::{Fault, FaultEffect, MemoryBackend, MemoryNode, MemorySnapshot, Op};
use kara_vfs::{Backend, BackendErrorKind as K, Cancel, RemotePath};

/// Builds `root` with 5 files and a `sub` dir holding 5 more (10 files).
/// Returns relative path -> content.
fn ten_file_tree(m: &MemoryBackend, root: &str) -> BTreeMap<String, Vec<u8>> {
    let real_dirs = m.capabilities().real_directories;
    if real_dirs {
        mkdir(m, root);
        mkdir(m, &format!("{root}/sub"));
    }
    let mut files = BTreeMap::new();
    for i in 0..5 {
        let rel = format!("f{i}");
        let content = format!("file {i} of the tree").into_bytes();
        write(m, &format!("{root}/{rel}"), &content);
        files.insert(rel, content);
        let rel = format!("sub/g{i}");
        let content = common::pattern(100 + i);
        write(m, &format!("{root}/{rel}"), &content);
        files.insert(rel, content);
    }
    files
}

fn file_nodes(s: &MemorySnapshot) -> Vec<(RemotePath, Vec<u8>)> {
    s.nodes
        .iter()
        .filter_map(|(path, node)| match node {
            MemoryNode::File { content } => Some((path.clone(), content.clone())),
            _ => None,
        })
        .collect()
}

// cb_12 ---------------------------------------------------------------------

#[test]
fn cb_12_rename_errors_name_the_right_argument() {
    let m = MemoryBackend::posix_like();
    write(&m, "/a", b"a");
    write(&m, "/b", b"b");
    let e = err(m.rename(&p("/missing"), &p("/c")), "from missing");
    assert_err(
        &e,
        K::NotFound,
        "/missing",
        "rename: from missing names from",
    );
    let e = err(m.rename(&p("/a"), &p("/b")), "to exists");
    assert_err(&e, K::AlreadyExists, "/b", "rename: to exists names to");
    let e = err(m.rename(&p("/a"), &p("/nodir/c")), "to parent missing");
    assert_err(
        &e,
        K::NotFound,
        "/nodir/c",
        "rename: missing parent of to names to",
    );
}

#[test]
fn cb_12_copy_within_errors_follow_the_rename_rule() {
    let m = MemoryBackend::object_store_like();
    write(&m, "/a", b"a");
    write(&m, "/b", b"b");
    let e = err(m.copy_within(&p("/missing"), &p("/c")), "from missing");
    assert_err(
        &e,
        K::NotFound,
        "/missing",
        "copy_within: from missing names from",
    );
    let e = err(m.copy_within(&p("/a"), &p("/b")), "to exists");
    assert_err(
        &e,
        K::AlreadyExists,
        "/b",
        "copy_within: to exists names to",
    );
}

#[test]
fn cb_12_remove_tree_failure_names_the_descendant() {
    for (label, m) in profiles() {
        ten_file_tree(&m, "/t");
        m.inject(Fault {
            op: Op::RemoveTree,
            path: None,
            after: 2,
            effect: FaultEffect::Fail(K::Other),
            times: Some(1),
        })
        .expect("inject");
        let e = err(m.remove_tree(&p("/t"), &Cancel::new()), label);
        assert_eq!(e.kind, K::Other, "{label}");
        let path = e.path.clone().expect("the error names a path");
        assert!(
            path.starts_with(&p("/t")) && path != p("/t"),
            "{label}: {path} is a descendant"
        );
    }
}

// cb_32 ---------------------------------------------------------------------

#[test]
fn cb_32_create_dir_creates_an_empty_directory() {
    for (label, m) in profiles() {
        mkdir(&m, "/new");
        assert!(is_dir(&m.stat(&p("/new")).expect("stat")), "{label}");
        assert!(names(&m, "/").contains("new"), "{label}");
        let listing = m.list(&p("/new"), &Cancel::new()).expect("list");
        assert!(
            listing.entries.is_empty(),
            "{label}: no entry, not even one named \"\""
        );
        assert!(listing.errors.is_empty(), "{label}");
    }
}

#[test]
fn cb_32_create_dir_on_anything_existing_is_already_exists() {
    for (label, m) in profiles() {
        mkdir(&m, "/dir");
        write(&m, "/dir/child", b"c");
        write(&m, "/file", b"f");
        let before = snap(&m);
        for path in ["/dir", "/file"] {
            let e = err(m.create_dir(&p(path)), path);
            assert_err(&e, K::AlreadyExists, path, label);
        }
        assert_eq!(snap(&m), before, "{label}: nothing changed, nothing merged");
    }
}

#[test]
fn cb_32_object_profile_create_dir_is_a_marker() {
    let m = MemoryBackend::object_store_like();
    mkdir(&m, "/dir");
    assert_eq!(snap(&m).nodes.get(&p("/dir")), Some(&MemoryNode::DirMarker));
    assert!(
        m.list(&p("/dir"), &Cancel::new())
            .expect("list")
            .entries
            .is_empty()
    );
    assert!(is_dir(&m.stat(&p("/dir")).expect("stat")));
}

#[test]
fn cb_32_object_profile_create_dir_on_an_implicit_prefix_is_already_exists() {
    let m = MemoryBackend::object_store_like();
    write(&m, "/pre/f", b"x");
    let before = snap(&m);
    let e = err(m.create_dir(&p("/pre")), "implicit prefix");
    assert_err(&e, K::AlreadyExists, "/pre", "implicit prefix");
    assert_eq!(snap(&m), before);
}

#[test]
fn cb_32_posix_create_dir_is_a_real_dir_node() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/dir");
    assert_eq!(snap(&m).nodes.get(&p("/dir")), Some(&MemoryNode::Dir));
}

// cb_33 ---------------------------------------------------------------------

#[test]
fn cb_33_rename_file_moves_the_content() {
    for (label, m) in profiles() {
        write(&m, "/old", b"payload");
        m.rename(&p("/old"), &p("/new")).expect("rename");
        assert_eq!(err(m.stat(&p("/old")), label).kind, K::NotFound, "{label}");
        assert_eq!(read(&m, "/new"), b"payload", "{label}");
    }
}

#[test]
fn cb_33_rename_dir_moves_the_whole_subtree() {
    for (label, m) in profiles() {
        let files = ten_file_tree(&m, "/src");
        m.rename(&p("/src"), &p("/dst")).expect("rename");
        for (rel, content) in &files {
            assert_eq!(&read(&m, &format!("/dst/{rel}")), content, "{label} {rel}");
            let old = p(&format!("/src/{rel}"));
            assert_eq!(err(m.stat(&old), label).kind, K::NotFound, "{label} {rel}");
        }
        assert_eq!(err(m.stat(&p("/src")), label).kind, K::NotFound, "{label}");
    }
}

#[test]
fn cb_33_rename_never_overwrites_anything() {
    for (label, m) in profiles() {
        write(&m, "/a", b"A");
        write(&m, "/b", b"B");
        mkdir(&m, "/d");
        write(&m, "/d/x", b"x");
        let before = snap(&m);
        for (from, to) in [("/a", "/b"), ("/a", "/d"), ("/d", "/b")] {
            let e = err(m.rename(&p(from), &p(to)), label);
            assert_err(&e, K::AlreadyExists, to, &format!("{label} {from}->{to}"));
        }
        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(read(&m, "/a"), b"A", "{label}");
        assert_eq!(read(&m, "/b"), b"B", "{label}");
    }
}

#[test]
fn cb_33_object_profile_rename_onto_marker_or_prefix_is_already_exists() {
    let m = MemoryBackend::object_store_like();
    write(&m, "/a", b"A");
    mkdir(&m, "/marker");
    write(&m, "/prefix/f", b"f");
    let before = snap(&m);
    for to in ["/marker", "/prefix"] {
        let e = err(m.rename(&p("/a"), &p(to)), to);
        assert_err(&e, K::AlreadyExists, to, to);
    }
    assert_eq!(snap(&m), before);
}

// cb_34 ---------------------------------------------------------------------

#[test]
fn cb_34_rename_onto_itself_is_a_noop() {
    for (label, m) in profiles() {
        write(&m, "/x", b"same");
        mkdir(&m, "/d");
        write(&m, "/d/y", b"y");
        let before = snap(&m);
        m.rename(&p("/x"), &p("/x")).expect("file onto itself");
        m.rename(&p("/d"), &p("/d")).expect("dir onto itself");
        assert_eq!(snap(&m), before, "{label}");
    }
}

// cb_35 ---------------------------------------------------------------------

#[test]
fn cb_35_rename_into_own_subtree_is_refused() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        mkdir(&m, "/d/sub");
        write(&m, "/d/sub/f", b"f");
        let before = snap(&m);
        for to in ["/d/sub/d2", "/d/d2"] {
            let e = err(m.rename(&p("/d"), &p(to)), label);
            assert_other_with(
                &e,
                io::ErrorKind::InvalidInput,
                &format!("{label} /d->{to}"),
            );
        }
        assert_eq!(snap(&m), before, "{label}");
    }
}

#[test]
fn cb_35_rename_to_a_sibling_sharing_a_prefix_is_fine() {
    for (label, m) in profiles() {
        mkdir(&m, "/a");
        write(&m, "/a/f", b"f");
        m.rename(&p("/a"), &p("/ab")).expect("/ab is not inside /a");
        assert_eq!(read(&m, "/ab/f"), b"f", "{label}");
    }
}

#[test]
fn cb_35_rename_from_or_to_the_root_is_refused() {
    for (label, m) in profiles() {
        mkdir(&m, "/x");
        write(&m, "/x/f", b"f");
        let before = snap(&m);
        let e = err(m.rename(&RemotePath::root(), &p("/y")), label);
        assert_other_with(&e, io::ErrorKind::InvalidInput, label);
        let e = err(m.rename(&p("/x"), &RemotePath::root()), label);
        assert_other_with(&e, io::ErrorKind::InvalidInput, label);
        assert_eq!(snap(&m), before, "{label}");
    }
}

#[test]
fn cb_35_rename_of_a_missing_source_is_not_found() {
    for (label, m) in profiles() {
        let before = snap(&m);
        let e = err(m.rename(&p("/missing"), &p("/y")), label);
        assert_err(&e, K::NotFound, "/missing", label);
        assert_eq!(snap(&m), before, "{label}");
    }
}

// cb_36 ---------------------------------------------------------------------

#[test]
fn cb_36_non_atomic_rename_never_loses_an_object() {
    for k in 0..10u64 {
        let m = MemoryBackend::object_store_like();
        let files = ten_file_tree(&m, "/src");
        m.inject(Fault {
            op: Op::Rename,
            path: None,
            after: k,
            effect: FaultEffect::Fail(K::Unavailable),
            times: Some(1),
        })
        .expect("inject");
        let e = err(m.rename(&p("/src"), &p("/dst")), "faulty rename");
        assert_eq!(e.kind, K::Unavailable, "k={k}");

        for (rel, content) in &files {
            let under_from = try_read(&m, &p(&format!("/src/{rel}"))).ok();
            let under_to = try_read(&m, &p(&format!("/dst/{rel}"))).ok();
            assert!(
                under_from.as_ref() == Some(content) || under_to.as_ref() == Some(content),
                "k={k}: {rel} lost (from: {under_from:?}, to: {under_to:?})"
            );
        }
        for (path, content) in file_nodes(&snap(&m)) {
            let rel = path
                .as_str()
                .strip_prefix("/src/")
                .or_else(|| path.as_str().strip_prefix("/dst/"))
                .unwrap_or_else(|| panic!("k={k}: unexpected object {path}"));
            assert_eq!(
                files.get(rel),
                Some(&content),
                "k={k}: {path} has foreign content"
            );
        }
    }
}

#[test]
fn cb_36_atomic_rename_fault_leaves_the_tree_under_from() {
    let m = MemoryBackend::posix_like();
    ten_file_tree(&m, "/src");
    let before = snap(&m);
    m.inject(Fault {
        op: Op::Rename,
        path: None,
        after: 0,
        effect: FaultEffect::Fail(K::Unavailable),
        times: Some(1),
    })
    .expect("inject");
    let e = err(m.rename(&p("/src"), &p("/dst")), "faulty rename");
    assert_eq!(e.kind, K::Unavailable);
    assert_eq!(snap(&m), before);
}

// cb_37 ---------------------------------------------------------------------

#[test]
fn cb_37_remove_file_and_empty_dir() {
    for (label, m) in profiles() {
        write(&m, "/f", b"f");
        mkdir(&m, "/empty");
        m.remove(&p("/f")).expect("remove file");
        m.remove(&p("/empty")).expect("remove empty dir");
        assert_eq!(err(m.stat(&p("/f")), label).kind, K::NotFound, "{label}");
        assert_eq!(
            err(m.stat(&p("/empty")), label).kind,
            K::NotFound,
            "{label}"
        );
        assert!(snap(&m).nodes.is_empty(), "{label}");
    }
}

#[test]
fn cb_37_remove_non_empty_dir_is_refused() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        write(&m, "/d/f", b"f");
        let before = snap(&m);
        let e = err(m.remove(&p("/d")), label);
        assert_other_with(&e, io::ErrorKind::DirectoryNotEmpty, label);
        assert_eq!(e.path, Some(p("/d")), "{label}");
        assert_eq!(snap(&m), before, "{label}");
    }
}

#[test]
fn cb_37_object_profile_implicit_prefix_with_children_is_not_empty() {
    let m = MemoryBackend::object_store_like();
    write(&m, "/pre/f", b"f");
    let before = snap(&m);
    let e = err(m.remove(&p("/pre")), "implicit prefix");
    assert_other_with(&e, io::ErrorKind::DirectoryNotEmpty, "implicit prefix");
    assert_eq!(snap(&m), before);
}

#[test]
fn cb_37_remove_missing_and_root() {
    for (label, m) in profiles() {
        write(&m, "/f", b"f");
        let before = snap(&m);
        let e = err(m.remove(&p("/missing")), label);
        assert_err(&e, K::NotFound, "/missing", label);
        let e = err(m.remove(&RemotePath::root()), label);
        assert_other_with(&e, io::ErrorKind::InvalidInput, label);
        assert_eq!(snap(&m), before, "{label}");
    }
}

#[test]
fn cb_37_object_profile_remove_of_a_marker_dir_removes_the_marker() {
    let m = MemoryBackend::object_store_like();
    mkdir(&m, "/m");
    m.remove(&p("/m")).expect("remove marker dir");
    assert!(snap(&m).nodes.is_empty());
    assert_eq!(err(m.stat(&p("/m")), "stat").kind, K::NotFound);
}

#[test]
fn cb_37_remove_of_a_symlink_removes_only_the_link() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/t");
    write(&m, "/t/keep.txt", b"keep");
    m.create_symlink(&p("/l"), "/t").expect("link");
    m.remove(&p("/l"))
        .expect("a link to a non-empty dir is removable");
    assert_eq!(read(&m, "/t/keep.txt"), b"keep");
    assert_eq!(snap(&m).nodes.get(&p("/l")), None);
    assert_eq!(snap(&m).nodes.get(&p("/t")), Some(&MemoryNode::Dir));
}

// cb_38 ---------------------------------------------------------------------

fn flat_tree(m: &MemoryBackend) -> Vec<String> {
    if m.capabilities().real_directories {
        mkdir(m, "/t");
    }
    (0..10)
        .map(|i| {
            let path = format!("/t/f{i}");
            write(m, &path, format!("{i}").as_bytes());
            path
        })
        .collect()
}

fn remaining_files(m: &MemoryBackend, paths: &[String]) -> Vec<String> {
    let s = snap(m);
    paths
        .iter()
        .filter(|x| s.nodes.contains_key(&p(x)))
        .cloned()
        .collect()
}

#[test]
fn cb_38_remove_tree_deletes_a_three_level_tree() {
    for (label, m) in profiles() {
        ten_file_tree(&m, "/t");
        if m.capabilities().real_directories {
            mkdir(&m, "/t/sub/deeper");
        }
        write(&m, "/t/sub/deeper/z", b"z");
        write(&m, "/outside", b"stays");
        m.remove_tree(&p("/t"), &Cancel::new())
            .expect("remove_tree");
        assert_eq!(err(m.stat(&p("/t")), label).kind, K::NotFound, "{label}");
        let s = snap(&m);
        assert_eq!(
            s.nodes.len(),
            1,
            "{label}: only /outside remains: {:?}",
            s.nodes
        );
        assert_eq!(
            s.nodes.get(&p("/outside")),
            Some(&file(b"stays")),
            "{label}"
        );
    }
}

#[test]
fn cb_38_remove_tree_of_a_file_and_of_a_missing_path() {
    for (label, m) in profiles() {
        write(&m, "/f", b"f");
        m.remove_tree(&p("/f"), &Cancel::new())
            .expect("remove_tree on a file");
        assert!(snap(&m).nodes.is_empty(), "{label}");
        let e = err(m.remove_tree(&p("/missing"), &Cancel::new()), label);
        assert_err(&e, K::NotFound, "/missing", label);
    }
}

#[test]
fn cb_38_remove_tree_refuses_the_root() {
    for (label, m) in profiles() {
        flat_tree(&m);
        let before = snap(&m);
        let e = err(m.remove_tree(&RemotePath::root(), &Cancel::new()), label);
        assert_other_with(&e, io::ErrorKind::InvalidInput, label);
        assert_eq!(snap(&m), before, "{label}");
    }
}

#[test]
fn cb_38_remove_tree_does_not_touch_a_sibling_sharing_a_prefix() {
    for (label, m) in profiles() {
        flat_tree(&m);
        if m.capabilities().real_directories {
            mkdir(&m, "/tt");
        }
        write(&m, "/tt/keep", b"keep");
        write(&m, "/t.txt", b"keep too");
        m.remove_tree(&p("/t"), &Cancel::new())
            .expect("remove_tree");
        assert_eq!(read(&m, "/tt/keep"), b"keep", "{label}");
        assert_eq!(read(&m, "/t.txt"), b"keep too", "{label}");
    }
}

#[test]
fn cb_38_precancelled_remove_tree_removes_nothing() {
    for (label, m) in profiles() {
        flat_tree(&m);
        let before = snap(&m);
        let token = Cancel::new();
        token.cancel();
        let e = err(m.remove_tree(&p("/t"), &token), label);
        assert_err(&e, K::Cancelled, "/t", label);
        assert_eq!(snap(&m), before, "{label}");
    }
}

#[test]
fn cb_38_cancel_midway_stops_after_exactly_the_processed_objects() {
    for (label, m) in profiles() {
        let paths = flat_tree(&m);
        let token = Cancel::new();
        m.inject(Fault {
            op: Op::RemoveTree,
            path: None,
            after: 3,
            effect: FaultEffect::Cancel(token.clone()),
            times: Some(1),
        })
        .expect("inject");
        let e = err(m.remove_tree(&p("/t"), &token), label);
        assert_err(&e, K::Cancelled, "/t", label);
        assert_eq!(
            remaining_files(&m, &paths).len(),
            7,
            "{label}: exactly 3 files gone"
        );
    }
}

#[test]
fn cb_38_failure_midway_names_the_entry_and_attempts_nothing_else() {
    for (label, m) in profiles() {
        let paths = flat_tree(&m);
        m.inject(Fault {
            op: Op::RemoveTree,
            path: None,
            after: 3,
            effect: FaultEffect::Fail(K::PermissionDenied),
            times: Some(1),
        })
        .expect("inject");
        let e = err(m.remove_tree(&p("/t"), &Cancel::new()), label);
        assert_eq!(e.kind, K::PermissionDenied, "{label}");
        let remaining = remaining_files(&m, &paths);
        assert_eq!(
            remaining.len(),
            7,
            "{label}: exactly 3 files gone, the rest not attempted"
        );
        let failed = e.path.clone().expect("path").as_str().to_owned();
        assert!(
            remaining.contains(&failed),
            "{label}: {failed} is the entry being processed"
        );
    }
}

#[test]
fn cb_38_remove_tree_deletes_children_before_parents() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/t");
    mkdir(&m, "/t/d");
    write(&m, "/t/d/f", b"leaf");
    m.inject(Fault {
        op: Op::RemoveTree,
        path: None,
        after: 1,
        effect: FaultEffect::Fail(K::Other),
        times: Some(1),
    })
    .expect("inject");
    let e = err(m.remove_tree(&p("/t"), &Cancel::new()), "after one object");
    assert_eq!(e.kind, K::Other);
    let s = snap(&m);
    assert_eq!(s.nodes.get(&p("/t/d/f")), None, "the leaf went first");
    assert_eq!(s.nodes.get(&p("/t/d")), Some(&MemoryNode::Dir));
    assert_eq!(s.nodes.get(&p("/t")), Some(&MemoryNode::Dir));
    assert_eq!(
        e.path,
        Some(p("/t/d")),
        "the entry being processed when the fault fired"
    );
}

// cb_39 ---------------------------------------------------------------------

#[test]
fn cb_39_remove_tree_never_follows_a_symlink() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/t");
    write(&m, "/t/keep.txt", b"keep");
    mkdir(&m, "/d");
    m.create_symlink(&p("/d/l"), "/t").expect("link");
    m.remove_tree(&p("/d"), &Cancel::new())
        .expect("remove_tree");
    assert_eq!(read(&m, "/t/keep.txt"), b"keep");
    let s = snap(&m);
    assert_eq!(
        s.nodes.len(),
        2,
        "only /t and /t/keep.txt remain: {:?}",
        s.nodes
    );

    m.create_symlink(&p("/l2"), "/t").expect("link");
    m.remove_tree(&p("/l2"), &Cancel::new())
        .expect("remove_tree on a link");
    assert_eq!(
        read(&m, "/t/keep.txt"),
        b"keep",
        "removing a link never removes its target"
    );
    assert!(is_dir(&m.stat(&p("/t")).expect("stat")));
}

#[test]
fn cb_39_broken_link_lists_and_removes() {
    let m = MemoryBackend::posix_like();
    m.create_symlink(&p("/b"), "/nowhere").expect("link");
    let e = common::entry(&m, "/", "b");
    assert!(e.is_symlink && e.symlink_broken);
    m.remove(&p("/b")).expect("remove broken link");
    assert_eq!(err(m.stat(&p("/b")), "stat").kind, K::NotFound);
}

#[test]
fn cb_39_rename_moves_the_link_and_open_read_follows_it() {
    let m = MemoryBackend::posix_like();
    write(&m, "/target.txt", b"through the link");
    m.create_symlink(&p("/l"), "/target.txt").expect("link");
    assert_eq!(read(&m, "/l"), b"through the link");
    m.rename(&p("/l"), &p("/l2")).expect("rename link");
    let s = snap(&m);
    assert_eq!(
        s.nodes.get(&p("/l2")),
        Some(&MemoryNode::Symlink {
            target: "/target.txt".into()
        })
    );
    assert_eq!(s.nodes.get(&p("/l")), None);
    assert_eq!(
        s.nodes.get(&p("/target.txt")),
        Some(&file(b"through the link"))
    );
}

#[test]
fn cb_39_object_profile_has_no_symlinks() {
    let m = MemoryBackend::object_store_like();
    write(&m, "/t", b"t");
    let before = snap(&m);
    let e = err(m.create_symlink(&p("/l"), "/t"), "create_symlink");
    assert_eq!(e.kind, K::Unsupported);
    assert_eq!(snap(&m), before);
}

// cb_40 ---------------------------------------------------------------------

#[test]
fn cb_40_unmarked_prefix_vanishes_with_its_last_object() {
    let m = MemoryBackend::object_store_like();
    write(&m, "/p/f", b"f");
    assert!(is_dir(
        &m.stat(&p("/p"))
            .expect("prefix exists while it has an object")
    ));
    m.remove(&p("/p/f")).expect("remove");
    assert_eq!(err(m.stat(&p("/p")), "stat").kind, K::NotFound);
    let e = err(m.list(&p("/p"), &Cancel::new()), "list");
    assert_err(&e, K::NotFound, "/p", "list of a vanished prefix");
    assert!(!names(&m, "/").contains("p"));
}

#[test]
fn cb_40_nested_prefixes_vanish_together() {
    let m = MemoryBackend::object_store_like();
    write(&m, "/p/q/f", b"f");
    write(&m, "/p/q/g", b"g");
    m.remove(&p("/p/q/f")).expect("remove f");
    assert!(
        names(&m, "/p").contains("q"),
        "one object left keeps /p/q alive"
    );
    m.remove(&p("/p/q/g")).expect("remove g");
    assert_eq!(err(m.stat(&p("/p/q")), "stat").kind, K::NotFound);
    assert_eq!(err(m.stat(&p("/p")), "stat").kind, K::NotFound);
    let root = m
        .list(&RemotePath::root(), &Cancel::new())
        .expect("root always lists");
    assert!(root.entries.is_empty());
}

#[test]
fn cb_40_marked_prefix_survives_its_last_object() {
    let m = MemoryBackend::object_store_like();
    mkdir(&m, "/m");
    write(&m, "/m/f", b"f");
    m.remove(&p("/m/f")).expect("remove");
    assert!(is_dir(&m.stat(&p("/m")).expect("a marker keeps the dir")));
    assert!(
        m.list(&p("/m"), &Cancel::new())
            .expect("list")
            .entries
            .is_empty()
    );
}

// cb_41 ---------------------------------------------------------------------

#[test]
fn cb_41_copy_within_without_capability_is_unsupported() {
    let m = MemoryBackend::posix_like();
    write(&m, "/a", b"abc");
    let before = snap(&m);
    let e = err(m.copy_within(&p("/a"), &p("/b")), "copy_within");
    assert_eq!(e.kind, K::Unsupported);
    assert_eq!(snap(&m), before);
}

#[test]
fn cb_41_copy_within_moves_no_bytes_through_the_client() {
    let m = MemoryBackend::object_store_like();
    let content = common::pattern(3 << 20);
    write(&m, "/big", &content);
    let before = stats(&m);
    m.copy_within(&p("/big"), &p("/copy")).expect("copy_within");
    let after = stats(&m);
    assert_eq!(
        after.bytes_read, before.bytes_read,
        "copy_within reads nothing client-side"
    );
    assert_eq!(
        after.bytes_written, before.bytes_written,
        "copy_within writes nothing client-side"
    );
    let s = snap(&m);
    assert!(
        s.nodes.get(&p("/copy"))
            == Some(&MemoryNode::File {
                content: content.clone()
            })
    );
    assert!(s.nodes.get(&p("/big")) == Some(&MemoryNode::File { content }));
}

#[test]
fn cb_41_copy_within_never_replaces_and_refuses_directories() {
    let m = MemoryBackend::object_store_like();
    write(&m, "/a", b"A");
    write(&m, "/b", b"B");
    write(&m, "/dir/f", b"f");
    let before = snap(&m);
    let e = err(m.copy_within(&p("/a"), &p("/b")), "onto existing");
    assert_err(&e, K::AlreadyExists, "/b", "onto existing");
    let e = err(m.copy_within(&p("/dir"), &p("/dir2")), "dir source");
    assert_eq!(e.kind, K::Unsupported);
    let e = err(m.copy_within(&p("/missing"), &p("/c")), "missing source");
    assert_err(&e, K::NotFound, "/missing", "missing source");
    assert_eq!(snap(&m), before);
}
