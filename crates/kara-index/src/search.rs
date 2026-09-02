//! Búsqueda por nombre, con alcance y resultados progresivos.
//!
//! Conveniencias de referencia: `ground/spec/04-busqueda.md` — «Alcance de
//! búsqueda», «Resultados incrementales al escribir», «Coincidencia sin
//! distinción de mayúsculas ni acentos» y «Progreso y estado vacío».
//!
//! # Por qué el plegado de acentos vive aquí y no en la collation
//!
//! `kara_core::sort::Collation` pliega mayúsculas pero **no** acentos, y es
//! deliberado: al ordenar, `arbol` y `árbol` son nombres distintos y deben
//! quedar en sitios distintos. Al buscar, quien teclea `arbol` espera encontrar
//! `Árbol.txt`. Son dos relaciones distintas sobre el mismo texto, así que cada
//! una tiene su función.

use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use crate::walk::{Cancel, WalkError, WalkOptions, walk};

/// Dónde busca la consulta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchScope {
    /// Solo el contenido directo de la carpeta.
    CurrentFolder,
    /// La carpeta y todas sus subcarpetas. Es el defecto recomendado por la spec
    /// para el objetivo tipo Windows 11.
    #[default]
    Subfolders,
    /// Todas las ubicaciones. Se modela como una travesía desde varias raíces.
    Everywhere,
}

/// Lo que va ocurriendo durante la búsqueda.
///
/// Es un flujo y no un `Vec` porque la spec exige resultados progresivos, un
/// indicador de «buscando» distinguible de «sin resultados», y cancelación con
/// Esc. Devolver la lista entera al final impediría las tres cosas.
#[derive(Debug)]
pub enum SearchEvent {
    /// Una coincidencia. Llegan primero las de la carpeta actual.
    Match(PathBuf),
    /// Latido de avance, para mover el indicador sin inundar a quien escucha.
    Progress { visited: u64 },
    /// Una ruta que no se pudo leer. La búsqueda continúa.
    Failed(WalkError),
    /// Fin. `cancelled` distingue «no hay nada» de «me pararon».
    Done { matches: u64, cancelled: bool },
}

/// Normaliza un texto para comparar: sin acentos y sin distinguir mayúsculas.
///
/// Es exactamente `kara_core::fold_for_match` aplicado al texto **sin
/// diacríticos**: buscar pliega una relación más que ordenar, y solo esa. Definir
/// aquí un plegado de caja propio haría que un día divergieran sin que nadie se
/// diera cuenta.
///
/// El orden importa. Primero se quitan los diacríticos —NFD separa la letra base
/// de sus marcas y se descartan las combinantes—, lo que de paso resuelve los
/// pocos caracteres cuya minúscula ocupa varios `char` y que
/// `fold_for_match` deja intactos a propósito: `İ` (U+0130) se descompone en `I`
/// más un punto combinante, así que llega al plegado de caja como una `I` normal
/// y `istanbul` encuentra `İSTANBUL`.
#[must_use]
pub fn fold_for_search(text: &str) -> String {
    let without_marks: String = text
        .nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect();
    kara_core::fold_for_match(&without_marks)
}

/// Consulta ya normalizada. Construirla una vez evita replegar el término por
/// cada una de las 250 000 entradas de un árbol grande.
#[derive(Debug, Clone)]
pub struct Query {
    folded: String,
}

impl Query {
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            folded: fold_for_search(text),
        }
    }

    /// Una consulta vacía no filtra nada y no debe lanzar travesía alguna.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.folded.is_empty()
    }

    /// Coincidencia por subcadena, como esperan Windows y Dolphin: `for` encuentra
    /// `informe.pdf`.
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        !self.folded.is_empty() && fold_for_search(name).contains(&self.folded)
    }
}

/// Cada cuántas entradas se emite un latido de progreso.
const PROGRESS_EVERY: u64 = 512;

/// Busca `query` bajo `root` con el alcance dado, emitiendo eventos conforme
/// aparecen.
///
/// Con [`SearchScope::Subfolders`] se recorre **primero el nivel directo** y
/// luego el resto del árbol: la spec pide mostrar antes las coincidencias de la
/// carpeta actual y traer las subcarpetas de forma progresiva, porque en un árbol
/// enorme una consulta de un carácter tarda y lo cercano es lo que el usuario
/// suele buscar.
pub fn search(
    root: &Path,
    query: &Query,
    scope: SearchScope,
    cancel: &Cancel,
    mut on_event: impl FnMut(SearchEvent),
) {
    if query.is_empty() {
        on_event(SearchEvent::Done {
            matches: 0,
            cancelled: false,
        });
        return;
    }

    let mut matches = 0u64;
    let mut visited = 0u64;

    let mut sweep = |options: WalkOptions, min_depth: usize, on: &mut dyn FnMut(SearchEvent)| {
        let summary = walk(root, &options, cancel, |item| {
            if item.depth < min_depth {
                return;
            }
            visited += 1;
            if visited.is_multiple_of(PROGRESS_EVERY) {
                on(SearchEvent::Progress { visited });
            }
            let name = item.path.file_name().unwrap_or_default().to_string_lossy();
            if query.matches(&name) {
                matches += 1;
                on(SearchEvent::Match(item.path));
            }
        });
        for error in summary.errors {
            on(SearchEvent::Failed(error));
        }
        summary.cancelled
    };

    let shallow = WalkOptions {
        max_depth: Some(1),
        ..WalkOptions::default()
    };

    let cancelled = match scope {
        SearchScope::CurrentFolder => sweep(shallow, 1, &mut on_event),
        // Primero el nivel directo; después el resto, saltando lo ya visto.
        SearchScope::Subfolders | SearchScope::Everywhere => {
            let stopped = sweep(shallow, 1, &mut on_event);
            if stopped {
                true
            } else {
                sweep(WalkOptions::default(), 2, &mut on_event)
            }
        }
    };

    on_event(SearchEvent::Done { matches, cancelled });
}
