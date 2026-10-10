//! MemoryBackend profiles and capability stability.
//! api_05 / api_09 capability tables, cb_17 (object safety, constant capabilities).

mod common;

use std::sync::Arc;
use std::thread;

use common::{mkdir, object_caps, p, posix_caps, read, write};
use kara_vfs::memory::MemoryBackend;
use kara_vfs::{Backend, BackendErrorKind as K, Capabilities, WriteSession};

#[test]
fn api_05_capabilities_default_declares_nothing() {
    let c = Capabilities::default();
    assert_eq!(
        c,
        Capabilities {
            trash: false,
            atomic_rename: false,
            server_side_copy: false,
            real_directories: false,
            posix_permissions: false,
            symlinks: false,
            watch: false,
            undo_rename: false,
            undo_move: false,
        }
    );
}

#[test]
fn api_09_posix_like_declares_the_sftp_column() {
    assert_eq!(MemoryBackend::posix_like().capabilities(), posix_caps());
}

#[test]
fn api_09_object_store_like_declares_the_s3_column() {
    assert_eq!(
        MemoryBackend::object_store_like().capabilities(),
        object_caps()
    );
}

#[test]
fn api_09_with_capabilities_declares_exactly_what_it_is_given() {
    let custom = Capabilities {
        trash: true,
        watch: true,
        real_directories: true,
        ..Capabilities::default()
    };
    assert_eq!(
        MemoryBackend::with_capabilities(custom).capabilities(),
        custom
    );
    assert_eq!(
        MemoryBackend::with_capabilities(posix_caps()).capabilities(),
        posix_caps()
    );
}

#[test]
fn api_09_with_capabilities_behaviour_follows_real_directories() {
    // Object-store behaviour: a missing prefix is fine.
    let obj = MemoryBackend::with_capabilities(object_caps());
    write(&obj, "/x/y/f", b"1");
    assert_eq!(read(&obj, "/x/y/f"), b"1");

    // Real directories: a missing parent is NotFound.
    let posix = MemoryBackend::with_capabilities(posix_caps());
    let e = common::err(
        posix.begin_write(&p("/x/f"), None, false),
        "begin_write under missing dir",
    );
    common::assert_err(&e, K::NotFound, "/x/f", "begin_write under missing dir");
}

#[test]
fn api_09_with_capabilities_behaviour_follows_server_side_copy_and_symlinks() {
    let no_copy = MemoryBackend::with_capabilities(Capabilities {
        server_side_copy: false,
        ..object_caps()
    });
    write(&no_copy, "/a", b"abc");
    let e = common::err(
        no_copy.copy_within(&p("/a"), &p("/b")),
        "copy_within without capability",
    );
    assert_eq!(e.kind, K::Unsupported);

    let with_copy = MemoryBackend::with_capabilities(Capabilities {
        server_side_copy: true,
        ..posix_caps()
    });
    write(&with_copy, "/a", b"abc");
    with_copy
        .copy_within(&p("/a"), &p("/b"))
        .expect("declared server_side_copy works");
    assert_eq!(read(&with_copy, "/b"), b"abc");

    let no_links = MemoryBackend::with_capabilities(Capabilities {
        symlinks: false,
        ..posix_caps()
    });
    mkdir(&no_links, "/t");
    let e = common::err(
        no_links.create_symlink(&p("/l"), "/t"),
        "create_symlink without capability",
    );
    assert_eq!(e.kind, K::Unsupported);
}

fn assert_send_sync<T: Send + Sync + ?Sized>() {}
fn assert_send<T: Send + ?Sized>() {}

#[test]
fn cb_17_backend_is_object_safe_and_shareable() {
    assert_send_sync::<dyn Backend>();
    assert_send_sync::<Arc<dyn Backend>>();
    assert_send_sync::<Box<dyn Backend>>();
    assert_send_sync::<MemoryBackend>();
    assert_send::<Box<dyn WriteSession>>();
    assert_send::<Box<dyn std::io::Read + Send>>();

    let b: Arc<dyn Backend> = Arc::new(MemoryBackend::posix_like());
    let caps = b.capabilities();
    let worker = Arc::clone(&b);
    let from_thread = thread::spawn(move || {
        write(worker.as_ref(), "/made-in-thread", b"hi");
        worker.capabilities()
    })
    .join()
    .expect("thread");
    assert_eq!(from_thread, caps);
    assert_eq!(read(b.as_ref(), "/made-in-thread"), b"hi");

    let boxed: Box<dyn Backend> = Box::new(MemoryBackend::object_store_like());
    assert_eq!(boxed.capabilities(), object_caps());
}

#[test]
fn cb_17_capabilities_are_constant_across_disconnect_and_faults() {
    for (label, m) in common::profiles() {
        let before = m.capabilities();
        m.disconnect().expect("disconnect");
        assert_eq!(m.capabilities(), before, "{label}: while disconnected");
        m.reconnect().expect("reconnect");
        assert_eq!(m.capabilities(), before, "{label}: after reconnect");
        m.set_capacity(Some(1)).expect("set_capacity");
        m.inject(kara_vfs::memory::Fault {
            op: kara_vfs::memory::Op::Stat,
            path: None,
            after: 0,
            effect: kara_vfs::memory::FaultEffect::Fail(K::Other),
            times: None,
        })
        .expect("inject");
        assert_eq!(
            m.capabilities(),
            before,
            "{label}: with faults and a capacity"
        );
    }
}
