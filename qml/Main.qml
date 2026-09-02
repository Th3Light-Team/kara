pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Window
import QtQuick.Layouts
import com.kara.ui

Window {
    width: 1100
    height: 700
    visible: true
    title: qsTr("Kara")

    App { id: app }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 10
        spacing: 10

        // Path display
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 40
            color: "#f9f9f9"
            border.color: "#dddddd"
            border.width: 1

            RowLayout {
                anchors.fill: parent
                anchors.margins: 5
                spacing: 5

                Text {
                    text: "Path:"
                    font.bold: true
                }
                Text {
                    text: app.path
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                    font.family: "monospace"
                }
            }
        }

        // List view
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            border.color: "#cccccc"
            border.width: 1

            ListView {
                anchors.fill: parent
                clip: true
                model: app.entry_names.length

                delegate: Rectangle {
                    width: listView.width
                    height: 30
                    color: index % 2 === 0 ? "#f5f5f5" : "white"

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 10
                        anchors.rightMargin: 10
                        spacing: 10

                        Text {
                            text: app.entry_names[index]
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }
                        Text {
                            text: app.entry_sizes[index]
                            Layout.preferredWidth: 80
                            horizontalAlignment: Text.AlignRight
                            color: "#666666"
                        }
                        Text {
                            text: app.entry_kinds[index]
                            Layout.preferredWidth: 60
                            color: "#666666"
                        }
                    }
                }

                id: listView
            }
        }

        // Status bar
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 30
            color: "#f0f0f0"
            border.color: "#cccccc"
            border.width: 1

            Text {
                anchors.leftMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                text: app.entry_count + " item" + (app.entry_count !== 1 ? "s" : "")
                font.pixelSize: 12
            }
        }
    }
}
