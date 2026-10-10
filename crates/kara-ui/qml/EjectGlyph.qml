// The eject symbol, drawn: a triangle over a bar. Neither Adwaita nor Yaru
// has a full-colour `media-eject`, and a font glyph would differ per desktop.
import QtQuick

Item {
    id: glyph

    property color ink: "black"

    implicitWidth: 12
    implicitHeight: 12

    // A square turned 45° is a diamond; showing only its upper half leaves
    // the triangle.
    Item {
        x: 1
        y: 1
        width: 10
        height: 5
        clip: true

        Rectangle {
            width: 7.07
            height: 7.07
            x: (10 - width) / 2
            y: 5 - height / 2
            rotation: 45
            antialiasing: true
            color: glyph.ink
        }
    }
    Rectangle {
        x: 1
        y: 8
        width: 10
        height: 1.5
        antialiasing: true
        color: glyph.ink
    }
}
