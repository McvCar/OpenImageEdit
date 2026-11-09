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
    // 形状 / 马赛克组的合并按钮（MarkShapePopover 贴它下面弹）。
    readonly property alias markShapeBtn: markShapeBtn
    readonly property alias widthBtn: widthBtn
    readonly property alias colorBtn: colorBtn
    // 选区面板的锚点（选区工具已从独立工具条搬到这里）。
    readonly property alias selBtn: selBtn
    // 两个「工具组」按钮的锚点（PaintPopover 贴它们下面弹）。
    readonly property alias healBtn: healBtn
    readonly property alias paintBtn: paintBtn
    visible: host.markBarVisible
    anchors.top: toolbar.bottom
    anchors.left: parent.left
    anchors.right: parent.right
    height: host.toolbarHeight
    color: "#25272b"
    z: 59
    // 窗口被压到极窄时按钮行会撑出条带右端；裁掉总比压在图层面板上好。
    clip: true

    // 窄窗口：一行的按钮放不下时，从右端成组收起（尺子 / 拾色器 / 它们
    // 前面的那条分隔线）。这三个命令在「⋯ → 图层与标记」里都有，不会
    // 变成够不着的功能。阈值按整行实测宽度（≈460）留一点余量。
    readonly property bool tailFits: width >= 500

    Row {
        id: markRow
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        // 移动工具（PS 的 V）：一个**开关**，默认开。开着时图层可拖动/缩放，
        // 并且和选区、画笔、标记工具并存；再点一次（或按 V）收起手柄。
        IconButton {
            id: moveBtn
            tip: "移动工具：拖动图层 / 缩放手柄（V 开关）"
            active: bridge.image_width > 0
            selected: host.moveMode
            onClicked: host.toggleMoveTool()
            // 四向箭头：画一个细十字 + 四个实心三角。
            Canvas {
                width: 22
                height: 22
                anchors.centerIn: parent
                property color ink: moveBtn.selected ? "#ffffff" : "#e9eaec"
                onInkChanged: requestPaint()
                onPaint: {
                    const c = getContext("2d")
                    c.clearRect(0, 0, width, height)
                    c.strokeStyle = ink
                    c.fillStyle = ink
                    c.lineWidth = 1.3
                    c.beginPath()
                    c.moveTo(11, 5.5); c.lineTo(11, 16.5)
                    c.moveTo(5.5, 11); c.lineTo(16.5, 11)
                    c.stroke()
                    const tri = function (x1, y1, x2, y2, x3, y3) {
                        c.beginPath()
                        c.moveTo(x1, y1); c.lineTo(x2, y2); c.lineTo(x3, y3)
                        c.closePath(); c.fill()
                    }
                    tri(11, 2.6, 8.4, 6.4, 13.6, 6.4)
                    tri(11, 19.4, 8.4, 15.6, 13.6, 15.6)
                    tri(2.6, 11, 6.4, 8.4, 6.4, 13.6)
                    tri(19.4, 11, 15.6, 8.4, 15.6, 13.6)
                }
            }
        }

        // 选区（PS 的选框工具）：点开浮动面板选矩形 / 椭圆 / 套索 /
        // 多边形。以前的整行固定选区工具条已经取消。
        IconButton {
            id: selBtn
            tip: "选区（矩形 / 椭圆 / 套索 / 多边形，M · L）"
            active: bridge.image_width > 0
            selected: host.selPopoverOpen
            onClicked: host.toggleSelPopover()
            // 虚线方框 = 蚂蚁线选框（和实线矩形标记区分开）。
            Canvas {
                width: 22
                height: 22
                anchors.centerIn: parent
                property color ink: selBtn.selected ? "#ffffff" : "#e9eaec"
                onInkChanged: requestPaint()
                onPaint: {
                    const c = getContext("2d")
                    c.clearRect(0, 0, width, height)
                    c.strokeStyle = ink
                    c.lineWidth = 1.4
                    c.setLineDash([3, 2.5])
                    c.beginPath()
                    c.rect(3.5, 4.5, 15.0, 13.0)
                    c.stroke()
                    c.setLineDash([])
                }
            }
        }

        Rectangle { width: 9; height: 18; color: "transparent"
            Rectangle { anchors.centerIn: parent; width: 1; height: 18; color: "#3c4043" } }

        // 形状工具组合并成一个按钮：矩形 / 椭圆 / 箭头 / 马赛克 全收进
        // 悬浮面板（点开选，和选区、油漆的做法一致），工具栏少三个按钮。
        // 按钮图标跟着当前工具变，选着哪个一眼能看出来。
        IconButton {
            id: markShapeBtn
            tip: "形状与马赛克（矩形 / 椭圆 / 箭头 / 马赛克）"
            active: bridge.image_width > 0
            selected: markShapePopover.visible
                      || host.markTool === "rect" || host.markTool === "ellipse"
                      || host.markTool === "arrow" || host.markTool === "brush"
                      || host.markTool === "mosaic"
            onClicked: host.toggleMarkShapeBtn()
            Canvas {
                width: 22
                height: 22
                anchors.centerIn: parent
                property color ink: markShapeBtn.selected ? "#ffffff" : "#e9eaec"
                property string kind: (host.markTool === "ellipse"
                                       || host.markTool === "arrow"
                                       || host.markTool === "brush"
                                       || host.markTool === "mosaic")
                                      ? host.markTool : "rect"
                onInkChanged: requestPaint()
                onKindChanged: requestPaint()
                onPaint: {
                    const c = getContext("2d")
                    c.clearRect(0, 0, width, height)
                    c.strokeStyle = ink
                    c.fillStyle = ink
                    c.lineWidth = 1.5
                    if (kind === "ellipse") {
                        c.beginPath()
                        c.ellipse(11, 11, 7.5, 5.5, 0, 0, Math.PI * 2)
                        c.stroke()
                    } else if (kind === "arrow") {
                        c.beginPath()
                        c.moveTo(5, 17); c.lineTo(16.5, 5.5)
                        c.stroke()
                        c.beginPath()
                        c.moveTo(17, 5); c.lineTo(11.5, 6.5); c.lineTo(15.5, 10.5)
                        c.closePath(); c.fill()
                    } else if (kind === "brush") {
                        c.beginPath()
                        c.moveTo(4.5, 16.5)
                        c.bezierCurveTo(8, 5.5, 14, 17.5, 18.5, 6.5)
                        c.stroke()
                    } else if (kind === "mosaic") {
                        for (let y = 0; y < 3; ++y)
                            for (let x = 0; x < 3; ++x) {
                                c.fillStyle = (x + y) % 2 === 0 ? ink : "#8a8f96"
                                c.fillRect(4 + x * 5, 4 + y * 5, 4.4, 4.4)
                            }
                    } else {
                        c.strokeRect(4, 6, 14, 10)
                    }
                }
            }
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
                markShapePopover.visible = false
                colorPopover.visible = false
                bridge.add_text_mark(
                    bridge.image_width / 2 - 40, bridge.image_height / 2,
                    "文字", host.markFont, host.markFontSize,
                    host.markBold, host.markItalic, host.markColor)
                fontPopover.visible = true
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
                markShapePopover.visible = false
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
                markShapePopover.visible = false
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
                markShapePopover.visible = false
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

        // ---- 修复工具组（污点修复 / 修复画笔 / 修补）------------------
        // 三个工具的入口收在同一个按钮里，点开是分组面板（PS 的工具组）。
        IconButton {
            id: healBtn
            tip: "修复工具组：污点修复 / 修复画笔 / 修补 (J)"
            active: bridge.image_width > 0
            selected: host.paintTool === "spot" || host.paintTool === "heal"
                      || host.paintTool === "patch"
            onClicked: host.togglePaintGroup("heal")
            // 创可贴图标：斜置的圆角长条 + 中间两条透气线。
            Canvas {
                width: 22
                height: 22
                anchors.centerIn: parent
                property color ink: healBtn.selected ? "#ffffff" : "#e9eaec"
                onInkChanged: requestPaint()
                onPaint: {
                    const c = getContext("2d")
                    c.clearRect(0, 0, width, height)
                    c.save()
                    c.translate(11, 11)
                    c.rotate(-Math.PI / 4)
                    c.strokeStyle = ink
                    c.lineWidth = 1.3
                    const r = 3.2
                    const w = 15, h = 8
                    c.beginPath()
                    c.moveTo(-w / 2 + r, -h / 2)
                    c.lineTo(w / 2 - r, -h / 2)
                    c.quadraticCurveTo(w / 2, -h / 2, w / 2, -h / 2 + r)
                    c.lineTo(w / 2, h / 2 - r)
                    c.quadraticCurveTo(w / 2, h / 2, w / 2 - r, h / 2)
                    c.lineTo(-w / 2 + r, h / 2)
                    c.quadraticCurveTo(-w / 2, h / 2, -w / 2, h / 2 - r)
                    c.lineTo(-w / 2, -h / 2 + r)
                    c.quadraticCurveTo(-w / 2, -h / 2, -w / 2 + r, -h / 2)
                    c.closePath()
                    c.stroke()
                    c.beginPath()
                    c.moveTo(-2.2, -2.4); c.lineTo(-2.2, 2.4)
                    c.moveTo(2.2, -2.4); c.lineTo(2.2, 2.4)
                    c.stroke()
                    c.restore()
                }
            }
        }

        // ---- 油漆工具组（油漆桶 / 渐变）-------------------------------
        IconButton {
            id: paintBtn
            tip: "油漆工具组：油漆桶 / 渐变 (G)"
            active: bridge.image_width > 0
            selected: host.paintTool === "bucket" || host.paintTool === "gradient"
            onClicked: host.togglePaintGroup("paint")
            // 油漆桶：梯形桶身 + 提手 + 一滴颜料。
            Canvas {
                width: 22
                height: 22
                anchors.centerIn: parent
                property color ink: paintBtn.selected ? "#ffffff" : "#e9eaec"
                onInkChanged: requestPaint()
                onPaint: {
                    const c = getContext("2d")
                    c.clearRect(0, 0, width, height)
                    c.strokeStyle = ink
                    c.fillStyle = ink
                    c.lineWidth = 1.3
                    c.beginPath()
                    c.moveTo(5.5, 8.5)
                    c.lineTo(17.5, 8.5)
                    c.lineTo(15.2, 18.5)
                    c.lineTo(7.8, 18.5)
                    c.closePath()
                    c.stroke()
                    c.beginPath()
                    c.moveTo(7.5, 8.5)
                    c.quadraticCurveTo(11.5, 3.0, 15.5, 8.5)
                    c.stroke()
                    c.beginPath()
                    c.ellipse(18.6, 5.2, 2.0, 2.6)
                    c.fill()
                }
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
            visible: markBar.tailFits
            Rectangle { anchors.centerIn: parent; width: 1; height: 18; color: "#3c4043" } }

        // 尺子：显示选区 / 选中图层的四角+中心坐标，并跟随鼠标读数。
        IconButton {
            id: rulerBtn
            visible: markBar.tailFits
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
            visible: markBar.tailFits
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
        // P0-5：变换数值面板开关（D7 首次默认隐藏 / D13 记住上次状态）。
        IconButton {
            id: transformPanelBtn
            glyph: "⊞"
            tip: "变换数值面板：外接框 X/Y/W/H、角度、参考点"
            selected: bridge.transformPanelVisible
            onClicked: bridge.update_transform_panel_visible(!bridge.transformPanelVisible)
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
        text: host.paintTool === "spot"
              ? "按住涂抹 · 周边像素自动盖住瑕疵 · [ ] 调笔刷"
              : host.paintTool === "heal"
              ? (host.healHasSource
                 ? "按住涂抹复制取样点 · Alt 重新取样 · [ ] 调笔刷"
                 : "Alt + 点击一处干净的地方取样")
              : host.paintTool === "patch"
              ? (host.patchHasRegion
                 ? "拖住区域到取样处松手 · Esc 放弃区域"
                 : "先拖出一个要修补的区域")
              : host.paintTool === "bucket"
              ? "点击填色 · 有选区时只填选区里 · 右键调参数"
              : host.paintTool === "gradient"
              ? "拖一条线定义渐变方向 · 右键调参数"
              // 选区工具：顺带说清「作用在哪一层」——选中图层时像素操作只动
              // 那一层，没选中才动背景（PS 的语义）。
              : host.selTool === "quick"
              ? (bridge.selected_layer >= 0
                 ? "按住涂抹吸附相近颜色 · 只作用选中图层 · Alt 减选"
                 : "按住涂抹吸附相近颜色 · 未选图层时作用背景 · Alt 减选")
              : host.selTool === "wand"
              ? (bridge.selected_layer >= 0
                 ? "点一下取相近颜色 · 只取选中图层 · Shift 加选 / Alt 减选"
                 : "点一下取相近颜色 · 未选图层时取背景 · Shift 加选 / Alt 减选")
              : (host.markTool === "none"
                 ? (host.selTool === "none" && bridge.selected_layer >= 0
                    ? "拖动图层移动 · 四角缩放 · 顶部圆点旋转 · 方向键微调"
                    : "选择工具绘制 · 点击标记可移动，双击文本可编辑")
                 : (host.markTool === "brush"
                    ? "按住拖拽绘制笔画"
                    : host.markTool === "mosaic"
                    ? "按住拖拽划选区打码 · 强度可调"
                    : "按住拖拽绘制 · 点击已有标记可移动/缩放"))
        color: "#6b7076"
        font.pixelSize: 11
    }
}

