pragma ComponentBehavior: Bound

// Lo que se puede hacer sobre una entrada, sea cual sea el modo de vista.
//
// Vive aparte porque los cuatro modos comparten los mismos gestos —marcar,
// entrar, menú contextual— y duplicarlos en cada rejilla es la forma segura de
// que acaben divergiendo.
import QtQuick
import QtQuick.Controls

MouseArea {
    id: control

    required property var app
    required property int index

    /// La vista abre el editor de nombre sobre esta entrada.
    signal renameRequested

    // One application of «Abrir con». A menu cannot hold a Repeater, and the
    // list is short — the type's default, what the user picked recently, what
    // declares the type — so a fixed set of slots, hidden when unused, does.
    component AppItem: MenuItem {
        required property var app
        required property int slot
        text: app.open_with_names[slot] ?? ""
        icon.source: app.open_with_icons[slot] ?? ""
        icon.color: "transparent"
        visible: slot < app.open_with_names.length
        height: visible ? implicitHeight : 0
        onTriggered: app.open_with(slot)
    }

    anchors.fill: parent
    hoverEnabled: true
    acceptedButtons: Qt.LeftButton | Qt.RightButton

    // Pulsar sobre un fichero le da el foco de teclado a la vista. Sin esto el
    // foco se queda donde lo dejó la última barra de texto —el filtro, la de
    // direcciones— y las teclas que un campo de texto reclama para sí,
    // Ctrl+A y Supr entre ellas, no llegan nunca a la lista.
    onPressed: control.forceActiveFocus()

    onClicked: mouse => {
        const ctrl = (mouse.modifiers & Qt.ControlModifier) !== 0;
        const shift = (mouse.modifiers & Qt.ShiftModifier) !== 0;

        if (mouse.button === Qt.RightButton) {
            // Pulsar con el derecho sobre algo que no está seleccionado lo
            // selecciona antes de abrir el menú: si no, las acciones actuarían
            // sobre otra cosa distinta de la que se acaba de señalar.
            if ((control.app.entry_selected[control.index] ?? 0) === 0)
                control.app.click_entry(control.index, false, false);
            entryMenu.popup();
            return;
        }

        control.app.click_entry(control.index, ctrl, shift);
    }

    // Si es carpeta lo dice el modelo, no la columna «Tipo»: ese texto es una
    // descripción traducida del sistema y compararla ata el comportamiento al
    // idioma.
    onDoubleClicked: control.app.open_entry(control.index)

    Menu {
        id: entryMenu

        /// Ruta absoluta de la entrada, que es lo que entiende `toggle_pinned`.
        property string folder: ""
        property bool pinned: false
        onAboutToShow: {
            control.app.prepare_menu();
            const separador = control.app.path.endsWith("/") ? "" : "/";
            entryMenu.folder = control.app.path + separador + control.app.entry_names[control.index];
            entryMenu.pinned = control.app.is_pinned(entryMenu.folder);
        }
        MenuItem {
            text: qsTr("Abrir")
            visible: !control.app.in_trash
            height: visible ? implicitHeight : 0
            onTriggered: control.app.open_entry(control.index)
        }
        // Reference: `ground/spec/06-contexto-power.md`, «Abrir con».
        Menu {
            id: openWith
            title: qsTr("Abrir con")
            enabled: !control.app.in_trash

            AppItem {
                app: control.app
                slot: 0
            }
            AppItem {
                app: control.app
                slot: 1
            }
            AppItem {
                app: control.app
                slot: 2
            }
            AppItem {
                app: control.app
                slot: 3
            }
            AppItem {
                app: control.app
                slot: 4
            }
            AppItem {
                app: control.app
                slot: 5
            }
            AppItem {
                app: control.app
                slot: 6
            }
            AppItem {
                app: control.app
                slot: 7
            }
            MenuSeparator {
                visible: control.app.open_with_names.length > 0
            }
            MenuItem {
                text: qsTr("Elegir otra aplicación…")
                onTriggered: control.app.open_chooser()
            }
        }
        MenuItem {
            // A disk image becomes a drive of its own: «montar una partición o
            // imagen ISO la hace navegable».
            text: qsTr("Montar")
            visible: !control.app.in_trash && control.app.menu_is_image
            height: visible ? implicitHeight : 0
            onTriggered: control.app.mount_selected_image()
        }
        MenuItem {
            text: qsTr("Abrir en una pestaña nueva")
            visible: !control.app.in_trash && (control.app.entry_dirs[control.index] ?? 0) !== 0
            height: visible ? implicitHeight : 0
            onTriggered: control.app.open_tab(entryMenu.folder, false)
        }
        MenuSeparator {
            visible: !control.app.in_trash
        }
        MenuItem {
            text: qsTr("Restaurar")
            visible: control.app.in_trash
            height: visible ? implicitHeight : 0
            onTriggered: control.app.restore_selected()
        }
        MenuItem {
            text: qsTr("Cortar")
            visible: !control.app.in_trash
            height: visible ? implicitHeight : 0
            onTriggered: control.app.cut_selection()
        }
        MenuItem {
            text: qsTr("Copiar")
            visible: !control.app.in_trash
            height: visible ? implicitHeight : 0
            onTriggered: control.app.copy_selection()
        }
        MenuItem {
            text: qsTr("Pegar")
            visible: !control.app.in_trash
            height: visible ? implicitHeight : 0
            onTriggered: control.app.paste()
        }
        MenuSeparator {}
        MenuItem {
            text: qsTr("Renombrar")
            visible: !control.app.in_trash
            // Renombrar es de una en una: con varias seleccionadas hace falta
            // el renombrado por lotes, que es otra conveniencia.
            enabled: control.app.selected_count <= 1
            onTriggered: control.renameRequested()
        }
        MenuItem {
            text: control.app.selected_count > 1 ? qsTr("Enviar %1 elementos a la papelera").arg(control.app.selected_count) : qsTr("Enviar a la papelera")
            visible: !control.app.in_trash
            height: visible ? implicitHeight : 0
            onTriggered: control.app.trash_selected()
        }
        MenuSeparator {}
        MenuItem {
            text: entryMenu.pinned ? qsTr("Quitar del Acceso rápido") : qsTr("Anclar a Acceso rápido")
            // Solo las carpetas se anclan: el Acceso rápido son ubicaciones.
            visible: (control.app.entry_dirs[control.index] ?? 0) !== 0
            height: visible ? implicitHeight : 0
            onTriggered: control.app.toggle_pinned(entryMenu.folder)
        }
        MenuSeparator {}
        MenuItem {
            text: control.app.selected_count > 1 ? qsTr("Copiar rutas\tCtrl+Shift+C") : qsTr("Copiar ruta\tCtrl+Shift+C")
            visible: !control.app.in_trash
            height: visible ? implicitHeight : 0
            onTriggered: control.app.copy_path()
        }
        MenuItem {
            text: qsTr("Abrir terminal aquí")
            visible: !control.app.in_trash
            height: visible ? implicitHeight : 0
            onTriggered: control.app.open_terminal_here(control.index)
        }
        MenuItem {
            text: qsTr("Eliminar permanentemente\tShift+Supr")
            visible: !control.app.in_trash
            height: visible ? implicitHeight : 0
            onTriggered: control.app.request_permanent_delete()
        }
        MenuItem {
            text: qsTr("Propiedades\tAlt+Intro")
            visible: !control.app.in_trash
            height: visible ? implicitHeight : 0
            onTriggered: control.app.show_properties()
        }
        MenuSeparator {}
        MenuItem {
            text: qsTr("Nueva carpeta")
            onTriggered: control.app.create_folder(qsTr("Nueva carpeta"))
        }
    }
}
