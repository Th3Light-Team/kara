import QtQuick
import QtQuick.Window
import com.kara.ui

// Ventana mínima de arranque. La UI real se migra desde el spike de
// `ground/` (Main.qml, FolderTree.qml, Theme.qml), quitándole el fondo
// flotante y los datos falsos.
Window {
    width: 1100
    height: 700
    visible: true
    title: qsTr("Kara")

    App { id: app }

    Text {
        anchors.centerIn: parent
        text: "Kara " + app.version
        font.pixelSize: 24
    }
}
