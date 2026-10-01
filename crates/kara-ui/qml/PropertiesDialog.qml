pragma ComponentBehavior: Bound

// The Properties window: a read-only list of what the system knows about the
// selection. Nothing is computed here — `app.prop_labels` and `app.prop_values`
// are parallel lists the bridge rebuilds, and the folder size row updates
// itself while a background walk is still counting.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.kara.ui

Item {
    id: root

    required property App app
    anchors.fill: parent

    Dialog {
        id: dialog
        visible: root.app.prop_open
        parent: Overlay.overlay
        anchors.centerIn: parent
        modal: false
        width: 480
        title: root.app.prop_title
        closePolicy: Popup.CloseOnEscape

        // Closing from anywhere —Esc, the button— stops a size walk that is
        // still running.
        onClosed: root.app.close_properties()

        contentItem: ColumnLayout {
            spacing: 10

            GridLayout {
                columns: 2
                columnSpacing: 14
                rowSpacing: 6
                Layout.fillWidth: true

                Repeater {
                    model: root.app.prop_labels.length

                    delegate: Item {
                        id: line
                        required property int index
                        Layout.columnSpan: 2
                        Layout.fillWidth: true
                        implicitHeight: Math.max(label.implicitHeight, value.implicitHeight)

                        Text {
                            id: label
                            width: 110
                            text: root.app.prop_labels[line.index] ?? ""
                            color: Theme.textDim
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeBase
                        }
                        Text {
                            id: value
                            anchors.left: label.right
                            anchors.leftMargin: 10
                            anchors.right: parent.right
                            text: root.app.prop_values[line.index] ?? ""
                            wrapMode: Text.WrapAnywhere
                            color: Theme.text
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeBase
                        }
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Item {
                    Layout.fillWidth: true
                }
                Button {
                    id: close
                    text: qsTr("Cerrar")
                    focus: true
                    onClicked: dialog.close()
                }
            }
        }

        onOpened: close.forceActiveFocus()
    }
}
