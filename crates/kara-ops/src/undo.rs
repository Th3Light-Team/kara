//! Pila de deshacer y rehacer operaciones de fichero.
//!
//! Conveniencia de referencia: `ground/spec/05-operaciones.md`, «Deshacer y
//! rehacer operaciones de archivo».
//!
//! # Decisiones
//!
//! - **Lo irreversible no entra en la pila.** El borrado permanente y el vaciado
//!   de la papelera no se pueden deshacer, así que no se registran. Tampoco
//!   vacían la pila: deshacer sigue llevándote a la operación reversible
//!   anterior, que es lo que hace Windows.
//! - **Deshacer una copia o una carpeta creada manda el resultado a la
//!   papelera**, no lo borra. La regla del proyecto dice que eliminar va siempre
//!   a la papelera, y no hay excepción por que lo pida un deshacer.
//! - **Se comprueba antes de actuar.** Si lo que hay que deshacer ya no está
//!   donde se dejó, se avisa con [`UndoError::Vanished`] en vez de fallar a
//!   medias o, peor, tocar un fichero distinto que ocupe ahora ese sitio.

use std::path::{Path, PathBuf};

use kara_fs::trash::{ConflictPolicy, TrashedItem, restore_item, trash_one};
use kara_fs::transfer::{TransferError, move_to, rename};
use kara_vfs::Location;

use crate::clock::trash_policy;
use crate::location::{BackendResolver, no_drives};

/// Una operación reversible ya ejecutada.
#[derive(Debug, Clone)]
pub enum Action {
    /// Se movió `from` y acabó en `to`.
    Moved { from: PathBuf, to: PathBuf },
    /// Se copió algo y se creó `created`.
    Copied { created: PathBuf },
    /// Se renombró `from` a `to`, en la misma carpeta.
    Renamed { from: PathBuf, to: PathBuf },
    /// Se creó la carpeta `path`.
    DirectoryCreated { path: PathBuf },
    /// Se envió algo a la papelera.
    Trashed { item: Box<TrashedItem> },
    /// A copy created `created` on a remote drive. Undo removes it, for good:
    /// a remote drive has no trash.
    RemoteCopied { created: Location },
    /// `from` was moved to `to` inside one remote drive that declares
    /// `undo_move`. Undo renames it back.
    RemoteMoved { from: Location, to: Location },
    /// `from` was renamed to `to` on a remote drive that declares
    /// `undo_rename`.
    RemoteRenamed { from: Location, to: Location },
    /// The folder `path` was created on a remote drive. Undo removes it only
    /// while it is still empty.
    RemoteDirectoryCreated { path: Location },
    /// Something happened that cannot be taken back. It stays on the stack so
    /// the menu can say «Deshacer `label`» disabled, with `reason`, as the
    /// spec asks for remote volumes.
    NotUndoable {
        label: &'static str,
        subject: Location,
        reason: String,
    },
}

impl Action {
    /// Etiqueta para el menú: «Deshacer mover», «Deshacer renombrar»…
    ///
    /// La spec pide decir *qué* se va a deshacer, no solo ofrecer deshacer.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Moved { .. } => "mover",
            Self::Copied { .. } => "copiar",
            Self::Renamed { .. } => "renombrar",
            Self::DirectoryCreated { .. } => "crear carpeta",
            Self::Trashed { .. } => "enviar a la papelera",
            Self::RemoteCopied { .. } => "copiar",
            Self::RemoteMoved { .. } => "mover",
            Self::RemoteRenamed { .. } => "renombrar",
            Self::RemoteDirectoryCreated { .. } => "crear carpeta",
            Self::NotUndoable { label, .. } => label,
        }
    }

    /// `false` for a record that only says what happened.
    #[must_use]
    pub fn is_undoable(&self) -> bool {
        !matches!(self, Self::NotUndoable { .. })
    }

    /// Why this cannot be undone, for the disabled menu entry.
    #[must_use]
    pub fn not_undoable_reason(&self) -> Option<&str> {
        match self {
            Self::NotUndoable { reason, .. } => Some(reason),
            _ => None,
        }
    }

    /// Dónde quedó el resultado, que es lo que hay que comprobar antes de
    /// deshacer. `None` for the remote records, checked by `remote_revert`.
    fn subject(&self) -> Option<&Path> {
        match self {
            Self::Moved { to, .. } | Self::Renamed { to, .. } => Some(to),
            Self::Copied { created } => Some(created),
            Self::DirectoryCreated { path } => Some(path),
            Self::Trashed { item } => Some(&item.trashed_path),
            Self::RemoteCopied { .. }
            | Self::RemoteMoved { .. }
            | Self::RemoteRenamed { .. }
            | Self::RemoteDirectoryCreated { .. }
            | Self::NotUndoable { .. } => None,
        }
    }

    fn is_remote(&self) -> bool {
        self.subject().is_none()
    }
}

/// Por qué no se pudo deshacer.
#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    /// Nada que deshacer o rehacer.
    #[error("nothing to undo")]
    Empty,
    /// Lo que había que deshacer ya no está donde se dejó.
    #[error("{0} is no longer where the operation left it")]
    Vanished(PathBuf),
    #[error(transparent)]
    Transfer(#[from] TransferError),
    #[error("{0}")]
    Trash(String),
    /// The action is recorded as not undoable; the reason says why.
    #[error("this action cannot be undone: {0}")]
    NotUndoable(String),
    /// The drive of a remote action is not connected (or no resolver was given).
    #[error("{} is on a drive that is not connected", .0.display())]
    DriveUnavailable(PathBuf),
    /// The remote drive refused; `path` is the item's URI.
    #[error("{}: {reason}", path.display())]
    Remote { path: PathBuf, reason: String },
}

/// Pila de deshacer/rehacer de una ventana.
#[derive(Debug, Default)]
pub struct UndoStack {
    done: Vec<Action>,
    undone: Vec<Action>,
}

impl UndoStack {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra una operación ya ejecutada. Invalida lo rehacible, igual que
    /// navegar invalida el «adelante» del historial.
    pub fn push(&mut self, action: Action) {
        self.done.push(action);
        self.undone.clear();
    }

    /// `false` when the stack is empty or its top is recorded as not
    /// undoable ([`Self::undo_disabled_reason`] says why).
    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.done.last().is_some_and(Action::is_undoable)
    }

    /// Why «Deshacer» is disabled although there is something on the stack.
    #[must_use]
    pub fn undo_disabled_reason(&self) -> Option<&str> {
        self.done.last().and_then(Action::not_undoable_reason)
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Qué diría el menú: `Some("mover")` para «Deshacer mover».
    #[must_use]
    pub fn undo_label(&self) -> Option<&'static str> {
        self.done.last().map(Action::label)
    }

    #[must_use]
    pub fn redo_label(&self) -> Option<&'static str> {
        self.undone.last().map(Action::label)
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.done.len()
    }

    /// Deshace la última operación.
    ///
    /// Si falla, la acción **vuelve a la pila**: un deshacer que no pudo con su
    /// trabajo no debe además perder el registro de que aquello ocurrió.
    pub fn undo(&mut self) -> Result<Action, UndoError> {
        self.undo_with(&no_drives())
    }

    /// [`Self::undo`] for a stack that may hold remote actions: `resolver`
    /// reaches their drives. Blocking (network), so the UI calls it from a
    /// worker when the top action is remote.
    ///
    /// A record that is not undoable stays on top and answers
    /// [`UndoError::NotUndoable`].
    pub fn undo_with(&mut self, resolver: &BackendResolver) -> Result<Action, UndoError> {
        let action = self.done.pop().ok_or(UndoError::Empty)?;
        let result = if action.is_remote() {
            crate::remote_undo::revert(&action, resolver)
        } else {
            revert(&action)
        };
        match result {
            Ok(reapplied) => {
                self.undone.push(action);
                Ok(reapplied)
            }
            Err(error) => {
                self.done.push(action);
                Err(error)
            }
        }
    }

    /// Rehace la última operación deshecha.
    pub fn redo(&mut self) -> Result<Action, UndoError> {
        self.redo_with(&no_drives())
    }

    /// [`Self::redo`] for a stack that may hold remote actions.
    pub fn redo_with(&mut self, resolver: &BackendResolver) -> Result<Action, UndoError> {
        let action = self.undone.pop().ok_or(UndoError::Empty)?;
        let result = if action.is_remote() {
            crate::remote_undo::reapply(&action, resolver)
        } else {
            reapply(&action)
        };
        match result {
            Ok(()) => {
                self.done.push(action.clone());
                Ok(action)
            }
            Err(error) => {
                self.undone.push(action);
                Err(error)
            }
        }
    }
}

/// Comprueba que el sujeto sigue donde se dejó antes de tocar nada.
fn ensure_present(path: &Path) -> Result<(), UndoError> {
    if std::fs::symlink_metadata(path).is_err() {
        return Err(UndoError::Vanished(path.to_path_buf()));
    }
    Ok(())
}

/// Ejecuta la inversa de `action`.
fn revert(action: &Action) -> Result<Action, UndoError> {
    if let Some(subject) = action.subject() {
        ensure_present(subject)?;
    }

    match action {
        Action::Renamed { from, to } => {
            let name = from
                .file_name()
                .ok_or_else(|| UndoError::Vanished(from.clone()))?;
            rename(to, name, ConflictPolicy::Fail)?;
            Ok(action.clone())
        }
        Action::Moved { from, to } => {
            let parent = from.parent().unwrap_or(Path::new("."));
            let landed = move_to(to, parent, ConflictPolicy::Fail)?;
            // Un «conservar ambos» pudo cambiar el nombre al mover; para que
            // deshacer devuelva el nombre EXACTO hay que corregirlo.
            if landed.destination != *from {
                let name = from
                    .file_name()
                    .ok_or_else(|| UndoError::Vanished(from.clone()))?;
                rename(&landed.destination, name, ConflictPolicy::Fail)?;
            }
            Ok(action.clone())
        }
        // A la papelera, no borrado: la regla del proyecto no tiene excepciones.
        Action::Copied { created } => {
            trash_one(created, &trash_policy()).map_err(|e| UndoError::Trash(e.to_string()))?;
            Ok(action.clone())
        }
        Action::DirectoryCreated { path } => {
            trash_one(path, &trash_policy()).map_err(|e| UndoError::Trash(e.to_string()))?;
            Ok(action.clone())
        }
        Action::Trashed { item } => {
            restore_item(item, ConflictPolicy::Fail)
                .map_err(|e| UndoError::Trash(e.to_string()))?;
            Ok(action.clone())
        }
        // Routed to `remote_undo` by `undo_with`; never reached.
        Action::RemoteCopied { .. }
        | Action::RemoteMoved { .. }
        | Action::RemoteRenamed { .. }
        | Action::RemoteDirectoryCreated { .. }
        | Action::NotUndoable { .. } => crate::remote_undo::revert(action, &no_drives()),
    }
}

/// Vuelve a aplicar `action` tras haberla deshecho.
fn reapply(action: &Action) -> Result<(), UndoError> {
    match action {
        Action::Renamed { from, to } => {
            ensure_present(from)?;
            let name = to
                .file_name()
                .ok_or_else(|| UndoError::Vanished(to.clone()))?;
            rename(from, name, ConflictPolicy::Fail)?;
            Ok(())
        }
        Action::Moved { from, to } => {
            ensure_present(from)?;
            let parent = to.parent().unwrap_or(Path::new("."));
            move_to(from, parent, ConflictPolicy::Fail)?;
            Ok(())
        }
        Action::Trashed { item } => {
            ensure_present(&item.original_path)?;
            trash_one(&item.original_path, &trash_policy())
                .map_err(|e| UndoError::Trash(e.to_string()))?;
            Ok(())
        }
        // Rehacer una copia o una carpeta creada exigiria repetir la operacion
        // original, cuyo origen esta fuera de lo que la pila registra.
        Action::Copied { .. } | Action::DirectoryCreated { .. } => {
            Err(UndoError::Trash("rehacer una copia no esta soportado".into()))
        }
        // Routed to `remote_undo` by `redo_with`; never reached.
        Action::RemoteCopied { .. }
        | Action::RemoteMoved { .. }
        | Action::RemoteRenamed { .. }
        | Action::RemoteDirectoryCreated { .. }
        | Action::NotUndoable { .. } => crate::remote_undo::reapply(action, &no_drives()),
    }
}
