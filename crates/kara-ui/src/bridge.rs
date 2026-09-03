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
use kara_core::tabs::{CloseOutcome, OpenMode, TabId, Tabs};
use kara_core::columns::{ColumnLayout, ColumnMemory};
use kara_core::selection::Selection;
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

use crate::bridge::qobject::{
    clipboard_clear, clipboard_gnome, clipboard_kde_cut, clipboard_uri_list, clipboard_write,
};
use crate::prefs::Prefs;
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
        /// Las columnas de la vista de detalles, en su orden.
        #[qproperty(QStringList, column_ids)]
        #[qproperty(QStringList, column_labels)]
        #[qproperty(QList_i32, column_widths)]
        #[qproperty(i32, column_count)]
        /// Las que se pueden añadir, para el menú de la cabecera.
        #[qproperty(QStringList, addable_ids)]
        #[qproperty(QStringList, addable_labels)]
        /// El contenido de la tabla, por filas: la celda `(fila, columna)` está
        /// en `fila * column_count + columna`. Una lista de listas no se puede
        /// exponer a QML, y una propiedad por columna obligaría a conocerlas de
        /// antemano, que es justo lo que «configurables» impide.
        #[qproperty(QStringList, entry_values)]
        /// Si lo que se enseña es la papelera y no una carpeta. Cambia lo que
        /// significan las acciones: ahí no se borra, se restaura o se elimina
        /// para siempre.
        #[qproperty(bool, in_trash)]
        /// Las pestañas abiertas, sus rótulos y cuál está activa.
        #[qproperty(QStringList, tab_titles)]
        #[qproperty(i32, tab_count)]
        #[qproperty(i32, active_tab)]
        /// Si hay algo que reabrir, para no ofrecer un gesto que no hace nada.
        #[qproperty(bool, can_reopen_tab)]
        /// Modo concentración: una sola pestaña a la vista y la barra
        /// escondida, con la ventana tal y como era antes de haber pestañas.
        /// Las demás **no se cierran**: siguen ahí, solo dejan de verse.
        #[qproperty(bool, focus_mode)]
        #[qproperty(QString, sort_column)]
        #[qproperty(bool, sort_ascending)]
        /// Qué entradas están seleccionadas, en 1 y 0.
        #[qproperty(QList_i32, entry_selected)]
        /// Cuántas hay seleccionadas y cuál tiene el cursor, o -1.
        #[qproperty(i32, selected_count)]
        #[qproperty(i32, focused_index)]
        /// Suma de los tamaños seleccionados, ya legible. Vacía si no hay
        /// selección o si ninguna entrada aporta tamaño.
        #[qproperty(QString, selected_size)]
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
        /// Ancho del panel de navegación, recordado entre sesiones.
        #[qproperty(i32, sidebar_width)]
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

        /// Copia la selección al portapapeles del escritorio.
        #[qinvokable]
        fn copy_selection(self: Pin<&mut App>);

        /// Corta la selección: no mueve nada hasta que se pega.
        #[qinvokable]
        fn cut_selection(self: Pin<&mut App>);

        /// Pega en la carpeta actual lo que haya en el portapapeles, venga de
        /// Kara o de cualquier otro gestor del escritorio.
        #[qinvokable]
        fn paste(self: Pin<&mut App>);

        /// Abre una carpeta en una pestaña nueva. En segundo plano no mueve el
        /// foco, que es lo que hace el clic central.
        #[qinvokable]
        fn open_tab(self: Pin<&mut App>, path: &QString, background: bool);

        /// Abre en una pestaña nueva la carpeta que hay bajo el cursor.
        #[qinvokable]
        fn open_focused_in_tab(self: Pin<&mut App>, background: bool);

        #[qinvokable]
        fn close_tab(self: Pin<&mut App>, index: i32);

        #[qinvokable]
        fn activate_tab(self: Pin<&mut App>, index: i32);

        #[qinvokable]
        fn duplicate_tab(self: Pin<&mut App>, index: i32);

        /// Reabre la última pestaña cerrada, con su historial.
        #[qinvokable]
        fn reopen_tab(self: Pin<&mut App>);

        #[qinvokable]
        fn next_tab(self: Pin<&mut App>);

        #[qinvokable]
        fn previous_tab(self: Pin<&mut App>);

        /// Reordena la barra arrastrando. El foco sigue a la pestaña movida.
        #[qinvokable]
        fn drag_tab(self: Pin<&mut App>, from: i32, to: i32);

        /// Entra o sale del modo concentración.
        #[qinvokable]
        fn use_focus_mode(self: Pin<&mut App>, on: bool);

        /// Enseña la papelera del escritorio.
        #[qinvokable]
        fn show_trash(self: Pin<&mut App>);

        /// Devuelve a su sitio lo seleccionado en la papelera.
        #[qinvokable]
        fn restore_selected(self: Pin<&mut App>);

        /// Vacía la papelera entera. **Irreversible**: quien llame ha tenido
        /// que confirmar antes.
        #[qinvokable]
        fn empty_trash(self: Pin<&mut App>);

        /// Envía a la papelera **todo lo seleccionado**.
        ///
        /// Un fallo no aborta el lote: se intenta cada uno y se resume al
        /// final, que es la regla del proyecto para cualquier operación por
        /// lotes.
        #[qinvokable]
        fn trash_selected(self: Pin<&mut App>);

        /// El nombre de la entrada con el cursor, o vacío. Lo necesita el
        /// editor de renombrado, que trabaja sobre una sola.
        #[qinvokable]
        fn focused_name(self: Pin<&mut App>) -> QString;

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

        /// Recuerda el ancho al que el usuario dejó el panel.
        ///
        /// No se llama `set_sidebar_width`: ese nombre ya lo genera la
        /// propiedad, y este además guarda en disco.
        #[qinvokable]
        fn remember_sidebar_width(self: Pin<&mut App>, width: i32);

        /// Ancla una carpeta al Acceso rápido, o la quita si ya lo estaba.
        #[qinvokable]
        fn toggle_pinned(self: Pin<&mut App>, path: &QString);

        /// Si una carpeta está anclada, para el rótulo del menú.
        #[qinvokable]
        fn is_pinned(self: Pin<&mut App>, path: &QString) -> bool;

        /// Enseña u oculta el panel de navegación (F9).
        #[qinvokable]
        fn toggle_sidebar(self: Pin<&mut App>);

        /// Un clic sobre una fila, con sus modificadores. La semántica de
        /// Ctrl y Mayúsculas vive en `kara_core::selection`, no aquí.
        #[qinvokable]
        fn click_entry(self: Pin<&mut App>, row: i32, ctrl: bool, shift: bool);

        #[qinvokable]
        fn select_all(self: Pin<&mut App>);

        #[qinvokable]
        fn deselect_all(self: Pin<&mut App>);

        #[qinvokable]
        fn invert_selection(self: Pin<&mut App>);

        /// Empieza un marco elástico, recordando la selección de partida.
        #[qinvokable]
        fn begin_band(self: Pin<&mut App>);

        /// El marco cubre las filas de `from` a `to`. Es lo que basta en una
        /// lista, donde lo que un rectángulo toca es siempre un tramo seguido.
        #[qinvokable]
        fn rubber_band(self: Pin<&mut App>, from: i32, to: i32, additive: bool);

        /// El marco cubre estas posiciones sueltas. En una rejilla un
        /// rectángulo toca el final de una fila y el principio de la siguiente,
        /// y lo de en medio queda fuera: un tramo diría lo que no es.
        #[qinvokable]
        fn band_set(self: Pin<&mut App>, covered: &QList_i32, additive: bool);

        /// Suelta el marco y fija lo seleccionado.
        #[qinvokable]
        fn end_band(self: Pin<&mut App>);

        /// Cancela el marco: la selección vuelve a como estaba al empezarlo.
        #[qinvokable]
        fn cancel_band(self: Pin<&mut App>);

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

        /// Cambia el ancho de una columna, arrastrando su separador.
        #[qinvokable]
        fn set_column_width(self: Pin<&mut App>, id: &QString, width: i32);

        /// Enseña u oculta una columna. La del nombre no se puede quitar.
        #[qinvokable]
        fn toggle_column(self: Pin<&mut App>, id: &QString);

        /// Mueve una columna a otra posición.
        #[qinvokable]
        fn move_column(self: Pin<&mut App>, from: i32, to: i32);

        /// Devuelve todas las columnas a su ancho normal.
        #[qinvokable]
        fn autofit_columns(self: Pin<&mut App>);

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
        // El portapapeles del escritorio. `cxx-qt-lib` no envuelve
        // `QClipboard` ni `QMimeData`, así que se llega por un trozo de C++
        // que no decide nada: mueve cadenas opacas y ya.
        include!("clipboard.h");
        #[namespace = "kara"]
        fn clipboard_write(uri_list: &str, gnome: &str, cut: bool);
        #[namespace = "kara"]
        fn clipboard_clear();
        #[namespace = "kara"]
        fn clipboard_uri_list() -> String;
        #[namespace = "kara"]
        fn clipboard_gnome() -> String;
        #[namespace = "kara"]
        fn clipboard_kde_cut() -> bool;

        type QString = cxx_qt_lib::QString;
        type QStringList = cxx_qt_lib::QStringList;
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }
}

/// Lo que cada pestaña tiene para ella sola.
///
/// El historial no está aquí: lo lleva `kara_core::tabs::Tab`, que ya lo posee.
/// Esto es lo demás que no puede compartirse: dos pestañas en la misma carpeta
/// pueden tener selecciones y filtros distintos, y una puede estar en la
/// papelera mientras la otra no.
#[derive(Default)]
struct TabView {
    selection: Selection,
    /// Las entradas que se están enseñando, en su orden. Se guardan para poder
    /// traducir la selección **por nombre** cuando la lista se rehace: al
    /// reordenar o filtrar los índices cambian, y una selección por índice
    /// señalaría a otros ficheros.
    visible: Vec<kara_core::FileEntry>,
    /// De qué carpeta son `visible` y la selección.
    visible_path: Option<PathBuf>,
    /// Si esta pestaña está enseñando la papelera.
    in_trash: bool,
    /// Las entradas de la papelera del último listado, en el orden en que
    /// llegaron. Las filas guardan su posición aquí en la bolsa de metadatos.
    trash: Vec<kara_fs::trash::TrashEntry>,
    /// El filtro por nombre, que es de la pestaña y no de la ventana.
    filter: String,
    /// La selección de antes de empezar el marco elástico.
    ///
    /// Hace falta por dos motivos: durante el arrastre el marco se aplica una y
    /// otra vez, y sin una base fija encogerlo no desharía nada; y la spec pide
    /// que Esc a mitad lo cancele **sin tocar la selección previa**.
    band_base: Option<Selection>,
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
    entry_selected: cxx_qt_lib::QList<i32>,
    entry_values: QStringList,
    in_trash: bool,
    tab_titles: QStringList,
    tab_count: i32,
    active_tab: i32,
    can_reopen_tab: bool,
    focus_mode: bool,
    column_ids: QStringList,
    column_labels: QStringList,
    column_widths: cxx_qt_lib::QList<i32>,
    column_count: i32,
    addable_ids: QStringList,
    addable_labels: QStringList,
    selected_count: i32,
    focused_index: i32,
    selected_size: QString,
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
    sidebar_width: i32,
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
    home: Option<PathBuf>,
    crumb_capacity: usize,
    tree: Tree,
    /// Resuelve el icono de cada entrada y recuerda lo ya buscado.
    icons: Icons,
    /// Descripciones de los tipos, para la columna «Tipo».
    descriptions: MimeDescriptions,
    /// Qué columnas enseña cada carpeta.
    columns: ColumnMemory,
    /// Lo que Kara recuerda entre sesiones.
    prefs: Prefs,
    /// Las pestañas abiertas. Cada una lleva su propio historial.
    tabs: Tabs,
    /// Lo que cada pestaña tiene para ella sola, por identificador: `Tabs` no
    /// admite carga útil, y la posición no sirve de clave porque reordenar
    /// mueve las pestañas.
    tab_views: HashMap<TabId, TabView>,
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

        let mut prefs = Prefs::load();
        let complaint = prefs.take_complaint();
        let pinned = prefs.pinned();
        let initial_view = prefs.default_view();

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
            entry_selected: cxx_qt_lib::QList::<i32>::default(),
            entry_values: QStringList::default(),
            in_trash: false,
            tab_titles: QStringList::default(),
            tab_count: 1,
            active_tab: 0,
            can_reopen_tab: false,
            focus_mode: prefs.focus_mode(),
            column_ids: QStringList::default(),
            column_labels: QStringList::default(),
            column_widths: cxx_qt_lib::QList::<i32>::default(),
            column_count: 0,
            addable_ids: QStringList::default(),
            addable_labels: QStringList::default(),
            selected_count: 0,
            focused_index: -1,
            selected_size: QString::default(),
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
            sidebar_visible: prefs.sidebar_visible(),
            sidebar_width: prefs.sidebar_width(),
            can_undo: false,
            can_redo: false,
            undo_label: QString::default(),
            redo_label: QString::default(),
            last_error: QString::default(),
            // Del fichero de ajustes, no de la constante: el modo que el
            // usuario dejó puesto tiene que estar aplicado ya en el primer
            // fotograma. `render` lo restaura al navegar, pero al arrancar
            // nadie ha navegado todavía.
            view_mode: mode_ordinal(initial_view.mode),
            icon_size: clamp_count(initial_view.icon_size as usize),
            can_zoom_out: !initial_view.is_smallest(),
            can_zoom_in: !initial_view.is_largest(),
            tabs: Tabs::new(start.clone()),
            tab_views: HashMap::new(),
            home: home.clone(),
            crumb_capacity: DEFAULT_CRUMB_CAPACITY,
            tree: Tree::new(sections(home.as_deref(), &pinned)),
            // 16 píxeles es la talla de la vista de detalles. Los temas
            // modernos son SVG, así que la talla solo decide de qué carpeta del
            // tema sale el fichero, no la nitidez.
            icons: Icons::load(ICON_SIZE),
            descriptions: MimeDescriptions::new(TYPE_LANGUAGES.iter().map(|l| (*l).to_string()).collect()),
            undo: UndoStack::new(),
            sort_defaults: prefs.sort_defaults(),
            views: ViewMemory::new(prefs.default_view(), kara_core::view::MEMORY_CAPACITY),
            thumbnail_jobs: Vec::new(),
            listing: Arc::new(AtomicU64::new(0)),
            thumbs: Vec::new(),
            place_kinds: place_kinds(home.as_deref()),
            columns: ColumnMemory::default(),
            prefs,
        };

        // Un fichero de ajustes ilegible no impide arrancar, pero tampoco se
        // calla: se enseña una vez y la sesión sigue con los valores por
        // defecto, sin sobrescribir lo que hubiera.
        if let Some(complaint) = complaint {
            app.last_error = QString::from(&complaint);
        }

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
            app.entry_values = snapshot.values;
            app.column_ids = snapshot.column_ids;
            app.column_labels = snapshot.column_labels;
            app.column_widths = snapshot.column_widths;
            app.column_count = snapshot.column_count;
            app.addable_ids = snapshot.addable_ids;
            app.addable_labels = snapshot.addable_labels;
            app.entry_count = snapshot.count;
            app.total_count = snapshot.total;
            app.crumb_names = snapshot.crumb_names;
            app.crumb_paths = snapshot.crumb_paths;
            app.overflow_names = snapshot.overflow_names;
            app.overflow_paths = snapshot.overflow_paths;
        }

        app.tab_titles = vec![QString::from(&present::tab_title(&start, false))]
            .into_iter()
            .collect();

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
    values: QStringList,
    column_ids: QStringList,
    column_labels: QStringList,
    column_widths: cxx_qt_lib::QList<i32>,
    column_count: i32,
    addable_ids: QStringList,
    addable_labels: QStringList,
    selected: cxx_qt_lib::QList<i32>,
    selected_count: i32,
    focused_index: i32,
    selected_size: QString,
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
    /// Lo que la pestaña activa tiene para ella sola.
    fn view(&self) -> &TabView {
        // `Tabs` garantiza que siempre hay una activa; el `unwrap_or_default`
        // no es una excusa sino el caso de una pestaña recién abierta a la que
        // todavía nadie ha volcado nada.
        static EMPTY: std::sync::OnceLock<TabView> = std::sync::OnceLock::new();
        self.tab_views
            .get(&self.tabs.active_id())
            .unwrap_or_else(|| EMPTY.get_or_init(TabView::default))
    }

    fn view_mut(&mut self) -> &mut TabView {
        self.tab_views.entry(self.tabs.active_id()).or_default()
    }

    /// La carpeta que enseña la pestaña activa.
    fn active_path(&self) -> PathBuf {
        self.tabs.active().path().to_path_buf()
    }

    /// Lista, oculta, filtra, ordena y parte la ruta en migas.
    ///
    /// `None` si la carpeta no se puede leer, y entonces quien llame **no
    /// cambia nada**: enseñar una vista vacía haría creer que la carpeta lo está.
    fn snapshot(&mut self, target: &Path) -> Option<Snapshot> {
        // La papelera se lee de otro sitio, pero produce las mismas filas: así
        // ordenar, filtrar, seleccionar y pintar siguen siendo el mismo código.
        let mut entries = if self.view().in_trash {
            let listing = kara_fs::trash::list_trash();
            let rows = listing
                .entries
                .iter()
                .enumerate()
                .map(|(index, entry)| present::trash_row(entry, index))
                .collect();
            self.view_mut().trash = listing.entries;
            rows
        } else {
            self.view_mut().trash.clear();
            list_directory(target).ok()?.entries
        };

        Visibility::default().retain_visible(&mut entries);
        let total = entries.len();

        let filter = NameFilter::new(&self.view().filter.clone());
        if !filter.is_empty() {
            entries.retain(|entry| filter.matches(entry));
        }

        // El criterio sale de lo que esta carpeta decidió, resuelto contra el
        // global: una carpeta que solo eligió el sentido sigue heredando el
        // resto.
        let sort = self.views.sort_for(target).resolve(&self.sort_defaults);
        kara_core::sort::sort_entries(&mut entries, &sort);

        // La selección se traduce **por nombre**, no por índice: reordenar,
        // filtrar o refrescar cambia los índices, y una selección por índice
        // acabaría señalando a otros ficheros. Cambiar de carpeta la vacía: los
        // nombres pueden coincidir y arrastrarla sería seleccionar a ciegas.
        let carried = if self.view().visible_path.as_deref() == Some(target) {
            let view = self.view();
            let previous = view.selection.to_view_state(&view.visible, 0.0);
            Selection::from_view_state(&previous, &entries)
        } else {
            Selection::new()
        };
        {
            let view = self.view_mut();
            view.selection = carried;
            view.visible_path = Some(target.to_path_buf());
            view.visible = entries.clone();
        }

        // La tabla se construye por filas para que QML pueda indexarla con
        // `fila * column_count + columna`.
        let layout = self.columns.layout_for(target);
        let columns: Vec<kara_core::sort::ColumnId> =
            layout.columns().iter().map(|c| c.id.clone()).collect();
        let mut values: Vec<QString> = Vec::with_capacity(entries.len() * columns.len());
        for entry in &entries {
            for id in &columns {
                // «Tipo» es el único que no sale de una función pura: su
                // descripción la resuelve la base de MIME, que tiene memoria.
                let text = if id.0.as_ref() == "kind" {
                    self.type_label(entry)
                } else {
                    present::cell_value(entry, id)
                };
                values.push(QString::from(&text));
            }
        }

        let selected = ints(
            (0..entries.len()).map(|index| i32::from(self.view().selection.is_selected(index))),
        );
        // Tamaño total de lo seleccionado, que es lo que la spec pide en la
        // barra de estado junto al conteo. Las carpetas no aportan: su tamaño
        // es recursivo y no se conoce.
        let bytes: u64 = self.view()
            .selection
            .selected()
            .iter()
            .filter_map(|index| entries.get(*index))
            .filter_map(|entry| entry.size)
            .sum();

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

        // La papelera no es una ruta y no tiene ancestros: una sola miga que
        // dice dónde está el usuario, sin fingir una jerarquía.
        let segments = if self.in_trash {
            Vec::new()
        } else {
            breadcrumb::segments(target, self.home.as_deref())
        };
        let split = breadcrumb::collapse(&segments, self.crumb_capacity);

        Some(Snapshot {
            path: QString::from(&if self.in_trash {
                "Papelera".to_string()
            } else {
                target.to_string_lossy().into_owned()
            }),
            names,
            sizes,
            kinds,
            icons,
            values: values.into_iter().collect(),
            column_ids: columns
                .iter()
                .map(|id| QString::from(&id.0.to_string()))
                .collect(),
            column_labels: columns
                .iter()
                .map(|id| QString::from(&present::column_label(id)))
                .collect(),
            column_widths: ints(layout.columns().iter().map(|c| clamp_count(c.width as usize))),
            column_count: clamp_count(columns.len()),
            addable_ids: layout
                .available_to_add()
                .iter()
                .map(|id| QString::from(&id.0.to_string()))
                .collect(),
            addable_labels: layout
                .available_to_add()
                .iter()
                .map(|id| QString::from(&present::column_label(id)))
                .collect(),
            selected,
            selected_count: clamp_count(self.view().selection.len()),
            focused_index: self.view()
                .selection
                .focused()
                .map_or(-1, clamp_count),
            selected_size: QString::from(&if bytes > 0 {
                present::format_size(bytes)
            } else {
                String::new()
            }),
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
fn sections(home: Option<&Path>, pinned: &[PathBuf]) -> Vec<Section> {
    let branch = |place: &kara_fs::places::Place| Branch {
        name: std::ffi::OsString::from(present::place_label(place)),
        path: place.path.clone(),
    };

    // Lo que el usuario ancló va primero, como en el Explorador: es lo que
    // eligió él, y las carpetas XDG son las que vienen de serie.
    let mut quick: Vec<Branch> = pinned.iter().map(Branch::at).collect();
    quick.extend(
        kara_fs::places::quick_access(home)
            .iter()
            .map(branch)
            // Anclar una carpeta que ya salía no la duplica.
            .filter(|place| !pinned.contains(&place.path)),
    );

    vec![
        Section {
            id: SectionId::QuickAccess,
            roots: quick,
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
        self.as_mut().set_entry_values(view.values);
        self.as_mut().set_column_ids(view.column_ids);
        self.as_mut().set_column_labels(view.column_labels);
        self.as_mut().set_column_widths(view.column_widths);
        self.as_mut().set_column_count(view.column_count);
        self.as_mut().set_addable_ids(view.addable_ids);
        self.as_mut().set_addable_labels(view.addable_labels);
        self.as_mut().set_entry_selected(view.selected);
        self.as_mut().set_selected_count(view.selected_count);
        self.as_mut().set_focused_index(view.focused_index);
        self.as_mut().set_selected_size(view.selected_size);
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
        self.as_mut().publish_nav();

        // El rótulo de la pestaña activa es el nombre de su carpeta, así que
        // navegar lo cambia.
        self.as_mut().publish_tabs();
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
        self.as_mut().rust_mut().get_mut().prefs.set_sidebar_visible(!visible);
        self.as_mut().persist();
    }

    fn remember_sidebar_width(mut self: Pin<&mut Self>, width: i32) {
        if *self.sidebar_width() == width {
            return;
        }
        self.as_mut().set_sidebar_width(width);
        self.as_mut().rust_mut().get_mut().prefs.set_sidebar_width(width);
        self.as_mut().persist();
    }

    fn is_pinned(self: Pin<&mut Self>, path: &QString) -> bool {
        self.rust().prefs.is_pinned(Path::new(&path.to_string()))
    }

    fn toggle_pinned(mut self: Pin<&mut Self>, path: &QString) {
        let path = PathBuf::from(path.to_string());
        if !path.is_absolute() {
            return;
        }

        {
            let state = self.as_mut().rust_mut().get_mut();
            if state.prefs.is_pinned(&path) {
                state.prefs.unpin(&path);
            } else {
                state.prefs.pin(&path);
            }

            // Cambiar las raíces no cierra lo que el usuario tenía desplegado:
            // `set_sections` conserva la expansión y lo ya leído.
            let pinned = state.prefs.pinned();
            state.tree.set_sections(sections(state.home.as_deref(), &pinned));
        }

        self.as_mut().persist();
        self.as_mut().publish_nav();
    }

    /// Escribe los ajustes. Que no se puedan guardar se cuenta y no detiene
    /// nada: perder una preferencia no puede costar la sesión.
    fn persist(mut self: Pin<&mut Self>) {
        if let Err(error) = self.rust().prefs.save() {
            self.as_mut()
                .report(&format!("No se pudieron guardar los ajustes: {error}"));
        }
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

    /// Vuelca la selección a las propiedades sin releer la carpeta.
    ///
    /// Un clic no puede costar un `scandir`: la lista visible ya está en
    /// memoria y lo único que cambia son las marcas.
    fn publish_selection(mut self: Pin<&mut Self>) {
        let (marks, count, focused, bytes) = {
            let state = self.rust();
            let marks = ints(
                (0..state.view().visible.len())
                    .map(|index| i32::from(state.view().selection.is_selected(index))),
            );
            let bytes: u64 = state
                .view()
                .selection
                .selected()
                .iter()
                .filter_map(|index| state.view().visible.get(*index))
                .filter_map(|entry| entry.size)
                .sum();
            (
                marks,
                clamp_count(state.view().selection.len()),
                state.view().selection.focused().map_or(-1, clamp_count),
                bytes,
            )
        };

        self.as_mut().set_entry_selected(marks);
        self.as_mut().set_selected_count(count);
        self.as_mut().set_focused_index(focused);
        self.as_mut().set_selected_size(QString::from(&if bytes > 0 {
            present::format_size(bytes)
        } else {
            String::new()
        }));
    }

    fn click_entry(mut self: Pin<&mut Self>, row: i32, ctrl: bool, shift: bool) {
        let Ok(index) = usize::try_from(row) else {
            return;
        };
        {
            let state = self.as_mut().rust_mut().get_mut();
            let len = state.view().visible.len();
            let selection = &mut state.view_mut().selection;
            // Mayúsculas manda sobre Ctrl, como en el Explorador: Ctrl+May+clic
            // añade el rango a lo que ya había.
            match (shift, ctrl) {
                (true, true) => selection.add_range(index, len),
                (true, false) => selection.select_range(index, len),
                (false, true) => selection.ctrl_click(index, len),
                (false, false) => selection.click(index, len),
            }
        }
        self.as_mut().publish_selection();
    }

    fn select_all(mut self: Pin<&mut Self>) {
        let len = self.rust().view().visible.len();
        self.as_mut()
            .rust_mut()
            .get_mut()
            .view_mut()
            .selection
            .select_all(len);
        self.as_mut().publish_selection();
    }

    fn deselect_all(mut self: Pin<&mut Self>) {
        self.as_mut()
            .rust_mut()
            .get_mut()
            .view_mut()
            .selection
            .deselect_all();
        self.as_mut().publish_selection();
    }

    fn invert_selection(mut self: Pin<&mut Self>) {
        let len = self.rust().view().visible.len();
        self.as_mut()
            .rust_mut()
            .get_mut()
            .view_mut()
            .selection
            .invert(len);
        self.as_mut().publish_selection();
    }

    fn begin_band(mut self: Pin<&mut Self>) {
        let base = self.rust().view().selection.clone();
        self.as_mut().rust_mut().get_mut().view_mut().band_base = Some(base);
    }

    /// La selección desde la que aplicar el marco: la de antes de empezarlo si
    /// suma, y ninguna si reemplaza.
    fn band_start(&self, additive: bool) -> Selection {
        let mut base = self.rust().view().band_base.clone().unwrap_or_default();
        if !additive {
            // Vacía lo marcado pero **conserva el cursor**, que es la misma
            // regla que sigue Esc: el marco decide qué está seleccionado, no
            // dónde estaba el usuario. Partir de una selección nueva lo perdía.
            base.deselect_all();
        }
        base
    }

    fn rubber_band(mut self: Pin<&mut Self>, from: i32, to: i32, additive: bool) {
        let (Ok(from), Ok(to)) = (usize::try_from(from), usize::try_from(to)) else {
            return;
        };
        let mut selection = self.band_start(additive);
        {
            let state = self.as_mut().rust_mut().get_mut();
            let len = state.view().visible.len();
            // Siempre sumando: lo que decide si el marco reemplaza o añade es
            // desde qué selección se parte, no cómo se aplica.
            selection.apply_rubber_band(from, to, len, true);
            state.view_mut().selection = selection;
        }
        self.as_mut().publish_selection();
    }

    fn band_set(mut self: Pin<&mut Self>, covered: &cxx_qt_lib::QList<i32>, additive: bool) {
        let positions: Vec<usize> = covered
            .iter()
            .filter_map(|position| usize::try_from(*position).ok())
            .collect();

        let mut selection = self.band_start(additive);
        {
            let state = self.as_mut().rust_mut().get_mut();
            let len = state.view().visible.len();
            selection.apply_band(&positions, len, true);
            state.view_mut().selection = selection;
        }
        self.as_mut().publish_selection();
    }

    fn end_band(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().get_mut().view_mut().band_base = None;
    }

    fn cancel_band(mut self: Pin<&mut Self>) {
        let base = self.as_mut().rust_mut().get_mut().view_mut().band_base.take();
        if let Some(base) = base {
            self.as_mut().rust_mut().get_mut().view_mut().selection = base;
            self.as_mut().publish_selection();
        }
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

    /// Las rutas seleccionadas, en el orden en que se ven.
    fn selected_paths(&self) -> Vec<PathBuf> {
        let current = PathBuf::from(self.path().to_string());
        let state = self.rust();
        state
            .view()
            .selection
            .selected()
            .iter()
            .filter_map(|index| state.view().visible.get(*index))
            .map(|entry| current.join(&entry.name))
            .collect()
    }

    fn put_on_clipboard(mut self: Pin<&mut Self>, action: kara_fs::clipboard::ClipboardAction) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }

        let state = match action {
            kara_fs::clipboard::ClipboardAction::Cut => {
                kara_fs::clipboard::ClipboardState::cut(paths)
            }
            kara_fs::clipboard::ClipboardAction::Copy => {
                kara_fs::clipboard::ClipboardState::copy(paths)
            }
        };
        let payload = state.to_formats();
        clipboard_write(
            &payload.uri_list,
            &payload.gnome_copied_files,
            state.is_cut(),
        );
        self.as_mut().clear_error();
    }

    fn copy_selection(mut self: Pin<&mut Self>) {
        self.as_mut()
            .put_on_clipboard(kara_fs::clipboard::ClipboardAction::Copy);
    }

    fn cut_selection(mut self: Pin<&mut Self>) {
        self.as_mut()
            .put_on_clipboard(kara_fs::clipboard::ClipboardAction::Cut);
    }

    fn paste(mut self: Pin<&mut Self>) {
        let uri_list = clipboard_uri_list();
        let gnome = clipboard_gnome();
        let cut_marker = clipboard_kde_cut();
        let kde = if cut_marker { "1" } else { "0" };

        let formats = kara_fs::clipboard::ClipboardFormats {
            uri_list: (!uri_list.is_empty()).then_some(uri_list.as_bytes()),
            gnome_copied_files: (!gnome.is_empty()).then_some(gnome.as_bytes()),
            kde_cut_selection: Some(kde.as_bytes()),
        };
        let Some(state) = kara_fs::clipboard::parse(formats) else {
            return;
        };

        let destination = PathBuf::from(self.path().to_string());
        let mut failures = Vec::new();
        let mut done: Vec<Action> = Vec::new();

        for source in &state.paths {
            // `paste_target` decide si esto se pega: devuelve `None` para un
            // corte en su propia carpeta, que no es un error sino un gesto sin
            // efecto. El nombre que calcula no se usa —`copy_to` y `move_to`
            // resuelven el suyo con la política— pero se le da un `exists` de
            // verdad para que la decisión sea la que el dominio tomaría.
            let target = state.paste_target(source, &destination, |name| {
                destination.join(name).exists()
            });
            if target.is_none() {
                continue;
            }

            // `KeepBoth`: sin diálogo de conflictos todavía, conservar los dos
            // es la única opción que no puede destruir nada. Cuando el diálogo
            // exista, aquí se preguntará.
            let outcome = if state.is_cut() {
                kara_fs::move_to(source, &destination, ConflictPolicy::KeepBoth).map(|moved| {
                    Action::Moved {
                        from: moved.source,
                        to: moved.destination,
                    }
                })
            } else {
                kara_fs::copy_to(source, &destination, ConflictPolicy::KeepBoth).map(|copied| {
                    Action::Copied {
                        created: copied.destination,
                    }
                })
            };

            match outcome {
                Ok(action) => done.push(action),
                Err(error) => failures.push(format!("{}: {error}", source.display())),
            }
        }

        for action in done {
            self.as_mut().rust_mut().get_mut().undo.push(action);
        }

        if failures.is_empty() {
            self.as_mut().clear_error();
        } else {
            let resumen = format!(
                "No se pudieron pegar {} de {}: {}",
                failures.len(),
                state.paths.len(),
                failures.join("; ")
            );
            self.as_mut().report(&resumen);
        }

        // Un corte es de un solo uso: una vez movido no queda nada en el
        // origen que volver a mover.
        if state.after_paste().is_none() {
            clipboard_clear();
        }

        self.as_mut().rust_mut().get_mut().tree.forget(&destination);
        self.as_mut().publish_undo();
        self.as_mut().render(&destination);
    }

    /// Las entradas de papelera que el usuario tiene señaladas.
    fn selected_trash(&self) -> Vec<kara_fs::trash::TrashEntry> {
        let state = self.rust();
        state
            .view()
            .selection
            .selected()
            .iter()
            .filter_map(|row| state.view().visible.get(*row))
            .filter_map(present::trash_index_of)
            .filter_map(|index| state.view().trash.get(index).cloned())
            .collect()
    }

    /// Vuelca la barra de pestañas: rótulos, cuál está activa y si hay algo
    /// que reabrir.
    fn publish_tabs(mut self: Pin<&mut Self>) {
        let (titles, count, active, can_reopen) = {
            let state = self.rust();
            let titles: Vec<QString> = state
                .tabs
                .tabs()
                .map(|tab| {
                    let in_trash = state
                        .tab_views
                        .get(&tab.id())
                        .is_some_and(|view| view.in_trash);
                    QString::from(&present::tab_title(tab.path(), in_trash))
                })
                .collect();
            let active = state
                .tabs
                .tabs()
                .position(|tab| tab.id() == state.tabs.active_id())
                .unwrap_or(0);
            (
                titles,
                clamp_count(state.tabs.len()),
                clamp_count(active),
                state.tabs.reopenable_count() > 0,
            )
        };

        self.as_mut().set_tab_titles(titles.into_iter().collect());
        self.as_mut().set_tab_count(count);
        self.as_mut().set_active_tab(active);
        self.as_mut().set_can_reopen_tab(can_reopen);
    }

    /// Enseña la pestaña activa: su carpeta, su filtro y su papelera.
    ///
    /// Cambiar de pestaña no es navegar: no toca el historial de ninguna, y
    /// cada una recupera el filtro y la selección con los que se dejó.
    fn show_active_tab(mut self: Pin<&mut Self>) {
        let (path, filter, in_trash) = {
            let state = self.rust();
            (
                state.active_path(),
                state.view().filter.clone(),
                state.view().in_trash,
            )
        };

        self.as_mut().set_filter_text(QString::from(&filter));
        self.as_mut().set_in_trash(in_trash);
        self.as_mut().render(&path);
        self.as_mut().publish_history();
        self.as_mut().publish_tabs();
    }

    /// El identificador de la pestaña que ocupa una posición de la barra.
    fn tab_at(&self, index: i32) -> Option<TabId> {
        let index = usize::try_from(index).ok()?;
        self.rust().tabs.tabs().nth(index).map(kara_core::Tab::id)
    }

    fn open_tab(mut self: Pin<&mut Self>, path: &QString, background: bool) {
        let path = PathBuf::from(path.to_string());
        if !path.is_dir() {
            return;
        }
        let mode = if background {
            OpenMode::Background
        } else {
            OpenMode::Foreground
        };
        self.as_mut().rust_mut().get_mut().tabs.open(path, mode);

        if background {
            // Abrir detrás no mueve el foco, así que no hay que reenseñar
            // nada: solo aparece una pestaña más en la barra.
            self.as_mut().publish_tabs();
        } else {
            self.as_mut().show_active_tab();
        }
    }

    fn open_focused_in_tab(mut self: Pin<&mut Self>, background: bool) {
        let target = {
            let state = self.rust();
            state
                .view()
                .selection
                .focused()
                .and_then(|index| state.view().visible.get(index))
                .filter(|entry| entry.kind == EntryKind::Directory)
                .map(|entry| state.active_path().join(&entry.name))
        };
        let Some(target) = target else {
            return;
        };
        let path = QString::from(&target.to_string_lossy().into_owned());
        self.as_mut().open_tab(&path, background);
    }

    fn close_tab(mut self: Pin<&mut Self>, index: i32) {
        let Some(id) = self.tab_at(index) else {
            return;
        };
        let outcome = self.as_mut().rust_mut().get_mut().tabs.close(id);
        match outcome {
            CloseOutcome::Closed { .. } => {
                // Lo que la pestaña tenía para ella sola se va con ella; si
                // vuelve por «reabrir», vuelve con su historial y una vista
                // limpia, que es lo honesto: la carpeta pudo cambiar.
                self.as_mut().rust_mut().get_mut().tab_views.remove(&id);
                self.as_mut().show_active_tab();
            }
            // Cerrar la última no vacía la barra: este módulo no cierra
            // ventanas, y una barra sin pestañas no es un estado que exista.
            CloseOutcome::LastTab | CloseOutcome::NotFound => {}
        }
    }

    fn activate_tab(mut self: Pin<&mut Self>, index: i32) {
        let Some(id) = self.tab_at(index) else {
            return;
        };
        if self.rust().tabs.active_id() == id {
            return;
        }
        self.as_mut().rust_mut().get_mut().tabs.activate(id);
        self.as_mut().show_active_tab();
    }

    fn duplicate_tab(mut self: Pin<&mut Self>, index: i32) {
        let Some(id) = self.tab_at(index) else {
            return;
        };
        self.as_mut()
            .rust_mut()
            .get_mut()
            .tabs
            .duplicate(id, OpenMode::Foreground);
        self.as_mut().show_active_tab();
    }

    fn reopen_tab(mut self: Pin<&mut Self>) {
        if self.as_mut().rust_mut().get_mut().tabs.reopen().is_none() {
            return;
        }
        self.as_mut().show_active_tab();
    }

    fn next_tab(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().get_mut().tabs.activate_next();
        self.as_mut().show_active_tab();
    }

    fn previous_tab(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().get_mut().tabs.activate_previous();
        self.as_mut().show_active_tab();
    }

    fn drag_tab(mut self: Pin<&mut Self>, from: i32, to: i32) {
        let (Ok(from), Ok(to)) = (usize::try_from(from), usize::try_from(to)) else {
            return;
        };
        self.as_mut().rust_mut().get_mut().tabs.move_tab(from, to);
        self.as_mut().publish_tabs();
    }

    fn use_focus_mode(mut self: Pin<&mut Self>, on: bool) {
        if *self.focus_mode() == on {
            return;
        }
        self.as_mut().set_focus_mode(on);
        self.as_mut().rust_mut().get_mut().prefs.set_focus_mode(on);
        self.as_mut().persist();
    }

    fn show_trash(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().get_mut().view_mut().in_trash = true;
        self.as_mut().set_in_trash(true);
        // La ruta deja de nombrar una carpeta: la barra de direcciones enseña
        // «Papelera» y el `in_trash` es lo que la vista mira, no el texto.
        let placeholder = PathBuf::from("/");
        self.as_mut().render(&placeholder);
    }

    /// Vuelve a una carpeta de verdad.
    fn leave_trash(mut self: Pin<&mut Self>) {
        if self.rust().view().in_trash {
            self.as_mut().rust_mut().get_mut().view_mut().in_trash = false;
            self.as_mut().set_in_trash(false);
        }
    }

    fn restore_selected(mut self: Pin<&mut Self>) {
        let entries = self.selected_trash();
        if entries.is_empty() {
            return;
        }

        let mut failures = Vec::new();
        let mut restored = 0_usize;
        for entry in &entries {
            let kara_fs::trash::TrashEntry::Item(item) = entry else {
                // Un registro cuyo fichero ya no está no se puede devolver a
                // ningún sitio; se puede borrar, que es otra acción.
                failures.push(format!(
                    "{}: ya no queda nada que restaurar",
                    entry.display_path().display()
                ));
                continue;
            };

            match kara_fs::trash::restore_item(item, ConflictPolicy::KeepBoth) {
                Ok(_) => restored += 1,
                Err(error) => failures.push(format!("{}: {error}", item.original_path.display())),
            }
        }

        if failures.is_empty() {
            self.as_mut().clear_error();
        } else {
            let resumen = format!(
                "No se pudieron restaurar {} de {}: {}",
                failures.len(),
                entries.len(),
                failures.join("; ")
            );
            self.as_mut().report(&resumen);
        }

        let _ = restored;
        self.as_mut().reload();
    }

    fn empty_trash(mut self: Pin<&mut Self>) {
        let outcome = kara_fs::trash::empty_trash(&mut kara_fs::trash::NullObserver);

        if outcome.failed.is_empty() {
            self.as_mut().clear_error();
        } else {
            let resumen = format!(
                "No se pudieron eliminar {} elementos de la papelera",
                outcome.failed.len()
            );
            self.as_mut().report(&resumen);
        }

        // Vaciar es irreversible por definición: nada entra en la pila de
        // deshacer, y quien llame ha tenido que confirmarlo antes.
        self.as_mut().reload();
    }

    fn focused_name(self: Pin<&mut Self>) -> QString {
        let state = self.rust();
        let name = state
            .view()
            .selection
            .focused()
            .and_then(|index| state.view().visible.get(index))
            .map(|entry| entry.display.clone())
            .unwrap_or_default();
        QString::from(&name)
    }

    fn trash_selected(mut self: Pin<&mut Self>) {
        let current = PathBuf::from(self.path().to_string());
        let victims: Vec<PathBuf> = {
            let state = self.rust();
            state
                .view()
                .selection
                .selected()
                .iter()
                .filter_map(|index| state.view().visible.get(*index))
                .map(|entry| current.join(&entry.name))
                .collect()
        };
        if victims.is_empty() {
            return;
        }

        let policy = kara_ops::trash_policy();
        let mut failures = Vec::new();
        let mut done = Vec::new();
        for victim in &victims {
            match kara_fs::trash::trash_one(victim, &policy) {
                Ok(item) => done.push(item),
                Err(error) => failures.push(format!("{}: {error}", victim.display())),
            }
        }

        // Cada elemento entra por separado en la pila: `UndoStack` no agrupa
        // todavía, así que deshacer un lote de tres son tres Ctrl+Z. Es honesto
        // y reversible; agruparlo es trabajo de la cola de operaciones.
        for item in done {
            self.as_mut().rust_mut().get_mut().undo.push(Action::Trashed {
                item: Box::new(item),
            });
        }

        if failures.is_empty() {
            self.as_mut().clear_error();
        } else {
            let resumen = format!(
                "No se pudieron enviar a la papelera {} de {}: {}",
                failures.len(),
                victims.len(),
                failures.join("; ")
            );
            self.as_mut().report(&resumen);
        }

        self.as_mut().rust_mut().get_mut().tree.forget(&current);
        self.as_mut().publish_undo();
        self.as_mut().render(&current);
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

    /// Cambia el reparto de columnas de esta carpeta y vuelve a pintarla.
    ///
    /// Se relista: los valores de las celdas se calculan al listar, y una
    /// columna nueva no tiene de dónde salir si no. Es el mismo precio que ya
    /// paga ordenar por una cabecera.
    fn change_columns(mut self: Pin<&mut Self>, change: impl FnOnce(&mut ColumnLayout)) {
        let folder = PathBuf::from(self.path().to_string());
        {
            let state = self.as_mut().rust_mut().get_mut();
            let mut layout = state.columns.layout_for(&folder);
            change(&mut layout);
            state.columns.remember(&folder, layout);
        }
        self.as_mut().render(&folder);
    }

    fn set_column_width(mut self: Pin<&mut Self>, id: &QString, width: i32) {
        let id = kara_core::sort::ColumnId(id.to_string().into());
        let width = u32::try_from(width).unwrap_or(0);
        self.as_mut()
            .change_columns(|layout| layout.set_width(&id, width));
    }

    fn toggle_column(mut self: Pin<&mut Self>, id: &QString) {
        let id = kara_core::sort::ColumnId(id.to_string().into());
        self.as_mut().change_columns(|layout| {
            if layout.is_visible(&id) {
                // Quitar la del nombre se rechaza en el dominio; aquí basta con
                // no insistir.
                let _ = layout.remove(&id);
            } else {
                layout.add(id);
            }
        });
    }

    fn move_column(mut self: Pin<&mut Self>, from: i32, to: i32) {
        let (Ok(from), Ok(to)) = (usize::try_from(from), usize::try_from(to)) else {
            return;
        };
        self.as_mut()
            .change_columns(|layout| layout.reorder(from, to));
    }

    fn autofit_columns(mut self: Pin<&mut Self>) {
        self.as_mut()
            .change_columns(kara_core::columns::ColumnLayout::autofit_all);
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
        {
            let state = self.as_mut().rust_mut().get_mut();
            state.views.remember(&folder, settings);
            // Y pasa a ser el modo con el que se abren las carpetas que nadie
            // ha configurado: sin un «aplicar a todas» explícito, lo que el
            // usuario acaba de elegir es la mejor apuesta para la siguiente.
            state.views.set_fallback(settings);
            state.prefs.set_default_view(settings);
        }
        self.as_mut().persist();

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
            let history = state.tabs.active().history();
            (history.can_go_back(), history.can_go_forward())
        };
        self.as_mut().set_can_go_back(back);
        self.as_mut().set_can_go_forward(forward);
    }

    /// Navega dejando huella en el historial.
    fn navigate_to(mut self: Pin<&mut Self>, target: &Path) -> bool {
        self.as_mut().leave_trash();
        if !self.as_mut().render(target) {
            return false;
        }
        self.as_mut()
            .rust_mut()
            .get_mut()
            .tabs
            .active_mut()
            .history_mut()
            .visit(target);
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
            let history = self
                .as_mut()
                .rust_mut()
                .get_mut()
                .tabs
                .active_mut()
                .history_mut();
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
                .tabs
                .active_mut()
                .history_mut()
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
        self.as_mut().rust_mut().get_mut().view_mut().filter = text.to_string();
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
