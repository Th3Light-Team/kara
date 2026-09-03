pragma ComponentBehavior: Bound

// Marco elástico sobre una vista con desplazamiento.
//
// Referencia: `ground/spec/02-seleccion.md`, «Selección con rubber-band».
//
// Va **por debajo** de los delegados a propósito: la spec avisa de que un marco
// que pueda empezar encima de un elemento se confunde con un arrastrar-mover, y
// la forma de impedirlo es que el gesto solo llegue aquí cuando se pulsa en
// hueco.
//
// El rectángulo se mide en coordenadas del contenido, no de la ventana: con
// autodesplazamiento el contenido se mueve bajo el puntero, y un marco medido
// contra la ventana se quedaría atrás.
import QtQuick
import com.kara.ui

Item {
    id: band

    required property var app
    /// La vista que se desplaza, para leer y mover su contenido.
    required property Flickable scroller

    /// El rectángulo que el usuario ha barrido, en coordenadas del contenido.
    /// Quien reciba esto sabe su geometría y la traduce a posiciones.
    signal swept(rect area, bool additive)

    anchors.fill: parent
    // Por debajo de los delegados.
    z: -1

    property real originX: 0
    property real originY: 0
    property real currentX: 0
    property real currentY: 0
    property bool active: false
    property bool additive: false

    readonly property rect area: Qt.rect(Math.min(band.originX, band.currentX), Math.min(band.originY, band.currentY), Math.abs(band.currentX - band.originX), Math.abs(band.currentY - band.originY))

    function finish() {
        if (!band.active)
            return;
        band.active = false;
        band.app.end_band();
    }

    MouseArea {
        id: area
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton

        onPressed: mouse => {
            band.additive = (mouse.modifiers & Qt.ControlModifier) !== 0;
            band.originX = mouse.x + band.scroller.contentX;
            band.originY = mouse.y + band.scroller.contentY;
            band.currentX = band.originX;
            band.currentY = band.originY;
            band.active = true;
            band.app.begin_band();
            // Pulsar en hueco sin arrastrar deselecciona, que es el gesto
            // universal; el marco vacío hace exactamente eso.
            band.swept(band.area, band.additive);
        }

        onPositionChanged: mouse => {
            if (!band.active)
                return;
            band.currentX = mouse.x + band.scroller.contentX;
            band.currentY = mouse.y + band.scroller.contentY;
            edge.pointer = mouse.y;
            band.swept(band.area, band.additive);
        }

        onReleased: band.finish()
        onCanceled: band.finish()
    }

    // Autodesplazamiento al llegar al borde, que la spec pide explícitamente:
    // sin él no se puede barrer más allá de lo que se ve.
    Timer {
        id: edge
        property real pointer: 0
        readonly property int margin: 24
        readonly property int step: 12

        interval: 16
        repeat: true
        running: band.active

        onTriggered: {
            let delta = 0;
            if (edge.pointer < edge.margin)
                delta = -edge.step;
            else if (edge.pointer > band.height - edge.margin)
                delta = edge.step;
            if (delta === 0)
                return;

            const limite = Math.max(0, band.scroller.contentHeight - band.height);
            const destino = Math.max(0, Math.min(limite, band.scroller.contentY + delta));
            if (destino === band.scroller.contentY)
                return;

            band.scroller.contentY = destino;
            band.currentY = edge.pointer + destino;
            band.swept(band.area, band.additive);
        }
    }

    // Esc cancela sin tocar la selección previa: lo que hubiera antes del marco
    // vuelve tal cual.
    Shortcut {
        sequence: "Escape"
        enabled: band.active
        onActivated: {
            band.active = false;
            band.app.cancel_band();
        }
    }

    Rectangle {
        visible: band.active && (band.area.width > 2 || band.area.height > 2)
        x: band.area.x - band.scroller.contentX
        y: band.area.y - band.scroller.contentY
        width: band.area.width
        height: band.area.height
        color: Qt.alpha(Theme.accent, 0.18)
        border.width: 1
        border.color: Theme.accent
    }
}
