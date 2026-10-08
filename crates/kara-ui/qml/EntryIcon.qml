pragma ComponentBehavior: Bound

// El icono de una entrada, con su miniatura cuando la hay.
import QtQuick

Image {
    id: control

    required property var app
    required property int index
    required property int side

    readonly property string iconUrl: control.app.entry_icons[control.index] ?? ""
    // La miniatura llega después que el listado, desde un hilo de fondo;
    // mientras no esté, manda el icono del tema.
    readonly property string thumbUrl: control.app.entry_thumbs[control.index] ?? ""

    // The theme's file that fails to load (an unreadable or broken SVG) is
    // replaced by the bundled icon: a gap where an icon belongs reads as a
    // file that is not there.
    property bool failed: false
    readonly property string fallbackUrl: (control.app.entry_dirs[control.index] ?? 0) !== 0 ? "qrc:/qt/qml/com/kara/ui/icons/folder.svg" : "qrc:/qt/qml/com/kara/ui/icons/file.svg"
    onIconUrlChanged: control.failed = false
    onStatusChanged: if (status === Image.Error && control.thumbUrl === "")
        control.failed = true

    width: control.side
    height: control.side
    source: control.thumbUrl !== "" ? control.thumbUrl : (control.failed ? control.fallbackUrl : control.iconUrl)
    // Una miniatura no es cuadrada; sin esto se estiraría, que es el modo por
    // defecto de Image.
    fillMode: Image.PreserveAspectFit
    // Los iconos del tema son SVG: sin `sourceSize` se rasterizan a su tamaño
    // nominal y se ven borrosos en cuanto el zoom sube.
    sourceSize.width: control.side
    sourceSize.height: control.side
    // Cargar del disco no puede parar el desplazado de una carpeta con miles
    // de entradas.
    asynchronous: true
    visible: status === Image.Ready
}
