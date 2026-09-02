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
pub mod conflict;

pub use clock::local_utc_offset_seconds;
pub use batch::{BatchPolicy, BatchReport, Failure, FailureAction, FailureKind};
pub use conflict::{
    ConflictKind, ConflictPolicy, Resolution, ResolutionCounts, split_name, unique_name,
};
