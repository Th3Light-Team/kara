pragma ComponentBehavior: Bound

// Register (or edit) a remote drive.
//
// The form is generic: which protocols exist, which fields each one has, which
// are required, which only apply when another says so, and every error message
// come from Rust (`kara_remote::form`). This file lays the fields out by kind
// and forwards what the user types, one key at a time, to `form_set`.
//
// Dialog rules (CLAUDE.md): `visible` is bound to state, Esc is a
// `Keys.onEscapePressed` on the contentItem, and `Main.qml` counts this dialog
// in `ops.promptOpen` so the window's shortcuts stand down.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.kara.ui

Item {
    id: root

    required property var drives
    anchors.fill: parent

    property bool showAdvanced: false

    Dialog {
        id: dialog
        visible: root.drives.dialog_open
        parent: Overlay.overlay
        anchors.centerIn: parent
        modal: true
        width: Math.min(540, parent ? parent.width - 40 : 540)
        height: Math.min(implicitHeight, parent ? parent.height - 40 : 600)
        title: root.drives.form_editing ? qsTr("Editar unidad remota") : qsTr("Añadir unidad remota")
        closePolicy: Popup.NoAutoClose

        contentItem: ColumnLayout {
            spacing: 10
            Keys.onEscapePressed: root.drives.close_dialog()

            ScrollView {
                id: scroll
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredHeight: form.implicitHeight
                clip: true
                contentWidth: availableWidth
                ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

                ColumnLayout {
                    id: form
                    width: scroll.availableWidth
                    spacing: 10
                    enabled: !root.drives.form_busy

                    Text {
                        Layout.fillWidth: true
                        visible: !root.drives.adapters_available
                        text: qsTr("Esta versión de Kara no incluye ningún protocolo de unidades remotas.")
                        wrapMode: Text.WordWrap
                        color: Theme.danger
                        font.family: Theme.family
                        font.pixelSize: Theme.sizeBase
                    }

                    // Protocol
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 3
                        Text {
                            text: qsTr("Protocolo")
                            color: Theme.textDim
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                        ComboBox {
                            Layout.fillWidth: true
                            model: root.drives.scheme_titles
                            currentIndex: root.drives.scheme_index
                            enabled: !root.drives.form_editing && count > 1
                            onActivated: index => root.drives.select_scheme(index)
                        }
                    }

                    // Label
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 3
                        Text {
                            text: qsTr("Nombre") + " *"
                            color: Theme.textDim
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                        TextField {
                            id: labelField
                            Layout.fillWidth: true
                            text: root.drives.form_label
                            placeholderText: qsTr("Cómo se llama en el panel")
                            onTextEdited: root.drives.form_set("label", text)
                            onAccepted: root.drives.form_submit()
                            Keys.onEscapePressed: root.drives.close_dialog()
                        }
                        Text {
                            Layout.fillWidth: true
                            visible: root.drives.form_label_error !== ""
                            text: root.drives.form_label_error
                            wrapMode: Text.WordWrap
                            color: Theme.danger
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                    }

                    // Protocol parameters, from the descriptors
                    Repeater {
                        model: root.drives.form_keys.length

                        delegate: ColumnLayout {
                            id: field

                            required property int index

                            readonly property string key: root.drives.form_keys[field.index] ?? ""
                            readonly property string kind: root.drives.form_kinds[field.index] ?? "text"
                            readonly property string value: root.drives.form_values[field.index] ?? ""
                            readonly property string fallback: root.drives.form_defaults[field.index] ?? ""
                            readonly property string error: root.drives.form_errors[field.index] ?? ""
                            readonly property bool applies: (root.drives.form_visible[field.index] ?? 0) !== 0
                            readonly property bool advanced: (root.drives.form_advanced[field.index] ?? 0) !== 0
                            readonly property bool needed: (root.drives.form_required[field.index] ?? 0) !== 0
                            // value=Label|value=Label
                            readonly property var options: field.kind === "choice" ? (root.drives.form_choices[field.index] ?? "").split("|").map(c => ({
                                        value: c.substring(0, c.indexOf("=")),
                                        label: c.substring(c.indexOf("=") + 1)
                                    })) : []

                            Layout.fillWidth: true
                            visible: field.applies && (!field.advanced || root.showAdvanced)
                            spacing: 3

                            Text {
                                text: (root.drives.form_labels[field.index] ?? "") + (field.needed ? " *" : "")
                                visible: field.kind !== "toggle"
                                color: Theme.textDim
                                font.family: Theme.family
                                font.pixelSize: Theme.sizeSmall
                            }

                            TextField {
                                Layout.fillWidth: true
                                visible: field.kind === "text" || field.kind === "number" || field.kind === "path"
                                text: field.value
                                placeholderText: field.fallback
                                inputMethodHints: field.kind === "number" ? Qt.ImhDigitsOnly : Qt.ImhNoPredictiveText
                                onTextEdited: root.drives.form_set(field.key, text)
                                onAccepted: root.drives.form_submit()
                                Keys.onEscapePressed: root.drives.close_dialog()
                            }

                            ComboBox {
                                Layout.fillWidth: true
                                visible: field.kind === "choice"
                                model: field.options.map(o => o.label)
                                currentIndex: {
                                    const current = field.value === "" ? field.fallback : field.value;
                                    return Math.max(0, field.options.findIndex(o => o.value === current));
                                }
                                onActivated: index => root.drives.form_set(field.key, field.options[index].value)
                            }

                            CheckBox {
                                visible: field.kind === "toggle"
                                text: root.drives.form_labels[field.index] ?? ""
                                checked: field.value === "true"
                                onToggled: root.drives.form_set(field.key, checked ? "true" : "false")
                            }

                            Text {
                                Layout.fillWidth: true
                                visible: field.error === "" && (root.drives.form_hints[field.index] ?? "") !== ""
                                text: root.drives.form_hints[field.index] ?? ""
                                wrapMode: Text.WordWrap
                                color: Theme.textDim
                                font.family: Theme.family
                                font.pixelSize: Theme.sizeSmall
                            }
                            Text {
                                Layout.fillWidth: true
                                visible: field.error !== ""
                                text: field.error
                                wrapMode: Text.WordWrap
                                color: Theme.danger
                                font.family: Theme.family
                                font.pixelSize: Theme.sizeSmall
                            }
                        }
                    }

                    // Secret
                    ColumnLayout {
                        Layout.fillWidth: true
                        visible: root.drives.form_secret_visible
                        spacing: 3
                        Text {
                            text: root.drives.form_secret_label
                            color: Theme.textDim
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                        TextField {
                            id: secretField
                            Layout.fillWidth: true
                            echoMode: TextInput.Password
                            placeholderText: root.drives.form_editing ? qsTr("Vacío: se mantiene la guardada") : ""
                            onTextEdited: root.drives.form_set("secret", text)
                            onAccepted: root.drives.form_submit()
                            Keys.onEscapePressed: root.drives.close_dialog()
                            // Never kept in the form once it is closed.
                            Connections {
                                target: root.drives
                                function onDialog_openChanged() {
                                    if (!root.drives.dialog_open)
                                        secretField.text = "";
                                }
                            }
                        }
                        Text {
                            Layout.fillWidth: true
                            text: root.drives.form_secret_hint
                            wrapMode: Text.WordWrap
                            color: Theme.textDim
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                        CheckBox {
                            text: qsTr("Recordar en el llavero")
                            checked: root.drives.form_remember
                            onToggled: root.drives.form_set_remember(checked)
                        }
                        Text {
                            Layout.fillWidth: true
                            visible: root.drives.secrets_note !== "" && root.drives.form_remember
                            text: root.drives.secrets_note
                            wrapMode: Text.WordWrap
                            color: Theme.danger
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                    }

                    // Group and identifier
                    Button {
                        flat: true
                        text: root.showAdvanced ? qsTr("Ocultar opciones avanzadas") : qsTr("Opciones avanzadas")
                        onClicked: root.showAdvanced = !root.showAdvanced
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        visible: root.showAdvanced
                        spacing: 3
                        Text {
                            text: qsTr("Grupo")
                            color: Theme.textDim
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                        TextField {
                            Layout.fillWidth: true
                            text: root.drives.form_group
                            placeholderText: qsTr("Opcional: las unidades de un grupo se listan juntas")
                            onTextEdited: root.drives.form_set("group", text)
                            Keys.onEscapePressed: root.drives.close_dialog()
                        }
                        Text {
                            Layout.fillWidth: true
                            visible: root.drives.form_group_error !== ""
                            text: root.drives.form_group_error
                            color: Theme.danger
                            wrapMode: Text.WordWrap
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        visible: root.showAdvanced || root.drives.form_name_error !== ""
                        spacing: 3
                        Text {
                            text: qsTr("Identificador")
                            color: Theme.textDim
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                        TextField {
                            Layout.fillWidth: true
                            text: root.drives.form_name
                            enabled: !root.drives.form_editing
                            placeholderText: qsTr("Vacío: se deduce del nombre (kara+sftp://identificador/…)")
                            onTextEdited: root.drives.form_set("name", text)
                            Keys.onEscapePressed: root.drives.close_dialog()
                        }
                        Text {
                            Layout.fillWidth: true
                            visible: root.drives.form_name_error !== ""
                            text: root.drives.form_name_error
                            color: Theme.danger
                            wrapMode: Text.WordWrap
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeSmall
                        }
                    }

                    Text {
                        Layout.fillWidth: true
                        visible: root.drives.form_general_error !== ""
                        text: root.drives.form_general_error
                        color: Theme.danger
                        wrapMode: Text.WordWrap
                        font.family: Theme.family
                        font.pixelSize: Theme.sizeBase
                    }
                }
            }

            // Result of «Probar conexión»
            RowLayout {
                Layout.fillWidth: true
                visible: root.drives.test_state !== ""
                spacing: 8
                BusyIndicator {
                    running: root.drives.test_state === "running"
                    visible: running
                    Layout.preferredWidth: 20
                    Layout.preferredHeight: 20
                }
                Text {
                    Layout.fillWidth: true
                    text: root.drives.test_text
                    wrapMode: Text.WordWrap
                    color: root.drives.test_state === "ok" ? Theme.success : root.drives.test_state === "error" ? Theme.danger : Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.sizeBase
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Button {
                    text: qsTr("Probar conexión")
                    enabled: root.drives.adapters_available && !root.drives.form_busy
                    onClicked: root.drives.form_test()
                }
                Item {
                    Layout.fillWidth: true
                }
                Button {
                    text: qsTr("Cancelar")
                    onClicked: root.drives.close_dialog()
                }
                Button {
                    text: root.drives.form_editing ? qsTr("Guardar") : qsTr("Añadir")
                    highlighted: true
                    enabled: root.drives.adapters_available && !root.drives.form_busy
                    onClicked: root.drives.form_submit()
                }
            }
        }

        onOpened: {
            root.showAdvanced = false;
            labelField.forceActiveFocus();
        }
    }
}
