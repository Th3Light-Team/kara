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

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use kara_core::breadcrumb;
use kara_core::entry::EntryKind;
use kara_core::filter::{NameFilter, Visibility};
use kara_core::history::History;
use kara_core::sort::SortSpec;
use kara_core::tree::{Branch, Expandable, RowKind, Section, SectionId, Tree};
use kara_fs::icons::Icons;
use kara_fs::mime::MimeDescriptions;
use kara_fs::list_directory;
use kara_fs::places::PlaceKind;
use kara_fs::thumbnails::{ThumbnailSize, Thumbnails};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

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
        /// URL del icono de cada entrada, del tema del escritorio.
        #[qproperty(QStringList, entry_icons)]
        /// URL de la miniatura de cada entrada, vacía mientras no haya una. Se
        /// rellena desde un hilo de fondo, así que cambia después del listado.
        #[qproperty(QStringList, entry_thumbs)]
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
        /// Filas del panel de navegación, en listas paralelas. Una cabecera de
        /// sección se reconoce porque su ruta está vacía: no lleva a ningún
        /// sitio.
        #[qproperty(QStringList, nav_labels)]
        #[qproperty(QStringList, nav_paths)]
        #[qproperty(QStringList, nav_icons)]
        #[qproperty(QList_i32, nav_depths)]
        #[qproperty(QList_i32, nav_expandable)]
        #[qproperty(QList_i32, nav_expanded)]
        #[qproperty(i32, nav_count)]
        /// Fila de la carpeta que se está viendo, o -1 si no sale en el panel.
        #[qproperty(i32, nav_current)]
        #[qproperty(bool, sidebar_visible)]
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

        /// Despliega o pliega una rama del panel **sin navegar**: la spec exige
        /// que abrir una rama no mueva la vista principal.
        #[qinvokable]
        fn nav_toggle(self: Pin<&mut App>, row: i32);

        /// Navega a la carpeta de una fila del panel.
        #[qinvokable]
        fn nav_activate(self: Pin<&mut App>, row: i32);

        /// Enseña u oculta el panel de navegación (F9).
        #[qinvokable]
        fn toggle_sidebar(self: Pin<&mut App>);
    }

    // Las miniaturas se leen y se generan fuera del hilo de la interfaz, y
    // vuelven encolando un cierre sobre él. Sin esto, mirar una carpeta de
    // fotos congelaría la ventana hasta acabar de decodificarlas todas.
    impl cxx_qt::Threading for App {}

    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        include!("cxx-qt-lib/qstringlist.h");
        include!("cxx-qt-lib/qlist.h");
        type QString = cxx_qt_lib::QString;
        type QStringList = cxx_qt_lib::QStringList;
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }
}

pub struct AppRust {
    version: QString,
    path: QString,
    entry_names: QStringList,
    entry_sizes: QStringList,
    entry_kinds: QStringList,
    entry_icons: QStringList,
    entry_thumbs: QStringList,
    entry_count: i32,
    total_count: i32,
    crumb_names: QStringList,
    crumb_paths: QStringList,
    overflow_names: QStringList,
    overflow_paths: QStringList,
    can_go_back: bool,
    can_go_forward: bool,
    filter_text: QString,
    nav_labels: QStringList,
    nav_paths: QStringList,
    nav_icons: QStringList,
    nav_depths: cxx_qt_lib::QList<i32>,
    nav_expandable: cxx_qt_lib::QList<i32>,
    nav_expanded: cxx_qt_lib::QList<i32>,
    nav_count: i32,
    nav_current: i32,
    sidebar_visible: bool,

    // Estado que no se expone a QML.
    history: History,
    home: Option<PathBuf>,
    crumb_capacity: usize,
    tree: Tree,
    /// Resuelve el icono de cada entrada y recuerda lo ya buscado.
    icons: Icons,
    /// Descripciones de los tipos, para la columna «Tipo».
    descriptions: MimeDescriptions,
    /// La caché de miniaturas del escritorio. `None` si no hay dónde ponerla.
    thumbnails: Option<Thumbnails>,
    /// Sube en cada navegación. Un hilo cuyo número ya no es el actual tira su
    /// trabajo: el usuario se fue de esa carpeta y nadie va a mirar el
    /// resultado.
    listing: Arc<AtomicU64>,
    /// Copia en Rust de `entry_thumbs`, para poder parchear una posición sin
    /// reconstruirla desde la lista de Qt.
    thumbs: Vec<String>,
    /// Qué ubicación es cada raíz del panel, para darle su icono propio: la
    /// carpeta de descargas no se enseña con la carpeta genérica.
    place_kinds: HashMap<PathBuf, PlaceKind>,
}

impl Default for AppRust {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|h| h.is_absolute());
        // Sin `$HOME` utilizable se arranca en la raiz: es la unica carpeta que
        // seguro existe, y arrancar con la vista vacia no orienta a nadie.
        let start = starting_folder().or_else(|| home.clone()).unwrap_or_else(|| PathBuf::from("/"));

        let mut app = Self {
            version: QString::from(env!("CARGO_PKG_VERSION")),
            path: QString::from(&start.to_string_lossy().into_owned()),
            entry_names: QStringList::default(),
            entry_sizes: QStringList::default(),
            entry_kinds: QStringList::default(),
            entry_icons: QStringList::default(),
            entry_thumbs: QStringList::default(),
            entry_count: 0,
            total_count: 0,
            crumb_names: QStringList::default(),
            crumb_paths: QStringList::default(),
            overflow_names: QStringList::default(),
            overflow_paths: QStringList::default(),
            can_go_back: false,
            can_go_forward: false,
            filter_text: QString::default(),
            nav_labels: QStringList::default(),
            nav_paths: QStringList::default(),
            nav_icons: QStringList::default(),
            nav_depths: cxx_qt_lib::QList::<i32>::default(),
            nav_expandable: cxx_qt_lib::QList::<i32>::default(),
            nav_expanded: cxx_qt_lib::QList::<i32>::default(),
            nav_count: 0,
            nav_current: -1,
            sidebar_visible: true,
            history: History::new(start.clone()),
            home: home.clone(),
            crumb_capacity: DEFAULT_CRUMB_CAPACITY,
            tree: Tree::new(sections(home.as_deref())),
            // 16 píxeles es la talla de la vista de detalles. Los temas
            // modernos son SVG, así que la talla solo decide de qué carpeta del
            // tema sale el fichero, no la nitidez.
            icons: Icons::load(ICON_SIZE),
            descriptions: MimeDescriptions::new(TYPE_LANGUAGES.iter().map(|l| (*l).to_string()).collect()),
            thumbnails: Thumbnails::shared(THUMBNAIL_SIZE),
            listing: Arc::new(AtomicU64::new(0)),
            thumbs: Vec::new(),
            place_kinds: place_kinds(home.as_deref()),
        };

        // Al arrancar no hay ninguna señal que emitir todavía, así que el
        // volcado va directo a los campos.
        if let Some(snapshot) = app.snapshot(&start) {
            app.version = QString::from(env!("CARGO_PKG_VERSION"));
            app.path = snapshot.path;
            app.entry_names = snapshot.names;
            app.entry_sizes = snapshot.sizes;
            app.entry_kinds = snapshot.kinds;
            app.entry_icons = snapshot.icons;
            app.entry_count = snapshot.count;
            app.total_count = snapshot.total;
            app.crumb_names = snapshot.crumb_names;
            app.crumb_paths = snapshot.crumb_paths;
            app.overflow_names = snapshot.overflow_names;
            app.overflow_paths = snapshot.overflow_paths;
        }

        app.reveal(&start);
        let nav = app.nav_view(&start);
        app.nav_labels = nav.labels;
        app.nav_paths = nav.paths;
        app.nav_icons = nav.icons;
        app.nav_depths = nav.depths;
        app.nav_expandable = nav.expandable;
        app.nav_expanded = nav.expanded;
        app.nav_count = nav.count;
        app.nav_current = nav.current;
        app
    }
}

/// La carpeta que se pide por línea de órdenes: `kara ~/Imágenes`.
///
/// Se ignora lo que no sea una carpeta legible en vez de arrancar con la vista
/// vacía: quien se equivoca de ruta prefiere ver su carpeta personal a ver nada.
fn starting_folder() -> Option<PathBuf> {
    let requested = PathBuf::from(std::env::args_os().nth(1)?);
    let absolute = if requested.is_absolute() {
        requested
    } else {
        std::env::current_dir().ok()?.join(requested)
    };
    absolute.is_dir().then_some(absolute)
}

/// Migas visibles cuando la vista todavía no ha dicho cuántas caben.
const DEFAULT_CRUMB_CAPACITY: usize = 6;

/// Talla de icono que se pide al tema.
const ICON_SIZE: u32 = 16;

/// Talla de miniatura que se pide a la caché compartida.
///
/// 128 píxeles es la talla «normal» del estándar, la que más probabilidades
/// tiene de estar ya generada por otro programa, y sobra para la vista de
/// detalles. Cuando haya vista de iconos con zoom, la talla saldrá del zoom.
const THUMBNAIL_SIZE: ThumbnailSize = ThumbnailSize::Normal;

/// Cuántas miniaturas se juntan antes de enseñarlas.
///
/// Una a una, cada una costaría reconstruir la lista entera y una vuelta al
/// bucle de eventos; de golpe al final, una carpeta grande no enseñaría nada
/// durante segundos.
const THUMBNAIL_BATCH: usize = 24;

/// Cada cuánto se enseña lo que haya, aunque el lote no esté lleno.
const THUMBNAIL_FLUSH: Duration = Duration::from_millis(120);

/// En qué idioma se piden las descripciones de tipo.
///
/// Se fija en español en vez de mirar el `locale` porque el resto de la ventana
/// está en español a pelo: con un `locale` inglés saldría «JSON document» junto
/// a «Carpeta de archivos», que se lee peor que traducirlo todo. Cuando la UI
/// tenga traducciones, esto pasa a ser la cadena de idiomas del entorno.
const TYPE_LANGUAGES: [&str; 1] = ["es"];

/// La vista entera de una carpeta, ya formateada.
struct Snapshot {
    path: QString,
    names: QStringList,
    sizes: QStringList,
    kinds: QStringList,
    icons: QStringList,
    count: i32,
    /// Qué entradas pueden tener miniatura, para el hilo de fondo.
    jobs: Vec<ThumbnailJob>,
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
    fn snapshot(&mut self, target: &Path) -> Option<Snapshot> {
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
            .map(|e| QString::from(&self.type_label(e)))
            .collect();
        let icons = entries
            .iter()
            .map(|e| {
                let path = self.icons.of(&e.display, e.kind);
                QString::from(&path.map(|p| present::file_url(&p)).unwrap_or_default())
            })
            .collect();

        // La caché se consulta para todo, no solo para las imágenes: un PDF o
        // un AppImage pueden tener miniatura hecha por otro programa. Generar,
        // en cambio, solo se intenta con imágenes.
        let jobs = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.kind != EntryKind::Directory)
            .filter_map(|(row, entry)| {
                let modified = entry.modified?;
                let may_generate = self
                    .icons
                    .mime_of(&entry.display)
                    .is_some_and(|mime| mime.starts_with("image/"));
                Some(ThumbnailJob {
                    row,
                    path: target.join(&entry.name),
                    modified,
                    may_generate,
                })
            })
            .collect();

        let segments = breadcrumb::segments(target, self.home.as_deref());
        let split = breadcrumb::collapse(&segments, self.crumb_capacity);

        Some(Snapshot {
            path: QString::from(&target.to_string_lossy().into_owned()),
            names,
            sizes,
            kinds,
            icons,
            jobs,
            count: clamp_count(entries.len()),
            total: clamp_count(total),
            crumb_names: labels(&split.visible),
            crumb_paths: paths(&split.visible),
            overflow_names: labels(&split.overflow),
            overflow_paths: paths(&split.overflow),
        })
    }
}

/// Una entrada a la que mirarle la miniatura.
struct ThumbnailJob {
    row: usize,
    path: PathBuf,
    modified: SystemTime,
    /// Si vale la pena intentar generarla cuando no esté en la caché. Solo las
    /// imágenes: un vídeo o un PDF necesitan herramientas externas, y probar con
    /// cada fichero suelto sería gastar por nada.
    may_generate: bool,
}

/// Las filas del panel de navegación, ya formateadas.
struct NavView {
    labels: QStringList,
    paths: QStringList,
    icons: QStringList,
    depths: cxx_qt_lib::QList<i32>,
    expandable: cxx_qt_lib::QList<i32>,
    expanded: cxx_qt_lib::QList<i32>,
    count: i32,
    current: i32,
}

/// Las dos secciones del panel, con las ubicaciones que hay ahora mismo.
fn sections(home: Option<&Path>) -> Vec<Section> {
    let branch = |place: &kara_fs::places::Place| Branch {
        name: std::ffi::OsString::from(present::place_label(place)),
        path: place.path.clone(),
    };

    vec![
        Section {
            id: SectionId::QuickAccess,
            roots: kara_fs::places::quick_access(home)
                .iter()
                .map(branch)
                .collect(),
        },
        Section {
            id: SectionId::ThisComputer,
            roots: kara_fs::places::this_computer().iter().map(branch).collect(),
        },
    ]
}

/// Qué ubicación es cada raíz, para darle su icono.
fn place_kinds(home: Option<&Path>) -> HashMap<PathBuf, PlaceKind> {
    kara_fs::places::quick_access(home)
        .into_iter()
        .chain(kara_fs::places::this_computer())
        .map(|place| (place.path, place.kind))
        .collect()
}

fn ints(values: impl IntoIterator<Item = i32>) -> cxx_qt_lib::QList<i32> {
    let mut list = cxx_qt_lib::QList::<i32>::default();
    for value in values {
        list.append(value);
    }
    list
}

impl AppRust {
    /// Aplana el árbol a listas paralelas y localiza la fila de `current`.
    fn nav_view(&mut self, current: &Path) -> NavView {
        let rows = self.tree.rows();

        let mut labels = Vec::with_capacity(rows.len());
        let mut paths = Vec::with_capacity(rows.len());
        let mut icons = Vec::with_capacity(rows.len());
        let mut depths = Vec::with_capacity(rows.len());
        let mut expandable = Vec::with_capacity(rows.len());
        let mut expanded = Vec::with_capacity(rows.len());
        let mut current_row = -1_i32;

        for (index, row) in rows.iter().enumerate() {
            depths.push(clamp_count(row.depth));
            match &row.kind {
                RowKind::Section(id) => {
                    labels.push(QString::from(present::section_label(*id)));
                    // Sin ruta: es lo que distingue una cabecera de una carpeta.
                    paths.push(QString::default());
                    icons.push(QString::default());
                    expandable.push(0);
                    expanded.push(0);
                }
                RowKind::Folder {
                    path,
                    name,
                    expandable: can_expand,
                    expanded: is_expanded,
                } => {
                    labels.push(QString::from(&name.to_string_lossy().into_owned()));
                    paths.push(QString::from(&path.to_string_lossy().into_owned()));

                    // Una raíz conocida usa su icono propio; lo que cuelga del
                    // árbol es siempre una carpeta.
                    let found = match self.place_kinds.get(path).copied() {
                        Some(kind) => self.icons.any_of(present::place_icons(kind)),
                        None => self
                            .icons
                            .of(&name.to_string_lossy(), kara_core::entry::EntryKind::Directory),
                    };
                    icons.push(QString::from(
                        &found.map(|p| present::file_url(&p)).unwrap_or_default(),
                    ));
                    // Mientras no se sepa, se ofrece la flecha: averiguarlo exige
                    // leer la carpeta, que es justo lo que se difiere.
                    expandable.push(i32::from(*can_expand != Expandable::No));
                    expanded.push(i32::from(*is_expanded));

                    // La primera fila que coincida gana: la carpeta personal sale
                    // antes que la misma ruta colgando de la raíz, y resaltar
                    // «Inicio» orienta mejor que resaltar `/home/ana`.
                    if current_row < 0 && path == current {
                        current_row = clamp_count(index);
                    }
                }
            }
        }

        NavView {
            count: clamp_count(rows.len()),
            current: current_row,
            labels: labels.into_iter().collect(),
            paths: paths.into_iter().collect(),
            icons: icons.into_iter().collect(),
            depths: ints(depths),
            expandable: ints(expandable),
            expanded: ints(expanded),
        }
    }

    /// Cómo se lee el tipo de una entrada en la columna «Tipo».
    ///
    /// La descripción sale de la base de FreeDesktop, que ya viene traducida:
    /// un `.json` se lee «documento JSON» sin que Kara traduzca nada. Lo que la
    /// base no describa cae a la extensión, como hace el Explorador.
    fn type_label(&mut self, entry: &kara_core::FileEntry) -> String {
        if let Some(intrinsic) = present::intrinsic_type_label(entry) {
            return intrinsic.to_string();
        }

        let described = self
            .icons
            .mime_of(&entry.display)
            .and_then(|mime| self.descriptions.of(mime))
            .map(present::capitalize_type);

        described.unwrap_or_else(|| present::fallback_type_label(&entry.display))
    }

    /// Lee las subcarpetas de una rama y se las entrega al árbol.
    ///
    /// Una carpeta que no se puede leer se registra como vacía: la flecha
    /// desaparece en vez de quedarse ofreciendo algo que nunca se va a abrir.
    fn load_children(&mut self, path: &Path) {
        let Ok(listing) = list_directory(path) else {
            self.tree.set_children(path, Vec::new());
            return;
        };

        let mut entries = listing.entries;
        // Solo carpetas: el panel navega, no lista contenido.
        entries.retain(|entry| entry.kind == EntryKind::Directory);
        Visibility::default().retain_visible(&mut entries);
        kara_core::sort::sort_entries(&mut entries, &SortSpec::default());

        let children = entries
            .iter()
            .map(|entry| Branch {
                path: path.join(&entry.name),
                name: entry.name.clone(),
            })
            .collect();
        self.tree.set_children(path, children);
    }

    /// Despliega lo necesario para que `target` se vea en el panel.
    fn reveal(&mut self, target: &Path) {
        for ancestor in self.tree.path_to_reveal(target) {
            if self.tree.expand(&ancestor) {
                self.load_children(&ancestor);
            }
        }
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

/// Vuelta de `i32` a `usize` para dimensionar la lista de miniaturas.
fn clamp_count_usize(count: i32) -> usize {
    usize::try_from(count).unwrap_or(0)
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
        let Some(view) = self.as_mut().rust_mut().get_mut().snapshot(target) else {
            return false;
        };
        self.as_mut().set_path(view.path);
        self.as_mut().set_entry_names(view.names);
        self.as_mut().set_entry_sizes(view.sizes);
        self.as_mut().set_entry_kinds(view.kinds);
        self.as_mut().set_entry_icons(view.icons);
        self.as_mut().start_thumbnails(view.jobs, clamp_count_usize(view.count));
        self.as_mut().set_entry_count(view.count);
        self.as_mut().set_total_count(view.total);
        self.as_mut().set_crumb_names(view.crumb_names);
        self.as_mut().set_crumb_paths(view.crumb_paths);
        self.as_mut().set_overflow_names(view.overflow_names);
        self.as_mut().set_overflow_paths(view.overflow_paths);

        // El panel sigue a la vista: despliega los ancestros de la carpeta que
        // se acaba de enseñar y la resalta.
        self.as_mut().rust_mut().get_mut().reveal(target);
        self.publish_nav();
        true
    }

    /// Vuelca las filas del panel de navegación.
    fn publish_nav(mut self: Pin<&mut Self>) {
        let current = PathBuf::from(self.path().to_string());
        let nav = self.as_mut().rust_mut().get_mut().nav_view(&current);
        self.as_mut().set_nav_labels(nav.labels);
        self.as_mut().set_nav_paths(nav.paths);
        self.as_mut().set_nav_icons(nav.icons);
        self.as_mut().set_nav_depths(nav.depths);
        self.as_mut().set_nav_expandable(nav.expandable);
        self.as_mut().set_nav_expanded(nav.expanded);
        self.as_mut().set_nav_count(nav.count);
        self.as_mut().set_nav_current(nav.current);
    }

    /// La ruta de una fila del panel, o `None` si es una cabecera de sección.
    fn nav_path_at(&self, row: i32) -> Option<PathBuf> {
        let paths = self.nav_paths();
        let index = isize::try_from(row).ok()?;
        let path = paths.get(index)?.to_string();
        if path.is_empty() {
            return None;
        }
        Some(PathBuf::from(path))
    }

    fn nav_toggle(mut self: Pin<&mut Self>, row: i32) {
        let Some(path) = self.nav_path_at(row) else {
            return;
        };
        let needs_children = self.as_mut().rust_mut().get_mut().tree.toggle(&path);
        if needs_children {
            self.as_mut().rust_mut().get_mut().load_children(&path);
        }
        self.publish_nav();
    }

    fn nav_activate(mut self: Pin<&mut Self>, row: i32) {
        let Some(path) = self.nav_path_at(row) else {
            return;
        };
        self.as_mut().navigate_to(&path);
    }

    fn toggle_sidebar(mut self: Pin<&mut Self>) {
        let visible = *self.sidebar_visible();
        self.as_mut().set_sidebar_visible(!visible);
    }

    /// Arranca la búsqueda de miniaturas de la carpeta recién enseñada.
    ///
    /// Se hace en dos pasadas y en un hilo aparte. La primera solo mira la caché
    /// compartida, que es barata, así que lo que otro programa ya generó aparece
    /// casi de inmediato. La segunda genera lo que falta, que es lento, y va
    /// goteando. Encadenarlas al revés dejaría la carpeta sin nada visible
    /// mientras se decodifica la primera foto.
    fn start_thumbnails(mut self: Pin<&mut Self>, jobs: Vec<ThumbnailJob>, rows: usize) {
        // Cualquier hilo anterior queda invalidado por este número: el usuario
        // ya no está en aquella carpeta.
        let generation = {
            let state = self.as_mut().rust_mut().get_mut();
            state.thumbs = vec![String::new(); rows];
            state.listing.fetch_add(1, Ordering::SeqCst) + 1
        };
        self.as_mut()
            .set_entry_thumbs(QStringList::default());

        let Some(thumbnails) = self.rust().thumbnails.clone() else {
            return;
        };
        if jobs.is_empty() {
            return;
        }

        let listing = Arc::clone(&self.rust().listing);
        let thread = self.qt_thread();

        std::thread::spawn(move || {
            let current = || listing.load(Ordering::SeqCst) == generation;
            let mut batch: Vec<(usize, String)> = Vec::new();
            let mut last_flush = Instant::now();

            let flush = |batch: &mut Vec<(usize, String)>| {
                if batch.is_empty() {
                    return;
                }
                let payload = std::mem::take(batch);
                // Si el objeto ya no está, no hay nada que hacer ni nada que
                // reportar: la ventana se cerró.
                let _ = thread.queue(move |app| {
                    app.apply_thumbnails(generation, payload);
                });
            };

            // Primera pasada: solo lo que ya está en la caché.
            let mut pending = Vec::new();
            for job in jobs {
                if !current() {
                    return;
                }
                match thumbnails.lookup(&job.path, job.modified) {
                    Some(found) => batch.push((job.row, kara_fs::file_uri(&found))),
                    None => pending.push(job),
                }
                if batch.len() >= THUMBNAIL_BATCH || last_flush.elapsed() >= THUMBNAIL_FLUSH {
                    flush(&mut batch);
                    last_flush = Instant::now();
                }
            }
            flush(&mut batch);

            // Segunda pasada: generar lo que falte.
            for job in pending {
                if !current() {
                    return;
                }
                if !job.may_generate || thumbnails.failed_before(&job.path, job.modified) {
                    continue;
                }
                if let Ok(written) = thumbnails.generate(&job.path, job.modified) {
                    batch.push((job.row, kara_fs::file_uri(&written)));
                }
                if batch.len() >= THUMBNAIL_BATCH || last_flush.elapsed() >= THUMBNAIL_FLUSH {
                    flush(&mut batch);
                    last_flush = Instant::now();
                }
            }
            flush(&mut batch);
        });
    }

    /// Vuelca un lote de miniaturas, si sigue siendo de la carpeta que se ve.
    fn apply_thumbnails(mut self: Pin<&mut Self>, generation: u64, found: Vec<(usize, String)>) {
        {
            let state = self.as_mut().rust_mut().get_mut();
            if state.listing.load(Ordering::SeqCst) != generation {
                return;
            }
            for (row, url) in found {
                if let Some(slot) = state.thumbs.get_mut(row) {
                    *slot = url;
                }
            }
        }

        let list: QStringList = self
            .rust()
            .thumbs
            .iter()
            .map(QString::from)
            .collect();
        self.as_mut().set_entry_thumbs(list);
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
        // Refrescar también refresca la rama: si no, una subcarpeta creada o
        // borrada fuera sigue saliendo en el panel hasta reiniciar. `forget` no
        // la pliega, así que lo que el usuario abrió sigue abierto.
        self.as_mut().rust_mut().get_mut().tree.forget(&current);
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
            self.as_mut().rust_mut().get_mut().tree.forget(&current);
            self.as_mut().render(&current);
        }
    }
}
