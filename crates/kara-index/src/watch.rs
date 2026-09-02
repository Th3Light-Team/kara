//! Vigilancia del directorio en pantalla, para que la vista no se quede rancia.
//!
//! Conveniencia de referencia: `ground/spec/01-navegacion.md`, «Refrescar /
//! recargar la vista»: «la vista debería refrescarse automáticamente mediante
//! vigilancia del sistema de archivos (inotify en Linux)», y el refresco manual
//! queda para «recursos de red, volúmenes montados, sistemas de archivos sin
//! notificaciones» donde la vigilancia falla.
//!
//! # Por qué hay un fusionador aparte
//!
//! inotify no entrega un cambio por acción del usuario: descomprimir un `.tar`
//! con mil ficheros produce miles de eventos en milisegundos, y volver a listar
//! con cada uno congela la interfaz. [`Coalescer`] junta una ráfaga en el
//! conjunto mínimo de cambios que la vista necesita, y es lógica pura y
//! testeable sin tocar el disco ni esperar a un reloj.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use notify::{EventKind, RecursiveMode, Watcher as _, event::ModifyKind, event::RenameMode};

/// Lo que la vista necesita saber que ha pasado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Apareció algo nuevo.
    Appeared(PathBuf),
    /// Desapareció.
    Vanished(PathBuf),
    /// Cambió su contenido o sus metadatos: hay que rehacer su `stat`.
    Touched(PathBuf),
    /// Cambió de nombre dentro de la misma carpeta.
    Renamed { from: PathBuf, to: PathBuf },
    /// Se perdieron eventos o pasó algo que no se puede describir por partes:
    /// hay que releer la carpeta entera. inotify tiene cola finita y la
    /// desborda un `rm -rf` grande, así que este caso ocurre de verdad.
    Rescan,
}

/// Junta una ráfaga de eventos en el conjunto mínimo de cambios.
///
/// Las reglas son las que evitan trabajo inútil en la vista:
///
/// - Aparecer y luego desaparecer se anulan; no queda rastro.
/// - Varios `Touched` sobre la misma ruta valen por uno.
/// - Un `Touched` sobre algo que acaba de aparecer se absorbe en el
///   `Appeared`: la vista va a hacer el `stat` de todas formas.
/// - Un [`Change::Rescan`] descarta todo lo anterior, porque releer la carpeta
///   entera ya lo cubre.
#[derive(Debug, Default)]
pub struct Coalescer {
    changes: Vec<Change>,
    rescan: bool,
}

impl Coalescer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, change: Change) {
        if self.rescan {
            return;
        }
        match change {
            Change::Rescan => {
                self.changes.clear();
                self.rescan = true;
            }
            Change::Vanished(path) => {
                // Si habia aparecido en esta misma rafaga, nunca llego a
                // existir para la vista.
                let appeared = self
                    .changes
                    .iter()
                    .any(|c| matches!(c, Change::Appeared(p) if *p == path));
                self.forget(&path);
                if !appeared {
                    self.changes.push(Change::Vanished(path));
                }
            }
            Change::Touched(path) => {
                let known = self.changes.iter().any(|c| match c {
                    Change::Appeared(p) | Change::Touched(p) => *p == path,
                    Change::Renamed { to, .. } => *to == path,
                    _ => false,
                });
                if !known {
                    self.changes.push(Change::Touched(path));
                }
            }
            Change::Appeared(path) => {
                self.forget(&path);
                self.changes.push(Change::Appeared(path));
            }
            Change::Renamed { from, to } => {
                self.forget(&from);
                self.forget(&to);
                self.changes.push(Change::Renamed { from, to });
            }
        }
    }

    fn forget(&mut self, path: &Path) {
        self.changes.retain(|c| match c {
            Change::Appeared(p) | Change::Vanished(p) | Change::Touched(p) => p != path,
            Change::Renamed { from, to } => from != path && to != path,
            Change::Rescan => true,
        });
    }

    /// `true` si no hay nada que aplicar.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.rescan && self.changes.is_empty()
    }

    /// Vacía el fusionador y devuelve lo que hay que aplicar.
    pub fn drain(&mut self) -> Vec<Change> {
        if self.rescan {
            self.rescan = false;
            self.changes.clear();
            return vec![Change::Rescan];
        }
        std::mem::take(&mut self.changes)
    }
}

/// Traduce un evento de `notify` al vocabulario de la vista.
///
/// Lo que no se sabe describir se convierte en [`Change::Rescan`] en vez de
/// adivinar: releer de más es barato, enseñar una carpeta equivocada no.
#[must_use]
pub fn translate(event: &notify::Event) -> Vec<Change> {
    let paths = &event.paths;
    match event.kind {
        EventKind::Create(_) => paths.iter().cloned().map(Change::Appeared).collect(),
        EventKind::Remove(_) => paths.iter().cloned().map(Change::Vanished).collect(),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if paths.len() == 2 => {
            vec![Change::Renamed {
                from: paths[0].clone(),
                to: paths[1].clone(),
            }]
        }
        // Un renombrado del que solo llega una mitad no se puede emparejar sin
        // inventar; releer resuelve las dos.
        EventKind::Modify(ModifyKind::Name(_)) => vec![Change::Rescan],
        EventKind::Modify(_) => paths.iter().cloned().map(Change::Touched).collect(),
        _ => vec![Change::Rescan],
    }
}

/// Vigila una carpeta y entrega los cambios por un canal.
///
/// Solo el nivel directo: la vista enseña una carpeta, no su árbol, y vigilar
/// recursivamente un `$HOME` grande cuesta un descriptor por subcarpeta.
pub struct Watcher {
    _inner: notify::RecommendedWatcher,
    events: Receiver<Change>,
}

impl Watcher {
    /// Empieza a vigilar `directory`.
    pub fn watch(directory: &Path) -> Result<Self, notify::Error> {
        let (tx, rx) = channel();
        let mut inner = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            let changes = match result {
                Ok(event) => translate(&event),
                // Un error de la propia vigilancia —cola desbordada, entre
                // otros— significa que se perdieron eventos.
                Err(_) => vec![Change::Rescan],
            };
            for change in changes {
                if tx.send(change).is_err() {
                    break;
                }
            }
        })?;
        inner.watch(directory, RecursiveMode::NonRecursive)?;
        Ok(Self {
            _inner: inner,
            events: rx,
        })
    }

    /// Recoge lo que haya llegado sin bloquear, ya fusionado.
    ///
    /// Devuelve la lista vacía cuando no ha pasado nada, que es el caso normal
    /// en cada latido de la interfaz.
    pub fn poll(&self) -> Vec<Change> {
        let mut coalescer = Coalescer::new();
        while let Ok(change) = self.events.try_recv() {
            coalescer.push(change);
        }
        coalescer.drain()
    }
}
