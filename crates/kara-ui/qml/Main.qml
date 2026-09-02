pragma ComponentBehavior: Bound

// Ventana principal de Kara.
//
// La ventana es **sin marco**: la barra de título es nuestra, como en el
// Explorador de Windows 11. Lo que se pierde al quitar la decoración del sistema
// —mover, redimensionar, maximizar con doble clic— se devuelve aquí, delegando
// en el compositor (`startSystemMove` / `startSystemResize`) para no pelearse
// con Wayland.
//
// Aquí no se decide nada del dominio: cada acción llama a un invocable de `App`
// y cada dato sale de una propiedad suya.
import QtQuick
import QtQuick.Window
import QtQuick.Layouts
import QtQuick.Controls
import com.kara.ui

Window {
    id: win

    width: 1150
    height: 720
    minimumWidth: 560
    minimumHeight: 360
    visible: true
    title: qsTr("Kara")
    flags: Qt.Window | Qt.FramelessWindowHint
    color: Theme.windowBg

    App {
        id: app
    }

    // ---- Atajos (ground/spec/07-atajos-teclado.md) --------------------------
    Shortcut {
        sequences: ["Ctrl+L", "Alt+D", "F4"]
        onActivated: address.startEditing()
    }
    Shortcut {
        sequence: "Alt+Left"
        onActivated: app.back()
    }
    Shortcut {
        sequence: "Alt+Right"
        onActivated: app.forward()
    }
    Shortcut {
        sequence: "Alt+Up"
        onActivated: app.up()
    }
    Shortcut {
        sequences: ["F5", "Ctrl+R"]
        onActivated: app.reload()
    }
    Shortcut {
        sequence: "Ctrl+F"
        onActivated: filterField.forceActiveFocus()
    }
    Shortcut {
        sequence: "F9"
        onActivated: app.toggle_sidebar()
    }

    // Ancho del panel de navegación. La spec lo quiere persistente entre
    // sesiones; hoy no hay dónde guardarlo, así que vuelve a su sitio al
    // arrancar.
    property int sidebarWidth: 240
    readonly property int sidebarMin: 160
    readonly property int sidebarMax: 480

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // ---- Barra de título -----------------------------------------------
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.titleBarHeight
            color: Theme.titleBar

            MouseArea {
                anchors.fill: parent
                onPressed: {
                    if (typeof win.startSystemMove === "function")
                        win.startSystemMove();
                }
                onDoubleClicked: win.toggleMaximized()
            }

            RowLayout {
                anchors.fill: parent
                spacing: 0

                Text {
                    Layout.leftMargin: 14
                    text: qsTr("Kara")
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeTitle
                    font.weight: Font.DemiBold
                }
                Item {
                    Layout.fillWidth: true
                }

                TitleButton {
                    glyph: Theme.dark ? "☀" : "☾"
                    tip: qsTr("Cambiar tema")
                    onClicked: Theme.toggle()
                }
                TitleButton {
                    glyph: "–"
                    tip: qsTr("Minimizar")
                    onClicked: win.showMinimized()
                }
                TitleButton {
                    glyph: win.visibility === Window.Maximized ? "❐" : "□"
                    tip: qsTr("Maximizar")
                    onClicked: win.toggleMaximized()
                }
                TitleButton {
                    glyph: "✕"
                    danger: true
                    tip: qsTr("Cerrar")
                    onClicked: win.close()
                }
            }
        }

        // ---- Barra de comandos ---------------------------------------------
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.commandBarHeight
            color: Theme.toolbar

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 10
                spacing: 2

                NavButton {
                    glyph: "☰"
                    tip: qsTr("Panel de navegación (F9)")
                    onClicked: app.toggle_sidebar()
                }
                NavButton {
                    glyph: "←"
                    tip: qsTr("Atrás (Alt+←)")
                    enabled: app.can_go_back
                    onClicked: app.back()
                }
                NavButton {
                    glyph: "→"
                    tip: qsTr("Adelante (Alt+→)")
                    enabled: app.can_go_forward
                    onClicked: app.forward()
                }
                NavButton {
                    glyph: "↑"
                    tip: qsTr("Subir (Alt+↑)")
                    enabled: app.path !== "/"
                    onClicked: app.up()
                }
                NavButton {
                    glyph: "↻"
                    tip: qsTr("Actualizar (F5)")
                    onClicked: app.reload()
                }

                AddressBar {
                    id: address
                    app: app
                    Layout.fillWidth: true
                    Layout.leftMargin: 6
                    Layout.preferredHeight: Theme.fieldHeight
                }

                // ---- Filtro por nombre --------------------------------------
                // Es filtro, no búsqueda: solo reduce lo ya listado en esta
                // carpeta. La búsqueda recursiva es otra conveniencia.
                Rectangle {
                    Layout.preferredWidth: 220
                    Layout.preferredHeight: Theme.fieldHeight
                    Layout.leftMargin: 6
                    radius: Theme.radius
                    color: Theme.field
                    border.width: 1
                    border.color: filterField.activeFocus ? Theme.accent : Theme.divider

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 9
                        anchors.rightMargin: 6
                        spacing: 6

                        Text {
                            text: "\u{1F50D}"
                            font.pixelSize: 12
                            color: Theme.textDim
                        }
                        TextField {
                            id: filterField
                            Layout.fillWidth: true
                            placeholderText: qsTr("Filtrar esta carpeta")
                            color: Theme.text
                            placeholderTextColor: Theme.textDim
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeBase
                            selectByMouse: true
                            background: Item {}
                            padding: 0

                            // Cada pulsación relistaría la carpeta; con 100 000
                            // entradas eso es un `scandir` por tecla. El retardo
                            // es el que la spec pide para la búsqueda incremental.
                            onTextChanged: filterDelay.restart()
                            onAccepted: {
                                filterDelay.stop();
                                app.apply_filter(filterField.text);
                            }
                            Keys.onEscapePressed: {
                                filterField.text = "";
                                filterDelay.stop();
                                app.apply_filter("");
                                fileList.forceActiveFocus();
                            }
                        }
                        Timer {
                            id: filterDelay
                            interval: 150
                            onTriggered: app.apply_filter(filterField.text)
                        }
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: Theme.divider
        }

        // ---- Cuerpo: panel de navegación + contenido -------------------------
        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            Sidebar {
                app: app
                visible: app.sidebar_visible
                Layout.preferredWidth: win.sidebarWidth
                Layout.fillHeight: true
            }

            // Separador arrastrable. Es ancho para poder agarrarlo y pinta una
            // línea de un píxel: un divisor de un píxel es imposible de coger.
            Item {
                visible: app.sidebar_visible
                Layout.preferredWidth: 5
                Layout.fillHeight: true

                Rectangle {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 1
                    height: parent.height
                    color: Theme.divider
                }

                MouseArea {
                    id: splitter
                    anchors.fill: parent
                    cursorShape: Qt.SplitHCursor
                    property real grabbedAt: 0

                    onPressed: mouse => {
                        splitter.grabbedAt = mouse.x;
                    }
                    onPositionChanged: mouse => {
                        if (!splitter.pressed)
                            return;
                        const propuesto = win.sidebarWidth + mouse.x - splitter.grabbedAt;
                        win.sidebarWidth = Math.max(win.sidebarMin, Math.min(win.sidebarMax, propuesto));
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                color: Theme.content

                // Cabecera de columnas. Todavía es un rótulo: ordenar pulsando aquí
                // llega con la vista de detalles completa.
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

                        // Hueco del icono: la cabecera tiene que llevar el mismo
                        // que las filas o las columnas dejan de alinearse.
                        Item {
                            Layout.preferredWidth: 16
                        }
                        Text {
                            text: qsTr("Nombre")
                            Layout.fillWidth: true
                            Layout.preferredWidth: 320
                            Layout.maximumWidth: 560
                            color: Theme.headerText
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                        Text {
                            text: qsTr("Tipo")
                            Layout.preferredWidth: 140
                            color: Theme.headerText
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                        Text {
                            text: qsTr("Tamaño")
                            Layout.preferredWidth: 110
                            horizontalAlignment: Text.AlignRight
                            color: Theme.headerText
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                        // Lo que sobra a la derecha se deja en blanco: estirar las
                        // columnas hasta el borde en una pantalla ancha separa el
                        // nombre de su tamaño hasta hacerlos ilegibles juntos.
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
                    id: fileList
                    anchors.top: columnHeader.bottom
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    clip: true
                    focus: true
                    model: app.entry_count
                    currentIndex: -1
                    highlightMoveDuration: 0
                    ScrollBar.vertical: ScrollBar {}

                    delegate: Item {
                        id: row

                        required property int index
                        readonly property bool current: fileList.currentIndex === row.index
                        // La URL se declara aparte y tipada: asignar directamente
                        // el resultado de `??` a `source` deja un valor sin tipo
                        // que QML no sabe convertir a URL.
                        readonly property string iconUrl: app.entry_icons[row.index] ?? ""

                        width: fileList.width
                        height: Theme.rowHeight

                        Rectangle {
                            anchors.fill: parent
                            anchors.leftMargin: 4
                            anchors.rightMargin: 4
                            radius: Theme.radius
                            color: {
                                if (row.current)
                                    return Theme.selection;
                                return rowArea.containsMouse ? Theme.hover : "transparent";
                            }
                        }

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 14
                            anchors.rightMargin: 22
                            spacing: 12

                            Image {
                                Layout.preferredWidth: 16
                                Layout.preferredHeight: 16
                                source: row.iconUrl
                                // El tema resuelve la talla, pero los ficheros
                                // son SVG: sin `sourceSize` se rasterizan a su
                                // tamaño nominal y se ven borrosos al escalar.
                                sourceSize.width: 16
                                sourceSize.height: 16
                                // Un icono que no está no deja un hueco roto:
                                // simplemente no se pinta.
                                visible: status === Image.Ready
                                // Cargar del disco no puede parar el desplazado
                                // de una carpeta con miles de entradas.
                                asynchronous: true
                            }
                            Text {
                                text: app.entry_names[row.index] ?? ""
                                Layout.fillWidth: true
                                Layout.preferredWidth: 320
                                Layout.maximumWidth: 560
                                elide: Text.ElideMiddle
                                color: Theme.text
                                font.family: Theme.family
                                font.pixelSize: Theme.sizeBase
                            }
                            Text {
                                text: app.entry_kinds[row.index] ?? ""
                                Layout.preferredWidth: 140
                                color: Theme.textDim
                                font.family: Theme.family
                                font.pixelSize: Theme.sizeBase
                            }
                            Text {
                                text: app.entry_sizes[row.index] ?? ""
                                Layout.preferredWidth: 110
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

                        MouseArea {
                            id: rowArea
                            anchors.fill: parent
                            hoverEnabled: true
                            acceptedButtons: Qt.LeftButton | Qt.RightButton
                            onClicked: mouse => {
                                fileList.currentIndex = row.index;
                                if (mouse.button === Qt.RightButton)
                                    rowMenu.popup();
                            }
                            onDoubleClicked: {
                                if (app.entry_kinds[row.index] === "Carpeta")
                                    app.cd(app.entry_names[row.index]);
                            }
                        }

                        Menu {
                            id: rowMenu
                            MenuItem {
                                text: qsTr("Enviar a la papelera")
                                onTriggered: app.trash(app.entry_names[row.index])
                            }
                        }
                    }
                }

                // Vacío explícito: una lista en blanco no distingue «carpeta vacía»
                // de «el filtro no deja pasar nada», y son dos situaciones distintas.
                Text {
                    anchors.centerIn: parent
                    visible: app.entry_count === 0
                    width: parent.width - 60
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    color: Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                    text: app.total_count === 0 ? qsTr("Esta carpeta está vacía") : qsTr("Ningún elemento coincide con el filtro")
                }
            }
        }

        // ---- Barra de estado -------------------------------------------------
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.statusBarHeight
            color: Theme.statusBar

            Rectangle {
                anchors.top: parent.top
                width: parent.width
                height: 1
                color: Theme.divider
            }

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                color: Theme.textDim
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
                // Con filtro activo se enseñan las dos cifras: si no, una carpeta
                // recortada por el filtro parece una carpeta pequeña.
                text: app.entry_count === app.total_count ? qsTr("%1 elementos").arg(app.total_count) : qsTr("%1 de %2 elementos").arg(app.entry_count).arg(app.total_count)
            }
        }
    }

    // Sin decoración del sistema no hay nada que separe la ventana del fondo:
    // en un escritorio oscuro, el borde superior de la barra de título se funde
    // con el escritorio y la ventana pierde su silueta. Este filo de un píxel es
    // lo que la devuelve. No intercepta el ratón: es un Rectangle, no un Item
    // con área.
    Rectangle {
        anchors.fill: parent
        color: "transparent"
        border.width: 1
        border.color: Theme.divider
        visible: win.visibility !== Window.Maximized
    }

    // ---- Agarres de redimensionado -----------------------------------------
    // Van al final para quedar por encima de todo lo demás.
    ResizeHandle {
        target: win
        edges: Qt.LeftEdge
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.margins: 6
        width: 6
    }
    ResizeHandle {
        target: win
        edges: Qt.RightEdge
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.margins: 6
        width: 6
    }
    ResizeHandle {
        target: win
        edges: Qt.TopEdge
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.margins: 6
        height: 6
    }
    ResizeHandle {
        target: win
        edges: Qt.BottomEdge
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.margins: 6
        height: 6
    }
    ResizeHandle {
        target: win
        edges: Qt.LeftEdge | Qt.TopEdge
        anchors.left: parent.left
        anchors.top: parent.top
        width: 8
        height: 8
    }
    ResizeHandle {
        target: win
        edges: Qt.RightEdge | Qt.TopEdge
        anchors.right: parent.right
        anchors.top: parent.top
        width: 8
        height: 8
    }
    ResizeHandle {
        target: win
        edges: Qt.LeftEdge | Qt.BottomEdge
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        width: 8
        height: 8
    }
    ResizeHandle {
        target: win
        edges: Qt.RightEdge | Qt.BottomEdge
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        width: 8
        height: 8
    }

    function toggleMaximized() {
        if (win.visibility === Window.Maximized)
            win.showNormal();
        else
            win.showMaximized();
    }
}
