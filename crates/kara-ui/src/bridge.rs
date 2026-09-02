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
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use kara_core::breadcrumb;
use kara_core::entry::EntryKind;
use kara_core::filter::{NameFilter, Visibility};
use kara_core::history::History;
use kara_core::sort::{ColumnId, SortOverrides, SortSpec, column_for_sort_key};
use kara_core::tree::{Branch, Expandable, RowKind, Section, SectionId, Tree};
use kara_core::view::{ViewMemory, ViewMode, ViewSettings};
use kara_fs::icons::Icons;
use kara_fs::mime::MimeDescriptions;
use kara_fs::list_directory;
use kara_fs::places::PlaceKind;
use kara_fs::trash::ConflictPolicy;
use kara_ops::{Action, UndoStack};
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
        /// Fecha de modificación de cada entrada, ya en hora local.
        #[qproperty(QStringList, entry_dates)]
        /// Columna por la que se está ordenando, con el identificador que
        /// entiende `kara_core::sort` («name», «kind», «size», «modified»), o
        /// vacía si no se ordena por ninguna. La cabecera la usa para saber
        /// dónde pintar la flecha.
        #[qproperty(QString, sort_column)]
        #[qproperty(bool, sort_ascending)]
        /// Qué entradas son carpetas, en 1 y 0.
        ///
        /// La vista necesita saberlo para entrar al hacer doble clic, y no
        /// puede deducirlo de la columna «Tipo»: ese texto es una descripción
        /// traducida del sistema, y compararla sería atar el comportamiento al
        /// idioma. Ya ocurrió: al pasar de «Carpeta» a «Carpeta de archivos»
        /// dejó de abrirse ninguna carpeta.
        #[qproperty(QList_i32, entry_dirs)]
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
        /// Si hay algo que deshacer o rehacer, y **qué**: la spec pide decir
        /// «Deshacer mover», no solo ofrecer deshacer.
        #[qproperty(bool, can_undo)]
        #[qproperty(bool, can_redo)]
        #[qproperty(QString, undo_label)]
        #[qproperty(QString, redo_label)]
        /// Lo último que salió mal, para enseñarlo. Vacía si no hay nada
        /// pendiente de contar: ninguna operación puede fallar en silencio.
        #[qproperty(QString, last_error)]
        /// Modo de vista actual, como el ordinal de `kara_core::view::ViewMode`:
        /// 0 detalles, 1 lista, 2 mosaico, 3 iconos.
        #[qproperty(i32, view_mode)]
        /// Lado del icono o la miniatura, en píxeles lógicos.
        #[qproperty(i32, icon_size)]
        /// Si queda sitio para seguir alejando o acercando.
        #[qproperty(bool, can_zoom_out)]
        #[qproperty(bool, can_zoom_in)]
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

        /// Deshace la última operación reversible.
        #[qinvokable]
        fn undo(self: Pin<&mut App>);

        /// Rehace la última que se deshizo.
        #[qinvokable]
        fn redo(self: Pin<&mut App>);

        /// Crea una carpeta en la actual. Devuelve el nombre con el que quedó,
        /// que puede no ser el pedido si ya existía uno igual.
        #[qinvokable]
        fn create_folder(self: Pin<&mut App>, name: &QString) -> QString;

        /// Cuántos caracteres del nombre son el nombre base, sin la extensión.
        ///
        /// Lo necesita el editor de renombrado: la spec pide que al abrirlo
        /// quede seleccionado solo el nombre base, para no borrar la extensión
        /// sin querer. Se resuelve aquí y no en QML porque `kara_core` ya sabe
        /// que `.tar.gz` es una sola extensión y QML no.
        #[qinvokable]
        fn base_name_length(self: Pin<&mut App>, name: &QString) -> i32;

        /// Renombra una entrada de la carpeta actual.
        #[qinvokable]
        fn rename_entry(self: Pin<&mut App>, from: &QString, to: &QString);

        /// Descarta el aviso de error que se esté enseñando.
        #[qinvokable]
        fn clear_error(self: Pin<&mut App>);

        /// Ordena por una columna de la vista de detalles. Un segundo clic en
        /// la misma invierte el sentido.
        #[qinvokable]
        fn sort_by(self: Pin<&mut App>, column: &QString);

        /// Cambia de modo de vista. `icon_size` menor o igual que cero pide el
        /// tamaño con el que ese modo se enseña normalmente.
        #[qinvokable]
        fn set_view(self: Pin<&mut App>, mode: i32, icon_size: i32);

        /// Un paso de la escala de zoom. Al quedarse sin tamaños de icono se
        /// pasa a los modos más densos, que es lo que hace la rueda en Windows.
        #[qinvokable]
        fn zoom_in(self: Pin<&mut App>);

        #[qinvokable]
        fn zoom_out(self: Pin<&mut App>);

        /// Vuelve al tamaño normal **del modo actual**, sin cambiar de modo.
        #[qinvokable]
        fn reset_zoom(self: Pin<&mut App>);
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
    entry_dirs: cxx_qt_lib::QList<i32>,
    entry_dates: QStringList,
    sort_column: QString,
    sort_ascending: bool,
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
    can_undo: bool,
    can_redo: bool,
    undo_label: QString,
    redo_label: QString,
    last_error: QString,
    view_mode: i32,
    icon_size: i32,
    can_zoom_out: bool,
    can_zoom_in: bool,

    // Estado que no se expone a QML.
    history: History,
    home: Option<PathBuf>,
    crumb_capacity: usize,
    tree: Tree,
    /// Resuelve el icono de cada entrada y recuerda lo ya buscado.
    icons: Icons,
    /// Descripciones de los tipos, para la columna «Tipo».
    descriptions: MimeDescriptions,
    /// Pila de deshacer/rehacer. Toda operación destructiva pasa por aquí:
    /// es la red de seguridad que la spec pone por encima de todo lo demás.
    undo: UndoStack,
    /// Criterio de ordenación de las carpetas que no han elegido otro.
    sort_defaults: SortSpec,
    /// Modo y zoom de cada carpeta. Se pierde al cerrar: la spec lo quiere en
    /// disco, pero todavía no hay dónde guardar ajustes.
    views: ViewMemory,
    /// Los encargos de miniatura de la carpeta que se enseña, para poder
    /// rehacerlos si el zoom cambia de talla sin volver a listar el disco.
    thumbnail_jobs: Vec<ThumbnailJob>,
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
            entry_dirs: cxx_qt_lib::QList::<i32>::default(),
            entry_dates: QStringList::default(),
            sort_column: QString::from("name"),
            sort_ascending: true,
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
            can_undo: false,
            can_redo: false,
            undo_label: QString::default(),
            redo_label: QString::default(),
            last_error: QString::default(),
            view_mode: mode_ordinal(ViewSettings::default().mode),
            icon_size: clamp_count(ViewSettings::default().icon_size as usize),
            can_zoom_out: false,
            can_zoom_in: true,
            history: History::new(start.clone()),
            home: home.clone(),
            crumb_capacity: DEFAULT_CRUMB_CAPACITY,
            tree: Tree::new(sections(home.as_deref())),
            // 16 píxeles es la talla de la vista de detalles. Los temas
            // modernos son SVG, así que la talla solo decide de qué carpeta del
            // tema sale el fichero, no la nitidez.
            icons: Icons::load(ICON_SIZE),
            descriptions: MimeDescriptions::new(TYPE_LANGUAGES.iter().map(|l| (*l).to_string()).collect()),
            undo: UndoStack::new(),
            sort_defaults: SortSpec::default(),
            views: ViewMemory::default(),
            thumbnail_jobs: Vec::new(),
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
            app.entry_dirs = snapshot.dirs;
            app.entry_dates = snapshot.dates;
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

/// El ordinal con el que un modo viaja a QML.
fn mode_ordinal(mode: ViewMode) -> i32 {
    match mode {
        ViewMode::Details => 0,
        ViewMode::List => 1,
        ViewMode::Tiles => 2,
        ViewMode::Icons => 3,
    }
}

/// Vuelta del ordinal al modo. Un valor que no existe deja el modo por defecto
/// en vez de fallar: viene de QML, no del dominio.
fn mode_from_ordinal(ordinal: i32) -> ViewMode {
    match ordinal {
        1 => ViewMode::List,
        2 => ViewMode::Tiles,
        3 => ViewMode::Icons,
        _ => ViewMode::Details,
    }
}

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
    dirs: cxx_qt_lib::QList<i32>,
    dates: QStringList,
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

        // El criterio sale de lo que esta carpeta decidió, resuelto contra el
        // global: una carpeta que solo eligió el sentido sigue heredando el
        // resto.
        let sort = self.views.sort_for(target).resolve(&self.sort_defaults);
        kara_core::sort::sort_entries(&mut entries, &sort);

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
            dates: entries
                .iter()
                .map(|e| QString::from(&present::modified_label(e.modified)))
                .collect(),
            dirs: ints(
                entries
                    .iter()
                    .map(|e| i32::from(e.kind == EntryKind::Directory)),
            ),
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
#[derive(Clone)]
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
        self.as_mut().set_entry_dirs(view.dirs);
        self.as_mut().set_entry_dates(view.dates);
        self.as_mut().publish_sort(target);

        // La carpeta manda sobre la vista: se restaura como se dejó antes de
        // pedir miniaturas, porque la talla que se pide sale de ahí.
        let remembered = self.rust().views.settings_for(target);
        self.as_mut().set_view_mode(mode_ordinal(remembered.mode));
        self.as_mut()
            .set_icon_size(clamp_count(remembered.icon_size as usize));
        self.as_mut().set_can_zoom_out(!remembered.is_smallest());
        self.as_mut().set_can_zoom_in(!remembered.is_largest());

        self.as_mut()
            .start_thumbnails(view.jobs, clamp_count_usize(view.count), true);
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
    fn start_thumbnails(
        mut self: Pin<&mut Self>,
        jobs: Vec<ThumbnailJob>,
        rows: usize,
        clear: bool,
    ) {
        // Cualquier hilo anterior queda invalidado por este número: el usuario
        // ya no está en aquella carpeta, o pidió otra talla.
        let generation = {
            let state = self.as_mut().rust_mut().get_mut();
            state.thumbnail_jobs = jobs.clone();
            if clear {
                state.thumbs = vec![String::new(); rows];
            }
            state.listing.fetch_add(1, Ordering::SeqCst) + 1
        };
        if clear {
            self.as_mut().set_entry_thumbs(QStringList::default());
        }

        // La talla sale del zoom: pedir 128 píxeles para un icono de 256 los
        // enseñaría emborronados, y pedir 256 para una fila de detalles sería
        // generar cuatro veces los píxeles que se ven.
        let wanted = u32::try_from(*self.icon_size()).unwrap_or(128);
        let Some(thumbnails) = Thumbnails::shared(ThumbnailSize::covering(wanted)) else {
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

    /// Refleja el estado de la pila de deshacer, con la etiqueta de qué se
    /// deshace: la spec pide decir «Deshacer mover», no solo ofrecer deshacer.
    fn publish_undo(mut self: Pin<&mut Self>) {
        let (can_undo, can_redo, undo_label, redo_label) = {
            let stack = &self.rust().undo;
            (
                stack.can_undo(),
                stack.can_redo(),
                stack.undo_label().unwrap_or_default().to_string(),
                stack.redo_label().unwrap_or_default().to_string(),
            )
        };
        self.as_mut().set_can_undo(can_undo);
        self.as_mut().set_can_redo(can_redo);
        self.as_mut().set_undo_label(QString::from(&undo_label));
        self.as_mut().set_redo_label(QString::from(&redo_label));
    }

    /// Deja constancia de un fallo para que la vista lo enseñe.
    ///
    /// Ninguna operación puede fallar en silencio: es una regla del proyecto, y
    /// una carpeta que no se crea sin decir por qué es indistinguible de un
    /// clic que no llegó.
    fn report(mut self: Pin<&mut Self>, message: &str) {
        self.as_mut().set_last_error(QString::from(&message.to_string()));
    }

    fn clear_error(mut self: Pin<&mut Self>) {
        self.as_mut().set_last_error(QString::default());
    }

    /// Apunta una operación ya hecha y refresca la vista.
    fn record(mut self: Pin<&mut Self>, action: Action) {
        self.as_mut().rust_mut().get_mut().undo.push(action);
        self.as_mut().publish_undo();
        let current = PathBuf::from(self.path().to_string());
        self.as_mut().render(&current);
    }

    fn undo(mut self: Pin<&mut Self>) {
        // Un deshacer que falla no pierde el registro: `UndoStack` devuelve la
        // acción a la pila, así que aquí basta con contarlo y no refrescar.
        if let Err(error) = self.as_mut().rust_mut().get_mut().undo.undo() {
            self.as_mut().report(&format!("No se pudo deshacer: {error}"));
            return;
        }
        self.as_mut().clear_error();
        self.as_mut().publish_undo();
        let current = PathBuf::from(self.path().to_string());
        self.as_mut().render(&current);
    }

    fn redo(mut self: Pin<&mut Self>) {
        if let Err(error) = self.as_mut().rust_mut().get_mut().undo.redo() {
            self.as_mut().report(&format!("No se pudo rehacer: {error}"));
            return;
        }
        self.as_mut().clear_error();
        self.as_mut().publish_undo();
        let current = PathBuf::from(self.path().to_string());
        self.as_mut().render(&current);
    }

    fn create_folder(mut self: Pin<&mut Self>, name: &QString) -> QString {
        let parent = PathBuf::from(self.path().to_string());
        let name = name.to_string();

        // `KeepBoth`: crear «Nueva carpeta» cuando ya hay una da «Nueva carpeta
        // (2)», como el Explorador. Fallar obligaría al usuario a inventar un
        // nombre antes de tener la carpeta delante.
        match kara_fs::create_directory(&parent, OsStr::new(&name), ConflictPolicy::KeepBoth) {
            Ok(path) => {
                let created = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.as_mut().clear_error();
                self.as_mut().record(Action::DirectoryCreated { path });
                QString::from(&created)
            }
            Err(error) => {
                self.as_mut()
                    .report(&format!("No se pudo crear la carpeta: {error}"));
                QString::default()
            }
        }
    }

    fn base_name_length(self: Pin<&mut Self>, name: &QString) -> i32 {
        let name = name.to_string();
        let (base, _) = kara_core::naming::split_name(&name);
        // QML cuenta en unidades UTF-16, que es como Qt indexa el texto.
        clamp_count(base.encode_utf16().count())
    }

    fn rename_entry(mut self: Pin<&mut Self>, from: &QString, to: &QString) {
        let parent = PathBuf::from(self.path().to_string());
        let source = parent.join(from.to_string());
        let target = to.to_string();

        if target.is_empty() || target == from.to_string() {
            return;
        }

        // `Fail`: renombrar encima de otro fichero lo perdería. Cuando haya
        // diálogo de conflictos, aquí se ofrecerán las opciones que pide la
        // spec; hasta entonces se avisa y no se toca nada.
        match kara_fs::rename(&source, OsStr::new(&target), ConflictPolicy::Fail) {
            Ok(destination) => {
                self.as_mut().clear_error();
                self.as_mut().record(Action::Renamed {
                    from: source,
                    to: destination,
                });
            }
            Err(error) => {
                self.as_mut()
                    .report(&format!("No se pudo renombrar: {error}"));
            }
        }
    }

    /// Refleja en las propiedades por qué columna se está ordenando.
    ///
    /// `column_for_sort_key` devuelve `None` para «sin ordenar», que no es
    /// ninguna columna: entonces no se resalta ninguna cabecera ni se pinta
    /// flecha, en vez de señalar una al azar.
    fn publish_sort(mut self: Pin<&mut Self>, folder: &Path) {
        let resolved = {
            let state = self.rust();
            state.views.sort_for(folder).resolve(&state.sort_defaults)
        };
        let column = column_for_sort_key(&resolved.key)
            .map(|id| id.0.into_owned())
            .unwrap_or_default();

        self.as_mut().set_sort_column(QString::from(&column));
        self.as_mut()
            .set_sort_ascending(resolved.order == kara_core::sort::SortOrder::Ascending);
    }

    /// Un clic en una cabecera de columna.
    ///
    /// La semántica —primera vez ascendente, otra vez invierte, cambiar de
    /// columna vuelve a ascendente— vive en `kara_core::sort`, no aquí: es una
    /// regla del dominio y hay pruebas que la fijan.
    fn sort_by(mut self: Pin<&mut Self>, column: &QString) {
        let folder = PathBuf::from(self.path().to_string());
        let id = ColumnId(column.to_string().into());

        let (current, defaults) = {
            let state = self.rust();
            (
                state.views.sort_for(&folder).resolve(&state.sort_defaults),
                state.sort_defaults.clone(),
            )
        };
        // Una columna que no ordena —una miniatura— se ignora sin más: el clic
        // no puede dejar la vista en un estado que nadie pidió.
        let Ok(next) = current.on_header_click(&id) else {
            return;
        };

        let overrides = SortOverrides::overriding(&next, &defaults);
        self.as_mut()
            .rust_mut()
            .get_mut()
            .views
            .remember_sort(&folder, overrides);
        self.as_mut().render(&folder);
    }

    /// Aplica unos ajustes de vista y los recuerda para esta carpeta.
    fn apply_view(mut self: Pin<&mut Self>, settings: ViewSettings) {
        let previous = ThumbnailSize::covering(u32::try_from(*self.icon_size()).unwrap_or(128));

        self.as_mut().set_view_mode(mode_ordinal(settings.mode));
        self.as_mut()
            .set_icon_size(clamp_count(settings.icon_size as usize));
        self.as_mut().set_can_zoom_out(!settings.is_smallest());
        self.as_mut().set_can_zoom_in(!settings.is_largest());

        let folder = PathBuf::from(self.path().to_string());
        self.as_mut()
            .rust_mut()
            .get_mut()
            .views
            .remember(&folder, settings);

        // Solo se vuelve a mirar la caché si el zoom cruza a otra talla del
        // estándar; dentro de la misma, las miniaturas que hay ya sirven.
        if ThumbnailSize::covering(settings.icon_size) != previous {
            let jobs = self.rust().thumbnail_jobs.clone();
            let rows = self.rust().thumbs.len();
            self.as_mut().start_thumbnails(jobs, rows, false);
        }
    }

    /// Ajustes actuales, tal y como los ven las propiedades.
    fn current_view(&self) -> ViewSettings {
        ViewSettings::new(
            mode_from_ordinal(*self.view_mode()),
            u32::try_from(*self.icon_size()).unwrap_or(20),
        )
    }

    fn set_view(mut self: Pin<&mut Self>, mode: i32, icon_size: i32) {
        let mode = mode_from_ordinal(mode);
        let settings = match u32::try_from(icon_size) {
            Ok(size) if size > 0 => ViewSettings::new(mode, size),
            _ => ViewSettings::for_mode(mode),
        };
        self.as_mut().apply_view(settings);
    }

    fn zoom_in(mut self: Pin<&mut Self>) {
        let next = self.current_view().zoom_in();
        self.as_mut().apply_view(next);
    }

    fn zoom_out(mut self: Pin<&mut Self>) {
        let next = self.current_view().zoom_out();
        self.as_mut().apply_view(next);
    }

    fn reset_zoom(mut self: Pin<&mut Self>) {
        let next = self.current_view().reset_zoom();
        self.as_mut().apply_view(next);
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
        match kara_fs::trash::trash_one(&victim, &kara_ops::trash_policy()) {
            Ok(item) => {
                self.as_mut().rust_mut().get_mut().tree.forget(&current);
                self.as_mut().clear_error();
                self.as_mut().record(Action::Trashed {
                    item: Box::new(item),
                });
            }
            Err(error) => {
                self.as_mut()
                    .report(&format!("No se pudo enviar a la papelera: {error}"));
            }
        }
    }
}
