//! Cola de operaciones de fichero: progreso con velocidad/ETA, resolución de
//! conflictos, manejo de errores por lote y pila de deshacer/rehacer.
//!
//! Conveniencias de referencia: `ground/spec/05-operaciones.md`.
//!
//! Invariantes:
//! - Mover, copiar, renombrar y crear entran en la pila de deshacer.
//! - Un fallo en un elemento no aborta el lote: reintentar / omitir / cancelar.
//! - Nada silencioso: fase "Calculando…", progreso y conflictos explícitos.

#![forbid(unsafe_code)]

pub mod batch;
pub mod clock;
pub mod location;
pub mod progress;
pub mod queue;
pub mod runner;
pub mod undo;
pub mod conflict;
mod remote_undo;

pub use clock::{local_utc_offset_seconds, trash_policy};
pub use batch::{BatchPolicy, BatchReport, ErrorDecision, Failure, FailureKind};
pub use conflict::{
    ConflictDecisions, ConflictKind, Resolution, ResolutionCounts,
};
// Reexportados desde kara-core, donde viven para que kara-fs pueda usarlos.
pub use kara_core::{split_name, unique_name};
pub use undo::{Action, UndoError, UndoStack};
pub use location::{
    BackendResolver, LocationOpError, LocationRequest, RequestError, create_dir_at, display_path,
    failure_kind, no_drives, parse_display_path, rename_at,
};
pub use progress::{Eta, Meter, Phase, humanize};
pub use queue::{Concurrency, Job, JobId, JobState, Kind, Queue};
