pragma ComponentBehavior: Bound

// La barra de pestañas.
//
// Referencia: `ground/spec/01-navegacion.md`, «Pestañas de carpetas»,
// «Reabrir pestaña cerrada» y «Abrir carpeta en pestaña o ventana nueva».
//
// En modo concentración la barra **se pliega, no se vacía**: las pestañas
// siguen abiertas y solo dejan de verse. Las que no están activas se desvanecen
// primero y la barra se cierra después, de modo que la ventana acaba
// exactamente como si nunca hubiera tenido pestañas.
import QtQuick
import com.kara.ui

Rectangle {
    id: strip

    required property var app

    readonly property int fullHeight: 34

    color: Theme.titleBar
    clip: true
    height: strip.app.focus_mode ? 0 : strip.fullHeight

    Behavior on height {
        NumberAnimation {
            duration: 180
            easing.type: Easing.OutCubic
        }
    }

    Row {
        anchors.left: parent.left
        anchors.leftMargin: 8
        anchors.bottom: parent.bottom
        spacing: 2

        Repeater {
            model: strip.app.tab_count

            delegate: Rectangle {
                id: tab

                required property int index
                readonly property bool current: strip.app.active_tab === tab.index

                width: Math.min(200, Math.max(120, title.implicitWidth + 52))
                height: strip.fullHeight - 4
                radius: Theme.radius
                color: {
                    if (tab.current)
                        return Theme.content;
                    return tabArea.containsMouse ? Theme.hover : "transparent";
                }

                // Solo la activa sigue ahí mientras la barra se pliega: es lo
                // que hace que el gesto se lea como «quedarse con esta».
                opacity: strip.app.focus_mode && !tab.current ? 0 : 1
                Behavior on opacity {
                    NumberAnimation {
                        duration: 140
                    }
                }

                Text {
                    id: title
                    anchors.left: parent.left
                    anchors.leftMargin: 12
                    anchors.right: closeButton.left
                    anchors.rightMargin: 4
                    anchors.verticalCenter: parent.verticalCenter
                    text: strip.app.tab_titles[tab.index] ?? ""
                    elide: Text.ElideRight
                    color: tab.current ? Theme.text : Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }

                Item {
                    id: closeButton
                    anchors.right: parent.right
                    anchors.rightMargin: 4
                    anchors.verticalCenter: parent.verticalCenter
                    width: 20
                    height: 20
                    // Cerrar la única pestaña no vacía la barra, así que
                    // tampoco se ofrece.
                    visible: strip.app.tab_count > 1

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radius
                        color: closeArea.containsMouse ? Theme.pressed : "transparent"
                    }
                    Text {
                        anchors.centerIn: parent
                        text: "✕"
                        color: Theme.textDim
                        font.pixelSize: 9
                    }
                    MouseArea {
                        id: closeArea
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: strip.app.close_tab(tab.index)
                    }
                }

                MouseArea {
                    id: tabArea
                    anchors.fill: parent
                    anchors.rightMargin: 24
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.MiddleButton
                    onClicked: mouse => {
                        // El clic central cierra, como en cualquier navegador.
                        if (mouse.button === Qt.MiddleButton)
                            strip.app.close_tab(tab.index);
                        else
                            strip.app.activate_tab(tab.index);
                    }
                }
            }
        }

        // Pestaña nueva.
        Item {
            width: 28
            height: strip.fullHeight - 4

            Rectangle {
                anchors.fill: parent
                anchors.margins: 4
                radius: Theme.radius
                color: addArea.containsMouse ? Theme.hover : "transparent"
            }
            Text {
                anchors.centerIn: parent
                text: "+"
                color: Theme.textDim
                font.pixelSize: 15
            }
            MouseArea {
                id: addArea
                anchors.fill: parent
                hoverEnabled: true
                onClicked: strip.app.open_tab(strip.app.path, false)
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
