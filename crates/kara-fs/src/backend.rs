//! `LocalBackend`: the local filesystem behind [`kara_vfs::Backend`].
//!
//! Contract slug: `localbackend-kara-fs-implements-kara-vfs-backend-step-2`.
//! Design: `docs/remote-backends.md`, «Order of work», step 2.
//!
//! The existing code stays the implementation: listing goes through
//! [`crate::listing::describe`], the no-replace rename is the one the trash
//! already uses, and `remove_tree` is [`crate::trash::delete_permanently`].
//! Writes go to a hidden temporary sibling that is fsynced and renamed.
//!
//! Mapping between local paths and [`RemotePath`] is lexical: the backend has a
//! root (`"/"` for [`LocalBackend::system`]) and a `RemotePath` names the root
//! joined with its segments, byte for byte.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use kara_core::{EntryKind, FileEntry};
use rustix::fs::AtFlags;
use kara_vfs::{
    Backend, BackendError, BackendErrorKind, Cancel, Capabilities, Listing, RemotePath,
    RemotePathError, WriteSession,
};

use crate::listing::{bare_entry, describe};
use crate::trash::{self, Flow, TrashError, TrashObserver};

/// The longest file name the kernel accepts, in bytes.
const NAME_MAX: usize = 255;
/// Suffix that marks a write in progress; the name also starts with a dot.
const PART_SUFFIX: &str = ".kara-part";
/// How many times a colliding temporary name is retried.
const TEMP_ATTEMPTS: u32 = 100;

/// Makes every temporary name of the process unique, together with the pid.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The local filesystem as a [`Backend`], rooted at a directory.
#[derive(Debug, Clone)]
pub struct LocalBackend {
    root: PathBuf,
}

/// Why a local path has no [`RemotePath`] in a given [`LocalBackend`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocalPathError {
    #[error("local path is not absolute")]
    NotAbsolute,
    #[error("local path is outside the backend root")]
    OutsideRoot,
    #[error(transparent)]
    Path(#[from] RemotePathError),
}

/// An error of `kind` about `path`, with an `io::Error` of `source` as cause.
fn backend_error(kind: BackendErrorKind, path: &RemotePath, source: io::ErrorKind) -> BackendError {
    BackendError::new(kind, Some(path.clone())).with_source(io::Error::from(source))
}

/// Maps an io error through the shared table, naming `path`.
fn io_error(error: io::Error, path: &RemotePath) -> BackendError {
    BackendError::from_io(error, Some(path))
}

/// ENOTDIR or ELOOP: something on the way is not a directory, or is a link
/// that resolves nowhere (a cycle is a broken link, as `stat` reports it).
fn unresolvable(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotADirectory
        || error.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error())
}

/// Like [`io_error`] for lookups: a path under a non-directory, or through a
/// broken link, does not exist.
fn lookup_error(error: io::Error, path: &RemotePath) -> BackendError {
    if unresolvable(&error) {
        return BackendError::new(BackendErrorKind::NotFound, Some(path.clone()))
            .with_source(error);
    }
    io_error(error, path)
}

/// Like [`io_error`] for an operation that creates `local`. ENOTDIR (or ELOOP)
/// means a non-directory (or a broken link) on the way: when the parent itself
/// is a file the answer is `Other` (not a directory), but when the parent does
/// not resolve at all, the parent is missing, which is `NotFound` as for any
/// other missing parent.
fn create_error(error: io::Error, path: &RemotePath, local: &Path) -> BackendError {
    if unresolvable(&error)
        && local
            .parent()
            .is_some_and(|parent| fs::metadata(parent).is_err())
    {
        return BackendError::new(BackendErrorKind::NotFound, Some(path.clone()))
            .with_source(error);
    }
    io_error(error, path)
}

/// Stops a permanent delete when the token is cancelled.
struct CancelObserver<'a> {
    cancel: &'a Cancel,
}

impl TrashObserver for CancelObserver<'_> {
    fn on_bytes(&mut self, copied: u64, total: Option<u64>) -> Flow {
        let _ = (copied, total);
        if self.cancel.is_cancelled() {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

impl LocalBackend {
    /// What a local POSIX volume offers (contract cb_02). `trash` is true even
    /// though the trait has no trash method: `kara-ops` routes Supr on a local
    /// location to `kara_fs::trash` (contract dec_01).
    pub const CAPABILITIES: Capabilities = Capabilities {
        trash: true,
        atomic_rename: true,
        server_side_copy: false,
        real_directories: true,
        posix_permissions: true,
        symlinks: true,
        watch: true,
        undo_rename: true,
        undo_move: true,
    };

    /// The whole system, rooted at `"/"`. No I/O, cannot fail.
    #[must_use]
    pub fn system() -> LocalBackend {
        LocalBackend {
            root: PathBuf::from("/"),
        }
    }

    /// A backend rooted at `root`, which must be an existing directory.
    ///
    /// The root is a path prefix, **not a sandbox**: a symlink inside it that
    /// points outside is followed by every operation. Do not use it to confine
    /// untrusted paths.
    pub fn with_root(root: &Path) -> Result<LocalBackend, BackendError> {
        if !root.is_absolute() {
            return Err(BackendError::new(BackendErrorKind::Other, None)
                .with_source(io::Error::from(io::ErrorKind::InvalidInput)));
        }
        let top = RemotePath::root();
        let metadata = fs::metadata(root).map_err(|error| io_error(error, &top))?;
        if !metadata.is_dir() {
            return Err(backend_error(
                BackendErrorKind::Other,
                &top,
                io::ErrorKind::NotADirectory,
            ));
        }
        Ok(LocalBackend {
            root: root.to_path_buf(),
        })
    }

    /// The root, exactly as given.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The local path a [`RemotePath`] names: lexical, no I/O.
    #[must_use]
    pub fn to_local(&self, path: &RemotePath) -> PathBuf {
        let mut local = self.root.clone();
        for segment in path.segments() {
            local.push(segment);
        }
        local
    }

    /// The [`RemotePath`] of a local path under the root: lexical, no I/O.
    pub fn to_remote(&self, local: &Path) -> Result<RemotePath, LocalPathError> {
        if !local.is_absolute() {
            return Err(LocalPathError::NotAbsolute);
        }
        let rest = local
            .strip_prefix(&self.root)
            .map_err(|_| LocalPathError::OutsideRoot)?;
        let mut remote = RemotePath::root();
        for component in rest.components() {
            match component {
                Component::Normal(name) => {
                    if name.as_bytes().contains(&0) {
                        return Err(RemotePathError::ContainsNul.into());
                    }
                    let segment = name.to_str().ok_or(RemotePathError::NotUtf8)?;
                    remote = remote.join(segment)?;
                }
                Component::ParentDir => {
                    return Err(RemotePathError::DotSegment {
                        raw: local.to_string_lossy().into_owned(),
                    }
                    .into());
                }
                Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            }
        }
        Ok(remote)
    }

    /// Describes a child found while listing `dir`, collecting its failures.
    fn list_entry(&self, dir: &RemotePath, local: PathBuf, listing: &mut Listing) {
        let name_is_utf8 = local.file_name().is_some_and(|name| name.to_str().is_some());
        if !name_is_utf8 {
            // The name cannot be addressed through the trait; show it anyway.
            let entry = describe(&local).unwrap_or_else(|_| bare_entry(&local));
            listing.entries.push(entry);
            listing.errors.push(backend_error(
                BackendErrorKind::Other,
                dir,
                io::ErrorKind::InvalidData,
            ));
            return;
        }
        match describe(&local) {
            Ok(entry) => listing.entries.push(entry),
            Err(failure) => {
                listing.entries.push(bare_entry(&local));
                let child = self.to_remote(&local).unwrap_or_else(|_| dir.clone());
                listing.errors.push(io_error(failure.source, &child));
            }
        }
    }

    /// Maps the failure of a permanent delete to the path it is about.
    fn trash_error(&self, error: TrashError, arg: &RemotePath) -> BackendError {
        let about = |local: &Path| self.to_remote(local).unwrap_or_else(|_| arg.clone());
        match error {
            TrashError::NotFound { path, source }
            | TrashError::PermissionDenied { path, source }
            | TrashError::Io { path, source } => io_error(source, &about(&path)),
            // A nested mount point names itself, so the caller sees which
            // directory stopped the walk; any other refusal is about `arg`.
            TrashError::RefusedSpecialPath { path, .. } => backend_error(
                BackendErrorKind::Other,
                &about(&path),
                io::ErrorKind::InvalidInput,
            ),
            TrashError::Cancelled => BackendError::new(BackendErrorKind::Cancelled, Some(arg.clone())),
            other => BackendError::new(BackendErrorKind::Other, Some(arg.clone())).with_source(other),
        }
    }
}

impl Backend for LocalBackend {
    fn capabilities(&self) -> Capabilities {
        LocalBackend::CAPABILITIES
    }

    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        let cancelled = || BackendError::new(BackendErrorKind::Cancelled, Some(dir.clone()));
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let local = self.to_local(dir);
        let reader = match fs::read_dir(&local) {
            Ok(reader) => reader,
            Err(error) if error.kind() == io::ErrorKind::NotADirectory => {
                // Either `dir` is a file, or something above it (or above
                // the target of a link) is: only a stat that follows the path
                // as read_dir did tells them apart. A link that resolves
                // nowhere is broken, and listing it is NotFound.
                return Err(match fs::metadata(&local) {
                    Ok(_) => io_error(error, dir),
                    Err(_) => lookup_error(error, dir),
                });
            }
            Err(error) => return Err(lookup_error(error, dir)),
        };
        let mut listing = Listing::default();
        for item in reader {
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
            match item {
                Ok(found) => self.list_entry(dir, found.path(), &mut listing),
                Err(error) => listing.errors.push(io_error(error, dir)),
            }
        }
        Ok(listing)
    }

    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        let mut entry =
            describe(&self.to_local(path)).map_err(|failure| lookup_error(failure.source, path))?;
        if path.is_root() {
            // A root given as a symlink to a directory is that directory: the
            // drive is what the link leads to, not the link.
            if entry.is_symlink {
                let target = fs::metadata(&self.root).map_err(|error| io_error(error, path))?;
                entry.kind = if target.is_dir() {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                };
                entry.is_symlink = false;
                entry.symlink_broken = false;
                entry.size = if target.is_dir() {
                    None
                } else {
                    Some(target.len())
                };
                entry.modified = target.modified().ok();
                entry.created = target.created().ok();
                entry.accessed = target.accessed().ok();
            }
            // The drive root is not named after the local folder that backs it.
            entry.name = "/".into();
            entry.display = String::from("/");
            entry.is_hidden = false;
            entry.location = None;
        }
        Ok(entry)
    }

    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn Read + Send>, BackendError> {
        let local = self.to_local(path);
        // Only a regular file is ever opened. Opening anything else can act on
        // it: a FIFO wakes up a writer blocked in open(2), a tape device
        // rewinds, a terminal can become the controlling one. So the kind is
        // checked on the path first, and opening is left for regular files.
        let found = fs::metadata(&local).map_err(|error| lookup_error(error, path))?;
        refuse_non_file(&found, path)?;
        // O_NONBLOCK so that a FIFO swapped in after the check still cannot
        // hang the worker; the kind is checked again with fstat on the
        // descriptor that was opened, which is the one that counts.
        let opened = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOCTTY)
                    .bits()
                    .cast_signed(),
            )
            .open(&local);
        let mut file = match opened {
            Ok(file) => file,
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::NXIO.raw_os_error()) => {
                // A socket: it exists but cannot be read as a file.
                return Err(match fs::metadata(&local) {
                    Ok(_) => BackendError::new(BackendErrorKind::Unsupported, Some(path.clone())),
                    Err(_) => lookup_error(error, path),
                });
            }
            Err(error) => return Err(lookup_error(error, path)),
        };
        let metadata = file.metadata().map_err(|error| io_error(error, path))?;
        refuse_non_file(&metadata, path)?;
        // A regular file it is: from here on reads block as they normally do.
        let flags =
            rustix::fs::fcntl_getfl(&file).map_err(|errno| io_error(errno.into(), path))?;
        rustix::fs::fcntl_setfl(&file, flags - rustix::fs::OFlags::NONBLOCK)
            .map_err(|errno| io_error(errno.into(), path))?;
        if from > metadata.len() {
            return Err(backend_error(
                BackendErrorKind::Other,
                path,
                io::ErrorKind::InvalidInput,
            ));
        }
        if from > 0 {
            file.seek(SeekFrom::Start(from))
                .map_err(|error| io_error(error, path))?;
        }
        Ok(Box::new(LocalReader {
            file,
            path: path.clone(),
        }))
    }

    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        let _ = size_hint;
        let Some(name) = path.file_name() else {
            // The root is a directory.
            return Err(BackendError::new(
                BackendErrorKind::AlreadyExists,
                Some(path.clone()),
            ));
        };
        let final_path = self.to_local(path);
        match fs::symlink_metadata(&final_path) {
            Ok(existing) => {
                if existing.is_dir() || !replace {
                    return Err(BackendError::new(
                        BackendErrorKind::AlreadyExists,
                        Some(path.clone()),
                    ));
                }
                refuse_read_only(&existing, path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(create_error(error, path, &final_path)),
        }
        let (file, dir, temp_name) = create_temp(&final_path, name, path)?;
        Ok(Box::new(LocalWriteSession {
            target: path.clone(),
            dir,
            final_name: name.to_owned(),
            temp_name,
            file: Some(file),
            replace,
            poisoned: None,
            done: false,
        }))
    }

    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        if path.is_root() {
            return Err(BackendError::new(
                BackendErrorKind::AlreadyExists,
                Some(path.clone()),
            ));
        }
        let local = self.to_local(path);
        fs::DirBuilder::new()
            .mode(0o777)
            .create(&local)
            .map_err(|error| create_error(error, path, &local))
    }

    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        for root in [from, to] {
            if root.is_root() {
                return Err(backend_error(
                    BackendErrorKind::Other,
                    root,
                    io::ErrorKind::InvalidInput,
                ));
            }
        }
        let source = self.to_local(from);
        let destination = self.to_local(to);
        let moved = fs::symlink_metadata(&source).map_err(|error| lookup_error(error, from))?;
        // Two spellings of one directory entry (a link on the way to one of
        // them leads to the other): renaming it onto itself does nothing.
        if from == to || same_entry(&source, &destination) {
            return Ok(());
        }
        // Only a directory can be moved into itself, and only to a parent that
        // exists; below a file or a missing or dangling parent the destination
        // checks report it. A link on the way to `to` can lead into `from` as
        // well, and this is checked before whether `to` exists, so it must be
        // resolved here.
        let parent_exists = destination
            .parent()
            .is_some_and(|parent| fs::metadata(parent).is_ok_and(|found| found.is_dir()));
        if moved.is_dir()
            && parent_exists
            && (to.starts_with(from) || resolves_inside(&source, &destination))
        {
            return Err(backend_error(
                BackendErrorKind::Other,
                to,
                io::ErrorKind::InvalidInput,
            ));
        }
        match fs::symlink_metadata(&destination) {
            Ok(_) => {
                return Err(BackendError::new(
                    BackendErrorKind::AlreadyExists,
                    Some(to.clone()),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(create_error(error, to, &destination)),
        }
        match trash::rename_noreplace(&source, &destination) {
            Ok(()) => Ok(()),
            Err(rustix::io::Errno::EXIST) => Err(BackendError::new(
                BackendErrorKind::AlreadyExists,
                Some(to.clone()),
            )),
            Err(rustix::io::Errno::XDEV) => Err(io_error(
                io::Error::from(rustix::io::Errno::XDEV),
                from,
            )),
            Err(errno) => Err(rename_failure(errno, from, to, &source)),
        }
    }

    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        if path.is_root() {
            return Err(backend_error(
                BackendErrorKind::Other,
                path,
                io::ErrorKind::InvalidInput,
            ));
        }
        let local = self.to_local(path);
        let metadata = fs::symlink_metadata(&local).map_err(|error| lookup_error(error, path))?;
        let removed = if metadata.is_dir() {
            fs::remove_dir(&local)
        } else {
            fs::remove_file(&local)
        };
        removed.map_err(|error| lookup_error(error, path))
    }

    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::new(
                BackendErrorKind::Cancelled,
                Some(path.clone()),
            ));
        }
        if path.is_root() {
            return Err(backend_error(
                BackendErrorKind::Other,
                path,
                io::ErrorKind::InvalidInput,
            ));
        }
        let local = self.to_local(path);
        fs::symlink_metadata(&local).map_err(|error| lookup_error(error, path))?;
        let mut observer = CancelObserver { cancel };
        trash::delete_permanently(&local, &mut observer)
            .map(|_| ())
            .map_err(|error| self.trash_error(error, path))
    }

    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        let _ = to;
        Err(BackendError::new(
            BackendErrorKind::Unsupported,
            Some(from.clone()),
        ))
    }
}

/// `open_read` reads regular files only: a directory is `Other` with an
/// `IsADirectory` source, anything else (FIFO, device, socket) `Unsupported`.
fn refuse_non_file(metadata: &fs::Metadata, path: &RemotePath) -> Result<(), BackendError> {
    if metadata.is_dir() {
        return Err(backend_error(
            BackendErrorKind::Other,
            path,
            io::ErrorKind::IsADirectory,
        ));
    }
    if !metadata.is_file() {
        return Err(BackendError::new(
            BackendErrorKind::Unsupported,
            Some(path.clone()),
        ));
    }
    Ok(())
}

/// Replacing a read-only regular file is refused, as writing it in place would be.
fn refuse_read_only(existing: &fs::Metadata, target: &RemotePath) -> Result<(), BackendError> {
    if existing.is_file() && existing.permissions().mode() & 0o222 == 0 {
        return Err(BackendError::new(
            BackendErrorKind::PermissionDenied,
            Some(target.clone()),
        ));
    }
    Ok(())
}

/// Flushes the directory entry of a file just renamed into place. Without it a
/// power cut can lose the new name while a later unlink of the source persists,
/// which is how a move would lose data. Filesystems that cannot fsync a
/// directory answer EINVAL or ENOTSUP and are let through.
fn sync_directory(directory: &OwnedFd) -> io::Result<()> {
    match rustix::fs::fsync(directory) {
        Ok(()) | Err(rustix::io::Errno::INVAL | rustix::io::Errno::NOTSUP) => Ok(()),
        Err(errno) => Err(errno.into()),
    }
}

/// Whether two paths name the same directory entry once the links on the way
/// to each are followed: the same name in the same directory.
fn same_entry(a: &Path, b: &Path) -> bool {
    match (a.parent(), a.file_name(), b.parent(), b.file_name()) {
        (Some(a_dir), Some(a_name), Some(b_dir), Some(b_name)) if a_name == b_name => {
            matches!(
                (fs::canonicalize(a_dir), fs::canonicalize(b_dir)),
                (Ok(x), Ok(y)) if x == y
            )
        }
        _ => false,
    }
}

/// Whether `destination`, with the links on the way to it followed, lies inside
/// the directory `source`. `false` when either does not resolve: the checks
/// that follow report that.
fn resolves_inside(source: &Path, destination: &Path) -> bool {
    let (Some(parent), Some(name)) = (destination.parent(), destination.file_name()) else {
        return false;
    };
    match (fs::canonicalize(source), fs::canonicalize(parent)) {
        (Ok(source), Ok(parent)) => parent.join(name).starts_with(source),
        _ => false,
    }
}

/// Which path a failed `rename` is about: the source if it vanished or its
/// directory cannot be written, otherwise the destination.
fn rename_failure(
    errno: rustix::io::Errno,
    from: &RemotePath,
    to: &RemotePath,
    source: &Path,
) -> BackendError {
    use rustix::io::Errno;
    let about_source = match errno {
        Errno::NOENT => fs::symlink_metadata(source).is_err(),
        Errno::ACCESS | Errno::PERM | Errno::ROFS => source.parent().is_some_and(|dir| {
            rustix::fs::access(dir, rustix::fs::Access::WRITE_OK).is_err()
        }),
        _ => false,
    };
    io_error(io::Error::from(errno), if about_source { from } else { to })
}

/// Opens the directory of `final_path` and creates the hidden temporary in
/// it. The session keeps that descriptor: the temporary is renamed, synced and
/// removed relative to it, so the file lands in the directory resolved here
/// even if a link on the way is replaced meanwhile (by this very write, too).
fn create_temp(
    final_path: &Path,
    name: &str,
    target: &RemotePath,
) -> Result<(File, OwnedFd, String), BackendError> {
    use rustix::fs::{Mode, OFlags};
    let directory = final_path.parent().unwrap_or(Path::new("/"));
    let dir = rustix::fs::open(
        directory,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|errno| create_error(errno.into(), target, final_path))?;
    let mut last = None;
    for _ in 0..TEMP_ATTEMPTS {
        let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = temp_name(name, serial);
        match rustix::fs::openat(
            &dir,
            temp.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o666),
        ) {
            Ok(fd) => return Ok((File::from(fd), dir, temp)),
            Err(rustix::io::Errno::EXIST) => last = Some(rustix::io::Errno::EXIST),
            Err(errno) => return Err(create_error(errno.into(), target, final_path)),
        }
    }
    let error = io::Error::from(last.unwrap_or(rustix::io::Errno::EXIST));
    Err(io_error(error, target))
}

/// `.<name>.<pid>.<serial>.kara-part`, with `name` cut to fit `NAME_MAX`.
fn temp_name(name: &str, serial: u64) -> String {
    let tail = format!(".{}.{serial}{PART_SUFFIX}", std::process::id());
    let room = NAME_MAX.saturating_sub(1 + tail.len());
    let mut cut = name.len().min(room);
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    let kept = name.get(..cut).unwrap_or("");
    format!(".{kept}{tail}")
}

/// A file opened for reading; every error carries the path asked for.
struct LocalReader {
    file: File,
    path: RemotePath,
}

/// Wraps an io error as one that `BackendError::from_io` can unwrap again.
/// An interruption passes through so callers keep retrying it.
fn wrap_io(error: io::Error, path: &RemotePath) -> io::Error {
    if error.kind() == io::ErrorKind::Interrupted {
        return error;
    }
    io::Error::from(BackendError::from_io(error, Some(path)))
}

impl Read for LocalReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf).map_err(|error| wrap_io(error, &self.path))
    }
}

/// An upload in progress: bytes go to a hidden temporary sibling that is
/// renamed over the final name by `finish`.
struct LocalWriteSession {
    target: RemotePath,
    /// The directory the file lands in, opened by `begin_write`.
    dir: OwnedFd,
    final_name: String,
    temp_name: String,
    file: Option<File>,
    replace: bool,
    poisoned: Option<BackendErrorKind>,
    /// The temporary is gone (renamed or removed): `Drop` has nothing to do.
    done: bool,
}

impl LocalWriteSession {
    fn poisoned_error(&self, kind: BackendErrorKind) -> io::Error {
        io::Error::from(BackendError::new(kind, Some(self.target.clone())))
    }

    /// Removes the temporary, best effort; used on the failure paths where the
    /// original error is the one worth reporting.
    fn discard(&mut self) {
        if !self.done {
            self.file = None;
            // `done` only once the temporary is really gone, so `Drop` retries a
            // removal that failed instead of leaving a `.kara-part` behind.
            match rustix::fs::unlinkat(&self.dir, self.temp_name.as_str(), AtFlags::empty()) {
                Ok(()) | Err(rustix::io::Errno::NOENT) => self.done = true,
                Err(_) => {}
            }
        }
    }

    fn commit(&mut self) -> Result<(), BackendError> {
        if let Some(kind) = self.poisoned {
            return Err(BackendError::new(kind, Some(self.target.clone())));
        }
        let file = self.file.take().ok_or_else(|| {
            BackendError::new(BackendErrorKind::Other, Some(self.target.clone()))
        })?;
        file.sync_all()
            .map_err(|error| io_error(error, &self.target))?;
        drop(file);
        let (temp, name) = (self.temp_name.as_str(), self.final_name.as_str());
        let renamed = if self.replace {
            rustix::fs::renameat(&self.dir, temp, &self.dir, name)
        } else {
            trash::rename_noreplace_at(self.dir.as_fd(), temp, self.dir.as_fd(), name)
        };
        match renamed {
            Ok(()) => {}
            // A directory took the name meanwhile (replace), or anything did
            // (no replace): either way the name is taken.
            Err(rustix::io::Errno::EXIST | rustix::io::Errno::ISDIR) => {
                return Err(BackendError::new(
                    BackendErrorKind::AlreadyExists,
                    Some(self.target.clone()),
                ));
            }
            Err(errno) => return Err(io_error(io::Error::from(errno), &self.target)),
        }
        self.done = true;
        sync_directory(&self.dir).map_err(|error| io_error(error, &self.target))
    }
}

impl Write for LocalWriteSession {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(kind) = self.poisoned {
            return Err(self.poisoned_error(kind));
        }
        let Some(file) = self.file.as_mut() else {
            return Err(self.poisoned_error(BackendErrorKind::Other));
        };
        match file.write(buf) {
            Ok(written) => Ok(written),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
            Err(error) => {
                let failure = BackendError::from_io(error, Some(&self.target));
                self.poisoned = Some(failure.kind);
                Err(io::Error::from(failure))
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(kind) = self.poisoned {
            return Err(self.poisoned_error(kind));
        }
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        match file.flush() {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
            Err(error) => {
                let failure = BackendError::from_io(error, Some(&self.target));
                self.poisoned = Some(failure.kind);
                Err(io::Error::from(failure))
            }
        }
    }
}

impl WriteSession for LocalWriteSession {
    fn finish(mut self: Box<Self>) -> Result<(), BackendError> {
        let outcome = self.commit();
        if outcome.is_err() {
            self.discard();
        }
        outcome
    }

    fn abort(mut self: Box<Self>) -> Result<(), BackendError> {
        self.file = None;
        if self.done {
            return Ok(());
        }
        match rustix::fs::unlinkat(&self.dir, self.temp_name.as_str(), AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => {
                self.done = true;
                Ok(())
            }
            // Not done: `Drop` tries once more.
            Err(errno) => Err(io_error(errno.into(), &self.target)),
        }
    }
}

impl Drop for LocalWriteSession {
    fn drop(&mut self) {
        self.discard();
    }
}
