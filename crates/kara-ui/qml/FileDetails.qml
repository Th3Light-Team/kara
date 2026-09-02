pragma ComponentBehavior: Bound

// Vista de detalles: una fila por entrada, con columnas.
//
// Referencia: `ground/spec/03-vistas.md`, «Modos de vista». Es el modo por
// defecto porque es el que aguanta una carpeta con diez mil ficheros.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.kara.ui

Item {
    id: view

    required property var app

    // Anchos compartidos por la cabecera y las filas. Si se separan, las
    // columnas dejan de alinearse en cuanto se toca una.
    /// Fila cuyo nombre se está editando, o -1 si ninguna.
    property int renamingIndex: -1

    /// Abre el editor sobre la fila que tenga el cursor.
    function startRename() {
        if (view.app.focused_index >= 0)
            view.renamingIndex = view.app.focused_index;
    }

    readonly property int dateWidth: 150
    readonly property int typeWidth: 180
    readonly property int sizeWidth: 100
    readonly property int nameMinimum: 320
    readonly property int nameMaximum: 560

    Rectangle {
        id: columnHeader
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: 30
        color: Theme.content

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 14
            anchors.rightMargin: 22
            spacing: 12

            // Hueco del icono: la cabecera tiene que llevar el mismo que las
            // filas o las columnas dejan de alinearse.
            Item {
                Layout.preferredWidth: view.app.icon_size
            }
            ColumnHeader {
                app: view.app
                column: "name"
                label: qsTr("Nombre")
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredWidth: view.nameMinimum
                Layout.maximumWidth: view.nameMaximum
            }
            ColumnHeader {
                app: view.app
                column: "modified"
                label: qsTr("Fecha de modificación")
                Layout.preferredWidth: view.dateWidth
                Layout.fillHeight: true
            }
            ColumnHeader {
                app: view.app
                column: "kind"
                label: qsTr("Tipo")
                Layout.preferredWidth: view.typeWidth
                Layout.fillHeight: true
            }
            ColumnHeader {
                app: view.app
                column: "size"
                label: qsTr("Tamaño")
                alignment: Text.AlignRight
                Layout.preferredWidth: view.sizeWidth
                Layout.fillHeight: true
            }
            // Lo que sobra a la derecha se deja en blanco: estirar las columnas
            // hasta el borde en una pantalla ancha separa el nombre de su
            // tamaño hasta hacerlos ilegibles juntos.
            Item {
                Layout.fillWidth: true
                Layout.preferredWidth: 0
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

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 14
                anchors.rightMargin: 22
                spacing: 12

                EntryIcon {
                    app: view.app
                    index: row.index
                    side: view.app.icon_size
                    Layout.preferredWidth: view.app.icon_size
                    Layout.preferredHeight: view.app.icon_size
                }
                Item {
                    Layout.fillWidth: true
                    Layout.leftMargin: 6
                    Layout.preferredWidth: view.nameMinimum
                    Layout.maximumWidth: view.nameMaximum
                    Layout.fillHeight: true

                    readonly property bool renaming: view.renamingIndex === row.index
                    readonly property string entryName: view.app.entry_names[row.index] ?? ""

                    Text {
                        anchors.fill: parent
                        visible: !parent.renaming
                        verticalAlignment: Text.AlignVCenter
                        text: parent.entryName
                        elide: Text.ElideMiddle
                        color: Theme.text
                        font.family: Theme.family
                        font.pixelSize: Theme.sizeBase
                    }

                    Loader {
                        anchors.fill: parent
                        anchors.topMargin: 2
                        anchors.bottomMargin: 2
                        // El editor se crea al empezar a renombrar y se
                        // destruye al terminar: mantener un TextField por fila
                        // en una carpeta de miles sería absurdo.
                        active: parent.renaming
                        sourceComponent: nameEditor
                    }

                    Component {
                        id: nameEditor

                        TextField {
                            text: view.app.entry_names[row.index] ?? ""
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
                                // Solo el nombre base queda seleccionado: la
                                // spec pide dejar fuera la extensión para no
                                // borrarla sin querer. Dónde acaba lo dice
                                // `kara-core`, que sabe que `.tar.gz` es una.
                                select(0, view.app.base_name_length(text));
                            }

                            onAccepted: {
                                view.app.rename_entry(view.app.entry_names[row.index], text);
                                view.renamingIndex = -1;
                            }
                            Keys.onEscapePressed: view.renamingIndex = -1
                            onActiveFocusChanged: if (!activeFocus)
                                view.renamingIndex = -1
                        }
                    }
                }
                Text {
                    text: view.app.entry_dates[row.index] ?? ""
                    Layout.preferredWidth: view.dateWidth
                    Layout.leftMargin: 6
                    elide: Text.ElideRight
                    color: Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }
                Text {
                    text: view.app.entry_kinds[row.index] ?? ""
                    Layout.preferredWidth: view.typeWidth
                    Layout.leftMargin: 6
                    elide: Text.ElideRight
                    color: Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }
                Text {
                    text: view.app.entry_sizes[row.index] ?? ""
                    Layout.preferredWidth: view.sizeWidth
                    Layout.rightMargin: 6
                    horizontalAlignment: Text.AlignRight
                    color: Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }
                Item {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 0
                }
            }

            EntryMouse {
                id: mouse
                app: view.app
                index: row.index
                onRenameRequested: view.renamingIndex = row.index
            }
        }
    }
}
