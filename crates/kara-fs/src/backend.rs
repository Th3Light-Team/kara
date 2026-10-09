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
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use kara_core::FileEntry;
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

/// Like [`io_error`] for lookups: a path under a non-directory does not exist.
fn lookup_error(error: io::Error, path: &RemotePath) -> BackendError {
    if error.kind() == io::ErrorKind::NotADirectory {
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
            TrashError::RefusedSpecialPath { .. } => {
                backend_error(BackendErrorKind::Other, arg, io::ErrorKind::InvalidInput)
            }
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
                // Either `dir` is a file, or something above it is: only a
                // stat of the path itself tells them apart.
                return Err(match fs::symlink_metadata(&local) {
                    Ok(_) => io_error(error, dir),
                    Err(_) => lookup_error(error, dir),
                });
            }
            Err(error) => return Err(io_error(error, dir)),
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
        // O_NONBLOCK so that a FIFO without a writer cannot hang the worker; the
        // kind is checked with fstat on the descriptor that was opened.
        let opened = OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NONBLOCK.bits().cast_signed())
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
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error, path)),
        }
        let (file, temp_path) = create_temp(&final_path, name, path)?;
        Ok(Box::new(LocalWriteSession {
            target: path.clone(),
            final_path,
            temp_path,
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
        fs::DirBuilder::new()
            .mode(0o777)
            .create(self.to_local(path))
            .map_err(|error| io_error(error, path))
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
        fs::symlink_metadata(&source).map_err(|error| lookup_error(error, from))?;
        if from == to {
            return Ok(());
        }
        if to.starts_with(from) {
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
            Err(error) => return Err(io_error(error, to)),
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
            Err(errno) => Err(io_error(io::Error::from(errno), to)),
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

/// Creates the hidden temporary sibling of `final_path`.
fn create_temp(
    final_path: &Path,
    name: &str,
    target: &RemotePath,
) -> Result<(File, PathBuf), BackendError> {
    let directory = final_path.parent().unwrap_or(Path::new("/"));
    let mut last = None;
    for _ in 0..TEMP_ATTEMPTS {
        let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_path = directory.join(temp_name(name, serial));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o666)
            .open(&temp_path)
        {
            Ok(file) => return Ok((file, temp_path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(io_error(error, target)),
        }
    }
    let error = last.unwrap_or_else(|| io::Error::from(io::ErrorKind::AlreadyExists));
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
    final_path: PathBuf,
    temp_path: PathBuf,
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
            self.done = true;
            self.file = None;
            let _ = fs::remove_file(&self.temp_path);
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
        if self.replace {
            match fs::rename(&self.temp_path, &self.final_path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::IsADirectory => {
                    return Err(BackendError::new(
                        BackendErrorKind::AlreadyExists,
                        Some(self.target.clone()),
                    ));
                }
                Err(error) => return Err(io_error(error, &self.target)),
            }
        } else {
            match trash::rename_noreplace(&self.temp_path, &self.final_path) {
                Ok(()) => {}
                Err(rustix::io::Errno::EXIST) => {
                    return Err(BackendError::new(
                        BackendErrorKind::AlreadyExists,
                        Some(self.target.clone()),
                    ));
                }
                Err(errno) => return Err(io_error(io::Error::from(errno), &self.target)),
            }
        }
        self.done = true;
        Ok(())
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
        self.done = true;
        match fs::remove_file(&self.temp_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(error, &self.target)),
        }
    }
}

impl Drop for LocalWriteSession {
    fn drop(&mut self) {
        self.discard();
    }
}
