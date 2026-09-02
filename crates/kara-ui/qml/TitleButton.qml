// Botón de la barra de título: minimizar, maximizar, cerrar, tema.
//
// Alto completo de la barra y sin radio, como en Windows 11: son botones que
// tocan el borde de la ventana, no pastillas flotantes.
import QtQuick
import com.kara.ui

Item {
    id: control

    required property string glyph
    // El de cerrar se pone rojo al pasar por encima; los demás, grises.
    property bool danger: false
    property string tip: ""

    signal clicked

    implicitWidth: 46
    implicitHeight: Theme.titleBarHeight

    Rectangle {
        anchors.fill: parent
        color: {
            if (!area.containsMouse)
                return "transparent";
            if (control.danger)
                return Theme.danger;
            return area.pressed ? Theme.pressed : Theme.hover;
        }
    }

    Text {
        anchors.centerIn: parent
        text: control.glyph
        font.family: Theme.family
        font.pixelSize: Theme.sizeSmall
        color: (control.danger && area.containsMouse) ? Theme.dangerText : Theme.text
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        onClicked: control.clicked()
    }
}
