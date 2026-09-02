pragma ComponentBehavior: Bound

// Los tres modos que no son una tabla: lista, mosaico e iconos.
//
// Referencia: `ground/spec/03-vistas.md`, «Modos de vista».
//
// Los tres son la misma rejilla con otra celda, así que comparten `GridView` en
// vez de ser tres vistas: cambiar de modo solo cambia el tamaño de celda y qué
// se pinta dentro, y el desplazamiento y la selección siguen siendo los mismos.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.kara.ui

GridView {
    id: grid

    required property var app

    // Los ordinales de `kara_core::view::ViewMode`.
    readonly property int modeList: 1
    readonly property int modeTiles: 2
    readonly property int mode: grid.app.view_mode

    clip: true
    focus: true
    model: grid.app.entry_count
    currentIndex: -1
    // «Lista» apila nombres en columnas verticales, con flujo de arriba abajo y
    // luego a la derecha; los otros dos llenan filas.
    flow: grid.mode === grid.modeList ? GridView.FlowTopToBottom : GridView.FlowLeftToRight

    cellWidth: {
        if (grid.mode === grid.modeList)
            return 260;
        if (grid.mode === grid.modeTiles)
            return 320;
        // Los iconos necesitan sitio para el nombre debajo, que casi siempre es
        // más ancho que el icono.
        return Math.max(grid.app.icon_size + 32, 108);
    }
    cellHeight: {
        if (grid.mode === grid.modeList)
            return Math.max(28, grid.app.icon_size + 8);
        if (grid.mode === grid.modeTiles)
            return grid.app.icon_size + 24;
        // Icono, hueco y dos líneas de nombre.
        return grid.app.icon_size + 12 + 2 * (Theme.sizeBase + 4) + 12;
    }

    ScrollBar.vertical: ScrollBar {
        policy: grid.flow === GridView.FlowLeftToRight ? ScrollBar.AsNeeded : ScrollBar.AlwaysOff
    }
    ScrollBar.horizontal: ScrollBar {
        policy: grid.flow === GridView.FlowTopToBottom ? ScrollBar.AsNeeded : ScrollBar.AlwaysOff
    }

    delegate: Item {
        id: cell

        required property int index
        readonly property bool selected: (grid.app.entry_selected[cell.index] ?? 0) !== 0
        readonly property bool current: grid.app.focused_index === cell.index
        readonly property string entryName: grid.app.entry_names[cell.index] ?? ""

        width: grid.cellWidth
        height: grid.cellHeight

        Rectangle {
            anchors.fill: parent
            anchors.margins: 3
            radius: Theme.radius
            color: {
                if (cell.selected)
                    return Theme.selection;
                return mouse.containsMouse ? Theme.hover : "transparent";
            }
            border.width: cell.current && !cell.selected ? 1 : 0
            border.color: Theme.accent
        }

        Loader {
            anchors.fill: parent
            anchors.margins: 3
            sourceComponent: {
                if (grid.mode === grid.modeList)
                    return listCell;
                if (grid.mode === grid.modeTiles)
                    return tileCell;
                return iconCell;
            }
        }

        // ---- Iconos: miniatura arriba, nombre debajo ----------------------
        Component {
            id: iconCell

            ColumnLayout {
                spacing: 6

                EntryIcon {
                    app: grid.app
                    index: cell.index
                    side: grid.app.icon_size
                    Layout.alignment: Qt.AlignHCenter
                    Layout.topMargin: 8
                    Layout.preferredWidth: grid.app.icon_size
                    Layout.preferredHeight: grid.app.icon_size
                }
                Text {
                    text: cell.entryName
                    Layout.fillWidth: true
                    Layout.leftMargin: 4
                    Layout.rightMargin: 4
                    horizontalAlignment: Text.AlignHCenter
                    // Dos líneas y a partir de ahí puntos suspensivos: un
                    // nombre largo no puede empujar la celda de al lado.
                    wrapMode: Text.Wrap
                    maximumLineCount: 2
                    elide: Text.ElideRight
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }
                Item {
                    Layout.fillHeight: true
                }
            }
        }

        // ---- Mosaico: icono a la izquierda, datos al lado -----------------
        Component {
            id: tileCell

            RowLayout {
                spacing: 10

                EntryIcon {
                    app: grid.app
                    index: cell.index
                    side: grid.app.icon_size
                    Layout.leftMargin: 8
                    Layout.alignment: Qt.AlignVCenter
                    Layout.preferredWidth: grid.app.icon_size
                    Layout.preferredHeight: grid.app.icon_size
                }
                ColumnLayout {
                    Layout.rightMargin: 8
                    Layout.alignment: Qt.AlignVCenter
                    spacing: 1

                    Text {
                        text: cell.entryName
                        Layout.fillWidth: true
                        elide: Text.ElideMiddle
                        color: Theme.text
                        font.family: Theme.family
                        font.pixelSize: Theme.sizeBase
                    }
                    Text {
                        text: grid.app.entry_kinds[cell.index] ?? ""
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                        color: Theme.textDim
                        font.family: Theme.family
                        font.pixelSize: Theme.sizeSmall
                    }
                    Text {
                        text: grid.app.entry_sizes[cell.index] ?? ""
                        visible: text !== ""
                        color: Theme.textDim
                        font.family: Theme.family
                        font.pixelSize: Theme.sizeSmall
                    }
                }
            }
        }

        // ---- Lista: solo el nombre ----------------------------------------
        Component {
            id: listCell

            RowLayout {
                spacing: 8

                EntryIcon {
                    app: grid.app
                    index: cell.index
                    side: grid.app.icon_size
                    Layout.leftMargin: 8
                    Layout.preferredWidth: grid.app.icon_size
                    Layout.preferredHeight: grid.app.icon_size
                }
                Text {
                    text: cell.entryName
                    Layout.fillWidth: true
                    Layout.rightMargin: 8
                    elide: Text.ElideMiddle
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }
            }
        }

        EntryMouse {
            id: mouse
            app: grid.app
            index: cell.index
        }
    }
}
