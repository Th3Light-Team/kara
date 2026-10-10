//! Undo and redo of the remote records of [`Action`].
//!
//! Rules (spec: «algunas operaciones sobre volúmenes remotos no se pueden
//! deshacer y la acción se deshabilita»):
//!
//! - A remote copy is undone by removing the copy, permanently: there is no
//!   remote trash. A folder created on a drive is removed only while empty, so
//!   an undo never destroys something the user put there afterwards.
//! - A move or rename is undone by renaming back, and only while the drive
//!   still declares `undo_move` / `undo_rename`.
//! - [`Action::NotUndoable`] is never undone. A permanent delete is never
//!   recorded at all.
//! - Nothing is touched before the subject is checked to be where the action
//!   left it, and nothing is ever overwritten.

use kara_vfs::{BackendError, BackendErrorKind, Cancel, Location};

use crate::location::{
    BackendResolver, Endpoint, EndpointError, NO_UNDO_MOVE, NO_UNDO_RENAME, display_path, reason,
};
use crate::undo::{Action, UndoError};

fn endpoint(location: &Location, resolver: &BackendResolver) -> Result<Endpoint, UndoError> {
    Endpoint::resolve(location, resolver).map_err(|error| match error {
        EndpointError::Drive(_) => UndoError::DriveUnavailable(display_path(location)),
        EndpointError::Name => UndoError::Vanished(display_path(location)),
    })
}

fn failed(end: &Endpoint, error: &BackendError) -> UndoError {
    match error.kind {
        BackendErrorKind::NotFound => UndoError::Vanished(end.display()),
        BackendErrorKind::Unavailable => UndoError::DriveUnavailable(end.display()),
        kind => UndoError::Remote {
            path: end.display(),
            reason: reason(kind),
        },
    }
}

/// The subject must still be there.
fn ensure_present(end: &Endpoint) -> Result<(), UndoError> {
    end.stat().map(|_| ()).map_err(|error| failed(end, &error))
}

/// The place something goes back to must be free.
fn ensure_free(end: &Endpoint) -> Result<(), UndoError> {
    match end.stat() {
        Ok(_) => Err(UndoError::Remote {
            path: end.display(),
            reason: reason(BackendErrorKind::AlreadyExists),
        }),
        Err(error) if error.kind == BackendErrorKind::NotFound => Ok(()),
        Err(error) => Err(failed(end, &error)),
    }
}

/// Renames `from` back to `to` on one drive, if the drive still allows it.
fn rename_back(
    from: &Location,
    to: &Location,
    allowed: impl Fn(&kara_vfs::Capabilities) -> bool,
    refusal: &str,
    resolver: &BackendResolver,
) -> Result<(), UndoError> {
    let source = endpoint(from, resolver)?;
    let target = endpoint(to, resolver)?;
    if !source.same_backend(&target) || !allowed(&source.backend.capabilities()) {
        return Err(UndoError::NotUndoable(refusal.to_string()));
    }
    ensure_present(&source)?;
    ensure_free(&target)?;
    source
        .backend
        .rename(&source.path, &target.path)
        .map_err(|error| failed(&source, &error))
}

/// Executes the inverse of a remote `action`.
pub(crate) fn revert(action: &Action, resolver: &BackendResolver) -> Result<Action, UndoError> {
    match action {
        Action::RemoteCopied { created } => {
            let end = endpoint(created, resolver)?;
            ensure_present(&end)?;
            end.backend
                .remove_tree(&end.path, &Cancel::new())
                .map_err(|error| failed(&end, &error))?;
        }
        Action::RemoteDirectoryCreated { path } => {
            let end = endpoint(path, resolver)?;
            ensure_present(&end)?;
            // `remove` refuses a folder that is no longer empty.
            end.backend
                .remove(&end.path)
                .map_err(|error| failed(&end, &error))?;
        }
        Action::RemoteMoved { from, to } => {
            rename_back(to, from, |caps| caps.undo_move, NO_UNDO_MOVE, resolver)?;
        }
        Action::RemoteRenamed { from, to } => {
            rename_back(to, from, |caps| caps.undo_rename, NO_UNDO_RENAME, resolver)?;
        }
        Action::NotUndoable { reason, .. } => return Err(UndoError::NotUndoable(reason.clone())),
        Action::Moved { .. }
        | Action::Copied { .. }
        | Action::Renamed { .. }
        | Action::DirectoryCreated { .. }
        | Action::Trashed { .. } => {
            return Err(UndoError::NotUndoable("not a remote action".to_string()));
        }
    }
    Ok(action.clone())
}

/// Applies a remote `action` again after it was undone.
pub(crate) fn reapply(action: &Action, resolver: &BackendResolver) -> Result<(), UndoError> {
    match action {
        Action::RemoteMoved { from, to } => {
            rename_back(from, to, |caps| caps.undo_move, NO_UNDO_MOVE, resolver)
        }
        Action::RemoteRenamed { from, to } => {
            rename_back(from, to, |caps| caps.undo_rename, NO_UNDO_RENAME, resolver)
        }
        // Redoing a copy would need the original source, which the record
        // does not keep; same rule as the local copy.
        Action::RemoteCopied { .. } | Action::RemoteDirectoryCreated { .. } => Err(
            UndoError::NotUndoable("rehacer una copia no está soportado".to_string()),
        ),
        Action::NotUndoable { reason, .. } => Err(UndoError::NotUndoable(reason.clone())),
        Action::Moved { .. }
        | Action::Copied { .. }
        | Action::Renamed { .. }
        | Action::DirectoryCreated { .. }
        | Action::Trashed { .. } => {
            Err(UndoError::NotUndoable("not a remote action".to_string()))
        }
    }
}
