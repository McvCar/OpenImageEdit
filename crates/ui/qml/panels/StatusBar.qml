import QtQuick
import com.imageedit

Rectangle {
    id: statusBar
    required property var host
    required property var bridge
    required property var view
    required property var zoomMenu
    // 供缩放菜单（ZoomMenu）重新锚定到百分比胶囊按钮。
    property var pctBtnRef: zoomPctBtn
    anchors.bottom: parent.bottom
    anchors.left: parent.left
    anchors.right: parent.right
    height: host.statusHeight
    color: "#242628"

    // ---- 三块文字的排版 ------------------------------------------------
    // 右端缩放控件是固定宽度，左边到它之间才是文字区。原来三块各自用
    // implicitWidth 向外撑（提示文字右锚、文件名左锚），窗口一窄就互相
    // 压过去：文件名盖住缩放控件、状态提示又盖住文件名。现在统一放进
    // leftZone，按空间依次退让：先砍提示 → 再砍状态 → 最后压文件名。
    readonly property real zoomReserve: zoomCtl.width + 20

    Item {
        id: leftZone
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.right: parent.right
        anchors.rightMargin: zoomReserve
        anchors.verticalCenter: parent.verticalCenter
        height: parent.height
        clip: true

        // 提示文字整条（操作说明，信息密度最低）只在够宽时出现。
        readonly property bool showHint: width >= 520
        // 文件名与缩放控件之间至少留 90px 才值得显示状态文字。
        readonly property bool showStatus:
            width - (fileInfo.visible ? fileInfo.width + 12 : 0)
            - (modeHint.visible ? modeHint.width + 10 : 0) >= 90
    }

    Text {
        id: modeHint
        parent: leftZone
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        // 上限占一半，剩下的留给文件名和状态文字；超长右对齐截断。
        width: Math.min(implicitWidth, Math.max(0, leftZone.width * 0.52))
        horizontalAlignment: Text.AlignRight
        elide: Text.ElideRight
        visible: leftZone.showHint
                 && (host.modeActive || host.selExists || bridge.selected_layer >= 0)
        text: host.pasteFloating
              ? "粘贴内容可拖动定位：Enter / 点击外部落定，Esc 撤销粘贴"
              : host.selRubberBanding
              ? "松手完成选区（Shift 加选 / Alt 减选 / Shift+Alt 交集）"
              // 缩放中只报图层当前宽高，不再挂操作说明（跟着鼠标跳的那条
              // 尺寸标签已经够看了，说明文字属于噪音）。
              : host.scaleW > 0
              ? "图层 " + Math.round(Math.abs(host.scaleW) / Math.max(host.toDispScale(), 0.0001))
                       + " × "
                       + Math.round(Math.abs(host.scaleH) / Math.max(host.toDispScale(), 0.0001))
              : host.selExists
              ? "已选区：拖选区内可移动，右键出边界/平滑/羽化；Enter 裁剪，Ctrl+D 取消"
              : "已选图层：点住拖动移动，拖手柄缩放；点击空白处或 Esc 取消选择"
        color: "#9fb6d8"
        font.pixelSize: 12
    }

    // File name / resolution / size. When a real file is loaded this is
    // a button: click copies the full path (hidden TextEdit.copy()
    // helper), hover shows a hint tooltip.
    Item {
        id: fileInfo
        parent: leftZone
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: parent.left
        height: parent.height
        // 文件名 + 分辨率各占一段：窄窗口先丢掉体积、再丢掉分辨率，最后才
        // 靠 elide 截文件名。整串一起 elide 会切在数字中间（"800×600" 变
        // "…0"），比不显示还难读。
        visible: bridge.fileName.length > 0 && leftZone.width >= 150
        width: Math.min(fileInfoName.implicitWidth
                        + (fileInfoMeta.visible ? fileInfoMeta.implicitWidth + 10 : 0),
                        Math.max(72, leftZone.width * 0.45))
        readonly property bool hasPath: bridge.sourcePath.length > 0
        readonly property string metaText:
            bridge.image_width > 0
            ? bridge.image_width + "×" + bridge.image_height
              + (leftZone.width >= 400 && bridge.fileSizeStr.length > 0
                 ? "  ·  " + bridge.fileSizeStr
                 : "")
            : ""
        property bool hovering: false
        property bool copied: false

        Text {
            id: fileInfoName
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left
            width: Math.max(0, fileInfo.width
                               - (fileInfoMeta.visible
                                  ? fileInfoMeta.implicitWidth + 10 : 0))
            elide: Text.ElideMiddle
            text: fileInfo.copied ? "已复制完整路径 ✓" : bridge.fileName
            color: fileInfo.copied ? "#7fc3ff"
                   : (fileInfo.hasPath && fileInfo.hovering ? "#c9dcf5" : "#9aa3b2")
            font.pixelSize: 12
            font.underline: fileInfo.hasPath && fileInfo.hovering && !fileInfo.copied
        }

        Text {
            id: fileInfoMeta
            anchors.verticalCenter: parent.verticalCenter
            anchors.right: parent.right
            visible: !fileInfo.copied && fileInfo.metaText.length > 0
                     && leftZone.width >= 300
            text: fileInfo.metaText
            color: fileInfo.hasPath && fileInfo.hovering ? "#c9dcf5" : "#8c95a3"
            font.pixelSize: 12
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

    // 中间的状态文字：夹在文件名与提示文字之间，两边都空出来时铺满。
    Text {
        id: statusText
        parent: leftZone
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: fileInfo.visible ? fileInfo.right : parent.left
        anchors.leftMargin: fileInfo.visible ? 12 : 0
        anchors.right: modeHint.visible ? modeHint.left : parent.right
        anchors.rightMargin: modeHint.visible ? 10 : 0
        visible: leftZone.showStatus && text.length > 0
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
            color: zoomPctMa.containsMouse || zoomMenu.visible
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
                    if (zoomMenu.visible)
                        zoomMenu.visible = false
                    else {
                        host.placePopover(zoomMenu, zoomPctBtn)
                        zoomMenu.visible = true
                    }
                }
            }
        }

        // 滑杆：对数刻度 2%..1600%，拖动与滚轮共用同一条节流管线。
        // 窄窗口下先把它收起来（百分比胶囊和 −/+ 还能用），省下的 100px
        // 全给左边的文件名/状态文字。
        Item {
            id: zoomSlider
            width: 96
            height: 20
            visible: statusBar.width >= 640
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
                view.queueZoomTo(valueToZoom(value))
            }

            Binding {
                target: zoomSlider
                property: "value"
                value: zoomSlider.zoomToValue(bridge.zoom_percent)
                // 等节流定时器把 zoom 落到 bridge 后再恢复跟随，否则
                // 松手瞬间会先弹回旧值、120ms 后又跳到新值。
                when: !zoomSlider.dragging && !view.zoomPending
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
                onClicked: view.queueZoom(1 / 1.25)
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
                onClicked: view.queueZoom(1.25)
            }
        }
    }
}
