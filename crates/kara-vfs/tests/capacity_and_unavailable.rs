//! MemoryBackend capacity accounting (replacing an object is not counted
//! twice; failed, aborted and disconnected sessions give their bytes back) and
//! the gaps in `Unavailable` coverage: errors of dead sessions and readers name
//! their path, and faults are not consumed by calls that never reached them.

mod common;

use std::io::{Read, Write};

use common::{assert_err, err, mkdir, p, profiles, read, snap, stats, write, write_with};
use kara_vfs::memory::{Fault, FaultEffect, MemoryBackend, Op};
use kara_vfs::{Backend, BackendError, BackendErrorKind as K, Cancel};

fn try_put(m: &MemoryBackend, path: &str, len: usize, replace: bool) -> Result<(), BackendError> {
    let mut session = m.begin_write(&p(path), None, replace)?;
    session
        .write_all(&vec![b'n'; len])
        .map_err(|e| BackendError::from_io(e, Some(&p(path))))?;
    session.finish()
}

#[test]
fn replacing_an_object_with_one_of_the_same_size_fits_a_full_drive() {
    for (label, m) in profiles() {
        m.set_capacity(Some(100)).expect("capacity");
        write(&m, "/a", &[b'o'; 60]);
        try_put(&m, "/a", 60, true).unwrap_or_else(|e| panic!("{label}: {e:?}"));
        assert_eq!(read(&m, "/a"), vec![b'n'; 60], "{label}");
        // And again, at the very edge.
        write(&m, "/b", &[b'o'; 40]);
        try_put(&m, "/b", 40, true).unwrap_or_else(|e| panic!("{label}: {e:?}"));
    }
}

#[test]
fn replacing_counts_the_rest_of_the_drive_exactly() {
    for (label, m) in profiles() {
        m.set_capacity(Some(100)).expect("capacity");
        write(&m, "/a", &[b'o'; 60]);
        write(&m, "/b", &[b'o'; 30]);
        let e = err(try_put(&m, "/a", 71, true), label);
        assert_eq!(e.kind, K::NoSpace, "{label}: 30 + 71 > 100");
        assert_eq!(read(&m, "/a"), vec![b'o'; 60], "{label}: old object kept");
        try_put(&m, "/a", 70, true).unwrap_or_else(|e| panic!("{label}: 30 + 70: {e:?}"));
        // A new object still counts everything that is stored.
        let e = err(try_put(&m, "/c", 1, false), label);
        assert_eq!(e.kind, K::NoSpace, "{label}: the drive is full");
    }
}

#[test]
fn replacing_a_link_frees_nothing() {
    let m = MemoryBackend::posix_like();
    m.set_capacity(Some(100)).expect("capacity");
    write(&m, "/target", &[b'o'; 60]);
    m.create_symlink(&p("/link"), "/target").expect("link");
    let e = err(try_put(&m, "/link", 41, true), "replace the link");
    assert_eq!(e.kind, K::NoSpace, "the target still occupies 60 bytes");
    try_put(&m, "/link", 40, true).expect("40 bytes fit");
    assert_eq!(read(&m, "/target"), vec![b'o'; 60], "the target is untouched");
}

#[test]
fn two_sessions_replacing_one_object_share_its_room_once() {
    for (label, m) in profiles() {
        m.set_capacity(Some(100)).expect("capacity");
        write(&m, "/a", &[b'o'; 60]);
        let mut first = m.begin_write(&p("/a"), None, true).expect("first");
        let mut second = m.begin_write(&p("/a"), None, true).expect("second");
        first.write_all(&[b'1'; 40]).expect("first 40");
        second.write_all(&[b'2'; 60]).expect("second 60: 60 + 40 + 60 - 60");
        assert!(second.write_all(&[b'2'; 1]).is_err(), "{label}: one more byte is too many");
        first.finish().expect("first wins");
        assert_eq!(read(&m, "/a"), vec![b'1'; 40], "{label}");
    }
}

#[test]
fn failed_aborted_and_dropped_sessions_give_their_bytes_back() {
    for (label, m) in profiles() {
        m.set_capacity(Some(100)).expect("capacity");
        let e = err(try_put(&m, "/too-big", 101, false), label);
        assert_eq!(e.kind, K::NoSpace, "{label}");

        let mut aborted = m.begin_write(&p("/aborted"), None, false).expect("begin");
        aborted.write_all(&[0; 50]).expect("write");
        aborted.abort().expect("abort");

        let mut dropped = m.begin_write(&p("/dropped"), None, false).expect("begin");
        dropped.write_all(&[0; 50]).expect("write");
        drop(dropped);

        try_put(&m, "/full", 100, false).unwrap_or_else(|e| panic!("{label}: leaked: {e:?}"));
        assert_eq!(stats(&m).open_sessions, 0, "{label}");
    }
}

#[test]
fn a_session_killed_by_a_disconnect_gives_its_bytes_back() {
    for (label, m) in profiles() {
        m.set_capacity(Some(100)).expect("capacity");
        let mut session = m.begin_write(&p("/w"), None, false).expect("begin");
        session.write_all(&[0; 70]).expect("write");
        m.disconnect().expect("disconnect");
        let failure = session.write_all(&[0; 1]).expect_err("dead session");
        let e = BackendError::from_io(failure, None);
        assert_err(&e, K::Unavailable, "/w", &format!("{label}: write after disconnect"));
        m.reconnect().expect("reconnect");
        let e = err(session.finish(), label);
        assert_err(&e, K::Unavailable, "/w", &format!("{label}: finish after reconnect"));
        try_put(&m, "/full", 100, false).unwrap_or_else(|e| panic!("{label}: leaked: {e:?}"));
        assert_eq!(stats(&m).open_sessions, 0, "{label}");
    }
}

#[test]
fn a_reader_killed_by_a_disconnect_names_its_path() {
    for (label, m) in profiles() {
        write(&m, "/r", b"0123456789");
        let mut reader = m.open_read(&p("/r"), 2).expect("open");
        m.disconnect().expect("disconnect");
        m.reconnect().expect("reconnect");
        let mut buf = [0u8; 4];
        let failure = reader.read(&mut buf).expect_err("dead reader");
        let e = BackendError::from_io(failure, None);
        assert_err(&e, K::Unavailable, "/r", label);
        assert_eq!(read(&m, "/r"), b"0123456789", "{label}: data untouched");
    }
}

#[test]
fn abort_while_disconnected_reports_it_and_commits_nothing() {
    for (label, m) in profiles() {
        let before = snap(&m);
        let mut session = m.begin_write(&p("/w"), None, false).expect("begin");
        session.write_all(b"half").expect("write");
        m.disconnect().expect("disconnect");
        let e = err(session.abort(), label);
        assert_err(&e, K::Unavailable, "/w", label);
        m.reconnect().expect("reconnect");
        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(stats(&m).open_sessions, 0, "{label}");
    }
}

#[test]
fn a_fault_is_not_consumed_by_a_call_that_failed_on_the_connection() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        m.inject(Fault {
            op: Op::Stat,
            path: None,
            after: 0,
            effect: FaultEffect::Fail(K::PermissionDenied),
            times: Some(1),
        })
        .expect("inject");
        m.disconnect().expect("disconnect");
        assert_eq!(err(m.stat(&p("/d")), label).kind, K::Unavailable, "{label}");
        m.reconnect().expect("reconnect");
        assert_eq!(
            err(m.stat(&p("/d")), label).kind,
            K::PermissionDenied,
            "{label}: the fault still fires once"
        );
        assert!(m.stat(&p("/d")).is_ok(), "{label}: and only once");
    }
}

#[test]
fn every_mutation_fails_whole_while_disconnected() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        write(&m, "/d/f", b"f");
        write_with(&m, "/g", b"g", false);
        let before = snap(&m);
        m.disconnect().expect("disconnect");
        let _ = m.create_dir(&p("/e"));
        let _ = m.rename(&p("/g"), &p("/h"));
        let _ = m.remove(&p("/g"));
        let _ = m.remove_tree(&p("/d"), &Cancel::new());
        let _ = m.copy_within(&p("/g"), &p("/i"));
        let _ = m.begin_write(&p("/g"), None, true).map(|s| s.finish());
        m.reconnect().expect("reconnect");
        assert_eq!(snap(&m), before, "{label}: nothing changed while disconnected");
    }
}
