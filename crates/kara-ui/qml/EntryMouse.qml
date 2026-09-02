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

    anchors.fill: parent
    hoverEnabled: true
    acceptedButtons: Qt.LeftButton | Qt.RightButton

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
    onDoubleClicked: {
        if ((control.app.entry_dirs[control.index] ?? 0) !== 0)
            control.app.cd(control.app.entry_names[control.index]);
    }

    Menu {
        id: entryMenu

        /// Ruta absoluta de la entrada, que es lo que entiende `toggle_pinned`.
        property string folder: ""
        property bool pinned: false
        onAboutToShow: {
            const separador = control.app.path.endsWith("/") ? "" : "/";
            entryMenu.folder = control.app.path + separador + control.app.entry_names[control.index];
            entryMenu.pinned = control.app.is_pinned(entryMenu.folder);
        }
        MenuItem {
            text: qsTr("Renombrar")
            // Renombrar es de una en una: con varias seleccionadas hace falta
            // el renombrado por lotes, que es otra conveniencia.
            enabled: control.app.selected_count <= 1
            onTriggered: control.renameRequested()
        }
        MenuItem {
            text: control.app.selected_count > 1 ? qsTr("Enviar %1 elementos a la papelera").arg(control.app.selected_count) : qsTr("Enviar a la papelera")
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
            text: qsTr("Nueva carpeta")
            onTriggered: control.app.create_folder(qsTr("Nueva carpeta"))
        }
    }
}
