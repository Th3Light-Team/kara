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
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequences: ["Ctrl+L", "Alt+D", "F4"]
        onActivated: address.startEditing()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Alt+Left"
        onActivated: app.back()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Alt+Right"
        onActivated: app.forward()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Alt+Up"
        onActivated: app.up()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequences: ["F5", "Ctrl+R"]
        onActivated: app.reload()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+F"
        onActivated: filterField.forceActiveFocus()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "F9"
        onActivated: app.toggle_sidebar()
    }

    // ---- Selección (ground/spec/07-atajos-teclado.md, «Selección») --------
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+A"
        onActivated: app.select_all()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequences: ["Ctrl+Shift+A", "Ctrl+Shift+I"]
        onActivated: app.invert_selection()
    }
    Shortcut {
        // Escalonado, como pide la spec: cancela lo que esté en curso —el
        // marco, el renombrado— y solo si no hay nada en curso quita la
        // selección. La vista es quien sabe si tiene algo a medias.
        sequence: "Escape"
        // Los campos de texto tienen su propio Esc y son ellos quienes deben
        // atenderlo: un atajo se lleva la tecla **antes** que el que tiene el
        // foco, así que si esto estuviera siempre activo, Esc dejaría de
        // limpiar el filtro y de cerrar la barra de direcciones, y el foco se
        // quedaría dentro del campo comiéndose Ctrl+A y Supr.
        enabled: !ops.promptOpen && !address.editing && !filterField.activeFocus
        onActivated: {
            // El Loader no declara el tipo de lo que carga y las dos vistas no
            // comparten un tipo común: se pregunta a la que esté puesta, con el
            // mismo molde que usa F2.
            const detalles = fileView.item as FileDetails;
            const rejilla = fileView.item as FileGrid;
            let atendido = false;
            if (detalles)
                atendido = detalles.cancelPending();
            else if (rejilla)
                atendido = rejilla.cancelPending();
            if (!atendido)
                app.deselect_all();
        }
    }
    Shortcut {
        // Enter abre lo que tiene el cursor. Un campo de texto con el foco
        // —el filtro, las direcciones, el editor de renombrado— usa Enter
        // para confirmar lo suyo, y un atajo de ventana se lo quitaría.
        sequences: ["Return", "Enter"]
        enabled: !ops.promptOpen && !(win.activeFocusItem && win.activeFocusItem.hasOwnProperty("cursorPosition"))
        onActivated: app.open_focused()
    }

    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        // La papelera, nunca el borrado: `Shift+Supr` pide un borrado
        // permanente que el backend todavía no tiene.
        sequence: "Delete"
        onActivated: app.trash_selected()
    }

    // Modos de vista, con la numeración del Explorador de Windows: 1 a 4 son
    // los cuatro tamaños de icono, 5 lista, 6 detalles, 7 mosaico. Falta el 8,
    // «contenido», que todavía no existe.
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+Shift+1"
        onActivated: app.set_view(3, 256)
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+Shift+2"
        onActivated: app.set_view(3, 128)
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+Shift+3"
        onActivated: app.set_view(3, 96)
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+Shift+4"
        onActivated: app.set_view(3, 48)
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+Shift+5"
        onActivated: app.set_view(1, 0)
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+Shift+6"
        onActivated: app.set_view(0, 0)
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+Shift+7"
        onActivated: app.set_view(2, 0)
    }

    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequences: ["Ctrl++", "Ctrl+="]
        onActivated: app.zoom_in()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+-"
        onActivated: app.zoom_out()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+0"
        onActivated: app.reset_zoom()
    }

    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+1"
        onActivated: {
            app.use_focus_mode(false);
            app.activate_tab(0);
        }
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+2"
        onActivated: {
            app.use_focus_mode(false);
            app.activate_tab(1);
        }
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+3"
        onActivated: {
            app.use_focus_mode(false);
            app.activate_tab(2);
        }
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+4"
        onActivated: {
            app.use_focus_mode(false);
            app.activate_tab(3);
        }
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+5"
        onActivated: {
            app.use_focus_mode(false);
            app.activate_tab(4);
        }
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+6"
        onActivated: {
            app.use_focus_mode(false);
            app.activate_tab(5);
        }
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+7"
        onActivated: {
            app.use_focus_mode(false);
            app.activate_tab(6);
        }
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+8"
        onActivated: {
            app.use_focus_mode(false);
            app.activate_tab(7);
        }
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        // El 9 va a la última, esté donde esté, como en los navegadores.
        sequence: "Ctrl+9"
        onActivated: {
            app.use_focus_mode(false);
            app.activate_tab(app.tab_count - 1);
        }
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+T"
        onActivated: {
            // Abrir una pestaña en modo concentración la dejaría escondida.
            app.use_focus_mode(false);
            app.open_tab(app.path, false);
        }
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+W"
        onActivated: app.close_tab(app.active_tab)
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+Shift+T"
        onActivated: {
            app.use_focus_mode(false);
            app.reopen_tab();
        }
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequences: ["Ctrl+Tab", "Ctrl+PgDown"]
        onActivated: app.next_tab()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequences: ["Ctrl+Shift+Tab", "Ctrl+PgUp"]
        onActivated: app.previous_tab()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+C"
        onActivated: app.copy_selection()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+X"
        onActivated: app.cut_selection()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+V"
        onActivated: app.paste()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+Z"
        onActivated: app.undo()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequences: ["Ctrl+Y", "Ctrl+Shift+Z"]
        onActivated: app.redo()
    }
    Shortcut {
        enabled: !ops.promptOpen
        sequences: ["Alt+Return", "Alt+Enter"]
        onActivated: app.show_properties()
    }
    Shortcut {
        enabled: !ops.promptOpen
        sequence: "Ctrl+Shift+C"
        onActivated: app.copy_path()
    }
    Shortcut {
        enabled: !ops.promptOpen
        sequences: ["Ctrl+H", "Alt+."]
        onActivated: app.toggle_hidden()
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "Ctrl+Shift+N"
        onActivated: app.create_folder(qsTr("Nueva carpeta"))
    }
    Shortcut {
        // Una pregunta abierta es de quien espera la respuesta: ni Supr ni Enter
        // ni Ctrl+V deben llegar a la vista que hay debajo.
        enabled: !ops.promptOpen
        sequence: "F2"
        onActivated: {
            // El editor en línea vive en la vista de detalles; en las rejillas
            // todavía no hay dónde escribir. El molde falla y da null cuando lo
            // cargado es la rejilla, que es justo la comprobación que hace
            // falta.
            const detalles = fileView.item as FileDetails;
            if (detalles)
                detalles.startRename();
        }
    }

    // Los límites tienen que coincidir con los que `prefs.rs` aplica al leer:
    // ahí se recortan los valores absurdos de un fichero editado a mano.
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

        TabStrip {
            app: app
            Layout.fillWidth: true
            Layout.preferredHeight: height
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
                NavButton {
                    glyph: "↶"
                    // La spec pide decir *qué* se deshace, no solo ofrecerlo.
                    tip: app.can_undo ? qsTr("Deshacer %1 (Ctrl+Z)").arg(app.undo_label) : qsTr("Nada que deshacer")
                    enabled: app.can_undo
                    onClicked: app.undo()
                }
                NavButton {
                    glyph: "↷"
                    tip: app.can_redo ? qsTr("Rehacer %1 (Ctrl+Y)").arg(app.redo_label) : qsTr("Nada que rehacer")
                    enabled: app.can_redo
                    onClicked: app.redo()
                }

                // Conmutador de modo de vista. Windows lo esconde en un menú
                // «Ver»; aquí está a la vista porque todavía no hay menú y un
                // modo que no se encuentra es un modo que no existe.
                Row {
                    Layout.leftMargin: 8
                    Layout.rightMargin: 2
                    spacing: 1

                    Repeater {
                        model: [
                            {
                                mode: 0,
                                tip: qsTr("Detalles (Ctrl+Shift+6)")
                            },
                            {
                                mode: 1,
                                tip: qsTr("Lista (Ctrl+Shift+5)")
                            },
                            {
                                mode: 2,
                                tip: qsTr("Mosaico (Ctrl+Shift+7)")
                            },
                            {
                                mode: 3,
                                tip: qsTr("Iconos (Ctrl+Shift+3)")
                            }
                        ]
                        delegate: ViewModeButton {
                            required property var modelData
                            mode: modelData.mode
                            tip: modelData.tip
                            currentMode: app.view_mode
                            onClicked: app.set_view(modelData.mode, 0)
                        }
                    }
                }

                // «Ver» y lo que no tiene botón propio. Todo lo de aquí tiene
                // también su atajo; el menú es para encontrarlo.
                NavButton {
                    id: moreButton
                    glyph: "⋯"
                    tip: qsTr("Más opciones")
                    onClicked: moreMenu.popup(moreButton, 0, moreButton.height)

                    Menu {
                        id: moreMenu
                        MenuItem {
                            text: qsTr("Mostrar archivos ocultos\tCtrl+H")
                            checkable: true
                            checked: app.show_hidden
                            onTriggered: app.toggle_hidden()
                        }
                        MenuItem {
                            text: qsTr("Mostrar extensiones de nombre")
                            checkable: true
                            checked: app.show_extensions
                            onTriggered: app.toggle_extensions()
                        }
                        MenuSeparator {}
                        MenuItem {
                            text: qsTr("Copiar ruta\tCtrl+Shift+C")
                            enabled: !app.in_trash
                            onTriggered: app.copy_path()
                        }
                        MenuItem {
                            text: qsTr("Abrir terminal aquí")
                            enabled: !app.in_trash
                            onTriggered: app.open_terminal_here(-1)
                        }
                        MenuSeparator {}
                        MenuItem {
                            text: qsTr("Propiedades\tAlt+Intro")
                            enabled: !app.in_trash
                            onTriggered: app.show_properties()
                        }
                    }
                }

                // Solo dentro de la papelera: vaciarla no es una acción que
                // deba estar a mano desde cualquier carpeta.
                NavButton {
                    glyph: "🗑"
                    tip: qsTr("Vaciar la papelera")
                    visible: app.in_trash
                    onClicked: emptyTrashDialog.open()
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
                                // El Loader no declara el tipo de lo que
                                // carga; el molde le dice a las herramientas
                                // que es un Item, que es lo único que se pide.
                                const vista = fileView.item as Item;
                                if (vista)
                                    vista.forceActiveFocus();
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
                Layout.preferredWidth: app.sidebar_width
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
                        const propuesto = app.sidebar_width + mouse.x - splitter.grabbedAt;
                        app.remember_sidebar_width(Math.max(win.sidebarMin, Math.min(win.sidebarMax, propuesto)));
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                color: Theme.content

                // Detalles es una tabla con cabecera y el resto son rejillas:
                // dos vistas distintas, no una con banderas. El Loader deja
                // viva solo la que se enseña, para que la otra no cree
                // delegados de miles de entradas que nadie mira.
                Loader {
                    id: fileView
                    anchors.fill: parent
                    sourceComponent: app.view_mode === 0 ? detailsMode : gridMode
                }

                Component {
                    id: detailsMode
                    FileDetails {
                        app: app
                    }
                }
                Component {
                    id: gridMode
                    FileGrid {
                        app: app
                    }
                }

                // Ctrl+rueda sobre el area de ficheros: la conveniencia de zoom
                // de la spec. Va por encima de la vista y solo se queda los
                // eventos con Ctrl; el resto siguen hasta la barra de
                // desplazamiento.
                WheelHandler {
                    acceptedModifiers: Qt.ControlModifier
                    onWheel: event => {
                        if (event.angleDelta.y > 0)
                            app.zoom_in();
                        else if (event.angleDelta.y < 0)
                            app.zoom_out();
                    }
                }

                // Aviso de lo último que salió mal. Va por encima de la vista y
                // solo aparece cuando hay algo que contar: una operación que
                // falla sin decirlo es indistinguible de un clic que no llegó.
                Rectangle {
                    id: errorBanner
                    anchors.top: parent.top
                    anchors.left: parent.left
                    anchors.right: parent.right
                    height: visible ? 34 : 0
                    visible: app.last_error !== ""
                    color: Theme.danger
                    z: 10

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 14
                        anchors.rightMargin: 6
                        spacing: 8

                        Text {
                            text: app.last_error
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                            color: Theme.dangerText
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeBase
                        }
                        Item {
                            implicitWidth: 26
                            implicitHeight: 26

                            Text {
                                anchors.centerIn: parent
                                text: "✕"
                                color: Theme.dangerText
                                font.pixelSize: Theme.sizeSmall
                            }
                            MouseArea {
                                anchors.fill: parent
                                cursorShape: Qt.PointingHandCursor
                                onClicked: app.clear_error()
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

            // Los dos modos de navegación, al pie a la derecha: concentración
            // —una sola pestaña, la ventana como si no las tuviera— o barra de
            // pestañas.
            Row {
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2

                TabModeButton {
                    focused: true
                    active: app.focus_mode
                    tip: qsTr("Concentración: una sola pestaña")
                    onClicked: app.use_focus_mode(true)
                }
                TabModeButton {
                    focused: false
                    active: !app.focus_mode
                    tip: qsTr("Pestañas (%1 abiertas)").arg(app.tab_count)
                    onClicked: app.use_focus_mode(false)
                }
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
                text: app.loading ? qsTr("Cargando…") : app.entry_count === app.total_count ? qsTr("%1 elementos").arg(app.total_count) : qsTr("%1 de %2 elementos").arg(app.entry_count).arg(app.total_count)
            }
        }
    }

    PropertiesDialog {
        app: app
    }

    OperationDialog {
        id: ops
        app: app
        onReleased: {
            const view = fileView.item as Item;
            if (view)
                view.forceActiveFocus();
        }
    }

    // Vaciar la papelera es irreversible, así que se confirma, y el foco
    // arranca en el botón que no destruye nada. Es una regla del proyecto, no
    // una cortesía.
    Dialog {
        id: emptyTrashDialog
        anchors.centerIn: parent
        modal: true
        title: qsTr("Vaciar la papelera")
        standardButtons: Dialog.Cancel | Dialog.Yes

        onOpened: {
            const cancelar = emptyTrashDialog.standardButton(Dialog.Cancel);
            if (cancelar)
                cancelar.forceActiveFocus();
        }
        onAccepted: app.empty_trash()

        Text {
            text: qsTr("Se eliminarán definitivamente todos los elementos.\nEsto no se puede deshacer.")
            color: Theme.text
            font.family: Theme.family
            font.pixelSize: Theme.sizeBase
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

    // Prueba de extremo a extremo, solo con `--e2e`. Inactivo el `Loader` no
    // crea nada, así que en un arranque normal esto no existe.
    Loader {
        active: Qt.application.arguments.indexOf("--e2e") >= 0
        sourceComponent: E2E {
            win: win
            app: app
            fileView: fileView
            address: address
            filterField: filterField
        }
    }
}
