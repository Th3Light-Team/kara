pragma ComponentBehavior: Bound

// The «Unidades remotas» block of the navigation panel: SFTP and object
// storage drives, each with its connection state, and the «+» that opens the
// register-a-drive dialog.
//
// Every fact comes from `Drives` (Rust): the rows, their states, what a menu
// may offer. This file only draws them and forwards clicks.
import QtQuick
import QtQuick.Controls
import com.kara.ui

Item {
    id: root

    required property var drives

    readonly property int rowHeight: 42
    readonly property int shownRows: Math.min(root.drives.drive_count, 4)

    implicitHeight: header.height + (root.drives.drive_count === 0 ? emptyHint.height : root.shownRows * root.rowHeight) + noticeBar.height + noteBar.height

    // ---- Header: title and «+» ------------------------------------------------
    Item {
        id: header
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 32

        Text {
            anchors.left: parent.left
            anchors.leftMargin: 14
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("Unidades remotas")
            color: Theme.headerText
            font.family: Theme.family
            font.pixelSize: Theme.sizeSmall
            font.weight: Font.DemiBold
        }

        Item {
            id: addButton
            anchors.right: parent.right
            anchors.rightMargin: 10
            anchors.verticalCenter: parent.verticalCenter
            width: 24
            height: 24
            enabled: root.drives.adapters_available
            opacity: enabled ? 1 : 0.4

            Rectangle {
                anchors.fill: parent
                radius: Theme.radius
                color: addArea.pressed ? Theme.pressed : (addArea.containsMouse ? Theme.hover : "transparent")
            }
            // A plus drawn from two bars: a font glyph differs per desktop.
            Rectangle {
                anchors.centerIn: parent
                width: 12
                height: 1.5
                color: addArea.containsMouse ? Theme.text : Theme.textDim
            }
            Rectangle {
                anchors.centerIn: parent
                width: 1.5
                height: 12
                color: addArea.containsMouse ? Theme.text : Theme.textDim
            }
            MouseArea {
                id: addArea
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.drives.open_add()
            }
            ToolTip.visible: addArea.containsMouse
            ToolTip.delay: 600
            ToolTip.text: addButton.enabled ? qsTr("Añadir unidad remota…") : qsTr("Esta versión no incluye protocolos remotos")
            Accessible.role: Accessible.Button
            Accessible.name: qsTr("Añadir unidad remota")
        }
    }

    Text {
        id: emptyHint
        visible: root.drives.drive_count === 0
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: header.bottom
        anchors.leftMargin: 14
        anchors.rightMargin: 10
        height: visible ? 30 : 0
        text: qsTr("Sin unidades. Pulsa + para añadir una.")
        elide: Text.ElideRight
        verticalAlignment: Text.AlignTop
        color: Theme.textDim
        font.family: Theme.family
        font.pixelSize: Theme.sizeSmall
    }

    // ---- The drives -------------------------------------------------------------
    ListView {
        id: list
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: header.bottom
        height: root.shownRows * root.rowHeight
        clip: true
        model: root.drives.drive_count
        ScrollBar.vertical: ScrollBar {}

        delegate: Item {
            id: row

            required property int index

            readonly property string driveId: root.drives.drive_ids[row.index] ?? ""
            readonly property string label: root.drives.drive_labels[row.index] ?? ""
            readonly property string subtitle: root.drives.drive_subtitles[row.index] ?? ""
            readonly property string state: root.drives.drive_states[row.index] ?? "disconnected"
            readonly property string stateText: root.drives.drive_state_texts[row.index] ?? ""
            readonly property int flags: root.drives.drive_flags[row.index] ?? 0
            readonly property bool broken: row.state === "lost" || row.state === "failed"

            width: list.width
            height: root.rowHeight

            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 6
                anchors.rightMargin: 6
                anchors.topMargin: 1
                anchors.bottomMargin: 1
                radius: Theme.radius
                color: rowArea.containsMouse ? Theme.hover : "transparent"
            }

            // Icon with the connection state as a badge on its corner.
            Item {
                id: iconBox
                anchors.left: parent.left
                anchors.leftMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                width: 18
                height: 18

                Image {
                    anchors.fill: parent
                    source: "qrc:/qt/qml/com/kara/ui/icons/network.svg"
                    sourceSize.width: 18
                    sourceSize.height: 18
                    opacity: row.state === "ready" ? 1 : 0.65
                }
                Rectangle {
                    id: badge
                    width: 9
                    height: 9
                    radius: 5
                    x: parent.width - 6
                    y: parent.height - 6
                    border.width: 2
                    border.color: Theme.sidebar
                    color: row.state === "ready" ? Theme.success : row.broken ? Theme.danger : row.state === "connecting" ? Theme.accent : Theme.textDim
                    SequentialAnimation on opacity {
                        running: row.state === "connecting"
                        loops: Animation.Infinite
                        NumberAnimation {
                            to: 0.3
                            duration: 500
                        }
                        NumberAnimation {
                            to: 1
                            duration: 500
                        }
                        onRunningChanged: if (!running)
                            badge.opacity = 1
                    }
                }
            }

            Column {
                anchors.left: iconBox.right
                anchors.leftMargin: 12
                anchors.right: reconnect.visible ? reconnect.left : parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                spacing: 1

                Text {
                    width: parent.width
                    text: row.label
                    elide: Text.ElideRight
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }
                Text {
                    width: parent.width
                    text: row.state === "connecting" ? qsTr("Conectando…") : row.broken ? row.stateText : row.subtitle
                    elide: Text.ElideRight
                    color: row.broken ? Theme.danger : Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeSmall
                }
            }

            // A lost or failed drive offers the way back right on its row.
            Text {
                id: reconnect
                visible: row.broken
                anchors.right: parent.right
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Reconectar")
                color: reconnectArea.containsMouse ? Theme.text : Theme.accent
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
                MouseArea {
                    id: reconnectArea
                    anchors.fill: parent
                    anchors.margins: -4
                    hoverEnabled: true
                    onClicked: root.drives.connect_drive(row.driveId)
                }
            }

            Menu {
                id: menu
                MenuItem {
                    text: row.broken ? qsTr("Reconectar") : qsTr("Conectar")
                    enabled: (row.flags & 1) !== 0
                    onTriggered: root.drives.connect_drive(row.driveId)
                }
                MenuItem {
                    text: qsTr("Desconectar")
                    enabled: (row.flags & 2) !== 0
                    onTriggered: root.drives.disconnect_drive(row.driveId)
                }
                MenuSeparator {}
                MenuItem {
                    text: qsTr("Editar…")
                    enabled: (row.flags & 4) !== 0
                    onTriggered: root.drives.open_edit(row.driveId)
                }
                MenuItem {
                    text: qsTr("Quitar…")
                    onTriggered: root.drives.ask_remove(row.driveId)
                }
            }

            MouseArea {
                id: rowArea
                anchors.fill: parent
                z: -1
                hoverEnabled: true
                acceptedButtons: Qt.LeftButton | Qt.RightButton
                onClicked: mouse => {
                    if (mouse.button === Qt.RightButton)
                        menu.popup();
                    else
                        root.drives.activate(row.driveId);
                }
            }
            ToolTip.visible: rowArea.containsMouse
            ToolTip.delay: 700
            ToolTip.text: row.label + "\n" + row.subtitle + "\n" + row.stateText
        }
    }

    // ---- What just happened ----------------------------------------------------
    Item {
        id: noticeBar
        visible: root.drives.notice !== ""
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: list.bottom
        anchors.topMargin: emptyHint.visible ? emptyHint.height : 0
        height: visible ? Math.max(noticeText.implicitHeight + 10, 26) : 0

        Timer {
            running: noticeBar.visible
            interval: 12000
            onTriggered: root.drives.dismiss_notice()
        }
        Rectangle {
            anchors.fill: parent
            anchors.leftMargin: 6
            anchors.rightMargin: 6
            radius: Theme.radius
            color: Theme.field
            border.width: 1
            border.color: Theme.divider
        }
        Text {
            id: noticeText
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            anchors.leftMargin: 14
            anchors.rightMargin: 14
            text: root.drives.notice
            wrapMode: Text.WordWrap
            maximumLineCount: 4
            elide: Text.ElideRight
            color: Theme.text
            font.family: Theme.family
            font.pixelSize: Theme.sizeSmall
        }
        MouseArea {
            anchors.fill: parent
            onClicked: root.drives.dismiss_notice()
        }
    }

    Text {
        id: noteBar
        visible: root.drives.secrets_note !== ""
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: noticeBar.bottom
        anchors.leftMargin: 14
        anchors.rightMargin: 10
        height: visible ? implicitHeight + 6 : 0
        text: root.drives.secrets_note
        wrapMode: Text.WordWrap
        color: Theme.textDim
        font.family: Theme.family
        font.pixelSize: Theme.sizeSmall
    }
}
