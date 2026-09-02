//! Puente cxx-qt: expone los modelos y acciones de Rust al motor QML.
//!
//! El QML **no** contiene lógica de negocio; todo lo que la UI necesita entra por
//! aquí desde `kara-ops` / `kara-index` / `kara-fs` / `kara-core`.
//!
//! # Por qué hay un `Snapshot`
//!
//! El estado de la vista se calcula **entero de una vez** y luego se vuelca a las
//! propiedades. La alternativa —ir tocando propiedades a medida que se resuelve
//! cada cosa— deja a QML leer estados intermedios: migas de una carpeta con el
//! listado de otra. Un solo punto de cálculo también evita que `Default` y las
//! acciones tengan cada una su copia de la misma lógica, que es como el listado y
//! las migas acaban discrepando.

use core::pin::Pin;
use std::path::{Path, PathBuf};

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QString, QStringList};
use kara_core::breadcrumb;
use kara_core::filter::{NameFilter, Visibility};
use kara_core::history::History;
use kara_core::sort::SortSpec;
use kara_fs::list_directory;

use crate::present;

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
        /// Entradas que se enseñan: ya filtradas.
        #[qproperty(i32, entry_count)]
        /// Entradas que hay en la carpeta antes de aplicar el filtro. La barra de
        /// estado necesita las dos para poder decir «N de M».
        #[qproperty(i32, total_count)]
        #[qproperty(QStringList, crumb_names)]
        #[qproperty(QStringList, crumb_paths)]
        /// Ancestros que no caben, para el menú del botón de desbordamiento.
        #[qproperty(QStringList, overflow_names)]
        #[qproperty(QStringList, overflow_paths)]
        #[qproperty(bool, can_go_back)]
        #[qproperty(bool, can_go_forward)]
        #[qproperty(QString, filter_text)]
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

        /// Vuelve a la ubicacion anterior del historial.
        #[qinvokable]
        fn back(self: Pin<&mut App>);

        /// Avanza a la siguiente ubicacion del historial.
        #[qinvokable]
        fn forward(self: Pin<&mut App>);

        /// Navega a una ruta absoluta ya conocida (una miga, el desbordamiento).
        #[qinvokable]
        fn navigate(self: Pin<&mut App>, path: &QString);

        /// Navega a lo tecleado en la barra de direcciones. Devuelve `false` si
        /// no lleva a ninguna carpeta legible, y entonces **no se mueve**: la
        /// spec exige conservar lo escrito para poder corregirlo.
        #[qinvokable]
        fn go_to(self: Pin<&mut App>, text: &QString) -> bool;

        /// Filtra el listado por subcadena. Solo reduce lo ya listado.
        #[qinvokable]
        fn apply_filter(self: Pin<&mut App>, text: &QString);

        /// Cuantas migas caben en la barra. Lo mide la vista, decide `kara-core`.
        #[qinvokable]
        fn set_crumb_capacity(self: Pin<&mut App>, visible: i32);

        /// Relee la carpeta actual sin tocar el historial.
        #[qinvokable]
        fn reload(self: Pin<&mut App>);
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        include!("cxx-qt-lib/qstringlist.h");
        type QString = cxx_qt_lib::QString;
        type QStringList = cxx_qt_lib::QStringList;
    }
}

pub struct AppRust {
    version: QString,
    path: QString,
    entry_names: QStringList,
    entry_sizes: QStringList,
    entry_kinds: QStringList,
    entry_count: i32,
    total_count: i32,
    crumb_names: QStringList,
    crumb_paths: QStringList,
    overflow_names: QStringList,
    overflow_paths: QStringList,
    can_go_back: bool,
    can_go_forward: bool,
    filter_text: QString,

    // Estado que no se expone a QML.
    history: History,
    home: Option<PathBuf>,
    crumb_capacity: usize,
}

impl Default for AppRust {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|h| h.is_absolute());
        // Sin `$HOME` utilizable se arranca en la raiz: es la unica carpeta que
        // seguro existe, y arrancar con la vista vacia no orienta a nadie.
        let start = home.clone().unwrap_or_else(|| PathBuf::from("/"));

        let mut app = Self {
            version: QString::from(env!("CARGO_PKG_VERSION")),
            path: QString::from(&start.to_string_lossy().into_owned()),
            entry_names: QStringList::default(),
            entry_sizes: QStringList::default(),
            entry_kinds: QStringList::default(),
            entry_count: 0,
            total_count: 0,
            crumb_names: QStringList::default(),
            crumb_paths: QStringList::default(),
            overflow_names: QStringList::default(),
            overflow_paths: QStringList::default(),
            can_go_back: false,
            can_go_forward: false,
            filter_text: QString::default(),
            history: History::new(start.clone()),
            home,
            crumb_capacity: DEFAULT_CRUMB_CAPACITY,
        };

        // Al arrancar no hay ninguna señal que emitir todavía, así que el
        // volcado va directo a los campos.
        if let Some(snapshot) = app.snapshot(&start) {
            app.version = QString::from(env!("CARGO_PKG_VERSION"));
            app.path = snapshot.path;
            app.entry_names = snapshot.names;
            app.entry_sizes = snapshot.sizes;
            app.entry_kinds = snapshot.kinds;
            app.entry_count = snapshot.count;
            app.total_count = snapshot.total;
            app.crumb_names = snapshot.crumb_names;
            app.crumb_paths = snapshot.crumb_paths;
            app.overflow_names = snapshot.overflow_names;
            app.overflow_paths = snapshot.overflow_paths;
        }
        app
    }
}

/// Migas visibles cuando la vista todavía no ha dicho cuántas caben.
const DEFAULT_CRUMB_CAPACITY: usize = 6;

/// La vista entera de una carpeta, ya formateada.
struct Snapshot {
    path: QString,
    names: QStringList,
    sizes: QStringList,
    kinds: QStringList,
    count: i32,
    total: i32,
    crumb_names: QStringList,
    crumb_paths: QStringList,
    overflow_names: QStringList,
    overflow_paths: QStringList,
}

impl AppRust {
    /// Lista, oculta, filtra, ordena y parte la ruta en migas.
    ///
    /// `None` si la carpeta no se puede leer, y entonces quien llame **no
    /// cambia nada**: enseñar una vista vacía haría creer que la carpeta lo está.
    fn snapshot(&self, target: &Path) -> Option<Snapshot> {
        let listing = list_directory(target).ok()?;
        let mut entries = listing.entries;

        Visibility::default().retain_visible(&mut entries);
        let total = entries.len();

        let filter = NameFilter::new(&self.filter_text.to_string());
        if !filter.is_empty() {
            entries.retain(|entry| filter.matches(entry));
        }

        kara_core::sort::sort_entries(&mut entries, &SortSpec::default());

        let names = entries.iter().map(|e| QString::from(&e.display)).collect();
        let sizes = entries
            .iter()
            .map(|e| QString::from(&present::size_label(e)))
            .collect();
        let kinds = entries
            .iter()
            .map(|e| QString::from(present::kind_label(e)))
            .collect();

        let segments = breadcrumb::segments(target, self.home.as_deref());
        let split = breadcrumb::collapse(&segments, self.crumb_capacity);

        Some(Snapshot {
            path: QString::from(&target.to_string_lossy().into_owned()),
            names,
            sizes,
            kinds,
            count: clamp_count(entries.len()),
            total: clamp_count(total),
            crumb_names: labels(&split.visible),
            crumb_paths: paths(&split.visible),
            overflow_names: labels(&split.overflow),
            overflow_paths: paths(&split.overflow),
        })
    }
}

fn labels(segments: &[breadcrumb::Segment]) -> QStringList {
    segments
        .iter()
        .map(|s| QString::from(&present::crumb_label(s)))
        .collect()
}

fn paths(segments: &[breadcrumb::Segment]) -> QStringList {
    segments
        .iter()
        .map(|s| QString::from(&s.path.to_string_lossy().into_owned()))
        .collect()
}

/// Qt cuenta con `int`. Una carpeta con más de 2^31 entradas no cabe, pero
/// tampoco puede tumbar el contador.
fn clamp_count(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

impl qobject::App {
    /// Vuelca una vista ya calculada usando los setters, para que QML reciba las
    /// señales de cambio. Devuelve `false` si la carpeta no se pudo leer.
    fn render(mut self: Pin<&mut Self>, target: &Path) -> bool {
        let Some(view) = self.rust().snapshot(target) else {
            return false;
        };
        self.as_mut().set_path(view.path);
        self.as_mut().set_entry_names(view.names);
        self.as_mut().set_entry_sizes(view.sizes);
        self.as_mut().set_entry_kinds(view.kinds);
        self.as_mut().set_entry_count(view.count);
        self.as_mut().set_total_count(view.total);
        self.as_mut().set_crumb_names(view.crumb_names);
        self.as_mut().set_crumb_paths(view.crumb_paths);
        self.as_mut().set_overflow_names(view.overflow_names);
        self.as_mut().set_overflow_paths(view.overflow_paths);
        true
    }

    /// Refleja en las propiedades si el historial puede ir atrás o adelante.
    fn publish_history(mut self: Pin<&mut Self>) {
        let (back, forward) = {
            let state = self.rust();
            (state.history.can_go_back(), state.history.can_go_forward())
        };
        self.as_mut().set_can_go_back(back);
        self.as_mut().set_can_go_forward(forward);
    }

    /// Navega dejando huella en el historial.
    fn navigate_to(mut self: Pin<&mut Self>, target: &Path) -> bool {
        if !self.as_mut().render(target) {
            return false;
        }
        self.as_mut().rust_mut().get_mut().history.visit(target);
        self.as_mut().publish_history();
        true
    }

    /// Salta a una entrada del historial.
    ///
    /// Si ya no se puede listar, se marca inválida y **no se mueve**: la carpeta
    /// pudo desaparecer mientras el usuario estaba en otra, y llevarle a una
    /// vista vacía sería peor que dejarle donde estaba.
    fn jump(mut self: Pin<&mut Self>, backwards: bool) {
        let target = {
            let history = &mut self.as_mut().rust_mut().get_mut().history;
            let entry = if backwards {
                history.back()
            } else {
                history.forward()
            };
            match entry {
                Some(entry) => entry.path.clone(),
                None => return,
            }
        };

        if !self.as_mut().render(&target) {
            self.as_mut()
                .rust_mut()
                .get_mut()
                .history
                .invalidate(&target);
        }
        self.publish_history();
    }

    fn cd(mut self: Pin<&mut Self>, name: &QString) {
        let mut target = PathBuf::from(self.path().to_string());
        target.push(name.to_string());
        self.as_mut().navigate_to(&target);
    }

    fn up(mut self: Pin<&mut Self>) {
        let current = PathBuf::from(self.path().to_string());
        if let Some(parent) = current.parent() {
            let parent = parent.to_path_buf();
            self.as_mut().navigate_to(&parent);
        }
    }

    fn back(self: Pin<&mut Self>) {
        self.jump(true);
    }

    fn forward(self: Pin<&mut Self>) {
        self.jump(false);
    }

    fn navigate(mut self: Pin<&mut Self>, path: &QString) {
        let target = PathBuf::from(path.to_string());
        self.as_mut().navigate_to(&target);
    }

    fn go_to(mut self: Pin<&mut Self>, text: &QString) -> bool {
        let current = PathBuf::from(self.path().to_string());
        let home = self.rust().home.clone();
        let Some(target) = present::expand_path(&text.to_string(), home.as_deref(), &current) else {
            return false;
        };
        self.as_mut().navigate_to(&target)
    }

    fn apply_filter(mut self: Pin<&mut Self>, text: &QString) {
        self.as_mut().set_filter_text(text.clone());
        let current = PathBuf::from(self.path().to_string());
        self.as_mut().render(&current);
    }

    fn set_crumb_capacity(mut self: Pin<&mut Self>, visible: i32) {
        let capacity = usize::try_from(visible).unwrap_or(DEFAULT_CRUMB_CAPACITY);
        if self.rust().crumb_capacity == capacity {
            return;
        }
        self.as_mut().rust_mut().get_mut().crumb_capacity = capacity;
        let current = PathBuf::from(self.path().to_string());
        self.as_mut().render(&current);
    }

    fn reload(mut self: Pin<&mut Self>) {
        let current = PathBuf::from(self.path().to_string());
        self.as_mut().render(&current);
    }

    /// Envia a la papelera, nunca borra.
    ///
    /// La politica se pide a `kara_ops::trash_policy()` y no se construye aqui:
    /// `TrashPolicy::default()` deja el desfase horario a cero y estampa el
    /// `.trashinfo` en UTC, que no falla en ninguna parte y corre la fecha.
    fn trash(mut self: Pin<&mut Self>, name: &QString) {
        let current = PathBuf::from(self.path().to_string());
        let victim = current.join(name.to_string());
        if kara_fs::trash::trash_one(&victim, &kara_ops::trash_policy()).is_ok() {
            self.as_mut().render(&current);
        }
    }
}
