pragma ComponentBehavior: Bound

// The passphrase of an encrypted volume.
//
// Reference: `ground/spec/06-contexto-power.md`, «Montar y expulsar
// unidades»: «Las unidades cifradas piden contraseña al montar». The
// passphrase goes to UDisks2 for the one call that unlocks the volume; the
// field is emptied whenever the dialog closes.
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
        visible: root.app.unlock_prompt
        parent: Overlay.overlay
        anchors.centerIn: parent
        modal: true
        width: 420
        title: qsTr("Desbloquear «%1»").arg(root.app.unlock_name)
        closePolicy: Popup.NoAutoClose

        function submit() {
            if (passphrase.text !== "" && !root.app.unlock_busy)
                root.app.unlock_volume(passphrase.text);
        }

        contentItem: ColumnLayout {
            spacing: 10
            Keys.onEscapePressed: root.app.cancel_unlock()

            Text {
                Layout.fillWidth: true
                text: qsTr("Este volumen está cifrado. Escribe su contraseña para abrirlo.")
                wrapMode: Text.WordWrap
                color: Theme.text
                font.family: Theme.family
                font.pixelSize: Theme.sizeBase
            }

            TextField {
                id: passphrase
                Layout.fillWidth: true
                echoMode: TextInput.Password
                placeholderText: qsTr("Contraseña")
                enabled: !root.app.unlock_busy
                onAccepted: dialog.submit()
                Keys.onEscapePressed: root.app.cancel_unlock()
            }

            Text {
                Layout.fillWidth: true
                visible: root.app.unlock_error !== "" || root.app.unlock_busy
                text: root.app.unlock_busy ? qsTr("Desbloqueando…") : root.app.unlock_error
                wrapMode: Text.WordWrap
                color: root.app.unlock_busy ? Theme.textDim : Theme.danger
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
            }

            RowLayout {
                Layout.fillWidth: true
                Item {
                    Layout.fillWidth: true
                }
                Button {
                    text: qsTr("Cancelar")
                    onClicked: root.app.cancel_unlock()
                }
                Button {
                    text: qsTr("Desbloquear")
                    highlighted: true
                    enabled: passphrase.text !== "" && !root.app.unlock_busy
                    onClicked: dialog.submit()
                }
            }
        }

        onOpened: {
            passphrase.text = "";
            passphrase.forceActiveFocus();
        }
        onClosed: passphrase.text = ""
    }

    // A wrong passphrase leaves the dialog open: select what was typed so the
    // next attempt replaces it.
    Connections {
        target: root.app
        function onUnlock_errorChanged() {
            if (root.app.unlock_error !== "") {
                passphrase.selectAll();
                passphrase.forceActiveFocus();
            }
        }
    }
}
