// ============================================================================
//  Explorador de archivos — SPIKE en QML puro (motor nativo Qt Quick)
//  Demuestra el potencial de QML: no es una implementación real, el árbol
//  de ficheros es falso. Estética Windows 11 / Fluent, claro y oscuro.
// ============================================================================
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls.Basic
import QtQuick.Effects
import "util.js" as Util

Window {
    id: win
    width: 1200
    height: 760
    visible: true
    color: "transparent"
    flags: Qt.Window | Qt.FramelessWindowHint
    title: "Explorador — Spike QML"

    // ---- Estado global -----------------------------------------------------
    property bool darkMode: false            // render.py lo sobrescribe
    property string viewMode: "details"      // "details" | "grid"
    property bool effectsEnabled: true       // sombra/blur; off para captura offscreen (sin GPU)
    property string searchText: ""
    property var currentNode: null
    property var currentTrail: []
    property int selIndex: -1
    property var history: []
    property int histIndex: -1

    Theme { id: theme; dark: win.darkMode }

    // ---- Sistema de ficheros FALSO ----------------------------------------
    readonly property var fsRoot: ({
        name: "Este equipo", type: "folder", children: [
            { name: "Escritorio", type: "folder", modified: "2026-08-30 09:14", children: [
                { name: "Proyectos", type: "folder", modified: "2026-08-28 18:02", children: [] },
                { name: "captura-pantalla.png", type: "file", modified: "2026-08-30 09:12", size: "842 KB" },
                { name: "ideas.txt", type: "file", modified: "2026-08-29 22:40", size: "3 KB" }
            ]},
            { name: "Documentos", type: "folder", modified: "2026-08-31 11:20", children: [
                { name: "Facturas", type: "folder", modified: "2026-07-15 10:00", children: [
                    { name: "factura-2024.pdf", type: "file", modified: "2024-12-31 23:59", size: "128 KB" },
                    { name: "factura-2025.pdf", type: "file", modified: "2025-12-31 23:58", size: "131 KB" }
                ]},
                { name: "Proyectos", type: "folder", modified: "2026-09-01 00:05", children: [
                    { name: "qml-explorer", type: "folder", modified: "2026-09-01 00:05", children: [
                        { name: "Main.qml", type: "file", modified: "2026-09-01 00:05", size: "14 KB" },
                        { name: "FolderTree.qml", type: "file", modified: "2026-09-01 00:04", size: "3 KB" },
                        { name: "main.py", type: "file", modified: "2026-09-01 00:01", size: "1 KB" },
                        { name: "README.md", type: "file", modified: "2026-09-01 00:06", size: "6 KB" }
                    ]},
                    { name: "web-app", type: "folder", modified: "2026-08-20 16:44", children: [
                        { name: "index.html", type: "file", modified: "2026-08-20 16:44", size: "9 KB" },
                        { name: "styles.css", type: "file", modified: "2026-08-20 16:40", size: "12 KB" },
                        { name: "app.js", type: "file", modified: "2026-08-20 16:42", size: "22 KB" }
                    ]}
                ]},
                { name: "CV.docx", type: "file", modified: "2026-06-11 08:30", size: "48 KB" },
                { name: "presupuesto.xlsx", type: "file", modified: "2026-08-01 12:15", size: "35 KB" },
                { name: "presentacion.pptx", type: "file", modified: "2026-05-22 19:05", size: "2.4 MB" }
            ]},
            { name: "Descargas", type: "folder", modified: "2026-08-31 20:11", children: [
                { name: "instalador-app.exe", type: "file", modified: "2026-08-31 20:11", size: "84 MB" },
                { name: "ubuntu-26.04.iso", type: "file", modified: "2026-08-25 14:00", size: "3.1 GB" },
                { name: "cancion.mp3", type: "file", modified: "2026-08-18 21:33", size: "7.2 MB" },
                { name: "tutorial.mp4", type: "file", modified: "2026-08-10 10:05", size: "312 MB" },
                { name: "recursos.zip", type: "file", modified: "2026-08-05 09:00", size: "56 MB" }
            ]},
            { name: "Imágenes", type: "folder", modified: "2026-08-29 17:50", children: [
                { name: "Vacaciones", type: "folder", modified: "2026-07-30 12:00", children: [
                    { name: "playa.jpg", type: "file", modified: "2026-07-28 15:20", size: "4.1 MB" },
                    { name: "montaña.jpg", type: "file", modified: "2026-07-29 09:10", size: "3.8 MB" }
                ]},
                { name: "wallpaper.jpg", type: "file", modified: "2026-08-29 17:50", size: "5.6 MB" },
                { name: "logo.png", type: "file", modified: "2026-08-12 11:11", size: "220 KB" }
            ]},
            { name: "Música", type: "folder", modified: "2026-08-14 22:00", children: [
                { name: "album-favorito.flac", type: "file", modified: "2026-08-14 22:00", size: "48 MB" },
                { name: "playlist.mp3", type: "file", modified: "2026-08-13 20:00", size: "9 MB" }
            ]},
            { name: "Vídeos", type: "folder", modified: "2026-08-09 18:00", children: [
                { name: "demo-qml.mp4", type: "file", modified: "2026-09-01 00:07", size: "88 MB" }
            ]},
            { name: "Disco local (C:)", type: "folder", modified: "2026-09-01 07:00", children: [
                { name: "Windows", type: "folder", modified: "2026-09-01 07:00", children: [] },
                { name: "Archivos de programa", type: "folder", modified: "2026-08-30 06:00", children: [] },
                { name: "Usuarios", type: "folder", modified: "2026-08-31 23:00", children: [
                    { name: "oliver", type: "folder", modified: "2026-09-01 00:00", children: [] }
                ]}
            ]},
            { name: "Disco (D:)", type: "folder", modified: "2026-08-28 13:00", children: [
                { name: "Backups", type: "folder", modified: "2026-08-28 13:00", children: [] },
                { name: "Juegos", type: "folder", modified: "2026-08-01 10:00", children: [] }
            ]}
        ]
    })

    // ---- Navegación --------------------------------------------------------
    function findTrail(node, target) {
        if (node === target) return [node];
        if (node.children)
            for (var i = 0; i < node.children.length; i++) {
                var r = findTrail(node.children[i], target);
                if (r) { r.unshift(node); return r; }
            }
        return null;
    }
    function findChild(node, name) {
        if (!node.children) return null;
        for (var i = 0; i < node.children.length; i++)
            if (node.children[i].name === name) return node.children[i];
        return null;
    }
    function navigateTo(node, record) {
        if (!node) return;
        if (record === undefined) record = true;
        currentNode = node;
        currentTrail = findTrail(fsRoot, node) || [node];
        selIndex = -1;
        searchText = "";
        if (record) {
            history = history.slice(0, histIndex + 1);
            history.push(node);
            histIndex = history.length - 1;
        }
    }
    function goBack()    { if (histIndex > 0) { histIndex--; navigateTo(history[histIndex], false); } }
    function goForward() { if (histIndex < history.length - 1) { histIndex++; navigateTo(history[histIndex], false); } }
    function goUp()      { if (currentTrail.length > 1) navigateTo(currentTrail[currentTrail.length - 2]); }

    // Contenido del panel derecho (carpeta actual, filtrado + ordenado)
    readonly property var currentContents: {
        searchText; currentNode;            // dependencias del binding
        if (!currentNode || !currentNode.children) return [];
        var s = searchText.trim().toLowerCase();
        var arr = currentNode.children.slice();
        if (s) arr = arr.filter(function (c) { return c.name.toLowerCase().indexOf(s) >= 0; });
        arr.sort(function (a, b) {
            if ((a.type === 'folder') !== (b.type === 'folder')) return a.type === 'folder' ? -1 : 1;
            return a.name.localeCompare(b.name);
        });
        return arr;
    }

    Component.onCompleted: navigateTo(findChild(fsRoot, "Documentos") || fsRoot)

    // Anchos de columna compartidos (cabecera + filas)
    readonly property int colModified: 165
    readonly property int colType: 150
    readonly property int colSize: 100

    // ========================================================================
    //  Fondo tipo escritorio (para que la ventana flote como captura Win11)
    // ========================================================================
    Rectangle {
        anchors.fill: parent
        gradient: Gradient {
            GradientStop { position: 0.0; color: win.darkMode ? "#0B1B2B" : "#3A6EA5" }
            GradientStop { position: 0.5; color: win.darkMode ? "#123047" : "#2E5F8A" }
            GradientStop { position: 1.0; color: win.darkMode ? "#0A2233" : "#1E4E79" }
        }
        // Un "blob" suave de luz (blur) para dar profundidad (requiere GPU)
        Rectangle {
            visible: win.effectsEnabled
            x: parent.width * 0.62; y: parent.height * 0.08
            width: 460; height: 460; radius: width / 2
            color: win.darkMode ? "#1E5A8A" : "#7FB2E5"; opacity: 0.35
            layer.enabled: win.effectsEnabled
            layer.effect: MultiEffect { blurEnabled: true; blur: 1.0; blurMax: 64 }
        }
    }

    // ========================================================================
    //  Ventana de la aplicación (flota con esquinas redondeadas + sombra)
    // ========================================================================
    Rectangle {
        id: appRect
        anchors.fill: parent
        anchors.margins: 28
        radius: 10
        clip: true
        color: theme.windowBg
        border.width: 1
        border.color: win.darkMode ? "#3A3A3A" : "#D8D8D8"

        // Sombra flotante (requiere GPU/RHI; se desactiva en captura offscreen)
        layer.enabled: win.effectsEnabled
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#77000000"
            shadowBlur: 1.0
            shadowVerticalOffset: 18
            autoPaddingEnabled: true
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            // ---- Barra de título personalizada ----------------------------
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 42
                color: theme.titleBar

                MouseArea {                       // arrastrar para mover
                    anchors.fill: parent
                    onPressed: if (typeof win.startSystemMove === "function") win.startSystemMove()
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 14
                    spacing: 8

                    Text { text: "🗂️"; font.pixelSize: 16 }
                    Text {
                        text: "Explorador de archivos"
                        color: theme.text; font.family: theme.family
                        font.pixelSize: 13; font.weight: Font.Medium
                    }
                    Item { Layout.fillWidth: true }

                    // Alternar tema
                    TitleButton {
                        theme: theme
                        glyph: win.darkMode ? "☀" : "☾"
                        onClicked: win.darkMode = !win.darkMode
                    }
                    TitleButton { theme: theme; glyph: "⎯"; onClicked: win.showMinimized() }
                    TitleButton {
                        theme: theme
                        glyph: win.visibility === Window.Maximized ? "🗗" : "🗖"
                        onClicked: win.visibility === Window.Maximized ? win.showNormal() : win.showMaximized()
                    }
                    TitleButton { theme: theme; glyph: "✕"; danger: true; onClicked: Qt.quit() }
                }
            }

            // ---- Barra de comandos (navegación + breadcrumb + búsqueda) ----
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 48
                color: theme.toolbar

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 10
                    anchors.rightMargin: 12
                    spacing: 4

                    NavButton { theme: theme; glyph: "←"; enabled: win.histIndex > 0; onClicked: win.goBack() }
                    NavButton { theme: theme; glyph: "→"; enabled: win.histIndex < win.history.length - 1; onClicked: win.goForward() }
                    NavButton { theme: theme; glyph: "↑"; enabled: win.currentTrail.length > 1; onClicked: win.goUp() }

                    // Breadcrumb (barra de direcciones)
                    Rectangle {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 32
                        Layout.leftMargin: 6
                        radius: 6
                        color: win.darkMode ? "#333333" : "#FFFFFF"
                        border.width: 1
                        border.color: theme.divider

                        Row {
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.left: parent.left
                            anchors.leftMargin: 10
                            spacing: 2
                            Repeater {
                                model: win.currentTrail
                                delegate: Row {
                                    required property var modelData
                                    required property int index
                                    spacing: 2
                                    Text {
                                        text: modelData.name
                                        color: index === win.currentTrail.length - 1 ? theme.text : theme.textDim
                                        font.family: theme.family; font.pixelSize: 13
                                        anchors.verticalCenter: parent.verticalCenter
                                        MouseArea {
                                            anchors.fill: parent
                                            cursorShape: Qt.PointingHandCursor
                                            onClicked: win.navigateTo(modelData)
                                        }
                                    }
                                    Text {
                                        text: "  ›  "
                                        visible: index < win.currentTrail.length - 1
                                        color: theme.textDim
                                        font.pixelSize: 13
                                        anchors.verticalCenter: parent.verticalCenter
                                    }
                                }
                            }
                        }
                    }

                    // Búsqueda
                    Rectangle {
                        Layout.preferredWidth: 220
                        Layout.preferredHeight: 32
                        radius: 6
                        color: win.darkMode ? "#333333" : "#FFFFFF"
                        border.width: 1
                        border.color: searchInput.activeFocus ? theme.accent : theme.divider

                        Row {
                            anchors.fill: parent
                            anchors.leftMargin: 10
                            spacing: 6
                            Text {
                                text: "🔍"; font.pixelSize: 13
                                anchors.verticalCenter: parent.verticalCenter
                            }
                            TextField {
                                id: searchInput
                                width: parent.width - 40
                                anchors.verticalCenter: parent.verticalCenter
                                placeholderText: "Buscar en " + (win.currentNode ? win.currentNode.name : "")
                                text: win.searchText
                                onTextChanged: win.searchText = text
                                color: theme.text
                                placeholderTextColor: theme.textDim
                                font.family: theme.family; font.pixelSize: 13
                                background: Item {}
                                leftPadding: 0
                            }
                        }
                    }
                }
            }

            Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: theme.divider }

            // ---- Cuerpo: navegación (izq) + contenido (der) ----------------
            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                // ----- Panel de navegación -----
                Rectangle {
                    Layout.preferredWidth: 250
                    Layout.fillHeight: true
                    color: theme.sidebar

                    Flickable {
                        id: navFlick
                        anchors.fill: parent
                        anchors.topMargin: 8
                        anchors.leftMargin: 8
                        anchors.rightMargin: 4
                        contentHeight: navCol.implicitHeight
                        clip: true
                        ScrollBar.vertical: ScrollBar { }

                        Column {
                            id: navCol
                            width: navFlick.width - 8
                            spacing: 2

                            Text {
                                text: "Acceso rápido"
                                color: theme.headerText
                                font.family: theme.family; font.pixelSize: 11; font.weight: Font.DemiBold
                                leftPadding: 8; bottomPadding: 4
                            }
                            Repeater {
                                model: ["Escritorio", "Descargas", "Documentos", "Imágenes"]
                                delegate: Rectangle {
                                    required property string modelData
                                    width: navCol.width; height: 30; radius: 5
                                    property var node: win.findChild(win.fsRoot, modelData)
                                    color: qaMouse.containsMouse ? theme.hover
                                         : (win.currentNode === node ? theme.selection : "transparent")
                                    Row {
                                        anchors.fill: parent; anchors.leftMargin: 10; spacing: 8
                                        Text { text: "⭐"; font.pixelSize: 13; anchors.verticalCenter: parent.verticalCenter }
                                        Text {
                                            text: modelData; color: theme.text
                                            font.family: theme.family; font.pixelSize: 13
                                            anchors.verticalCenter: parent.verticalCenter
                                        }
                                    }
                                    MouseArea {
                                        id: qaMouse; anchors.fill: parent; hoverEnabled: true
                                        onClicked: win.navigateTo(node)
                                    }
                                }
                            }

                            Rectangle { width: navCol.width - 8; height: 1; color: theme.divider; x: 4 }
                            Item { width: 1; height: 4 }

                            // El árbol recursivo
                            FolderTree {
                                width: navCol.width
                                node: win.fsRoot
                                theme: theme
                                currentNode: win.currentNode
                                onActivated: function (n) { win.navigateTo(n); }
                            }
                        }
                    }
                }

                Rectangle { Layout.preferredWidth: 1; Layout.fillHeight: true; color: theme.divider }

                // ----- Panel de contenido -----
                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    color: theme.content

                    ColumnLayout {
                        anchors.fill: parent
                        spacing: 0

                        // Cabecera del contenido: título + conmutador de vista
                        RowLayout {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 44
                            Layout.leftMargin: 16
                            Layout.rightMargin: 12

                            Text {
                                text: win.currentNode ? win.currentNode.name : ""
                                color: theme.text
                                font.family: theme.family; font.pixelSize: 18; font.weight: Font.DemiBold
                            }
                            Item { Layout.fillWidth: true }
                            ViewToggle { theme: theme; glyph: "☰"; active: win.viewMode === "details"; onClicked: win.viewMode = "details" }
                            ViewToggle { theme: theme; glyph: "▦"; active: win.viewMode === "grid";    onClicked: win.viewMode = "grid" }
                        }

                        // Cabecera de columnas (solo en vista detalles)
                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 30
                            visible: win.viewMode === "details"
                            color: "transparent"
                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: 42
                                anchors.rightMargin: 26
                                spacing: 10
                                Text { text: "Nombre"; Layout.fillWidth: true; color: theme.headerText; font.family: theme.family; font.pixelSize: 12 }
                                Text { text: "Fecha de modificación"; Layout.preferredWidth: win.colModified; color: theme.headerText; font.family: theme.family; font.pixelSize: 12 }
                                Text { text: "Tipo"; Layout.preferredWidth: win.colType; color: theme.headerText; font.family: theme.family; font.pixelSize: 12 }
                                Text { text: "Tamaño"; Layout.preferredWidth: win.colSize; horizontalAlignment: Text.AlignRight; color: theme.headerText; font.family: theme.family; font.pixelSize: 12 }
                            }
                            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: theme.divider }
                        }

                        // --- Vista DETALLES (lista) ---
                        ListView {
                            id: detailsView
                            visible: win.viewMode === "details"
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.topMargin: 4
                            clip: true
                            model: win.currentContents
                            currentIndex: win.selIndex
                            ScrollBar.vertical: ScrollBar { }

                            delegate: Rectangle {
                                required property var modelData
                                required property int index
                                width: ListView.view.width
                                height: 34
                                color: rowMa.containsMouse ? theme.hover
                                     : (win.selIndex === index ? theme.selection : "transparent")

                                Rectangle {
                                    width: 3; radius: 2; height: parent.height - 14
                                    anchors.verticalCenter: parent.verticalCenter; x: 6
                                    color: theme.accent; visible: win.selIndex === index
                                }
                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: 18
                                    anchors.rightMargin: 26
                                    spacing: 10
                                    Text { text: Util.iconFor(modelData); font.pixelSize: 16; Layout.preferredWidth: 22 }
                                    Text {
                                        text: modelData.name; Layout.fillWidth: true
                                        color: theme.text; font.family: theme.family; font.pixelSize: 13
                                        elide: Text.ElideRight
                                    }
                                    Text { text: modelData.modified || ""; Layout.preferredWidth: win.colModified; color: theme.textDim; font.family: theme.family; font.pixelSize: 12 }
                                    Text { text: Util.typeLabel(modelData); Layout.preferredWidth: win.colType; color: theme.textDim; font.family: theme.family; font.pixelSize: 12; elide: Text.ElideRight }
                                    Text { text: modelData.size || ""; Layout.preferredWidth: win.colSize; horizontalAlignment: Text.AlignRight; color: theme.textDim; font.family: theme.family; font.pixelSize: 12 }
                                }
                                MouseArea {
                                    id: rowMa; anchors.fill: parent; hoverEnabled: true
                                    onClicked: win.selIndex = index
                                    onDoubleClicked: if (modelData.type === 'folder') win.navigateTo(modelData)
                                }
                            }
                        }

                        // --- Vista ICONOS (cuadrícula) ---
                        GridView {
                            id: gridView
                            visible: win.viewMode === "grid"
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.topMargin: 8
                            Layout.leftMargin: 12
                            clip: true
                            cellWidth: 118
                            cellHeight: 108
                            model: win.currentContents
                            ScrollBar.vertical: ScrollBar { }

                            delegate: Item {
                                required property var modelData
                                required property int index
                                width: gridView.cellWidth
                                height: gridView.cellHeight
                                Rectangle {
                                    anchors.fill: parent; anchors.margins: 5; radius: 8
                                    color: cellMa.containsMouse ? theme.hover
                                         : (win.selIndex === index ? theme.selection : "transparent")
                                    Column {
                                        anchors.centerIn: parent
                                        width: parent.width - 12
                                        spacing: 6
                                        Text {
                                            text: Util.iconFor(modelData)
                                            font.pixelSize: 44
                                            anchors.horizontalCenter: parent.horizontalCenter
                                        }
                                        Text {
                                            text: modelData.name
                                            width: parent.width
                                            horizontalAlignment: Text.AlignHCenter
                                            wrapMode: Text.Wrap; maximumLineCount: 2; elide: Text.ElideRight
                                            color: theme.text; font.family: theme.family; font.pixelSize: 12
                                        }
                                    }
                                    MouseArea {
                                        id: cellMa; anchors.fill: parent; hoverEnabled: true
                                        onClicked: win.selIndex = index
                                        onDoubleClicked: if (modelData.type === 'folder') win.navigateTo(modelData)
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ---- Barra de estado ------------------------------------------
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 26
                color: theme.toolbar
                Rectangle { width: parent.width; height: 1; color: theme.divider }
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 14
                    anchors.rightMargin: 14
                    Text {
                        text: win.currentContents.length + " elementos"
                            + (win.selIndex >= 0 ? "  ·  1 seleccionado" : "")
                        color: theme.textDim; font.family: theme.family; font.pixelSize: 12
                    }
                    Item { Layout.fillWidth: true }
                    Text {
                        text: "Disco local (C:) — 128 GB libres de 512 GB"
                        color: theme.textDim; font.family: theme.family; font.pixelSize: 12
                    }
                }
            }
        }
    }

    // ---- Componentes reutilizables de botón (inline) -----------------------
    component TitleButton: Rectangle {
        property var theme
        property string glyph: ""
        property bool danger: false
        signal clicked()
        width: 42; height: 42
        color: tbMa.containsMouse ? (danger ? "#E81123" : theme.hover) : "transparent"
        Text {
            anchors.centerIn: parent; text: glyph
            font.pixelSize: 13
            color: (danger && tbMa.containsMouse) ? "white" : theme.text
        }
        MouseArea { id: tbMa; anchors.fill: parent; hoverEnabled: true; onClicked: parent.clicked() }
    }

    component NavButton: Rectangle {
        property var theme
        property string glyph: ""
        signal clicked()
        width: 34; height: 32; radius: 6
        opacity: enabled ? 1 : 0.35
        color: nbMa.containsMouse && enabled ? theme.hover : "transparent"
        Text { anchors.centerIn: parent; text: glyph; font.pixelSize: 15; color: theme.text }
        MouseArea { id: nbMa; anchors.fill: parent; hoverEnabled: true; onClicked: if (parent.enabled) parent.clicked() }
    }

    component ViewToggle: Rectangle {
        property var theme
        property string glyph: ""
        property bool active: false
        signal clicked()
        width: 34; height: 30; radius: 6
        color: active ? theme.selection : (vtMa.containsMouse ? theme.hover : "transparent")
        Text { anchors.centerIn: parent; text: glyph; font.pixelSize: 15; color: active ? theme.accent : theme.text }
        MouseArea { id: vtMa; anchors.fill: parent; hoverEnabled: true; onClicked: parent.clicked() }
    }
}
