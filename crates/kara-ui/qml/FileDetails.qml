pragma ComponentBehavior: Bound

// Vista de detalles: una fila por entrada, con columnas configurables.
//
// Referencia: `ground/spec/03-vistas.md`, «Modos de vista», «Columnas
// configurables en Detalles», «Autoajustar ancho de columnas» y «Ordenar con
// clic en cabecera de columna».
//
// Ni la cabecera ni las filas saben qué columnas hay: las dos recorren
// `column_ids`, y el contenido sale de `entry_values`, que llega como una tabla
// por filas. Cablear aquí «nombre, fecha, tipo, tamaño» sería justo lo que
// «configurables» impide.
import QtQuick
import QtQuick.Controls
import com.kara.ui

Item {
    id: view

    required property var app

    /// Fila cuyo nombre se está editando, o -1 si ninguna.
    property int renamingIndex: -1

    /// Abre el editor sobre la fila que tenga el cursor.
    function startRename() {
        if (view.app.focused_index >= 0)
            view.renamingIndex = view.app.focused_index;
    }

    /// Cancela lo que haya a medias y dice si había algo. Es lo que la ventana
    /// pregunta antes de dejar que Esc quite la selección.
    function cancelPending() {
        if (band.cancel())
            return true;
        if (view.renamingIndex >= 0) {
            view.renamingIndex = -1;
            return true;
        }
        return false;
    }

    /// Los números se leen mejor alineados a la derecha; el resto, a la
    /// izquierda. Es la única regla de presentación que depende de la columna.
    function alignsRight(id) {
        return id === "size" || id === "rating" || id === "duration";
    }

    readonly property int leftMargin: 14
    readonly property int gap: 12

    /// Hasta dónde llega el contenido de una fila. A la derecha de eso hay
    /// hueco, y el hueco es donde puede empezar el marco elástico: la spec
    /// avisa de que un marco que arranque sobre un elemento se confunde con un
    /// arrastrar-mover.
    readonly property int rowContentWidth: {
        let ancho = view.leftMargin + view.app.icon_size;
        for (let i = 0; i < view.app.column_count; ++i)
            ancho += view.gap + (view.app.column_widths[i] ?? 0);
        return ancho;
    }

    Rectangle {
        id: columnHeader
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: 30
        color: Theme.content

        Row {
            anchors.fill: parent
            anchors.leftMargin: view.leftMargin
            spacing: view.gap

            // Hueco del icono: las filas lo llevan delante del nombre y sin él
            // las columnas dejan de alinearse.
            Item {
                width: view.app.icon_size
                height: 1
            }

            Repeater {
                model: view.app.column_count

                delegate: Item {
                    id: head

                    required property int index
                    readonly property string columnId: view.app.column_ids[head.index] ?? ""

                    width: view.app.column_widths[head.index] ?? 100
                    height: columnHeader.height

                    ColumnHeader {
                        anchors.fill: parent
                        app: view.app
                        column: head.columnId
                        label: view.app.column_labels[head.index] ?? ""
                        alignment: view.alignsRight(head.columnId) ? Text.AlignRight : Text.AlignLeft
                    }

                    // Separador arrastrable. Es la forma que pide la spec de
                    // cambiar el ancho, y va por encima de la cabecera para que
                    // arrastrar no cuente además como un clic de ordenación.
                    Item {
                        width: 7
                        height: parent.height
                        anchors.right: parent.right
                        anchors.rightMargin: -view.gap / 2

                        Rectangle {
                            anchors.centerIn: parent
                            width: 1
                            height: parent.height - 12
                            color: grip.pressed ? Theme.accent : Theme.divider
                        }

                        MouseArea {
                            id: grip
                            anchors.fill: parent
                            cursorShape: Qt.SplitHCursor
                            property real grabbedAt: 0

                            onPressed: mouse => {
                                grip.grabbedAt = mouse.x;
                            }
                            onPositionChanged: mouse => {
                                if (!grip.pressed)
                                    return;
                                const propuesto = head.width + mouse.x - grip.grabbedAt;
                                // El mínimo lo impone `kara-core`; aquí solo se
                                // evita mandar un ancho negativo.
                                view.app.set_column_width(head.columnId, Math.max(0, propuesto));
                            }
                            onDoubleClicked: view.app.autofit_columns()
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        acceptedButtons: Qt.RightButton
                        onClicked: headerMenu.popup()
                    }

                    Menu {
                        id: headerMenu

                        MenuItem {
                            text: qsTr("Mover a la izquierda")
                            enabled: head.index > 0
                            onTriggered: view.app.move_column(head.index, head.index - 1)
                        }
                        MenuItem {
                            text: qsTr("Mover a la derecha")
                            enabled: head.index + 1 < view.app.column_count
                            onTriggered: view.app.move_column(head.index, head.index + 1)
                        }
                        MenuSeparator {}
                        MenuItem {
                            text: qsTr("Quitar esta columna")
                            // La del nombre no se puede quitar: una tabla de
                            // ficheros sin nombre no es nada.
                            enabled: head.columnId !== "name"
                            onTriggered: view.app.toggle_column(head.columnId)
                        }
                        Menu {
                            title: qsTr("Añadir columna")
                            enabled: view.app.addable_ids.length > 0

                            Repeater {
                                model: view.app.addable_ids
                                delegate: MenuItem {
                                    required property int index
                                    text: view.app.addable_labels[index] ?? ""
                                    onTriggered: view.app.toggle_column(view.app.addable_ids[index])
                                }
                            }
                        }
                        MenuSeparator {}
                        MenuItem {
                            text: qsTr("Ajustar todas las columnas")
                            onTriggered: view.app.autofit_columns()
                        }
                    }
                }
            }
        }

        Rectangle {
            anchors.bottom: parent.bottom
            width: parent.width
            height: 1
            color: Theme.divider
        }
    }

    ListView {
        id: rows
        anchors.top: columnHeader.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        clip: true
        focus: true
        model: view.app.entry_count
        // El cursor lo lleva el modelo, no la vista: `focused_index` es la
        // única fuente, para que la selección y el cursor no se contradigan.
        currentIndex: -1
        highlightMoveDuration: 0
        ScrollBar.vertical: ScrollBar {}

        RubberBand {
            id: band
            app: view.app
            scroller: rows
            onSwept: (area, additive) => {
                // En una lista lo que un rectángulo toca es siempre un tramo
                // seguido de filas, así que basta con los extremos.
                const alto = Math.max(1, rows.contentHeight / Math.max(1, view.app.entry_count));
                const primera = Math.floor(area.y / alto);
                const ultima = Math.floor((area.y + area.height) / alto);
                if (ultima < 0 || primera >= view.app.entry_count) {
                    view.app.band_set([], additive);
                    return;
                }
                view.app.rubber_band(Math.max(0, primera), Math.min(view.app.entry_count - 1, ultima), additive);
            }
        }

        delegate: Item {
            id: row

            required property int index
            // Seleccionada y con el cursor son dos cosas distintas: Esc quita
            // la selección y conserva el cursor, y Mayúsculas+clic extiende
            // desde donde está el cursor aunque no esté seleccionado.
            readonly property bool selected: (view.app.entry_selected[row.index] ?? 0) !== 0
            readonly property bool current: view.app.focused_index === row.index

            width: rows.width
            height: Math.max(Theme.rowHeight, view.app.icon_size + 8)

            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 4
                anchors.rightMargin: 4
                radius: Theme.radius
                color: {
                    if (row.selected)
                        return Theme.selection;
                    return mouse.containsMouse ? Theme.hover : "transparent";
                }
            }

            Rectangle {
                visible: row.current && !row.selected
                anchors.fill: parent
                anchors.leftMargin: 4
                anchors.rightMargin: 4
                radius: Theme.radius
                color: "transparent"
                border.width: 1
                border.color: Theme.accent
            }

            Row {
                anchors.fill: parent
                anchors.leftMargin: view.leftMargin
                spacing: view.gap

                EntryIcon {
                    app: view.app
                    index: row.index
                    side: view.app.icon_size
                    anchors.verticalCenter: parent.verticalCenter
                }

                Repeater {
                    model: view.app.column_count

                    delegate: Item {
                        id: cell

                        required property int index
                        readonly property string columnId: view.app.column_ids[cell.index] ?? ""
                        readonly property bool isName: cell.columnId === "name"
                        readonly property string text: view.app.entry_values[row.index * view.app.column_count + cell.index] ?? ""

                        width: view.app.column_widths[cell.index] ?? 100
                        height: row.height

                        Text {
                            anchors.fill: parent
                            visible: !(cell.isName && view.renamingIndex === row.index)
                            verticalAlignment: Text.AlignVCenter
                            horizontalAlignment: view.alignsRight(cell.columnId) ? Text.AlignRight : Text.AlignLeft
                            text: cell.text
                            // El nombre se recorta por el medio, donde menos
                            // información se pierde; el resto por el final.
                            elide: cell.isName ? Text.ElideMiddle : Text.ElideRight
                            color: cell.isName ? Theme.text : Theme.textDim
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeBase
                        }

                        Loader {
                            anchors.fill: parent
                            anchors.topMargin: 2
                            anchors.bottomMargin: 2
                            // El editor se crea al empezar a renombrar y se
                            // destruye al terminar: mantener un TextField por
                            // fila en una carpeta de miles sería absurdo.
                            active: cell.isName && view.renamingIndex === row.index
                            sourceComponent: nameEditor
                        }

                        Component {
                            id: nameEditor

                            TextField {
                                text: cell.text
                                color: Theme.text
                                font.family: Theme.family
                                font.pixelSize: Theme.sizeBase
                                selectByMouse: true
                                padding: 2

                                background: Rectangle {
                                    radius: Theme.radius
                                    color: Theme.field
                                    border.width: 1
                                    border.color: Theme.accent
                                }

                                Component.onCompleted: {
                                    forceActiveFocus();
                                    // Solo el nombre base queda seleccionado:
                                    // la spec pide dejar fuera la extensión
                                    // para no borrarla sin querer. Dónde acaba
                                    // lo dice `kara-core`, que sabe que
                                    // `.tar.gz` es una sola.
                                    select(0, view.app.base_name_length(text));
                                }

                                onAccepted: {
                                    view.app.rename_entry(cell.text, text);
                                    view.renamingIndex = -1;
                                }
                                Keys.onEscapePressed: view.renamingIndex = -1
                                onActiveFocusChanged: if (!activeFocus)
                                    view.renamingIndex = -1
                            }
                        }
                    }
                }
            }

            EntryMouse {
                id: mouse
                app: view.app
                index: row.index
                // Solo sobre el contenido: a la derecha empieza el hueco.
                anchors.fill: undefined
                anchors.left: parent.left
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: Math.min(row.width, view.rowContentWidth)
                onRenameRequested: view.renamingIndex = row.index
            }
        }
    }
}
