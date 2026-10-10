//! MemoryBackend never blocks and is safe to share across threads.
//! Edge cases cb_47 and cb_17 (Arc<dyn Backend> across threads).

mod common;

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use common::{mkdir, names, p, read, write};
use kara_vfs::memory::MemoryBackend;
use kara_vfs::{Backend, BackendErrorKind as K, Cancel};

#[test]
fn cb_47_eight_threads_share_one_backend() {
    for base in [
        MemoryBackend::posix_like as fn() -> MemoryBackend,
        MemoryBackend::object_store_like,
    ] {
        let b: Arc<dyn Backend> = Arc::new(base());
        if b.capabilities().real_directories {
            for t in 0..8 {
                mkdir(b.as_ref(), &format!("/t{t}"));
            }
        }
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let b = Arc::clone(&b);
                thread::spawn(move || {
                    let mut expected = BTreeSet::new();
                    let dir = format!("/t{t}");
                    for i in 0..200 {
                        let name = format!("f{i}");
                        let path = p(&format!("{dir}/{name}"));
                        let content = format!("{t}-{i}").into_bytes();
                        let mut s = b.begin_write(&path, None, false).expect("begin_write");
                        s.write_all(&content).expect("write");
                        s.finish().expect("finish");
                        let st = b.stat(&path).expect("stat");
                        assert_eq!(st.size, Some(content.len() as u64));
                        let listing = b.list(&p(&dir), &Cancel::new()).expect("list own dir");
                        assert!(listing.entries.iter().any(|e| e.display == name));
                        if i % 3 == 0 {
                            b.remove(&path).expect("remove");
                        } else {
                            expected.insert(name);
                        }
                        // Read another thread's directory while it is being written.
                        let other = format!("/t{}", (t + 1) % 8);
                        match b.list(&p(&other), &Cancel::new()) {
                            Ok(_) => {}
                            Err(e) => {
                                assert_eq!(e.kind, K::NotFound, "only 'not yet' is acceptable")
                            }
                        }
                    }
                    (dir, expected)
                })
            })
            .collect();
        for h in handles {
            let (dir, expected) = h.join().expect("worker thread panicked");
            assert_eq!(names(b.as_ref(), &dir), expected, "{dir}");
            for name in &expected {
                let i: usize = name[1..].parse().expect("index");
                let t = &dir[2..];
                assert_eq!(
                    read(b.as_ref(), &format!("{dir}/{name}")),
                    format!("{t}-{i}").into_bytes()
                );
            }
        }
    }
}

#[test]
fn cb_47_a_disconnected_backend_answers_at_once() {
    let m = MemoryBackend::posix_like();
    write(&m, "/f", b"f");
    m.disconnect().expect("disconnect");
    let start = Instant::now();
    for _ in 0..10_000 {
        let e = m.stat(&p("/f")).expect_err("disconnected");
        assert_eq!(e.kind, K::Unavailable);
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "10 000 stats took {elapsed:?}"
    );
}

#[test]
fn cb_47_open_readers_and_sessions_do_not_hold_the_backend() {
    let m = Arc::new(MemoryBackend::posix_like());
    write(m.as_ref(), "/a", &common::pattern(10_000));
    let mut reader = m.open_read(&p("/a"), 0).expect("open_read");
    let mut session = m.begin_write(&p("/b"), None, false).expect("begin_write");
    session.write_all(b"in progress").expect("write");

    let (tx, rx) = mpsc::channel();
    let other = Arc::clone(&m);
    thread::spawn(move || {
        let listed = other.list(&p("/"), &Cancel::new()).map(|l| l.entries.len());
        let stat = other.stat(&p("/a")).map(|e| e.size);
        let mut s2 = other
            .begin_write(&p("/c"), None, false)
            .expect("second session");
        s2.write_all(b"c").expect("write c");
        let finished = s2.finish();
        let _ = tx.send((listed, stat, finished.is_ok()));
    });
    let (listed, stat, finished) = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("another thread must not block on an open reader or session");
    assert_eq!(listed.expect("list"), 1, "only /a is committed");
    assert_eq!(stat.expect("stat"), Some(10_000));
    assert!(finished);

    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).expect("reader still works");
    assert_eq!(buf.len(), 10_000);
    session.finish().expect("session still works");
    assert_eq!(read(m.as_ref(), "/b"), b"in progress");
}
