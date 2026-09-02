//! Cálculo recursivo del tamaño de una carpeta, en segundo plano y cancelable.
//!
//! Conveniencia de referencia: `ground/spec/05-operaciones.md`, «Cálculo del
//! tamaño de carpeta». Alimenta además la fase «Calculando…» previa a copiar.
//!
//! # Los tres casos borde que la spec nombra y son fáciles de fallar
//!
//! - **Enlaces simbólicos**: no se siguen. Seguirlos contaría el destino dos
//!   veces y, con un enlace a un ancestro, no terminaría nunca.
//! - **Enlaces duros**: un fichero con varios nombres ocupa disco una sola vez.
//!   Se lleva registro de los `(dispositivo, inodo)` ya contados, y solo de los
//!   que tienen más de un nombre: guardarlos todos costaría memoria proporcional
//!   al árbol sin cambiar el resultado.
//! - **Carpetas sin permiso**: el total queda marcado como parcial y se anota qué
//!   falló, en vez de mentir con una cifra menor sin avisar.

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::walk::{Cancel, WalkError, WalkOptions, walk};

/// Lo acumulado hasta ahora. Se emite en vivo para el «Calculando… 1.234
/// archivos, 5,6 GB» que pide la spec.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SizeProgress {
    pub files: u64,
    pub directories: u64,
    /// Suma de tamaños lógicos: lo que dicen los ficheros que miden.
    pub logical: u64,
    /// Espacio realmente ocupado en disco (bloques de 512 B). Difiere del lógico
    /// con ficheros dispersos y con el relleno del último bloque.
    pub on_disk: u64,
}

/// Resultado final del cálculo.
#[derive(Debug)]
pub struct SizeReport {
    pub totals: SizeProgress,
    /// Rutas que no se pudieron leer. Su contenido no está en los totales.
    pub errors: Vec<WalkError>,
    pub cancelled: bool,
}

impl SizeReport {
    /// `true` si los totales no cubren todo el árbol, por cancelación o por
    /// rutas ilegibles. La UI debe decirlo en vez de presentar la cifra como
    /// definitiva.
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.cancelled || !self.errors.is_empty()
    }
}

/// Cada cuántas entradas se avisa del avance.
const REPORT_EVERY: u64 = 256;

/// Suma recursivamente el contenido de `root`.
///
/// `on_progress` se llama periódicamente y al final, para que el diálogo se
/// actualice en vivo sin congelarse. El cálculo se puede parar con `cancel` y
/// los totales parciales siguen siendo válidos.
pub fn folder_size(
    root: &Path,
    cancel: &Cancel,
    mut on_progress: impl FnMut(SizeProgress),
) -> SizeReport {
    let mut totals = SizeProgress::default();
    let mut counted_hard_links: HashSet<(u64, u64)> = HashSet::new();
    let mut since_report = 0u64;
    let mut errors = Vec::new();

    let options = WalkOptions {
        follow_links: false,
        ..WalkOptions::default()
    };

    let summary = walk(root, &options, cancel, |item| {
        if item.is_dir && !item.is_symlink {
            totals.directories += 1;
        } else {
            // symlink_metadata: el peso del enlace, no el de su destino.
            match std::fs::symlink_metadata(&item.path) {
                Ok(metadata) => {
                    let already_counted = metadata.nlink() > 1
                        && !counted_hard_links.insert((metadata.dev(), metadata.ino()));
                    if !already_counted {
                        totals.files += 1;
                        totals.logical += metadata.len();
                        totals.on_disk += metadata.blocks() * 512;
                    }
                }
                Err(source) => errors.push(WalkError {
                    path: item.path.clone(),
                    source,
                }),
            }
        }

        since_report += 1;
        if since_report >= REPORT_EVERY {
            since_report = 0;
            on_progress(totals);
        }
    });

    errors.extend(summary.errors);
    on_progress(totals);

    SizeReport {
        totals,
        errors,
        cancelled: summary.cancelled,
    }
}
