//! Errores tipados de la papelera.
//!
//! Cada variante nombra la ruta implicada y conserva el `errno` original en
//! `source`: la spec exige que el mensaje de error diga *qué* fichero falló
//! (`ground/spec/06-contexto-power.md`, «Eliminar (papelera) y borrado
//! permanente»).

use std::path::{Path, PathBuf};

/// Why a trash directory cannot be used for a given path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    /// No mount point could be resolved for the path.
    NoTopDir,
    /// The volume top directory is mounted read-only.
    TopDirReadOnly,
    /// The volume top directory exists but is not writable by this user.
    TopDirNotWritable,
    /// `$topdir/.Trash` exists but failed the sticky/real-directory checks.
    StickyTrashRejected,
    /// `$topdir/.Trash-$uid` is missing and the policy forbids creating it.
    VolumeTrashMissingAndCreationDisabled,
    /// The path already lives inside a trash directory.
    PathIsInsideTrash,
    /// The path is itself a mount point (top directory).
    PathIsTopDir,
}

/// Why a path is refused outright, before any syscall that could change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// The filesystem root `/`.
    Root,
    /// A mount point.
    MountPoint,
    /// An ancestor of the trash directory that would be used.
    TrashAncestor,
    /// A relative path: the trash record needs an absolute original path.
    Relative,
    /// An empty path.
    Empty,
}

/// Failure of a trash or permanent-delete operation on a single path.
#[derive(Debug, thiserror::Error)]
pub enum TrashError {
    #[error("path not found: {path}")]
    NotFound {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("permission denied: {path}")]
    PermissionDenied {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("no usable trash on the volume holding {path}")]
    NoTrashOnVolume {
        path: PathBuf,
        reason: UnavailableReason,
    },
    #[error(
        "{path} needs {needed_bytes} bytes but only {available_bytes} are available in the trash"
    )]
    ExceedsTrashCapacity {
        path: PathBuf,
        needed_bytes: u64,
        available_bytes: u64,
    },
    #[error("{path} is on a different device than the trash at {trash_root}")]
    CrossDevice { path: PathBuf, trash_root: PathBuf },
    #[error("{path} is already inside the trash")]
    PathIsInsideTrash { path: PathBuf },
    #[error("refused to trash {path}")]
    RefusedSpecialPath {
        path: PathBuf,
        reason: RefusalReason,
    },
    #[error("could not find a free name in the trash for {path} after {attempts} attempts")]
    NameExhausted { path: PathBuf, attempts: u32 },
    #[error("could not write the trash info file {info_path}")]
    InfoWrite {
        info_path: PathBuf,
        source: std::io::Error,
    },
    #[error("i/o error on {path}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("operation cancelled")]
    Cancelled,
}

/// Failure of restoring an item out of the trash.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error("the trashed file {trashed_path} is gone")]
    TrashEntryMissing { trashed_path: PathBuf },
    #[error("{destination} already exists")]
    DestinationExists { destination: PathBuf },
    #[error("the volume holding {destination} is not available")]
    DestinationVolumeUnavailable {
        destination: PathBuf,
        source: std::io::Error,
    },
    #[error("could not recreate the parent directory {parent}")]
    ParentCreation {
        parent: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Info(#[from] TrashInfoError),
    #[error("i/o error on {path}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Maps a raw I/O error into the [`TrashError`] variant its errno implies,
/// keeping `source` so the original errno is never lost. `ENOENT` becomes
/// [`TrashError::NotFound`]; `EACCES`/`EPERM` become
/// [`TrashError::PermissionDenied`]; everything else is [`TrashError::Io`].
pub(crate) fn classify_io_error(path: &Path, source: std::io::Error) -> TrashError {
    match source.raw_os_error() {
        Some(2) => TrashError::NotFound {
            path: path.to_path_buf(),
            source,
        },
        Some(1) | Some(13) => TrashError::PermissionDenied {
            path: path.to_path_buf(),
            source,
        },
        _ => TrashError::Io {
            path: path.to_path_buf(),
            source,
        },
    }
}

/// Failure of reading, parsing or writing a `.trashinfo` file.
#[derive(Debug, thiserror::Error)]
pub enum TrashInfoError {
    #[error("missing the `[Trash Info]` header")]
    MissingHeader,
    #[error("missing the `Path=` key")]
    MissingPath,
    #[error("missing the `DeletionDate=` key")]
    MissingDeletionDate,
    #[error("malformed deletion date: {raw}")]
    MalformedDate { raw: String },
    #[error("invalid percent encoding: {raw}")]
    InvalidPercentEncoding { raw: String },
    #[error("the recorded path is not absolute: {raw}")]
    NotAbsolute { raw: String },
    #[error("i/o error on {path}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}
