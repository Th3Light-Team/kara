pragma ComponentBehavior: Bound

// «Elegir otra aplicación»: every installed application, a search box, and
// «usar siempre» to make the choice the type's default.
//
// Reference: `ground/spec/06-contexto-power.md`, «Abrir con»: «ofrece 'Elegir
// otra aplicación' para explorar todas las instaladas y, opcionalmente, fijar
// una como predeterminada mediante una casilla 'usar siempre'». The checkbox
// only appears when the selection is of a single type: there is no one
// default to set for a PDF and a photo together.
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
        visible: root.app.chooser_open
        parent: Overlay.overlay
        anchors.centerIn: parent
        modal: true
        width: 440
        height: Math.min(560, (parent ? parent.height : 600) - 80)
        title: root.app.chooser_title
        closePolicy: Popup.NoAutoClose

        /// The application rows that match the search, as indices into the
        /// bridge's lists.
        readonly property var matches: {
            const wanted = search.text.trim().toLowerCase();
            const names = root.app.chooser_names;
            const out = [];
            for (let i = 0; i < names.length; ++i) {
                if (wanted === "" || names[i].toLowerCase().indexOf(wanted) >= 0)
                    out.push(i);
            }
            return out;
        }

        function choose() {
            if (list.currentIndex < 0 || list.currentIndex >= dialog.matches.length)
                return;
            root.app.choose_app(dialog.matches[list.currentIndex], always.visible && always.checked);
        }

        contentItem: ColumnLayout {
            spacing: 8
            Keys.onEscapePressed: root.app.close_chooser()

            TextField {
                id: search
                Layout.fillWidth: true
                placeholderText: qsTr("Buscar una aplicación")
                onTextChanged: list.currentIndex = dialog.matches.length > 0 ? 0 : -1
                Keys.onDownPressed: list.incrementCurrentIndex()
                Keys.onUpPressed: list.decrementCurrentIndex()
                Keys.onReturnPressed: dialog.choose()
                Keys.onEscapePressed: root.app.close_chooser()
            }

            ListView {
                id: list
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: dialog.matches
                currentIndex: -1
                ScrollBar.vertical: ScrollBar {}

                delegate: Rectangle {
                    id: line
                    required property int index
                    required property int modelData
                    readonly property string iconUrl: root.app.chooser_icons[line.modelData] ?? ""
                    readonly property string name: root.app.chooser_names[line.modelData] ?? ""

                    width: list.width
                    height: 34
                    radius: Theme.radius
                    color: list.currentIndex === line.index ? Theme.selection : (area.containsMouse ? Theme.hover : "transparent")

                    Row {
                        anchors.left: parent.left
                        anchors.leftMargin: 8
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 8

                        Image {
                            width: 20
                            height: 20
                            anchors.verticalCenter: parent.verticalCenter
                            source: line.iconUrl
                            sourceSize.width: 20
                            sourceSize.height: 20
                            visible: status === Image.Ready
                            asynchronous: true
                        }
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: line.name
                            color: Theme.text
                            font.family: Theme.family
                            font.pixelSize: Theme.sizeBase
                        }
                    }

                    MouseArea {
                        id: area
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: list.currentIndex = line.index
                        onDoubleClicked: {
                            list.currentIndex = line.index;
                            dialog.choose();
                        }
                    }
                }
            }

            CheckBox {
                id: always
                visible: root.app.chooser_kind !== ""
                text: qsTr("Usar siempre para «%1»").arg(root.app.chooser_kind)
            }

            RowLayout {
                Layout.fillWidth: true
                Item {
                    Layout.fillWidth: true
                }
                Button {
                    text: qsTr("Cancelar")
                    onClicked: root.app.close_chooser()
                }
                Button {
                    text: qsTr("Abrir")
                    highlighted: true
                    enabled: list.currentIndex >= 0
                    onClicked: dialog.choose()
                }
            }
        }

        onOpened: {
            search.text = "";
            always.checked = false;
            list.currentIndex = -1;
            search.forceActiveFocus();
        }
    }
}
