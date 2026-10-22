import QtQuick
import com.imageedit
import "../js/adjust.js" as Adjust

Item {
    id: adjustPanel
    required property var host
    required property var bridge
    required property var toolbar
    required property var markBar
    required property var adjustBtn
    visible: bridge.adjustOpen
    width: 320
    // 弹窗样式：贴着「调整颜色」按钮的正下方（与其它标记弹窗一致），
    // 窗口太矮时面板内部滚动。高度与 y 无关，拖动时不会越拖越矮。
    readonly property real maxHeight: Math.max(240, host.height - toolbar.height - 24)
    // 按钮在 root 里的 x（标记栏开关 / 布局变化都会改变它）→ 用于重新锚定。
    readonly property real anchorX: adjustBtn.x + adjustBtn.parent.x
    height: Math.min(adjustCol.implicitHeight + 24, maxHeight)
    x: 8
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    z: 220
    // 用户拖动过标题栏之后就不再自动贴回按钮；每次重新打开复位。
    property bool userMoved: false
    onVisibleChanged: {
        if (visible) {
            userMoved = false
            host.placePopover(adjustPanel, adjustBtn)
        }
    }
    onAnchorXChanged: if (visible && !userMoved) host.placePopover(adjustPanel, adjustBtn)
    // 窗口变矮 / 面板内容变高时，把用户拖过的位置重新夹回窗口内。
    onHeightChanged: if (visible && userMoved) moveTo(x, y)

    // 拖动标题栏移动弹窗（限制在窗口内，别拖丢）。
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
        id: adjustFlick
        anchors.fill: parent
        anchors.margins: 12
        contentWidth: width
        contentHeight: adjustCol.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height

        Column {
            id: adjustCol
            width: adjustFlick.width
            spacing: 7

            // ---- 标题栏：拖动这里可移动弹窗；右侧 ✕ = 应用并关闭 ----
            Item {
                id: adjustTitleBar
                width: parent.width
                height: 26

                // 拖动区先声明（在 ✕ 按钮之下），按钮仍能正常点击。
                MouseArea {
                    id: adjustDragArea
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
                        fromX = adjustPanel.x
                        fromY = adjustPanel.y
                    }
                    onPositionChanged: (mouse) => {
                        if (!pressed)
                            return
                        const p = mapToItem(host, mouse.x, mouse.y)
                        adjustPanel.moveTo(fromX + (p.x - pressX),
                                           fromY + (p.y - pressY))
                    }
                }

                Text {
                    id: adjustTitle
                    anchors.verticalCenter: parent.verticalCenter
                    text: "调整颜色"
                    color: "#e9eaec"
                    font.pixelSize: 13
                    font.bold: true
                }
                Text {
                    anchors.left: adjustTitle.right
                    anchors.leftMargin: 8
                    anchors.right: adjustClose.left
                    anchors.rightMargin: 6
                    anchors.verticalCenter: parent.verticalCenter
                    text: bridge.adjustTargetName.length > 0
                          ? "作用：" + bridge.adjustTargetName : ""
                    color: "#8f96a3"
                    font.pixelSize: 10
                    elide: Text.ElideRight
                }
                IconButton {
                    id: adjustClose
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    glyph: "✕"
                    tip: "应用并关闭（Esc 放弃）"
                    onClicked: host.commitAdjust()
                }
            }

            // ---- 直方图（白底 + RGB 三通道叠加，对齐 macOS 预览）+ 色阶三手柄 ----
            Item {
                width: parent.width
                height: 122

                Rectangle {
                    id: adjustHistWell
                    anchors.top: parent.top
                    width: parent.width
                    height: 84
                    color: "#ffffff"
                    radius: 4
                    border.color: "#b9bdc4"
                    border.width: 1
                    clip: true

                    Canvas {
                        id: adjustHistCanvas
                        anchors.fill: parent
                        renderStrategy: Canvas.Immediate
                        renderTarget: Canvas.Image
                        property var hist: Adjust.parseHistogram(bridge.adjustHistogram)
                        onHistChanged: requestPaint()
                        // 裁剪区随色阶手柄变化重绘。
                        property real blk: host.adjustVals.bl
                        property real wht: host.adjustVals.wh
                        onBlkChanged: requestPaint()
                        onWhtChanged: requestPaint()

                        onPaint: {
                            const ctx = getContext("2d")
                            const w = width
                            const h = height
                            ctx.clearRect(0, 0, w, h)
                            // macOS 预览的直方图是白色底板 + RGB 三通道
                            // 半透明面积叠加；线性纵向比例（只有最高峰
                            // 顶到上沿，其余贴近基线）。
                            ctx.fillStyle = "#ffffff"
                            ctx.fillRect(0, 0, w, h)

                            const hist = adjustHistCanvas.hist
                            const n = (hist && hist.n) ? hist.n : 64
                            let peak = 0
                            if (hist) {
                                for (let i = 0; i < n; i++)
                                    peak = Math.max(peak, hist.r[i], hist.g[i], hist.b[i])
                            }
                            if (peak > 0) {
                                const bar = w / n
                                // 阶梯面积图：每个 bin 一条竖带。
                                const area = function (bins, color) {
                                    ctx.beginPath()
                                    ctx.moveTo(0, h)
                                    for (let i = 0; i < n; i++) {
                                        const y = h - Math.min(1, bins[i] / peak) * (h - 2)
                                        ctx.lineTo(i * bar, y)
                                        ctx.lineTo((i + 1) * bar, y)
                                    }
                                    ctx.lineTo(w, h)
                                    ctx.closePath()
                                    ctx.fillStyle = color
                                    ctx.fill()
                                }
                                // R→G→B 依次叠加：单通道处是本色，多通道
                                // 重叠处压暗（预览里深色的基线就是这么来的）。
                                ctx.globalAlpha = 0.75
                                area(hist.r, "#e02020")
                                area(hist.g, "#12b112")
                                area(hist.b, "#2646e0")
                                ctx.globalAlpha = 1.0
                            }

                            // 色阶范围外的裁剪区：左侧压黑、右侧蒙白。截图里
                            // 直方图左边那条深灰、右边那条浅灰就是这个（提示
                            // 这部分像素会被裁成纯黑/纯白）。
                            if (adjustHistCanvas.blk > 0.002) {
                                ctx.fillStyle = "rgba(0,0,0,0.55)"
                                ctx.fillRect(0, 0, adjustHistCanvas.blk * w, h)
                            }
                            if (adjustHistCanvas.wht < 0.998) {
                                ctx.fillStyle = "rgba(255,255,255,0.72)"
                                ctx.fillRect(adjustHistCanvas.wht * w, 0,
                                             Math.max(0, w - adjustHistCanvas.wht * w), h)
                            }
                        }
                    }
                }

                // 三个手柄：黑场 / Gamma / 白场。
                Item {
                    id: adjustHandles
                    // 锚到底板（而不是 Canvas 子项：跨父项锚定会被 Qt 拒绝）。
                    anchors.top: adjustHistWell.bottom
                    width: parent.width
                    height: 20

                    function setHandle(kind, x) {
                        const t = Math.max(0, Math.min(1, x / width))
                        const v = host.adjustVals
                        if (kind === "black")
                            host.setLevels(t, v.ga, v.wh)
                        else if (kind === "white")
                            host.setLevels(v.bl, v.ga, t)
                        else
                            host.setLevels(v.bl, Math.max(0.05, Math.min(20,
                                           Math.log(Math.max(0.02, Math.min(0.98, t)))
                                           / Math.log(0.5))), v.wh)
                    }

                    Repeater {
                        model: [{ kind: "black" }, { kind: "mid" }, { kind: "white" }]

                        delegate: Item {
                            id: handleItem
                            required property var modelData
                            readonly property real pos: {
                                const v = host.adjustVals
                                if (modelData.kind === "black")
                                    return v.bl
                                if (modelData.kind === "white")
                                    return v.wh
                                return v.bl + (v.wh - v.bl) * v.mh
                            }
                            readonly property bool active: handleMa.containsMouse
                                                            || handleMa.pressed
                            x: Math.max(0, Math.min(adjustHandles.width - 16,
                                                   pos * adjustHandles.width - 8))
                            width: 16
                            height: 20

                            // macOS 预览式手柄：顶部三角 + 圆角矩形本体，
                            // 悬停/拖动时整体变蓝。
                            Canvas {
                                id: knobCanvas
                                anchors.fill: parent
                                renderStrategy: Canvas.Immediate
                                renderTarget: Canvas.Image
                                property bool hot: handleItem.active
                                onHotChanged: requestPaint()

                                function paintKnob(active) {
                                    const ctx = getContext("2d")
                                    const w = width
                                    const h = height
                                    ctx.clearRect(0, 0, w, h)
                                    const cx = w / 2
                                    const tip = 0.5
                                    const shoulder = 7
                                    const bottom = h - 0.5
                                    const edge = 6
                                    ctx.beginPath()
                                    ctx.moveTo(cx, tip)
                                    ctx.lineTo(cx + 3.5, shoulder - 2)
                                    ctx.lineTo(cx + edge, shoulder)
                                    ctx.lineTo(cx + edge, bottom - 2.5)
                                    ctx.quadraticCurveTo(cx + edge, bottom, cx + edge - 2.5, bottom)
                                    ctx.lineTo(cx - edge + 2.5, bottom)
                                    ctx.quadraticCurveTo(cx - edge, bottom, cx - edge, bottom - 2.5)
                                    ctx.lineTo(cx - edge, shoulder)
                                    ctx.lineTo(cx - 3.5, shoulder - 2)
                                    ctx.closePath()
                                    ctx.fillStyle = active ? "#c6e3ff" : "#f4f5f7"
                                    ctx.fill()
                                    ctx.strokeStyle = active ? "#4f9dff" : "#8d949d"
                                    ctx.lineWidth = 1
                                    ctx.stroke()
                                    // 中间的握柄横线
                                    ctx.fillStyle = active ? "#4f9dff" : "#b9bec6"
                                    ctx.fillRect(cx - 3, bottom - 8, 6, 2)
                                }

                                onPaint: paintKnob(hot)
                            }

                            MouseArea {
                                id: handleMa
                                anchors.fill: parent
                                anchors.margins: -3
                                hoverEnabled: true
                                cursorShape: Qt.SizeHorCursor
                                onPressed: (mouse) => adjustHandles.setHandle(
                                                  modelData.kind,
                                                  (mapToItem(adjustHandles, mouse.x, 0).x))
                                onPositionChanged: (mouse) => {
                                    if (pressed)
                                        adjustHandles.setHandle(
                                            modelData.kind,
                                            mapToItem(adjustHandles, mouse.x, 0).x)
                                }
                            }
                        }
                    }

                    // 色阶读数放在手柄行下方单独一行（见下方 Text），
                    // 避免与白场手柄重叠。
                }

                // 色阶读数（手柄单独一行，避免与白场手柄重叠）。
                Text {
                    anchors.top: adjustHandles.bottom
                    anchors.left: parent.left
                    text: "黑场 " + Math.round(host.adjustVals.bl * 255)
                          + "    中间 " + host.adjustVals.ga.toFixed(2)
                          + "    白场 " + Math.round(host.adjustVals.wh * 255)
                    color: "#7c828b"
                    font.pixelSize: 9
                }
            }

            // ---- 自动色阶 ----
            Item {
                width: parent.width
                height: 26
                ToolButton {
                    anchors.centerIn: parent
                    label: "自动色阶"
                    active: bridge.image_width > 0
                    onClicked: bridge.adjust_auto_levels()
                }
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 组 1：光量 ----
            AdjustRow {
                glyph: "☼"; label: "曝光"; key: "exposure"; value: host.adjustVals.e
                onMoved: (v) => host.setAdjust("exposure", v)
                onResetRequested: host.applyAdjustNow("exposure", 0)
            }
            AdjustRow {
                glyph: "◑"; label: "对比度"; key: "contrast"; value: host.adjustVals.c
                onMoved: (v) => host.setAdjust("contrast", v)
                onResetRequested: host.applyAdjustNow("contrast", 0)
            }
            AdjustRow {
                glyph: "◉"; label: "高光"; key: "highlights"; value: host.adjustVals.hi
                onMoved: (v) => host.setAdjust("highlights", v)
                onResetRequested: host.applyAdjustNow("highlights", 0)
            }
            AdjustRow {
                glyph: "◍"; label: "阴影"; key: "shadows"; value: host.adjustVals.sh
                onMoved: (v) => host.setAdjust("shadows", v)
                onResetRequested: host.applyAdjustNow("shadows", 0)
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 组 2：颜色 ----
            AdjustRow {
                glyph: "◈"; label: "饱和度"; key: "saturation"; value: host.adjustVals.sa
                onMoved: (v) => host.setAdjust("saturation", v)
                onResetRequested: host.applyAdjustNow("saturation", 0)
            }
            AdjustRow {
                glyph: "❄"; label: "色温"; key: "temperature"; value: host.adjustVals.te
                onMoved: (v) => host.setAdjust("temperature", v)
                onResetRequested: host.applyAdjustNow("temperature", 0)
            }
            AdjustRow {
                glyph: "◐"; label: "色调"; key: "tint"; value: host.adjustVals.ti
                onMoved: (v) => host.setAdjust("tint", v)
                onResetRequested: host.applyAdjustNow("tint", 0)
            }
            AdjustRow {
                glyph: "▤"; label: "棕褐色"; key: "sepia"; value: host.adjustVals.se
                minVal: 0; maxVal: 1
                onMoved: (v) => host.setAdjust("sepia", v)
                onResetRequested: host.applyAdjustNow("sepia", 0)
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 组 3：清晰度 ----
            AdjustRow {
                glyph: "◆"; label: "清晰度"; key: "sharpness"; value: host.adjustVals.sp
                minVal: 0; maxVal: 1
                onMoved: (v) => host.setAdjust("sharpness", v)
                onResetRequested: host.applyAdjustNow("sharpness", 0)
            }

            // ---- 全部还原 ----
            Item {
                width: parent.width
                height: 28
                ToolButton {
                    anchors.centerIn: parent
                    label: "全部还原"
                    active: bridge.adjustOpen
                    onClicked: {
                        host.adjustDirty = false
                        host.levelsDirty = false
                        bridge.adjust_reset()
                    }
                }
            }
        }
    }

    // 调整颜色的节流下发：用 repeat 计时器（不是 restart）——快速拖动时
    // 鼠标事件会不断重置 restart 型计时器，预览就永远不更新（见 MEMORY）。
    Timer {
        id: adjustFlushTimer
        interval: 33
        repeat: true
        running: bridge.adjustOpen && (host.adjustDirty || host.levelsDirty)
        onTriggered: {
            host.flushAdjust()
            host.flushLevels()
        }
    }
}
