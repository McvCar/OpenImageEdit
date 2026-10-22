import QtQuick
import com.imageedit

Rectangle {
    id: zoomMenu
    required property var host
    required property var bridge
    required property var statusBar
    required property var view
    required property var pctBtn
    visible: false
    z: 220
    width: 200
    height: zoomMenuCol.height + 12
    color: "#2a2d31"
    border.color: "#3c4043"
    radius: 6
    y: statusBar.y - height - 4
    onVisibleChanged: if (visible) host.placePopover(zoomMenu, pctBtn)

    Column {
        id: zoomMenuCol
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.topMargin: 6
        spacing: 0

        Repeater {
            model: [25, 50, 100, 200, 400, 800]

            delegate: Item {
                required property int modelData
                width: parent.width
                height: 30

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 2
                    radius: 4
                    color: Math.abs(bridge.zoom_percent - modelData) < 1
                           ? "#3d5a80"
                           : zoomItemMa.containsMouse ? "#3a3d42" : "transparent"
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    text: modelData + "%"
                    color: "#d5d8dd"
                    font.pixelSize: 12
                }
                MouseArea {
                    id: zoomItemMa
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        view.queueZoomTo(modelData)
                        zoomMenu.visible = false
                    }
                }
            }
        }

        Item { width: parent.width; height: 7
            Rectangle { anchors.centerIn: parent; width: parent.width - 16; height: 1; color: "#3c4043" }
        }

        Repeater {
            model: [
                { l: "适应窗口 (Ctrl+9)", run: function () { bridge.zoom_fit() } },
                { l: "100% 实际大小 (Ctrl+0)", run: function () { bridge.zoom_actual() } }
            ]

            delegate: Item {
                required property var modelData
                width: parent.width
                height: 30

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 2
                    radius: 4
                    color: zoomItemMa2.containsMouse ? "#3a3d42" : "transparent"
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    text: modelData.l
                    color: "#d5d8dd"
                    font.pixelSize: 12
                }
                MouseArea {
                    id: zoomItemMa2
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        modelData.run()
                        zoomMenu.visible = false
                    }
                }
            }
        }
    }
}
