//! `SftpBackend`: an SFTP server behind [`kara_vfs::Backend`].
//!
//! The semantics are `LocalBackend`'s (`kara-fs/src/backend.rs`), checked by
//! the same conformance suite and by a differential test against it:
//!
//! - errors name the caller's [`RemotePath`], never a server path or a temporary;
//! - `rename` never replaces anything, and a directory is never moved into
//!   its own subtree (links on the way included, resolved with `realpath`);
//! - writes go to a hidden temporary sibling that is renamed into place by
//!   `finish`; see [`super::io`];
//! - `remove_tree` walks iteratively, post-order, by the types `readdir`
//!   reports (`lstat`), so it never follows a link.
//!
//! OpenSSH folds `ENOTDIR` and `ELOOP` into `NO_SUCH_FILE` and most other
//! errnos into a bare `FAILURE`; where `LocalBackend` tells them apart by
//! errno, this backend looks (`lstat`/`stat` of the target or its parent)
//! after the failure.

use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kara_core::{EntryKind, FileEntry, MetadataBag, MetadataKey, MetadataValue};
use kara_vfs::{
    Backend, BackendError, BackendErrorKind, Cancel, Capabilities, Listing, RemotePath,
    WriteSession,
};
use russh_sftp::protocol::{FileAttributes, OpenFlags};

use super::error::{Fail, error};
use super::io::{SftpReader, SftpWriteSession, temp_name};
use super::session::Session;

const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_IFLNK: u32 = 0o120_000;
/// How many times a colliding temporary name is retried.
const TEMP_ATTEMPTS: u32 = 100;

/// What a mode says the entry is. `None` when the server sent no mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Type {
    Dir,
    File,
    Link,
    Special,
}

fn type_of(attrs: &FileAttributes) -> Option<Type> {
    let mode = attrs.permissions?;
    Some(match mode & S_IFMT {
        S_IFDIR => Type::Dir,
        S_IFREG => Type::File,
        S_IFLNK => Type::Link,
        _ => Type::Special,
    })
}

fn is_dir(attrs: &FileAttributes) -> bool {
    type_of(attrs) == Some(Type::Dir)
}

fn time(seconds: Option<u32>) -> Option<SystemTime> {
    seconds.map(|s| UNIX_EPOCH + Duration::from_secs(u64::from(s)))
}

/// A listing entry from the entry's own attributes (`lstat`) and, for a link,
/// its target's (`stat`, `None` when the link is broken). Same rules as
/// `kara_fs::listing::describe`: the kind is the target's, the size and the
/// times are the entry's own, a directory has no size.
fn describe(name: &str, own: &FileAttributes, target: Option<&FileAttributes>) -> FileEntry {
    let is_symlink = type_of(own) == Some(Type::Link);
    let resolved = if is_symlink { target } else { Some(own) };
    let kind = match resolved {
        Some(attrs) if is_dir(attrs) => EntryKind::Directory,
        _ => EntryKind::File,
    };
    let mut extra = MetadataBag::new();
    if let Some(attrs) = resolved {
        if let Some(mode) = attrs.permissions {
            extra.insert(
                MetadataKey::Custom("posix.mode".into()),
                MetadataValue::Unsigned(u64::from(mode & 0o7777)),
            );
        }
        if let Some(uid) = attrs.uid {
            extra.insert(
                MetadataKey::Custom("posix.uid".into()),
                MetadataValue::Unsigned(u64::from(uid)),
            );
        }
        if let Some(gid) = attrs.gid {
            extra.insert(
                MetadataKey::Custom("posix.gid".into()),
                MetadataValue::Unsigned(u64::from(gid)),
            );
        }
    }
    FileEntry {
        name: OsString::from(name),
        display: name.to_owned(),
        kind,
        is_symlink,
        symlink_broken: is_symlink && target.is_none(),
        is_hidden: name.starts_with('.'),
        size: match kind {
            EntryKind::Directory => None,
            EntryKind::File => own.size,
        },
        modified: time(own.mtime),
        created: None,
        accessed: time(own.atime),
        type_label: None,
        location: None,
        extra,
    }
}

/// An SFTP drive. Cheap to share; every method blocks the calling thread.
pub struct SftpBackend {
    pub(crate) session: Arc<Session>,
    /// The server folder `RemotePath` "/" stands for.
    root: String,
    caps: Capabilities,
    /// `user@host:port`, for `Debug` only.
    label: String,
}

impl fmt::Debug for SftpBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SftpBackend")
            .field("server", &self.label)
            .field("root", &self.root)
            .field("capabilities", &self.caps)
            .finish_non_exhaustive()
    }
}

impl SftpBackend {
    pub(crate) fn new(session: Session, root: String, label: String) -> SftpBackend {
        let atomic = session.posix_rename;
        let caps = Capabilities {
            trash: false,
            atomic_rename: atomic,
            server_side_copy: false,
            real_directories: true,
            posix_permissions: true,
            symlinks: true,
            watch: false,
            undo_rename: atomic,
            undo_move: atomic,
        };
        SftpBackend {
            session: Arc::new(session),
            root,
            caps,
            label,
        }
    }

    /// Whether the session is still up. Once it is not, every call answers
    /// `Unavailable` at once; reconnecting is the registry's job.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        !self.session.is_closed()
    }

    /// The server path a `RemotePath` names: lexical, no I/O.
    #[must_use]
    pub fn server_path(&self, path: &RemotePath) -> String {
        if self.root == "/" {
            path.as_str().to_owned()
        } else if path.is_root() {
            self.root.clone()
        } else {
            format!("{}{}", self.root, path.as_str())
        }
    }

    fn sp(&self, path: &RemotePath) -> String {
        self.server_path(path)
    }

    /// `lstat`, the failure naming `path`.
    fn lstat(&self, path: &RemotePath) -> Result<FileAttributes, BackendError> {
        self.session.lstat(&self.sp(path)).map_err(|fail| fail.at(path))
    }

    /// Whether something (anything, a broken link too) is at `path`.
    fn exists(&self, path: &RemotePath) -> bool {
        self.session.lstat(&self.sp(path)).is_ok()
    }

    /// The error for a failed create of `path`. OpenSSH reports a parent that
    /// is a file and a missing parent alike (`NO_SUCH_FILE`); like
    /// `LocalBackend`, a parent that exists but is not a directory is `Other`.
    fn create_error(&self, fail: Fail, path: &RemotePath) -> BackendError {
        if fail.kind() != BackendErrorKind::NotFound {
            return fail.at(path);
        }
        let parent = path.parent().unwrap_or_else(RemotePath::root);
        match self.session.stat(&self.sp(&parent)) {
            Ok(attrs) if !is_dir(&attrs) => fail.with_kind(BackendErrorKind::Other, path),
            _ => fail.at(path),
        }
    }

    /// Whether two directories are the same once links are resolved.
    fn same_directory(&self, a: &RemotePath, b: &RemotePath) -> bool {
        if a == b {
            return true;
        }
        match (
            self.session.realpath(&self.sp(a)),
            self.session.realpath(&self.sp(b)),
        ) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
    }

    /// Whether `to`, with the links on the way followed, lies inside `from`.
    fn resolves_inside(&self, from: &RemotePath, to: &RemotePath) -> bool {
        let (Some(parent), Some(name)) = (to.parent(), to.file_name()) else {
            return false;
        };
        match (
            self.session.realpath(&self.sp(from)),
            self.session.realpath(&self.sp(&parent)),
        ) {
            (Ok(source), Ok(parent)) => {
                let target = if parent == "/" {
                    format!("/{name}")
                } else {
                    format!("{parent}/{name}")
                };
                target == source
                    || source == "/"
                    || target
                        .strip_prefix(source.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            }
            _ => false,
        }
    }

    /// Describes `path` given its own attributes.
    fn entry_at(&self, name: &str, path: &RemotePath, own: &FileAttributes) -> FileEntry {
        if type_of(own) == Some(Type::Link) {
            let target = self.session.stat(&self.sp(path)).ok();
            describe(name, own, target.as_ref())
        } else {
            describe(name, own, None)
        }
    }

    /// Removes one non-directory (or a link to one) or an empty directory.
    fn remove_one(&self, path: &RemotePath, own: &FileAttributes) -> Result<(), Fail> {
        if is_dir(own) {
            self.session.rmdir(&self.sp(path))
        } else {
            self.session.remove(&self.sp(path))
        }
    }

    /// Creates the hidden temporary in the server directory `dir_sp`.
    fn create_temp(
        &self,
        target: &RemotePath,
        dir_sp: &str,
        name: &str,
    ) -> Result<(String, String), BackendError> {
        let mut last = None;
        for _ in 0..TEMP_ATTEMPTS {
            let temp = temp_name(name);
            let temp_sp = if dir_sp == "/" {
                format!("/{temp}")
            } else {
                format!("{dir_sp}/{temp}")
            };
            match self.session.open(
                &temp_sp,
                OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
            ) {
                Ok(handle) => return Ok((temp_sp, handle)),
                // A leftover with the same name: try the next one.
                Err(fail) if fail.kind() == BackendErrorKind::Other && self.session.lstat(&temp_sp).is_ok() => {
                    last = Some(fail);
                }
                Err(fail) => return Err(self.create_error(fail, target)),
            }
        }
        Err(last
            .map(|fail| fail.at(target))
            .unwrap_or_else(|| error(BackendErrorKind::Other, target, io::ErrorKind::AlreadyExists)))
    }
}

impl Backend for SftpBackend {
    fn capabilities(&self) -> Capabilities {
        self.caps
    }

    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        let cancelled = || BackendError::new(BackendErrorKind::Cancelled, Some(dir.clone()));
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let entries = match self.session.read_dir(&self.sp(dir), cancel) {
            Ok(entries) => entries,
            Err(Fail::Cancelled) => return Err(cancelled()),
            Err(fail) if fail.kind() == BackendErrorKind::NotFound => {
                // Either nothing is there, or a file is (OpenSSH says
                // NO_SUCH_FILE for ENOTDIR): a stat that follows the path the
                // way opendir did tells them apart.
                return Err(match self.session.stat(&self.sp(dir)) {
                    Ok(attrs) if !is_dir(&attrs) => fail.with_kind(BackendErrorKind::Other, dir),
                    _ => fail.at(dir),
                });
            }
            Err(fail) => return Err(fail.at(dir)),
        };
        let mut listing = Listing::default();
        for found in entries {
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
            let child = match dir.join(&found.name) {
                // A name that came through lossy UTF-8 cannot be addressed.
                Ok(child) if !found.name.contains('\u{fffd}') => child,
                _ => {
                    listing.entries.push(describe(&found.name, &found.attrs, None));
                    listing.errors.push(error(
                        BackendErrorKind::Other,
                        dir,
                        io::ErrorKind::InvalidData,
                    ));
                    continue;
                }
            };
            let own = if found.attrs.permissions.is_some() {
                found.attrs
            } else {
                match self.session.lstat(&self.sp(&child)) {
                    Ok(attrs) => attrs,
                    Err(fail) if fail.kind() == BackendErrorKind::Unavailable => {
                        return Err(fail.at(dir));
                    }
                    Err(fail) => {
                        listing.entries.push(describe(&found.name, &found.attrs, None));
                        listing.errors.push(fail.at(&child));
                        continue;
                    }
                }
            };
            listing.entries.push(self.entry_at(&found.name, &child, &own));
        }
        if !self.is_connected() {
            return Err(Fail::Lost(String::from("the connection closed while listing")).at(dir));
        }
        Ok(listing)
    }

    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        if path.is_root() {
            // The drive is what its root leads to, not a link to it.
            let attrs = self
                .session
                .stat(&self.sp(path))
                .map_err(|fail| fail.at(path))?;
            let mut entry = describe("/", &attrs, None);
            entry.is_hidden = false;
            return Ok(entry);
        }
        let own = self.lstat(path)?;
        let name = path.file_name().unwrap_or("/");
        Ok(self.entry_at(name, path, &own))
    }

    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn Read + Send>, BackendError> {
        let attrs = self
            .session
            .stat(&self.sp(path))
            .map_err(|fail| fail.at(path))?;
        match type_of(&attrs) {
            Some(Type::Dir) => {
                return Err(error(BackendErrorKind::Other, path, io::ErrorKind::IsADirectory));
            }
            Some(Type::Special) => {
                return Err(BackendError::new(BackendErrorKind::Unsupported, Some(path.clone())));
            }
            Some(Type::File | Type::Link) | None => {}
        }
        if attrs.size.is_some_and(|size| from > size) {
            return Err(error(BackendErrorKind::Other, path, io::ErrorKind::InvalidInput));
        }
        let handle = self
            .session
            .open(&self.sp(path), OpenFlags::READ)
            .map_err(|fail| fail.at(path))?;
        Ok(Box::new(SftpReader::new(
            Arc::clone(&self.session),
            handle,
            from,
            path.clone(),
        )))
    }

    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        let _ = size_hint;
        let Some(name) = path.file_name() else {
            return Err(BackendError::new(BackendErrorKind::AlreadyExists, Some(path.clone())));
        };
        match self.session.lstat(&self.sp(path)) {
            Ok(existing) => {
                if is_dir(&existing) || !replace {
                    return Err(BackendError::new(
                        BackendErrorKind::AlreadyExists,
                        Some(path.clone()),
                    ));
                }
                // Replacing a read-only file is refused, as writing it in place would be.
                let read_only = type_of(&existing) == Some(Type::File)
                    && existing.permissions.is_some_and(|mode| mode & 0o222 == 0);
                if read_only {
                    return Err(BackendError::new(
                        BackendErrorKind::PermissionDenied,
                        Some(path.clone()),
                    ));
                }
            }
            Err(fail) if fail.kind() == BackendErrorKind::NotFound => {}
            Err(fail) => return Err(self.create_error(fail, path)),
        }
        // The directory is resolved once, here, as LocalBackend opens it once:
        // the file lands where the path led at begin_write, even if a link on
        // the way is replaced meanwhile (by this very write, too).
        let parent = path.parent().unwrap_or_else(RemotePath::root);
        let dir_sp = self
            .session
            .realpath(&self.sp(&parent))
            .map_err(|fail| self.create_error(fail, path))?;
        let destination = if dir_sp == "/" {
            format!("/{name}")
        } else {
            format!("{dir_sp}/{name}")
        };
        let (temp, handle) = self.create_temp(path, &dir_sp, name)?;
        Ok(Box::new(SftpWriteSession::new(
            Arc::clone(&self.session),
            path.clone(),
            temp,
            destination,
            handle,
            replace,
        )))
    }

    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        if path.is_root() {
            return Err(BackendError::new(BackendErrorKind::AlreadyExists, Some(path.clone())));
        }
        match self.session.mkdir(&self.sp(path)) {
            Ok(()) => Ok(()),
            Err(fail) if fail.kind() == BackendErrorKind::NotFound => Err(self.create_error(fail, path)),
            Err(fail) if fail.kind() == BackendErrorKind::Other && self.exists(path) => {
                Err(fail.with_kind(BackendErrorKind::AlreadyExists, path))
            }
            Err(fail) => Err(fail.at(path)),
        }
    }

    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        for root in [from, to] {
            if root.is_root() {
                return Err(error(BackendErrorKind::Other, root, io::ErrorKind::InvalidInput));
            }
        }
        let moved = self.lstat(from)?;
        // Two spellings of one directory entry: renaming it onto itself does nothing.
        if from == to {
            return Ok(());
        }
        if let (Some(a), Some(b)) = (from.parent(), to.parent())
            && from.file_name() == to.file_name()
            && self.same_directory(&a, &b)
        {
            return Ok(());
        }
        // Only a directory can go into itself, and only to a parent that is a
        // directory; links on the way to `to` count.
        let parent = to.parent().unwrap_or_else(RemotePath::root);
        let parent_is_dir = self
            .session
            .stat(&self.sp(&parent))
            .is_ok_and(|attrs| is_dir(&attrs));
        if is_dir(&moved) && parent_is_dir && (to.starts_with(from) || self.resolves_inside(from, to)) {
            return Err(error(BackendErrorKind::Other, to, io::ErrorKind::InvalidInput));
        }
        match self.session.lstat(&self.sp(to)) {
            Ok(_) => {
                return Err(BackendError::new(BackendErrorKind::AlreadyExists, Some(to.clone())));
            }
            Err(fail) if fail.kind() == BackendErrorKind::NotFound => {}
            Err(fail) => return Err(fail.at(to)),
        }
        // Plain SSH_FXP_RENAME: OpenSSH never replaces with it (link + unlink
        // for files, a stat check for the rest).
        match self.session.rename(&self.sp(from), &self.sp(to)) {
            Ok(()) => Ok(()),
            Err(fail) if fail.kind() == BackendErrorKind::NotFound => {
                if self.exists(from) {
                    Err(self.create_error(fail, to))
                } else {
                    Err(fail.at(from))
                }
            }
            Err(fail) if fail.kind() == BackendErrorKind::Other && self.exists(to) => {
                Err(fail.with_kind(BackendErrorKind::AlreadyExists, to))
            }
            Err(fail) => Err(fail.at(to)),
        }
    }

    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        if path.is_root() {
            return Err(error(BackendErrorKind::Other, path, io::ErrorKind::InvalidInput));
        }
        let own = self.lstat(path)?;
        self.remove_one(path, &own).map_err(|fail| fail.at(path))
    }

    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        let cancelled = || BackendError::new(BackendErrorKind::Cancelled, Some(path.clone()));
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        if path.is_root() {
            return Err(error(BackendErrorKind::Other, path, io::ErrorKind::InvalidInput));
        }
        let own = self.lstat(path)?;
        if !is_dir(&own) {
            return self.remove_one(path, &own).map_err(|fail| fail.at(path));
        }
        // Post-order without recursion: a directory is pushed back marked
        // «emptied» before its children, and removed when it comes up again.
        let mut stack: Vec<(RemotePath, bool)> = vec![(path.clone(), false)];
        while let Some((dir, emptied)) = stack.pop() {
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
            if emptied {
                match self.session.rmdir(&self.sp(&dir)) {
                    Ok(()) => {}
                    Err(fail) if fail.kind() == BackendErrorKind::NotFound && dir != *path => {}
                    Err(fail) => return Err(fail.at(&dir)),
                }
                continue;
            }
            let children = match self.session.read_dir(&self.sp(&dir), cancel) {
                Ok(children) => children,
                Err(Fail::Cancelled) => return Err(cancelled()),
                Err(fail) => return Err(fail.at(&dir)),
            };
            stack.push((dir.clone(), true));
            for found in children {
                if cancel.is_cancelled() {
                    return Err(cancelled());
                }
                let child = match dir.join(&found.name) {
                    Ok(child) if !found.name.contains('\u{fffd}') => child,
                    _ => {
                        return Err(error(BackendErrorKind::Other, &dir, io::ErrorKind::InvalidData));
                    }
                };
                // The type readdir reports is the entry's own (lstat): a link
                // is removed as a link and never entered.
                let own = match type_of(&found.attrs) {
                    Some(_) => found.attrs,
                    None => match self.lstat(&child) {
                        Ok(attrs) => attrs,
                        Err(e) if e.kind == BackendErrorKind::NotFound => continue,
                        Err(e) => return Err(e),
                    },
                };
                if is_dir(&own) {
                    stack.push((child, false));
                } else {
                    match self.session.remove(&self.sp(&child)) {
                        Ok(()) => {}
                        Err(fail) if fail.kind() == BackendErrorKind::NotFound => {}
                        Err(fail) => return Err(fail.at(&child)),
                    }
                }
            }
        }
        Ok(())
    }

    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        let _ = to;
        Err(BackendError::new(BackendErrorKind::Unsupported, Some(from.clone())))
    }
}
