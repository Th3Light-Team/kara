//! MemoryBackend reads: list, stat, open_read.
//! Edge cases cb_03 (backend part), cb_18, cb_19, cb_20, cb_21.

mod common;

use std::ffi::OsString;
use std::io::{self, Read};

use common::{
    assert_err, assert_other_with, entry, err, is_dir, mkdir, names, p, profiles, read, snap, write,
};
use kara_core::EntryKind;
use kara_vfs::memory::{Fault, FaultEffect, MemoryBackend, Op};
use kara_vfs::{Backend, BackendErrorKind as K, Cancel, RemotePath};

// cb_03 ---------------------------------------------------------------------

#[test]
fn cb_03_nfc_and_nfd_names_coexist_in_one_directory() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        write(&m, "/d/caf\u{e9}", b"nfc");
        write(&m, "/d/cafe\u{301}", b"nfd");
        let listing = m.list(&p("/d"), &Cancel::new()).expect("list");
        assert_eq!(listing.entries.len(), 2, "{label}");
        assert_eq!(read(&m, "/d/caf\u{e9}"), b"nfc", "{label}");
        assert_eq!(read(&m, "/d/cafe\u{301}"), b"nfd", "{label}");
    }
}

// cb_18 ---------------------------------------------------------------------

#[test]
fn cb_18_fresh_root_lists_empty() {
    for (label, m) in profiles() {
        let listing = m
            .list(&RemotePath::root(), &Cancel::new())
            .expect("list root");
        assert!(listing.entries.is_empty(), "{label}");
        assert!(listing.errors.is_empty(), "{label}");
    }
}

#[test]
fn cb_18_entry_fields_for_files_and_directories() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        write(&m, "/d/file.txt", b"12345");
        write(&m, "/d/.hidden", b"");
        mkdir(&m, "/d/sub");

        let listing = m.list(&p("/d"), &Cancel::new()).expect("list");
        assert!(listing.errors.is_empty(), "{label}");
        let mut got: Vec<String> = listing.entries.iter().map(|e| e.display.clone()).collect();
        got.sort();
        assert_eq!(
            got,
            vec![".hidden", "file.txt", "sub"],
            "{label}: no '.'/'..', nothing else"
        );

        for e in &listing.entries {
            assert_eq!(
                e.name,
                OsString::from(&e.display),
                "{label}: display == name"
            );
            assert_eq!(
                e.created, None,
                "{label}: MemoryBackend has no creation time"
            );
            assert_eq!(
                e.accessed, None,
                "{label}: MemoryBackend has no access time"
            );
            assert_eq!(e.type_label, None, "{label}");
            assert_eq!(e.location, None, "{label}: location is step 3");
            assert_eq!(e.extra.keys().count(), 0, "{label}: extra is empty");
            assert!(!e.is_symlink, "{label}");
            assert!(!e.symlink_broken, "{label}");
        }

        let f = entry(&m, "/d", "file.txt");
        assert_eq!(f.kind, EntryKind::File, "{label}");
        assert_eq!(f.size, Some(5), "{label}");
        assert!(!f.is_hidden, "{label}");

        let h = entry(&m, "/d", ".hidden");
        assert!(h.is_hidden, "{label}: dot names are listed, flagged hidden");
        assert_eq!(h.size, Some(0), "{label}");

        let s = entry(&m, "/d", "sub");
        assert!(is_dir(&s), "{label}");
        assert_eq!(s.size, None, "{label}: directory size is None, not zero");
    }
}

#[test]
fn cb_18_empty_directory_lists_ok() {
    for (label, m) in profiles() {
        mkdir(&m, "/empty");
        let listing = m.list(&p("/empty"), &Cancel::new()).expect("list");
        assert!(listing.entries.is_empty(), "{label}");
        assert!(listing.errors.is_empty(), "{label}");
    }
}

#[test]
fn cb_18_listing_a_missing_path_is_not_found() {
    for (label, m) in profiles() {
        let e = err(m.list(&p("/nope"), &Cancel::new()), label);
        assert_err(&e, K::NotFound, "/nope", label);
    }
}

#[test]
fn cb_18_listing_a_file_is_other_not_a_directory() {
    for (label, m) in profiles() {
        write(&m, "/f.txt", b"x");
        let before = snap(&m);
        let e = err(m.list(&p("/f.txt"), &Cancel::new()), label);
        assert_other_with(&e, io::ErrorKind::NotADirectory, label);
        assert_eq!(e.path, Some(p("/f.txt")), "{label}");
        assert_eq!(snap(&m), before, "{label}");
    }
}

#[test]
fn cb_18_symlink_entries() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/t");
    write(&m, "/t/keep.txt", b"keep");
    mkdir(&m, "/d");
    m.create_symlink(&p("/d/to-dir"), "/t")
        .expect("link to dir");
    m.create_symlink(&p("/d/to-file"), "/t/keep.txt")
        .expect("link to file");
    m.create_symlink(&p("/d/broken"), "/nowhere")
        .expect("broken link");

    let to_dir = entry(&m, "/d", "to-dir");
    assert!(to_dir.is_symlink);
    assert!(!to_dir.symlink_broken);
    assert_eq!(
        to_dir.kind,
        EntryKind::Directory,
        "kind is resolved through the target"
    );

    let to_file = entry(&m, "/d", "to-file");
    assert!(to_file.is_symlink);
    assert!(!to_file.symlink_broken);
    assert_eq!(to_file.kind, EntryKind::File);

    let broken = entry(&m, "/d", "broken");
    assert!(broken.is_symlink);
    assert!(broken.symlink_broken);
    assert_eq!(broken.kind, EntryKind::File, "a broken link is a File");
}

// cb_19 ---------------------------------------------------------------------

#[test]
fn cb_19_stat_equals_the_list_entry_on_every_field() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        write(&m, "/d/a.txt", b"aaa");
        write(&m, "/d/.dot", b"");
        write(&m, "/d/b c#%?.bin", &common::pattern(1000));
        mkdir(&m, "/d/sub");
        write(&m, "/d/sub/inner", b"i");
        mkdir(&m, "/d/empty");

        let listing = m.list(&p("/d"), &Cancel::new()).expect("list");
        assert_eq!(listing.entries.len(), 5, "{label}");
        for e in &listing.entries {
            let path = p("/d").join(&e.display).expect("join");
            let st = m
                .stat(&path)
                .unwrap_or_else(|x| panic!("{label}: stat({path}) {x:?}"));
            assert_eq!(&st, e, "{label}: stat({path}) differs from its list entry");
        }
    }
}

#[test]
fn cb_19_stat_equals_list_for_symlinks() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/t");
    m.create_symlink(&p("/l"), "/t").expect("link");
    m.create_symlink(&p("/b"), "/missing").expect("broken");
    for name in ["l", "b"] {
        let e = entry(&m, "/", name);
        assert_eq!(m.stat(&p(&format!("/{name}"))).expect("stat"), e, "{name}");
    }
}

#[test]
fn cb_19_stat_of_root_is_a_directory_named_slash() {
    for (label, m) in profiles() {
        let st = m.stat(&RemotePath::root()).expect("stat root");
        assert_eq!(st.kind, EntryKind::Directory, "{label}");
        assert_eq!(st.name, OsString::from("/"), "{label}");
        assert_eq!(st.display, "/", "{label}");
        assert!(!st.is_hidden, "{label}");
    }
}

#[test]
fn cb_19_stat_under_a_file_is_not_found() {
    for (label, m) in profiles() {
        write(&m, "/f.txt", b"x");
        let e = err(m.stat(&p("/f.txt/x")), label);
        assert_err(&e, K::NotFound, "/f.txt/x", label);
        let e = err(m.stat(&p("/missing")), label);
        assert_err(&e, K::NotFound, "/missing", label);
    }
}

// cb_20 ---------------------------------------------------------------------

#[test]
fn cb_20_precancelled_list_is_cancelled_never_ok() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        write(&m, "/d/a", b"1");
        let token = Cancel::new();
        token.cancel();
        let e = err(m.list(&p("/d"), &token), label);
        assert_err(&e, K::Cancelled, "/d", label);
        let e = err(m.list(&RemotePath::root(), &token), label);
        assert_eq!(e.kind, K::Cancelled, "{label}: even an empty-ish root");
        let e = err(m.list(&p("/d"), &token), label);
        assert_eq!(e.kind, K::Cancelled, "{label}: still cancelled on retry");
    }
}

fn big_dir(entries: usize) -> MemoryBackend {
    let m = MemoryBackend::object_store_like();
    for i in 0..entries {
        write(&m, &format!("/big/f{i:06}"), b"");
    }
    m
}

#[test]
fn cb_20_cancel_mid_list_returns_cancelled_not_a_partial_ok() {
    let m = big_dir(100_000);
    let token = Cancel::new();
    m.inject(Fault {
        op: Op::List,
        path: Some(p("/big")),
        after: 10,
        effect: FaultEffect::Cancel(token.clone()),
        times: Some(1),
    })
    .expect("inject");
    let e = err(m.list(&p("/big"), &token), "cancel after 10 entries");
    assert_err(&e, K::Cancelled, "/big", "cancel after 10 entries");
    assert!(token.is_cancelled(), "the fault cancelled the shared token");
}

#[test]
fn cb_20_cancel_just_before_the_last_entry_is_still_observed() {
    let m = big_dir(2_000);
    let token = Cancel::new();
    m.inject(Fault {
        op: Op::List,
        path: None,
        after: 1_999,
        effect: FaultEffect::Cancel(token.clone()),
        times: Some(1),
    })
    .expect("inject");
    let e = err(m.list(&p("/big"), &token), "cancel before the last entry");
    assert_eq!(e.kind, K::Cancelled);
}

#[test]
fn cb_20_a_list_fault_past_the_end_never_fires() {
    let m = big_dir(5);
    let token = Cancel::new();
    m.inject(Fault {
        op: Op::List,
        path: None,
        after: 10,
        effect: FaultEffect::Cancel(token.clone()),
        times: Some(1),
    })
    .expect("inject");
    let listing = m.list(&p("/big"), &token).expect("5 entries < after = 10");
    assert_eq!(listing.entries.len(), 5);
    assert!(!token.is_cancelled());
    assert_eq!(names(&m, "/big").len(), 5);
}

// cb_21 ---------------------------------------------------------------------

fn read_from(m: &MemoryBackend, path: &str, from: u64) -> Vec<u8> {
    let mut r = m
        .open_read(&p(path), from)
        .unwrap_or_else(|e| panic!("open_read({path}, {from}): {e:?}"));
    let mut out = Vec::new();
    r.read_to_end(&mut out).expect("read_to_end");
    out
}

#[test]
fn cb_21_offsets_within_the_file() {
    for (label, m) in profiles() {
        write(&m, "/ten", b"0123456789");
        assert_eq!(read_from(&m, "/ten", 0), b"0123456789", "{label}");
        assert_eq!(read_from(&m, "/ten", 4), b"456789", "{label}");
        assert_eq!(read_from(&m, "/ten", 9), b"9", "{label}");
        assert_eq!(
            read_from(&m, "/ten", 10),
            b"",
            "{label}: from == len is EOF"
        );
    }
}

#[test]
fn cb_21_offset_past_the_end_fails_at_open() {
    for (label, m) in profiles() {
        write(&m, "/ten", b"0123456789");
        for from in [11, 1 << 40, u64::MAX] {
            let e = err(m.open_read(&p("/ten"), from), label);
            assert_other_with(&e, io::ErrorKind::InvalidInput, label);
            assert_eq!(e.path, Some(p("/ten")), "{label}");
        }
    }
}

#[test]
fn cb_21_reading_a_directory_or_a_missing_path() {
    for (label, m) in profiles() {
        mkdir(&m, "/dir");
        let e = err(m.open_read(&p("/dir"), 0), label);
        assert_other_with(&e, io::ErrorKind::IsADirectory, label);
        assert_eq!(e.path, Some(p("/dir")), "{label}");
        let e = err(m.open_read(&p("/missing"), 0), label);
        assert_err(&e, K::NotFound, "/missing", label);
    }
}

#[test]
fn cb_21_dropping_a_reader_midway_leaves_the_source_intact() {
    for (label, m) in profiles() {
        let content = common::pattern(10_000);
        write(&m, "/f", &content);
        let before = snap(&m);
        {
            let mut r = m.open_read(&p("/f"), 0).expect("open");
            let mut half = vec![0u8; 5_000];
            r.read_exact(&mut half).expect("read half");
            assert_eq!(half[..], content[..5_000], "{label}");
        }
        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(
            m.stat(&p("/f")).expect("stat").size,
            Some(10_000),
            "{label}"
        );
    }
}

#[test]
fn cb_21_reading_counts_bytes_read() {
    for (label, m) in profiles() {
        write(&m, "/ten", b"0123456789");
        let before = common::stats(&m).bytes_read;
        assert_eq!(read_from(&m, "/ten", 4).len(), 6);
        assert_eq!(common::stats(&m).bytes_read - before, 6, "{label}");
    }
}
