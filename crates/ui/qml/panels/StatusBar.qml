import QtQuick
import com.imageedit

Rectangle {
    id: statusBar
    required property var host
    required property var bridge
    anchors.bottom: parent.bottom
    anchors.left: parent.left
    anchors.right: parent.right
    height: host.statusHeight
    color: "#242628"

    Text {
        id: modeHint
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: zoomCtl.left
        anchors.rightMargin: 10
        visible: host.modeActive || host.selExists || bridge.selected_layer >= 0
        text: host.pasteFloating
              ? "粘贴内容可拖动定位：Enter / 点击外部落定，Esc 撤销粘贴"
              : host.selRubberBanding
              ? "松手完成选区，随后可拖手柄精修、拖内部移动"
              : host.scaleW > 0
              ? "拖角/边调整图层大小（角点等比，Shift 自由）；松手应用，Esc 取消"
              : host.selExists
              ? "已选区：拖手柄缩放 / 拖内部移动；Enter 或 Ctrl+K 应用裁剪，Esc 取消"
              : "已选图层：点住拖动移动，拖手柄缩放；点击空白处或 Esc 取消选择"
        color: "#9fb6d8"
        font.pixelSize: 12
    }

    // File name / resolution / size. When a real file is loaded this is
    // a button: click copies the full path (hidden TextEdit.copy()
    // helper), hover shows a hint tooltip.
    Item {
        id: fileInfo
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: parent.left
        anchors.leftMargin: 10
        height: parent.height
        visible: bridge.fileName.length > 0
        width: fileInfoText.implicitWidth
        readonly property bool hasPath: bridge.sourcePath.length > 0
        property bool hovering: false
        property bool copied: false

        Text {
            id: fileInfoText
            anchors.verticalCenter: parent.verticalCenter
            text: fileInfo.copied
                  ? "已复制完整路径 ✓"
                  : bridge.fileName
                    + (bridge.image_width > 0
                       ? "   ·   " + bridge.image_width + "×" + bridge.image_height
                       : "")
                    + (bridge.fileSizeStr.length > 0 ? "   ·   " + bridge.fileSizeStr : "")
            color: fileInfo.copied ? "#7fc3ff"
                   : (fileInfo.hasPath && fileInfo.hovering ? "#c9dcf5" : "#9aa3b2")
            font.pixelSize: 12
            font.underline: fileInfo.hasPath && fileInfo.hovering && !fileInfo.copied
        }

        // Hand-rolled tooltip (no QtQuick.Controls dependency).
        Rectangle {
            id: fileInfoTip
            visible: false
            z: 1000
            x: 0
            y: parent.height + 4
            width: fileInfoTipText.implicitWidth + 16
            height: 22
            radius: 4
            color: "#111214"
            border.color: "#3c4043"
            Text {
                id: fileInfoTipText
                anchors.centerIn: parent
                text: "点击复制完整路径"
                color: "#d7dade"
                font.pixelSize: 11
            }
        }
        Timer {
            id: fileInfoTipDelay
            interval: 500
            onTriggered: fileInfoTip.visible = fileInfo.hasPath && fileInfo.hovering
        }

        MouseArea {
            id: fileInfoMa
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: fileInfo.hasPath ? Qt.PointingHandCursor : Qt.ArrowCursor
            onContainsMouseChanged: {
                if (containsMouse) {
                    fileInfo.hovering = true
                    if (fileInfo.hasPath && !fileInfo.copied)
                        fileInfoTipDelay.restart()
                    else
                        fileInfoTip.visible = false
                } else {
                    fileInfo.hovering = false
                    fileInfoTip.visible = false
                    fileInfoTipDelay.stop()
                }
            }
            onClicked: {
                if (!fileInfo.hasPath)
                    return
                host.copyToClipboard(bridge.sourcePath)
                fileInfo.copied = true
                fileInfoTip.visible = false
                fileInfoCopiedReset.restart()
            }
        }
        Timer {
            id: fileInfoCopiedReset
            interval: 2000
            onTriggered: fileInfo.copied = false
        }
    }

    Text {
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: fileInfo.visible ? fileInfo.right : parent.left
        anchors.leftMargin: fileInfo.visible ? 12 : 10
        anchors.right: (host.modeActive || host.selExists) ? modeHint.left : zoomCtl.left
        anchors.rightMargin: 10
        elide: Text.ElideMiddle
        text: bridge.status
        color: "#b8bcc0"
        font.pixelSize: 12
    }

    // 可操作缩放控件：百分比下拉（预设/适应窗口/实际大小）+ 滑杆 + 加减。
    Row {
        id: zoomCtl
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        anchors.rightMargin: 10
        spacing: 6
        visible: bridge.image_width > 0

        // 百分比胶囊按钮 → 弹预设菜单。
        Rectangle {
            id: zoomPctBtn
            width: zoomPctRow.implicitWidth + 18
            height: 20
            anchors.verticalCenter: parent.verticalCenter
            radius: 4
            color: zoomPctMa.containsMouse || host.zoomMenu.visible
                   ? "#3a3d42" : "#2f3236"
            border.color: "#4a4e54"
            border.width: 1

            Row {
                id: zoomPctRow
                anchors.centerIn: parent
                spacing: 4
                Text {
                    text: bridge.zoom_percent + "%"
                    color: "#d5d8dd"
                    font.pixelSize: 11
                }
                Text {
                    text: "▾"
                    color: "#9aa0a6"
                    font.pixelSize: 9
                    anchors.verticalCenter: parent.verticalCenter
                }
            }

            MouseArea {
                id: zoomPctMa
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    if (host.zoomMenu.visible)
                        host.zoomMenu.visible = false
                    else {
                        host.placePopover(host.zoomMenu, zoomPctBtn)
                        host.zoomMenu.visible = true
                    }
                }
            }
        }

        // 滑杆：对数刻度 2%..1600%，拖动与滚轮共用同一条节流管线。
        Item {
            id: zoomSlider
            width: 96
            height: 20
            anchors.verticalCenter: parent.verticalCenter

            property real value: 0.5
            property bool dragging: false

            function zoomToValue(pct) {
                const z = Math.min(1600, Math.max(2, pct))
                return Math.log(z / 2) / Math.log(800)
            }
            function valueToZoom(v) {
                return Math.round(2 * Math.pow(800, v))
            }
            function setValue(x) {
                value = Math.min(1, Math.max(0, x / width))
                host.view.queueZoomTo(valueToZoom(value))
            }

            Binding {
                target: zoomSlider
                property: "value"
                value: zoomSlider.zoomToValue(bridge.zoom_percent)
                // 等节流定时器把 zoom 落到 bridge 后再恢复跟随，否则
                // 松手瞬间会先弹回旧值、120ms 后又跳到新值。
                when: !zoomSlider.dragging && !host.view.zoomPending
            }

            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width
                height: 4
                radius: 2
                color: "#3c4043"
            }
            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: zoomSlider.value * zoomSlider.width
                height: 4
                radius: 2
                color: "#5b9dff"
            }
            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                x: zoomSlider.value * (zoomSlider.width - 10)
                width: 10
                height: 10
                radius: 5
                color: zoomMa.containsMouse || zoomSlider.dragging
                       ? "#7fc3ff" : "#e9eaec"
            }

            MouseArea {
                id: zoomMa
                anchors.fill: parent
                anchors.margins: -6
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onPressed: (mouse) => {
                    zoomSlider.dragging = true
                    zoomSlider.setValue(mouse.x + 6)
                }
                onPositionChanged: (mouse) => {
                    if (zoomSlider.dragging)
                        zoomSlider.setValue(mouse.x + 6)
                }
                onReleased: zoomSlider.dragging = false
            }
        }

        // − / + 按钮：与 Ctrl+滚轮同步长（×1.25）。
        Rectangle {
            width: 18
            height: 18
            anchors.verticalCenter: parent.verticalCenter
            radius: 9
            color: zoomMinusMa.containsMouse ? "#3a3d42" : "transparent"
            border.color: "#4a4e54"
            border.width: 1
            Text {
                anchors.centerIn: parent
                text: "−"
                color: "#d5d8dd"
                font.pixelSize: 12
            }
            MouseArea {
                id: zoomMinusMa
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: host.view.queueZoom(1 / 1.25)
            }
        }
        Rectangle {
            width: 18
            height: 18
            anchors.verticalCenter: parent.verticalCenter
            radius: 9
            color: zoomPlusMa.containsMouse ? "#3a3d42" : "transparent"
            border.color: "#4a4e54"
            border.width: 1
            Text {
                anchors.centerIn: parent
                text: "+"
                color: "#d5d8dd"
                font.pixelSize: 12
            }
            MouseArea {
                id: zoomPlusMa
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: host.view.queueZoom(1.25)
            }
        }
    }
}
