pragma ComponentBehavior: Bound

// Los dos botones que eligen cómo se navega: una sola pestaña o varias.
//
// El pictograma se dibuja con rectángulos y no con un glifo, por lo mismo que
// el conmutador de vista: un símbolo que la tipografía no tenga sale como una
// caja vacía.
import QtQuick
import QtQuick.Controls
import com.kara.ui

Item {
    id: control

    /// `true` para el botón de concentración, `false` para el de pestañas.
    required property bool focused
    required property bool active
    property string tip: ""

    signal clicked

    readonly property color ink: control.active ? Theme.accent : Theme.textDim

    implicitWidth: 26
    implicitHeight: 20

    Rectangle {
        anchors.fill: parent
        anchors.margins: 1
        radius: Theme.radius
        color: {
            if (control.active)
                return Theme.selection;
            return area.containsMouse ? Theme.hover : "transparent";
        }
    }

    // Concentración: un solo panel.
    Rectangle {
        anchors.centerIn: parent
        visible: control.focused
        width: 13
        height: 10
        radius: 1
        color: "transparent"
        border.width: 1
        border.color: control.ink
    }

    // Pestañas: uno delante y dos detrás asomando.
    Row {
        anchors.centerIn: parent
        visible: !control.focused
        spacing: 2

        Repeater {
            model: 3
            Rectangle {
                required property int index
                width: 4
                height: 10
                radius: 1
                color: index === 0 ? control.ink : "transparent"
                border.width: 1
                border.color: control.ink
            }
        }
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: control.clicked()
    }

    ToolTip.visible: control.tip !== "" && area.containsMouse
    ToolTip.text: control.tip
    ToolTip.delay: 600
}
