//! Recursive permanent delete through directory descriptors.
//!
//! A path-based walk (`lstat` a child, then `read_dir` it by path) has two
//! holes: a child directory swapped for a symlink between the two calls is
//! followed, so the walk deletes whatever the link points at; and nothing
//! stops it from descending into a filesystem mounted inside the tree.
//!
//! This walk closes both. Every directory is opened relative to the
//! descriptor of its parent with `O_DIRECTORY | O_NOFOLLOW`, so a symlink in
//! its place makes the open fail (and is then unlinked as a link) instead of
//! being followed; every unlink names an entry relative to the descriptor that
//! was opened, never a path that could be re-resolved; and each opened
//! directory is `fstat`ed and compared with the device of the root of the
//! walk: a directory on another device is a nested mount point and the walk
//! stops there with [`RefusalReason::MountPoint`] instead of entering it.
//!
//! The walk is iterative and holds one descriptor per level of depth, so the
//! depth it can reach is bounded by the descriptor limit, not by the stack.

use std::ffi::{OsStr, OsString};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
use rustix::io::Errno;

use super::error::classify_io_error;
use super::{Flow, RefusalReason, TrashError, TrashObserver};

/// Flags every directory of the walk is opened with.
fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

/// Whether an entry whose filesystem is `entry_dev` lies outside the
/// filesystem of the root of the walk, `root_dev`: then it is a mount point.
pub(crate) fn crosses_device<D: PartialEq>(root_dev: D, entry_dev: D) -> bool {
    root_dev != entry_dev
}

fn io_failure(path: &Path, errno: Errno) -> TrashError {
    classify_io_error(path, errno.into())
}

fn mount_point(path: PathBuf) -> TrashError {
    TrashError::RefusedSpecialPath {
        path,
        reason: RefusalReason::MountPoint,
    }
}

/// Opens the directory `name` inside `parent` without following a link.
fn open_dir_at(parent: BorrowedFd<'_>, name: &OsStr) -> Result<OwnedFd, Errno> {
    rustix::fs::openat(parent, name, dir_flags(), Mode::empty())
}

/// The names in the directory open as `fd`, without `.` and `..`.
///
/// They are read in full before anything is removed, so unlinking while the
/// directory stream is open can never make the walk skip an entry.
fn names_in(fd: &OwnedFd, path: &Path) -> Result<Vec<OsString>, TrashError> {
    let mut dir = Dir::read_from(fd.as_fd()).map_err(|errno| io_failure(path, errno))?;
    let mut names = Vec::new();
    while let Some(entry) = dir.read() {
        let entry = entry.map_err(|errno| io_failure(path, errno))?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            names.push(OsStr::from_bytes(name).to_os_string());
        }
    }
    Ok(names)
}

/// One open directory of the walk and what is left to remove inside it.
struct Frame {
    fd: OwnedFd,
    path: PathBuf,
    /// The name under the directory of the previous frame; `None` for the root.
    name: Option<OsString>,
    pending: Vec<OsString>,
}

/// Unlinks the non-directory `name` inside `parent`. An entry that is already
/// gone counts as removed by someone else, not as a failure.
fn unlink_entry(parent: BorrowedFd<'_>, name: &OsStr, path: &Path) -> Result<bool, TrashError> {
    match rustix::fs::unlinkat(parent, name, AtFlags::empty()) {
        Ok(()) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(errno) => Err(io_failure(path, errno)),
    }
}

/// Deletes `path`, described by `metadata` (an `lstat`), and everything below
/// it. Returns how many entries were removed. `observer.on_bytes` is asked
/// once per entry before it is removed, and [`Flow::Cancel`] stops the walk
/// with what is left still in place.
pub(crate) fn delete_tree(
    path: &Path,
    metadata: &std::fs::Metadata,
    observer: &mut dyn TrashObserver,
) -> Result<u64, TrashError> {
    let Some(name) = path.file_name() else {
        return Err(TrashError::RefusedSpecialPath {
            path: path.to_path_buf(),
            reason: RefusalReason::Empty,
        });
    };
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let parent_fd = rustix::fs::openat(
        rustix::fs::CWD,
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|errno| io_failure(parent, errno))?;

    let is_dir = metadata.is_dir() && !metadata.file_type().is_symlink();
    if !is_dir {
        return match rustix::fs::unlinkat(&parent_fd, name, AtFlags::empty()) {
            Ok(()) => Ok(1),
            Err(errno) => Err(io_failure(path, errno)),
        };
    }
    let root_fd = match open_dir_at(parent_fd.as_fd(), name) {
        Ok(fd) => fd,
        // Swapped for a link or a file since the lstat: it goes as what it is now.
        Err(Errno::LOOP | Errno::NOTDIR) => {
            return match rustix::fs::unlinkat(&parent_fd, name, AtFlags::empty()) {
                Ok(()) => Ok(1),
                Err(errno) => Err(io_failure(path, errno)),
            };
        }
        Err(errno) => return Err(io_failure(path, errno)),
    };
    let root_dev = rustix::fs::fstat(&root_fd)
        .map_err(|errno| io_failure(path, errno))?
        .st_dev;
    let parent_dev = rustix::fs::fstat(&parent_fd)
        .map_err(|errno| io_failure(parent, errno))?
        .st_dev;
    if crosses_device(parent_dev, root_dev) {
        return Err(mount_point(path.to_path_buf()));
    }

    let mut removed: u64 = 0;
    let pending = names_in(&root_fd, path)?;
    let mut stack = vec![Frame {
        fd: root_fd,
        path: path.to_path_buf(),
        name: None,
        pending,
    }];
    while let Some(top) = stack.last_mut() {
        let Some(child) = top.pending.pop() else {
            // Emptied: remove it from the directory one level up.
            let Some(done) = stack.pop() else {
                break;
            };
            let Some(done_name) = done.name else {
                break;
            };
            drop(done.fd);
            let Some(up) = stack.last() else {
                break;
            };
            match rustix::fs::unlinkat(&up.fd, &done_name, AtFlags::REMOVEDIR) {
                Ok(()) => removed = removed.saturating_add(1),
                Err(Errno::NOENT) => {}
                Err(errno) => return Err(io_failure(&done.path, errno)),
            }
            continue;
        };
        let child_path = top.path.join(&child);
        if observer.on_bytes(removed, None) == Flow::Cancel {
            return Err(TrashError::Cancelled);
        }
        let stat = match rustix::fs::statat(&top.fd, &child, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(Errno::NOENT) => continue,
            Err(errno) => return Err(io_failure(&child_path, errno)),
        };
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
            if unlink_entry(top.fd.as_fd(), &child, &child_path)? {
                removed = removed.saturating_add(1);
            }
            continue;
        }
        let child_fd = match open_dir_at(top.fd.as_fd(), &child) {
            Ok(fd) => fd,
            Err(Errno::LOOP | Errno::NOTDIR) => {
                if unlink_entry(top.fd.as_fd(), &child, &child_path)? {
                    removed = removed.saturating_add(1);
                }
                continue;
            }
            Err(Errno::NOENT) => continue,
            Err(errno) => return Err(io_failure(&child_path, errno)),
        };
        let child_dev = rustix::fs::fstat(&child_fd)
            .map_err(|errno| io_failure(&child_path, errno))?
            .st_dev;
        if crosses_device(root_dev, child_dev) {
            return Err(mount_point(child_path));
        }
        let pending = names_in(&child_fd, &child_path)?;
        stack.push(Frame {
            fd: child_fd,
            path: child_path,
            name: Some(child),
            pending,
        });
    }
    drop(stack);
    rustix::fs::unlinkat(&parent_fd, name, AtFlags::REMOVEDIR)
        .map_err(|errno| io_failure(path, errno))?;
    Ok(removed.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::crosses_device;

    #[test]
    fn the_same_device_is_not_a_mount_point() {
        assert!(!crosses_device(42_u64, 42_u64));
    }

    #[test]
    fn another_device_is_a_mount_point_in_either_direction() {
        assert!(crosses_device(42_u64, 43_u64));
        assert!(crosses_device(43_u64, 42_u64));
        assert!(crosses_device(0_u64, u64::MAX));
    }
}
