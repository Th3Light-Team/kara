pragma ComponentBehavior: Bound

// Vista de detalles: una fila por entrada, con columnas.
//
// Referencia: `ground/spec/03-vistas.md`, «Modos de vista». Es el modo por
// defecto porque es el que aguanta una carpeta con diez mil ficheros.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.kara.ui

Item {
    id: view

    required property var app

    // Anchos compartidos por la cabecera y las filas. Si se separan, las
    // columnas dejan de alinearse en cuanto se toca una.
    readonly property int typeWidth: 210
    readonly property int sizeWidth: 110
    readonly property int nameMinimum: 320
    readonly property int nameMaximum: 560

    Rectangle {
        id: columnHeader
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: 30
        color: Theme.content

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 14
            anchors.rightMargin: 22
            spacing: 12

            // Hueco del icono: la cabecera tiene que llevar el mismo que las
            // filas o las columnas dejan de alinearse.
            Item {
                Layout.preferredWidth: view.app.icon_size
            }
            Text {
                text: qsTr("Nombre")
                Layout.fillWidth: true
                Layout.preferredWidth: view.nameMinimum
                Layout.maximumWidth: view.nameMaximum
                color: Theme.headerText
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
            }
            Text {
                text: qsTr("Tipo")
                Layout.preferredWidth: view.typeWidth
                color: Theme.headerText
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
            }
            Text {
                text: qsTr("Tamaño")
                Layout.preferredWidth: view.sizeWidth
                horizontalAlignment: Text.AlignRight
                color: Theme.headerText
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
            }
            // Lo que sobra a la derecha se deja en blanco: estirar las columnas
            // hasta el borde en una pantalla ancha separa el nombre de su
            // tamaño hasta hacerlos ilegibles juntos.
            Item {
                Layout.fillWidth: true
                Layout.preferredWidth: 0
            }
        }

        Rectangle {
            anchors.bottom: parent.bottom
            width: parent.width
            height: 1
            color: Theme.divider
        }
    }

    ListView {
        id: rows
        anchors.top: columnHeader.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        clip: true
        focus: true
        model: view.app.entry_count
        currentIndex: -1
        highlightMoveDuration: 0
        ScrollBar.vertical: ScrollBar {}

        delegate: Item {
            id: row

            required property int index
            readonly property bool current: rows.currentIndex === row.index

            width: rows.width
            height: Math.max(Theme.rowHeight, view.app.icon_size + 8)

            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 4
                anchors.rightMargin: 4
                radius: Theme.radius
                color: {
                    if (row.current)
                        return Theme.selection;
                    return mouse.containsMouse ? Theme.hover : "transparent";
                }
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 14
                anchors.rightMargin: 22
                spacing: 12

                EntryIcon {
                    app: view.app
                    index: row.index
                    side: view.app.icon_size
                    Layout.preferredWidth: view.app.icon_size
                    Layout.preferredHeight: view.app.icon_size
                }
                Text {
                    text: view.app.entry_names[row.index] ?? ""
                    Layout.fillWidth: true
                    Layout.preferredWidth: view.nameMinimum
                    Layout.maximumWidth: view.nameMaximum
                    elide: Text.ElideMiddle
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }
                Text {
                    text: view.app.entry_kinds[row.index] ?? ""
                    Layout.preferredWidth: view.typeWidth
                    elide: Text.ElideRight
                    color: Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }
                Text {
                    text: view.app.entry_sizes[row.index] ?? ""
                    Layout.preferredWidth: view.sizeWidth
                    horizontalAlignment: Text.AlignRight
                    color: Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }
                Item {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 0
                }
            }

            EntryMouse {
                id: mouse
                app: view.app
                index: row.index
                onPicked: rows.currentIndex = row.index
            }
        }
    }
}
