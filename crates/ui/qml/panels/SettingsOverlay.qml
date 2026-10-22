import QtQuick
import com.imageedit

// Settings panel: transparency checkerboard colors, "open file" behavior,
// and default-viewer registration. Every change persists immediately
// (%APPDATA%/ImageEdit/settings.ini via the Bridge).
Rectangle {
    id: settingsOverlay
    required property var bridge
    anchors.fill: parent
    visible: false
    z: 200
    color: "#88000000"
    onVisibleChanged: if (!visible) {
        hexLight.focus = false
        hexDark.focus = false
    }

    // Blocks clicks on the dimmed background; the panel closes via the
    // ✕ button or Esc only (click-away closing felt accidental).
    MouseArea {
        anchors.fill: parent
    }

    Rectangle {
        width: 420
        height: settingsCol.height + 28
        anchors.centerIn: parent
        radius: 8
        color: "#2b2e33"
        border.color: "#3c4043"

        Column {
            id: settingsCol
            anchors.top: parent.top
            anchors.topMargin: 14
            anchors.left: parent.left
            anchors.leftMargin: 14
            anchors.right: parent.right
            anchors.rightMargin: 14
            spacing: 12

            // Title bar: title + close button.
            Row {
                width: parent.width
                spacing: 8
                Text {
                    id: setTitle
                    anchors.verticalCenter: parent.verticalCenter
                    text: "设置"
                    color: "#e9eaec"
                    font.pixelSize: 14
                    font.bold: true
                }
                Item { width: parent.width - setTitle.implicitWidth - 30 - 16; height: 1 }
                IconButton {
                    glyph: "✕"
                    tip: "关闭"
                    onClicked: settingsOverlay.visible = false
                }
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 1. checkerboard colors ----
            Text {
                text: "透明背景（棋盘格）颜色"
                color: "#b8bcc0"
                font.pixelSize: 12
            }
            Row {
                spacing: 10

                Canvas {
                    id: checkerPreview
                    width: 76
                    height: 26
                    anchors.verticalCenter: parent.verticalCenter
                    readonly property color cLight: bridge.checkerLight
                    readonly property color cDark: bridge.checkerDark
                    onCLightChanged: requestPaint()
                    onCDarkChanged: requestPaint()
                    onPaint: {
                        var ctx = getContext("2d")
                        const cell = 8
                        ctx.fillStyle = cDark
                        ctx.fillRect(0, 0, width, height)
                        ctx.fillStyle = cLight
                        for (var gy = 0; gy < height; gy += cell)
                            for (var gx = 0; gx < width; gx += cell)
                                if (((gx / cell) | 0) % 2 === ((gy / cell) | 0) % 2)
                                    ctx.fillRect(gx, gy, cell, cell)
                    }
                }

                Column {
                    spacing: 4
                    Text { text: "亮格"; color: "#8f96a3"; font.pixelSize: 11 }
                    Item {
                        width: 72
                        height: 24
                        Rectangle { anchors.fill: parent; color: "#1b1c1e"; border.color: "#3c4043"; radius: 3 }
                        TextInput {
                            id: hexLight
                            anchors.fill: parent
                            anchors.margins: 4
                            text: bridge.checkerLight
                            font.pixelSize: 12
                            color: "#e9eaec"
                            verticalAlignment: TextInput.AlignVCenter
                            onEditingFinished: {
                                if (/^#[0-9A-Fa-f]{6}$/.test(text))
                                    bridge.update_checker_light(text)
                                else
                                    text = bridge.checkerLight
                            }
                        }
                    }
                }

                Column {
                    spacing: 4
                    Text { text: "暗格"; color: "#8f96a3"; font.pixelSize: 11 }
                    Item {
                        width: 72
                        height: 24
                        Rectangle { anchors.fill: parent; color: "#1b1c1e"; border.color: "#3c4043"; radius: 3 }
                        TextInput {
                            id: hexDark
                            anchors.fill: parent
                            anchors.margins: 4
                            text: bridge.checkerDark
                            font.pixelSize: 12
                            color: "#e9eaec"
                            verticalAlignment: TextInput.AlignVCenter
                            onEditingFinished: {
                                if (/^#[0-9A-Fa-f]{6}$/.test(text))
                                    bridge.update_checker_dark(text)
                                else
                                    text = bridge.checkerDark
                            }
                        }
                    }
                }
            }
            Row {
                spacing: 6
                ToolButton {
                    label: "经典"
                    onClicked: {
                        bridge.update_checker_light("#ffffff")
                        bridge.update_checker_dark("#cccccc")
                        hexLight.text = "#ffffff"
                        hexDark.text = "#cccccc"
                    }
                }
                ToolButton {
                    label: "暗色"
                    onClicked: {
                        bridge.update_checker_light("#3c4043")
                        bridge.update_checker_dark("#202225")
                        hexLight.text = "#3c4043"
                        hexDark.text = "#202225"
                    }
                }
                ToolButton {
                    label: "纯白"
                    onClicked: {
                        bridge.update_checker_light("#ffffff")
                        bridge.update_checker_dark("#f2f2f2")
                        hexLight.text = "#ffffff"
                        hexDark.text = "#f2f2f2"
                    }
                }
                ToolButton {
                    label: "炭黑"
                    onClicked: {
                        bridge.update_checker_light("#2a2d31")
                        bridge.update_checker_dark("#141517")
                        hexLight.text = "#2a2d31"
                        hexDark.text = "#141517"
                    }
                }
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 2. save quality ----
            Text {
                text: "保存质量"
                color: "#b8bcc0"
                font.pixelSize: 12
            }
            Row {
                spacing: 8

                ToolButton {
                    label: "−"
                    active: bridge.saveQuality > 1
                    onClicked: {
                        bridge.update_save_quality(bridge.saveQuality - 5)
                        qualityInput.text = bridge.saveQuality
                    }
                }

                Item {
                    width: 56
                    height: 26
                    anchors.verticalCenter: parent.verticalCenter

                    Rectangle {
                        anchors.fill: parent
                        color: "#1b1c1e"
                        border.color: qualityInput.activeFocus ? "#6aa9ff" : "#3c4043"
                        border.width: qualityInput.activeFocus ? 2 : 1
                        radius: 3
                    }

                    TextInput {
                        id: qualityInput
                        anchors.fill: parent
                        anchors.margins: 5
                        text: bridge.saveQuality
                        font.pixelSize: 13
                        color: "#e9eaec"
                        verticalAlignment: TextInput.AlignVCenter
                        validator: IntValidator { bottom: 1; top: 100 }
                        selectByMouse: true
                        onActiveFocusChanged: if (activeFocus) selectAll()
                        onEditingFinished: {
                            const v = parseInt(text)
                            if (!isNaN(v))
                                bridge.update_save_quality(v)
                            text = bridge.saveQuality
                        }
                    }
                }

                ToolButton {
                    label: "+"
                    active: bridge.saveQuality < 100
                    onClicked: {
                        bridge.update_save_quality(bridge.saveQuality + 5)
                        qualityInput.text = bridge.saveQuality
                    }
                }

                Text {
                    text: "（1-100）"
                    color: "#6b7076"
                    font.pixelSize: 11
                    anchors.verticalCenter: parent.verticalCenter
                }
            }
            Text {
                width: parent.width
                text: "控制「另存副本 / 另存为 / 导出」的质量：\n· JPG：数值越低文件越小（有损压缩）\n· WebP：本版本仅无损编码，质量不影响体积\n· PNG / TIFF 等无损：质量 < 50 时改用最大压缩，文件最小、保存稍慢"
                color: "#8f96a3"
                font.pixelSize: 11
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 3. open behavior ----
            Text {
                text: "打开文件时"
                color: "#b8bcc0"
                font.pixelSize: 12
            }
            Repeater {
                model: [
                    { label: "在当前窗口显示（替换现有图片）", value: false },
                    { label: "在单独的新窗口显示", value: true }
                ]
                delegate: MouseArea {
                    id: radioRow
                    required property var modelData
                    width: 16 + 8 + lbl.implicitWidth
                    height: Math.max(16, lbl.implicitHeight)
                    cursorShape: Qt.PointingHandCursor
                    onClicked: bridge.update_open_in_new_window(modelData.value)

                    Item {
                        width: 16
                        height: 16
                        anchors.verticalCenter: parent.verticalCenter
                        Rectangle {
                            anchors.fill: parent
                            radius: 8
                            color: "transparent"
                            border.color: bridge.openInNewWindow === radioRow.modelData.value ? "#6aa9ff" : "#5a6069"
                            border.width: 2
                        }
                        Rectangle {
                            anchors.centerIn: parent
                            width: 8
                            height: 8
                            radius: 4
                            color: "#6aa9ff"
                            visible: bridge.openInNewWindow === radioRow.modelData.value
                        }
                    }
                    Text {
                        id: lbl
                        x: 24
                        anchors.verticalCenter: parent.verticalCenter
                        text: radioRow.modelData.label
                        color: "#d5d8dd"
                        font.pixelSize: 12
                    }
                }
            }

            MouseArea {
                id: downsampleRow
                width: 16 + 8 + dsLbl.implicitWidth
                height: Math.max(16, dsLbl.implicitHeight)
                cursorShape: Qt.PointingHandCursor
                onClicked: bridge.update_large_downsample(!bridge.largeDownsample)

                Item {
                    width: 16
                    height: 16
                    anchors.verticalCenter: parent.verticalCenter
                    Rectangle {
                        anchors.fill: parent
                        radius: 4
                        color: "transparent"
                        border.color: bridge.largeDownsample ? "#6aa9ff" : "#5a6069"
                        border.width: 2
                    }
                    Rectangle {
                        anchors.centerIn: parent
                        width: 8
                        height: 8
                        radius: 2
                        color: "#6aa9ff"
                        visible: bridge.largeDownsample
                    }
                }
                Text {
                    id: dsLbl
                    x: 24
                    anchors.verticalCenter: parent.verticalCenter
                    text: "超大图降采样（最长边 8192px）"
                    color: "#d5d8dd"
                    font.pixelSize: 12
                }
            }
            Text {
                width: parent.width
                text: "打开最长边超过 8192px 的图片时自动降到工作分辨率，打开更快、编辑更流畅。\n注意：另存/保存的分辨率也会是降采样后的；取消勾选则保留原始分辨率。"
                color: "#8f96a3"
                font.pixelSize: 11
                wrapMode: Text.WordWrap
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 4. default viewer ----
            Text {
                text: "默认图片查看器"
                color: "#b8bcc0"
                font.pixelSize: 12
            }
            Row {
                spacing: 10
                ToolButton { label: "注册并打开系统设置…"; onClicked: bridge.register_as_viewer() }
                Text {
                    text: "写入「打开方式」列表后打开\n系统设置，请在其中选择 ImageEdit"
                    color: "#8f96a3"
                    font.pixelSize: 11
                }
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 5. 尺子（测量坐标系）----
            Text {
                text: "尺子 / 测量坐标"
                color: "#b8bcc0"
                font.pixelSize: 12
            }
            CheckRow {
                label: "Y 轴从下往上（原点在左下角）"
                checked: bridge.rulerBottomUp
                onToggled: bridge.update_ruler_bottom_up(!bridge.rulerBottomUp)
            }
            Text {
                width: parent.width
                text: "关闭时原点在左上角（Y 向下，与屏幕顺序一致）；\n"
                      + "打开后 Y 从画布底边往上计数，适合按图纸/排版坐标测量。\n"
                      + "尺子开关在标记工具栏里（直尺图标的那个按钮）。"
                color: "#8f96a3"
                font.pixelSize: 11
            }

        }
    }
}
