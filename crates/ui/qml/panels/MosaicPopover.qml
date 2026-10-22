import QtQuick
import com.imageedit

// 马赛克强度弹窗：5 档块尺寸，选中档高亮。
Rectangle {
    id: mosaicPopover
    required property var host
    required property var bridge
    required property var toolbar
    required property var markBar
    visible: false
    z: 220
    width: 190
    height: 5 * 30 + 12
    color: "#2a2d31"
    border.color: "#3c4043"
    radius: 6
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    onVisibleChanged: if (visible) host.placePopover(mosaicPopover, markBar.mosaicBtn)

    Column {
        anchors.fill: parent
        anchors.margins: 6

        Repeater {
            model: [
                { cell: 4,  label: "细腻" },
                { cell: 8,  label: "较细" },
                { cell: 12, label: "适中" },
                { cell: 18, label: "较粗" },
                { cell: 26, label: "强烈" }
            ]

            delegate: Item {
                required property var modelData
                width: parent.width
                height: 30

                Rectangle {
                    anchors.fill: parent
                    radius: 4
                    color: host.markMosaic === modelData.cell ? "#3d5a80"
                           : mosaicHover.containsMouse ? "#3a3d42" : "transparent"
                }
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 12
                    width: modelData.cell * 1.4 + 8
                    height: modelData.cell * 1.4 + 8
                    radius: 2
                    color: "#7a7f86"
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 70
                    text: modelData.label + "（" + modelData.cell + "px）"
                    color: "#d5d8dd"
                    font.pixelSize: 12
                }
                Text {
                    visible: host.markMosaic === modelData.cell
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: 12
                    text: "✓"
                    color: "#ffffff"
                    font.pixelSize: 13
                }
                MouseArea {
                    id: mosaicHover
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        host.markMosaic = modelData.cell
                        // 选中标记是马赛克时同步改它的强度。
                        if (bridge.selected_mark >= 0)
                            bridge.update_mark_cell(host.markMosaic)
                        mosaicPopover.visible = false
                    }
                }
            }
        }
    }
}

