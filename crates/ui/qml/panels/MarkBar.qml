import QtQuick
import com.imageedit

// ---- 标记工具栏：主工具栏下方的第二行（开关 = 标题栏 ✎ 按钮）------
Rectangle {
    id: markBar
    required property var host
    required property var bridge
    required property var toolbar
    // 暴露给 EditorContent：AdjustPanel/PickerPanel 实例接线 + 弹窗定位锚点。
    readonly property alias adjustBtn: adjustBtn
    readonly property alias pickerBtn: pickerBtn
    // 暴露给 4 个 Popover：placePopover 需要按钮作锚点（块内 id 跨文件不可达）。
    readonly property alias fontBtn: fontBtn
    readonly property alias mosaicBtn: mosaicBtn
    readonly property alias widthBtn: widthBtn
    readonly property alias colorBtn: colorBtn
    visible: host.markBarVisible
    anchors.top: toolbar.bottom
    anchors.left: parent.left
    anchors.right: parent.right
    height: host.toolbarHeight
    color: "#25272b"
    z: 59

    Row {
        id: markRow
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        IconButton {
            glyph: "▭"
            tip: "矩形标记（再点一次取消）"
            selected: host.markTool === "rect"
            onClicked: host.setMarkTool("rect")
        }
        IconButton {
            glyph: "◯"
            tip: "椭圆标记（再点一次取消）"
            selected: host.markTool === "ellipse"
            onClicked: host.setMarkTool("ellipse")
        }
        IconButton {
            glyph: "↗"
            tip: "箭头标记（再点一次取消）"
            selected: host.markTool === "arrow"
            onClicked: host.setMarkTool("arrow")
        }
        IconButton {
            glyph: "✏"
            tip: "自由画笔（再点一次取消）"
            selected: host.markTool === "brush"
            onClicked: host.setMarkTool("brush")
        }
        IconButton {
            glyph: "T"
            tip: "添加文本标记（双击文本可编辑）"
            // 普通动作按钮：不保持选中态，每次点击在图片中央放一个
            // 默认文本标记，并打开字体面板供调整。
            onClicked: {
                if (bridge.image_width <= 0)
                    return
                // 字体按钮是普通动作按钮：取消其它工具按钮的选中态，
                // 避免点了 T 还残留矩形/椭圆等绘制工具。
                host.markTool = "none"
                widthPopover.visible = false
                mosaicPopover.visible = false
                colorPopover.visible = false
                bridge.add_text_mark(
                    bridge.image_width / 2 - 40, bridge.image_height / 2,
                    "文字", host.markFont, host.markFontSize,
                    host.markBold, host.markItalic, host.markColor)
                fontPopover.visible = true
            }
        }
        IconButton {
            tip: "马赛克（划选区打码，再点一次取消）"
            selected: host.markTool === "mosaic"
            onClicked: host.setMarkTool("mosaic")
            // 像素块图标：3x3 深浅交错的小方块，比 ▦ 更直观。
            Grid {
                anchors.centerIn: parent
                columns: 3
                spacing: 1
                Repeater {
                    model: 9
                    Rectangle {
                        required property int index
                        width: 4
                        height: 4
                        radius: 1
                        color: index % 2 === 0 ? "#e9eaec" : "#8a8f96"
                    }
                }
            }
        }

        Rectangle { width: 9; height: 18; color: "transparent"
            Rectangle { anchors.centerIn: parent; width: 1; height: 18; color: "#3c4043" } }

        // 字体控制：选中文本标记时显示。
        IconButton {
            id: fontBtn
            glyph: "Aa"
            tip: "文本样式"
            visible: host.selMarkIsText
            selected: fontPopover.visible
            onClicked: {
                mosaicPopover.visible = false
                fontPopover.visible = !fontPopover.visible
            }
            Text {
                anchors.bottom: parent.bottom
                anchors.right: parent.right
                anchors.margins: 2
                text: host.markFontSize
                color: "#9aa3b2"
                font.pixelSize: 9
            }
        }

        // 马赛克强度：仅马赛克工具激活时显示。
        IconButton {
            id: mosaicBtn
            tip: "马赛克强度"
            visible: host.markTool === "mosaic"
            selected: mosaicPopover.visible
            onClicked: {
                fontPopover.visible = false
                mosaicPopover.visible = !mosaicPopover.visible
            }
            Grid {
                anchors.centerIn: parent
                anchors.verticalCenterOffset: -3
                columns: 3
                spacing: 1
                Repeater {
                    model: 9
                    Rectangle {
                        required property int index
                        width: 3
                        height: 3
                        radius: 1
                        color: index % 2 === 0 ? "#e9eaec" : "#8a8f96"
                    }
                }
            }
            Text {
                anchors.bottom: parent.bottom
                anchors.right: parent.right
                anchors.margins: 2
                text: host.markMosaic
                color: "#9aa3b2"
                font.pixelSize: 9
            }
        }

        IconButton {
            id: widthBtn
            glyph: "─"
            tip: "笔画粗细"
            selected: widthPopover.visible
            onClicked: {
                colorPopover.visible = false
                fontPopover.visible = false
                mosaicPopover.visible = false
                widthPopover.visible = !widthPopover.visible
            }
            Text {
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.margins: 2
                text: host.markStroke
                color: "#9aa3b2"
                font.pixelSize: 9
            }
        }

        IconButton {
            id: colorBtn
            tip: "颜色"
            selected: colorPopover.visible
            onClicked: {
                widthPopover.visible = false
                fontPopover.visible = false
                mosaicPopover.visible = false
                colorPopover.visible = !colorPopover.visible
            }
            Rectangle {
                anchors.centerIn: parent
                width: 14
                height: 14
                radius: 3
                color: host.markColor
                border.color: "#e9eaec"
                border.width: 1
            }
        }

        IconButton {
            glyph: "▨"
            tip: "填充色开关（矩形/椭圆）"
            active: host.markTool === "rect" || host.markTool === "ellipse"
            selected: host.markFill
                         && (host.markTool === "rect" || host.markTool === "ellipse")
            onClicked: {
                host.markFill = !host.markFill
                host.applyMarkStyle()
            }
            Rectangle {
                anchors.centerIn: parent
                width: 14
                height: 14
                radius: 3
                color: host.markFillColor
                border.color: host.markFill ? "#e9eaec" : "#5a5e64"
                border.width: 1
            }
        }

        Rectangle { width: 9; height: 18; color: "transparent"
            Rectangle { anchors.centerIn: parent; width: 1; height: 18; color: "#3c4043" } }

        // 调整颜色（macOS 预览风格）：打开实时调色面板；再点一次 = 应用。
        IconButton {
            id: adjustBtn
            glyph: "◑"
            tip: "调整颜色（曝光 / 对比度 / 色阶 …）"
            active: bridge.image_width > 0
            selected: bridge.adjustOpen
            onClicked: host.toggleAdjustPanel()
        }

        Rectangle { width: 9; height: 18; color: "transparent"
            Rectangle { anchors.centerIn: parent; width: 1; height: 18; color: "#3c4043" } }

        // 尺子：显示选区 / 选中图层的四角+中心坐标，并跟随鼠标读数。
        IconButton {
            id: rulerBtn
            tip: "尺子（显示坐标，用于测量）"
            active: bridge.image_width > 0
            selected: host.rulerOn
            onClicked: host.toggleRuler()
            // 直尺图标：外框 + 三根刻度。
            Canvas {
                width: 22
                height: 22
                anchors.centerIn: parent
                // selected 变化时重绘（onPaint 里的绑定不是响应式的）。
                property color ink: rulerBtn.selected ? "#ffffff" : "#e9eaec"
                onInkChanged: requestPaint()
                onPaint: {
                    const c = getContext("2d")
                    c.clearRect(0, 0, width, height)
                    c.strokeStyle = ink
                    c.lineWidth = 1.4
                    c.lineCap = "butt"
                    c.beginPath()
                    c.rect(1.5, 6.5, 19, 9)
                    c.stroke()
                    c.beginPath()
                    c.moveTo(6.0, 6.5); c.lineTo(6.0, 11.5)
                    c.moveTo(11.0, 6.5); c.lineTo(11.0, 14.0)
                    c.moveTo(16.0, 6.5); c.lineTo(16.0, 11.5)
                    c.stroke()
                }
            }
        }

        // 拾色器：打开取色弹窗，点击图片取色并复制色值。
        IconButton {
            id: pickerBtn
            tip: "拾色器（点击图片取色，可复制色值）"
            active: bridge.image_width > 0
            selected: bridge.pickerOpen
            onClicked: host.togglePickerPanel()
            // 吸管图标：斜置的管身 + 尖端 + 顶端胶囊。
            Canvas {
                width: 22
                height: 22
                anchors.centerIn: parent
                property color ink: pickerBtn.selected ? "#ffffff" : "#e9eaec"
                onInkChanged: requestPaint()
                onPaint: {
                    const c = getContext("2d")
                    c.clearRect(0, 0, width, height)
                    c.strokeStyle = ink
                    c.fillStyle = ink
                    c.lineWidth = 1.4
                    c.save()
                    c.translate(11, 11)
                    c.rotate(-Math.PI / 4)
                    c.beginPath()
                    c.rect(-2.6, -6.0, 5.2, 8.5)
                    c.stroke()
                    c.beginPath()
                    c.moveTo(-2.6, 2.5)
                    c.lineTo(2.6, 2.5)
                    c.lineTo(0, 8.0)
                    c.closePath()
                    c.fill()
                    c.beginPath()
                    c.rect(-3.8, -9.2, 7.6, 3.4)
                    c.stroke()
                    c.restore()
                }
            }
        }
    }

    // 操作提示：贴在按钮行右边剩下的空间里，右对齐 + 省略号，窄窗口下绝不
    // 盖住左边的按钮（原来是右侧绝对定位，窄窗口会和按钮叠在一起）。
    Text {
        anchors.left: markRow.right
        anchors.leftMargin: 12
        anchors.right: parent.right
        anchors.rightMargin: 12
        anchors.verticalCenter: parent.verticalCenter
        horizontalAlignment: Text.AlignRight
        elide: Text.ElideRight
        // 剩不到 120px 就别硬塞了：提示是辅助信息，按钮才是本体。
        visible: width > 120
        text: host.markTool === "none"
              ? "选择工具绘制 · 点击标记可移动，双击文本可编辑"
              : (host.markTool === "brush"
                 ? "按住拖拽绘制笔画"
                 : host.markTool === "mosaic"
                 ? "按住拖拽划选区打码 · 强度可调"
                 : "按住拖拽绘制 · 点击已有标记可移动/缩放")
        color: "#6b7076"
        font.pixelSize: 11
    }
}

