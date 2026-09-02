pragma ComponentBehavior: Bound

// Botón del conmutador de modo de vista.
//
// El pictograma se dibuja con rectángulos en vez de escribirse con un glifo:
// los símbolos de rejilla de Unicode no están en todas las tipografías, y uno
// que falta sale como una caja vacía. Ya pasó con el icono de la barra de
// título.
import QtQuick
import QtQuick.Controls
import com.kara.ui

Item {
    id: control

    /// Ordinal de `kara_core::view::ViewMode`.
    required property int mode
    required property int currentMode
    property string tip: ""

    signal clicked

    readonly property bool active: control.mode === control.currentMode
    readonly property color ink: control.active ? Theme.accent : Theme.textDim

    implicitWidth: 32
    implicitHeight: 28

    Rectangle {
        anchors.fill: parent
        radius: Theme.radius
        color: {
            if (control.active)
                return Theme.selection;
            return area.containsMouse ? Theme.hover : "transparent";
        }
    }

    // Detalles: filas de ancho completo.
    Column {
        anchors.centerIn: parent
        visible: control.mode === 0
        spacing: 3
        Repeater {
            model: 3
            Rectangle {
                width: 14
                height: 2
                radius: 1
                color: control.ink
            }
        }
    }

    // Lista: dos columnas de filas cortas.
    Row {
        anchors.centerIn: parent
        visible: control.mode === 1
        spacing: 4
        Repeater {
            model: 2
            Column {
                spacing: 3
                Repeater {
                    model: 3
                    Rectangle {
                        width: 6
                        height: 2
                        radius: 1
                        color: control.ink
                    }
                }
            }
        }
    }

    // Mosaico: cuadro con una línea al lado, dos veces.
    Column {
        anchors.centerIn: parent
        visible: control.mode === 2
        spacing: 4
        Repeater {
            model: 2
            Row {
                spacing: 3
                Rectangle {
                    width: 6
                    height: 6
                    radius: 1
                    color: control.ink
                }
                Rectangle {
                    width: 7
                    height: 2
                    y: 2
                    radius: 1
                    color: control.ink
                }
            }
        }
    }

    // Iconos: rejilla de cuadros.
    Grid {
        anchors.centerIn: parent
        visible: control.mode === 3
        columns: 2
        spacing: 3
        Repeater {
            model: 4
            Rectangle {
                width: 6
                height: 6
                radius: 1
                color: control.ink
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
