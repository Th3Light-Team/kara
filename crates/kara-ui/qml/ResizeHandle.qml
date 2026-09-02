// Franja invisible del borde de la ventana que la redimensiona.
//
// Hace falta porque la ventana es sin marco: al quitar la decoración del
// sistema se va con ella el agarre de los bordes, y hay que devolverlo. El
// redimensionado real lo hace el compositor vía `startSystemResize`, que es lo
// único que funciona bien en Wayland.
import QtQuick

MouseArea {
    id: handle

    required property Window target
    required property int edges

    // `left`, `right`, `top` y `bottom` ya existen en Item —son las lineas de
    // anclaje, y son FINAL—, asi que estos van con otro nombre.
    readonly property bool atLeft: (handle.edges & Qt.LeftEdge) !== 0
    readonly property bool atRight: (handle.edges & Qt.RightEdge) !== 0
    readonly property bool atTop: (handle.edges & Qt.TopEdge) !== 0
    readonly property bool atBottom: (handle.edges & Qt.BottomEdge) !== 0

    cursorShape: {
        if ((handle.atLeft && handle.atTop) || (handle.atRight && handle.atBottom))
            return Qt.SizeFDiagCursor;
        if ((handle.atRight && handle.atTop) || (handle.atLeft && handle.atBottom))
            return Qt.SizeBDiagCursor;
        return (handle.atLeft || handle.atRight) ? Qt.SizeHorCursor : Qt.SizeVerCursor;
    }

    // Maximizada no hay nada que redimensionar, y dejar el cursor de agarre
    // encima invitaría a un gesto que no hace nada.
    enabled: handle.target.visibility !== Window.Maximized

    onPressed: {
        if (typeof handle.target.startSystemResize === "function")
            handle.target.startSystemResize(handle.edges);
    }
}
