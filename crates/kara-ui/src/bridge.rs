//! Puente cxx-qt: expone los modelos y acciones de Rust al motor QML.
//!
//! El QML **no** contiene lógica de negocio; todo lo que la UI necesita entra por
//! aquí desde `kara-ops` / `kara-index` / `kara-fs`.

use core::pin::Pin;
use std::path::{Path, PathBuf};
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

        /// Entra en una subcarpeta de la actual. Si no se puede listar, no se
        /// mueve: dejar la ruta apuntando a un sitio ilegible dejaria la vista
        /// vacia sin forma de volver.
        #[qinvokable]
        fn cd(self: Pin<&mut App>, name: &QString);

        /// Sube a la carpeta padre. En la raiz no hace nada.
        #[qinvokable]
        fn up(self: Pin<&mut App>);

        /// Envia una entrada de la carpeta actual a la papelera y refresca.
        #[qinvokable]
        fn trash(self: Pin<&mut App>, name: &QString);
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

impl qobject::App {
    /// Navega a `target` solo si se puede listar, y actualiza las propiedades
    /// por sus setters para que QML reciba las senales de cambio.
    fn navigate_to(mut self: Pin<&mut Self>, target: &Path) {
        let Some(view) = build_view(target) else {
            return;
        };
        self.as_mut()
            .set_path(cxx_qt_lib::QString::from(&target.to_string_lossy().into_owned()));
        self.as_mut().set_entry_names(view.names);
        self.as_mut().set_entry_sizes(view.sizes);
        self.as_mut().set_entry_kinds(view.kinds);
        self.as_mut().set_entry_count(view.count);
    }

    fn cd(self: Pin<&mut Self>, name: &cxx_qt_lib::QString) {
        let mut target = PathBuf::from(self.path().to_string());
        target.push(name.to_string());
        self.navigate_to(&target);
    }

    /// Envia a la papelera, nunca borra.
    ///
    /// La politica se pide a `kara_ops::trash_policy()` y no se construye aqui:
    /// `TrashPolicy::default()` deja el desfase horario a cero y estampa el
    /// `.trashinfo` en UTC, que no falla en ninguna parte y corre la fecha.
    fn trash(mut self: Pin<&mut Self>, name: &cxx_qt_lib::QString) {
        let current = PathBuf::from(self.path().to_string());
        let victim = current.join(name.to_string());
        if kara_fs::trash::trash_one(&victim, &kara_ops::trash_policy()).is_ok() {
            self.as_mut().navigate_to(&current);
        }
    }

    fn up(self: Pin<&mut Self>) {
        let current = PathBuf::from(self.path().to_string());
        if let Some(parent) = current.parent() {
            let parent = parent.to_path_buf();
            self.navigate_to(&parent);
        }
    }
}

/// Las columnas ya formateadas de una carpeta.
struct View {
    names: cxx_qt_lib::QStringList,
    sizes: cxx_qt_lib::QStringList,
    kinds: cxx_qt_lib::QStringList,
    count: i32,
}

/// Lista, filtra, ordena y formatea. `None` si la carpeta no se puede leer.
fn build_view(path: &Path) -> Option<View> {
    let listing = list_directory(path).ok()?;
    let mut entries = listing.entries;
    Visibility::default().retain_visible(&mut entries);
    kara_core::sort::sort_entries(&mut entries, &SortSpec::default());

    let names: Vec<cxx_qt_lib::QString> = entries
        .iter()
        .map(|e| cxx_qt_lib::QString::from(&e.display))
        .collect();
    let sizes: Vec<cxx_qt_lib::QString> = entries
        .iter()
        .map(|e| cxx_qt_lib::QString::from(&size_label(e)))
        .collect();
    let kinds: Vec<cxx_qt_lib::QString> = entries
        .iter()
        .map(|e| {
            cxx_qt_lib::QString::from(match e.kind {
                kara_core::entry::EntryKind::Directory => "Folder",
                kara_core::entry::EntryKind::File => "File",
            })
        })
        .collect();

    Some(View {
        count: i32::try_from(entries.len()).unwrap_or(i32::MAX),
        names: names.into_iter().collect(),
        sizes: sizes.into_iter().collect(),
        kinds: kinds.into_iter().collect(),
    })
}

/// El tamano de una carpeta se muestra vacio, no cero: calcularlo es recursivo.
fn size_label(entry: &kara_core::FileEntry) -> String {
    match entry.size {
        Some(bytes) => format_size(bytes),
        None => String::new(),
    }
}
