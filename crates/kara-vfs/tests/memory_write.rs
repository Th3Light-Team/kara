//! MemoryBackend write sessions: commit, replace, visibility, abort, poisoning,
//! disconnect. Edge cases cb_22 to cb_31.

mod common;

use std::collections::BTreeSet;
use std::io::{Read, Write};

use common::{
    assert_err, err, file, mkdir, names, object_caps, p, profiles, read, snap, stats, write,
    write_with,
};
use kara_vfs::memory::{Fault, FaultEffect, MemoryBackend, MemoryNode, Op};
use kara_vfs::{Backend, BackendError, BackendErrorKind as K, Cancel, RemotePath, TRANSFER_CHUNK};

// cb_22 ---------------------------------------------------------------------

#[test]
fn cb_22_write_then_read_back_exact_bytes() {
    for (label, m) in profiles() {
        write(&m, "/small", b"hello, world");
        assert_eq!(read(&m, "/small"), b"hello, world", "{label}");
        assert_eq!(
            snap(&m).nodes.get(&p("/small")),
            Some(&file(b"hello, world")),
            "{label}"
        );
    }
}

#[test]
fn cb_22_empty_session_commits_an_empty_file() {
    for (label, m) in profiles() {
        let s = m.begin_write(&p("/empty"), Some(0), false).expect("begin");
        s.finish().expect("finish with no write");
        assert_eq!(m.stat(&p("/empty")).expect("stat").size, Some(0), "{label}");
        assert_eq!(read(&m, "/empty"), b"", "{label}");
        assert_eq!(
            snap(&m).nodes.get(&p("/empty")),
            Some(&file(b"")),
            "{label}"
        );
    }
}

#[test]
fn cb_22_multi_chunk_file_in_uneven_pieces() {
    for (label, m) in profiles() {
        let content = common::pattern(2 * TRANSFER_CHUNK + 17);
        let mut s = m.begin_write(&p("/big"), None, false).expect("begin");
        let mut offset: usize = 0;
        for piece in [1, 4095, TRANSFER_CHUNK + 3, 7, usize::MAX] {
            let end = offset.saturating_add(piece).min(content.len());
            s.write_all(&content[offset..end]).expect("write");
            offset = end;
        }
        s.finish().expect("finish");
        assert_eq!(
            m.stat(&p("/big")).expect("stat").size,
            Some(content.len() as u64),
            "{label}"
        );
        assert!(read(&m, "/big") == content, "{label}: content differs");
    }
}

#[test]
fn cb_22_size_hint_is_never_enforced() {
    for (label, m) in profiles() {
        let mut s = m.begin_write(&p("/short"), Some(10), false).expect("begin");
        s.write_all(b"abc").expect("write");
        s.finish().expect("fewer bytes than the hint");
        assert_eq!(read(&m, "/short"), b"abc", "{label}");

        let mut s = m.begin_write(&p("/long"), Some(0), false).expect("begin");
        s.write_all(b"12345").expect("write");
        s.finish().expect("more bytes than the hint");
        assert_eq!(read(&m, "/long"), b"12345", "{label}");
    }
}

#[test]
fn cb_22_bytes_written_counts_exactly_the_session_bytes() {
    for (label, m) in profiles() {
        let before = stats(&m).bytes_written;
        write(&m, "/a", &common::pattern(12_345));
        assert_eq!(stats(&m).bytes_written - before, 12_345, "{label}");
        let s = m.begin_write(&p("/b"), None, false).expect("begin");
        s.finish().expect("finish");
        assert_eq!(
            stats(&m).bytes_written - before,
            12_345,
            "{label}: empty file adds 0"
        );
    }
}

#[test]
fn cb_22_flush_never_commits() {
    for (label, m) in profiles() {
        let mut s = m.begin_write(&p("/f"), None, false).expect("begin");
        s.write_all(b"data").expect("write");
        s.flush().expect("flush");
        assert_eq!(
            err(m.stat(&p("/f")), label).kind,
            K::NotFound,
            "{label}: flush is not finish"
        );
        s.finish().expect("finish");
        assert_eq!(read(&m, "/f"), b"data", "{label}");
    }
}

// cb_23 ---------------------------------------------------------------------

#[test]
fn cb_23_replace_false_on_existing_fails_immediately() {
    for (label, m) in profiles() {
        write(&m, "/x", b"original");
        let before = snap(&m);
        let written_before = stats(&m).bytes_written;
        let e = err(m.begin_write(&p("/x"), Some(3), false), label);
        assert_err(&e, K::AlreadyExists, "/x", label);
        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(
            stats(&m).bytes_written,
            written_before,
            "{label}: no bytes sent"
        );
        assert_eq!(stats(&m).open_sessions, 0, "{label}: no session left open");
    }
}

#[test]
fn cb_23_replace_false_is_checked_again_at_finish() {
    for (label, m) in profiles() {
        let mut first = m
            .begin_write(&p("/race"), None, false)
            .expect("first begin");
        first.write_all(b"first").expect("first write");

        let mut second = m
            .begin_write(&p("/race"), None, false)
            .expect("second begin");
        second.write_all(b"second").expect("second write");
        second.finish().expect("second finish");
        let after_second = snap(&m);

        let e = err(first.finish(), label);
        assert_err(&e, K::AlreadyExists, "/race", label);
        assert_eq!(
            snap(&m),
            after_second,
            "{label}: the losing writer commits nothing"
        );
        assert_eq!(read(&m, "/race"), b"second", "{label}");
        assert_eq!(stats(&m).open_sessions, 0, "{label}");
    }
}

// cb_24 ---------------------------------------------------------------------

#[test]
fn cb_24_replace_true_shows_old_content_until_finish() {
    for (label, m) in profiles() {
        write(&m, "/doc", b"old content");
        let mut s = m
            .begin_write(&p("/doc"), None, true)
            .expect("begin replace");
        s.write_all(b"NEW").expect("write");
        assert_eq!(read(&m, "/doc"), b"old content", "{label}: before finish");
        assert_eq!(m.stat(&p("/doc")).expect("stat").size, Some(11), "{label}");
        s.finish().expect("finish");
        assert_eq!(read(&m, "/doc"), b"NEW", "{label}: after finish");
        assert_eq!(m.stat(&p("/doc")).expect("stat").size, Some(3), "{label}");
    }
}

#[test]
fn cb_24_reader_opened_before_finish_sees_the_whole_old_content() {
    for (label, m) in profiles() {
        let old = common::pattern(3 * 4096);
        write(&m, "/doc", &old);
        let mut reader = m.open_read(&p("/doc"), 0).expect("open");
        let mut first = vec![0u8; 100];
        reader.read_exact(&mut first).expect("read a little");

        write_with(&m, "/doc", b"brand new", true);

        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).expect("finish reading");
        first.extend(rest);
        assert!(
            first == old,
            "{label}: a reader never sees old bytes then new bytes"
        );
    }
}

// cb_25 ---------------------------------------------------------------------

#[test]
fn cb_25_nothing_visible_before_finish() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        let before = snap(&m);
        let mut s = m.begin_write(&p("/d/new"), None, false).expect("begin");
        s.write_all(b"partial").expect("write");
        assert_eq!(
            err(m.stat(&p("/d/new")), label).kind,
            K::NotFound,
            "{label}"
        );
        assert!(!names(&m, "/d").contains("new"), "{label}");
        assert_eq!(
            snap(&m),
            before,
            "{label}: no node at all for an open session"
        );
        s.finish().expect("finish");
        assert!(names(&m, "/d").contains("new"), "{label}");
    }
}

#[test]
fn cb_25_abort_leaves_nothing_and_closes_the_session() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        write(&m, "/d/keep", b"keep");
        let before = snap(&m);
        let sessions_before = stats(&m).open_sessions;
        let mut s = m.begin_write(&p("/d/new"), None, false).expect("begin");
        s.write_all(&common::pattern(5000)).expect("write");
        assert_eq!(stats(&m).open_sessions, sessions_before + 1, "{label}");
        s.abort().expect("abort");
        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(stats(&m).open_sessions, sessions_before, "{label}");
        assert_eq!(
            names(&m, "/d"),
            BTreeSet::from(["keep".to_owned()]),
            "{label}"
        );
    }
}

#[test]
fn cb_25_abort_of_a_replace_keeps_the_old_content() {
    for (label, m) in profiles() {
        write(&m, "/x", b"old");
        let before = snap(&m);
        let mut s = m.begin_write(&p("/x"), None, true).expect("begin");
        s.write_all(b"new new new").expect("write");
        s.abort().expect("abort");
        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(read(&m, "/x"), b"old", "{label}");
    }
}

#[test]
fn cb_25_drop_leaves_nothing_and_closes_the_session() {
    for (label, m) in profiles() {
        write(&m, "/x", b"old");
        let before = snap(&m);
        {
            let mut s = m.begin_write(&p("/new"), None, false).expect("begin");
            s.write_all(b"half").expect("write");
            let mut r = m.begin_write(&p("/x"), None, true).expect("begin replace");
            r.write_all(b"replacement").expect("write");
            assert_eq!(stats(&m).open_sessions, 2, "{label}");
        }
        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(stats(&m).open_sessions, 0, "{label}");
        assert_eq!(read(&m, "/x"), b"old", "{label}");
    }
}

#[test]
fn cb_25_open_sessions_count_against_capacity_until_aborted_or_dropped() {
    for (label, m) in profiles() {
        m.set_capacity(Some(1000)).expect("capacity");
        let mut a = m.begin_write(&p("/a"), None, false).expect("begin a");
        a.write_all(&[1u8; 750]).expect("750 of 1000");

        // While `a` holds 750 bytes, a second 750-byte write cannot fit.
        let mut b = m.begin_write(&p("/b"), None, false).expect("begin b");
        let blocked = b.write_all(&[2u8; 750]).and_then(|()| b.flush());
        let b_kind = match blocked {
            Err(e) => {
                drop(b);
                BackendError::from_io(e, None).kind
            }
            Ok(()) => err(b.finish(), label).kind,
        };
        assert_eq!(b_kind, K::NoSpace, "{label}: open bytes are counted");
        assert_eq!(
            stats(&m).open_sessions,
            1,
            "{label}: only `a` is still open"
        );

        a.abort().expect("abort a");
        let mut c = m.begin_write(&p("/c"), None, false).expect("begin c");
        c.write_all(&[3u8; 750]).expect("space released by abort");
        c.finish().expect("finish c");

        // Same through Drop.
        m.set_capacity(Some(750 + 1000)).expect("capacity");
        {
            let mut d = m.begin_write(&p("/d"), None, false).expect("begin d");
            d.write_all(&[4u8; 750]).expect("d");
        }
        let mut e = m.begin_write(&p("/e"), None, false).expect("begin e");
        e.write_all(&[5u8; 750]).expect("space released by drop");
        e.finish().expect("finish e");
        assert_eq!(read(&m, "/e").len(), 750, "{label}");
    }
}

// cb_26 ---------------------------------------------------------------------

#[test]
fn cb_26_out_of_space_poisons_the_session() {
    for (label, m) in profiles() {
        m.set_capacity(Some(100)).expect("capacity");
        let before = snap(&m);
        let mut s = m.begin_write(&p("/big"), None, false).expect("begin");
        let io_err = s
            .write_all(&[7u8; 150])
            .expect_err("150 bytes do not fit in 100");
        assert_eq!(
            BackendError::from_io(io_err, None).kind,
            K::NoSpace,
            "{label}"
        );
        let again = s
            .write_all(b"x")
            .expect_err("a poisoned session refuses later writes");
        assert_eq!(
            BackendError::from_io(again, None).kind,
            K::NoSpace,
            "{label}"
        );
        let e = err(s.finish(), label);
        assert_eq!(
            e.kind,
            K::NoSpace,
            "{label}: finish reports the first error's kind"
        );
        assert_eq!(snap(&m), before, "{label}: nothing committed");
        assert_eq!(stats(&m).open_sessions, 0, "{label}");
    }
}

#[test]
fn cb_26_failed_replace_keeps_the_old_content() {
    for (label, m) in profiles() {
        write(&m, "/x", b"old bytes");
        m.set_capacity(Some(100)).expect("capacity");
        let before = snap(&m);
        let mut s = m.begin_write(&p("/x"), None, true).expect("begin");
        assert!(s.write_all(&[9u8; 150]).is_err(), "{label}");
        assert_eq!(err(s.finish(), label).kind, K::NoSpace, "{label}");
        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(read(&m, "/x"), b"old bytes", "{label}");
    }
}

#[test]
fn cb_26_injected_write_fault_poisons_with_its_kind() {
    for (label, m) in profiles() {
        let before = snap(&m);
        m.inject(Fault {
            op: Op::Write,
            path: Some(p("/f")),
            after: 5,
            effect: FaultEffect::Fail(K::PermissionDenied),
            times: Some(1),
        })
        .expect("inject");
        let mut s = m.begin_write(&p("/f"), None, false).expect("begin");
        let io_err = s.write_all(b"0123456789").expect_err("fault after 5 bytes");
        assert_eq!(
            BackendError::from_io(io_err, None).kind,
            K::PermissionDenied,
            "{label}"
        );
        // The fault fired once, but the session stays poisoned.
        assert!(s.write_all(b"more").is_err(), "{label}: still poisoned");
        assert_eq!(err(s.finish(), label).kind, K::PermissionDenied, "{label}");
        assert_eq!(snap(&m), before, "{label}");
    }
}

// cb_27 ---------------------------------------------------------------------

#[test]
fn cb_27_disconnect_reaches_every_method_with_the_argument_path() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        write(&m, "/d/a.txt", b"hello");
        m.disconnect().expect("disconnect");
        let c = Cancel::new();
        let checks: Vec<(&str, Result<(), BackendError>, &str)> = vec![
            ("list", m.list(&p("/d"), &c).map(drop), "/d"),
            ("stat", m.stat(&p("/d/a.txt")).map(drop), "/d/a.txt"),
            (
                "open_read",
                m.open_read(&p("/d/a.txt"), 0).map(drop),
                "/d/a.txt",
            ),
            (
                "begin_write",
                m.begin_write(&p("/d/n"), None, false).map(drop),
                "/d/n",
            ),
            ("create_dir", m.create_dir(&p("/d/e")), "/d/e"),
            ("remove", m.remove(&p("/d/a.txt")), "/d/a.txt"),
            ("remove_tree", m.remove_tree(&p("/d"), &c), "/d"),
        ];
        for (what, r, path) in checks {
            let e = err(r, what);
            assert_err(&e, K::Unavailable, path, &format!("{label} {what}"));
        }
        let e = err(m.rename(&p("/d/a.txt"), &p("/d/b.txt")), "rename");
        assert_eq!(e.kind, K::Unavailable, "{label} rename");
        assert!(
            e.path == Some(p("/d/a.txt")) || e.path == Some(p("/d/b.txt")),
            "{label} rename names an argument: {e:?}"
        );
        m.reconnect().expect("reconnect");
        assert_eq!(read(&m, "/d/a.txt"), b"hello", "{label}");
    }
}

#[test]
fn cb_27_disconnect_while_reading_and_writing_commits_nothing() {
    for (label, m) in profiles() {
        write(&m, "/a.txt", &common::pattern(4096));
        let before = snap(&m);

        let mut s = m.begin_write(&p("/new"), None, false).expect("begin");
        s.write_all(b"abc").expect("write");
        let mut r = m.open_read(&p("/a.txt"), 0).expect("open_read");

        m.disconnect().expect("disconnect");

        let mut buf = [0u8; 16];
        let read_err = r
            .read(&mut buf)
            .expect_err("an open reader notices the disconnect");
        assert_eq!(
            BackendError::from_io(read_err, None).kind,
            K::Unavailable,
            "{label}"
        );

        let write_err = s
            .write_all(b"def")
            .expect_err("an open session notices too");
        assert_eq!(
            BackendError::from_io(write_err, None).kind,
            K::Unavailable,
            "{label}"
        );
        assert_eq!(err(s.finish(), label).kind, K::Unavailable, "{label}");

        m.reconnect().expect("reconnect");
        assert_eq!(
            snap(&m),
            before,
            "{label}: committed data kept, unfinished file absent"
        );
        assert!(m.stat(&p("/a.txt")).is_ok(), "{label}");
        assert_eq!(
            names(&m, "/"),
            BTreeSet::from(["a.txt".to_owned()]),
            "{label}"
        );
    }
}

#[test]
fn cb_27_object_profile_copy_within_is_unavailable_when_disconnected() {
    let m = MemoryBackend::object_store_like();
    write(&m, "/a", b"x");
    m.disconnect().expect("disconnect");
    let e = err(m.copy_within(&p("/a"), &p("/b")), "copy_within");
    assert_eq!(e.kind, K::Unavailable);
    assert!(e.path == Some(p("/a")) || e.path == Some(p("/b")), "{e:?}");
    m.reconnect().expect("reconnect");
    assert_eq!(snap(&m).nodes.len(), 1);
}

// cb_28 ---------------------------------------------------------------------

#[test]
fn cb_28_a_directory_is_never_replaced_by_a_file() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        write(&m, "/d/child", b"c");
        let before = snap(&m);
        for replace in [false, true] {
            let e = err(m.begin_write(&p("/d"), None, replace), label);
            assert_err(
                &e,
                K::AlreadyExists,
                "/d",
                &format!("{label} replace={replace}"),
            );
        }
        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(read(&m, "/d/child"), b"c", "{label}");
    }
}

#[test]
fn cb_28_object_profile_markers_and_implicit_prefixes_are_directories() {
    let m = MemoryBackend::object_store_like();
    m.create_dir(&p("/marker")).expect("marker");
    write(&m, "/prefix/f", b"f");
    let before = snap(&m);
    for dir in ["/marker", "/prefix"] {
        for replace in [false, true] {
            let e = err(m.begin_write(&p(dir), None, replace), dir);
            assert_err(
                &e,
                K::AlreadyExists,
                dir,
                &format!("{dir} replace={replace}"),
            );
        }
    }
    assert_eq!(snap(&m), before);
}

// cb_29 ---------------------------------------------------------------------

#[test]
fn cb_29_real_directories_missing_parent_is_not_found() {
    let m = MemoryBackend::posix_like();
    write(&m, "/a.txt", b"a");
    let before = snap(&m);
    let e = err(m.create_dir(&p("/missing/x")), "create_dir");
    assert_err(
        &e,
        K::NotFound,
        "/missing/x",
        "create_dir under a missing parent",
    );
    let e = err(m.begin_write(&p("/missing/f"), None, false), "begin_write");
    assert_err(
        &e,
        K::NotFound,
        "/missing/f",
        "begin_write under a missing parent",
    );
    let e = err(m.rename(&p("/a.txt"), &p("/missing/b")), "rename");
    assert_err(
        &e,
        K::NotFound,
        "/missing/b",
        "rename into a missing parent",
    );
    let e = err(m.create_dir(&p("/m/n/o")), "create_dir is not recursive");
    assert_err(&e, K::NotFound, "/m/n/o", "create_dir is not recursive");
    assert_eq!(snap(&m), before);
}

#[test]
fn cb_29_object_profile_missing_prefix_is_implicit() {
    let m = MemoryBackend::object_store_like();
    write(&m, "/x/y/z", b"z");
    let root = m
        .list(&RemotePath::root(), &Cancel::new())
        .expect("list root");
    let x = root
        .entries
        .iter()
        .find(|e| e.display == "x")
        .expect("x listed");
    assert!(common::is_dir(x), "an ancestor prefix lists as a Directory");
    let y = common::entry(&m, "/x", "y");
    assert!(common::is_dir(&y));
    assert!(common::is_dir(&m.stat(&p("/x/y")).expect("stat prefix")));

    let s = snap(&m);
    assert_eq!(s.nodes.get(&p("/x/y/z")), Some(&file(b"z")));
    assert_eq!(s.nodes.get(&p("/x")), None, "no marker for /x");
    assert_eq!(s.nodes.get(&p("/x/y")), None, "no marker for /x/y");
    assert_eq!(s.nodes.len(), 1);
}

#[test]
fn cb_29_object_profile_create_dir_marks_only_the_requested_path() {
    let m = MemoryBackend::object_store_like();
    m.create_dir(&p("/q/r"))
        .expect("create_dir under a missing prefix");
    let s = snap(&m);
    assert_eq!(s.nodes.get(&p("/q/r")), Some(&MemoryNode::DirMarker));
    assert_eq!(s.nodes.len(), 1, "only /q/r gets a marker: {:?}", s.nodes);
    assert!(common::is_dir(&common::entry(&m, "/", "q")));
}

#[test]
fn cb_29_with_capabilities_follows_real_directories() {
    let m = MemoryBackend::with_capabilities(object_caps());
    write(&m, "/deep/er/f", b"1");
    assert!(names(&m, "/").contains("deep"));
}

// cb_30 ---------------------------------------------------------------------

#[test]
fn cb_30_concurrent_replace_sessions_never_mix() {
    for (label, m) in profiles() {
        write(&m, "/t", b"original");
        let a_bytes = vec![b'A'; 3000];
        let b_bytes = vec![b'B'; 2000];
        let mut a = m.begin_write(&p("/t"), None, true).expect("a");
        let mut b = m.begin_write(&p("/t"), None, true).expect("b");
        assert_eq!(stats(&m).open_sessions, 2, "{label}");
        for i in 0..10 {
            a.write_all(&a_bytes[i * 300..(i + 1) * 300])
                .expect("a write");
            b.write_all(&b_bytes[i * 200..(i + 1) * 200])
                .expect("b write");
        }
        b.finish().expect("b finish");
        a.finish().expect("a finish");
        let got = read(&m, "/t");
        assert!(
            got == a_bytes || got == b_bytes,
            "{label}: content is exactly one session's"
        );
        assert_eq!(stats(&m).open_sessions, 0, "{label}");
    }
}

#[test]
fn cb_30_concurrent_replace_false_sessions_second_finish_loses() {
    for (label, m) in profiles() {
        let mut a = m.begin_write(&p("/t"), None, false).expect("a");
        let mut b = m.begin_write(&p("/t"), None, false).expect("b");
        assert_eq!(stats(&m).open_sessions, 2, "{label}");
        a.write_all(b"aaa").expect("a write");
        b.write_all(b"bbbb").expect("b write");
        a.finish().expect("first finish wins");
        let e = err(b.finish(), label);
        assert_err(&e, K::AlreadyExists, "/t", label);
        assert_eq!(read(&m, "/t"), b"aaa", "{label}");
    }
}

// cb_31 ---------------------------------------------------------------------

#[test]
fn cb_31_abort_reports_its_own_failure_but_still_leaves_nothing() {
    for (label, m) in profiles() {
        let before = snap(&m);
        let mut s = m.begin_write(&p("/n"), None, false).expect("begin");
        s.write_all(b"bytes").expect("write");
        m.disconnect().expect("disconnect");
        let e = err(s.abort(), label);
        assert_eq!(e.kind, K::Unavailable, "{label}");
        m.reconnect().expect("reconnect");
        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(stats(&m).open_sessions, 0, "{label}");
    }
}

#[test]
fn cb_31_dropping_a_session_while_disconnected_does_not_panic() {
    for (label, m) in profiles() {
        let before = snap(&m);
        let mut s = m.begin_write(&p("/n"), None, false).expect("begin");
        s.write_all(b"bytes").expect("write");
        m.disconnect().expect("disconnect");
        drop(s);
        m.reconnect().expect("reconnect");
        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(stats(&m).open_sessions, 0, "{label}");
    }
}
