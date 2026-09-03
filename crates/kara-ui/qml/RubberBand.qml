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
// Por debajo, pero **dentro del contenido desplazable**. Ser hijo del propio
// `Flickable` con `z` negativo no basta: el orden de acierto de Qt Quick es
// hijos con z ≥ 0, luego el elemento mismo, y solo después los de z negativo.
// Como un `Flickable` acepta el botón izquierdo, se quedaba la pulsación y aquí
// no llegaba nada. Dentro de `contentItem` el marco queda detrás de los
// delegados y delante del `Flickable`, que es lo que hace falta.
//
// Y estando dentro del contenido, las coordenadas del ratón **ya son** las del
// contenido: no hay que sumarles el desplazamiento.
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

    parent: band.scroller.contentItem
    // Por debajo de los delegados.
    z: -1
    x: 0
    y: 0
    // Cubre el contenido, y al menos lo que se ve: en una carpeta con pocas
    // entradas el hueco de debajo de la última también es sitio desde el que
    // barrer.
    width: Math.max(band.scroller.width, band.scroller.contentWidth)
    height: Math.max(band.scroller.height, band.scroller.contentHeight)

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

    /// Cancela el barrido en curso: la selección vuelve a la de antes de
    /// empezarlo. Devuelve si había algo que cancelar.
    ///
    /// Lo dispara la ventana, no un `Shortcut` de aquí dentro: uno anidado a
    /// esta profundidad no llega a activarse, y además Esc está escalonado —la
    /// ventana tiene que poder preguntar «¿había algo a medias?» antes de
    /// quitar la selección.
    function cancel() {
        if (!band.active)
            return false;
        band.active = false;
        band.app.cancel_band();
        return true;
    }

    MouseArea {
        id: area
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
        // Sin esto el `Flickable` roba el arrastre en cuanto pasa del umbral y
        // el marco muere a mitad de barrido, justo en las carpetas grandes que
        // son las que lo necesitan.
        preventStealing: true

        onPressed: mouse => {
            // Pulsar en hueco también es pulsar en la vista: el foco de teclado
            // viene aquí, como en cualquier explorador.
            area.forceActiveFocus();
            band.additive = (mouse.modifiers & Qt.ControlModifier) !== 0;
            band.originX = mouse.x;
            band.originY = mouse.y;
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
            band.currentX = mouse.x;
            band.currentY = mouse.y;
            // El autodesplazamiento mira dónde está el puntero **en lo que se
            // ve**, no en el contenido.
            edge.pointer = mouse.y - band.scroller.contentY;
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
            else if (edge.pointer > band.scroller.height - edge.margin)
                delta = edge.step;
            if (delta === 0)
                return;

            const limite = Math.max(0, band.scroller.contentHeight - band.scroller.height);
            const destino = Math.max(0, Math.min(limite, band.scroller.contentY + delta));
            if (destino === band.scroller.contentY)
                return;

            band.scroller.contentY = destino;
            band.currentY = edge.pointer + destino;
            band.swept(band.area, band.additive);
        }
    }

    Rectangle {
        visible: band.active && (band.area.width > 2 || band.area.height > 2)
        x: band.area.x
        y: band.area.y
        width: band.area.width
        height: band.area.height
        color: Qt.alpha(Theme.accent, 0.18)
        border.width: 1
        border.color: Theme.accent
    }
}
