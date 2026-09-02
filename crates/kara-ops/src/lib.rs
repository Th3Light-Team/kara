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
