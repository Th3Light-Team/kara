//! Travesía paralela (`jwalk`), búsqueda y vigilancia del sistema de ficheros
//! (`notify` / inotify).
//!
//! Es la capa que justifica el stack nativo: la travesía recursiva de `/usr`
//! (251 k ficheros) baja de 2,29 s en Python a 0,15 s en paralelo. Ninguna
//! travesía debe bloquear la UI ni asumir que el árbol termina.

#![forbid(unsafe_code)]

pub mod search;
pub mod size;
pub mod walk;
pub mod watch;

pub use search::{Query, SearchEvent, SearchScope, fold_for_search, search};
pub use size::{SizeProgress, SizeReport, folder_size};
pub use walk::{Cancel, WalkError, WalkItem, WalkOptions, WalkSummary, walk};
pub use watch::{Change, Coalescer, Watcher};
