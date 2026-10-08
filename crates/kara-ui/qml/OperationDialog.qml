pragma ComponentBehavior: Bound

// Everything the user sees of a copy or a move in progress: the progress card,
// and the three questions the engine can ask — a name conflict, a failure on
// one item, and the closing summary.
//
// No decision is made here. `app.op_state` says which of them is open; every
// button hands a plain word back to the bridge (`answer_conflict`,
// `answer_failure`, `cancel_operation`), and the worker thread is blocked until
// one of them arrives. That is why none of the dialogs can be dismissed by
// clicking outside: closing without answering would leave the job waiting for
// ever. Esc answers with the least destructive option.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.kara.ui

Item {
    id: root

    required property App app

    /// Every question is answered and its dialog is closing: the keyboard
    /// focus should go back to whatever the window was showing, not stay on a
    /// button of a dialog that is no longer there.
    signal released
    // Parent for the popups. Not `parent`: a Popup in an Item that is not the
    // window's content centres itself on that Item.
    anchors.fill: parent

    /// A question is open: the window's shortcuts must stand down.
    // The passphrase prompt and the application chooser count too: a
    // window shortcut takes its key before the focused field, and Supr typed
    // into a passphrase must not send the selection to the trash.
    readonly property bool promptOpen: root.app.delete_prompt || root.app.op_state === "conflict" || root.app.op_state === "failure" || root.app.op_state === "summary" || root.app.unlock_prompt || root.app.chooser_open

    readonly property bool progressVisible: root.app.op_state === "calculating" || root.app.op_state === "running" || root.app.op_state === "conflict" || root.app.op_state === "failure"

    // ---- Progress ------------------------------------------------------------
    Rectangle {
        id: card
        visible: root.progressVisible
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.rightMargin: 16
        anchors.bottomMargin: Theme.statusBarHeight + 12
        width: 360
        height: cardColumn.implicitHeight + 24
        radius: Theme.radiusLarge
        color: Theme.toolbar
        border.color: Theme.divider
        z: 20

        ColumnLayout {
            id: cardColumn
            anchors.fill: parent
            anchors.margins: 12
            spacing: 6

            RowLayout {
                Layout.fillWidth: true
                Text {
                    text: root.app.op_title
                    Layout.fillWidth: true
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                    font.bold: true
                }
                Button {
                    id: stop
                    text: qsTr("Cancelar")
                    onClicked: root.app.cancel_operation()
                }
            }
            ProgressBar {
                Layout.fillWidth: true
                // -1 means «still measuring»: an indeterminate bar says so,
                // a bar stuck at 0 % would say the opposite.
                indeterminate: root.app.op_progress < 0
                value: Math.max(0, root.app.op_progress)
            }
            Text {
                text: root.app.op_current
                visible: text !== ""
                Layout.fillWidth: true
                elide: Text.ElideMiddle
                color: Theme.text
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
            }
            Text {
                text: root.app.op_detail
                Layout.fillWidth: true
                elide: Text.ElideRight
                color: Theme.textDim
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
            }
        }
    }

    // ---- Permanent delete ----------------------------------------------------
    // Cannot be switched off, and focus starts on the button that destroys
    // nothing: an Enter pressed by reflex must not delete anything.
    Dialog {
        id: confirmDelete
        visible: root.app.delete_prompt
        parent: Overlay.overlay
        anchors.centerIn: parent
        modal: true
        closePolicy: Popup.NoAutoClose
        title: qsTr("Eliminar permanentemente")
        width: 440

        contentItem: ColumnLayout {
            spacing: 14
            Keys.onEscapePressed: root.app.cancel_permanent_delete()

            Text {
                text: root.app.delete_text
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Theme.text
                font.family: Theme.family
                font.pixelSize: Theme.sizeBase
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                Item {
                    Layout.fillWidth: true
                }
                Button {
                    id: keep
                    text: qsTr("Cancelar")
                    focus: true
                    onClicked: root.app.cancel_permanent_delete()
                }
                Button {
                    text: qsTr("Eliminar")
                    palette.button: Theme.danger
                    palette.buttonText: Theme.dangerText
                    onClicked: root.app.confirm_permanent_delete()
                }
            }
        }

        onOpened: keep.forceActiveFocus()
    }

    // ---- Name conflict -------------------------------------------------------
    Dialog {
        id: conflict
        visible: root.app.op_state === "conflict"
        parent: Overlay.overlay
        anchors.centerIn: parent
        modal: true
        closePolicy: Popup.NoAutoClose
        title: qsTr("Ya existe un elemento con ese nombre")
        width: 460

        contentItem: ColumnLayout {
            spacing: 10
            // Esc is «Omitir»: it leaves the destination exactly as it was.
            Keys.onEscapePressed: root.app.answer_conflict("skip", false)

            Text {
                text: root.app.op_name
                Layout.fillWidth: true
                elide: Text.ElideMiddle
                color: Theme.text
                font.family: Theme.family
                font.pixelSize: Theme.sizeBase
                font.bold: true
            }
            Text {
                visible: root.app.op_mixed_kinds
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Theme.danger
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
                text: qsTr("Uno es una carpeta y el otro un fichero: casi siempre es un error de destino.")
            }
            Text {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Theme.text
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
                text: qsTr("Entrante: %1\nYa en el destino: %2").arg(root.app.op_incoming).arg(root.app.op_existing)
            }
            CheckBox {
                id: everyOne
                text: qsTr("Hacer esto con todos los conflictos de este tipo")
                checked: false
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: 6
                // Focus starts on the safe one, as with every destructive prompt.
                Button {
                    id: skip
                    text: qsTr("Omitir")
                    focus: true
                    onClicked: root.app.answer_conflict("skip", everyOne.checked)
                }
                Button {
                    text: qsTr("Conservar ambos")
                    onClicked: root.app.answer_conflict("keep_both", everyOne.checked)
                }
                Button {
                    text: qsTr("Combinar")
                    visible: root.app.op_can_merge
                    onClicked: root.app.answer_conflict("merge", everyOne.checked)
                }
                Button {
                    text: qsTr("Reemplazar")
                    onClicked: root.app.answer_conflict("replace", everyOne.checked)
                }
                Item {
                    Layout.fillWidth: true
                }
                Button {
                    text: qsTr("Cancelar")
                    onClicked: root.app.cancel_operation()
                }
            }
        }

        onOpened: {
            everyOne.checked = false;
            skip.forceActiveFocus();
        }
    }

    // ---- Failure on one item -------------------------------------------------
    Dialog {
        id: failure
        visible: root.app.op_state === "failure"
        parent: Overlay.overlay
        anchors.centerIn: parent
        modal: true
        closePolicy: Popup.NoAutoClose
        title: qsTr("No se pudo completar")
        width: 460

        contentItem: ColumnLayout {
            spacing: 10
            Keys.onEscapePressed: root.app.answer_failure("cancel")

            Text {
                text: root.app.op_name
                Layout.fillWidth: true
                elide: Text.ElideMiddle
                color: Theme.text
                font.family: Theme.family
                font.pixelSize: Theme.sizeBase
                font.bold: true
            }
            Text {
                text: root.app.op_reason
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Theme.text
                font.family: Theme.family
                font.pixelSize: Theme.sizeSmall
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: 6
                Button {
                    id: failureSkip
                    text: qsTr("Omitir")
                    focus: true
                    onClicked: root.app.answer_failure("skip")
                }
                Button {
                    text: qsTr("Omitir todos")
                    onClicked: root.app.answer_failure("skip_all")
                }
                Button {
                    text: qsTr("Reintentar")
                    visible: root.app.op_can_retry
                    onClicked: root.app.answer_failure("retry")
                }
                Item {
                    Layout.fillWidth: true
                }
                Button {
                    text: qsTr("Cancelar")
                    onClicked: root.app.answer_failure("cancel")
                }
            }
        }

        onOpened: failureSkip.forceActiveFocus()
    }

    // ---- Closing summary -----------------------------------------------------
    Dialog {
        id: summary
        visible: root.app.op_state === "summary"
        parent: Overlay.overlay
        anchors.centerIn: parent
        modal: true
        closePolicy: Popup.NoAutoClose
        title: qsTr("Resumen de la operación")
        width: 520

        contentItem: ColumnLayout {
            spacing: 10
            Keys.onEscapePressed: root.app.dismiss_summary()

            ScrollView {
                Layout.fillWidth: true
                Layout.preferredHeight: Math.min(220, summaryText.implicitHeight + 8)
                Text {
                    id: summaryText
                    width: 480
                    text: root.app.op_reason
                    wrapMode: Text.Wrap
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeSmall
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
                    onClicked: root.app.dismiss_summary()
                }
            }
        }

        onOpened: close.forceActiveFocus()
    }

    // The dialogs' `visible` is bound to the state, not toggled from signal
    // handlers: a handler that asks «is it open?» is wrong during the opening and
    // closing animations, and a question answered faster than that (a quick Esc,
    // then the same shortcut again) left a modal up with nothing to close it, or
    // a prompt pending with no dialog.
    onPromptOpenChanged: {
        if (!root.promptOpen)
            root.released();
    }
}
