pragma ComponentBehavior: Bound

// Panel de navegación: secciones, ubicaciones y el árbol de carpetas.
//
// Referencia: `ground/spec/01-navegacion.md`, «Panel de navegación en árbol»,
// «Expandir y colapsar nodos del árbol» y «Sincronizar el árbol con la carpeta
// actual».
//
// Es una lista plana, no componentes anidados: el árbol llega ya aplanado desde
// `kara-core::tree` con la sangría en `nav_depths`, y así el motor recicla los
// delegados aunque haya cien ramas abiertas. Una fila sin ruta es la cabecera de
// una sección.
import QtQuick
import QtQuick.Controls
import com.kara.ui

Rectangle {
    id: panel

    required property var app

    color: Theme.sidebar

    ListView {
        id: rows
        anchors.fill: parent
        anchors.topMargin: 6
        anchors.bottomMargin: 6
        clip: true
        model: panel.app.nav_count
        currentIndex: -1
        ScrollBar.vertical: ScrollBar {}

        // Flecha derecha despliega, izquierda pliega. La spec pide además que
        // desplegar no mueva la vista principal, y por eso van a `nav_toggle` y
        // no a `nav_activate`.
        Keys.onRightPressed: if (rows.currentIndex >= 0)
            panel.app.nav_toggle(rows.currentIndex)
        Keys.onLeftPressed: if (rows.currentIndex >= 0)
            panel.app.nav_toggle(rows.currentIndex)
        Keys.onReturnPressed: if (rows.currentIndex >= 0)
            panel.app.nav_activate(rows.currentIndex)

        delegate: Item {
            id: row

            required property int index

            readonly property string label: panel.app.nav_labels[row.index] ?? ""
            readonly property string path: panel.app.nav_paths[row.index] ?? ""
            readonly property int depth: panel.app.nav_depths[row.index] ?? 0
            readonly property bool section: row.path === ""
            readonly property bool expandable: (panel.app.nav_expandable[row.index] ?? 0) !== 0
            readonly property bool expanded: (panel.app.nav_expanded[row.index] ?? 0) !== 0
            readonly property bool current: panel.app.nav_current === row.index

            width: rows.width
            // Las cabeceras respiran por arriba para que la sección se lea como
            // un bloque y no como una fila más de la lista.
            height: row.section ? 34 : 30

            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 6
                anchors.rightMargin: 6
                anchors.topMargin: 1
                anchors.bottomMargin: 1
                radius: Theme.radius
                visible: !row.section
                color: {
                    if (row.current)
                        return Theme.selection;
                    return rowArea.containsMouse ? Theme.hover : "transparent";
                }
            }

            // Barra de acento a la izquierda de la carpeta que se está viendo:
            // es lo que distingue «aquí estás» de «esto está seleccionado».
            Rectangle {
                visible: row.current
                width: 3
                height: 16
                radius: 2
                x: 8
                anchors.verticalCenter: parent.verticalCenter
                color: Theme.accent
            }

            // ---- Cabecera de sección -------------------------------------
            Text {
                visible: row.section
                anchors.left: parent.left
                anchors.leftMargin: 14
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 4
                text: row.label
                color: Theme.headerText
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
                font.weight: Font.DemiBold
            }

            // ---- Carpeta ---------------------------------------------------
            Row {
                visible: !row.section
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.leftMargin: 14 + row.depth * 14
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 4

                Item {
                    width: 16
                    height: 16
                    anchors.verticalCenter: parent.verticalCenter

                    Text {
                        anchors.centerIn: parent
                        visible: row.expandable
                        text: "›"
                        color: chevronArea.containsMouse ? Theme.text : Theme.textDim
                        font.pixelSize: 14
                        rotation: row.expanded ? 90 : 0
                        Behavior on rotation {
                            NumberAnimation {
                                duration: 120
                                easing.type: Easing.OutCubic
                            }
                        }
                    }

                    // Pulsar la flecha despliega sin navegar; pulsar la fila
                    // navega. Son dos gestos distintos y la spec los separa.
                    MouseArea {
                        id: chevronArea
                        anchors.fill: parent
                        anchors.margins: -4
                        hoverEnabled: true
                        enabled: row.expandable
                        onClicked: panel.app.nav_toggle(row.index)
                    }
                }

                Text {
                    width: parent.width - 20
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.label
                    elide: Text.ElideRight
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }
            }

            MouseArea {
                id: rowArea
                anchors.fill: parent
                hoverEnabled: true
                enabled: !row.section
                // La flecha va por encima: su área se declara después y gana.
                z: -1
                onClicked: {
                    rows.currentIndex = row.index;
                    panel.app.nav_activate(row.index);
                }
                onDoubleClicked: if (row.expandable)
                    panel.app.nav_toggle(row.index)
            }
        }
    }
}
