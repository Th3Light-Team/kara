//! `remove_tree` / `delete_permanently` walk through directory descriptors:
//! links inside the tree are removed as links and never followed (also when
//! one is swapped in mid-walk), nested mount points are refused, deep trees
//! beyond `PATH_MAX` go, and a cancelled walk leaves a consistent remainder.

mod local_backend_support;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use kara_fs::trash::{Flow, RefusalReason, TrashError, TrashObserver, delete_permanently};
use kara_vfs::{Backend, BackendErrorKind, Cancel};
use local_backend_support::{
    Node, assert_err, chmod, err_of, permissions_enforced, rooted, rp, snapshot, tmp_tempdir,
};

/// A tree with files at several levels and links of every flavour pointing
/// out of it: to a directory, to a file, relative, absolute, to `..`, broken.
fn tree_with_outward_links(base: &Path, outside: &Path) -> io::Result<PathBuf> {
    let tree = base.join("tree");
    fs::create_dir_all(tree.join("a/b/c"))?;
    fs::write(tree.join("top"), b"top")?;
    fs::write(tree.join("a/one"), b"one")?;
    fs::write(tree.join("a/b/two"), b"two")?;
    fs::write(tree.join("a/b/c/three"), b"three")?;
    symlink(outside, tree.join("abs-dir"))?;
    symlink(outside.join("keep.txt"), tree.join("a/abs-file"))?;
    symlink("../../../outside", tree.join("a/b/rel-dir"))?;
    symlink("..", tree.join("a/b/c/up"))?;
    symlink("/nowhere/at/all", tree.join("a/broken"))?;
    Ok(tree)
}

fn outside_dir(base: &Path) -> io::Result<PathBuf> {
    let outside = base.join("outside");
    fs::create_dir_all(outside.join("inner"))?;
    fs::write(outside.join("keep.txt"), b"keep")?;
    fs::write(outside.join("inner/deep.txt"), b"deep")?;
    Ok(outside)
}

#[test]
fn links_inside_the_tree_are_removed_and_their_targets_survive() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let outside = outside_dir(tmp.path())?;
    let before = snapshot(&outside)?;
    tree_with_outward_links(tmp.path(), &outside)?;
    let backend = rooted(tmp.path())?;

    backend.remove_tree(&rp("/tree")?, &Cancel::new())?;

    assert!(fs::symlink_metadata(tmp.path().join("tree")).is_err());
    assert_eq!(snapshot(&outside)?, before, "nothing outside the tree changes");
    Ok(())
}

#[test]
fn delete_permanently_never_follows_a_link_either() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let outside = outside_dir(tmp.path())?;
    let before = snapshot(&outside)?;
    let tree = tree_with_outward_links(tmp.path(), &outside)?;

    let removed = delete_permanently(&tree, &mut kara_fs::trash::NullObserver)
        .map_err(io::Error::other)?;

    // tree, top, a, one, b, two, c, three + 5 links.
    assert_eq!(removed, 13, "every entry is counted once, links included");
    assert!(fs::symlink_metadata(&tree).is_err());
    assert_eq!(snapshot(&outside)?, before);
    Ok(())
}

#[test]
fn a_symlink_to_a_directory_is_removed_as_a_link() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let outside = outside_dir(tmp.path())?;
    let before = snapshot(&outside)?;
    symlink(&outside, tmp.path().join("link"))?;
    let backend = rooted(tmp.path())?;

    backend.remove_tree(&rp("/link")?, &Cancel::new())?;

    assert!(fs::symlink_metadata(tmp.path().join("link")).is_err());
    assert_eq!(snapshot(&outside)?, before);
    Ok(())
}

#[test]
fn a_tree_deeper_than_path_max_is_removed() -> io::Result<()> {
    use rustix::fs::{CWD, Mode, OFlags, mkdirat, openat};

    let tmp = tmp_tempdir()?;
    let backend = rooted(tmp.path())?;
    let segment = "d".repeat(40);
    let depth = 400; // about 16 KiB of path: four times PATH_MAX.
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    mkdirat(CWD, tmp.path().join("deep"), Mode::from_raw_mode(0o755))?;
    let mut fd = openat(CWD, tmp.path().join("deep"), flags, Mode::empty())?;
    for level in 0..depth {
        if level % 50 == 0 {
            rustix::fs::openat(
                &fd,
                "file",
                OFlags::WRONLY | OFlags::CREATE | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o644),
            )?;
        }
        mkdirat(&fd, segment.as_str(), Mode::from_raw_mode(0o755))?;
        fd = openat(&fd, segment.as_str(), flags, Mode::empty())?;
    }
    drop(fd);

    backend.remove_tree(&rp("/deep")?, &Cancel::new())?;

    assert!(fs::symlink_metadata(tmp.path().join("deep")).is_err());
    Ok(())
}

/// Cancels on the `limit`-th entry.
struct CancelAfter {
    seen: u64,
    limit: u64,
}

impl TrashObserver for CancelAfter {
    fn on_bytes(&mut self, copied: u64, total: Option<u64>) -> Flow {
        let _ = (copied, total);
        self.seen += 1;
        if self.seen >= self.limit {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn wide_tree(root: &Path) -> io::Result<BTreeMap<PathBuf, Node>> {
    for dir in 0..6 {
        let sub = root.join(format!("dir{dir}"));
        fs::create_dir_all(sub.join("inner"))?;
        for file in 0..8 {
            fs::write(sub.join(format!("f{file}")), format!("{dir}/{file}").repeat(50))?;
            fs::write(sub.join("inner").join(format!("g{file}")), vec![dir as u8; 777])?;
        }
    }
    snapshot(root)
}

/// What is left after a cancel is a subset of the original tree, node for node:
/// no file is truncated, no directory replaced, nothing new appears.
fn assert_consistent_remainder(
    root: &Path,
    before: &BTreeMap<PathBuf, Node>,
) -> io::Result<usize> {
    let after = snapshot(root)?;
    for (path, node) in &after {
        assert_eq!(
            before.get(path),
            Some(node),
            "{path:?} is not what it was before the walk"
        );
    }
    Ok(after.len())
}

#[test]
fn a_cancelled_walk_stops_and_leaves_a_consistent_remainder() -> io::Result<()> {
    let total_entries = {
        let tmp = tmp_tempdir()?;
        wide_tree(&tmp.path().join("t"))?.len()
    };
    for limit in [1_u64, 2, 7, 30, 61, 100] {
        let tmp = tmp_tempdir()?;
        let root = tmp.path().join("t");
        let before = wide_tree(&root)?;
        let mut observer = CancelAfter { seen: 0, limit };
        let result = delete_permanently(&root, &mut observer);
        assert!(
            matches!(result, Err(TrashError::Cancelled)),
            "limit {limit}: {result:?}"
        );
        assert_eq!(observer.seen, limit, "limit {limit}: the walk stops at once");
        assert!(root.is_dir(), "limit {limit}: the root survives a cancel");
        let left = assert_consistent_remainder(&root, &before)?;
        // Each answer before the cancel removed at most one entry.
        let removed = total_entries - left;
        assert!(
            removed < usize::try_from(limit).unwrap_or(usize::MAX),
            "limit {limit}: {removed} entries removed after {limit} checks"
        );
    }
    Ok(())
}

#[test]
fn a_cancel_racing_remove_tree_leaves_either_nothing_or_a_consistent_tree() -> io::Result<()> {
    for delay_us in [0_u64, 50, 200, 1000] {
        let tmp = tmp_tempdir()?;
        let root = tmp.path().join("t");
        let before = wide_tree(&root)?;
        let backend = rooted(tmp.path())?;
        let cancel = Cancel::new();
        let remote = rp("/t")?;
        let canceller = {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_micros(delay_us));
                cancel.cancel();
            })
        };
        let result = backend.remove_tree(&remote, &cancel);
        let _ = canceller.join();
        match result {
            Ok(()) => assert!(fs::symlink_metadata(&root).is_err()),
            Err(error) => {
                assert_err(&error, BackendErrorKind::Cancelled, &remote, "racing cancel");
                assert_consistent_remainder(&root, &before)?;
            }
        }
    }
    Ok(())
}

#[test]
fn a_directory_swapped_for_a_link_mid_walk_is_never_followed() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let outside = outside_dir(tmp.path())?;
    let before = snapshot(&outside)?;
    let backend = rooted(tmp.path())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut rounds = 0;
    while Instant::now() < deadline || rounds < 20 {
        rounds += 1;
        let tree = tmp.path().join("tree");
        for dir in 0..4 {
            let sub = tree.join(format!("sub{dir}"));
            fs::create_dir_all(&sub)?;
            for file in 0..30 {
                fs::write(sub.join(format!("f{file}")), b"x")?;
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let swapper = {
            let stop = Arc::clone(&stop);
            let tree = tree.clone();
            let outside = outside.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    for dir in 0..4 {
                        let sub = tree.join(format!("sub{dir}"));
                        let aside = tree.join(format!("aside{dir}"));
                        if fs::rename(&sub, &aside).is_ok() {
                            let _ = symlink(&outside, &sub);
                        }
                    }
                }
            })
        };
        let _ = backend.remove_tree(&rp("/tree")?, &Cancel::new());
        stop.store(true, Ordering::Relaxed);
        let _ = swapper.join();
        let _ = backend.remove_tree(&rp("/tree")?, &Cancel::new());
        assert_eq!(
            snapshot(&outside)?,
            before,
            "round {rounds}: a swapped-in link was followed"
        );
    }
    Ok(())
}

#[test]
fn an_unreadable_subdirectory_is_reported_by_its_remote_path() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let locked = tmp.path().join("tree/locked");
    fs::create_dir_all(&locked)?;
    fs::write(locked.join("f"), b"f")?;
    chmod(&locked, 0o555)?;
    let enforced = permissions_enforced(&locked);
    chmod(&locked, 0o000)?;
    if !enforced {
        chmod(&locked, 0o755)?;
        eprintln!("skipped: permissions are not enforced for this user");
        return Ok(());
    }
    let backend = rooted(tmp.path())?;
    let error = err_of(
        backend.remove_tree(&rp("/tree")?, &Cancel::new()),
        "remove_tree over an unreadable directory",
    );
    chmod(&locked, 0o755)?;
    assert_err(
        &error,
        BackendErrorKind::PermissionDenied,
        &rp("/tree/locked")?,
        "unreadable subdirectory",
    );
    Ok(())
}

/// Unmounts on drop, so a failing assertion never leaves a mount behind.
struct Mounted(PathBuf);

impl Drop for Mounted {
    fn drop(&mut self) {
        let _ = Command::new("umount").arg(&self.0).status();
    }
}

/// Mounts a fresh tmpfs on `at`, or `None` where this user may not mount.
fn mount_tmpfs(at: &Path) -> Option<Mounted> {
    let status = Command::new("mount")
        .args(["-t", "tmpfs", "-o", "size=1m", "kara-test"])
        .arg(at)
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    status.success().then(|| Mounted(at.to_path_buf()))
}

#[test]
fn a_nested_mount_point_is_refused_and_never_entered() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let tree = tmp.path().join("tree");
    let mount = tree.join("sub/mnt");
    fs::create_dir_all(&mount)?;
    fs::write(tree.join("sub/file"), b"file")?;
    let Some(mounted) = mount_tmpfs(&mount) else {
        eprintln!("skipped: cannot mount a tmpfs here (needs root)");
        return Ok(());
    };
    fs::write(mount.join("on-the-mount"), b"precious")?;
    fs::create_dir(mount.join("dir"))?;
    let backend = rooted(tmp.path())?;

    let error = err_of(
        backend.remove_tree(&rp("/tree")?, &Cancel::new()),
        "remove_tree across a mount point",
    );
    assert_err(
        &error,
        BackendErrorKind::Other,
        &rp("/tree/sub/mnt")?,
        "the mount point names itself",
    );
    assert_eq!(fs::read(mount.join("on-the-mount"))?, b"precious");
    assert!(mount.join("dir").is_dir());

    let direct = delete_permanently(&tree, &mut kara_fs::trash::NullObserver);
    match direct {
        Err(TrashError::RefusedSpecialPath { path, reason }) => {
            assert_eq!(path, mount);
            assert_eq!(reason, RefusalReason::MountPoint);
        }
        other => panic!("delete_permanently across a mount: {other:?}"),
    }
    assert_eq!(fs::read(mount.join("on-the-mount"))?, b"precious");
    drop(mounted);
    backend.remove_tree(&rp("/tree")?, &Cancel::new())?;
    Ok(())
}

#[test]
fn the_mount_point_itself_is_refused_as_the_root_of_the_walk() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let mount = tmp.path().join("mnt");
    fs::create_dir(&mount)?;
    let Some(mounted) = mount_tmpfs(&mount) else {
        eprintln!("skipped: cannot mount a tmpfs here (needs root)");
        return Ok(());
    };
    fs::write(mount.join("f"), b"f")?;
    let backend = rooted(tmp.path())?;
    let error = err_of(
        backend.remove_tree(&rp("/mnt")?, &Cancel::new()),
        "remove_tree of a mount point",
    );
    assert_eq!(error.kind, BackendErrorKind::Other);
    assert_eq!(error.path, Some(rp("/mnt")?));
    assert_eq!(fs::read(mount.join("f"))?, b"f");
    drop(mounted);
    Ok(())
}
