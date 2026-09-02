//! Autocompletado de rutas para la barra de direcciones.
//!
//! Conveniencia de referencia: `ground/spec/01-navegacion.md`, «Autocompletado de
//! rutas».
//!
//! Lógica pura: aquí no se lista ningún directorio. Quien sí lo hace pasa las
//! entradas hijas ya leídas, y esta capa decide qué se ofrece y en qué orden.
//!
//! # La regla que manda sobre todas
//!
//! «SIN bloquear la escritura de una ruta que aún no existe: el autocompletado no
//! debe imponer una sugerencia si el usuario sigue tecleando.» Por eso nada aquí
//! modifica el texto: [`Suggestions::inline_remainder`] devuelve lo que *podría*
//! añadirse y [`Suggestions::accept`] solo actúa cuando alguien lo pide
//! explícitamente. Escribir una ruta que todavía no existe siempre funciona.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::entry::{EntryKind, FileEntry};
use crate::sort::{Collation, compare_names, fold_for_match};

/// El texto tecleado, partido en la carpeta que hay que listar y el prefijo que
/// filtra sus hijas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathInput {
    /// Carpeta cuyo contenido se ofrece.
    pub directory: PathBuf,
    /// Lo tecleado del último segmento, que puede estar vacío.
    pub prefix: String,
}

/// Parte lo tecleado por el último separador.
///
/// Es lo que hace que el autocompletado avance **segmento a segmento**: en cuanto
/// se escribe `/`, el prefijo queda vacío y se ofrece el contenido del nuevo
/// nivel, que es justo lo que pide la spec.
///
/// `/home/oli` → listar `/home`, prefijo `oli`
/// `/home/`    → listar `/home`, prefijo vacío
/// `/`         → listar `/`, prefijo vacío
#[must_use]
pub fn split_input(text: &str) -> PathInput {
    match text.rfind('/') {
        Some(cut) => PathInput {
            // El separador se conserva para que `/x` liste la raíz y no `""`.
            directory: PathBuf::from(&text[..=cut]),
            prefix: text[cut + 1..].to_string(),
        },
        None => PathInput {
            directory: PathBuf::new(),
            prefix: text.to_string(),
        },
    }
}

/// De dónde salió una sugerencia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Una carpeta que existe ahora mismo en el disco.
    Disk,
    /// Una ruta que el usuario escribió antes. Puede no existir ya.
    History,
}

/// Una candidata a completar.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// Ruta completa que quedaría en la barra al aceptarla.
    pub path: PathBuf,
    /// Nombre del segmento, que es lo que se pinta en la lista.
    pub name: OsString,
    pub source: Source,
}

/// Genera sugerencias respetando la sensibilidad a mayúsculas del sistema de
/// ficheros, que se recibe porque `kara-core` no puede consultarla: ext4 y APFS
/// no se comportan igual y preguntarlo es I/O.
#[derive(Debug, Clone)]
pub struct Completer {
    pub case_sensitive: bool,
}

impl Default for Completer {
    fn default() -> Self {
        // Linux, el objetivo del proyecto, distingue mayúsculas.
        Self {
            case_sensitive: true,
        }
    }
}

impl Completer {
    fn starts_with(&self, name: &str, prefix: &str) -> bool {
        if self.case_sensitive {
            name.starts_with(prefix)
        } else {
            fold_for_match(name).starts_with(&fold_for_match(prefix))
        }
    }

    /// Ofrece las carpetas hijas que empiezan por el prefijo, más las rutas del
    /// historial que continúan lo tecleado.
    ///
    /// Solo carpetas: la barra de direcciones navega, y ofrecer ficheros llevaría
    /// a rutas que no se pueden abrir como ubicación.
    ///
    /// El historial va después del disco y sin repetir lo que el disco ya ofrece:
    /// lo que existe ahora pesa más que lo que se escribió una vez.
    #[must_use]
    pub fn suggest(
        &self,
        input: &PathInput,
        children: &[FileEntry],
        history: &[PathBuf],
    ) -> Suggestions {
        let collation = Collation {
            case_sensitive: self.case_sensitive,
            natural_numeric: true,
        };

        let mut disk: Vec<&FileEntry> = children
            .iter()
            .filter(|e| e.kind == EntryKind::Directory)
            .filter(|e| self.starts_with(&e.display, &input.prefix))
            .collect();
        disk.sort_by(|a, b| compare_names(&a.display, &b.display, &collation));

        let mut items: Vec<Suggestion> = disk
            .into_iter()
            .map(|entry| Suggestion {
                path: input.directory.join(&entry.name),
                name: entry.name.clone(),
                source: Source::Disk,
            })
            .collect();

        let typed = input.directory.join(&input.prefix);
        for path in history {
            let continues = self.starts_with(
                &path.to_string_lossy(),
                &typed.to_string_lossy(),
            );
            if continues && !items.iter().any(|s| s.path == *path) {
                items.push(Suggestion {
                    name: path
                        .file_name()
                        .map_or_else(|| OsString::from(path), std::ffi::OsStr::to_os_string),
                    path: path.clone(),
                    source: Source::History,
                });
            }
        }

        Suggestions {
            items,
            cursor: None,
            prefix: input.prefix.clone(),
        }
    }
}

/// Lista de sugerencias con el cursor que recorren Tab y las flechas.
#[derive(Debug, Clone, Default)]
pub struct Suggestions {
    items: Vec<Suggestion>,
    cursor: Option<usize>,
    prefix: String,
}

impl Suggestions {
    #[must_use]
    pub fn items(&self) -> &[Suggestion] {
        &self.items
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// La sugerencia resaltada, si alguien ha recorrido la lista.
    #[must_use]
    pub fn selected(&self) -> Option<&Suggestion> {
        self.cursor.and_then(|i| self.items.get(i))
    }

    /// Tab / flecha abajo. Da la vuelta al llegar al final.
    pub fn next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.cursor = Some(match self.cursor {
            Some(i) => (i + 1) % self.items.len(),
            None => 0,
        });
    }

    /// Shift+Tab / flecha arriba.
    pub fn previous(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.cursor = Some(match self.cursor {
            Some(0) | None => self.items.len() - 1,
            Some(i) => i - 1,
        });
    }

    /// Esc: cierra la lista. No toca el texto tecleado, que es de quien escribe.
    pub fn dismiss(&mut self) {
        self.cursor = None;
        self.items.clear();
    }

    /// Enter: la ruta a la que navegar, o `None` si no hay nada resaltado — en
    /// cuyo caso navega lo que el usuario haya escrito, exista o no.
    #[must_use]
    pub fn accept(&self) -> Option<&Path> {
        self.selected().map(|s| s.path.as_path())
    }

    /// Lo que faltaría por escribir para llegar a la primera sugerencia, para
    /// pintarlo en línea como texto seleccionado.
    ///
    /// Es una propuesta, no una imposición: quien sigue tecleando la sustituye.
    /// Devuelve `None` si no hay sugerencias o si el prefijo no encaja con la
    /// primera, que es lo que pasa cuando el sistema no distingue mayúsculas y
    /// completar en línea desordenaría lo escrito.
    #[must_use]
    pub fn inline_remainder(&self) -> Option<&str> {
        let first = self.items.first()?;
        let name = first.name.to_str()?;
        name.strip_prefix(self.prefix.as_str())
    }
}
