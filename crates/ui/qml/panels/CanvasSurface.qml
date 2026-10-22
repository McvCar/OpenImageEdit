import QtQuick
import com.imageedit
import "../js/marks.js" as Marks

Item {
    id: canvasSurface
    required property var host
    required property var bridge
    required property var view
    required property var previewImage
    // 暴露给 EditorContent：光标/探针/选区/缩放/文字编辑等外部交互逻辑
    // 仍留在 EditorContent，通过 instance.alias 访问这些内部 id。
    readonly property alias selDragMouse: selDragMouse
    readonly property alias markMa: markMa
    readonly property alias markPreview: markPreview
    readonly property alias textPopup: textPopup
    readonly property alias scaleTimer: scaleTimer
    visible: bridge.image_width > 0
    z: 5
    x: previewImage.x - view.pad
    y: previewImage.y - view.pad
    width: previewImage.width + 2 * view.pad
    height: previewImage.height + 2 * view.pad

    // Bottom layer: the single canvas interaction area. It is always
    // enabled so hover coordinates never freeze (a disabled MouseArea
    // stops receiving hover events, which deadlocks any cursor/enabled
    // logic bound to them). Press behavior:
    //   - no selection + hit on a layer → select it; moving >3px
    //     starts a live drag of that layer (pick-and-move);
    //   - otherwise → rubber-band selection; a click on empty canvas
    //     clears both the selection and any layer selection.
    // Cursor: SizeAll over an unlocked layer's content, CrossCursor
    // elsewhere (the selection-move area shows its own SizeAll).
    MouseArea {
        id: selDragMouse
        anchors.fill: parent
        hoverEnabled: true
        // undefined: MouseAreas default to ArrowCursor and shadow
        // every cursor below; the canvas HoverHandler owns cursors.
        cursorShape: undefined
        property real startX: 0
        property real startY: 0
        property real curX: 0
        property real curY: 0
        property bool dragging: false
        // Layer pick-and-move gesture (no selection active).
        property bool pickDrag: false
        property bool pickMoved: false
        // Last deltas sent to the bridge (skip redundant renders).
        property int lastDx: 0
        property int lastDy: 0

        onPressed: (mouse) => {
            const hit = host.selExists ? null : host.topLayerAt(mouse.x, mouse.y)
            if (hit !== null) {
                if (bridge.selected_mark >= 0)
                    bridge.select_mark(-1)   // 标记/图层选中互斥
                bridge.select_layer(hit.i)
                pickDrag = true
                pickMoved = false
                startX = mouse.x
                startY = mouse.y
                curX = mouse.x
                curY = mouse.y
                return
            }
            startX = mouse.x
            startY = mouse.y
            curX = mouse.x
            curY = mouse.y
            dragging = true
            host.selW = 0
            host.selH = 0
            host.settlePaste()
        }
        onPositionChanged: (mouse) => {
            if (host.rulerOn)
                host.noteHover(mouse.x, mouse.y)
            if (pickDrag) {
                curX = mouse.x
                curY = mouse.y
                if (!pickMoved
                    && Math.abs(curX - startX) + Math.abs(curY - startY) > 3) {
                    pickMoved = true
                    lastDx = 0
                    lastDy = 0
                    bridge.drag_translate_begin()
                    // Start ONCE and keep repeating: restarting the
                    // timer on every mouse-move event would reset the
                    // countdown faster than it can expire (mouse
                    // events arrive at ~125Hz), so fast drags never
                    // rendered live.
                    liveTimer.start()
                }
                return
            }
            if (!dragging)
                return
            curX = mouse.x
            curY = mouse.y
            // Clamp the rubber band to the canvas: a selection can
            // never exceed the image bounds. Mouse coords are in
            // canvasSurface space; the image spans [pad, pad+size].
            const pad = view.pad
            const x1 = Math.max(pad, Math.min(startX, curX))
            const y1 = Math.max(pad, Math.min(startY, curY))
            const x2 = Math.min(pad + previewImage.width, Math.max(startX, curX))
            const y2 = Math.min(pad + previewImage.height, Math.max(startY, curY))
            host.selX = host.snapDisplay(x1 - pad)
            host.selY = host.snapDisplay(y1 - pad)
            host.selW = Math.max(host.snapDisplay(x2 - x1), host.toDispScale())
            host.selH = Math.max(host.snapDisplay(y2 - y1), host.toDispScale())
        }
        onReleased: {
            if (pickDrag) {
                pickDrag = false
                if (pickMoved) {
                    liveTimer.stop()
                    const scale = host.toImageScale()
                    bridge.drag_translate_update(
                        Math.round((curX - startX) * scale),
                        Math.round((curY - startY) * scale))
                    bridge.drag_translate_end(true)
                    if (host.pasteFloating)
                        host.floatingOps += 1
                }
                return
            }
            dragging = false
            if (host.selW < 3 || host.selH < 3) {
                // Click on empty canvas: clear the selection and any
                // layer selection with it.
                host.selW = 0
                host.selH = 0
                if (bridge.selected_layer >= 0)
                    bridge.select_layer(-1)
            }
        }
    }

    // ---- 标记绘制/编辑区：标记工具激活时盖在选择交互之上 -------
    //   按下在选中标记的 8 向手柄上 → 缩放该标记（live + 提交）
    //   按下在任一标记上 → 选中并拖动（live + 提交）
    //   否则 → 用当前工具绘制新标记；点击空白取消选中
    // cursorShape: undefined — 光标由 canvasHoverHandler 统一决定。
    MouseArea {
        id: markMa
        anchors.fill: parent
        // 标记与图层统一交互：标记元素随时可选中/拖动/缩放/双击编辑
        // （不再要求打开标记工具栏）；绘制新元素仍需 markTool 激活，
        // 且只发生在空白处——按在已有标记上 = 只拖拽/调整。
        // 与图层的分工：未命中标记时按下事件放行（accepted=false）
        // 给下层的 selDragMouse（图层拾取/移动/框选）。
        enabled: bridge.image_width > 0
        hoverEnabled: true
        cursorShape: undefined
        acceptedButtons: Qt.LeftButton

        // 0 idle / 1 drawing / 2 moving / 3 resizing
        property int mode: 0
        property string resizeRole: ""
        property rect startMark: Qt.rect(0, 0, 0, 0)
        property real startX: 0
        property real startY: 0
        // 当前鼠标（canvasSurface 坐标）。显式属性而非内置 mouseX：
        // 无头探针需要能直接驱动预览几何。
        property real curX: 0
        property real curY: 0
        property var brushPts: []
        // In-progress mark geometry in image coords (move/resize).
        property var gestureRect: null

        function imgX(mx) { return (mx - view.pad) * host.toImageScale() }
        function imgY(my) { return (my - view.pad) * host.toImageScale() }

        function handleCursor(role) {
            if (role === "n" || role === "s")
                return Qt.SizeVerCursor
            if (role === "e" || role === "w")
                return Qt.SizeHorCursor
            if (role === "nw" || role === "se")
                return Qt.SizeFDiagCursor
            return Qt.SizeBDiagCursor
        }

        // Topmost mark under (mx,my) in canvasSurface coords.
        function markAt(mx, my) {
            const ix = imgX(mx)
            const iy = imgY(my)
            const tol = 4 / Math.max(host.toDispScale(), 0.0001)
            for (var i = host.markList.length - 1; i >= 0; i--) {
                const m = host.markList[i]
                if (m.v === false)
                    continue
                if (ix >= m.x - tol && ix <= m.x + m.w + tol
                    && iy >= m.y - tol && iy <= m.y + m.h + tol)
                    return m.i
            }
            return -1
        }

        // Which resize handle (if any) is under (mx,my); 7px grab.
        function handleAt(mx, my) {
            if (bridge.selected_mark < 0)
                return ""
            const b = host.selectedMarkBox()
            if (!b)
                return ""
            // canvasSurface coords: the box is preview-relative.
            const bx = view.pad + b[0]
            const by = view.pad + b[1]
            const bw = b[2]
            const bh = b[3]
            const pts = {
                "nw": [bx, by],
                "n":  [bx + bw / 2, by],
                "ne": [bx + bw, by],
                "e":  [bx + bw, by + bh / 2],
                "se": [bx + bw, by + bh],
                "s":  [bx + bw / 2, by + bh],
                "sw": [bx, by + bh],
                "w":  [bx, by + bh / 2]
            }
            for (const k in pts) {
                if (Math.abs(mx - pts[k][0]) <= 7 && Math.abs(my - pts[k][1]) <= 7)
                    return k
            }
            return ""
        }

        function resizeTarget(role, r, dx, dy) {
            let x = r.x
            let y = r.y
            let w = r.width
            let h = r.height
            if (role.indexOf("w") !== -1) { x += dx; w -= dx }
            if (role.indexOf("e") !== -1) { w += dx }
            if (role.indexOf("n") !== -1) { y += dy; h -= dy }
            if (role.indexOf("s") !== -1) { h += dy }
            return Qt.rect(x, y, Math.max(1, w), Math.max(1, h))
        }

        function beginGesture(kind, mouse) {
            const m = host.findMark(bridge.selected_mark)
            if (!m)
                return false
            startMark = Qt.rect(m.x, m.y, m.w, m.h)
            startX = mouse.x
            startY = mouse.y
            mode = kind
            markTimer.lastSent = null
            bridge.mark_drag_begin()
            markTimer.start()
            return true
        }

        // Clamp a canvasSurface point into the image display rect.
        // Pad-area presses (aiming at image corner 0,0) must not
        // create marks with negative / overflowing rect coords.
        function clampCanvas(x, y) {
            const xa = view.pad
            const ya = view.pad
            const xb = view.pad + previewImage.width
            const yb = view.pad + previewImage.height
            return [Math.max(xa, Math.min(xb, x)),
                    Math.max(ya, Math.min(yb, y))]
        }

        onPressed: (mouse) => {
            // 正在就地编辑文本：点击别处先提交编辑。
            if (textPopup.visible) {
                textPopup.close(true)
                return
            }
            // 1) 选中标记的缩放手柄（任何工具下都优先）。
            const h = handleAt(mouse.x, mouse.y)
            if (h !== "") {
                resizeRole = h
                if (beginGesture(3, mouse))
                    return
            }
            // 2) 命中已有标记 → 选中并拖动。绘制工具激活时也是如此：
            //    点中标记元素只允许拖拽/调整，不产生新元素。
            const hit = markAt(mouse.x, mouse.y)
            if (hit >= 0) {
                if (bridge.selected_layer >= 0)
                    bridge.select_layer(-1)   // 标记/图层选中互斥
                bridge.select_mark(hit)
                if (beginGesture(2, mouse))
                    return
            }
            // 3) 绘制工具激活 + 空白处 → 绘制新元素。起点钳制到图片
            //    范围——瞄准图片 (0,0) 时鼠标极易落在 pad 区，不钳制
            //    会产生负坐标标记（渲染被裁剪成角落小残块）。
            if (host.markTool !== "none") {
                mode = 1
                const p = clampCanvas(mouse.x, mouse.y)
                startX = p[0]
                startY = p[1]
                curX = p[0]
                curY = p[1]
                brushPts = []
                if (host.markTool === "brush")
                    brushPts.push([imgX(startX), imgY(startY)])
                markPreview.requestPaint()
                return
            }
            // 4) 空白处 + 无绘制工具 → 取消标记选中，事件下放给下层
            //    的图层交互（选中图层 / 移动 / 框选 / 点空取消）。
            if (bridge.selected_mark >= 0)
                bridge.select_mark(-1)
            mouse.accepted = false
        }

        // 双击文本标记 → 就地编辑内容。
        onDoubleClicked: (mouse) => {
            const hit = markAt(mouse.x, mouse.y)
            if (hit < 0)
                return
            const m = host.findMark(hit)
            if (m && m.k === 5) {
                bridge.select_mark(hit)
                textPopup.openEdit(m)
            }
        }

        onPositionChanged: (mouse) => {
            if (host.rulerOn)
                host.noteHover(mouse.x, mouse.y)
            if (mode === 1) {
                const p = clampCanvas(mouse.x, mouse.y)
                curX = p[0]
                curY = p[1]
                if (host.markTool === "brush") {
                    const ix = imgX(curX)
                    const iy = imgY(curY)
                    const last = brushPts.length > 0
                                 ? brushPts[brushPts.length - 1] : null
                    if (!last || Math.abs(ix - last[0]) + Math.abs(iy - last[1]) >= 2)
                        brushPts.push([ix, iy])
                }
                markPreview.requestPaint()
                return
            }
            if (mode === 2) {
                const s = host.toImageScale()
                gestureRect = Qt.rect(
                    startMark.x + (mouse.x - startX) * s,
                    startMark.y + (mouse.y - startY) * s,
                    startMark.width, startMark.height)
            } else if (mode === 3) {
                gestureRect = resizeTarget(
                    resizeRole, startMark,
                    (mouse.x - startX) * host.toImageScale(),
                    (mouse.y - startY) * host.toImageScale())
            }
        }

        onReleased: (mouse) => {
            if (mode === 1) {
                const s = host.toImageScale()
                // 用跟踪的 curX/curY（已钳制到图片范围）而不是
                // mouse.x/y：提交的 rect 与预览完全一致。
                const x0 = (startX - view.pad) * s
                const y0 = (startY - view.pad) * s
                const x1 = (curX - view.pad) * s
                const y1 = (curY - view.pad) * s
                if (host.markTool === "brush") {
                    if (brushPts.length >= 2) {
                        bridge.add_brush_mark(
                            brushPts.map(p => p[0].toFixed(1) + "," + p[1].toFixed(1)).join(" "),
                            host.markStroke, host.markColor)
                    }
                } else if (host.markTool === "mosaic") {
                    // 选区模式：整块矩形打码（core 的 region 分支）。
                    if (Math.abs(x1 - x0) >= 1 && Math.abs(y1 - y0) >= 1)
                        bridge.add_mosaic_rect(x0, y0, x1, y1, host.markMosaic)
                } else if (Math.abs(x1 - x0) >= 1 || Math.abs(y1 - y0) >= 1) {
                    const k = host.markTool === "rect" ? 0
                              : host.markTool === "ellipse" ? 1 : 2
                    bridge.add_shape_mark(k, x0, y0, x1, y1,
                                          host.markStroke, host.markColor,
                                          host.markFill, host.markFillColor)
                }
            } else if (mode === 2 || mode === 3) {
                markTimer.stop()
                if (gestureRect)
                    bridge.mark_drag_update(gestureRect.x, gestureRect.y,
                                            gestureRect.width, gestureRect.height)
                bridge.mark_drag_end(true)
                gestureRect = null
            }
            mode = 0
            markPreview.requestPaint()
        }

        // Grab lost (another item / popup took the mouse): a stale
        // mode would freeze the drag preview on screen forever.
        // Cancel cleanly — no commit, no leftover preview.
        onCanceled: {
            markTimer.stop()
            gestureRect = null
            brushPts = []
            mode = 0
            markPreview.requestPaint()
        }
    }

    // Live move/resize throttle (same pattern as scaleTimer).
    Timer {
        id: markTimer
        interval: 40
        repeat: true
        property var lastSent: null
        onTriggered: {
            if (!markMa.gestureRect)
                return
            const g = markMa.gestureRect
            const rounded = Qt.rect(Math.round(g.x), Math.round(g.y),
                                    Math.round(g.width), Math.round(g.height))
            if (lastSent && lastSent.x === rounded.x && lastSent.y === rounded.y
                && lastSent.width === rounded.width
                && lastSent.height === rounded.height)
                return
            lastSent = rounded
            bridge.mark_drag_update(rounded.x, rounded.y,
                                    rounded.width, rounded.height)
        }
    }

    // 绘制中的标记预览（display 坐标，松手后由 core 接管渲染）。
    // 拖拽期间持续触发重绘：Canvas 会丢弃单次 requestPaint（尤其
    // 场景静止/光标停顿时），不补拍的话预览就冻结在早期帧——
    // 即"拖动时小椭圆、放手后变大"的元凶。
    Timer {
        id: markPreviewTimer
        interval: 16
        repeat: true
        running: markMa.mode === 1
        onTriggered: markPreview.requestPaint()
    }

    Canvas {
        id: markPreview
        anchors.fill: parent
        visible: markMa.mode === 1
        // Immediate + Image：requestPaint 时在调用线程同步重绘。
        // 默认 Cooperative 策略会在场景静止时丢弃单次重绘请求
        //（实测：onPaint 执行了但屏幕永不更新）——拖拽中某一帧
        // 丢失后预览就永久冻结，即"拖动时小椭圆、放手后变大"。
        renderStrategy: Canvas.Immediate
        renderTarget: Canvas.Image
        onPaint: {
            try {
                var ctx = getContext("2d")
                ctx.clearRect(0, 0, width, height)
                if (markMa.mode !== 1)
                    return
                paintImpl(ctx)
            } catch (e) {
                // onPaint 抛异常会让 Canvas 停留在上一帧内容
                //（"冻结预览"），这里保证错误可见且不扩散。
                bridge.mark("PAINT-ERR " + e)
            }
        }
        function paintImpl(ctx) {
            const s = host.toDispScale()
            ctx.lineWidth = Math.max(1, host.markStroke * s)
            ctx.strokeStyle = host.markColor
            ctx.fillStyle = host.markFillColor
            ctx.lineCap = "round"
            ctx.lineJoin = "round"
            const px = v => view.pad + v * s
            const x0 = markMa.startX
            const y0 = markMa.startY
            const x1 = markMa.curX
            const y1 = markMa.curY
            if (host.markTool === "mosaic") {
                // 选区预览：半透明灰块 + 虚线框；真正的像素化由
                // core 提交时渲染。
                ctx.fillStyle = "rgba(128,128,128,0.55)"
                ctx.fillRect(x0, y0, x1 - x0, y1 - y0)
                ctx.strokeStyle = "#e9eaec"
                ctx.lineWidth = 1
                ctx.setLineDash([4, 3])
                ctx.strokeRect(x0, y0, x1 - x0, y1 - y0)
                ctx.setLineDash([])
                return
            }
            if (host.markTool === "brush") {
                ctx.beginPath()
                for (var i = 0; i < markMa.brushPts.length; i++) {
                    const p = markMa.brushPts[i]
                    if (i === 0)
                        ctx.moveTo(px(p[0]), px(p[1]))
                    else
                        ctx.lineTo(px(p[0]), px(p[1]))
                }
                ctx.stroke()
                return
            }
            if (host.markTool === "rect") {
                if (host.markFill) {
                    ctx.fillRect(x0, y0, x1 - x0, y1 - y0)
                }
                ctx.strokeRect(x0, y0, x1 - x0, y1 - y0)
                return
            }
            if (host.markTool === "ellipse") {
                // core 的椭圆描边画在边界内侧（ring），预览保持一致：
                // 半径先内收半个线宽。
                const shrink = Math.max(1, host.markStroke * s) / 2
                const ecx = (x0 + x1) / 2
                const ecy = (y0 + y1) / 2
                const erx = Math.max(0.5, Math.abs(x1 - x0) / 2 - shrink)
                const ery = Math.max(0.5, Math.abs(y1 - y0) / 2 - shrink)
                // ★Qt 的 Context2D.ellipse(x, y, w, h, ...) 把 (x,y)
                // 当外接矩形左上角、(w,h) 当宽高，而非 HTML 标准的
                // 圆心+半径——传圆心+半径会画进右下四分之一子矩形
                //（"拖动时小椭圆跑偏、放手后变大"的真正根因，REDBBOX
                // 实测 [ecx..ecx+erx, ecy..ecy+ery]）。改用单位圆 +
                // 坐标缩放构造：路径点在缩放变换生效时记录，恢复变换
                // 后再描边，线宽保持均匀，且不依赖任何 ellipse 语义。
                ctx.save()
                ctx.translate(ecx, ecy)
                ctx.scale(erx, ery)
                ctx.beginPath()
                ctx.arc(0, 0, 1, 0, Math.PI * 2)
                ctx.restore()
                if (host.markFill)
                    ctx.fill()
                ctx.stroke()
                return
            }
            if (host.markTool === "arrow") {
                const dx = x1 - x0
                const dy = y1 - y0
                const len = Math.sqrt(dx * dx + dy * dy)
                if (len < 2)
                    return
                const ux = dx / len
                const uy = dy / len
                const head = Math.max(host.markStroke * s * 3.5, 10)
                ctx.beginPath()
                ctx.moveTo(x0, y0)
                ctx.lineTo(x1 - ux * head, y1 - uy * head)
                ctx.stroke()
                const hw = head * 0.55
                ctx.beginPath()
                ctx.moveTo(x1, y1)
                ctx.lineTo(x1 - ux * head - uy * hw, y1 - uy * head + ux * hw)
                ctx.lineTo(x1 - ux * head + uy * hw, y1 - uy * head - ux * hw)
                ctx.closePath()
                ctx.fillStyle = host.markColor
                ctx.fill()
            }
        }
    }

    // 选中标记的边框 + 8 向手柄（display 坐标）。标记随时可选中，
    // 不依赖标记工具栏是否打开。
    DashedRect {
        id: markSelViz
        visible: bridge.selected_mark >= 0
                 && markMa.mode === 0
        // canvasSurface 坐标：画布原点 = (pad, pad)。不能用
        // previewImage.x/y（那是 Flickable 内容坐标，图片居中时
        // ≠ pad，选中框会整体漂移）。
        x: view.pad
        y: view.pad
        width: previewImage.width
        height: previewImage.height
        rx: box ? box[0] : 0
        ry: box ? box[1] : 0
        rw: box ? box[2] : 0
        rh: box ? box[3] : 0
        dashColor: "#378add"

        property var box: host.selectedMarkBox()
        onBoxChanged: if (visible) requestPaint()
    }

    Repeater {
        model: bridge.selected_mark >= 0
               && markMa.mode === 0 ? 8 : 0

        delegate: Rectangle {
            required property int index
            readonly property var roles: ["nw", "n", "ne", "e", "se", "s", "sw", "w"]
            readonly property string role: roles[index]
            readonly property var box: host.selectedMarkBox()
            visible: box !== null
            width: 8
            height: 8
            radius: 4
            color: "#378add"
            border.color: "#ffffff"
            border.width: 1
            // canvasSurface 坐标 = 预览相对坐标 + pad。
            x: box ? view.pad + (role.indexOf("w") !== -1 ? box[0] - 4
                     : role.indexOf("e") !== -1 ? box[0] + box[2] - 4
                     : box[0] + box[2] / 2 - 4) : 0
            y: box ? view.pad + (role.indexOf("n") !== -1 ? box[1] - 4
                     : role.indexOf("s") !== -1 ? box[1] + box[3] - 4
                     : box[1] + box[3] / 2 - 4) : 0
        }
    }

    // 文本标记的就地编辑框：直接叠在标记原位（预览里该标记由
    // set_editing_mark 临时隐藏，避免叠影），Enter 提交，Esc 取消。
    TextInput {
        id: textPopup
        visible: false
        z: 80
        // 编辑中的标记快照（字段来自 marks_json：fn/fs/b/it/c/t）。
        property var editMark: null
        property int editIndex: -1

        font.family: editMark ? Marks.qmlFontFamily(editMark.fn) : "Microsoft YaHei"
        font.pixelSize: editMark ? Math.max(8, editMark.fs * host.toDispScale()) : 20
        font.bold: editMark ? !!editMark.b : false
        font.italic: editMark ? !!editMark.it : false
        color: editMark ? editMark.c : "#ffffff"
        verticalAlignment: TextInput.AlignTop
        clip: false
        selectByMouse: true
        onAccepted: textPopup.close(true)
        Keys.onEscapePressed: textPopup.close(false)

        function openEdit(m) {
            editMark = m
            editIndex = m.i
            const s = host.toDispScale()
            x = view.pad + m.x * s
            y = view.pad + m.y * s
            width = Math.max(m.w * s, implicitWidth + 4)
            height = Math.max(m.h * s, implicitHeight)
            text = m.t || ""
            visible = true
            bridge.set_editing_mark(m.i)
            forceActiveFocus()
            selectAll()
        }
        function close(commit) {
            if (commit && editIndex >= 0 && text.trim() !== "")
                bridge.update_text_content(text)
            bridge.set_editing_mark(-1)
            visible = false
            editIndex = -1
            editMark = null
            text = ""
        }
    }

    Timer {
        id: liveTimer
        interval: 33
        repeat: true
        onTriggered: {
            if (!selDragMouse.pickMoved)
                return
            const scale = host.toImageScale()
            const dx = Math.round((selDragMouse.curX - selDragMouse.startX) * scale)
            const dy = Math.round((selDragMouse.curY - selDragMouse.startY) * scale)
            if (dx === selDragMouse.lastDx && dy === selDragMouse.lastDy)
                return
            selDragMouse.lastDx = dx
            selDragMouse.lastDy = dy
            bridge.drag_translate_update(dx, dy)
        }
    }

    // Live scale preview throttle for the selected layer's handles.
    // repeat:true + start-once: restarting per mouse-move event would
    // starve the timer during fast drags (same as liveTimer).
    Timer {
        id: scaleTimer
        interval: 60
        repeat: true
        property int lastW: -1
        property int lastH: -1
        property int lastX: 0
        property int lastY: 0
        onTriggered: {
            if (host.scaleW <= 0)
                return
            const s = host.toDispScale()
            const w = Math.max(1, Math.round(host.scaleW / s))
            const h = Math.max(1, Math.round(host.scaleH / s))
            const x = Math.round(host.scaleX / s)
            const y = Math.round(host.scaleY / s)
            if (w === lastW && h === lastH && x === lastX && y === lastY)
                return
            lastW = w; lastH = h; lastX = x; lastY = y
            bridge.scale_layer_update(w, h, x, y)
        }
    }

    // ---- 拾色器取样层 ----
    // 面板打开期间画布点击 = 取色（不再进入选区/图层手势），点击空白
    // 不会清选区。关闭面板即恢复正常交互。
    MouseArea {
        id: pickerPickMa
        anchors.fill: parent
        z: 40
        visible: bridge.pickerOpen && bridge.image_width > 0
        enabled: visible
        // undefined：光标统一由 canvasHoverHandler 决定。
        cursorShape: undefined
        onClicked: (mouse) => {
            const s = host.toDispScale()
            if (s <= 0)
                return
            bridge.picker_pick(Math.floor((mouse.x - view.pad) / s),
                               Math.floor((mouse.y - view.pad) / s))
        }
    }

    // Authoritative canvas cursor: a HoverHandler's cursorShape beats
    // every MouseArea's (documented precedence), and one handler over
    // the whole surface means no stacking conflicts. Coordinates are
    // in canvasSurface space — image coords are offset by view.pad.
    HoverHandler {
        id: canvasHoverHandler
        cursorShape: panMa.pressed ? Qt.ClosedHandCursor
                                   : host.canvasCursor(point.position.x,
                                                  point.position.y)
        // 尺子读数：handler 是唯一贯穿整块画布的 hover 来源（各个
        // MouseArea 谁在上面不确定），这里补一路坐标。
        onPointChanged: if (host.rulerOn)
                            host.noteHover(point.position.x, point.position.y)
        onHoveredChanged: if (!hovered) host.clearHover()
    }
}
