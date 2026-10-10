//! Every error kind is reachable through a real MemoryBackend path, and a
//! failed mutation changes nothing. Edge case cb_42, plus the fault controls.

mod common;

use std::io::Write;

use common::{assert_err, err, mkdir, p, profiles, read, snap, stats, write};
use kara_vfs::memory::{Fault, FaultEffect, MemoryBackend, Op};
use kara_vfs::{Backend, BackendErrorKind as K, Cancel, RemotePath};

fn fault(op: Op, kind: K) -> Fault {
    Fault {
        op,
        path: None,
        after: 0,
        effect: FaultEffect::Fail(kind),
        times: None,
    }
}

#[test]
fn cb_42_not_found() {
    for (label, m) in profiles() {
        let e = err(m.stat(&p("/nope")), label);
        assert_err(&e, K::NotFound, "/nope", label);
    }
}

#[test]
fn cb_42_already_exists() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        let before = snap(&m);
        let e = err(m.create_dir(&p("/d")), label);
        assert_err(&e, K::AlreadyExists, "/d", label);
        assert_eq!(snap(&m), before, "{label}");
    }
}

#[test]
fn cb_42_permission_denied() {
    for (label, m) in profiles() {
        let before = snap(&m);
        m.inject(fault(Op::CreateDir, K::PermissionDenied))
            .expect("inject");
        let e = err(m.create_dir(&p("/d")), label);
        assert_err(&e, K::PermissionDenied, "/d", label);
        assert_eq!(snap(&m), before, "{label}");
    }
}

#[test]
fn cb_42_no_space() {
    for (label, m) in profiles() {
        m.set_capacity(Some(10)).expect("capacity");
        let before = snap(&m);
        let mut s = m.begin_write(&p("/f"), None, false).expect("begin");
        let _ = s.write_all(&[0u8; 11]);
        let e = err(s.finish(), label);
        assert_eq!(e.kind, K::NoSpace, "{label}");
        assert_eq!(snap(&m), before, "{label}");
    }
}

#[test]
fn cb_42_unavailable() {
    for (label, m) in profiles() {
        m.disconnect().expect("disconnect");
        let e = err(m.stat(&p("/x")), label);
        assert_err(&e, K::Unavailable, "/x", label);
        m.reconnect().expect("reconnect");
        assert!(
            m.stat(&RemotePath::root()).is_ok(),
            "{label}: reconnect restores service"
        );
    }
}

#[test]
fn cb_42_auth_required() {
    for (label, m) in profiles() {
        write(&m, "/f", b"f");
        let before = snap(&m);
        m.inject(fault(Op::Remove, K::AuthRequired))
            .expect("inject");
        let e = err(m.remove(&p("/f")), label);
        assert_err(&e, K::AuthRequired, "/f", label);
        assert_eq!(snap(&m), before, "{label}");
    }
}

#[test]
fn cb_42_unsupported() {
    let posix = MemoryBackend::posix_like();
    write(&posix, "/a", b"a");
    let before = snap(&posix);
    let e = err(posix.copy_within(&p("/a"), &p("/b")), "copy_within");
    assert_eq!(e.kind, K::Unsupported);
    assert_eq!(snap(&posix), before);

    let object = MemoryBackend::object_store_like();
    let e = err(object.create_symlink(&p("/l"), "/a"), "create_symlink");
    assert_eq!(e.kind, K::Unsupported);
    assert!(snap(&object).nodes.is_empty());
}

#[test]
fn cb_42_cancelled() {
    for (label, m) in profiles() {
        let token = Cancel::new();
        token.cancel();
        let e = err(m.list(&RemotePath::root(), &token), label);
        assert_eq!(e.kind, K::Cancelled, "{label}");
    }
}

#[test]
fn cb_42_other() {
    let m = MemoryBackend::posix_like();
    write(&m, "/a", b"a");
    let before = snap(&m);
    m.inject(fault(Op::Rename, K::Other)).expect("inject");
    let e = err(m.rename(&p("/a"), &p("/b")), "rename");
    assert_eq!(e.kind, K::Other);
    assert_eq!(snap(&m), before);
}

#[test]
fn cb_42_faults_on_mutating_ops_change_nothing() {
    for (label, m) in profiles() {
        mkdir(&m, "/d");
        write(&m, "/d/f", b"content");
        let before = snap(&m);

        m.inject(fault(Op::CreateDir, K::PermissionDenied))
            .expect("inject");
        assert_eq!(
            err(m.create_dir(&p("/d/new")), label).kind,
            K::PermissionDenied,
            "{label}"
        );
        m.clear_faults().expect("clear");

        m.inject(fault(Op::Remove, K::PermissionDenied))
            .expect("inject");
        assert_eq!(
            err(m.remove(&p("/d/f")), label).kind,
            K::PermissionDenied,
            "{label}"
        );
        m.clear_faults().expect("clear");

        m.inject(fault(Op::BeginWrite, K::NoSpace)).expect("inject");
        assert_eq!(
            err(m.begin_write(&p("/d/g"), None, false), label).kind,
            K::NoSpace,
            "{label}"
        );
        m.clear_faults().expect("clear");

        m.inject(fault(Op::Finish, K::Unavailable)).expect("inject");
        let mut s = m.begin_write(&p("/d/f"), None, true).expect("begin");
        s.write_all(b"replacement").expect("write");
        assert_eq!(err(s.finish(), label).kind, K::Unavailable, "{label}");
        m.clear_faults().expect("clear");

        assert_eq!(snap(&m), before, "{label}");
        assert_eq!(stats(&m).open_sessions, 0, "{label}");
        assert_eq!(read(&m, "/d/f"), b"content", "{label}");
    }
}

#[test]
fn cb_42_fault_on_posix_rename_changes_nothing() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/d");
    write(&m, "/d/f", b"content");
    let before = snap(&m);
    m.inject(fault(Op::Rename, K::PermissionDenied))
        .expect("inject");
    assert_eq!(
        err(m.rename(&p("/d"), &p("/e")), "rename").kind,
        K::PermissionDenied
    );
    assert_eq!(snap(&m), before);
}

#[test]
fn cb_42_fault_path_filter_matches_only_that_path() {
    let m = MemoryBackend::posix_like();
    write(&m, "/a", b"a");
    write(&m, "/b", b"b");
    m.inject(Fault {
        op: Op::Stat,
        path: Some(p("/a")),
        after: 0,
        effect: FaultEffect::Fail(K::PermissionDenied),
        times: None,
    })
    .expect("inject");
    assert!(m.stat(&p("/b")).is_ok(), "a fault on /a leaves /b alone");
    assert_eq!(err(m.stat(&p("/a")), "stat /a").kind, K::PermissionDenied);
    assert!(
        m.open_read(&p("/a"), 0).is_ok(),
        "a Stat fault leaves OpenRead alone"
    );
}

#[test]
fn cb_42_fault_times_limits_how_often_it_fires() {
    let m = MemoryBackend::posix_like();
    write(&m, "/a", b"a");
    m.inject(Fault {
        op: Op::Stat,
        path: None,
        after: 0,
        effect: FaultEffect::Fail(K::AuthRequired),
        times: Some(2),
    })
    .expect("inject");
    assert_eq!(err(m.stat(&p("/a")), "1st").kind, K::AuthRequired);
    assert_eq!(err(m.stat(&p("/a")), "2nd").kind, K::AuthRequired);
    assert!(m.stat(&p("/a")).is_ok(), "times: Some(2) fires twice only");
}

#[test]
fn cb_42_fault_without_times_fires_until_cleared() {
    let m = MemoryBackend::posix_like();
    write(&m, "/a", b"a");
    m.inject(fault(Op::OpenRead, K::Other)).expect("inject");
    for i in 0..5 {
        assert_eq!(
            err(m.open_read(&p("/a"), 0), "open").kind,
            K::Other,
            "attempt {i}"
        );
    }
    m.clear_faults().expect("clear");
    assert_eq!(read(&m, "/a"), b"a");
}
