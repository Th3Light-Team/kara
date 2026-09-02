//! Visibilidad de entradas y presentación de nombres.
//!
//! Conveniencias de referencia: `ground/spec/03-vistas.md` — «Mostrar/ocultar
//! archivos ocultos» y «Mostrar extensiones de nombre de archivo» — y
//! `04-busqueda.md`, «Filtro en vivo por nombre».
//!
//! Lógica pura: aquí no se lee ningún `.hidden` ni se consulta ningún permiso.
//! Quien sí toca el disco pasa lo que ha averiguado.

use std::collections::BTreeSet;
use std::ffi::OsString;

use crate::entry::FileEntry;
use crate::sort::fold_for_match;

/// Qué se considera oculto y si se muestra.
#[derive(Debug, Clone, Default)]
pub struct Visibility {
    /// Mostrar los ocultos. Suelen dibujarse atenuados; eso lo decide la vista.
    pub show_hidden: bool,
    /// Nombres extra a ocultar, leídos del `.hidden` de la carpeta.
    ///
    /// La spec pide respetarlo en Linux, pero leer ese fichero es I/O: `kara-fs`
    /// lo parsea y entrega el conjunto ya resuelto.
    pub hidden_names: BTreeSet<OsString>,
}

impl Visibility {
    /// `true` si la entrada está oculta, por punto inicial o por `.hidden`.
    #[must_use]
    pub fn is_hidden(&self, entry: &FileEntry) -> bool {
        entry.is_hidden || self.hidden_names.contains(&entry.name)
    }

    /// `true` si la entrada debe aparecer en la vista.
    #[must_use]
    pub fn is_visible(&self, entry: &FileEntry) -> bool {
        self.show_hidden || !self.is_hidden(entry)
    }

    /// Deja solo lo visible, conservando el orden.
    pub fn retain_visible(&self, entries: &mut Vec<FileEntry>) {
        entries.retain(|entry| self.is_visible(entry));
    }
}

/// Cómo se escribe el nombre de una entrada en la vista.
#[derive(Debug, Clone)]
pub struct NameDisplay {
    /// Mostrar la extensión. El defecto es `true`, el valor seguro: en Dolphin,
    /// Nautilus y Nemo siempre se ven, y ocultarlas es un hábito de Windows.
    pub show_extensions: bool,
    /// Mostrar la extensión de ejecutables y scripts **aunque** el resto estén
    /// ocultas. Es una salvaguarda de seguridad, no una preferencia estética:
    /// con las extensiones ocultas, `factura.pdf.exe` se lee como `factura.pdf`.
    pub always_show_for_executables: bool,
}

impl Default for NameDisplay {
    fn default() -> Self {
        Self {
            show_extensions: true,
            always_show_for_executables: true,
        }
    }
}

impl NameDisplay {
    /// Nombre a pintar.
    ///
    /// `executable` lo aporta quien haya hecho el `stat`: en Linux ser ejecutable
    /// es un bit de modo, no un sufijo, y `kara-core` no consulta permisos.
    ///
    /// Las carpetas conservan siempre su nombre completo: un punto en el nombre
    /// de una carpeta no es una extensión.
    #[must_use]
    pub fn label(&self, entry: &FileEntry, executable: bool) -> String {
        let full = entry.display.clone();
        if self.show_extensions
            || entry.kind == crate::entry::EntryKind::Directory
            || (executable && self.always_show_for_executables)
        {
            return full;
        }
        match base_and_extension(&full) {
            Some((base, _)) => base.to_string(),
            None => full,
        }
    }
}

/// Parte `nombre.ext` en `("nombre", "ext")`.
///
/// Devuelve `None` cuando no hay extensión que separar: sin punto, con punto
/// final (`file.`) o en un dotfile sin más puntos (`.bashrc`, cuyo punto marca
/// «oculto» y no una extensión).
#[must_use]
pub fn base_and_extension(name: &str) -> Option<(&str, &str)> {
    let dot = name.rfind('.')?;
    if dot == 0 || dot + 1 == name.len() {
        return None;
    }
    Some((&name[..dot], &name[dot + 1..]))
}

/// Filtro en vivo por nombre: reduce lo ya listado, sin recursión ni índice.
///
/// Coincidencia por subcadena y sin distinción de mayúsculas, como pide la spec.
/// No pliega acentos: eso es cosa de la búsqueda, que sí recorre subcarpetas.
#[derive(Debug, Clone, Default)]
pub struct NameFilter {
    folded: String,
}

impl NameFilter {
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            folded: fold_for_match(text),
        }
    }

    /// Un filtro vacío no esconde nada.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.folded.is_empty()
    }

    #[must_use]
    pub fn matches(&self, entry: &FileEntry) -> bool {
        self.folded.is_empty() || fold_for_match(&entry.display).contains(&self.folded)
    }

    /// Cuenta cuántas entradas pasan el filtro, para el «N de M elementos» de la
    /// barra de estado: sin ese conteo el usuario cree que la carpeta está vacía.
    #[must_use]
    pub fn count_matching(&self, entries: &[FileEntry]) -> usize {
        entries.iter().filter(|e| self.matches(e)).count()
    }
}
