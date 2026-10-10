//! Tests written because a deliberate mutation of `LocalBackend` survived the
//! suite (see docs/remote-backends-testing.md):
//!
//! - a cancel that arrives while `list` is reading must win: the existing race
//!   test accepts a full listing, so dropping the per-entry check went unseen;
//! - a cancel that arrives while `remove_tree` is walking must stop the walk;
//! - remove_tree must open every directory with `O_NOFOLLOW`: the lstat before
//!   it is not enough, because a directory can be exchanged for a link between
//!   the two calls.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use kara_fs::LocalBackend;
use kara_vfs::{Backend, BackendErrorKind, Cancel, RemotePath};

fn rp(text: &str) -> io::Result<RemotePath> {
    RemotePath::parse(text).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

fn rooted(dir: &Path) -> io::Result<LocalBackend> {
    LocalBackend::with_root(dir).map_err(io::Error::from)
}

fn joined<T>(handle: thread::JoinHandle<T>) -> io::Result<T> {
    handle
        .join()
        .map_err(|_| io::Error::other("a worker thread panicked"))
}

#[test]
fn a_cancel_that_arrives_while_list_reads_is_always_cancelled() -> io::Result<()> {
    // Far more entries than can be described in the few milliseconds before
    // the cancel: a correct backend can only answer Cancelled.
    const COUNT: usize = 50_000;
    let tmp = tempfile::tempdir()?;
    let backend = rooted(tmp.path())?;
    let dir = tmp.path().join("many");
    fs::create_dir(&dir)?;
    for index in 0..COUNT {
        fs::File::create(dir.join(format!("f{index:05}")))?;
    }
    let path = rp("/many")?;
    for delay_ms in [1_u64, 2, 4, 8] {
        let cancel = Cancel::new();
        let worker = {
            let backend = backend.clone();
            let cancel = cancel.clone();
            let path = path.clone();
            thread::spawn(move || backend.list(&path, &cancel))
        };
        thread::sleep(Duration::from_millis(delay_ms));
        cancel.cancel();
        match joined(worker)? {
            Ok(listing) => panic!(
                "cancelled {delay_ms} ms into the list, it still returned Ok with {} entries",
                listing.entries.len()
            ),
            Err(error) => {
                assert_eq!(error.kind, BackendErrorKind::Cancelled, "{error:?}");
                assert_eq!(error.path.as_ref(), Some(&path), "{error:?}");
            }
        }
    }
    Ok(())
}

#[test]
fn a_cancel_that_arrives_while_remove_tree_walks_stops_the_walk() -> io::Result<()> {
    const DIRS: usize = 20;
    const FILES: usize = 1_000;
    let tmp = tempfile::tempdir()?;
    let backend = rooted(tmp.path())?;
    let tree = tmp.path().join("tree");
    let mut samples = Vec::new();
    for d in 0..DIRS {
        let sub = tree.join(format!("d{d:02}"));
        fs::create_dir_all(&sub)?;
        for f in 0..FILES {
            let file = sub.join(format!("f{f:04}"));
            fs::File::create(&file)?;
            if f % 50 == 0 {
                samples.push(file);
            }
        }
    }
    let cancel = Cancel::new();
    let worker = {
        let backend = backend.clone();
        let cancel = cancel.clone();
        let path = rp("/tree")?;
        thread::spawn(move || backend.remove_tree(&path, &cancel))
    };
    // Cancel as soon as the walk has visibly started removing entries.
    let deadline = Instant::now() + Duration::from_secs(20);
    while samples.iter().all(|file| file.exists()) && Instant::now() < deadline {
        std::hint::spin_loop();
    }
    cancel.cancel();
    let outcome = joined(worker)?;
    match outcome {
        Ok(()) => panic!("cancelled mid-walk, remove_tree still removed the whole tree"),
        Err(error) => assert_eq!(error.kind, BackendErrorKind::Cancelled, "{error:?}"),
    }
    let left = fs::read_dir(&tree)?.count();
    assert!(left > 0, "a cancelled walk must leave the rest in place");
    Ok(())
}

/// Every path below `root` with the bytes of each file.
fn snapshot(root: &Path) -> io::Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if fs::symlink_metadata(&path)?.is_dir() {
            out.extend(snapshot(&path)?);
        } else {
            out.insert(path.clone(), fs::read(&path)?);
        }
    }
    Ok(out)
}

#[test]
fn a_directory_exchanged_for_a_link_between_lstat_and_open_is_never_followed()
-> io::Result<()> {
    let tmp = tempfile::tempdir()?;
    let outside = tmp.path().join("outside");
    fs::create_dir(&outside)?;
    for index in 0..8 {
        fs::write(outside.join(format!("keep{index}")), b"precious")?;
    }
    let before = snapshot(&outside)?;
    let backend = rooted(tmp.path())?;
    let tree = tmp.path().join("tree");
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut rounds = 0_u32;
    while Instant::now() < deadline || rounds < 200 {
        rounds += 1;
        // `d` is a directory and `x` a link to `outside`; the two names are
        // exchanged atomically (RENAME_EXCHANGE) as fast as possible, so the
        // walk keeps finding a directory by lstat that is a link by the time
        // it is opened.
        fs::create_dir_all(tree.join("d"))?;
        fs::write(tree.join("d/inner"), b"x")?;
        symlink(&outside, tree.join("x"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let swapper = {
            let stop = Arc::clone(&stop);
            let (d, x) = (tree.join("d"), tree.join("x"));
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = rustix::fs::renameat_with(
                        rustix::fs::CWD,
                        &d,
                        rustix::fs::CWD,
                        &x,
                        rustix::fs::RenameFlags::EXCHANGE,
                    );
                }
            })
        };
        let _ = backend.remove_tree(&rp("/tree")?, &Cancel::new());
        stop.store(true, Ordering::Relaxed);
        joined(swapper)?;
        let _ = backend.remove_tree(&rp("/tree")?, &Cancel::new());
        assert_eq!(
            snapshot(&outside)?,
            before,
            "round {rounds}: a link exchanged in was followed"
        );
        assert!(fs::symlink_metadata(&tree).is_err(), "round {rounds}");
    }
    Ok(())
}
