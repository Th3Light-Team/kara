//! Travesía paralela y cancelable del sistema de ficheros.
//!
//! Es el motor sobre el que se apoyan la búsqueda y el cálculo de tamaño.
//!
//! Medido con este código sobre `/usr` (292 k entradas) en esta máquina: una
//! búsqueda recursiva completa, emparejado incluido, tarda **0,35 s**, y sumar el
//! tamaño de las 268 k ficheros, **0,83 s**. La referencia que motivó elegir un
//! stack nativo — el mismo recorrido en Python — tardaba 2,29 s. Cancelar es
//! inmediato a efectos humanos: pedida la parada a los 120 ms, la llamada
//! devuelve a los 130 ms.
//!
//! # Invariantes
//!
//! - **Un fallo no aborta el recorrido.** Una carpeta sin permiso de lectura se
//!   anota en [`WalkSummary::errors`] y la travesía continúa. Es la misma regla
//!   que el proyecto exige para las operaciones por lotes.
//! - **La legibilidad de cada directorio se comprueba a mano.** Medido: `jwalk`
//!   0.9 entrega una carpeta ilegible como una entrada correcta y **no emite
//!   ningún error** — ni como item, ni por `process_read_dir`; su campo
//!   `read_children.error` es `pub(crate)`. Sin esta comprobación, una carpeta
//!   sin permiso es indistinguible de una vacía y el total sale menor en
//!   silencio, que es justo lo que la spec prohíbe para el tamaño de carpeta.
//!   Cuesta un `access(2)` por directorio, no por entrada.
//! - **La cancelación es cooperativa** y se comprueba por entrada: cancelar es
//!   inmediato a efectos humanos sin matar hilos a media syscall.
//! - **Los enlaces simbólicos no se siguen por defecto**, que es lo que evita
//!   bucles infinitos y contar dos veces.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Testigo de cancelación compartido entre quien lanza la travesía y quien la
/// para. Clonarlo comparte el mismo estado.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pide la parada. Idempotente.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Un fallo en una ruta concreta. No detiene el recorrido.
#[derive(Debug, thiserror::Error)]
#[error("{path}: {source}")]
pub struct WalkError {
    pub path: PathBuf,
    #[source]
    pub source: std::io::Error,
}

/// Una entrada encontrada.
#[derive(Debug, Clone)]
pub struct WalkItem {
    pub path: PathBuf,
    /// Profundidad desde la raíz; la propia raíz es 0.
    pub depth: usize,
    pub is_dir: bool,
    pub is_symlink: bool,
}

/// Qué recorrer.
#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// `None` = sin límite. `Some(1)` = solo el contenido directo.
    pub max_depth: Option<usize>,
    /// Seguir enlaces simbólicos. Por defecto `false`: ver los invariantes.
    pub follow_links: bool,
    /// Incluir entradas ocultas (las que empiezan por `.`).
    pub include_hidden: bool,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            max_depth: None,
            follow_links: false,
            include_hidden: true,
        }
    }
}

/// Cómo terminó el recorrido.
#[derive(Debug, Default)]
pub struct WalkSummary {
    /// Entradas entregadas al llamante, sin contar la raíz.
    pub visited: u64,
    /// Rutas que fallaron, con su error. El recorrido siguió pese a ellas.
    pub errors: Vec<WalkError>,
    /// `true` si se paró por petición y no por haber terminado.
    pub cancelled: bool,
}

/// Recorre `root` en paralelo, entregando cada entrada a `on_item`.
///
/// La raíz no se entrega: se recorren sus descendientes. Devuelve el resumen aun
/// habiendo cancelado o fallado, porque un recorrido parcial sigue siendo
/// información útil (la spec la pide explícitamente para el tamaño de carpeta).
pub fn walk(
    root: &Path,
    options: &WalkOptions,
    cancel: &Cancel,
    mut on_item: impl FnMut(WalkItem),
) -> WalkSummary {
    let mut summary = WalkSummary::default();

    let mut builder = jwalk::WalkDir::new(root)
        .skip_hidden(!options.include_hidden)
        .follow_links(options.follow_links);
    if let Some(depth) = options.max_depth {
        builder = builder.max_depth(depth);
    }

    for entry in builder {
        if cancel.is_cancelled() {
            summary.cancelled = true;
            break;
        }
        match entry {
            Ok(entry) => {
                if entry.depth() == 0 {
                    continue;
                }
                if entry.file_type().is_dir()
                    && let Err(errno) = rustix::fs::access(
                        entry.path().as_path(),
                        rustix::fs::Access::READ_OK | rustix::fs::Access::EXEC_OK,
                    )
                {
                    summary.errors.push(WalkError {
                        path: entry.path(),
                        source: std::io::Error::from_raw_os_error(errno.raw_os_error()),
                    });
                }

                summary.visited += 1;
                on_item(WalkItem {
                    is_dir: entry.file_type().is_dir(),
                    is_symlink: entry.path_is_symlink(),
                    depth: entry.depth(),
                    path: entry.path(),
                });
            }
            Err(error) => {
                let path = error.path().unwrap_or(root).to_path_buf();
                summary.errors.push(WalkError {
                    path,
                    source: error.into_io_error().unwrap_or_else(|| {
                        std::io::Error::other("error de travesía sin causa de E/S")
                    }),
                });
            }
        }
    }

    summary
}
