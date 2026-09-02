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

use crate::clock::trash_policy;

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
        }
    }

    /// Dónde quedó el resultado, que es lo que hay que comprobar antes de
    /// deshacer.
    fn subject(&self) -> &Path {
        match self {
            Self::Moved { to, .. } | Self::Renamed { to, .. } => to,
            Self::Copied { created } => created,
            Self::DirectoryCreated { path } => path,
            Self::Trashed { item } => &item.trashed_path,
        }
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

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
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
        let action = self.done.pop().ok_or(UndoError::Empty)?;
        match revert(&action) {
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
        let action = self.undone.pop().ok_or(UndoError::Empty)?;
        match reapply(&action) {
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
    ensure_present(action.subject())?;

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
    }
}
