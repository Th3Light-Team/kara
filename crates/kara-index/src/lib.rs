//! Travesía paralela (`jwalk`), búsqueda y vigilancia del sistema de ficheros
//! (`notify` / inotify).
//!
//! Es la capa que justifica el stack nativo: la travesía recursiva de `/usr`
//! (251 k ficheros) baja de 2,29 s en Python a 0,15 s en paralelo. Ninguna
//! travesía debe bloquear la UI ni asumir que el árbol termina.

#![forbid(unsafe_code)]
