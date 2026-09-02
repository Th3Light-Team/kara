// Árbol de carpetas recursivo, 100% QML declarativo.
// Recursión mediante Loader (carga por URL) para evitar el ciclo estático
// de tipos que QML prohíbe. Muestra expandir/colapsar animado, chevron que
// rota, y estados hover/selección.
import QtQuick
import QtQuick.Controls.Basic

Column {
    id: root

    property var node                 // { name, type, children, ... }
    property var theme
    property int depth: 0
    property var currentNode          // nodo seleccionado en toda la app
    signal activated(var node)

    // Alias de color a prueba de `theme` indefinido (los hijos por Loader
    // pueden evaluar antes de recibir el theme). Se re-evalúan al llegar.
    readonly property color cText:   theme ? theme.text      : "#1A1A1A"
    readonly property color cDim:    theme ? theme.textDim   : "#808080"
    readonly property color cHover:  theme ? theme.hover     : "transparent"
    readonly property color cSel:    theme ? theme.selection : "transparent"
    readonly property color cAccent: theme ? theme.accent    : "#005FB8"
    readonly property string cFamily: theme ? theme.family   : "Segoe UI"

    // Solo carpetas en el panel de navegación (como Windows).
    readonly property var childFolders: {
        if (!node || !node.children) return [];
        return node.children.filter(function (c) { return c.type === 'folder'; });
    }
    property bool expanded: depth === 0

    width: parent ? parent.width : 240
    spacing: 0

    // --- Fila de este nodo -------------------------------------------------
    Rectangle {
        id: rowRect
        width: root.width
        height: 32
        radius: 5
        color: rowMouse.containsMouse ? root.cHover
             : (root.currentNode === root.node ? root.cSel : "transparent")

        // Barra de acento a la izquierda cuando está seleccionado (Win11)
        Rectangle {
            width: 3; radius: 2
            height: parent.height - 12
            anchors.verticalCenter: parent.verticalCenter
            x: 2
            color: root.cAccent
            visible: root.currentNode === root.node
        }

        Row {
            anchors.fill: parent
            anchors.leftMargin: 8 + root.depth * 16
            anchors.rightMargin: 6
            spacing: 4

            // Chevron (solo si hay subcarpetas)
            Item {
                width: 18; height: parent.height
                visible: root.childFolders.length > 0
                Text {
                    anchors.centerIn: parent
                    text: "❯"
                    font.pixelSize: 11
                    font.family: root.cFamily
                    color: root.cDim
                    rotation: root.expanded ? 90 : 0
                    Behavior on rotation { NumberAnimation { duration: 130; easing.type: Easing.OutCubic } }
                }
                MouseArea {
                    anchors.fill: parent
                    onClicked: root.expanded = !root.expanded
                }
            }
            Item {
                width: 18; height: parent.height
                visible: root.childFolders.length === 0
            }

            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: root.expanded && root.childFolders.length > 0 ? "📂" : "📁"
                font.pixelSize: 15
            }

            Text {
                anchors.verticalCenter: parent.verticalCenter
                width: root.width - (8 + root.depth * 16) - 52
                text: root.node ? root.node.name : ""
                color: root.cText
                font.family: root.cFamily
                font.pixelSize: 13
                elide: Text.ElideRight
            }
        }

        MouseArea {
            id: rowMouse
            anchors.fill: parent
            hoverEnabled: true
            onClicked: root.activated(root.node)
            onDoubleClicked: if (root.childFolders.length > 0) root.expanded = !root.expanded
        }
    }

    // --- Hijos (contenedor con altura animada) -----------------------------
    Item {
        width: root.width
        clip: true
        height: root.expanded ? childCol.implicitHeight : 0
        opacity: root.expanded ? 1 : 0
        Behavior on height { NumberAnimation { duration: 160; easing.type: Easing.OutCubic } }
        Behavior on opacity { NumberAnimation { duration: 160 } }

        Column {
            id: childCol
            width: parent.width
            spacing: 0
            Repeater {
                model: root.childFolders
                // Recursión vía Loader: carga dinámica por URL para romper
                // el ciclo estático de tipos.
                delegate: Loader {
                    required property var modelData
                    width: childCol.width
                    height: item ? item.implicitHeight : 0
                    source: "FolderTree.qml"
                    onLoaded: {
                        item.node = modelData;
                        item.depth = root.depth + 1;
                        item.theme = Qt.binding(function () { return root.theme; });
                        item.currentNode = Qt.binding(function () { return root.currentNode; });
                        item.activated.connect(function (n) { root.activated(n); });
                    }
                }
            }
        }
    }
}
