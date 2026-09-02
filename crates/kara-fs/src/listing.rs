//! Listado de un directorio: `scandir` + `stat` a `FileEntry`.
//!
//! Es la primitiva sobre la que se apoya toda la vista. `kara-core` ordena,
//! agrupa y filtra lo que produce este módulo, y nunca toca el disco.
//!
//! # Invariantes
//!
//! - **Una entrada que falla no aborta el listado.** Si el `stat` de un fichero
//!   falla —permisos, carrera con quien lo borra, enlace roto— se devuelve la
//!   entrada con los metadatos que se hayan podido leer y el fallo se anota
//!   aparte. Perder la carpeta entera por un fichero es la regla que el proyecto
//!   prohíbe explícitamente.
//! - **Dos `stat` por entrada, no uno.** `symlink_metadata` describe el enlace y
//!   `metadata` su destino: hacen falta los dos para distinguir un enlace a
//!   directorio (que agrupa con las carpetas) de uno roto (que no).
//! - **El tamaño de un directorio es `None`**, no cero. Calcularlo es recursivo y
//!   vive en `kara-index`; devolver `0` mentiría y ordenar por tamaño pondría
//!   todas las carpetas juntas como si estuvieran vacías.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use kara_core::{EntryKind, FileEntry, MetadataBag};

/// Un fallo al leer una entrada concreta. No detiene el listado.
#[derive(Debug, thiserror::Error)]
#[error("{path}: {source}")]
pub struct EntryError {
    pub path: PathBuf,
    #[source]
    pub source: std::io::Error,
}

/// Lo que produce listar un directorio.
#[derive(Debug, Default)]
pub struct Listing {
    /// Entradas leídas, en el orden que dio el sistema de ficheros: sin ordenar.
    /// Ordenarlas es cosa de `kara_core::sort`.
    pub entries: Vec<FileEntry>,
    /// Entradas que no se pudieron describir del todo.
    pub errors: Vec<EntryError>,
}

/// Nombres extra a ocultar, leídos del `.hidden` de la carpeta.
///
/// La spec pide respetarlo en Linux. Un `.hidden` ilegible o inexistente no es un
/// error: simplemente no oculta nada.
#[must_use]
pub fn read_hidden_file(directory: &Path) -> std::collections::BTreeSet<OsString> {
    fs::read_to_string(directory.join(".hidden"))
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(OsString::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Lista `directory`.
///
/// El error de nivel superior solo se devuelve si no se pudo abrir el directorio
/// en absoluto. A partir de ahí, cada entrada que falle se anota en
/// [`Listing::errors`] y el resto se devuelve igualmente.
pub fn list_directory(directory: &Path) -> Result<Listing, EntryError> {
    let iter = fs::read_dir(directory).map_err(|source| EntryError {
        path: directory.to_path_buf(),
        source,
    })?;

    let mut listing = Listing::default();

    for item in iter {
        let dir_entry = match item {
            Ok(entry) => entry,
            Err(source) => {
                listing.errors.push(EntryError {
                    path: directory.to_path_buf(),
                    source,
                });
                continue;
            }
        };
        let path = dir_entry.path();
        match describe(&path) {
            Ok(entry) => listing.entries.push(entry),
            Err(error) => {
                // Aun sin metadatos, el nombre existe: se muestra la entrada en
                // vez de hacerla desaparecer de la vista.
                listing.entries.push(bare_entry(&path));
                listing.errors.push(error);
            }
        }
    }

    Ok(listing)
}

/// Describe una ruta suelta. Es lo que necesita el diálogo de propiedades.
pub fn describe(path: &Path) -> Result<FileEntry, EntryError> {
    let link_meta = fs::symlink_metadata(path).map_err(|source| EntryError {
        path: path.to_path_buf(),
        source,
    })?;
    let is_symlink = link_meta.file_type().is_symlink();

    // Del destino si es un enlace; del propio fichero si no. Un enlace roto deja
    // `None` y se trata como fichero, que es lo que dice el contrato de EntryKind.
    let target_meta = if is_symlink {
        fs::metadata(path).ok()
    } else {
        Some(link_meta.clone())
    };
    let symlink_broken = is_symlink && target_meta.is_none();

    let kind = match &target_meta {
        Some(meta) if meta.is_dir() => EntryKind::Directory,
        _ => EntryKind::File,
    };

    // Los tiempos y el tamaño se leen del enlace, no del destino: es lo que
    // describe la entrada que se está listando.
    let size = match kind {
        // Recursivo, y por tanto de kara-index. `None` no es cero.
        EntryKind::Directory => None,
        EntryKind::File => Some(link_meta.len()),
    };

    let name = path
        .file_name()
        .map_or_else(|| OsString::from(path), std::ffi::OsStr::to_os_string);
    let display = name.to_string_lossy().into_owned();

    Ok(FileEntry {
        is_hidden: display.starts_with('.'),
        name,
        display,
        kind,
        is_symlink,
        symlink_broken,
        size,
        modified: link_meta.modified().ok(),
        created: link_meta.created().ok(),
        accessed: link_meta.accessed().ok(),
        type_label: None,
        location: path.parent().map(Path::to_path_buf),
        extra: MetadataBag::new(),
    })
}

/// Entrada con lo único que se sabe seguro —el nombre— cuando el `stat` falla.
fn bare_entry(path: &Path) -> FileEntry {
    let name = path
        .file_name()
        .map_or_else(|| OsString::from(path), std::ffi::OsStr::to_os_string);
    let display = name.to_string_lossy().into_owned();
    FileEntry {
        is_hidden: display.starts_with('.'),
        name,
        display,
        kind: EntryKind::File,
        is_symlink: false,
        symlink_broken: false,
        size: None,
        modified: None,
        created: None,
        accessed: None,
        type_label: None,
        location: path.parent().map(Path::to_path_buf),
        extra: MetadataBag::new(),
    }
}
