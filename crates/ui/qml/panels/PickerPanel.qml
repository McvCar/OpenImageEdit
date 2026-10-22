import QtQuick
import com.imageedit

Item {
    id: pickerPanel
    required property var host
    required property var bridge
    required property var toolbar
    required property var markBar
    required property var pickerBtn
    visible: bridge.pickerOpen
    width: 316
    readonly property real maxHeight: Math.max(220, host.height - toolbar.height - 24)
    readonly property real anchorX: pickerBtn.x + pickerBtn.parent.x
    height: Math.min(pickerCol.implicitHeight + 24, maxHeight)
    x: 8
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    z: 221
    property bool userMoved: false
    onVisibleChanged: {
        if (visible) {
            userMoved = false
            host.placePopover(pickerPanel, pickerBtn)
        }
    }
    onAnchorXChanged: if (visible && !userMoved) host.placePopover(pickerPanel, pickerBtn)
    onHeightChanged: if (visible && userMoved) moveTo(x, y)

    function moveTo(nx, ny) {
        userMoved = true
        x = Math.max(4, Math.min(Math.max(4, host.width - width - 4), nx))
        const minY = toolbar.height + 2
        const maxY = Math.max(minY, host.height - height - 8)
        y = Math.max(minY, Math.min(maxY, ny))
    }

    Rectangle {
        anchors.fill: parent
        radius: 8
        color: "#2b2e33"
        border.color: "#3c4043"
        border.width: 1
    }

    Flickable {
        id: pickerFlick
        anchors.fill: parent
        anchors.margins: 12
        contentWidth: width
        contentHeight: pickerCol.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height

        Column {
            id: pickerCol
            width: pickerFlick.width
            spacing: 8

            // 标题栏：拖动区先声明，✕ 后声明才能点得到。
            Item {
                id: pickerTitleBar
                width: parent.width
                height: 24

                MouseArea {
                    id: pickerDragArea
                    anchors.fill: parent
                    anchors.leftMargin: -10
                    anchors.rightMargin: -2
                    cursorShape: Qt.SizeAllCursor
                    property real pressX: 0
                    property real pressY: 0
                    property real fromX: 0
                    property real fromY: 0
                    onPressed: (mouse) => {
                        const p = mapToItem(host, mouse.x, mouse.y)
                        pressX = p.x
                        pressY = p.y
                        fromX = pickerPanel.x
                        fromY = pickerPanel.y
                    }
                    onPositionChanged: (mouse) => {
                        if (!pressed)
                            return
                        const p = mapToItem(host, mouse.x, mouse.y)
                        pickerPanel.moveTo(fromX + (p.x - pressX),
                                           fromY + (p.y - pressY))
                    }
                }

                Text {
                    id: pickerTitle
                    anchors.verticalCenter: parent.verticalCenter
                    text: "拾色器"
                    color: "#e9eaec"
                    font.pixelSize: 13
                    font.bold: true
                }
                Text {
                    anchors.left: pickerTitle.right
                    anchors.leftMargin: 8
                    anchors.right: pickerClose.left
                    anchors.rightMargin: 6
                    anchors.verticalCenter: parent.verticalCenter
                    text: host.pickerVals ? "已取样" : "点击图片取色"
                    color: "#8f96a3"
                    font.pixelSize: 10
                    elide: Text.ElideRight
                }
                IconButton {
                    id: pickerClose
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    glyph: "✕"
                    tip: "关闭拾色器（Esc）"
                    onClicked: bridge.picker_close()
                }
            }

            // ---- 当前颜色：大色块 + HEX + 坐标 ----
            Row {
                spacing: 10

                Rectangle {
                    width: 56
                    height: 56
                    radius: 5
                    color: host.pickerVals ? host.pickerVals.hex : "#1b1c1e"
                    border.color: "#5a6069"
                    border.width: 1
                }

                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 3

                    Text {
                        text: host.pickerVals ? host.pickerVals.hex : "—"
                        color: "#f2f3f5"
                        font.pixelSize: 20
                        font.family: "Consolas"
                        font.bold: true
                    }
                    Text {
                        text: host.pickerVals
                              ? "像素 (" + host.pickerVals.x + ", "
                                + host.pickerVals.y + ")"
                                + (host.pickerVals.a < 255
                                   ? "　不透明度 "
                                     + Math.round(host.pickerVals.a / 255 * 100) + "%"
                                   : "")
                              : "在图片上点一下取色"
                        color: "#8f96a3"
                        font.pixelSize: 10
                    }
                }
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 四种色值表示，每行右侧一个复制按钮 ----
            Repeater {
                model: host.pickerSpecs
                delegate: Item {
                    required property var modelData
                    width: pickerCol.width
                    height: 26

                    Text {
                        x: 0
                        width: 44
                        anchors.verticalCenter: parent.verticalCenter
                        text: modelData.k
                        color: "#8f96a3"
                        font.pixelSize: 11
                    }
                    Text {
                        x: 48
                        width: parent.width - 48 - 66
                        anchors.verticalCenter: parent.verticalCenter
                        text: modelData.v
                        color: "#e9eaec"
                        font.pixelSize: 12
                        font.family: "Consolas"
                        elide: Text.ElideRight
                    }
                    ToolButton {
                        width: 58
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        label: host.pickerCopied === modelData.k ? "已复制" : "复制"
                        selected: host.pickerCopied === modelData.k
                        onClicked: host.copyPickerValue(modelData.k, modelData.v)
                    }
                }
            }

            // ---- 最近取色（点击切换当前色，便于反复复制）----
            Item {
                width: parent.width
                height: visible ? pickerRecentLabel.height + 4 + 24 + 4 : 0
                visible: host.pickerHist.length > 0

                Text {
                    id: pickerRecentLabel
                    text: "最近取色（点击切换）"
                    color: "#8f96a3"
                    font.pixelSize: 10
                }
                Text {
                    anchors.right: parent.right
                    anchors.top: parent.top
                    text: "清空"
                    color: "#8f96a3"
                    font.pixelSize: 10
                    MouseArea {
                        anchors.fill: parent
                        anchors.margins: -3
                        cursorShape: Qt.PointingHandCursor
                        onClicked: bridge.picker_clear()
                    }
                }
                Row {
                    anchors.top: pickerRecentLabel.bottom
                    anchors.topMargin: 4
                    spacing: 4

                    Repeater {
                        model: host.pickerHist

                        delegate: Rectangle {
                            required property var modelData
                            required property int index
                            width: 22
                            height: 22
                            radius: 3
                            color: modelData.hex
                            border.color: host.pickerVals
                                          && host.pickerVals.hex === modelData.hex
                                          ? "#6aa9ff" : "#5a6069"
                            border.width: host.pickerVals
                                          && host.pickerVals.hex === modelData.hex ? 2 : 1
                            MouseArea {
                                anchors.fill: parent
                                cursorShape: Qt.PointingHandCursor
                                onClicked: bridge.picker_select(index)
                            }
                        }
                    }
                }
            }

            Text {
                width: parent.width
                text: "点击图片取色 · Esc 关闭 · 拖动标题栏可移动\n"
                      + "CMYK 为无色彩配置的近似换算"
                color: "#6b7076"
                font.pixelSize: 10
                wrapMode: Text.WordWrap
            }
        }
    }

    // 「已复制」提示的自动复位。
    Timer {
        id: pickerCopiedTimer
        interval: 1200
        onTriggered: host.pickerCopied = ""
    }
}
