//! Localización de la papelera: la del hogar y las de volumen.
//!
//! Papelera del hogar: `$XDG_DATA_HOME/Trash`, con caída a
//! `$HOME/.local/share/Trash`. Papeleras por volumen: `$topdir/.Trash/$uid`
//! (solo si `.Trash` es un directorio real con el bit sticky) o, en su
//! defecto, `$topdir/.Trash-$uid` (`ground/spec/05-operaciones.md`).

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::error::{TrashError, UnavailableReason, classify_io_error};

/// Which of the two FreeDesktop trash flavours a directory is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashKind {
    Home,
    Volume,
}

/// A resolved trash directory and its two mandatory subdirectories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashDir {
    pub root: PathBuf,
    pub files: PathBuf,
    pub info: PathBuf,
    pub kind: TrashKind,
    /// `Some` only for [`TrashKind::Volume`]; it makes `Path=` relative.
    pub top_dir: Option<PathBuf>,
}

/// Caller-supplied limits and permissions for the operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashPolicy {
    /// Create `$topdir/.Trash-$uid` when it is missing.
    pub create_volume_trash: bool,
    /// Allow copying bytes across devices instead of failing with
    /// [`TrashError::CrossDevice`].
    pub allow_cross_device_copy: bool,
    /// Refuse items whose apparent size exceeds this many bytes.
    pub max_item_bytes: Option<u64>,
    /// Free space that must remain on the trash filesystem after a copy.
    pub free_space_margin_bytes: u64,
    /// Seconds east of UTC to apply when stamping a `DeletionDate` (cb_07).
    ///
    /// FreeDesktop's `DeletionDate` is local time without a zone suffix, and
    /// this crate deliberately never reads the machine's timezone itself —
    /// the offset is injected so the conversion stays deterministic and
    /// testable. `kara-ops` is expected to fill this in with the real local
    /// offset; the default of `0` (UTC) is only correct on the Greenwich
    /// meridian, but it is a typed, documented default rather than a value
    /// silently hardcoded deep inside `trash_one`.
    pub utc_offset_seconds: i32,
}

impl Default for TrashPolicy {
    fn default() -> Self {
        TrashPolicy {
            create_volume_trash: true,
            allow_cross_device_copy: false,
            max_item_bytes: None,
            free_space_margin_bytes: 0,
            utc_offset_seconds: 0,
        }
    }
}

/// Result of asking, without side effects, whether a path can be trashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashAvailability {
    Available {
        kind: TrashKind,
        /// `true` when the trash directory does not exist yet but the policy
        /// would create it. Probing never creates it.
        would_create: bool,
        free_bytes: Option<u64>,
    },
    Unavailable {
        reason: UnavailableReason,
    },
    ExceedsCapacity {
        needed_bytes: u64,
        available_bytes: u64,
    },
}

/// Resolves the home trash root purely from environment variables: no I/O.
///
/// `$XDG_DATA_HOME/Trash` when `XDG_DATA_HOME` is set, non-empty and
/// absolute; otherwise `$HOME/.local/share/Trash`.
pub(crate) fn home_trash_root() -> Result<PathBuf, TrashError> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME")
        && !xdg.is_empty()
    {
        let candidate = PathBuf::from(&xdg);
        if candidate.is_absolute() {
            return Ok(candidate.join("Trash"));
        }
    }
    let home = std::env::var_os("HOME").ok_or_else(|| TrashError::Io {
        path: PathBuf::from("$HOME"),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "HOME is not set"),
    })?;
    Ok(PathBuf::from(home).join(".local/share/Trash"))
}

/// `lstat`, classified into a typed [`TrashError`] instead of a raw
/// `io::Error`. Never follows the last path component.
pub(crate) fn lstat(path: &Path) -> Result<std::fs::Metadata, TrashError> {
    std::fs::symlink_metadata(path).map_err(|source| classify_io_error(path, source))
}

pub(crate) fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Creates `path` (and its ancestors) if missing, and sets `mode` on it only
/// when it did not already exist: a directory an earlier call locked down on
/// purpose must stay locked down.
fn ensure_dir(path: &Path, mode: u32) -> Result<(), TrashError> {
    let existed = path.is_dir();
    std::fs::create_dir_all(path).map_err(|source| classify_io_error(path, source))?;
    if !existed {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|source| classify_io_error(path, source))?;
    }
    Ok(())
}

/// Walks up from `path` until it finds an ancestor that exists, and returns
/// its device. Used to guess the device a not-yet-created directory (like the
/// home trash before its first use) will end up on.
fn nearest_existing_dev(path: &Path) -> Result<u64, TrashError> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if let Ok(metadata) = std::fs::symlink_metadata(candidate) {
            return Ok(metadata.dev());
        }
        current = candidate.parent();
    }
    Err(TrashError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "no existing ancestor"),
    })
}

/// Finds the mount point that holds `path`: the deepest ancestor that still
/// shares its device.
pub(crate) fn topdir_of(path: &Path) -> Result<PathBuf, TrashError> {
    let path_dev = lstat(path)?.dev();
    let mut mount_point = PathBuf::from("/");
    let mut current = path.parent();
    while let Some(dir) = current {
        match std::fs::symlink_metadata(dir) {
            Ok(metadata) if metadata.dev() == path_dev => {
                mount_point = dir.to_path_buf();
                current = dir.parent();
            }
            _ => break,
        }
    }
    Ok(mount_point)
}

/// A `$topdir/.Trash` is only trustworthy when it is a real directory (not a
/// symlink) with the sticky bit set; a rejected candidate is never followed.
fn is_valid_shared_trash_dir(dot_trash: &Path) -> Result<bool, TrashError> {
    match std::fs::symlink_metadata(dot_trash) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Ok(false);
            }
            Ok(metadata.permissions().mode() & 0o1000 != 0)
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(classify_io_error(dot_trash, source)),
    }
}

/// State of a volume trash root (`$topdir/.Trash-$uid`, or `$topdir/.Trash/$uid`)
/// before anything is written into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VolumeRootState {
    /// Nothing is there: it can be created if the policy allows it.
    Missing,
    /// A real directory owned by this user.
    Usable,
    /// Something is there, but following it would be unsafe.
    Rejected,
}

/// The same distrust `is_valid_shared_trash_dir` applies to `$topdir/.Trash`,
/// applied to the trash root that actually gets written into.
///
/// `$topdir` is frequently world-writable (that is the whole reason the shared
/// `.Trash` needs a sticky bit), so anybody can pre-create `.Trash-$uid` there
/// as a symbolic link pointing wherever they like. Following it would move
/// this user's deleted files, and their `.trashinfo` records, straight into
/// somebody else's directory — silently, since the operation would otherwise
/// succeed. `stat`-based checks such as `Path::is_dir` follow symlinks and
/// cannot see this; `lstat` can. The ownership check is the same one glib's
/// local trash backend applies before reusing an existing `.Trash-$uid`.
fn volume_root_state(root: &Path) -> Result<VolumeRootState, TrashError> {
    match std::fs::symlink_metadata(root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.uid() != current_uid()
            {
                Ok(VolumeRootState::Rejected)
            } else {
                Ok(VolumeRootState::Usable)
            }
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(VolumeRootState::Missing),
        Err(source) => Err(classify_io_error(root, source)),
    }
}

/// Where the volume trash for `path` would live, without creating anything.
fn volume_candidate(path: &Path) -> Result<(PathBuf, PathBuf), TrashError> {
    let top_dir = topdir_of(path)?;
    let uid = current_uid();
    let dot_trash = top_dir.join(".Trash");
    let candidate_root = if is_valid_shared_trash_dir(&dot_trash)? {
        dot_trash.join(uid.to_string())
    } else {
        top_dir.join(format!(".Trash-{uid}"))
    };
    Ok((top_dir, candidate_root))
}

/// Read-only check (`access(2)`) of whether `top_dir` looks writable, used to
/// explain *why* a volume trash cannot be used without attempting to create
/// anything.
fn top_dir_unavailable_reason(top_dir: &Path) -> Result<Option<UnavailableReason>, TrashError> {
    match rustix::fs::access(top_dir, rustix::fs::Access::WRITE_OK) {
        Ok(()) => Ok(None),
        Err(rustix::io::Errno::ROFS) => Ok(Some(UnavailableReason::TopDirReadOnly)),
        Err(rustix::io::Errno::ACCESS) | Err(rustix::io::Errno::PERM) => {
            Ok(Some(UnavailableReason::TopDirNotWritable))
        }
        Err(rustix::io::Errno::NOENT) => Ok(Some(UnavailableReason::NoTopDir)),
        Err(errno) => Err(TrashError::Io {
            path: top_dir.to_path_buf(),
            source: errno.into(),
        }),
    }
}

/// Same classification, but from the `io::Error` an actual creation attempt
/// failed with.
fn classify_unavailable(source: &std::io::Error) -> UnavailableReason {
    match source.raw_os_error() {
        Some(30) => UnavailableReason::TopDirReadOnly, // EROFS
        Some(2) => UnavailableReason::NoTopDir,        // ENOENT
        Some(1) | Some(13) => UnavailableReason::TopDirNotWritable, // EPERM / EACCES
        _ => UnavailableReason::TopDirNotWritable,
    }
}

pub(crate) fn free_bytes_on(path: &Path) -> Option<u64> {
    let stat = rustix::fs::statvfs(path).ok()?;
    stat.f_bavail.checked_mul(stat.f_frsize)
}

/// Resolves the home trash, creating `files/` and `info/` with mode 0700 when
/// they are missing.
pub fn home_trash_dir() -> Result<TrashDir, TrashError> {
    let root = home_trash_root()?;
    let files = root.join("files");
    let info = root.join("info");
    ensure_dir(&root, 0o700)?;
    ensure_dir(&files, 0o700)?;
    ensure_dir(&info, 0o700)?;
    Ok(TrashDir {
        root,
        files,
        info,
        kind: TrashKind::Home,
        top_dir: None,
    })
}

fn build_volume_trash_dir(
    root: PathBuf,
    top_dir: PathBuf,
    policy: &TrashPolicy,
    original_path: &Path,
) -> Result<TrashDir, TrashError> {
    let state = volume_root_state(&root)?;
    if state == VolumeRootState::Rejected {
        return Err(TrashError::NoTrashOnVolume {
            path: original_path.to_path_buf(),
            reason: UnavailableReason::VolumeTrashRejected,
        });
    }
    if state == VolumeRootState::Missing {
        if !policy.create_volume_trash {
            return Err(TrashError::NoTrashOnVolume {
                path: original_path.to_path_buf(),
                reason: UnavailableReason::VolumeTrashMissingAndCreationDisabled,
            });
        }
        if let Err(source) = std::fs::create_dir_all(&root) {
            return Err(TrashError::NoTrashOnVolume {
                path: original_path.to_path_buf(),
                reason: classify_unavailable(&source),
            });
        }
        if let Err(source) = std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        {
            return Err(TrashError::NoTrashOnVolume {
                path: original_path.to_path_buf(),
                reason: classify_unavailable(&source),
            });
        }
    }
    let files = root.join("files");
    let info = root.join("info");
    if let Err(error) = ensure_dir(&files, 0o700) {
        return Err(wrap_as_no_trash_on_volume(error, original_path));
    }
    if let Err(error) = ensure_dir(&info, 0o700) {
        return Err(wrap_as_no_trash_on_volume(error, original_path));
    }
    Ok(TrashDir {
        root,
        files,
        info,
        kind: TrashKind::Volume,
        top_dir: Some(top_dir),
    })
}

/// A volume trash whose root exists but whose `files/` or `info/` cannot be
/// created is functionally the same situation as one that could not be
/// created at all: the caller that only branches on [`TrashError::NoTrashOnVolume`]
/// to offer "delete permanently with a warning" must see it that way too.
fn wrap_as_no_trash_on_volume(error: TrashError, original_path: &Path) -> TrashError {
    let reason = match &error {
        TrashError::NotFound { .. } => UnavailableReason::NoTopDir,
        TrashError::PermissionDenied { .. } => UnavailableReason::TopDirNotWritable,
        _ => UnavailableReason::TopDirNotWritable,
    };
    TrashError::NoTrashOnVolume {
        path: original_path.to_path_buf(),
        reason,
    }
}

/// Resolves the trash directory that must hold `path`, according to the device
/// the path lives on.
pub fn resolve_trash_dir(path: &Path, policy: &TrashPolicy) -> Result<TrashDir, TrashError> {
    let path_dev = lstat(path)?.dev();
    let home_root = home_trash_root()?;
    let home_dev = nearest_existing_dev(&home_root)?;
    if path_dev == home_dev {
        return home_trash_dir();
    }
    let (top_dir, candidate_root) = volume_candidate(path)?;
    build_volume_trash_dir(candidate_root, top_dir, policy, path)
}

/// Answers whether `path` could be trashed. Never creates, writes or moves
/// anything, even when the policy allows creating a volume trash.
pub fn probe_trash(path: &Path, policy: &TrashPolicy) -> Result<TrashAvailability, TrashError> {
    // A probe whose answer disagrees with what `trash_one` would then do is
    // worse than no probe at all: the point of asking first is to warn before
    // acting (cb_10, cb_25), so a path `trash_one` refuses outright — `/`, a
    // relative path, an ancestor of the trash itself — must be refused here
    // with the very same error instead of being reported as trashable
    // (cb_23). The one refusal kept in `Unavailable` form is a path already
    // inside the trash, which is a *state* a caller can display next to the
    // other availability reasons rather than a programming error.
    match super::refuse_special_path(path) {
        Ok(()) => {}
        Err(TrashError::PathIsInsideTrash { .. }) => {
            return Ok(TrashAvailability::Unavailable {
                reason: UnavailableReason::PathIsInsideTrash,
            });
        }
        Err(error) => return Err(error),
    }

    let metadata = lstat(path)?;

    // Mirrors the rest of the refusals `trash_one` applies before moving
    // anything (cb_23), surfaced the way `probe` reports everything else: as a
    // typed reason to show *before* acting, not an error only discovered after
    // the fact (cb_12's "the reason is exposed as ... in probe").
    if let Ok(home_root) = home_trash_root()
        && (path.starts_with(home_root.join("files")) || path.starts_with(home_root.join("info")))
    {
        return Ok(TrashAvailability::Unavailable {
            reason: UnavailableReason::PathIsInsideTrash,
        });
    }
    if metadata.is_dir()
        && let Some(parent) = path.parent()
        && let Ok(parent_metadata) = std::fs::symlink_metadata(parent)
        && parent_metadata.dev() != metadata.dev()
    {
        return Ok(TrashAvailability::Unavailable {
            reason: UnavailableReason::PathIsTopDir,
        });
    }

    if let Some(limit) = policy.max_item_bytes {
        // Mirrors `check_capacity`'s use of `apparent_size`: `metadata.len()`
        // alone is the inode size (4 KiB-ish for most directories), not the
        // size of the tree it holds, so a directory over the limit must be
        // walked the same way `trash_one` walks it — otherwise this
        // early-warning check and the one `trash_one` actually enforces
        // would disagree (cb_13).
        // `probe_trash` takes no observer (its signature is normative): the
        // walk below is bounded by `limit` the same way `check_capacity`'s
        // is, but has no way to be cancelled mid-flight.
        let size = super::apparent_size(path, &metadata, limit, &mut super::NullObserver)?;
        if size > limit {
            return Ok(TrashAvailability::ExceedsCapacity {
                needed_bytes: size,
                available_bytes: limit,
            });
        }
    }

    let path_dev = metadata.dev();
    let home_root = home_trash_root()?;
    let home_dev = nearest_existing_dev(&home_root)?;

    if path_dev == home_dev {
        let would_create = !(home_root.join("files").is_dir() && home_root.join("info").is_dir());
        return Ok(TrashAvailability::Available {
            kind: TrashKind::Home,
            would_create,
            free_bytes: free_bytes_on(&home_root),
        });
    }

    let (top_dir, candidate_root) = volume_candidate(path)?;
    match volume_root_state(&candidate_root)? {
        VolumeRootState::Usable => {
            return Ok(TrashAvailability::Available {
                kind: TrashKind::Volume,
                would_create: false,
                free_bytes: free_bytes_on(&candidate_root),
            });
        }
        VolumeRootState::Rejected => {
            return Ok(TrashAvailability::Unavailable {
                reason: UnavailableReason::VolumeTrashRejected,
            });
        }
        VolumeRootState::Missing => {}
    }

    // `$topdir/.Trash` existing but failing the sticky/real-directory check
    // is a more specific, more actionable reason than the generic ones
    // below: whenever the volume turns out unavailable anyway (no usable
    // `.Trash-$uid` either), it explains *why* the shared trash was skipped
    // instead of just that nothing usable was found (cb_12). When
    // `.Trash-$uid` is itself fine, the rejection is silent, exactly as
    // `resolve_trash_dir` treats it: it is not the reason anything failed.
    let sticky_rejected = dot_trash_was_rejected(&top_dir)?;

    if !policy.create_volume_trash {
        return Ok(TrashAvailability::Unavailable {
            reason: if sticky_rejected {
                UnavailableReason::StickyTrashRejected
            } else {
                UnavailableReason::VolumeTrashMissingAndCreationDisabled
            },
        });
    }

    match top_dir_unavailable_reason(&top_dir)? {
        Some(reason) => Ok(TrashAvailability::Unavailable {
            reason: if sticky_rejected {
                UnavailableReason::StickyTrashRejected
            } else {
                reason
            },
        }),
        None => Ok(TrashAvailability::Available {
            kind: TrashKind::Volume,
            would_create: true,
            free_bytes: free_bytes_on(&top_dir),
        }),
    }
}

/// Whether `$top_dir/.Trash` exists but was rejected by
/// [`is_valid_shared_trash_dir`] (a symlink, not a directory, or missing the
/// sticky bit) — as opposed to simply not existing at all.
fn dot_trash_was_rejected(top_dir: &Path) -> Result<bool, TrashError> {
    let dot_trash = top_dir.join(".Trash");
    match std::fs::symlink_metadata(&dot_trash) {
        Ok(_) => Ok(!is_valid_shared_trash_dir(&dot_trash)?),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(classify_io_error(&dot_trash, source)),
    }
}
