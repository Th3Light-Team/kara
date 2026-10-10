pragma ComponentBehavior: Bound

// The questions a remote drive asks while connecting (password, unknown host
// key, changed host key) and the confirmation before forgetting a drive.
//
// A host key is a security decision, so the safe answer is always the default:
// the focus starts on the refusing button, and a changed key reads as a warning
// in the danger colour, never as a routine question.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.kara.ui

Item {
    id: root

    required property var drives
    anchors.fill: parent

    Dialog {
        id: prompt
        visible: root.drives.prompt_open
        parent: Overlay.overlay
        anchors.centerIn: parent
        modal: true
        width: 460
        title: root.drives.prompt_title
        closePolicy: Popup.NoAutoClose

        readonly property bool password: root.drives.prompt_kind === "password"
        readonly property bool changed: root.drives.prompt_kind === "changed"

        contentItem: ColumnLayout {
            spacing: 10
            focus: true
            Keys.onEscapePressed: root.drives.answer_prompt(false, "", 0)

            Text {
                Layout.fillWidth: true
                text: root.drives.prompt_text
                wrapMode: Text.WordWrap
                color: prompt.changed ? Theme.danger : Theme.text
                font.family: Theme.family
                font.pixelSize: Theme.sizeBase
                font.weight: prompt.changed ? Font.DemiBold : Font.Normal
            }

            // Fingerprints, where they can be compared character by character.
            Rectangle {
                Layout.fillWidth: true
                visible: root.drives.prompt_detail !== ""
                implicitHeight: detail.implicitHeight + 16
                radius: Theme.radius
                color: Theme.field
                border.width: 1
                border.color: prompt.changed ? Theme.danger : Theme.divider
                TextEdit {
                    id: detail
                    anchors.fill: parent
                    anchors.margins: 8
                    readOnly: true
                    selectByMouse: true
                    wrapMode: TextEdit.WrapAnywhere
                    text: root.drives.prompt_detail
                    color: Theme.text
                    font.family: "monospace"
                    font.pixelSize: Theme.sizeSmall
                }
            }

            TextField {
                id: secret
                Layout.fillWidth: true
                visible: prompt.password
                echoMode: TextInput.Password
                placeholderText: qsTr("Contraseña o frase de paso")
                onAccepted: if (text !== "")
                    prompt.submitPassword()
                Keys.onEscapePressed: root.drives.answer_prompt(false, "", 0)
            }
            CheckBox {
                id: keep
                visible: prompt.password
                text: qsTr("Recordar en el llavero")
            }

            RowLayout {
                Layout.fillWidth: true
                Item {
                    Layout.fillWidth: true
                }
                // Password
                Button {
                    visible: prompt.password
                    text: qsTr("Cancelar")
                    onClicked: root.drives.answer_prompt(false, "", 0)
                }
                Button {
                    visible: prompt.password
                    text: qsTr("Conectar")
                    highlighted: true
                    enabled: secret.text !== ""
                    onClicked: prompt.submitPassword()
                }
                // Host key: the refusing button comes first and holds the focus.
                Button {
                    id: refuse
                    visible: !prompt.password
                    text: prompt.changed ? qsTr("No continuar") : qsTr("Cancelar")
                    highlighted: true
                    onClicked: root.drives.answer_prompt(false, "", 0)
                }
                Button {
                    visible: root.drives.prompt_kind === "trust"
                    text: qsTr("Confiar solo ahora")
                    onClicked: root.drives.answer_prompt(true, "", 0)
                }
                Button {
                    visible: root.drives.prompt_kind === "trust"
                    text: qsTr("Confiar y recordar")
                    onClicked: root.drives.answer_prompt(true, "", 1)
                }
                Button {
                    visible: prompt.changed
                    text: qsTr("Continuar de todos modos")
                    onClicked: root.drives.answer_prompt(true, "", 0)
                }
            }
        }

        function submitPassword() {
            root.drives.answer_prompt(true, secret.text, keep.checked ? 1 : 0);
        }

        onOpened: {
            secret.text = "";
            keep.checked = false;
            if (prompt.password)
                secret.forceActiveFocus();
            else
                refuse.forceActiveFocus();
        }
        onClosed: secret.text = ""
    }

    // Forgetting a drive drops its configuration and stored password. It
    // touches nothing on the server, but it is not undoable, so it asks, with
    // the focus on the harmless button.
    Dialog {
        id: removal
        visible: root.drives.remove_prompt
        parent: Overlay.overlay
        anchors.centerIn: parent
        modal: true
        width: 420
        title: qsTr("Quitar unidad")
        closePolicy: Popup.NoAutoClose

        contentItem: ColumnLayout {
            spacing: 12
            focus: true
            Keys.onEscapePressed: root.drives.cancel_remove()

            Text {
                Layout.fillWidth: true
                text: root.drives.remove_text
                wrapMode: Text.WordWrap
                color: Theme.text
                font.family: Theme.family
                font.pixelSize: Theme.sizeBase
            }
            RowLayout {
                Layout.fillWidth: true
                Item {
                    Layout.fillWidth: true
                }
                Button {
                    id: keepDrive
                    text: qsTr("Cancelar")
                    highlighted: true
                    onClicked: root.drives.cancel_remove()
                }
                Button {
                    text: qsTr("Quitar")
                    onClicked: root.drives.confirm_remove()
                }
            }
        }

        onOpened: keepDrive.forceActiveFocus()
    }
}
