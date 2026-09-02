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

    /// La vista marca esta entrada como la que tiene el foco.
    signal picked

    anchors.fill: parent
    hoverEnabled: true
    acceptedButtons: Qt.LeftButton | Qt.RightButton

    onClicked: mouse => {
        control.picked();
        if (mouse.button === Qt.RightButton)
            entryMenu.popup();
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
        MenuItem {
            text: qsTr("Enviar a la papelera")
            onTriggered: control.app.trash(control.app.entry_names[control.index])
        }
    }
}
