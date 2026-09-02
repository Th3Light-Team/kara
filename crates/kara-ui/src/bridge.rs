//! Puente cxx-qt: expone los modelos y acciones de Rust al motor QML.
//!
//! El QML **no** contiene lógica de negocio; todo lo que la UI necesita entra por
//! aquí desde `kara-ops` / `kara-index` / `kara-fs`.

use std::path::PathBuf;
use kara_core::filter::Visibility;
use kara_core::sort::SortSpec;
use kara_fs::list_directory;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, version)]
        #[qproperty(QString, path)]
        #[qproperty(QStringList, entry_names)]
        #[qproperty(QStringList, entry_sizes)]
        #[qproperty(QStringList, entry_kinds)]
        #[qproperty(i32, entry_count)]
        type App = super::AppRust;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        include!("cxx-qt-lib/qstringlist.h");
        type QString = cxx_qt_lib::QString;
        type QStringList = cxx_qt_lib::QStringList;
    }
}

pub struct AppRust {
    version: cxx_qt_lib::QString,
    path: cxx_qt_lib::QString,
    entry_names: cxx_qt_lib::QStringList,
    entry_sizes: cxx_qt_lib::QStringList,
    entry_kinds: cxx_qt_lib::QStringList,
    entry_count: i32,
}

impl Default for AppRust {
    fn default() -> Self {
        let home = std::env::var("HOME")
            .ok()
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "/".to_string());

        let mut app = Self {
            version: cxx_qt_lib::QString::from(env!("CARGO_PKG_VERSION")),
            path: cxx_qt_lib::QString::from(&home),
            entry_names: cxx_qt_lib::QStringList::default(),
            entry_sizes: cxx_qt_lib::QStringList::default(),
            entry_kinds: cxx_qt_lib::QStringList::default(),
            entry_count: 0,
        };

        app.refresh_listing();
        app
    }
}


impl AppRust {
    /// Actualiza el listado basándose en `self.path`.
    fn refresh_listing(&mut self) {
        let path = PathBuf::from(self.path.to_string());

        // Listar el directorio.
        let listing = match list_directory(&path) {
            Ok(l) => l,
            Err(_) => {
                // Si falla el listado, vaciar y retornar.
                self.entry_names = cxx_qt_lib::QStringList::default();
                self.entry_sizes = cxx_qt_lib::QStringList::default();
                self.entry_kinds = cxx_qt_lib::QStringList::default();
                self.entry_count = 0;
                return;
            }
        };

        let mut entries = listing.entries;

        // Aplicar visibilidad (ocultos escondidos por defecto).
        let visibility = Visibility::default();
        visibility.retain_visible(&mut entries);

        // Ordenar con el criterio por defecto.
        let sort_spec = SortSpec::default();
        kara_core::sort::sort_entries(&mut entries, &sort_spec);

        // Convertir a listas de strings para QML usando iteradores.
        let names: Vec<cxx_qt_lib::QString> = entries
            .iter()
            .map(|e| cxx_qt_lib::QString::from(&e.display))
            .collect();

        let sizes: Vec<cxx_qt_lib::QString> = entries
            .iter()
            .map(|e| {
                let size_str = match e.size {
                    Some(bytes) => format_size(bytes),
                    None => {
                        if e.kind == kara_core::entry::EntryKind::Directory {
                            "—".to_string()
                        } else {
                            "?".to_string()
                        }
                    }
                };
                cxx_qt_lib::QString::from(&size_str)
            })
            .collect();

        let kinds: Vec<cxx_qt_lib::QString> = entries
            .iter()
            .map(|e| {
                let kind_str = match e.kind {
                    kara_core::entry::EntryKind::Directory => "Folder",
                    kara_core::entry::EntryKind::File => "File",
                };
                cxx_qt_lib::QString::from(kind_str)
            })
            .collect();

        self.entry_names = names.into_iter().collect();
        self.entry_sizes = sizes.into_iter().collect();
        self.entry_kinds = kinds.into_iter().collect();
        self.entry_count = entries.len() as i32;
    }
}

/// Formatea un tamaño en bytes a una cadena legible (B, KB, MB, GB).
fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}
