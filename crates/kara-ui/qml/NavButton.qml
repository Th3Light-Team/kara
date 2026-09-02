// Botón cuadrado de la barra de comandos: atrás, adelante, subir, refrescar.
import QtQuick
import QtQuick.Controls
import com.kara.ui

Item {
    id: control

    required property string glyph
    property bool enabled: true
    property string tip: ""

    signal clicked

    implicitWidth: 34
    implicitHeight: 32

    Rectangle {
        anchors.fill: parent
        radius: Theme.radius
        color: {
            if (!control.enabled || !area.containsMouse)
                return "transparent";
            return area.pressed ? Theme.pressed : Theme.hover;
        }
    }

    Text {
        anchors.centerIn: parent
        text: control.glyph
        font.family: Theme.family
        font.pixelSize: 14
        color: control.enabled ? Theme.text : Theme.textDisabled
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        enabled: control.enabled
        cursorShape: control.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: control.clicked()
    }

    ToolTip.visible: control.tip !== "" && area.containsMouse
    ToolTip.text: control.tip
    ToolTip.delay: 600
}
