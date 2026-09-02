pragma ComponentBehavior: Bound

// Una cabecera de columna de la vista de detalles.
//
// Referencia: `ground/spec/03-vistas.md`, «Ordenar con clic en cabecera de
// columna». El primer clic ordena ascendente, el segundo invierte, y cambiar de
// columna vuelve a ascendente — pero eso lo decide `kara_core::sort`; aquí solo
// se avisa de que se ha pulsado y se pinta la flecha de la que está activa.
import QtQuick
import com.kara.ui

Item {
    id: header

    required property var app
    /// Identificador que entiende `kara_core::sort`: «name», «size», «modified».
    required property string column
    required property string label
    property int alignment: Text.AlignLeft

    readonly property bool active: header.app.sort_column === header.column

    implicitHeight: 30

    Rectangle {
        anchors.fill: parent
        anchors.topMargin: 3
        anchors.bottomMargin: 3
        radius: Theme.radius
        color: area.containsMouse ? Theme.hover : "transparent"
    }

    Row {
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: header.alignment === Text.AlignLeft ? parent.left : undefined
        anchors.right: header.alignment === Text.AlignRight ? parent.right : undefined
        anchors.leftMargin: 6
        anchors.rightMargin: 6
        spacing: 4

        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: header.label
            // La columna activa se lee un punto más fuerte que las demás: la
            // flecha sola es fácil de pasar por alto en una cabecera estrecha.
            color: header.active ? Theme.text : Theme.headerText
            font.family: Theme.family
            font.pixelSize: Theme.sizeSmall
        }
        Text {
            anchors.verticalCenter: parent.verticalCenter
            visible: header.active
            text: header.app.sort_ascending ? "▲" : "▼"
            color: Theme.accent
            font.pixelSize: 8
        }
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: header.app.sort_by(header.column)
    }
}
