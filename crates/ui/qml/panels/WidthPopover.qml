import QtQuick
import com.imageedit

// 粗细弹窗：8 档，选中档高亮。
Rectangle {
    id: widthPopover
    required property var host
    required property var toolbar
    required property var markBar
    visible: false
    z: 220
    width: 168
    height: 8 * 30 + 12
    color: "#2a2d31"
    border.color: "#3c4043"
    radius: 6
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    onVisibleChanged: if (visible) host.placePopover(widthPopover, markBar.widthBtn)

    Column {
        anchors.fill: parent
        anchors.margins: 6

        Repeater {
            model: [2, 3, 4, 6, 8, 12, 16, 24]

            delegate: Item {
                required property int modelData
                width: parent.width
                height: 30

                Rectangle {
                    anchors.fill: parent
                    radius: 4
                    color: host.markStroke === modelData ? "#3d5a80"
                           : widthHover.containsMouse ? "#3a3d42" : "transparent"
                }
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    width: 90
                    height: modelData
                    radius: modelData / 2
                    color: "#d5d8dd"
                }
                Text {
                    visible: host.markStroke === modelData
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: 12
                    text: "✓"
                    color: "#ffffff"
                    font.pixelSize: 13
                }
                MouseArea {
                    id: widthHover
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        host.markStroke = modelData
                        host.applyMarkStyle()
                        widthPopover.visible = false
                    }
                }
            }
        }
    }
}

