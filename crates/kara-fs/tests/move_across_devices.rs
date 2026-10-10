//! `transfer::move_to` between two filesystems: copy everything, and only then
//! delete the source. A copy that fails part-way must leave the source whole.
//!
//! Needs a destination on another device: a tmpfs /tmp (CI), /dev/shm, or a
//! tmpfs this user may mount. Without any of them the tests say so and pass.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use kara_fs::transfer::{ConflictPolicy, move_to};
use tempfile::TempDir;

/// A tempdir on the disk filesystem that holds the build.
fn disk_tempdir() -> io::Result<TempDir> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(&base)?;
    tempfile::tempdir_in(base)
}

/// Unmounts on drop, so a failing assertion never leaves a mount behind.
struct Mounted(PathBuf);

impl Drop for Mounted {
    fn drop(&mut self) {
        let _ = Command::new("umount").arg(&self.0).status();
    }
}

/// A directory on another device than `here`, plus the guards that keep it.
struct Elsewhere {
    dir: PathBuf,
    _mount: Option<Mounted>,
    _tmp: TempDir,
}

fn device(path: &Path) -> io::Result<u64> {
    Ok(fs::metadata(path)?.dev())
}

fn elsewhere(here: &Path) -> io::Result<Option<Elsewhere>> {
    let dev = device(here)?;
    for base in [std::env::temp_dir(), PathBuf::from("/dev/shm")] {
        if let Ok(tmp) = tempfile::tempdir_in(&base)
            && device(tmp.path())? != dev
        {
            return Ok(Some(Elsewhere {
                dir: tmp.path().to_path_buf(),
                _mount: None,
                _tmp: tmp,
            }));
        }
    }
    let tmp = disk_tempdir()?;
    let at = tmp.path().join("mnt");
    fs::create_dir(&at)?;
    let mounted = Command::new("mount")
        .args(["-t", "tmpfs", "-o", "size=4m", "kara-test"])
        .arg(&at)
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !mounted {
        return Ok(None);
    }
    Ok(Some(Elsewhere {
        dir: at.clone(),
        _mount: Some(Mounted(at)),
        _tmp: tmp,
    }))
}

/// Every path under `root` with its content (`None` for anything but a file).
fn snapshot(root: &Path) -> io::Result<BTreeMap<PathBuf, Option<Vec<u8>>>> {
    let mut out = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            let meta = fs::symlink_metadata(&path)?;
            let rel = path.strip_prefix(root).map_err(io::Error::other)?.to_path_buf();
            if meta.is_dir() {
                pending.push(path);
                out.insert(rel, None);
            } else if meta.is_file() {
                out.insert(rel, Some(fs::read(&path)?));
            } else {
                out.insert(rel, None);
            }
        }
    }
    Ok(out)
}

fn source_tree(base: &Path) -> io::Result<PathBuf> {
    let tree = base.join("tree");
    fs::create_dir_all(tree.join("sub"))?;
    fs::write(tree.join("a.txt"), b"first")?;
    fs::write(tree.join("sub/inner.txt"), b"precious")?;
    Ok(tree)
}

#[test]
fn a_move_across_devices_copies_then_removes_the_source() -> io::Result<()> {
    let src = disk_tempdir()?;
    let Some(dst) = elsewhere(src.path())? else {
        eprintln!("skipped: no second filesystem available");
        return Ok(());
    };
    let tree = source_tree(src.path())?;
    let before = snapshot(&tree)?;

    let done = move_to(&tree, &dst.dir, ConflictPolicy::Fail).map_err(io::Error::other)?;
    assert_eq!(done.destination, dst.dir.join("tree"));
    assert_eq!(done.bytes_copied, Some(13), "bytes were copied, not renamed");
    assert!(fs::symlink_metadata(&tree).is_err(), "the source is gone");
    assert_eq!(snapshot(&dst.dir.join("tree"))?, before);
    Ok(())
}

#[test]
fn a_copy_that_fails_part_way_never_deletes_the_source() -> io::Result<()> {
    let src = disk_tempdir()?;
    let Some(dst) = elsewhere(src.path())? else {
        eprintln!("skipped: no second filesystem available");
        return Ok(());
    };
    let tree = source_tree(src.path())?;
    // A socket cannot be copied as a file: open(2) refuses it with ENXIO.
    let _listener = UnixListener::bind(tree.join("sub/sock"))?;
    let before = snapshot(&tree)?;

    let outcome = move_to(&tree, &dst.dir, ConflictPolicy::Fail);
    assert!(outcome.is_err(), "the copy of a socket must fail: {outcome:?}");
    assert_eq!(
        snapshot(&tree)?,
        before,
        "a move must never delete what it failed to copy"
    );
    Ok(())
}

#[test]
fn an_overwrite_blocked_part_way_never_deletes_the_source() -> io::Result<()> {
    let src = disk_tempdir()?;
    let Some(dst) = elsewhere(src.path())? else {
        eprintln!("skipped: no second filesystem available");
        return Ok(());
    };
    let tree = source_tree(src.path())?;
    // On the destination `sub` is a file, so `sub/inner.txt` cannot be written.
    fs::create_dir(dst.dir.join("tree"))?;
    fs::write(dst.dir.join("tree/sub"), b"in the way")?;
    let before = snapshot(&tree)?;

    let outcome = move_to(&tree, &dst.dir, ConflictPolicy::Overwrite);
    assert!(outcome.is_err(), "writing under a file must fail: {outcome:?}");
    assert_eq!(
        snapshot(&tree)?,
        before,
        "a move must never delete what it failed to copy"
    );
    Ok(())
}
