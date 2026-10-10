//! BackendError (kinds, Display, source, io bridging) and Cancel.
//! Edge cases cb_11, cb_13, cb_14, cb_16.

mod common;

use std::error::Error as _;
use std::io::{self, ErrorKind as IoKind};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use common::{p, write};
use kara_vfs::conformance::ConformanceError;
use kara_vfs::memory::{Fault, FaultEffect, MemoryBackend, Op};
use kara_vfs::{Backend, BackendError, BackendErrorKind as K, Cancel};

const ALL_KINDS: [K; 9] = [
    K::NotFound,
    K::AlreadyExists,
    K::PermissionDenied,
    K::NoSpace,
    K::Unavailable,
    K::AuthRequired,
    K::Unsupported,
    K::Cancelled,
    K::Other,
];

fn assert_error_bounds<T: Send + Sync + 'static + std::error::Error>() {}

// cb_11 ---------------------------------------------------------------------

#[test]
fn cb_11_errors_are_send_sync_static_errors() {
    assert_error_bounds::<BackendError>();
    assert_error_bounds::<ConformanceError>();
    assert_error_bounds::<kara_vfs::RemotePathError>();
    assert_error_bounds::<kara_vfs::LocationError>();
    assert_error_bounds::<kara_vfs::DriveIdError>();
}

#[test]
fn cb_11_new_sets_fields_and_has_no_source() {
    let e = BackendError::new(K::NotFound, Some(p("/a b")));
    assert_eq!(e.kind, K::NotFound);
    assert_eq!(e.kind(), K::NotFound);
    assert_eq!(e.path, Some(p("/a b")));
    assert!(e.source.is_none());
    assert!(e.source().is_none());
}

#[test]
fn cb_11_with_source_keeps_the_cause() {
    let path = p("/dir/file.txt");
    let e = BackendError::new(K::NotFound, Some(path.clone()))
        .with_source(io::Error::from(IoKind::NotFound));
    let source = e.source().expect("source() must return the wrapped error");
    let io_err = source
        .downcast_ref::<io::Error>()
        .expect("source is the io::Error");
    assert_eq!(io_err.kind(), IoKind::NotFound);
    assert_eq!(e.kind, K::NotFound, "with_source must not change the kind");
    assert_eq!(e.path, Some(path), "with_source must not change the path");
}

#[test]
fn cb_11_display_names_the_kind_and_the_path() {
    let path = p("/home/ana/a b.txt");
    for kind in ALL_KINDS {
        let with_path = BackendError::new(kind, Some(path.clone()));
        assert_eq!(format!("{with_path}"), format!("{kind}: {}", path.as_str()));
        assert!(format!("{with_path}").contains(path.as_str()));
        let without = BackendError::new(kind, None);
        assert_eq!(format!("{without}"), format!("{kind}"));
    }
}

#[test]
fn cb_11_kind_display_is_a_distinct_nonempty_phrase() {
    let texts: Vec<String> = ALL_KINDS.iter().map(|k| k.to_string()).collect();
    for (kind, text) in ALL_KINDS.iter().zip(&texts) {
        assert!(!text.trim().is_empty(), "{kind:?} displays as empty");
    }
    let unique: std::collections::BTreeSet<&String> = texts.iter().collect();
    assert_eq!(
        unique.len(),
        ALL_KINDS.len(),
        "kind phrases must differ: {texts:?}"
    );
    assert!(K::NotFound.to_string().to_lowercase().contains("not found"));
}

// cb_13 ---------------------------------------------------------------------

#[test]
fn cb_13_backend_error_to_io_error_kinds() {
    let table = [
        (K::NotFound, IoKind::NotFound),
        (K::AlreadyExists, IoKind::AlreadyExists),
        (K::PermissionDenied, IoKind::PermissionDenied),
        (K::NoSpace, IoKind::StorageFull),
        (K::Unavailable, IoKind::NotConnected),
        (K::AuthRequired, IoKind::PermissionDenied),
        (K::Unsupported, IoKind::Unsupported),
        (K::Cancelled, IoKind::Other),
        (K::Other, IoKind::Other),
    ];
    for (kind, io_kind) in table {
        let as_io = io::Error::from(BackendError::new(kind, Some(p("/x"))));
        assert_eq!(as_io.kind(), io_kind, "{kind:?}");
    }
}

#[test]
fn cb_13_cancelled_is_never_interrupted() {
    let as_io = io::Error::from(BackendError::new(K::Cancelled, None));
    assert_ne!(as_io.kind(), IoKind::Interrupted);
    let as_io = io::Error::from(BackendError::new(K::Cancelled, Some(p("/x"))));
    assert_ne!(as_io.kind(), IoKind::Interrupted);
}

#[test]
fn cb_13_io_round_trip_recovers_kind_and_path_exactly() {
    let path = p("/a/b c.txt");
    for kind in ALL_KINDS {
        let as_io = io::Error::from(BackendError::new(kind, Some(path.clone())));
        // The other path argument must NOT override the original one.
        let back = BackendError::from_io(as_io, Some(&p("/elsewhere")));
        assert_eq!(back.kind, kind, "{kind:?}");
        assert_eq!(back.path, Some(path.clone()), "{kind:?}");

        let as_io = io::Error::from(BackendError::new(kind, Some(path.clone())));
        let back = BackendError::from_io(as_io, None);
        assert_eq!(
            (back.kind, back.path),
            (kind, Some(path.clone())),
            "{kind:?} with None"
        );
    }
}

#[test]
fn cb_13_io_round_trip_keeps_the_original_source() {
    let original = BackendError::new(K::Unavailable, Some(p("/f")))
        .with_source(io::Error::from(IoKind::ConnectionReset));
    let back = BackendError::from_io(io::Error::from(original), None);
    let src = back
        .source()
        .and_then(|s| s.downcast_ref::<io::Error>())
        .expect("original source survives the round trip");
    assert_eq!(src.kind(), IoKind::ConnectionReset);
}

#[test]
fn cb_13_read_fault_through_io_copy_maps_back_to_unavailable() {
    let content = common::pattern(100);
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let m = MemoryBackend::posix_like();
        write(&m, "/f", &content);
        m.inject(Fault {
            op: Op::Read,
            path: Some(p("/f")),
            after: 4,
            effect: FaultEffect::Fail(K::Unavailable),
            times: None,
        })
        .expect("inject");
        let mut reader = m.open_read(&p("/f"), 0).expect("open_read");
        let mut sink = Vec::new();
        let result = io::copy(&mut reader, &mut sink);
        let _ = tx.send((
            result.map_err(|e| BackendError::from_io(e, None)),
            sink,
            content,
        ));
    });
    // io::copy retries Interrupted forever: a hang here means Cancelled/Unavailable
    // was turned into Interrupted.
    let (result, sink, content) = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("io::copy must not loop forever");
    let e = result.expect_err("io::copy must fail on the injected fault");
    assert_eq!(e.kind, K::Unavailable);
    assert!(
        sink.len() <= 4,
        "no byte past the fault point may be delivered: {}",
        sink.len()
    );
    assert_eq!(
        sink[..],
        content[..sink.len()],
        "delivered bytes are the file's prefix"
    );
}

// cb_14 ---------------------------------------------------------------------

#[test]
fn cb_14_foreign_io_kinds_map_to_backend_kinds() {
    let table = [
        (IoKind::NotFound, K::NotFound),
        (IoKind::AlreadyExists, K::AlreadyExists),
        (IoKind::PermissionDenied, K::PermissionDenied),
        (IoKind::ReadOnlyFilesystem, K::PermissionDenied),
        (IoKind::StorageFull, K::NoSpace),
        (IoKind::QuotaExceeded, K::NoSpace),
        (IoKind::ConnectionRefused, K::Unavailable),
        (IoKind::ConnectionReset, K::Unavailable),
        (IoKind::ConnectionAborted, K::Unavailable),
        (IoKind::NotConnected, K::Unavailable),
        (IoKind::BrokenPipe, K::Unavailable),
        (IoKind::TimedOut, K::Unavailable),
        (IoKind::HostUnreachable, K::Unavailable),
        (IoKind::NetworkUnreachable, K::Unavailable),
        (IoKind::NetworkDown, K::Unavailable),
        (IoKind::StaleNetworkFileHandle, K::Unavailable),
        (IoKind::Unsupported, K::Unsupported),
        (IoKind::Interrupted, K::Other),
        (IoKind::InvalidData, K::Other),
        (IoKind::InvalidInput, K::Other),
        (IoKind::UnexpectedEof, K::Other),
        (IoKind::Other, K::Other),
    ];
    let path = p("/x y");
    for (io_kind, kind) in table {
        let e = BackendError::from_io(io::Error::from(io_kind), Some(&path));
        assert_eq!(e.kind, kind, "{io_kind:?}");
        assert_eq!(
            e.path,
            Some(path.clone()),
            "{io_kind:?} keeps the given path"
        );
        let src = e
            .source()
            .and_then(|s| s.downcast_ref::<io::Error>())
            .unwrap_or_else(|| panic!("{io_kind:?}: the io::Error is kept as source"));
        assert_eq!(src.kind(), io_kind);
    }
}

#[test]
fn cb_14_raw_errnos_for_vanished_media_map_to_unavailable() {
    // EIO, ENXIO, ENODEV, ENOMEDIUM: checked before the io kind.
    for errno in [5, 6, 19, 123] {
        let e = BackendError::from_io(io::Error::from_raw_os_error(errno), None);
        assert_eq!(e.kind, K::Unavailable, "errno {errno}");
        assert_eq!(e.path, None);
        assert!(e.source().is_some(), "errno {errno} keeps its source");
    }
}

#[test]
fn cb_14_raw_errnos_follow_their_io_kind() {
    let path = p("/q");
    let table = [
        (28, K::NoSpace),          // ENOSPC
        (122, K::NoSpace),         // EDQUOT
        (2, K::NotFound),          // ENOENT
        (17, K::AlreadyExists),    // EEXIST
        (13, K::PermissionDenied), // EACCES
        (30, K::PermissionDenied), // EROFS
        (104, K::Unavailable),     // ECONNRESET
        (110, K::Unavailable),     // ETIMEDOUT
        (4, K::Other),             // EINTR
    ];
    for (errno, kind) in table {
        let e = BackendError::from_io(io::Error::from_raw_os_error(errno), Some(&path));
        assert_eq!(e.kind, kind, "errno {errno}");
        assert_eq!(e.path, Some(path.clone()), "errno {errno}");
    }
}

// cb_16 ---------------------------------------------------------------------

#[test]
fn cb_16_cancel_clones_share_one_flag() {
    let a = Cancel::new();
    let b = a.clone();
    assert!(!a.is_cancelled());
    assert!(!b.is_cancelled());
    b.cancel();
    assert!(a.is_cancelled());
    assert!(b.is_cancelled());
}

#[test]
fn cb_16_cancel_is_idempotent_and_irreversible() {
    let a = Cancel::new();
    a.cancel();
    a.cancel();
    assert!(a.is_cancelled());
    let c = a.clone();
    assert!(
        c.is_cancelled(),
        "a clone of a cancelled token is cancelled"
    );
}

#[test]
fn cb_16_fresh_tokens_are_independent() {
    assert!(!Cancel::default().is_cancelled());
    let a = Cancel::new();
    let b = Cancel::new();
    a.cancel();
    assert!(!b.is_cancelled(), "two Cancel::new() share nothing");
    assert!(!Cancel::default().is_cancelled());
}

#[test]
fn cb_16_cancel_crosses_threads() {
    let a = Cancel::new();
    let b = a.clone();
    thread::spawn(move || b.cancel()).join().expect("thread");
    assert!(a.is_cancelled());
}
