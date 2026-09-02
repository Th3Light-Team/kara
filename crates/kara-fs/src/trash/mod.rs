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
//!
//! Esta capa no pregunta ni confirma nada: el ajuste de confirmación y los
//! diálogos viven por encima (`ground/spec/06-contexto-power.md`).

mod dir;
mod error;
mod info;

pub use dir::{TrashAvailability, TrashDir, TrashKind, TrashPolicy, home_trash_dir, probe_trash, resolve_trash_dir};
pub use error::{RefusalReason, RestoreError, TrashError, TrashInfoError, UnavailableReason};
pub use info::{DeletionDate, TrashInfo, read_trash_info};

use std::path::{Path, PathBuf};

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

    /// Fine-grained byte progress. Only ever called during an authorised
    /// cross-device copy or during [`delete_permanently`].
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

/// Moves a single path to the trash. Never follows the last symlink component:
/// the link itself is trashed, not its target. Never walks the contents of a
/// directory unless the policy authorises a cross-device copy.
pub fn trash_one(path: &Path, policy: &TrashPolicy) -> Result<TrashedItem, TrashError> {
    let _ = (path, policy);
    todo!("trash_one")
}

/// Moves several paths to the trash, in the given order, reporting each one.
pub fn trash_batch(
    paths: &[PathBuf],
    policy: &TrashPolicy,
    observer: &mut dyn TrashObserver,
) -> BatchOutcome {
    let _ = (paths, policy, observer);
    todo!("trash_batch")
}

/// Puts a trashed item back. Returns the path it actually landed on, which may
/// differ from `original_path` under [`ConflictPolicy::KeepBoth`]. The
/// `.trashinfo` is removed only once the final rename succeeded.
pub fn restore_item(
    item: &TrashedItem,
    on_conflict: ConflictPolicy,
) -> Result<PathBuf, RestoreError> {
    let _ = (item, on_conflict);
    todo!("restore_item")
}

/// Deletes a path permanently, recursively and cancellably. Returns how many
/// entries were removed. This is the explicit way out for volumes with no
/// trash: nothing in this module ever calls it on its own.
pub fn delete_permanently(
    path: &Path,
    observer: &mut dyn TrashObserver,
) -> Result<u64, TrashError> {
    let _ = (path, observer);
    todo!("delete_permanently")
}
