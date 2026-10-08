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
use kara_core::filter::{NameDisplay, NameFilter, Visibility};
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
    clipboard_clear, clipboard_gnome, clipboard_kde_cut, clipboard_set_text, clipboard_uri_list,
    clipboard_write, present_window,
};
use kara_desktop::{
    Associations, ColorScheme, DesktopIntegration, FileManagerRequest, FileManagerService, Volume,
    VolumeAction, VolumeError,
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
        /// Como se rotula cada entrada en la rejilla: igual que `entry_names`,
        /// salvo que sin extension si el usuario las oculta.
        #[qproperty(QStringList, entry_labels)]
        /// 1 si la entrada esta oculta, para dibujarla atenuada.
        #[qproperty(QList_i32, entry_hidden)]
        /// El diálogo de Propiedades: abierto, su título y sus filas
        /// (etiqueta y valor, en dos listas paralelas).
        /// La pregunta de borrado permanente: abierta y qué dice.
        #[qproperty(bool, delete_prompt)]
        #[qproperty(QString, delete_text)]
        #[qproperty(bool, prop_open)]
        #[qproperty(QString, prop_title)]
        #[qproperty(QStringList, prop_labels)]
        #[qproperty(QStringList, prop_values)]
        #[qproperty(bool, show_hidden)]
        #[qproperty(bool, show_extensions)]
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
        /// Hay un listado de carpeta en curso. La ventana sigue viva mientras
        /// tanto: leer el disco ocurre en otro hilo.
        #[qproperty(bool, loading)]
        /// Estado de la operación de copiar o mover: vacío si no hay ninguna,
        /// `calculating`, `running`, `conflict`, `failure` o `summary`.
        #[qproperty(QString, op_state)]
        #[qproperty(QString, op_title)]
        /// El elemento en curso.
        #[qproperty(QString, op_current)]
        /// Fracción hecha, o -1 si todavía no se sabe.
        #[qproperty(f64, op_progress)]
        /// Cuenta, velocidad y tiempo restante en una línea.
        #[qproperty(QString, op_detail)]
        /// Conflicto: nombre del elemento, y cómo son el entrante y el que ya
        /// está. Fallo: la ruta que falló.
        #[qproperty(QString, op_name)]
        #[qproperty(QString, op_incoming)]
        #[qproperty(QString, op_existing)]
        /// Fallo: por qué. Resumen: qué no salió, una línea por elemento.
        #[qproperty(QString, op_reason)]
        /// Conflicto: si entre estas dos carpetas se puede combinar.
        #[qproperty(bool, op_can_merge)]
        /// Conflicto entre un fichero y una carpeta: casi siempre es un error
        /// de destino, y el diálogo lo dice.
        #[qproperty(bool, op_mixed_kinds)]
        /// Fallo: si reintentar puede servir de algo.
        #[qproperty(bool, op_can_retry)]
        /// Modo de vista actual, como el ordinal de `kara_core::view::ViewMode`:
        /// 0 detalles, 1 lista, 2 mosaico, 3 iconos.
        #[qproperty(i32, view_mode)]
        /// Lado del icono o la miniatura, en píxeles lógicos.
        #[qproperty(i32, icon_size)]
        /// Si queda sitio para seguir alejando o acercando.
        #[qproperty(bool, can_zoom_out)]
        #[qproperty(bool, can_zoom_in)]
        /// What the desktop says about its look, through the Settings portal:
        /// -1 when it does not say, 0 no preference, 1 dark, 2 light.
        #[qproperty(i32, desktop_scheme)]
        /// The desktop's accent colour as `#rrggbb`, or empty.
        #[qproperty(QString, desktop_accent)]
        /// The button beside each pane row: 0 none, 1 eject, 2 unmount,
        /// 3 disconnect.
        #[qproperty(QList_i32, nav_actions)]
        /// Something worth telling that is not an error: a drive being
        /// ejected, and that it can now be pulled out.
        #[qproperty(QString, notice)]
        /// Whether the notice stays until replaced (work in progress) or
        /// fades on its own.
        #[qproperty(bool, notice_sticky)]
        /// The passphrase prompt of an encrypted volume.
        #[qproperty(bool, unlock_prompt)]
        #[qproperty(QString, unlock_name)]
        #[qproperty(QString, unlock_error)]
        #[qproperty(bool, unlock_busy)]
        /// «Abrir con»: the applications for the selection, filled when the
        /// context menu opens.
        #[qproperty(QStringList, open_with_names)]
        #[qproperty(QStringList, open_with_icons)]
        /// Whether the selection is a disk image Kara can mount.
        #[qproperty(bool, menu_is_image)]
        /// «Elegir otra aplicación»: every installed application.
        #[qproperty(bool, chooser_open)]
        #[qproperty(QString, chooser_title)]
        /// The type «usar siempre» would apply to, or empty when the
        /// selection mixes types and there is no single one.
        #[qproperty(QString, chooser_kind)]
        #[qproperty(QStringList, chooser_names)]
        #[qproperty(QStringList, chooser_icons)]
        /// Whether the window should be on screen. False only while Kara,
        /// started by D-Bus to show something, waits for the request.
        #[qproperty(bool, window_wanted)]
        type App = super::AppRust;

        /// The eject (or unmount, or disconnect) button of pane row `row`.
        #[qinvokable]
        fn nav_eject(self: Pin<&mut App>, row: i32);

        /// Unlocks the volume the passphrase prompt is about.
        #[qinvokable]
        fn unlock_volume(self: Pin<&mut App>, passphrase: &QString);

        #[qinvokable]
        fn cancel_unlock(self: Pin<&mut App>);

        #[qinvokable]
        fn dismiss_notice(self: Pin<&mut App>);

        /// Fills «Abrir con» for the current selection. The context menu
        /// calls it as it opens.
        #[qinvokable]
        fn prepare_menu(self: Pin<&mut App>);

        /// Opens the selection with application `index` of «Abrir con».
        #[qinvokable]
        fn open_with(self: Pin<&mut App>, index: i32);

        /// «Elegir otra aplicación…»
        #[qinvokable]
        fn open_chooser(self: Pin<&mut App>);

        /// Opens the selection with application `index` of the chooser, and
        /// makes it the default for the type when `always` is set.
        #[qinvokable]
        fn choose_app(self: Pin<&mut App>, index: i32, always: bool);

        #[qinvokable]
        fn close_chooser(self: Pin<&mut App>);

        /// Mounts the selected disk image and opens it.
        #[qinvokable]
        fn mount_selected_image(self: Pin<&mut App>);

        /// Shift+Supr: pregunta antes de borrar la selección para siempre. No
        /// borra nada por sí sola.
        #[qinvokable]
        fn request_permanent_delete(self: Pin<&mut App>);

        /// El usuario confirmó: se borra lo que la pregunta enseñaba.
        #[qinvokable]
        fn confirm_permanent_delete(self: Pin<&mut App>);

        /// El usuario se echó atrás: no se toca nada.
        #[qinvokable]
        fn cancel_permanent_delete(self: Pin<&mut App>);

        /// Abre Propiedades de la selección, o de la carpeta actual si no hay
        /// nada seleccionado.
        #[qinvokable]
        fn show_properties(self: Pin<&mut App>);

        /// Cierra Propiedades y para el cálculo de tamaño si seguía en curso.
        #[qinvokable]
        fn close_properties(self: Pin<&mut App>);

        /// Copia como texto la ruta de la selección, o la de la carpeta actual si
        /// no hay nada seleccionado. Una por línea.
        #[qinvokable]
        fn copy_path(self: Pin<&mut App>);

        /// Abre una terminal en la carpeta de la fila `row` si es una carpeta, o
        /// en la carpeta actual si `row` es -1 o es un fichero.
        #[qinvokable]
        fn open_terminal_here(self: Pin<&mut App>, row: i32);

        /// Muestra u oculta los archivos ocultos.
        #[qinvokable]
        fn toggle_hidden(self: Pin<&mut App>);

        /// Muestra u oculta las extensiones de los nombres.
        #[qinvokable]
        fn toggle_extensions(self: Pin<&mut App>);

        /// Responde al conflicto abierto: `skip`, `keep_both`, `replace` o
        /// `merge`, y si vale para todos los que queden de su clase.
        #[qinvokable]
        fn answer_conflict(self: Pin<&mut App>, resolution: &QString, apply_to_all: bool);

        /// Responde al fallo abierto: `retry`, `skip`, `skip_all` o `cancel`.
        #[qinvokable]
        fn answer_failure(self: Pin<&mut App>, decision: &QString);

        /// Para el trabajo en curso después del fichero que se esté copiando.
        #[qinvokable]
        fn cancel_operation(self: Pin<&mut App>);

        /// Cierra el resumen final.
        #[qinvokable]
        fn dismiss_summary(self: Pin<&mut App>);

        /// Abre la entrada visible `row`: una carpeta se entra, un fichero se
        /// entrega a la aplicación que el escritorio tenga asociada. Dentro de
        /// la papelera no hace nada: lo que se ve allí no está en esa ruta.
        #[qinvokable]
        fn open_entry(self: Pin<&mut App>, row: i32);

        /// Abre el elemento con el cursor, que es lo que hace Enter.
        #[qinvokable]
        fn open_focused(self: Pin<&mut App>);

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

    // The desktop's watchers (appearance, drives, FileManager1) need the Qt
    // thread to report back to, which only exists once the object does.
    impl cxx_qt::Initialize for App {}

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
        fn clipboard_set_text(text: &str);
        #[namespace = "kara"]
        fn clipboard_uri_list() -> String;
        #[namespace = "kara"]
        fn clipboard_gnome() -> String;
        #[namespace = "kara"]
        fn clipboard_kde_cut() -> bool;

        // Window-system calls cxx-qt-lib does not wrap.
        include!("window.h");
        #[namespace = "kara"]
        fn set_desktop_file_name(name: &str);
        #[namespace = "kara"]
        fn present_window(activation_token: &str);

        type QString = cxx_qt_lib::QString;
        type QStringList = cxx_qt_lib::QStringList;
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }
}

/// La línea de detalle de la barra de progreso: «12 elementos · 35,2 MB/s ·
/// Aprox. 2 min».
fn progress_detail(meter: &kara_ops::Meter) -> String {
    let mut parts = vec![format!("{} elementos", meter.items_done())];
    if let Some(speed) = meter.bytes_per_second() {
        parts.push(format!("{}/s", present::format_size(speed as u64)));
    }
    parts.push(kara_ops::humanize(meter.eta()));
    parts.join(" · ")
}

/// Lo que se lee del disco al listar una carpeta.
struct Loaded {
    entries: Vec<kara_core::FileEntry>,
    /// Nombres que el `.hidden` de la carpeta pide ocultar.
    hidden: std::collections::BTreeSet<std::ffi::OsString>,
}

/// Lee una carpeta entera, `.hidden` incluido. Bloquea: se llama desde un hilo
/// aparte, o al arrancar.
fn read_folder(path: &Path) -> Result<Loaded, String> {
    let listing = list_directory(path).map_err(|error| error.source.to_string())?;
    Ok(Loaded {
        entries: listing.entries,
        hidden: kara_fs::read_hidden_file(path),
    })
}

/// Qué hacer con un listado cuando llega del hilo que lo leyó.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Commit {
    /// El usuario fue a una carpeta: se pinta y se apunta en el historial.
    Navigate,
    /// Atrás o Adelante: se pinta, y si ya no se puede leer se marca inválida.
    Jump,
    /// Refrescar la carpeta que se enseña.
    Reload,
    /// Pintar una carpeta de la que no había nada en memoria.
    Show,
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
    /// El último listado de disco de `raw_path`, sin filtrar ni ordenar.
    ///
    /// Ordenar, filtrar, cambiar columnas o el zoom vuelven a pintar desde aquí
    /// y no tocan el disco; solo navegar y refrescar lo leen, y eso ocurre en
    /// otro hilo.
    raw: Vec<kara_core::FileEntry>,
    raw_path: Option<PathBuf>,
    /// Los nombres que el `.hidden` de `raw_path` pide ocultar.
    raw_hidden: std::collections::BTreeSet<std::ffi::OsString>,
    /// El filtro por nombre, que es de la pestaña y no de la ventana.
    filter: String,
    /// La selección de antes de empezar el marco elástico.
    ///
    /// Hace falta por dos motivos: durante el arrastre el marco se aplica una y
    /// otra vez, y sin una base fija encogerlo no desharía nada; y la spec pide
    /// que Esc a mitad lo cancele **sin tocar la selección previa**.
    band_base: Option<Selection>,
    /// Entries to select once `folder` is listed: what «Show in folder»
    /// asked for, or a file given on the command line.
    reveal: Option<Reveal>,
}

/// A request to show entries selected in their folder, which can only be
/// met once that folder's listing has arrived from its thread.
#[derive(Debug, Clone)]
struct Reveal {
    folder: PathBuf,
    names: Vec<std::ffi::OsString>,
    /// Open Properties on them too (`ShowItemProperties`).
    properties: bool,
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
    entry_labels: QStringList,
    entry_hidden: cxx_qt_lib::QList<i32>,
    show_hidden: bool,
    show_extensions: bool,
    prop_open: bool,
    delete_prompt: bool,
    delete_text: QString,
    /// Lo que se borraría si el usuario confirma.
    delete_pending: Vec<PathBuf>,
    prop_title: QString,
    prop_labels: QStringList,
    prop_values: QStringList,
    /// Las filas de Propiedades, para poder cambiar una sin rehacer las demás.
    prop_rows: Vec<(String, String)>,
    /// Qué filas se rellenan cuando acaba de calcularse el tamaño.
    prop_size_row: Option<usize>,
    prop_contents_row: Option<usize>,
    /// Para el cálculo de tamaño en curso, si lo hay.
    prop_cancel: Option<kara_index::walk::Cancel>,
    /// Sube con cada diálogo: un tamaño que llega con otro número es de un
    /// diálogo que ya se cerró.
    prop_generation: u64,
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
    loading: bool,
    /// Número del último listado pedido. Uno que llega con otro número lo
    /// pidió alguien que ya se fue a otra carpeta, y se tira.
    list_request: u64,
    op_state: QString,
    op_title: QString,
    op_current: QString,
    op_progress: f64,
    op_detail: QString,
    op_name: QString,
    op_incoming: QString,
    op_existing: QString,
    op_reason: QString,
    op_can_merge: bool,
    op_mixed_kinds: bool,
    op_can_retry: bool,
    /// El trabajo de copiar o mover en marcha, si lo hay.
    paste_job: Option<kara_ops::runner::Handle>,
    /// Velocidad y ETA del trabajo en marcha.
    paste_meter: kara_ops::Meter,
    paste_clock: Instant,
    paste_destination: PathBuf,
    /// Si el portapapeles hay que vaciarlo cuando el trabajo acabe: un corte es
    /// de un solo uso.
    paste_clears_clipboard: bool,
    view_mode: i32,
    icon_size: i32,
    can_zoom_out: bool,
    can_zoom_in: bool,
    desktop_scheme: i32,
    desktop_accent: QString,
    nav_actions: cxx_qt_lib::QList<i32>,
    notice: QString,
    notice_sticky: bool,
    unlock_prompt: bool,
    unlock_name: QString,
    unlock_error: QString,
    unlock_busy: bool,
    open_with_names: QStringList,
    open_with_icons: QStringList,
    menu_is_image: bool,
    chooser_open: bool,
    chooser_title: QString,
    chooser_kind: QString,
    chooser_names: QStringList,
    chooser_icons: QStringList,
    window_wanted: bool,

    // Estado que no se expone a QML.
    /// Everything that depends on the desktop Kara runs on.
    desktop: Arc<dyn DesktopIntegration>,
    /// The drives and network locations, once the desktop has said; `None`
    /// until then, and the pane shows the mount table meanwhile.
    volumes: Option<Vec<Volume>>,
    /// Which volume each pane row key stands for.
    volume_rows: HashMap<PathBuf, usize>,
    /// The volume the passphrase prompt is about.
    unlock_target: Option<String>,
    /// Kept alive while Kara answers `org.freedesktop.FileManager1`.
    file_manager: Option<FileManagerService>,
    /// Started by D-Bus activation and still waiting for its first request:
    /// that request shows its folder in the first tab instead of a new one.
    awaiting_request: bool,
    /// Applications and types, read from disk, and when.
    associations: Option<(Instant, Arc<Associations>)>,
    /// What «Abrir con» and the chooser act on, and what they list.
    menu_files: Vec<PathBuf>,
    menu_mimes: Vec<String>,
    menu_apps: Vec<String>,
    chooser_apps: Vec<String>,
    /// Icons at the size the entries are drawn. `icons` stays at the pane's
    /// 16 px; a theme of bitmaps (Yaru) has another file for every size.
    entry_icon_set: Icons,
    /// The icon theme both resolvers use.
    icon_theme: String,
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
        let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
        let startup = crate::args::parse(&args, std::env::current_dir().ok().as_deref());
        // Sin `$HOME` utilizable se arranca en la raiz: es la unica carpeta que
        // seguro existe, y arrancar con la vista vacia no orienta a nadie.
        let start = startup
            .folder
            .clone()
            .or_else(|| home.clone())
            .unwrap_or_else(|| PathBuf::from("/"));
        let icon_theme = kara_fs::icons::current_theme_name();

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
            entry_labels: QStringList::default(),
            entry_hidden: cxx_qt_lib::QList::<i32>::default(),
            show_hidden: prefs.show_hidden(),
            prop_open: false,
            delete_prompt: false,
            delete_text: QString::default(),
            delete_pending: Vec::new(),
            prop_title: QString::default(),
            prop_labels: QStringList::default(),
            prop_values: QStringList::default(),
            prop_rows: Vec::new(),
            prop_size_row: None,
            prop_contents_row: None,
            prop_cancel: None,
            prop_generation: 0,
            show_extensions: prefs.show_extensions(),
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
            loading: false,
            list_request: 0,
            op_state: QString::default(),
            op_title: QString::default(),
            op_current: QString::default(),
            op_progress: -1.0,
            op_detail: QString::default(),
            op_name: QString::default(),
            op_incoming: QString::default(),
            op_existing: QString::default(),
            op_reason: QString::default(),
            op_can_merge: false,
            op_mixed_kinds: false,
            op_can_retry: false,
            paste_job: None,
            paste_meter: kara_ops::Meter::measuring(),
            paste_clock: Instant::now(),
            paste_destination: PathBuf::new(),
            paste_clears_clipboard: false,
            // Del fichero de ajustes, no de la constante: el modo que el
            // usuario dejó puesto tiene que estar aplicado ya en el primer
            // fotograma. `render` lo restaura al navegar, pero al arrancar
            // nadie ha navegado todavía.
            view_mode: mode_ordinal(initial_view.mode),
            icon_size: clamp_count(initial_view.icon_size as usize),
            can_zoom_out: !initial_view.is_smallest(),
            can_zoom_in: !initial_view.is_largest(),
            desktop_scheme: -1,
            desktop_accent: QString::default(),
            nav_actions: cxx_qt_lib::QList::<i32>::default(),
            notice: QString::default(),
            notice_sticky: false,
            unlock_prompt: false,
            unlock_name: QString::default(),
            unlock_error: QString::default(),
            unlock_busy: false,
            open_with_names: QStringList::default(),
            open_with_icons: QStringList::default(),
            menu_is_image: false,
            chooser_open: false,
            chooser_title: QString::default(),
            chooser_kind: QString::default(),
            chooser_names: QStringList::default(),
            chooser_icons: QStringList::default(),
            window_wanted: !startup.service,
            desktop: kara_desktop::detect(),
            volumes: None,
            volume_rows: HashMap::new(),
            unlock_target: None,
            file_manager: None,
            awaiting_request: startup.service,
            associations: None,
            menu_files: Vec::new(),
            menu_mimes: Vec::new(),
            menu_apps: Vec::new(),
            chooser_apps: Vec::new(),
            entry_icon_set: Icons::with_theme(&icon_theme, entry_icon_size(initial_view.icon_size)),
            tabs: Tabs::new(start.clone()),
            tab_views: HashMap::new(),
            home: home.clone(),
            crumb_capacity: DEFAULT_CRUMB_CAPACITY,
            tree: Tree::new(sections(home.as_deref(), &pinned, None)),
            // 16 píxeles es la talla del panel y de la vista de detalles. Las
            // entradas de las rejillas usan `entry_icon_set`, a su tamaño.
            icons: Icons::with_theme(&icon_theme, ICON_SIZE),
            icon_theme,
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
        // El primer listado, el único síncrono: la ventana todavía no existe y
        // no hay nada que congelar.
        if let Ok(loaded) = read_folder(&start) {
            app.store_raw(&start, loaded);
        }
        if !startup.select.is_empty() {
            app.view_mut().reveal = Some(Reveal {
                folder: start.clone(),
                names: startup.select.clone(),
                properties: false,
            });
        }
        if let Some(snapshot) = app.snapshot(&start) {
            app.version = QString::from(env!("CARGO_PKG_VERSION"));
            app.path = snapshot.path;
            app.entry_names = snapshot.names;
            app.entry_sizes = snapshot.sizes;
            app.entry_kinds = snapshot.kinds;
            app.entry_icons = snapshot.icons;
            app.entry_dirs = snapshot.dirs;
            app.entry_labels = snapshot.labels;
            app.entry_hidden = snapshot.hidden;
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

        for branch in app.reveal(&start) {
            let children = AppRust::read_children(&branch);
            app.tree.set_children(&branch, children);
        }
        let nav = app.nav_view(&start);
        app.nav_labels = nav.labels;
        app.nav_paths = nav.paths;
        app.nav_icons = nav.icons;
        app.nav_depths = nav.depths;
        app.nav_expandable = nav.expandable;
        app.nav_expanded = nav.expanded;
        app.nav_actions = nav.actions;
        app.nav_count = nav.count;
        app.nav_current = nav.current;
        app
    }
}

/// The size entry icons are looked up at for an icon drawn `side` pixels
/// wide. Rounded up to whole multiples of 8 so a zoom step inside the same
/// band does not throw away what was already found.
fn entry_icon_size(side: u32) -> u32 {
    side.clamp(16, 256).div_ceil(8) * 8
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
    labels: QStringList,
    hidden: cxx_qt_lib::QList<i32>,
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

    /// «Documento JSON», «Carpeta de archivos»… de lo que enseña Propiedades.
    fn properties_type_label(&mut self, info: &kara_fs::props::Properties) -> String {
        if info.is_symlink {
            return "Enlace simbólico".to_string();
        }
        if info.is_dir {
            return "Carpeta de archivos".to_string();
        }
        let name = info
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let described = self
            .icons
            .mime_of(&name)
            .and_then(|mime| self.descriptions.of(mime))
            .map(present::capitalize_type);
        described.unwrap_or_else(|| present::fallback_type_label(&name))
    }

    /// Guarda el listado de disco de `target` en la pestaña activa.
    fn store_raw(&mut self, target: &Path, loaded: Loaded) {
        let view = self.view_mut();
        view.raw = loaded.entries;
        view.raw_hidden = loaded.hidden;
        view.raw_path = Some(target.to_path_buf());
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
            // Sin listado en memoria de esta carpeta no hay nada que pintar:
            // quien llame lo pide en otro hilo. Leer el disco aquí es lo que
            // congelaba la ventana con una carpeta enorme o un montaje colgado.
            let view = self.view();
            if view.raw_path.as_deref() != Some(target) {
                return None;
            }
            view.raw.clone()
        };

        let visibility = Visibility {
            show_hidden: self.show_hidden,
            hidden_names: self.view().raw_hidden.clone(),
        };
        visibility.retain_visible(&mut entries);
        let total = entries.len();
        let name_display = NameDisplay {
            show_extensions: self.show_extensions,
            ..NameDisplay::default()
        };

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
                } else if id.0.as_ref() == "name" {
                    name_display.label(entry, present::looks_executable(&entry.display))
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
        // At the size this folder draws its entries: the remembered view, not
        // the one on screen, which still belongs to the previous folder.
        self.entry_icon_set
            .set_size(entry_icon_size(self.views.settings_for(target).icon_size));
        let icons = entries
            .iter()
            .map(|e| QString::from(&self.entry_icon_url(e)))
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
            labels: entries
                .iter()
                .map(|e| QString::from(&name_display.label(e, present::looks_executable(&e.display))))
                .collect(),
            hidden: ints(
                entries
                    .iter()
                    .map(|e| i32::from(visibility.is_hidden(e))),
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
    /// The button beside each row; see `present::volume_button`.
    actions: cxx_qt_lib::QList<i32>,
    count: i32,
    current: i32,
}

/// The key a pane row uses for a volume: where it is mounted, or, while it
/// is not, a relative path no folder can have, which the tree treats like any
/// other row and which lists as empty, so it never grows an arrow.
fn volume_key(volume: &Volume) -> PathBuf {
    volume
        .mount_point
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("kara-volume:{}", volume.id)))
}

/// Las secciones del panel, con las ubicaciones que hay ahora mismo.
///
/// `volumes` is what the desktop reported; `None` until it has, and then the
/// mount table stands in, as it did before Kara asked UDisks2.
fn sections(home: Option<&Path>, pinned: &[PathBuf], volumes: Option<&[Volume]>) -> Vec<Section> {
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

    let Some(volumes) = volumes else {
        return vec![
            Section {
                id: SectionId::QuickAccess,
                roots: quick,
            },
            Section {
                id: SectionId::ThisComputer,
                roots: kara_fs::places::this_computer().iter().map(branch).collect(),
            },
        ];
    };

    let root = kara_fs::places::Place {
        path: PathBuf::from("/"),
        kind: PlaceKind::Root,
        label: None,
    };
    let row = |volume: &Volume| Branch {
        name: std::ffi::OsString::from(present::volume_label(volume)),
        path: volume_key(volume),
    };
    let mut computer = vec![branch(&root)];
    computer.extend(volumes.iter().filter(|v| v.network.is_none()).map(row));
    let network: Vec<Branch> = volumes.iter().filter(|v| v.network.is_some()).map(row).collect();

    let mut out = vec![
        Section {
            id: SectionId::QuickAccess,
            roots: quick,
        },
        Section {
            id: SectionId::ThisComputer,
            roots: computer,
        },
    ];
    // «Red» appears only when there is something in it: an empty heading
    // is a promise the pane cannot keep.
    if !network.is_empty() {
        out.push(Section {
            id: SectionId::Network,
            roots: network,
        });
    }
    out
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
    /// The volume a pane row key stands for.
    fn volume_at(&self, key: &Path) -> Option<&Volume> {
        let index = *self.volume_rows.get(key)?;
        self.volumes.as_ref()?.get(index)
    }

    /// The MIME type of a path, as «Abrir con» sees it.
    fn mime_of_path(&self, path: &Path) -> String {
        if path.is_dir() {
            return "inode/directory".to_string();
        }
        path.file_name()
            .and_then(|name| self.icons.mime_of(&name.to_string_lossy()))
            .unwrap_or("application/octet-stream")
            .to_string()
    }

    /// The applications and their types, read again when the copy in memory
    /// is more than a few seconds old: installing an application or changing
    /// a default elsewhere must show up the next time the menu opens.
    fn associations(&mut self) -> Arc<Associations> {
        if let Some((read, set)) = &self.associations
            && read.elapsed() < Duration::from_secs(10)
        {
            return set.clone();
        }
        let languages: Vec<String> = TYPE_LANGUAGES.iter().map(|l| (*l).to_string()).collect();
        let set = Arc::new(Associations::load(
            &kara_desktop::BaseDirs::from_env(),
            &kara_desktop::session::current_desktops(),
            &languages,
        ));
        self.associations = Some((Instant::now(), set.clone()));
        set
    }

    /// The icon of an application: a theme name or an absolute path.
    fn app_icon_url(&mut self, icon: Option<&str>) -> String {
        match icon {
            Some(path) if path.starts_with('/') => present::file_url(Path::new(path)),
            Some(name) => self
                .icons
                .any_of(&[name, "application-x-executable"])
                .map(|p| present::file_url(&p))
                .unwrap_or_default(),
            None => self
                .icons
                .any_of(&["application-x-executable"])
                .map(|p| present::file_url(&p))
                .unwrap_or_default(),
        }
    }

    /// Aplana el árbol a listas paralelas y localiza la fila de `current`.
    fn nav_view(&mut self, current: &Path) -> NavView {
        let rows = self.tree.rows();

        let mut labels = Vec::with_capacity(rows.len());
        let mut paths = Vec::with_capacity(rows.len());
        let mut icons = Vec::with_capacity(rows.len());
        let mut depths = Vec::with_capacity(rows.len());
        let mut expandable = Vec::with_capacity(rows.len());
        let mut expanded = Vec::with_capacity(rows.len());
        let mut actions = Vec::with_capacity(rows.len());
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
                    actions.push(0);
                }
                RowKind::Folder {
                    path,
                    name,
                    expandable: can_expand,
                    expanded: is_expanded,
                } => {
                    labels.push(QString::from(&name.to_string_lossy().into_owned()));
                    paths.push(QString::from(&path.to_string_lossy().into_owned()));

                    // A volume uses its own icon, a known root its own, and
                    // whatever hangs from the tree is a folder. When the theme
                    // has none of them, a bundled icon stands in.
                    let volume = (row.depth == 1)
                        .then(|| self.volume_rows.get(path))
                        .flatten()
                        .and_then(|index| self.volumes.as_ref()?.get(*index));
                    let url = if let Some(volume) = volume {
                        actions.push(present::volume_button(volume));
                        let names: Vec<&str> = volume.icons.iter().map(String::as_str).collect();
                        self.icons
                            .any_of(&names)
                            .map_or_else(|| present::fallback_volume_icon(volume.kind).to_string(), |p| present::file_url(&p))
                    } else {
                        actions.push(0);
                        let found = match self.place_kinds.get(path).copied() {
                            Some(kind) => self.icons.any_of(present::place_icons(kind)),
                            None => self
                                .icons
                                .of(&name.to_string_lossy(), kara_core::entry::EntryKind::Directory),
                        };
                        found.map_or_else(|| present::FALLBACK_FOLDER_ICON.to_string(), |p| present::file_url(&p))
                    };
                    icons.push(QString::from(&url));
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
            actions: ints(actions),
        }
    }

    /// The icon of an entry at the entry size, or the bundled one when the
    /// theme has nothing for it.
    fn entry_icon_url(&mut self, entry: &kara_core::FileEntry) -> String {
        self.entry_icon_set
            .of(&entry.display, entry.kind)
            .map_or_else(|| present::fallback_icon(entry.kind).to_string(), |p| present::file_url(&p))
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

    /// Lee las subcarpetas de una rama.
    ///
    /// Una carpeta que no se puede leer da una lista vacía: la flecha
    /// desaparece en vez de quedarse ofreciendo algo que nunca se va a abrir.
    /// No toca el estado: se llama desde otro hilo y su resultado se entrega
    /// con [`qobject::App::request_children`].
    fn read_children(path: &Path) -> Vec<Branch> {
        let Ok(listing) = list_directory(path) else {
            return Vec::new();
        };

        let mut entries = listing.entries;
        // Solo carpetas: el panel navega, no lista contenido.
        entries.retain(|entry| entry.kind == EntryKind::Directory);
        Visibility::default().retain_visible(&mut entries);
        kara_core::sort::sort_entries(&mut entries, &SortSpec::default());

        entries
            .iter()
            .map(|entry| Branch {
                path: path.join(&entry.name),
                name: entry.name.clone(),
            })
            .collect()
    }

    /// Despliega lo necesario para que `target` se vea en el panel.
    ///
    /// Devuelve las ramas que acaban de abrirse y todavía no tienen hijos: leerlas
    /// es cosa de quien llama, en otro hilo.
    fn reveal(&mut self, target: &Path) -> Vec<PathBuf> {
        let mut to_load = Vec::new();
        for ancestor in self.tree.path_to_reveal(target) {
            if self.tree.expand(&ancestor) {
                to_load.push(ancestor);
            }
        }
        to_load
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

/// The types UDisks2 can attach as a loop device and mount.
const DISK_IMAGE_TYPES: [&str; 5] = [
    "application/x-cd-image",
    "application/x-iso9660-image",
    "application/x-raw-disk-image",
    "application/vnd.efi.iso",
    "application/vnd.efi.img",
];

/// Items grouped by their folder, in the order the folders first appear,
/// for `ShowItems`: one tab per folder, its items selected together.
fn by_folder(items: Vec<PathBuf>) -> Vec<(PathBuf, Vec<std::ffi::OsString>)> {
    let mut groups: Vec<(PathBuf, Vec<std::ffi::OsString>)> = Vec::new();
    for item in items {
        let (Some(parent), Some(name)) = (item.parent(), item.file_name()) else {
            continue;
        };
        if !parent.is_dir() {
            continue;
        }
        match groups.iter_mut().find(|(folder, _)| folder == parent) {
            Some((_, names)) => names.push(name.to_os_string()),
            None => groups.push((parent.to_path_buf(), vec![name.to_os_string()])),
        }
    }
    groups
}

impl cxx_qt::Initialize for qobject::App {
    fn initialize(self: Pin<&mut Self>) {
        self.start_desktop();
    }
}

impl qobject::App {
    /// Vuelca una vista ya calculada usando los setters, para que QML reciba las
    /// señales de cambio. Devuelve `false` si la carpeta no se pudo leer.
    fn render(mut self: Pin<&mut Self>, target: &Path) -> bool {
        let Some(view) = self.as_mut().rust_mut().get_mut().snapshot(target) else {
            // Nada en memoria de esta carpeta: se pide a otro hilo y la vista
            // se pinta cuando llegue.
            if !*self.in_trash() {
                self.as_mut().request_listing(target.to_path_buf(), Commit::Show);
            }
            return false;
        };
        self.as_mut().set_path(view.path);
        self.as_mut().set_entry_names(view.names);
        self.as_mut().set_entry_sizes(view.sizes);
        self.as_mut().set_entry_kinds(view.kinds);
        self.as_mut().set_entry_icons(view.icons);
        self.as_mut().set_entry_dirs(view.dirs);
        self.as_mut().set_entry_labels(view.labels);
        self.as_mut().set_entry_hidden(view.hidden);
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
        let to_load = self.as_mut().rust_mut().get_mut().reveal(target);
        for branch in to_load {
            self.as_mut().request_children(branch);
        }
        self.as_mut().publish_nav();

        // El rótulo de la pestaña activa es el nombre de su carpeta, así que
        // navegar lo cambia.
        self.as_mut().publish_tabs();
        self.as_mut().apply_reveal(target);
        true
    }

    // ---- The desktop -------------------------------------------------------

    /// Starts listening to the desktop: its look, its drives, and requests
    /// from other applications. Everything that may wait runs on threads and
    /// reports back through the Qt thread.
    fn start_desktop(mut self: Pin<&mut Self>) {
        let desktop = self.rust().desktop.clone();
        let thread = self.qt_thread();

        {
            let desktop = desktop.clone();
            let thread = thread.clone();
            std::thread::spawn(move || {
                if let Some(look) = desktop.appearance() {
                    let _ = thread.queue(move |app| app.apply_appearance(look));
                }
                let later = thread.clone();
                desktop.watch_appearance(Arc::new(move |look| {
                    let _ = later.queue(move |app| app.apply_appearance(look));
                }));
            });
        }

        {
            let thread = thread.clone();
            desktop.watch_volumes(Arc::new(move || {
                let _ = thread.queue(|app| app.request_volumes());
            }));
        }
        self.as_mut().request_volumes();

        // Answering FileManager1 is for the user's file manager: Kara takes
        // the name when it is the default for folders, or when the bus
        // started it for exactly that.
        let service = self.rust().awaiting_request;
        let languages: Vec<String> = TYPE_LANGUAGES.iter().map(|l| (*l).to_string()).collect();
        std::thread::spawn(move || {
            let is_default = Associations::load(
                &kara_desktop::BaseDirs::from_env(),
                &kara_desktop::session::current_desktops(),
                &languages,
            )
            .default_for("inode/directory")
            .is_some_and(|app| app.id == "kara.desktop");
            if !(service || is_default) {
                return;
            }
            let requests = thread.clone();
            let served = desktop.serve_file_manager(Arc::new(move |request| {
                let _ = requests.queue(move |app| app.handle_file_manager(request));
            }));
            if let Ok(served) = served {
                let _ = thread.queue(move |mut app| app.as_mut().rust_mut().get_mut().file_manager = Some(served));
            }
        });

        let current = self.rust().active_path();
        self.as_mut().apply_reveal(&current);
    }

    /// Applies what the desktop says about its look. The icon theme takes
    /// effect at once: both resolvers are rebuilt and what is on screen is
    /// drawn again.
    fn apply_appearance(mut self: Pin<&mut Self>, look: kara_desktop::Appearance) {
        let scheme = match look.color_scheme {
            None => -1,
            Some(ColorScheme::NoPreference) => 0,
            Some(ColorScheme::Dark) => 1,
            Some(ColorScheme::Light) => 2,
        };
        self.as_mut().set_desktop_scheme(scheme);
        let accent = look.accent.map(kara_desktop::Accent::to_hex).unwrap_or_default();
        self.as_mut().set_desktop_accent(QString::from(&accent));

        let Some(theme) = look.icon_theme else {
            return;
        };
        if theme == self.rust().icon_theme {
            return;
        }
        {
            let state = self.as_mut().rust_mut().get_mut();
            let entry_size = state.entry_icon_set.size();
            state.icons = Icons::with_theme(&theme, ICON_SIZE);
            state.entry_icon_set = Icons::with_theme(&theme, entry_size);
            state.icon_theme = theme;
        }
        let current = self.rust().active_path();
        self.as_mut().render(&current);
        self.as_mut().publish_nav();
    }

    /// Asks the desktop for its volumes on another thread: UDisks2 may be
    /// slow to answer, and the pane must not wait for it.
    fn request_volumes(self: Pin<&mut Self>) {
        let desktop = self.rust().desktop.clone();
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let volumes = desktop.volumes();
            let _ = thread.queue(move |app| app.apply_volumes(volumes));
        });
    }

    fn apply_volumes(mut self: Pin<&mut Self>, volumes: Vec<Volume>) {
        {
            let state = self.as_mut().rust_mut().get_mut();
            state.volume_rows = volumes.iter().enumerate().map(|(i, v)| (volume_key(v), i)).collect();
            for volume in volumes.iter().filter(|v| v.mount_point.is_none()) {
                // Nothing to list: no arrow.
                state.tree.set_children(&volume_key(volume), Vec::new());
            }
            state.volumes = Some(volumes);
            let pinned = state.prefs.pinned();
            let sections = sections(state.home.as_deref(), &pinned, state.volumes.as_deref());
            state.tree.set_sections(sections);
        }
        self.publish_nav();
    }

    fn nav_eject(mut self: Pin<&mut Self>, row: i32) {
        let Some(path) = self.nav_path_at(row) else {
            return;
        };
        let Some(volume) = self.rust().volume_at(&path).cloned() else {
            return;
        };
        self.as_mut().run_volume_action(volume, VolumeAction::Eject);
    }

    fn ask_passphrase(mut self: Pin<&mut Self>, volume: &Volume) {
        self.as_mut().rust_mut().get_mut().unlock_target = Some(volume.id.clone());
        self.as_mut().set_unlock_name(QString::from(&present::volume_label(volume)));
        self.as_mut().set_unlock_error(QString::default());
        self.as_mut().set_unlock_busy(false);
        self.as_mut().set_unlock_prompt(true);
    }

    fn unlock_volume(mut self: Pin<&mut Self>, passphrase: &QString) {
        let Some(id) = self.rust().unlock_target.clone() else {
            return;
        };
        let Some(volume) = self.rust().volumes.as_ref().and_then(|v| v.iter().find(|v| v.id == id)).cloned() else {
            self.as_mut().set_unlock_prompt(false);
            return;
        };
        self.as_mut().set_unlock_error(QString::default());
        self.as_mut().set_unlock_busy(true);
        self.as_mut()
            .run_volume_action(volume, VolumeAction::Unlock(passphrase.to_string()));
    }

    fn cancel_unlock(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().get_mut().unlock_target = None;
        self.as_mut().set_unlock_busy(false);
        self.as_mut().set_unlock_prompt(false);
    }

    fn show_notice(mut self: Pin<&mut Self>, text: &str, sticky: bool) {
        self.as_mut().set_notice_sticky(sticky);
        self.as_mut().set_notice(QString::from(text));
    }

    fn dismiss_notice(mut self: Pin<&mut Self>) {
        self.as_mut().set_notice_sticky(false);
        self.as_mut().set_notice(QString::default());
    }

    /// Mounts, unlocks or ejects a volume on the desktop's thread, and
    /// carries on in [`Self::finish_volume_action`].
    fn run_volume_action(mut self: Pin<&mut Self>, volume: Volume, action: VolumeAction) {
        let eject = action == VolumeAction::Eject;
        if eject {
            // The spec: no ejecting while an operation may be writing to it.
            if self.rust().paste_job.is_some() {
                self.as_mut().report(&format!(
                    "Espera a que termine la operación en curso antes de expulsar «{}».",
                    present::volume_label(&volume)
                ));
                return;
            }
            self.as_mut().show_notice(&present::ejecting_notice(&volume), true);
        }
        let thread = self.qt_thread();
        let id = volume.id.clone();
        self.rust().desktop.volume_action(
            &id,
            action,
            Box::new(move |result| {
                let _ = thread.queue(move |app| app.finish_volume_action(volume, eject, result));
            }),
        );
    }

    fn finish_volume_action(
        mut self: Pin<&mut Self>,
        volume: Volume,
        eject: bool,
        result: Result<Option<PathBuf>, VolumeError>,
    ) {
        let unlocking = *self.unlock_prompt();
        match result {
            Ok(mounted) => {
                if unlocking {
                    self.as_mut().cancel_unlock();
                }
                if eject {
                    self.as_mut().show_notice(&present::ejected_notice(&volume), false);
                    // A folder on the volume that just went is nowhere: go
                    // home rather than show what is no longer there.
                    let current = self.rust().active_path();
                    if let Some(point) = &volume.mount_point
                        && current.starts_with(point)
                    {
                        let home = self.rust().home.clone().unwrap_or_else(|| PathBuf::from("/"));
                        self.as_mut().navigate_to(&home);
                    }
                } else if let Some(point) = mounted {
                    self.as_mut().navigate_to(&point);
                }
            }
            Err(VolumeError::Dismissed) => {
                self.as_mut().set_unlock_busy(false);
                self.as_mut().dismiss_notice();
            }
            Err(error @ VolumeError::WrongPassphrase(_)) if unlocking => {
                self.as_mut().set_unlock_busy(false);
                self.as_mut().set_unlock_error(QString::from(&error.to_string()));
            }
            Err(error) => {
                if unlocking {
                    self.as_mut().cancel_unlock();
                }
                self.as_mut().dismiss_notice();
                self.as_mut().report(&error.to_string());
            }
        }
        self.as_mut().request_volumes();
    }

    fn mount_selected_image(self: Pin<&mut Self>) {
        let Some(image) = self.selected_paths().into_iter().next() else {
            return;
        };
        let thread = self.qt_thread();
        self.rust().desktop.mount_image(
            &image,
            Box::new(move |result| {
                let _ = thread.queue(move |mut app| match result {
                    Ok(Some(point)) => app.as_mut().navigate_to(&point),
                    Ok(None) | Err(VolumeError::Dismissed) => {}
                    Err(error) => app.as_mut().report(&error.to_string()),
                });
            }),
        );
    }

    // ---- «Abrir con» -----------------------------------------------------------

    fn prepare_menu(mut self: Pin<&mut Self>) {
        let files = self.selected_paths();
        let (names, icons, is_image) = {
            let state = self.as_mut().rust_mut().get_mut();
            let mut mimes: Vec<String> = Vec::new();
            for file in &files {
                let mime = state.mime_of_path(file);
                if !mimes.contains(&mime) {
                    mimes.push(mime);
                }
            }
            let set = state.associations();
            let canonical: Vec<String> = mimes.iter().map(|m| set.canonical(m)).collect();
            let mut apps: Vec<&kara_desktop::AppInfo> = set.apps_for_all(&canonical);
            // A type nothing claims still opens in a text editor, which is the
            // generic editor the spec asks to offer.
            if apps.is_empty() && !canonical.iter().any(|m| m == "inode/directory") {
                apps = set.apps_for("text/plain");
            }
            let ids: Vec<String> = apps.iter().map(|a| a.id.clone()).collect();
            let labels: Vec<String> = apps.iter().map(|a| a.name.clone()).collect();
            let icon_names: Vec<Option<String>> = apps.iter().map(|a| a.icon.clone()).collect();
            let icons: Vec<String> = icon_names.iter().map(|i| state.app_icon_url(i.as_deref())).collect();
            let is_image = files.len() == 1
                && canonical.len() == 1
                && DISK_IMAGE_TYPES.contains(&canonical[0].as_str());
            state.menu_files = files;
            state.menu_mimes = canonical;
            state.menu_apps = ids;
            (labels, icons, is_image)
        };
        self.as_mut()
            .set_open_with_names(names.iter().map(QString::from).collect());
        self.as_mut()
            .set_open_with_icons(icons.iter().map(QString::from).collect());
        self.as_mut().set_menu_is_image(is_image);
    }

    fn open_with(mut self: Pin<&mut Self>, index: i32) {
        let Some(id) = usize::try_from(index).ok().and_then(|i| self.rust().menu_apps.get(i).cloned()) else {
            return;
        };
        self.as_mut().launch_app(&id, false);
    }

    fn open_chooser(mut self: Pin<&mut Self>) {
        let (title, kind, names, icons) = {
            let state = self.as_mut().rust_mut().get_mut();
            if state.menu_files.is_empty() {
                return;
            }
            let set = state.associations();
            let apps: Vec<(String, String, Option<String>)> = set
                .all_apps()
                .into_iter()
                .map(|a| (a.id.clone(), a.name.clone(), a.icon.clone()))
                .collect();
            let icons: Vec<String> = apps.iter().map(|(_, _, icon)| state.app_icon_url(icon.as_deref())).collect();
            state.chooser_apps = apps.iter().map(|(id, _, _)| id.clone()).collect();
            let title = match state.menu_files.as_slice() {
                [one] => format!(
                    "Abrir «{}» con",
                    one.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                ),
                many => format!("Abrir {} elementos con", many.len()),
            };
            // «Usar siempre» needs one type to apply to.
            let kind = match state.menu_mimes.as_slice() {
                [mime] => state
                    .descriptions
                    .of(mime)
                    .map(present::capitalize_type)
                    .unwrap_or_else(|| mime.clone()),
                _ => String::new(),
            };
            (title, kind, apps.into_iter().map(|(_, name, _)| name).collect::<Vec<_>>(), icons)
        };
        self.as_mut().set_chooser_title(QString::from(&title));
        self.as_mut().set_chooser_kind(QString::from(&kind));
        self.as_mut()
            .set_chooser_names(names.iter().map(QString::from).collect());
        self.as_mut()
            .set_chooser_icons(icons.iter().map(QString::from).collect());
        self.as_mut().set_chooser_open(true);
    }

    fn choose_app(mut self: Pin<&mut Self>, index: i32, always: bool) {
        let Some(id) = usize::try_from(index).ok().and_then(|i| self.rust().chooser_apps.get(i).cloned()) else {
            return;
        };
        self.as_mut().set_chooser_open(false);
        self.as_mut().launch_app(&id, always);
    }

    fn close_chooser(mut self: Pin<&mut Self>) {
        self.as_mut().set_chooser_open(false);
    }

    /// Starts application `id` on the menu's files and records the choice
    /// the way GIO does, so Nautilus and Kara list the same recent picks:
    /// as the default when the user ticked «usar siempre», as the most
    /// recently used otherwise.
    fn launch_app(mut self: Pin<&mut Self>, id: &str, always: bool) {
        let (app, files, mimes) = {
            let state = self.as_mut().rust_mut().get_mut();
            let set = state.associations();
            (set.app(id).cloned(), state.menu_files.clone(), state.menu_mimes.clone())
        };
        let Some(app) = app else {
            self.as_mut().report("Esa aplicación ya no está instalada.");
            return;
        };
        if let Err(error) = self.rust().desktop.launch(&app, &files) {
            self.as_mut().report(&error.to_string());
            return;
        }
        self.as_mut().clear_error();

        let Some(list) = kara_desktop::BaseDirs::from_env().user_mimeapps() else {
            return;
        };
        let recorded = if always && mimes.len() == 1 {
            kara_desktop::apps::mimeapps::set_default(&list, &mimes[0], &app.id)
        } else {
            mimes
                .iter()
                .try_for_each(|mime| kara_desktop::apps::mimeapps::remember_used(&list, mime, &app.id))
        };
        if let Err(error) = recorded {
            self.as_mut().report(&format!("No se pudo guardar la elección de aplicación: {error}"));
        }
        // Read again next time: the order just changed.
        self.as_mut().rust_mut().get_mut().associations = None;
    }

    // ---- org.freedesktop.FileManager1 ----------------------------------------

    fn handle_file_manager(mut self: Pin<&mut Self>, request: FileManagerRequest) {
        let (groups, properties, token) = match request {
            FileManagerRequest::ShowFolders {
                folders,
                activation_token,
            } => (
                folders
                    .into_iter()
                    .filter(|f| f.is_dir())
                    .map(|f| (f, Vec::new()))
                    .collect(),
                false,
                activation_token,
            ),
            FileManagerRequest::ShowItems {
                items,
                activation_token,
            } => (by_folder(items), false, activation_token),
            FileManagerRequest::ShowItemProperties {
                items,
                activation_token,
            } => (by_folder(items), true, activation_token),
        };
        for (folder, names) in groups {
            self.as_mut().reveal_in_tab(folder, names, properties);
        }
        self.as_mut().set_window_wanted(true);
        present_window(&token);
    }

    /// Shows `folder` with `names` selected: in the first tab when the bus
    /// started Kara for this, in a new one otherwise, as Kara keeps one
    /// window and its tabs.
    fn reveal_in_tab(mut self: Pin<&mut Self>, folder: PathBuf, names: Vec<std::ffi::OsString>, properties: bool) {
        let reuse = std::mem::replace(&mut self.as_mut().rust_mut().get_mut().awaiting_request, false);
        if reuse {
            self.as_mut().navigate_to(&folder);
        } else {
            self.as_mut()
                .open_tab(&QString::from(&folder.to_string_lossy().into_owned()), false);
        }
        self.as_mut().rust_mut().get_mut().view_mut().reveal = Some(Reveal {
            folder: folder.clone(),
            names,
            properties,
        });
        // If the folder was already in memory it is on screen by now, and
        // no further render would come to apply it.
        self.as_mut().apply_reveal(&folder);
    }

    /// Selects what a [`Reveal`] asked for, once `target` is the listing on
    /// screen.
    fn apply_reveal(mut self: Pin<&mut Self>, target: &Path) {
        let ready = {
            let state = self.rust();
            let view = state.view();
            view.visible_path.as_deref() == Some(target)
                && view.reveal.as_ref().is_some_and(|r| r.folder == target)
        };
        if !ready {
            return;
        }
        let Some(reveal) = self.as_mut().rust_mut().get_mut().view_mut().reveal.take() else {
            return;
        };
        if !reveal.names.is_empty() {
            let state = self.as_mut().rust_mut().get_mut();
            let len = state.view().visible.len();
            let indices: Vec<usize> = reveal
                .names
                .iter()
                .filter_map(|name| state.view().visible.iter().position(|e| &e.name == name))
                .collect();
            let selection = &mut state.view_mut().selection;
            selection.deselect_all();
            for (n, index) in indices.iter().enumerate() {
                if n == 0 {
                    selection.click(*index, len);
                } else {
                    selection.ctrl_click(*index, len);
                }
            }
            self.as_mut().publish_selection();
        }
        if reveal.properties {
            self.as_mut().show_properties();
        }
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
        self.as_mut().set_nav_actions(nav.actions);
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
            self.as_mut().request_children(path.clone());
        }
        self.publish_nav();
    }

    fn nav_activate(mut self: Pin<&mut Self>, row: i32) {
        let Some(path) = self.nav_path_at(row) else {
            return;
        };
        // A volume with nowhere to browse yet is mounted first — after its
        // passphrase, if it is encrypted — and opened when it is.
        if let Some(volume) = self.rust().volume_at(&path).cloned()
            && volume.mount_point.is_none()
        {
            if volume.locked {
                self.as_mut().ask_passphrase(&volume);
            } else {
                self.as_mut().run_volume_action(volume, VolumeAction::Mount);
            }
            return;
        }
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
            state.tree.set_sections(sections(state.home.as_deref(), &pinned, state.volumes.as_deref()));
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
        self.as_mut().render_fresh(&current);
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
        self.as_mut().render_fresh(&current);
    }

    fn redo(mut self: Pin<&mut Self>) {
        if let Err(error) = self.as_mut().rust_mut().get_mut().undo.redo() {
            self.as_mut().report(&format!("No se pudo rehacer: {error}"));
            return;
        }
        self.as_mut().clear_error();
        self.as_mut().publish_undo();
        let current = PathBuf::from(self.path().to_string());
        self.as_mut().render_fresh(&current);
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
        // Un trabajo a la vez: pegar mientras otro corre mezclaría dos
        // diálogos sobre las mismas propiedades.
        if self.rust().paste_job.is_some() {
            self.as_mut()
                .report("Ya hay una operación en curso; espera a que termine.");
            return;
        }

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
        let op = if state.is_cut() {
            kara_ops::runner::Op::Move
        } else {
            kara_ops::runner::Op::Copy
        };
        let request = kara_ops::runner::Request {
            op,
            sources: state.paths.clone(),
            dest_dir: destination,
        };
        let clears = state.after_paste().is_none();
        self.start_job(request, clears);
    }

    /// Arranca un trabajo de copiar, mover o eliminar en su hilo.
    fn start_job(mut self: Pin<&mut Self>, request: kara_ops::runner::Request, clears: bool) {
        let op = request.op;
        let destination = request.dest_dir.clone();
        let thread = self.qt_thread();
        let handle = kara_ops::runner::spawn(request, move |event| {
            // Si el objeto ya no está, la ventana se cerró y nadie espera nada.
            let _ = thread.queue(move |app| app.on_paste_event(event));
        });

        self.as_mut().clear_error();
        let title = format!("{}…", op.gerund());
        self.as_mut().set_op_title(QString::from(&title));
        self.as_mut().set_op_state(QString::from("calculating"));
        self.as_mut().set_op_progress(-1.0);
        self.as_mut().set_op_current(QString::default());
        self.as_mut().set_op_detail(QString::from("Calculando…"));
        let state_mut = self.as_mut().rust_mut().get_mut();
        state_mut.paste_job = Some(handle);
        state_mut.paste_meter = kara_ops::Meter::measuring();
        state_mut.paste_clock = Instant::now();
        state_mut.paste_destination = destination;
        state_mut.paste_clears_clipboard = clears;
    }

    /// Lo que cuenta el hilo de copiar o mover. Llega siempre en el hilo de la
    /// interfaz, que es el único que puede tocar las propiedades.
    fn on_paste_event(mut self: Pin<&mut Self>, event: kara_ops::runner::Event) {
        use kara_ops::runner::Event;

        match event {
            Event::Calculating => {}
            Event::Started {
                total_bytes,
                total_items,
            } => {
                let meter = &mut self.as_mut().rust_mut().get_mut().paste_meter;
                meter.start(Some(total_bytes), total_items);
                self.as_mut().set_op_state(QString::from("running"));
            }
            Event::Progress {
                current,
                bytes_done,
                items_done,
            } => {
                let at = self.rust().paste_clock.elapsed().as_secs_f64();
                let (fraction, detail) = {
                    let meter = &mut self.as_mut().rust_mut().get_mut().paste_meter;
                    meter.sample(at, bytes_done, items_done);
                    (meter.fraction(), progress_detail(meter))
                };
                if !current.is_empty() {
                    self.as_mut().set_op_current(QString::from(&current));
                }
                self.as_mut().set_op_progress(fraction.unwrap_or(-1.0));
                self.as_mut().set_op_detail(QString::from(&detail));
            }
            Event::Conflict(prompt) => {
                use kara_ops::ConflictKind;
                let describe = |path: &Path| -> String {
                    match std::fs::symlink_metadata(path) {
                        Ok(meta) if meta.is_dir() => format!(
                            "Carpeta · modificada {}",
                            present::modified_label(meta.modified().ok())
                        ),
                        Ok(meta) => format!(
                            "{} · modificado {}",
                            present::format_size(meta.len()),
                            present::modified_label(meta.modified().ok())
                        ),
                        Err(_) => String::new(),
                    }
                };
                let incoming = describe(&prompt.source);
                let existing = describe(&prompt.destination);
                self.as_mut().set_op_name(QString::from(&prompt.name));
                self.as_mut().set_op_incoming(QString::from(&incoming));
                self.as_mut().set_op_existing(QString::from(&existing));
                self.as_mut()
                    .set_op_can_merge(prompt.kind == ConflictKind::DirectoryOverDirectory);
                self.as_mut().set_op_mixed_kinds(matches!(
                    prompt.kind,
                    ConflictKind::FileOverDirectory | ConflictKind::DirectoryOverFile
                ));
                self.as_mut().set_op_state(QString::from("conflict"));
            }
            Event::Failure(prompt) => {
                let name = prompt.path.display().to_string();
                self.as_mut().set_op_name(QString::from(&name));
                self.as_mut().set_op_reason(QString::from(&prompt.reason));
                self.as_mut().set_op_can_retry(prompt.kind.retry_may_help());
                self.as_mut().set_op_state(QString::from("failure"));
            }
            Event::Finished(outcome) => self.finish_paste(*outcome),
        }
    }

    fn finish_paste(mut self: Pin<&mut Self>, outcome: kara_ops::runner::Outcome) {
        let destination = self.rust().paste_destination.clone();
        let clears = self.rust().paste_clears_clipboard;
        let clean = outcome.report.is_clean();
        {
            let state = self.as_mut().rust_mut().get_mut();
            state.paste_job = None;
            for action in outcome.actions {
                state.undo.push(action);
            }
        }
        // Un corte es de un solo uso: una vez movido no queda nada en el
        // origen que volver a mover. Si se canceló o algo falló, lo que quedó
        // sin mover sigue en el portapapeles para volver a intentarlo.
        if clears && clean {
            clipboard_clear();
        }

        self.as_mut().rust_mut().get_mut().tree.forget(&destination);
        self.as_mut().publish_undo();
        // La vista puede haberse movido de carpeta mientras se copiaba.
        self.as_mut().reload();

        if clean {
            self.as_mut().set_op_state(QString::default());
            return;
        }

        let mut lines: Vec<String> = Vec::new();
        if outcome.cancelled {
            lines.push("La operación se canceló antes de terminar.".to_string());
        }
        if !outcome.report.failures.is_empty() {
            let verb = match outcome.op {
                kara_ops::runner::Op::Move => "mover",
                kara_ops::runner::Op::Copy => "copiar",
                kara_ops::runner::Op::Delete => "eliminar",
            };
            lines.push(format!(
                "{} elemento(s) no se pudieron {verb}:",
                outcome.report.failures.len()
            ));
            for failure in &outcome.report.failures {
                lines.push(format!("• {}: {}", failure.path.display(), failure.reason));
            }
        }
        self.as_mut().set_op_reason(QString::from(&lines.join("\n")));
        self.as_mut().set_op_state(QString::from("summary"));
    }

    fn answer_conflict(mut self: Pin<&mut Self>, resolution: &QString, apply_to_all: bool) {
        use kara_ops::Resolution;
        use kara_ops::runner::Answer;

        let resolution = match resolution.to_string().as_str() {
            "skip" => Resolution::Skip,
            "keep_both" => Resolution::KeepBoth,
            "replace" => Resolution::Replace,
            "merge" => Resolution::Merge,
            // Una respuesta que no existe no se adivina: se para.
            _ => {
                self.as_mut().cancel_operation();
                return;
            }
        };
        if let Some(job) = &self.rust().paste_job {
            job.answer(Answer::Conflict {
                resolution,
                apply_to_all,
            });
        }
        self.as_mut().set_op_state(QString::from("running"));
    }

    fn answer_failure(mut self: Pin<&mut Self>, decision: &QString) {
        use kara_ops::runner::Answer;

        let decision = match decision.to_string().as_str() {
            "retry" => kara_ops::ErrorDecision::Retry,
            "skip" => kara_ops::ErrorDecision::Skip,
            "skip_all" => kara_ops::ErrorDecision::SkipAll,
            _ => kara_ops::ErrorDecision::Cancel,
        };
        if let Some(job) = &self.rust().paste_job {
            job.answer(Answer::Error(decision));
        }
        self.as_mut().set_op_state(QString::from("running"));
    }

    fn request_permanent_delete(mut self: Pin<&mut Self>) {
        if *self.in_trash() {
            self.as_mut()
                .report("Dentro de la papelera, lo definitivo es «Vaciar la papelera».");
            return;
        }
        if self.rust().paste_job.is_some() {
            self.as_mut()
                .report("Ya hay una operación en curso; espera a que termine.");
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }

        // La pregunta dice cuántos y, si es uno, cuál: es lo que impide un
        // borrado masivo por un Shift+Supr con la selección equivocada.
        let text = match paths.as_slice() {
            [one] => format!(
                "¿Eliminar permanentemente «{}»?\nEsta acción no se puede deshacer.",
                one.file_name().map_or_else(|| one.display().to_string(), |n| n.to_string_lossy().into_owned())
            ),
            many => format!(
                "¿Eliminar permanentemente estos {} elementos?\nEsta acción no se puede deshacer.",
                many.len()
            ),
        };
        self.as_mut().rust_mut().get_mut().delete_pending = paths;
        self.as_mut().set_delete_text(QString::from(&text));
        self.as_mut().set_delete_prompt(true);
    }

    fn confirm_permanent_delete(mut self: Pin<&mut Self>) {
        let paths = std::mem::take(&mut self.as_mut().rust_mut().get_mut().delete_pending);
        self.as_mut().set_delete_prompt(false);
        if paths.is_empty() {
            return;
        }
        let request = kara_ops::runner::Request {
            op: kara_ops::runner::Op::Delete,
            sources: paths,
            dest_dir: PathBuf::from(self.path().to_string()),
        };
        self.start_job(request, false);
    }

    fn cancel_permanent_delete(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().get_mut().delete_pending.clear();
        self.as_mut().set_delete_prompt(false);
    }

    fn show_properties(mut self: Pin<&mut Self>) {
        if *self.in_trash() {
            return;
        }
        // Un diálogo anterior que siguiera calculando ya no le importa a nadie.
        self.as_mut().stop_size_calculation();

        let mut targets = self.selected_paths();
        if targets.is_empty() {
            targets.push(PathBuf::from(self.path().to_string()));
        }

        let mut infos = Vec::new();
        let mut unreadable = Vec::new();
        for target in &targets {
            match kara_fs::props::properties(target) {
                Ok(info) => infos.push(info),
                Err(error) => unreadable.push(format!("{}: {error}", target.display())),
            }
        }
        if infos.is_empty() {
            let message = format!("No se pudieron leer las propiedades: {}", unreadable.join("; "));
            self.as_mut().report(&message);
            return;
        }

        let single_label = if infos.len() == 1 {
            let state = self.as_mut().rust_mut().get_mut();
            Some(state.properties_type_label(&infos[0]))
        } else {
            None
        };
        let layout = present::properties_rows(&infos, single_label.as_deref());

        let title = if infos.len() == 1 {
            let name = infos[0]
                .path
                .file_name()
                .map_or_else(|| infos[0].path.display().to_string(), |n| n.to_string_lossy().into_owned());
            format!("Propiedades de {name}")
        } else {
            format!("Propiedades ({} elementos)", infos.len())
        };

        let generation = {
            let state = self.as_mut().rust_mut().get_mut();
            state.prop_generation += 1;
            state.prop_rows = layout.rows.clone();
            state.prop_size_row = layout.size_row;
            state.prop_contents_row = layout.contents_row;
            state.prop_generation
        };
        self.as_mut().set_prop_title(QString::from(&title));
        self.as_mut().publish_properties();
        self.as_mut().set_prop_open(true);

        // Las carpetas no tienen un tamaño que leer: hay que recorrerlas, y
        // eso va en otro hilo con su progreso y su cancelación.
        let folders: Vec<PathBuf> = infos
            .iter()
            .filter(|info| info.is_dir && !info.is_symlink)
            .map(|info| info.path.clone())
            .collect();
        if folders.is_empty() {
            return;
        }
        let files_bytes: u64 = infos
            .iter()
            .filter(|info| !info.is_dir || info.is_symlink)
            .map(|info| info.size)
            .sum();
        let cancel = kara_index::walk::Cancel::new();
        self.as_mut().rust_mut().get_mut().prop_cancel = Some(cancel.clone());
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let mut done = kara_index::size::SizeProgress::default();
            let mut partial = false;
            for (position, folder) in folders.iter().enumerate() {
                let finished_before = done;
                let report = kara_index::size::folder_size(folder, &cancel, |progress| {
                    let now = present::add_progress(finished_before, progress);
                    let update = present::SizeUpdate::new(files_bytes, now, false, false);
                    let _ = thread.queue(move |app| app.apply_size(generation, update));
                });
                done = present::add_progress(finished_before, report.totals);
                partial |= report.is_partial();
                if cancel.is_cancelled() {
                    return;
                }
                let last = position + 1 == folders.len();
                if last {
                    let update = present::SizeUpdate::new(files_bytes, done, true, partial);
                    let _ = thread.queue(move |app| app.apply_size(generation, update));
                }
            }
        });
    }

    /// Un tamaño calculado en segundo plano llega al diálogo abierto.
    fn apply_size(mut self: Pin<&mut Self>, generation: u64, update: present::SizeUpdate) {
        if generation != self.rust().prop_generation || !*self.prop_open() {
            return;
        }
        {
            let state = self.as_mut().rust_mut().get_mut();
            if let Some(row) = state.prop_size_row.and_then(|row| state.prop_rows.get_mut(row)) {
                row.1 = update.size_text();
            }
            if let Some(row) = state.prop_contents_row.and_then(|row| state.prop_rows.get_mut(row)) {
                row.1 = update.contents_text();
            }
        }
        self.publish_properties();
    }

    fn publish_properties(mut self: Pin<&mut Self>) {
        let (labels, values): (Vec<QString>, Vec<QString>) = self
            .rust()
            .prop_rows
            .iter()
            .map(|(label, value)| (QString::from(label), QString::from(value)))
            .unzip();
        self.as_mut().set_prop_labels(labels.into_iter().collect());
        self.as_mut().set_prop_values(values.into_iter().collect());
    }

    fn stop_size_calculation(mut self: Pin<&mut Self>) {
        if let Some(cancel) = self.as_mut().rust_mut().get_mut().prop_cancel.take() {
            cancel.cancel();
        }
    }

    fn close_properties(mut self: Pin<&mut Self>) {
        self.as_mut().stop_size_calculation();
        self.as_mut().rust_mut().get_mut().prop_generation += 1;
        self.as_mut().set_prop_open(false);
    }

    fn copy_path(mut self: Pin<&mut Self>) {
        // En la papelera las filas no están en la ruta que enseña la barra.
        if *self.in_trash() {
            return;
        }
        let mut paths = self.selected_paths();
        if paths.is_empty() {
            paths.push(PathBuf::from(self.path().to_string()));
        }
        clipboard_set_text(&present::paths_as_text(&paths));
        self.as_mut().clear_error();
    }

    fn open_terminal_here(mut self: Pin<&mut Self>, row: i32) {
        if *self.in_trash() {
            return;
        }
        let current = PathBuf::from(self.path().to_string());
        let folder = usize::try_from(row)
            .ok()
            .and_then(|row| self.rust().view().visible.get(row))
            .filter(|entry| entry.kind == EntryKind::Directory)
            .map_or_else(|| current.clone(), |entry| current.join(&entry.name));
        let thread = self.qt_thread();
        self.rust().desktop.open_terminal(
            &folder,
            Arc::new(move |error| {
                let message = error.to_string();
                let _ = thread.queue(move |app| app.report(&message));
            }),
        );
        self.as_mut().clear_error();
    }

    fn toggle_hidden(mut self: Pin<&mut Self>) {
        let on = !*self.show_hidden();
        self.as_mut().set_show_hidden(on);
        self.as_mut().rust_mut().get_mut().prefs.set_show_hidden(on);
        self.as_mut().persist();
        let current = PathBuf::from(self.path().to_string());
        self.as_mut().render(&current);
    }

    fn toggle_extensions(mut self: Pin<&mut Self>) {
        let on = !*self.show_extensions();
        self.as_mut().set_show_extensions(on);
        self.as_mut().rust_mut().get_mut().prefs.set_show_extensions(on);
        self.as_mut().persist();
        let current = PathBuf::from(self.path().to_string());
        self.as_mut().render(&current);
    }

    fn cancel_operation(self: Pin<&mut Self>) {
        if let Some(job) = &self.rust().paste_job {
            job.cancel();
        }
    }

    fn dismiss_summary(mut self: Pin<&mut Self>) {
        self.as_mut().set_op_state(QString::default());
        self.as_mut().set_op_reason(QString::default());
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
        // Lo que la pestaña tenía en memoria se enseña al instante y se refresca
        // por detrás; sin nada en memoria, `render` lo pide. Cualquier listado
        // pendiente de la pestaña anterior ya no es de nadie.
        self.as_mut().rust_mut().get_mut().list_request += 1;
        self.as_mut().set_loading(false);
        if self.as_mut().render(&path) && !in_trash {
            self.as_mut().request_listing(path.clone(), Commit::Reload);
        }
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
        self.as_mut().render_fresh(&current);
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

        let wanted = entry_icon_size(settings.icon_size);
        if self.rust().entry_icon_set.size() != wanted {
            let icons: QStringList = {
                let state = self.as_mut().rust_mut().get_mut();
                state.entry_icon_set.set_size(wanted);
                let visible = state.view().visible.clone();
                visible.iter().map(|e| QString::from(&state.entry_icon_url(e))).collect()
            };
            self.as_mut().set_entry_icons(icons);
        }

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
    fn navigate_to(self: Pin<&mut Self>, target: &Path) {
        self.request_listing(target.to_path_buf(), Commit::Navigate);
    }

    /// Lee las subcarpetas de una rama del panel en otro hilo y, cuando llegan,
    /// se las entrega al árbol. Desplegar un montaje colgado deja de congelar
    /// la ventana.
    fn request_children(self: Pin<&mut Self>, path: PathBuf) {
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let children = AppRust::read_children(&path);
            let _ = thread.queue(move |mut app| {
                app.as_mut().rust_mut().get_mut().tree.set_children(&path, children);
                app.publish_nav();
            });
        });
    }

    /// Pide el listado de `target` a otro hilo y, cuando llega, lo pinta y
    /// aplica `commit`. La ventana no espera: un volumen de red colgado deja de
    /// ser un congelado y pasa a ser un «Cargando…» del que se puede salir
    /// navegando a otro sitio.
    fn request_listing(mut self: Pin<&mut Self>, target: PathBuf, commit: Commit) {
        let id = {
            let state = self.as_mut().rust_mut().get_mut();
            state.list_request += 1;
            state.list_request
        };
        self.as_mut().set_loading(true);

        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let result = read_folder(&target);
            // Si la ventana se cerró, nadie espera el resultado.
            let _ = thread.queue(move |app| app.finish_listing(id, target, commit, result));
        });
    }

    /// Recibe un listado pedido con [`Self::request_listing`].
    fn finish_listing(
        mut self: Pin<&mut Self>,
        id: u64,
        target: PathBuf,
        commit: Commit,
        result: Result<Loaded, String>,
    ) {
        // Otro listado se pidió después: este es de una carpeta que el usuario
        // ya dejó atrás.
        if id != self.rust().list_request {
            return;
        }
        self.as_mut().set_loading(false);

        let loaded = match result {
            Ok(loaded) => loaded,
            Err(reason) => {
                // Un salto por el historial que ya no se puede listar se marca
                // inválido y no se mueve: la carpeta pudo desaparecer.
                if commit == Commit::Jump {
                    self.as_mut()
                        .rust_mut()
                        .get_mut()
                        .tabs
                        .active_mut()
                        .history_mut()
                        .invalidate(&target);
                    self.as_mut().publish_history();
                }
                let message = format!("No se pudo abrir {}: {reason}", target.display());
                self.as_mut().report(&message);
                return;
            }
        };

        // Un refresco o un cambio de pestaña solo vale si la vista sigue en esa
        // carpeta.
        if matches!(commit, Commit::Reload | Commit::Show) && self.rust().active_path() != target {
            return;
        }

        if matches!(commit, Commit::Navigate | Commit::Jump) {
            self.as_mut().leave_trash();
        }
        self.as_mut().rust_mut().get_mut().store_raw(&target, loaded);
        if !self.as_mut().render(&target) {
            return;
        }
        if commit == Commit::Navigate {
            self.as_mut()
                .rust_mut()
                .get_mut()
                .tabs
                .active_mut()
                .history_mut()
                .visit(&target);
        }
        if matches!(commit, Commit::Navigate | Commit::Jump) {
            self.as_mut().clear_error();
        }
        self.as_mut().publish_history();
    }

    /// Relee del disco **ahora** y repinta. Solo para después de que Kara
    /// misma haya tocado la carpeta —crear, renombrar, deshacer—: esa operación
    /// ya demostró que la carpeta responde, y quien llama necesita ver el
    /// resultado ya (el editor de renombrado busca la entrada recién creada).
    fn render_fresh(mut self: Pin<&mut Self>, target: &Path) -> bool {
        if !*self.in_trash() {
            let Ok(loaded) = read_folder(target) else {
                return false;
            };
            self.as_mut().rust_mut().get_mut().store_raw(target, loaded);
        }
        self.render(target)
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

        self.request_listing(target, Commit::Jump);
    }

    fn open_entry(mut self: Pin<&mut Self>, row: i32) {
        if *self.in_trash() {
            return;
        }
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        let current = PathBuf::from(self.path().to_string());
        let Some((target, is_dir)) = self.rust().view().visible.get(row).map(|entry| {
            (
                current.join(&entry.name),
                matches!(entry.kind, kara_core::entry::EntryKind::Directory),
            )
        }) else {
            return;
        };

        if is_dir {
            self.as_mut().navigate_to(&target);
            return;
        }
        // The desktop launches it on a thread of its own; a failure comes
        // back here to be shown.
        let thread = self.qt_thread();
        self.rust().desktop.open(
            &target,
            Arc::new(move |error| {
                let message = error.to_string();
                let _ = thread.queue(move |app| app.report(&message));
            }),
        );
        self.as_mut().clear_error();
    }

    fn open_focused(mut self: Pin<&mut Self>) {
        let focused = self.rust().view().selection.focused();
        if let Some(row) = focused.and_then(|row| i32::try_from(row).ok()) {
            self.as_mut().open_entry(row);
        }
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
        // Ya no se sabe aquí si la carpeta se puede abrir: se sabrá cuando el
        // listado llegue, y si no, el aviso lo dice.
        self.as_mut().navigate_to(&target);
        true
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
        if *self.in_trash() {
            self.as_mut().render(&current);
        } else {
            self.as_mut().request_listing(current, Commit::Reload);
        }
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
