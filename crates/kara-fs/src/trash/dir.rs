//! Localización de la papelera: la del hogar y las de volumen.
//!
//! Papelera del hogar: `$XDG_DATA_HOME/Trash`, con caída a
//! `$HOME/.local/share/Trash`. Papeleras por volumen: `$topdir/.Trash/$uid`
//! (solo si `.Trash` es un directorio real con el bit sticky) o, en su
//! defecto, `$topdir/.Trash-$uid` (`ground/spec/05-operaciones.md`).

use std::path::{Path, PathBuf};

use super::error::{TrashError, UnavailableReason};

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
}

impl Default for TrashPolicy {
    fn default() -> Self {
        TrashPolicy {
            create_volume_trash: true,
            allow_cross_device_copy: false,
            max_item_bytes: None,
            free_space_margin_bytes: 0,
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

/// Resolves the home trash, creating `files/` and `info/` with mode 0700 when
/// they are missing.
pub fn home_trash_dir() -> Result<TrashDir, TrashError> {
    todo!("home_trash_dir")
}

/// Resolves the trash directory that must hold `path`, according to the device
/// the path lives on.
pub fn resolve_trash_dir(path: &Path, policy: &TrashPolicy) -> Result<TrashDir, TrashError> {
    let _ = (path, policy);
    todo!("resolve_trash_dir")
}

/// Answers whether `path` could be trashed. Never creates, writes or moves
/// anything, even when the policy allows creating a volume trash.
pub fn probe_trash(path: &Path, policy: &TrashPolicy) -> Result<TrashAvailability, TrashError> {
    let _ = (path, policy);
    todo!("probe_trash")
}
