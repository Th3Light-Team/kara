//! Papelera FreeDesktop: enviar a la papelera, restaurar y borrado permanente.
//!
//! Conveniencias cubiertas (fuente: `ground/spec/05-operaciones.md`):
//!
//! - **Enviar a la papelera** (§ «Enviar a la papelera»): la operación es
//!   rápida (`rename(2)`, sin recorrer bytes salvo entre volúmenes), registra
//!   la ruta original y la fecha en un `.trashinfo` y es deshacible.
//! - **Restaurar desde la papelera** (§ «Restaurar desde la papelera»), solo
//!   la primitiva de sistema de ficheros: recrea las carpetas intermedias y
//!   delega el conflicto de nombre en el llamador.
//! - **Eliminación permanente** (§ «Eliminación permanente (Shift+Supr)»)
//!   como salida explícita para volúmenes sin papelera; nunca se invoca sola.
//! - **Manejo de errores con reintentar / omitir / cancelar**
//!   (§ «Manejo de errores»): un fallo por elemento no aborta el lote.
//! - **Listar la papelera**, base de «Restaurar desde la papelera» y
//!   «Vaciar la papelera»: empareja `info/` con `files/` en la papelera
//!   personal y en la de cada volumen, sin abortar por un `.trashinfo`
//!   ilegible ni por una entrada desemparejada — ver [`listing`].
//!
//! Esta capa no pregunta ni confirma nada: el ajuste de confirmación y los
//! diálogos viven por encima (`ground/spec/06-contexto-power.md`).

mod dir;
mod error;
mod info;
mod listing;
mod remove;

pub use dir::{
    TrashAvailability, TrashDir, TrashKind, TrashPolicy, home_trash_dir, probe_trash,
    resolve_trash_dir,
};
pub use error::{RefusalReason, RestoreError, TrashError, TrashInfoError, UnavailableReason};
pub use info::{DeletionDate, TrashInfo, read_trash_info};
pub use listing::{
    EmptyOutcome, TrashEntry, TrashListing, delete_trash_entry, empty_trash, empty_trash_in,
    list_trash, list_trash_in,
};

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use error::classify_io_error;

/// Everything the undo stack needs to put an item back where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedItem {
    pub original_path: PathBuf,
    pub trashed_path: PathBuf,
    pub info_path: PathBuf,
    pub deletion_date: DeletionDate,
    pub kind: TrashKind,
    pub top_dir: Option<PathBuf>,
    /// `None` for a plain `rename(2)`; `Some(n)` only after an authorised
    /// cross-device copy.
    pub bytes_copied: Option<u64>,
}

/// Whether to keep going or stop, as answered by the observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Cancel,
}

/// What to do about one item that failed inside a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorDecision {
    Retry,
    Skip,
    SkipAll,
    Cancel,
}

/// What to do when the restore destination is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    Fail,
    KeepBoth,
    Overwrite,
}

/// Outcome of a batch. Never a `Result`: one failure does not abort the batch.
///
/// `trashed.len() + skipped.len() <= paths.len()`; the difference is the items
/// that were never attempted because of a cancellation.
#[derive(Debug)]
pub struct BatchOutcome {
    pub trashed: Vec<TrashedItem>,
    pub skipped: Vec<(PathBuf, TrashError)>,
    pub cancelled: bool,
}

/// Progress and error callbacks. Every method has a default so that a null
/// observer is trivial to write.
pub trait TrashObserver {
    /// Called once before each item; returning [`Flow::Cancel`] stops the
    /// batch without touching the item.
    fn on_item_start(&mut self, path: &Path, index: usize, total: usize) -> Flow {
        let _ = (path, index, total);
        Flow::Continue
    }

    /// Fine-grained byte progress. Called during an authorised cross-device
    /// copy, during [`delete_permanently`], and — once per entry, so that the
    /// walk stays cancellable — while measuring an item against
    /// `TrashPolicy::max_item_bytes`. A plain same-volume `rename(2)` with no
    /// size limit set never calls it (cb_02).
    fn on_bytes(&mut self, copied: u64, total: Option<u64>) -> Flow {
        let _ = (copied, total);
        Flow::Continue
    }

    /// Called on every failure; the answer drives retry / skip / cancel.
    fn on_error(&mut self, path: &Path, error: &TrashError) -> ErrorDecision {
        let _ = (path, error);
        ErrorDecision::Skip
    }

    /// Called after an item has been successfully trashed.
    fn on_item_done(&mut self, item: &TrashedItem) {
        let _ = item;
    }
}

/// An observer that answers every callback with its default: used whenever a
/// caller does not need progress or error handling of its own.
///
/// Public because emptying the trash needs one and the window has nowhere to
/// show progress yet; when the operations queue drives it, that queue becomes
/// the observer instead.
pub struct NullObserver;

impl TrashObserver for NullObserver {}

// ---------------------------------------------------------------------------
// Path refusals (cb_23) and other pre-flight checks that must never touch the
// filesystem beyond a handful of read-only `stat`s.
// ---------------------------------------------------------------------------

pub(crate) fn refuse_special_path(path: &Path) -> Result<(), TrashError> {
    if path.as_os_str().is_empty() {
        return Err(TrashError::RefusedSpecialPath {
            path: path.to_path_buf(),
            reason: RefusalReason::Empty,
        });
    }
    if !path.is_absolute() {
        return Err(TrashError::RefusedSpecialPath {
            path: path.to_path_buf(),
            reason: RefusalReason::Relative,
        });
    }
    if path == Path::new("/") {
        return Err(TrashError::RefusedSpecialPath {
            path: path.to_path_buf(),
            reason: RefusalReason::Root,
        });
    }
    if let Ok(trash_root) = dir::home_trash_root() {
        if trash_root.starts_with(path) {
            return Err(TrashError::RefusedSpecialPath {
                path: path.to_path_buf(),
                reason: RefusalReason::TrashAncestor,
            });
        }
        if path.starts_with(trash_root.join("files")) || path.starts_with(trash_root.join("info")) {
            return Err(TrashError::PathIsInsideTrash {
                path: path.to_path_buf(),
            });
        }
    }
    Ok(())
}

pub(crate) fn refuse_mount_point(path: &Path, metadata: &std::fs::Metadata) -> Result<(), TrashError> {
    if !metadata.is_dir() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        let parent_metadata = dir::lstat(parent)?;
        if parent_metadata.dev() != metadata.dev() {
            return Err(TrashError::RefusedSpecialPath {
                path: path.to_path_buf(),
                reason: RefusalReason::MountPoint,
            });
        }
    }
    Ok(())
}

fn check_capacity(
    path: &Path,
    metadata: &std::fs::Metadata,
    policy: &TrashPolicy,
    observer: &mut dyn TrashObserver,
) -> Result<(), TrashError> {
    if let Some(limit) = policy.max_item_bytes {
        let size = apparent_size(path, metadata, limit, observer)?;
        if size > limit {
            return Err(TrashError::ExceedsTrashCapacity {
                path: path.to_path_buf(),
                needed_bytes: size,
                available_bytes: limit,
            });
        }
    }
    Ok(())
}

/// Apparent size of `path`: the file's own length for anything that is not a
/// real directory (symlinks included, so a link is never followed), or the
/// sum of its descendants' lengths otherwise. The walk stops as soon as the
/// running total exceeds `limit`, so this never scans a whole large tree just
/// to prove it is over budget; it is only ever called when a limit is set
/// (cb_02, cb_13). Consults `observer` once per entry, exactly like
/// the permanent delete walk, so walking a large tree here is cancellable and never
/// runs unattended (cb_29).
fn apparent_size(
    path: &Path,
    metadata: &std::fs::Metadata,
    limit: u64,
    observer: &mut dyn TrashObserver,
) -> Result<u64, TrashError> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(metadata.len());
    }
    let mut total: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|source| classify_io_error(&dir, source))?;
        for entry in entries {
            let entry = entry.map_err(|source| classify_io_error(&dir, source))?;
            let child_path = entry.path();
            let child_metadata = dir::lstat(&child_path)?;
            if observer.on_bytes(total, None) == Flow::Cancel {
                return Err(TrashError::Cancelled);
            }
            total = total.saturating_add(child_metadata.len());
            if total > limit {
                return Ok(total);
            }
            if child_metadata.is_dir() && !child_metadata.file_type().is_symlink() {
                stack.push(child_path);
            }
        }
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// Name reservation (cb_08, cb_09, cb_28)
// ---------------------------------------------------------------------------

const NAME_MAX: usize = 255;
const TRASHINFO_SUFFIX: &str = ".trashinfo";

/// Splits `name` into `(stem, extension)`, where `extension` keeps its
/// leading dot. A name with no dot, or a dot only at position 0 (a dotfile),
/// has no extension.
fn split_stem_ext(name: &[u8]) -> (&[u8], &[u8]) {
    if let Some(pos) = name.iter().rposition(|&b| b == b'.')
        && pos > 0
        && pos < name.len() - 1
    {
        return (&name[..pos], &name[pos..]);
    }
    (name, &[])
}

/// Builds the candidate basename for the given attempt: the plain name on the
/// first attempt, `stem.N.ext` afterwards, truncating only the stem so the
/// result fits in `max_len` bytes. The budget is computed for the suffix and
/// extension *first*, so distinct attempts never collapse into the same
/// truncated candidate and the extension is never the part that gets cut
/// (cb_28). Returns `None` only when the suffix and extension alone already
/// exceed `max_len`.
fn candidate_name(stem: &[u8], ext: &[u8], attempt: u32, max_len: usize) -> Option<Vec<u8>> {
    let mut suffix = Vec::new();
    if attempt > 1 {
        suffix.push(b'.');
        suffix.extend_from_slice(attempt.to_string().as_bytes());
    }
    let reserved = suffix.len().checked_add(ext.len())?;
    if reserved > max_len {
        return None;
    }
    let stem_budget = max_len - reserved;
    let mut out = Vec::with_capacity(max_len);
    if stem.len() <= stem_budget {
        out.extend_from_slice(stem);
    } else {
        out.extend_from_slice(&stem[..stem_budget]);
    }
    out.extend_from_slice(&suffix);
    out.extend_from_slice(ext);
    Some(out)
}

// ---------------------------------------------------------------------------
// Moving a single item (cb_01..cb_29 pieces that concern trash_one)
// ---------------------------------------------------------------------------

fn trash_one_core(
    path: &Path,
    policy: &TrashPolicy,
    observer: &mut dyn TrashObserver,
) -> Result<TrashedItem, TrashError> {
    refuse_special_path(path)?;
    let metadata = dir::lstat(path)?;
    refuse_mount_point(path, &metadata)?;
    check_capacity(path, &metadata, policy, observer)?;

    let trash_dir = resolve_trash_dir(path, policy)?;
    if path.starts_with(&trash_dir.files) || path.starts_with(&trash_dir.info) {
        return Err(TrashError::PathIsInsideTrash {
            path: path.to_path_buf(),
        });
    }

    place_in_trash(path, &metadata, &trash_dir, policy, observer)
}

fn place_in_trash(
    path: &Path,
    metadata: &std::fs::Metadata,
    trash_dir: &TrashDir,
    policy: &TrashPolicy,
    observer: &mut dyn TrashObserver,
) -> Result<TrashedItem, TrashError> {
    let file_name = match path.file_name() {
        Some(name) => name,
        None => {
            return Err(TrashError::RefusedSpecialPath {
                path: path.to_path_buf(),
                reason: RefusalReason::Empty,
            });
        }
    };
    let (stem, ext) = split_stem_ext(file_name.as_bytes());

    // The UTC offset used to build a `DeletionDate` is injected via the
    // policy (`policy.utc_offset_seconds`), never read from the machine's
    // timezone by this crate: `kara-ops` is expected to fill in the real
    // local offset, which keeps the conversion deterministic and testable.
    let deletion_date = match DeletionDate::from_system_time_local(
        SystemTime::now(),
        policy.utc_offset_seconds,
    ) {
        Ok(date) => date,
        Err(_) => {
            return Err(TrashError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other("system clock is not representable"),
            });
        }
    };

    let max_base_len = NAME_MAX.saturating_sub(TRASHINFO_SUFFIX.len());
    let max_attempts: u32 = 1_000;
    let mut attempt: u32 = 1;

    loop {
        if attempt > max_attempts {
            return Err(TrashError::NameExhausted {
                path: path.to_path_buf(),
                attempts: attempt - 1,
            });
        }
        let candidate = match candidate_name(stem, ext, attempt, max_base_len) {
            Some(candidate) => candidate,
            None => {
                return Err(TrashError::NameExhausted {
                    path: path.to_path_buf(),
                    attempts: attempt - 1,
                });
            }
        };
        let candidate_name = OsStr::from_bytes(&candidate).to_os_string();

        let mut info_name = candidate_name.clone();
        info_name.push(TRASHINFO_SUFFIX);
        let info_path = trash_dir.info.join(&info_name);
        let target_path = trash_dir.files.join(&candidate_name);
        let info_dir_before = std::fs::metadata(&trash_dir.info).ok();

        let mut open_options = std::fs::OpenOptions::new();
        open_options.write(true).create_new(true);
        let mut info_file = match open_options.open(&info_path) {
            Ok(file) => file,
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                attempt += 1;
                continue;
            }
            Err(source) => return Err(classify_io_error(&info_path, source)),
        };
        let release_reservation = |info_path: &Path| {
            let _ = std::fs::remove_file(info_path);
            if let Some(before) = &info_dir_before {
                restore_dir_times(&trash_dir.info, before);
            }
        };

        let record = TrashInfo {
            original_path: path.to_path_buf(),
            deletion_date,
        };
        let content = match record.serialize(trash_dir.top_dir.as_deref()) {
            Ok(text) => text,
            Err(_) => {
                drop(info_file);
                release_reservation(&info_path);
                return Err(TrashError::InfoWrite {
                    info_path,
                    source: std::io::Error::other("could not serialize the trash record"),
                });
            }
        };
        if let Err(source) = info_file.write_all(content.as_bytes()) {
            drop(info_file);
            release_reservation(&info_path);
            return Err(TrashError::InfoWrite { info_path, source });
        }
        drop(info_file);

        // NOREPLACE closes the race the `.trashinfo` reservation alone does
        // not cover: without it, `rename(2)` silently clobbers a regular
        // file that already sits at `target_path` (an orphaned `files/`
        // entry left by a purged `.trashinfo`, another trash implementation,
        // a crash mid-operation). EEXIST here means the same collision
        // `.trashinfo`'s O_EXCL already guards against, so it is handled the
        // same way: release the reservation and try the next candidate name.
        match rename_noreplace(path, &target_path) {
            Ok(()) => {
                return Ok(TrashedItem {
                    original_path: path.to_path_buf(),
                    trashed_path: target_path,
                    info_path,
                    deletion_date,
                    kind: trash_dir.kind,
                    top_dir: trash_dir.top_dir.clone(),
                    bytes_copied: None,
                });
            }
            Err(rustix::io::Errno::EXIST) => {
                release_reservation(&info_path);
                attempt += 1;
                continue;
            }
            Err(source) if source == rustix::io::Errno::XDEV => {
                // EXDEV: the item lives on a different device than the trash
                // that `resolve_trash_dir` picked for it (this can happen
                // when the caller's own device detection disagrees with the
                // kernel's, e.g. bind mounts).
                if !policy.allow_cross_device_copy {
                    release_reservation(&info_path);
                    return Err(TrashError::CrossDevice {
                        path: path.to_path_buf(),
                        trash_root: trash_dir.root.clone(),
                    });
                }
                if let Err(error) =
                    check_free_space_for_copy(path, metadata, &trash_dir.root, policy, observer)
                {
                    release_reservation(&info_path);
                    return Err(error);
                }
                match copy_across_devices(path, &target_path, metadata, observer) {
                    Ok(bytes) => {
                        // Only once the copy is complete do we remove the
                        // original (cb_24): losing it earlier would leave
                        // nothing to fall back on if the copy had failed
                        // halfway, and keeping it after a successful copy
                        // would leave the item duplicated instead of moved.
                        if let Err(error) = remove_original_tree(path) {
                            remove_path_recursive(&target_path);
                            release_reservation(&info_path);
                            return Err(error);
                        }
                        return Ok(TrashedItem {
                            original_path: path.to_path_buf(),
                            trashed_path: target_path,
                            info_path,
                            deletion_date,
                            kind: trash_dir.kind,
                            top_dir: trash_dir.top_dir.clone(),
                            bytes_copied: Some(bytes),
                        });
                    }
                    Err(error) => {
                        remove_path_recursive(&target_path);
                        release_reservation(&info_path);
                        return Err(error);
                    }
                }
            }
            Err(source) => {
                release_reservation(&info_path);
                return Err(classify_io_error(path, source.into()));
            }
        }
    }
}

/// `rename(2)` that refuses to clobber an existing destination, reporting the
/// collision as `EEXIST`.
///
/// `RENAME_NOREPLACE` is the only way to make that atomic, but `renameat2` is
/// not universally available: kernels older than 3.15 answer `ENOSYS`, and
/// several filesystems a file manager routinely meets — NFS, some FUSE mounts,
/// older overlayfs — answer `EINVAL` or `EOPNOTSUPP` for the flag even on a
/// modern kernel. Failing there would mean "send to trash" simply does not
/// work on those mounts, which is worse than the degradation this fallback
/// accepts: a `stat` followed by a plain rename, which is what every other
/// FreeDesktop implementation does unconditionally. The window between the two
/// is narrow and, for the trash, already guarded on the other side by the
/// `.trashinfo` name reservation (cb_08).
pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> Result<(), rustix::io::Errno> {
    rename_noreplace_at(rustix::fs::CWD, from, rustix::fs::CWD, to)
}

/// [`rename_noreplace`] with each name relative to a directory descriptor, so
/// the rename happens in the directories that were opened, whatever their
/// paths mean by now.
pub(crate) fn rename_noreplace_at<P: rustix::path::Arg + Copy>(
    from_dir: std::os::fd::BorrowedFd<'_>,
    from: P,
    to_dir: std::os::fd::BorrowedFd<'_>,
    to: P,
) -> Result<(), rustix::io::Errno> {
    match rustix::fs::renameat_with(
        from_dir,
        from,
        to_dir,
        to,
        rustix::fs::RenameFlags::NOREPLACE,
    ) {
        Err(rustix::io::Errno::NOSYS)
        | Err(rustix::io::Errno::INVAL)
        | Err(rustix::io::Errno::OPNOTSUPP) => {
            // `link` fails with EEXIST atomically, so link-then-unlink keeps the
            // no-clobber guarantee where RENAME_NOREPLACE is missing. Directories
            // and filesystems without hard links refuse it; only those fall
            // through to the check-then-rename below.
            match rustix::fs::linkat(from_dir, from, to_dir, to, rustix::fs::AtFlags::empty()) {
                Ok(()) => {
                    return match rustix::fs::unlinkat(from_dir, from, rustix::fs::AtFlags::empty())
                    {
                        Ok(()) => Ok(()),
                        Err(errno) => {
                            let _ = rustix::fs::unlinkat(to_dir, to, rustix::fs::AtFlags::empty());
                            Err(errno)
                        }
                    };
                }
                Err(rustix::io::Errno::EXIST) => return Err(rustix::io::Errno::EXIST),
                Err(_) => {}
            }
            if rustix::fs::statat(to_dir, to, rustix::fs::AtFlags::SYMLINK_NOFOLLOW).is_ok() {
                return Err(rustix::io::Errno::EXIST);
            }
            rustix::fs::renameat(from_dir, from, to_dir, to)
        }
        other => other,
    }
}

/// Restores `path`'s modification/access time to what `metadata` recorded.
/// Used after reserving-then-releasing a name in a directory (cb_09): the
/// mandated order (create the `.trashinfo` with `O_EXCL`, attempt the rename,
/// delete the `.trashinfo` again on failure) touches the directory twice, so
/// without this its `mtime` would drift even though its contents end up
/// exactly as they were.
fn restore_dir_times(path: &Path, metadata: &std::fs::Metadata) {
    let times = rustix::fs::Timestamps {
        last_access: rustix::fs::Timespec {
            tv_sec: metadata.atime(),
            tv_nsec: metadata.atime_nsec() as _,
        },
        last_modification: rustix::fs::Timespec {
            tv_sec: metadata.mtime(),
            tv_nsec: metadata.mtime_nsec() as _,
        },
    };
    let _ = rustix::fs::utimensat(rustix::fs::CWD, path, &times, rustix::fs::AtFlags::empty());
}

/// Removes `path` for real, after it has already been fully copied into the
/// trash (cb_24): a directory is removed recursively, anything else — plain
/// file or symlink — with a single unlink. Never follows the last component.
fn remove_original_tree(path: &Path) -> Result<(), TrashError> {
    let metadata = dir::lstat(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path).map_err(|source| classify_io_error(path, source))
    } else {
        std::fs::remove_file(path).map_err(|source| classify_io_error(path, source))
    }
}

/// Before starting a cross-device copy, makes sure the trash filesystem has
/// room for it: `needed_bytes` (the item's real apparent size) plus
/// `policy.free_space_margin_bytes` must fit in the free space reported for
/// `trash_root`. When free space cannot be determined the check is skipped
/// rather than blocking a copy that might well succeed.
fn check_free_space_for_copy(
    path: &Path,
    metadata: &std::fs::Metadata,
    trash_root: &Path,
    policy: &TrashPolicy,
    observer: &mut dyn TrashObserver,
) -> Result<(), TrashError> {
    let Some(available) = dir::free_bytes_on(trash_root) else {
        return Ok(());
    };
    let needed = apparent_size(path, metadata, u64::MAX, observer)?;
    let required = needed.saturating_add(policy.free_space_margin_bytes);
    if required > available {
        return Err(TrashError::ExceedsTrashCapacity {
            path: path.to_path_buf(),
            needed_bytes: needed,
            available_bytes: available,
        });
    }
    Ok(())
}

fn remove_path_recursive(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            let _ = std::fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(path);
        }
        Err(_) => {}
    }
}

/// Copies `source` into `dest` byte by byte (only reachable when the policy
/// authorises a cross-device move), reporting progress through `on_bytes` in
/// chunks no larger than 1 MiB and honouring [`Flow::Cancel`] between them.
fn copy_across_devices(
    source: &Path,
    dest: &Path,
    metadata: &std::fs::Metadata,
    observer: &mut dyn TrashObserver,
) -> Result<u64, TrashError> {
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(source)
            .map_err(|source_err| classify_io_error(source, source_err))?;
        std::os::unix::fs::symlink(&target, dest)
            .map_err(|source_err| classify_io_error(dest, source_err))?;
        return Ok(0);
    }

    if metadata.is_dir() {
        std::fs::create_dir(dest).map_err(|source_err| classify_io_error(dest, source_err))?;
        let entries = std::fs::read_dir(source)
            .map_err(|source_err| classify_io_error(source, source_err))?;
        let mut total: u64 = 0;
        for entry in entries {
            let entry = entry.map_err(|source_err| classify_io_error(source, source_err))?;
            let child_source = entry.path();
            let child_metadata = dir::lstat(&child_source)?;
            let child_dest = dest.join(entry.file_name());
            if observer.on_bytes(total, None) == Flow::Cancel {
                return Err(TrashError::Cancelled);
            }
            total = total.saturating_add(copy_across_devices(
                &child_source,
                &child_dest,
                &child_metadata,
                observer,
            )?);
        }
        copy_metadata(source, dest);
        return Ok(total);
    }

    let mut input =
        std::fs::File::open(source).map_err(|source_err| classify_io_error(source, source_err))?;
    // `create_new` refuses to clobber a file that already sits at `dest`,
    // for the same reason the final rename into `files/` uses NOREPLACE
    // (cb_08, cb_09): a pre-existing regular file at the destination must
    // never be silently overwritten by a cross-device copy.
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .map_err(|source_err| classify_io_error(dest, source_err))?;
    let total_size = metadata.len();
    let mut copied: u64 = 0;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|source_err| classify_io_error(source, source_err))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|source_err| classify_io_error(dest, source_err))?;
        copied = copied.saturating_add(read as u64);
        if observer.on_bytes(copied, Some(total_size)) == Flow::Cancel {
            return Err(TrashError::Cancelled);
        }
    }
    copy_metadata(source, dest);
    Ok(copied)
}

/// Preserves `source`'s mode and mtime on `dest` after a byte-for-byte copy
/// (cb_24: an item that crosses devices must not come back from the trash
/// with a falsified modification time). Best-effort: failing to preserve
/// metadata does not undo an otherwise-successful copy.
fn copy_metadata(source: &Path, dest: &Path) {
    if let Ok(metadata) = std::fs::symlink_metadata(source) {
        let _ = std::fs::set_permissions(dest, metadata.permissions());
        let times = rustix::fs::Timestamps {
            last_access: rustix::fs::Timespec {
                tv_sec: metadata.atime(),
                tv_nsec: metadata.atime_nsec() as _,
            },
            last_modification: rustix::fs::Timespec {
                tv_sec: metadata.mtime(),
                tv_nsec: metadata.mtime_nsec() as _,
            },
        };
        let _ = rustix::fs::utimensat(rustix::fs::CWD, dest, &times, rustix::fs::AtFlags::empty());
    }
}

/// Moves a single path to the trash. Never follows the last symlink component:
/// the link itself is trashed, not its target. Never walks the contents of a
/// directory unless the policy authorises a cross-device copy.
pub fn trash_one(path: &Path, policy: &TrashPolicy) -> Result<TrashedItem, TrashError> {
    trash_one_core(path, policy, &mut NullObserver)
}

/// Moves several paths to the trash, in the given order, reporting each one.
pub fn trash_batch(
    paths: &[PathBuf],
    policy: &TrashPolicy,
    observer: &mut dyn TrashObserver,
) -> BatchOutcome {
    let mut trashed = Vec::new();
    let mut skipped = Vec::new();
    let total = paths.len();
    let mut cancelled = false;
    let mut skip_all = false;

    'items: for (index, path) in paths.iter().enumerate() {
        if observer.on_item_start(path, index, total) == Flow::Cancel {
            cancelled = true;
            break 'items;
        }

        loop {
            match trash_one_core(path, policy, observer) {
                Ok(item) => {
                    observer.on_item_done(&item);
                    trashed.push(item);
                    continue 'items;
                }
                Err(TrashError::Cancelled) => {
                    // A cooperative cancellation surfaced from inside the
                    // operation itself (e.g. `Flow::Cancel` returned to
                    // `on_bytes` mid cross-device copy) is terminal: it is
                    // not a failure to hand to the observer's error policy,
                    // and the item was never really attempted from the
                    // batch's point of view (its own reversion already put
                    // everything back), so it does not belong in `skipped`
                    // either (cb_20, cb_24).
                    cancelled = true;
                    break 'items;
                }
                Err(error) => {
                    if skip_all {
                        skipped.push((path.clone(), error));
                        continue 'items;
                    }
                    match observer.on_error(path, &error) {
                        ErrorDecision::Retry => continue,
                        ErrorDecision::Skip => {
                            skipped.push((path.clone(), error));
                            continue 'items;
                        }
                        ErrorDecision::SkipAll => {
                            skipped.push((path.clone(), error));
                            skip_all = true;
                            continue 'items;
                        }
                        ErrorDecision::Cancel => {
                            // Unlike the internal-cancellation branch above,
                            // this item did fail for a reportable reason;
                            // the user chose to stop, but the failure that
                            // triggered the choice must still show up in the
                            // batch's summary of what did not make it.
                            skipped.push((path.clone(), error));
                            cancelled = true;
                            break 'items;
                        }
                    }
                }
            }
        }
    }

    BatchOutcome {
        trashed,
        skipped,
        cancelled,
    }
}

// ---------------------------------------------------------------------------
// Restoring (cb_14..cb_18)
// ---------------------------------------------------------------------------

fn classify_parent_or_volume_error(parent: &Path, source: std::io::Error) -> RestoreError {
    match source.raw_os_error() {
        // ENOENT, ENXIO, ENODEV, EIO: the ancestor filesystem itself is not
        // reachable (an unplugged drive), as opposed to a plain permission
        // problem.
        Some(2) | Some(6) | Some(19) | Some(5) => RestoreError::DestinationVolumeUnavailable {
            destination: parent.to_path_buf(),
            source,
        },
        _ => RestoreError::ParentCreation {
            parent: parent.to_path_buf(),
            source,
        },
    }
}

fn keep_both_destination(original: &Path) -> Result<PathBuf, RestoreError> {
    let parent = original.parent().unwrap_or_else(|| Path::new("."));
    let file_name = match original.file_name() {
        Some(name) => name,
        None => {
            return Err(RestoreError::DestinationExists {
                destination: original.to_path_buf(),
            });
        }
    };
    let (stem, ext) = split_stem_ext(file_name.as_bytes());
    for n in 2..10_000u32 {
        let mut candidate = stem.to_vec();
        candidate.extend_from_slice(format!(" ({n})").as_bytes());
        candidate.extend_from_slice(ext);
        let candidate_path = parent.join(OsStr::from_bytes(&candidate));
        if std::fs::symlink_metadata(&candidate_path).is_err() {
            return Ok(candidate_path);
        }
    }
    Err(RestoreError::DestinationExists {
        destination: original.to_path_buf(),
    })
}

fn resolve_restore_destination(
    original: &Path,
    on_conflict: ConflictPolicy,
) -> Result<PathBuf, RestoreError> {
    match on_conflict {
        ConflictPolicy::Overwrite => Ok(original.to_path_buf()),
        ConflictPolicy::Fail => {
            if std::fs::symlink_metadata(original).is_ok() {
                return Err(RestoreError::DestinationExists {
                    destination: original.to_path_buf(),
                });
            }
            Ok(original.to_path_buf())
        }
        ConflictPolicy::KeepBoth => {
            if std::fs::symlink_metadata(original).is_err() {
                Ok(original.to_path_buf())
            } else {
                keep_both_destination(original)
            }
        }
    }
}

/// Puts a trashed item back. Returns the path it actually landed on, which may
/// differ from `original_path` under [`ConflictPolicy::KeepBoth`]. The
/// `.trashinfo` is removed only once the final rename succeeded.
pub fn restore_item(
    item: &TrashedItem,
    on_conflict: ConflictPolicy,
) -> Result<PathBuf, RestoreError> {
    if std::fs::symlink_metadata(&item.trashed_path).is_err() {
        return Err(RestoreError::TrashEntryMissing {
            trashed_path: item.trashed_path.clone(),
        });
    }

    let destination = resolve_restore_destination(&item.original_path, on_conflict)?;

    if let Some(parent) = destination.parent()
        && !parent.as_os_str().is_empty()
        && std::fs::symlink_metadata(parent).is_err()
        && let Err(source) = std::fs::create_dir_all(parent)
    {
        return Err(classify_parent_or_volume_error(parent, source));
    }

    // Only `Overwrite` may land on an occupied path. For the other two the
    // check above is not enough on its own: between it and the rename another
    // process — or the user, or a running download — can create the very file
    // the check just found free, and a plain `rename(2)` would destroy it
    // without a word. Undo is the operation the whole convenience is built
    // around ("la operación debe ser deshacible con Ctrl+Z"), so it is the
    // last place that may lose a file (cb_16).
    let renamed = if on_conflict == ConflictPolicy::Overwrite {
        rustix::fs::renameat(
            rustix::fs::CWD,
            &item.trashed_path,
            rustix::fs::CWD,
            &destination,
        )
    } else {
        rename_noreplace(&item.trashed_path, &destination)
    };

    match renamed {
        Ok(()) => {
            // A `.trashinfo` that outlives its `files/` entry leaves a phantom
            // row in the trash view. Nothing in `RestoreError` can describe
            // "restored, but the record stayed behind" without claiming the
            // restore failed, so the cleanup is best-effort here and the trash
            // reader is expected to treat an entry with no file as stale.
            let _ = std::fs::remove_file(&item.info_path);
            Ok(destination)
        }
        Err(rustix::io::Errno::EXIST) => Err(RestoreError::DestinationExists { destination }),
        Err(source) => Err(RestoreError::Io {
            path: destination,
            source: source.into(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Permanent deletion (cb_26)
// ---------------------------------------------------------------------------

/// Deletes a path permanently, recursively and cancellably. Returns how many
/// entries were removed. This is the explicit way out for volumes with no
/// trash: nothing in this module ever calls it on its own.
pub fn delete_permanently(
    path: &Path,
    observer: &mut dyn TrashObserver,
) -> Result<u64, TrashError> {
    // The same two refusals `trash_one` applies (cb_23) matter more here, not
    // less: this is the irreversible half of the pair, and "reversibilidad por
    // defecto" (00-filosofia) has nothing left to fall back on once the walk
    // starts. Only `/` and mount points are refused, deliberately not the rest
    // of `refuse_special_path`: emptying the trash means calling this on paths
    // that live *inside* the trash, and refusing those would break it.
    if path == Path::new("/") {
        return Err(TrashError::RefusedSpecialPath {
            path: path.to_path_buf(),
            reason: RefusalReason::Root,
        });
    }
    let metadata = dir::lstat(path)?;
    refuse_mount_point(path, &metadata)?;
    // Through directory descriptors: links are never followed, even when one
    // is swapped in mid-walk, and nested mount points are refused.
    remove::delete_tree(path, &metadata, observer)
}
