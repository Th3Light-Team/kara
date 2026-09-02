pragma ComponentBehavior: Bound

// Barra de direcciones: migas de pan y, al pedirlo, campo de texto editable.
//
// Referencia: `ground/spec/01-navegacion.md`, «Barra de direcciones tipo
// breadcrumb» y «Editar y escribir ruta (Ctrl+L)».
//
// Aquí no se parte ninguna ruta ni se decide qué miga se oculta: eso lo hace
// `kara-core::breadcrumb` y llega ya resuelto en `crumb_*` y `overflow_*`. Lo
// único que mide esta capa es cuántas migas caben, que es información de layout
// y no la sabe nadie más.
import QtQuick
import QtQuick.Controls
import com.kara.ui

Rectangle {
    id: bar

    required property var app

    // Verdadero mientras se escribe una ruta a mano.
    readonly property bool editing: bar.state === "editing"

    radius: Theme.radius
    color: Theme.field
    border.width: 1
    border.color: {
        if (bar.invalid)
            return Theme.danger;
        return field.activeFocus ? Theme.accent : Theme.divider;
    }

    // Se enciende cuando lo tecleado no lleva a ninguna carpeta legible. La spec
    // exige no borrar lo escrito ni movernos: solo avisar.
    property bool invalid: false

    // Ancho medio de una miga, medido a ojo sobre la tipografía de la barra. Se
    // usa solo para estimar cuántas caben; `collapse` garantiza que la carpeta
    // actual y su padre nunca se esconden, así que quedarse corto degrada bien.
    readonly property int crumbWidthEstimate: 130

    onWidthChanged: bar.app.set_crumb_capacity(Math.max(2, Math.floor(bar.width / bar.crumbWidthEstimate)))

    function startEditing() {
        bar.invalid = false;
        field.text = bar.app.path;
        bar.state = "editing";
        field.forceActiveFocus();
        field.selectAll();
    }

    function stopEditing() {
        bar.invalid = false;
        bar.state = "";
    }

    function commit() {
        if (bar.app.go_to(field.text))
            bar.stopEditing();
        else
            bar.invalid = true;
    }

    states: State {
        name: "editing"
    }

    // ---- Modo migas --------------------------------------------------------
    Row {
        id: crumbs
        visible: !bar.editing
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.leftMargin: 6
        anchors.rightMargin: 6
        anchors.verticalCenter: parent.verticalCenter
        spacing: 0

        // Desbordamiento: los ancestros que no caben viven detrás de este botón.
        Item {
            width: overflowVisible ? 24 : 0
            height: 24
            visible: overflowVisible
            anchors.verticalCenter: parent.verticalCenter

            readonly property bool overflowVisible: bar.app.overflow_names.length > 0

            Rectangle {
                anchors.fill: parent
                radius: Theme.radius
                color: overflowArea.containsMouse ? Theme.hover : "transparent"
            }
            Text {
                anchors.centerIn: parent
                text: "«"
                color: Theme.textDim
                font.family: Theme.family
                font.pixelSize: Theme.sizeBase
            }
            MouseArea {
                id: overflowArea
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: overflowMenu.popup()
            }
            Menu {
                id: overflowMenu
                Repeater {
                    model: bar.app.overflow_names
                    delegate: MenuItem {
                        required property int index
                        required property string modelData
                        text: modelData
                        onTriggered: bar.app.navigate(bar.app.overflow_paths[index])
                    }
                }
            }
        }

        Repeater {
            model: bar.app.crumb_names
            delegate: Row {
                id: crumb
                required property int index
                required property string modelData
                readonly property bool last: crumb.index === bar.app.crumb_names.length - 1
                anchors.verticalCenter: parent.verticalCenter
                spacing: 0

                Rectangle {
                    width: label.implicitWidth + 14
                    height: 24
                    radius: Theme.radius
                    color: crumbArea.containsMouse ? Theme.hover : "transparent"

                    Text {
                        id: label
                        anchors.centerIn: parent
                        text: crumb.modelData
                        // La carpeta actual se lee entera; los ancestros son
                        // referencia y se apagan.
                        color: crumb.last ? Theme.text : Theme.textDim
                        font.family: Theme.family
                        font.pixelSize: Theme.sizeBase
                        font.weight: crumb.last ? Font.DemiBold : Font.Normal
                    }
                    MouseArea {
                        id: crumbArea
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: bar.app.navigate(bar.app.crumb_paths[crumb.index])
                    }
                }

                Text {
                    visible: !crumb.last
                    text: "›"
                    color: Theme.textDim
                    font.pixelSize: Theme.sizeBase
                    anchors.verticalCenter: parent.verticalCenter
                }
            }
        }
    }

    // Clic en el hueco a la derecha de las migas: pasa a editar, como Windows.
    MouseArea {
        visible: !bar.editing
        anchors.left: crumbs.left
        anchors.leftMargin: crumbs.childrenRect.width
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        cursorShape: Qt.IBeamCursor
        onClicked: bar.startEditing()
    }

    // ---- Modo texto --------------------------------------------------------
    TextField {
        id: field
        visible: bar.editing
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        verticalAlignment: TextInput.AlignVCenter
        color: Theme.text
        font.family: Theme.family
        font.pixelSize: Theme.sizeBase
        selectByMouse: true
        background: Item {}
        padding: 0

        onTextEdited: bar.invalid = false
        onAccepted: bar.commit()
        Keys.onEscapePressed: bar.stopEditing()
        // Salir con el ratón cancela igual que Esc: dejar el campo abierto con
        // una ruta a medias haría creer que ya se navegó.
        onActiveFocusChanged: if (!field.activeFocus && bar.editing)
            bar.stopEditing()
    }
}
