pragma ComponentBehavior: Bound

// Botón de la barra de título: minimizar, maximizar, cerrar, tema.
//
// Alto completo de la barra y sin radio, como en Windows 11: son botones que
// tocan el borde de la ventana, no pastillas flotantes.
//
// The glyphs are drawn, not typed. A font character looks different on every
// desktop: Ubuntu has no Segoe Fluent Icons, and fontconfig's fallback for a
// sun or a moon is a colour emoji. Drawn from rectangles, the caption buttons
// are the same on GNOME and on Plasma, and stay sharp at any scale.
import QtQuick
import QtQuick.Controls
import com.kara.ui

Item {
    id: control

    /// `minimize`, `maximize`, `restore`, `close`, `sun` or `moon`.
    required property string kind
    // El de cerrar se pone rojo al pasar por encima; los demás, grises.
    property bool danger: false
    property string tip: ""

    signal clicked

    implicitWidth: 46
    implicitHeight: Theme.titleBarHeight

    readonly property bool hot: area.containsMouse
    readonly property color ink: (control.danger && control.hot) ? Theme.dangerText : Theme.text
    // What is behind the glyph, opaque. The restore and moon glyphs hide part of
    // a shape by painting over it, so they need the real colour, never
    // "transparent".
    readonly property color backdrop: {
        if (!control.hot)
            return Theme.titleBar;
        if (control.danger)
            return Theme.danger;
        return area.pressed ? Theme.pressed : Theme.hover;
    }

    Rectangle {
        anchors.fill: parent
        color: control.hot ? control.backdrop : "transparent"
    }

    // A 10 × 10 box, as in Segoe Fluent Icons. Its position is rounded so
    // one-pixel strokes land on whole pixels instead of smearing across two.
    Item {
        id: glyph

        width: 10
        height: 10
        x: Math.round((control.width - width) / 2)
        y: Math.round((control.height - height) / 2)

        Rectangle {
            visible: control.kind === "minimize"
            y: 5
            width: 10
            height: 1
            color: control.ink
        }

        Rectangle {
            visible: control.kind === "maximize"
            anchors.fill: parent
            color: "transparent"
            radius: 1.5
            border.width: 1
            border.color: control.ink
        }

        // Two windows: the one behind peeks out above and to the right, and
        // the one in front is filled so it covers the other's corner.
        Item {
            visible: control.kind === "restore"
            anchors.fill: parent

            Rectangle {
                x: 2
                y: 0
                width: 8
                height: 8
                color: "transparent"
                radius: 1.5
                border.width: 1
                border.color: control.ink
            }
            Rectangle {
                x: 0
                y: 2
                width: 8
                height: 8
                color: control.backdrop
                radius: 1.5
                border.width: 1
                border.color: control.ink
            }
        }

        Repeater {
            model: control.kind === "close" ? [45, -45] : []
            delegate: Rectangle {
                required property int modelData
                anchors.centerIn: parent
                width: 14
                height: 1
                rotation: modelData
                antialiasing: true
                color: control.ink
            }
        }

        // Sun: a disc and eight rays.
        Item {
            visible: control.kind === "sun"
            anchors.centerIn: parent
            width: 12
            height: 12

            Rectangle {
                anchors.centerIn: parent
                width: 5
                height: 5
                radius: 2.5
                color: "transparent"
                border.width: 1
                border.color: control.ink
            }
            Repeater {
                model: 8
                delegate: Item {
                    required property int index
                    anchors.fill: parent
                    rotation: index * 45
                    antialiasing: true

                    Rectangle {
                        x: (parent.width - width) / 2
                        y: 0
                        width: 1
                        height: 2.5
                        antialiasing: true
                        color: control.ink
                    }
                }
            }
        }

        // Moon: a disc with a second one, the colour of the button, taking a
        // bite out of it.
        Item {
            visible: control.kind === "moon"
            anchors.centerIn: parent
            width: 11
            height: 11
            clip: true

            Rectangle {
                anchors.fill: parent
                radius: width / 2
                color: control.ink
            }
            Rectangle {
                x: 3.5
                y: -2
                width: 10
                height: 10
                radius: 5
                color: control.backdrop
            }
        }
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        onClicked: control.clicked()
    }

    ToolTip.visible: control.tip !== "" && area.containsMouse
    ToolTip.text: control.tip
    ToolTip.delay: 600
}
