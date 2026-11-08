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
    readonly property alias selPreview: selPreview
    readonly property alias markMa: markMa
    readonly property alias markPreview: markPreview
    readonly property alias textPopup: textPopup
    readonly property alias scaleTimer: scaleTimer

    // ---- 多边形套索的进行中状态（跨多次点击，所以放在根上）----
    // 已提交的顶点（图像坐标），以及跟随光标的橡皮筋位置（画布坐标）。
    property bool selPolyActive: false
    property var polyPts: []
    property real polyCurX: 0
    property real polyCurY: 0
    // 加选/减选模式在「第一个点」时锁定：中途松掉 Shift 不该改变意图。
    property int polyMode: 0

    // ---- 快速选择正在涂抹 ----
    // 按下到松开之间，每次移动都把新的一段笔迹送去扩散（Rust 侧是增量，
    // 所以选区能跟着笔尖长）。放在根上是为了让切工具/Esc 能取消它。
    property bool quickDrag: false

    // ---- 修复 / 油漆手势 ----
    // paintStroke：污点修复 / 修复画笔 / 渐变 的按住拖动。
    property bool paintStroke: false
    // patchDrawing：修补工具的第一段（圈区域）。
    property bool patchDrawing: false
    // patchDragging：修补工具的第二段（把区域拖到取样处）。
    property bool patchDragging: false
    // 圈区域时的临时顶点 + 已提交区域的顶点（图像坐标，QML 侧留着画轮廓）。
    property var patchPts: []
    property var patchRegion: []
    // 第二段拖动中的显示偏移（像素，画布坐标）。
    property real patchDx: 0
    property real patchDy: 0
    property real patchStartX: 0
    property real patchStartY: 0

    // ---- 拖动已有选区（PS 默认：按住选区内部拖 = 移动蚂蚁线）----
    // 拖动期间只在 QML 里给轮廓加偏移，松手才让 Rust 一次性平移 mask：
    // 每帧跨语言重算 mask + 重抽轮廓，大选区会明显掉帧。
    property bool moveSelActive: false
    property real moveFromX: 0
    property real moveFromY: 0
    // 显示坐标下的临时偏移（SelectionViz 绘制时叠加）。
    property real selOffX: 0
    property real selOffY: 0
    // ★11-07 晚修正：gestureMods / liveMods 已移入 selDragMouse MouseArea。
    //   原来声明在根级，而 liveTimer 读的是 `selDragMouse.liveMods` ——
    //   MouseArea 上根本没有这个属性，读到 undefined，传给 Rust 的
    //   qinvokable(int) 被静默强转成 0：拖动全程修饰键恒为「无」，
    //   移动中按 Ctrl 永远关不掉吸附（用户复测报的就是这个）。

    // 把画布坐标换算成图像坐标（可能越界，交给 Rust 裁剪）。
    function toImage(mx, my) {
        const s = host.toDispScale()
        return s > 0 ? [(mx - view.pad) / s, (my - view.pad) / s] : [0, 0]
    }

    // 按下时命中的图层（null = 不拾取）。
    //
    // ★优先级：有选区且按在选区内部 → 选区手势优先（移动选区 / 拖手柄）。
    //   选区之外、图层之上仍然是「选中并拖动图层」；Shift/Alt 永远走选区
    //   加选 / 减选；快速选择 / 魔术棒、绘画 / 标记工具各自接管手势。
    //   ★11-07 复测修正：这里只保证「选区内部」让位；只要本函数真的
    //   返回了图层（选区之外，或移动工具模式），onPressed 就会清掉残留
    //   框选。之前完全不清会让 Delete 被选区截胡，用户看到「选中图层按
    //   Del 没反应」。选区压在图层上时，点在选区内部依然不清、不打断。
    function layerPickAt(mx, my, mods) {
        if (!host.moveMode)
            return null
        if (host.selTool === "quick" || host.selTool === "wand")
            return null
        if (host.paintTool !== "none" || host.markTool !== "none")
            return null
        if (host.selTool !== "none"
                && ((mods | 0) & (Qt.ShiftModifier | Qt.AltModifier)) !== 0)
            return null
        if (host.selExists && host.selTool !== "none") {
            const p = toImage(mx, my)
            if (bridge.sel_contains(Math.floor(p[0]), Math.floor(p[1])))
                return null
        }
        return host.topLayerAt(mx, my)
    }

    // 空白处单击 = 退出「图层 / 选区」选择（PS 的「点空白清选择」）。
    // ★图层优先，一次点击只解一层：否则「图层悄悄不选了、选区还在」这种
    //   看不见的状态会让人以为点击没生效。旧逻辑只在 selTool 为 none 时
    //   执行，而默认工具是矩形选框 —— 于是「选中图层 → 点空白」什么都不会
    //   发生（用户报的就是这个）。
    function clickEmptyCanvas() {
        if (bridge.selected_layer >= 0)
            bridge.select_layer(-1)
        else if (host.selExists)
            bridge.sel_clear()
    }

    // 移动工具开着、并且已经有选中图层时：在**图层框外**起手画选区 =
    // 明确要把这次选区作用在背景上，先取消图层选中。不取消的话选区会被
    // `effective_selection` 裁到那一层里、抠图 / 调色也只动那一层，用户看到
    // 的就是「在别处拉了一框，结果只选中了图层里的那一小块」。
    // * 框内起手不动它：用户可能就是要在这层里选区。
    // * 移动工具关掉时保持原样（纯选区模式，图层选中不影响手感）。
    function dropLayerForSelection(mx, my) {
        if (Qt.application.arguments.indexOf("--probe-keys") !== -1) {
            const bx = host.selLayerBox
            const pp = toImage(mx, my)
            bridge.mark("drop-layer: mm=" + host.moveMode
                        + " sel=" + bridge.selected_layer
                        + " p=" + Math.round(pp[0]) + "," + Math.round(pp[1])
                        + " b=" + (bx ? [Math.round(bx.x), Math.round(bx.y),
                                          Math.round(bx.w), Math.round(bx.h)].join(",") : "null")
                        + " tool=" + host.selTool)
        }
        if (!host.moveMode || bridge.selected_layer < 0)
            return
        const b = host.selLayerBox
        if (b === null || !(b.w > 0) || !(b.h > 0)) {
            bridge.select_layer(-1)
            return
        }
        const p = toImage(mx, my)
        if (p[0] < b.x || p[0] > b.x + b.w || p[1] < b.y || p[1] > b.y + b.h)
            bridge.select_layer(-1)
    }

    function cancelPolygon() {
        if (!selPolyActive && polyPts.length === 0)
            return
        selPolyActive = false
        polyPts = []
        selPreview.requestPaint()
    }

    // 放弃正在进行的快速选择涂抹（切工具 / Esc）：回到按下前的选区，
    // 也不留撤销记录。
    function cancelQuickSelect() {
        if (!quickDrag)
            return
        quickDrag = false
        bridge.quick_select_cancel()
    }

    // 放弃正在进行的修复 / 渐变手势（切工具 / Esc）：把已经预览写入的像素
    // 还原回去，不产生撤销记录。
    function cancelPaint() {
        if (paintStroke) {
            paintStroke = false
            if (host.paintTool === "gradient")
                bridge.gradient_cancel()
            else
                bridge.retouch_cancel()
        }
        if (patchDragging) {
            patchDragging = false
            patchDx = 0
            patchDy = 0
            bridge.patch_drag_cancel()
            patchViz.requestPaint()
        }
        if (patchDrawing) {
            patchDrawing = false
            patchPts = []
            patchViz.requestPaint()
        }
    }

    // 放弃修补区域（Esc / 切工具）：只清区域，不动像素。
    function clearPatchRegion() {
        patchRegion = []
        patchPts = []
        patchDx = 0
        patchDy = 0
        bridge.patch_clear_region()
        patchViz.requestPaint()
    }

    // 闭合多边形 → 提交给 Rust（mode 取决于按下时的 Shift/Alt）。
    function commitPolygon() {
        if (polyPts.length >= 3) {
            const text = polyPts
                .map(p => p[0].toFixed(1) + "," + p[1].toFixed(1))
                .join(" ")
            bridge.sel_set_polygon(text, polyMode)
        }
        selPolyActive = false
        polyPts = []
        selPreview.requestPaint()
    }

    function addPolygonPoint(mx, my) {
        const p = toImage(mx, my)
        if (!selPolyActive) {
            selPolyActive = true
            polyPts = []
            polyMode = selDragMouse.selMode
        }
        // 点回起点（8px 内）= 闭合多边形。
        if (polyPts.length >= 3) {
            const s = Math.max(host.toDispScale(), 0.0001)
            const first = polyPts[0]
            const dx = (first[0] - p[0]) * s
            const dy = (first[1] - p[1]) * s
            if (Math.sqrt(dx * dx + dy * dy) <= 8) {
                commitPolygon()
                return
            }
        }
        polyPts = polyPts.concat([[p[0], p[1]]])
        polyCurX = mx
        polyCurY = my
        selPreview.requestPaint()
    }

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
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        // undefined: MouseAreas default to ArrowCursor and shadow
        // every cursor below; the canvas HoverHandler owns cursors.
        cursorShape: undefined
        property real startX: 0
        property real startY: 0
        property real curX: 0
        property real curY: 0
        property bool dragging: false
        // 本次手势的合成模式：0 替换 / 1 加选 / 2 减选 / 3 相交。
        // 按下时的 Shift/Alt 决定，松开时一起提交给 Rust。
        property int selMode: 0
        // 套索的采样点（画布坐标）。
        property var lassoPts: []
        // Layer pick-and-move gesture (no selection active).
        property bool pickDrag: false
        property bool pickMoved: false
        // Last deltas sent to the bridge (skip redundant renders).
        property int lastDx: 0
        property int lastDy: 0
        // 手势起手时锁存的修饰键位掩码（只用于「起手那一刻」的判定）。
        property int gestureMods: 0
        // ★拖动过程中的**实时**修饰键：按 Ctrl 想临时关掉吸附时，按下瞬间
        //   的掩码里还没有 Ctrl。Qt 只在鼠标事件里带 modifiers，所以每次
        //   onPositionChanged 都刷新一次，手感上等于「按住 Ctrl 立刻生效」。
        //   必须声明在 MouseArea 上：liveTimer 用 `selDragMouse.liveMods`
        //   读它，放根级会读到 undefined → 强转 0 → Ctrl 失效。
        property int liveMods: 0

        // PS 惯例：Shift = 加选，Alt = 减选，Shift+Alt = 相交，否则用
        // 工具条上的粘滞模式。
        function modeFrom(mods) {
            const add = (mods & Qt.ShiftModifier) !== 0
            const sub = (mods & Qt.AltModifier) !== 0
            if (add && sub)
                return 3
            if (add)
                return 1
            if (sub)
                return 2
            return host.selMode
        }

        onPressed: (mouse) => {
            // 按下画布 = 离开所有浮层输入框：字号框握着 activeFocus 会把
            // Delete / 工具键整组禁掉（详见 host.dropPopoverFocus 注释）。
            host.dropPopoverFocus()
            // 在画布上按下 = 选区面板收起（不吞掉这次点击）。
            if (host.selPopoverOpen)
                host.selPopoverOpen = false
            // 修复 / 油漆工具的参数面板同理：点画布就收起来，但这一下继续用来画。
            if (host.paintPopoverOpen)
                host.closePaintPopovers()
            // 右键：修复 / 油漆工具 → 光标处弹参数面板；否则落在选区内 →
            // 弹选区菜单（PS 的右键菜单）。
            if (mouse.button === Qt.RightButton) {
                if (host.paintTool !== "none") {
                    const g = mapToItem(host, mouse.x, mouse.y)
                    host.openPaintMenu(g.x, g.y)
                    mouse.accepted = true
                    return
                }
                const p = toImage(mouse.x, mouse.y)
                if (bridge.sel_contains(Math.floor(p[0]), Math.floor(p[1]))) {
                    const g = mapToItem(host, mouse.x, mouse.y)
                    host.openSelMenu(g.x, g.y)
                    mouse.accepted = true
                }
                return
            }
            // ---- 修复 / 油漆工具：优先于图层选择与选区手势 ----
            if (host.paintTool !== "none") {
                const p = toImage(mouse.x, mouse.y)
                if (host.paintTool === "bucket") {
                    bridge.bucket_fill(p[0], p[1], host.bucketTolerance,
                                       host.bucketContiguous,
                                       host.bucketOpacity / 100, host.markColor)
                    return
                }
                if (host.paintTool === "patch") {
                    if (patchRegion.length >= 3
                            && bridge.patch_contains(p[0], p[1])) {
                        patchDragging = true
                        patchStartX = mouse.x
                        patchStartY = mouse.y
                        patchDx = 0
                        patchDy = 0
                        bridge.patch_drag_begin(p[0], p[1], host.patchFeather)
                    } else {
                        patchDrawing = true
                        patchRegion = []
                        patchPts = [[p[0], p[1]]]
                    }
                    patchViz.requestPaint()
                    return
                }
                // 修复画笔：Alt + 点击 = 重新取样（不起笔）。
                if (host.paintTool === "heal"
                        && (mouse.modifiers & Qt.AltModifier) !== 0) {
                    bridge.retouch_set_source(p[0], p[1])
                    host.healHasSource = true
                    return
                }
                paintStroke = true
                if (host.paintTool === "gradient") {
                    bridge.gradient_begin(p[0], p[1], host.gradRadial,
                                          host.gradReverse,
                                          host.gradOpacity / 100,
                                          host.markColor, host.markFillColor)
                } else {
                    bridge.retouch_begin(host.paintTool, p[0], p[1],
                                         host.healSize, host.healHardness / 100,
                                         host.healSpacing / 100,
                                         host.healStrength / 100)
                }
                return
            }
            // 选区工具激活时不拾取图层（见 layerPickAt）。
            const hit = layerPickAt(mouse.x, mouse.y, mouse.modifiers)
            if (hit !== null) {
                // 旋转手势万一没收尾（QML 丢了 released / canceled），先把角度
                // 提交掉再起拖；Rust 的 drag_translate_begin 还会再兜一次底，
                // 把残留的临时预览态清干净，避免「框拖走了、内容没动」。
                if (host.rotating)
                    host.finishLayerRotate(true, 0)
                if (bridge.selected_mark >= 0)
                    bridge.select_mark(-1)   // 标记/图层选中互斥
                // 上一次拖动如果没收尾（切窗口、Alt+Tab、浮层抢焦点导致丢了
                // release），这些标志会残留为真，把这一次拖动整段吃掉。
                // 既然现在明确要拖图层，就地清干净。
                moveSelActive = false
                selOffX = 0
                selOffY = 0
                // 明确点中图层 = 放弃残留的框选选区。layerPickAt 已经保证
                // 「按在选区内部」不会走到这里，所以选区与图层重叠时选区
                // 手势仍然优先。顺带修好 Delete：选区残留会被 onDeleteKey
                // 抢先当成「删除选区像素」，图层怎么按都删不掉。
                // 放在 select_layer 之前，避免 sel_clear 的重绘/信号影响选中态。
                if (host.selExists)
                    bridge.sel_clear()
                bridge.select_layer(hit.i)
                gestureMods = mouse.modifiers
                liveMods = mouse.modifiers
                pickDrag = true
                pickMoved = false
                startX = mouse.x
                startY = mouse.y
                curX = mouse.x
                curY = mouse.y
                return
            }
            host.settlePaste()
            selMode = modeFrom(mouse.modifiers)
            // 魔术棒：一次点击出一片选区，没有拖拽过程（PS 也是点一下就好）。
            // 必须排在「按在选区内 = 移动选区」前面：魔棒每次点击都是重新
            // 取色，不能因为点在旧选区里就变成拖选区。
            if (host.selTool === "wand") {
                dropLayerForSelection(mouse.x, mouse.y)
                const pw = toImage(mouse.x, mouse.y)
                bridge.sel_magic_wand(pw[0], pw[1], host.wandTolerance,
                                      host.wandContiguous, host.wandSampleAll,
                                      selMode)
                return
            }
            // 快速选择：按住涂抹。第一笔就决定这次的合成模式（Shift 加选 /
            // Alt 减选），中途改修饰键不会让选区跳变。
            if (host.selTool === "quick") {
                dropLayerForSelection(mouse.x, mouse.y)
                const pw = toImage(mouse.x, mouse.y)
                quickDrag = true
                bridge.quick_select_brush(pw[0], pw[1], host.quickRadius,
                                          host.quickTolerance, host.quickGrow,
                                          selMode)
                return
            }
            // 已有选区时按在选区内部 = 移动选区（不动像素）。带 Shift/Alt
            // 说明想加选/减选，那就照常新建，不进移动手势。
            if (host.selExists && selMode === 0
                    && host.selTool !== "polygon" && host.selTool !== "none") {
                const pi = toImage(mouse.x, mouse.y)
                if (bridge.sel_contains(Math.floor(pi[0]), Math.floor(pi[1]))) {
                    // Ctrl + 按在选区内 = 剪刀（PS 语义）：把选区里的像素剪成
                    // 独立图层，接着直接进入图层拖动。剪完把选区清掉 —— 像素
                    // 已经搬走了，留一个空框在原地只会让人以为还能拖它。
                    if ((mouse.modifiers & Qt.ControlModifier) !== 0
                            && bridge.cut_selection_to_layer()) {
                        bridge.sel_clear()
                        gestureMods = mouse.modifiers
                        liveMods = mouse.modifiers
                        pickDrag = true
                        pickMoved = false
                        startX = mouse.x
                        startY = mouse.y
                        curX = mouse.x
                        curY = mouse.y
                        return
                    }
                    moveSelActive = true
                    moveFromX = mouse.x
                    moveFromY = mouse.y
                    selOffX = 0
                    selOffY = 0
                    return
                }
            }
            // 多边形套索：一次点击加一个顶点，双击 / 点回起点 / 回车闭合。
            if (host.selTool === "polygon") {
                dropLayerForSelection(mouse.x, mouse.y)
                addPolygonPoint(mouse.x, mouse.y)
                return
            }
            if (host.selTool === "none") {
                // 没选任何选区工具：点空白 = 退出选择（旧行为）。
                clickEmptyCanvas()
                return
            }
            // 框选 / 套索起手：图层框外起手就先退出图层选中（见函数注释）。
            dropLayerForSelection(mouse.x, mouse.y)
            startX = mouse.x
            startY = mouse.y
            curX = mouse.x
            curY = mouse.y
            lassoPts = []
            if (host.selTool === "lasso")
                lassoPts = [toImage(mouse.x, mouse.y)]
            dragging = true
            selPreview.requestPaint()
        }

        onDoubleClicked: (mouse) => {
            if (mouse.button !== Qt.LeftButton)
                return
            if (host.selTool === "polygon" && selPolyActive)
                commitPolygon()
        }

        onPositionChanged: (mouse) => {
            if (host.rulerOn)
                host.noteHover(mouse.x, mouse.y)
            // ★图层拖动排在最前：它是唯一一条「按下就锁定了目标图层」的手势，
            //   一旦 pickDrag 为真，别的分支都不能再抢这次移动。以前它在
            //   moveSelActive / selPolyActive 之后 —— 那两个标志只要因为丢过
            //   一次 release（切窗口、Alt+Tab、弹窗抢焦点）而残留为真，
            //   后续每次拖动都会被它们吃掉：鼠标在动，只有选区框在跑，
            //   图层一步不动（用户报的正是这个）。
            if (pickDrag) {
                // 修饰键实时刷新（见 liveMods 注释）：吸附的临时关闭要在
                // 「按住 Ctrl」这一刻就生效，而不是等下一次按下。
                liveMods = mouse.modifiers
                curX = mouse.x
                curY = mouse.y
                if (!pickMoved
                    && Math.abs(curX - startX) + Math.abs(curY - startY) > 3) {
                    pickMoved = true
                    lastDx = 0
                    lastDy = 0
                    // P0-2b2：三件套统一入口（内部仍拉起旧 live 预览态）。
                    // 坐标是图像像素，mods 原样透传，吸附由 Rust 求解。
                    const p0 = toImage(startX, startY)
                    bridge.transform_begin(0, bridge.selected_layer,
                                           p0[0], p0[1],
                                           gestureMods, host.toDispScale())
                    // Start ONCE and keep repeating: restarting the
                    // timer on every mouse-move event would reset the
                    // countdown faster than it can expire (mouse
                    // events arrive at ~125Hz), so fast drags never
                    // rendered live.
                    liveTimer.start()
                }
                return
            }
            // 修补工具：第一段收顶点，第二段拖动取样偏移。
            if (patchDrawing) {
                const pp = toImage(mouse.x, mouse.y)
                const last = patchPts.length > 0
                             ? patchPts[patchPts.length - 1] : null
                if (!last || Math.abs(pp[0] - last[0]) + Math.abs(pp[1] - last[1]) >= 1)
                    patchPts = patchPts.concat([[pp[0], pp[1]]])
                patchViz.requestPaint()
                return
            }
            if (patchDragging) {
                patchDx = mouse.x - patchStartX
                patchDy = mouse.y - patchStartY
                const s = Math.max(host.toDispScale(), 0.0001)
                bridge.patch_drag_move(patchDx / s, patchDy / s)
                patchViz.requestPaint()
                return
            }
            if (paintStroke) {
                const pp = toImage(mouse.x, mouse.y)
                if (host.paintTool === "gradient")
                    bridge.gradient_move(pp[0], pp[1])
                else
                    bridge.retouch_move(pp[0], pp[1])
                return
            }
            // 快速选择涂抹：每一步都送一段过去，选区跟着笔尖长。
            if (quickDrag) {
                const pw = toImage(mouse.x, mouse.y)
                bridge.quick_select_brush(pw[0], pw[1], host.quickRadius,
                                          host.quickTolerance, host.quickGrow,
                                          selMode)
                return
            }
            // 多边形套索的橡皮筋跟随光标（不需要按住）。
            if (selPolyActive) {
                polyCurX = mouse.x
                polyCurY = mouse.y
                selPreview.requestPaint()
                return
            }
            if (moveSelActive) {
                selOffX = mouse.x - moveFromX
                selOffY = mouse.y - moveFromY
                return
            }
            if (!dragging)
                return
            curX = mouse.x
            curY = mouse.y
            if (host.selTool === "lasso") {
                const p = toImage(curX, curY)
                const last = lassoPts.length > 0
                             ? lassoPts[lassoPts.length - 1] : null
                // 丢太近的点没用：套索常见几万个采样，送去 Rust 的是
                // 直线段近似，1px 以内直接合并。
                if (!last || Math.abs(p[0] - last[0]) + Math.abs(p[1] - last[1]) >= 1)
                    lassoPts = lassoPts.concat([[p[0], p[1]]])
            }
            selPreview.requestPaint()
        }

        onReleased: (mouse) => {
            // ★和 onPositionChanged 同一条铁律：图层拖动一进一出必须配对。
            //   松手时先收它，别让残留的选区 / 画笔状态把提交吞掉 ——
            //   吞掉的后果是图层停在旧位置，而框（来自文档 offset 的 JSON）
            //   已经跟着鼠标走完了整段。
            if (pickDrag) {
                pickDrag = false
                if (pickMoved) {
                    pickMoved = false
                    liveTimer.stop()
                    // ★框不额外加本地偏移：drag_translate_update 每次都会改
                    //   文档里的 layer.offset 并重新发布 layers_json，QML 的
                    //   selLayerBox / 8 点 / 旋柄 / 徽标本来就跟着走。上一版
                    //   又叠了一份鼠标位移，等于两份位移相加 —— 拖得越远，
                    //   框和内容分得越开（用户截图里的"图层渲染跑到框外"）。
                    // P0-2b2：最终帧也走统一会话，保证吸附修正量带进落定。
                    // ★落定这一帧用**松手事件自己的坐标**，不要用 curX/curY：
                    //   curX 只在 onPositionChanged 里更新，Qt 会把最后几个
                    //   move 合并 / 丢掉（GPU 纹理生成时尤其明显），于是图层
                    //   停在「最后一帧收到的 move」上 —— 实测能差 11 图像像素，
                    //   刚好把 8px 的吸附窗口跳过去（用户看到「差一点点吸不进
                    //   去」）。松手事件总是带真实坐标，用它最准。
                    curX = mouse.x
                    curY = mouse.y
                    const pf = toImage(mouse.x, mouse.y)
                    if (Qt.application.arguments.indexOf("--probe-keys") !== -1) {
                        const p0 = toImage(startX, startY)
                        bridge.mark("move-release: from=" + Math.round(p0[0]) + ","
                                    + Math.round(p0[1]) + " to=" + Math.round(pf[0]) + ","
                                    + Math.round(pf[1]) + " layer=" + bridge.selected_layer)
                    }
                    bridge.transform_update(0, pf[0], pf[1],
                                            liveMods, host.toDispScale())
                    bridge.drag_translate_end(true)
                    bridge.transform_end(true, liveMods)
                    if (host.pasteFloating)
                        host.floatingOps += 1
                }
                return
            }
            if (patchDrawing) {
                patchDrawing = false
                if (patchPts.length >= 3) {
                    patchRegion = patchPts
                    const text = patchPts
                        .map(q => q[0].toFixed(1) + "," + q[1].toFixed(1))
                        .join(" ")
                    bridge.patch_set_region(text)
                } else {
                    patchRegion = []
                    bridge.patch_clear_region()
                }
                patchPts = []
                patchViz.requestPaint()
                return
            }
            if (patchDragging) {
                patchDragging = false
                const applied = bridge.patch_drag_end()
                if (applied)
                    patchRegion = []
                patchDx = 0
                patchDy = 0
                patchViz.requestPaint()
                return
            }
            if (paintStroke) {
                paintStroke = false
                if (host.paintTool === "gradient")
                    bridge.gradient_end()
                else
                    bridge.retouch_end()
                return
            }
            if (quickDrag) {
                quickDrag = false
                // 松手才把整段涂抹记成一条撤销记录。
                bridge.quick_select_commit()
                return
            }
            if (mouse.button !== Qt.LeftButton)
                return
            if (moveSelActive) {
                moveSelActive = false
                const sc = Math.max(host.toImageScale(), 0.0001)
                const mdx = Math.round(selOffX * sc)
                const mdy = Math.round(selOffY * sc)
                // 先归零再提交：同一个事件里两次重绘会被合并成一帧，
                // 不会出现「新位置 + 旧偏移」的双倍位移闪烁。
                selOffX = 0
                selOffY = 0
                if (mdx !== 0 || mdy !== 0)
                    bridge.sel_translate(mdx, mdy)
                return
            }
            if (pickDrag) {
                pickDrag = false
                pickMoved = false
                return
            }
            if (!dragging)
                return
            dragging = false
            const a = toImage(startX, startY)
            const b = toImage(curX, curY)
            const dx = Math.abs(b[0] - a[0])
            const dy = Math.abs(b[1] - a[1])
            if (host.selTool === "lasso") {
                // 判"是否真的拖出了面积"必须用轨迹包围盒：画闭环时起点
                // ≈ 终点，用端点距离会把套索误判成单击（然后清空选区）。
                var lx0 = Infinity, ly0 = Infinity, lx1 = -Infinity, ly1 = -Infinity
                for (var li = 0; li < lassoPts.length; li++) {
                    lx0 = Math.min(lx0, lassoPts[li][0])
                    ly0 = Math.min(ly0, lassoPts[li][1])
                    lx1 = Math.max(lx1, lassoPts[li][0])
                    ly1 = Math.max(ly1, lassoPts[li][1])
                }
                if (lassoPts.length >= 3 && (lx1 - lx0) >= 2 && (ly1 - ly0) >= 2) {
                    const text = lassoPts
                        .map(p => p[0].toFixed(1) + "," + p[1].toFixed(1))
                        .join(" ")
                    bridge.sel_set_polygon(text, selMode)
                } else {
                    // 套索单击（没拖动）= 点空白。
                    clickEmptyCanvas()
                }
            } else if (host.selTool === "rect" || host.selTool === "ellipse") {
                if (dx < 0.5 || dy < 0.5) {
                    // 点击（没拖出面积）= 点空白。
                    clickEmptyCanvas()
                } else if (host.selTool === "rect") {
                    bridge.sel_set_rect(a[0], a[1], b[0], b[1], selMode)
                } else {
                    bridge.sel_set_ellipse(a[0], a[1], b[0], b[1], selMode)
                }
            }
            lassoPts = []
            selPreview.requestPaint()
        }

        // 丢失鼠标捕获（Alt+Tab / 系统弹窗抢焦点 / 另一个 item 抢走手势）时
        // 不会来 onReleased。不在这里复位，pickDrag / moveSelActive 就会一直
        // 为真，下一次在画布上拖动会被它们整段吃掉 —— 用户看到的就是"框被
        // 拖走了，图层一步没动"。图层位移这时按取消处理，回滚到拖之前。
        onCanceled: {
            if (pickDrag) {
                pickDrag = false
                pickMoved = false
                liveTimer.stop()
                bridge.drag_translate_end(false)
                bridge.transform_end(false, gestureMods)
            }
            if (moveSelActive) {
                moveSelActive = false
                selOffX = 0
                selOffY = 0
            }
            if (quickDrag)
                cancelQuickSelect()
            if (paintStroke)
                cancelPaint()
            if (dragging) {
                dragging = false
                lassoPts = []
                selPreview.requestPaint()
            }
        }
    }

    // 框选 / 套索进行中的预览（画布坐标，松手后由 Rust 的选区接管）。
    // 与 markPreview 同一套打法：Immediate + Image + 16ms 补拍，否则
    // 场景静止时单次 requestPaint 会被丢掉，预览冻结在早期帧。
    Canvas {
        id: selPreview
        anchors.fill: parent
        visible: selDragMouse.dragging || selPolyActive
        z: 6
        renderStrategy: Canvas.Immediate
        renderTarget: Canvas.Image
        // 蚂蚁线相位：由 selPreviewTimer 每帧推进，虚线看起来在流动。
        property real antPhase: 0
        // 用当前路径画一条蚂蚁线（白实线打底 + 黑虚线）。路径不能清，
        // 所以这里只动描边状态，两次 stroke() 复用同一段 path。
        function strokeAnts(ctx) {
            ctx.lineWidth = 1
            ctx.setLineDash([])
            ctx.strokeStyle = "#ffffff"
            ctx.stroke()
            ctx.setLineDash([4, 4])
            ctx.lineDashOffset = -antPhase
            ctx.strokeStyle = "rgba(0,0,0,0.9)"
            ctx.stroke()
            ctx.setLineDash([])
            ctx.lineDashOffset = 0
        }
        onPaint: {
            try {
                var ctx = getContext("2d")
                ctx.clearRect(0, 0, width, height)
                paintImpl(ctx)
            } catch (e) {
                bridge.mark("SELPAINT-ERR " + e)
            }
        }
        function paintImpl(ctx) {
            // 预览线一律走「白实线打底 + 黑虚线叠在上面」的蚂蚁线：纯黑虚线
            // 在深色照片上看不见（原来是 #000000 单色，全黑图上等于没画）。
            ctx.fillStyle = "rgba(255,255,255,0.08)"
            if (selPolyActive) {
                if (polyPts.length === 0)
                    return
                const s = Math.max(host.toDispScale(), 0.0001)
                const px = v => view.pad + v * s
                ctx.beginPath()
                ctx.moveTo(px(polyPts[0][0]), px(polyPts[0][1]))
                for (var i = 1; i < polyPts.length; i++)
                    ctx.lineTo(px(polyPts[i][0]), px(polyPts[i][1]))
                ctx.lineTo(polyCurX, polyCurY)
                strokeAnts(ctx)
                // 顶点小方块：方便看清已提交的点在哪。
                ctx.fillStyle = "#ffffff"
                ctx.strokeStyle = "#000000"
                for (var j = 0; j < polyPts.length; j++) {
                    const vx = px(polyPts[j][0])
                    const vy = px(polyPts[j][1])
                    ctx.fillRect(vx - 2, vy - 2, 4, 4)
                    ctx.strokeRect(vx - 2, vy - 2, 4, 4)
                }
                return
            }
            if (!selDragMouse.dragging)
                return
            const x0 = selDragMouse.startX
            const y0 = selDragMouse.startY
            const x1 = selDragMouse.curX
            const y1 = selDragMouse.curY
            if (host.selTool === "lasso") {
                if (selDragMouse.lassoPts.length < 2)
                    return
                const s2 = Math.max(host.toDispScale(), 0.0001)
                const qx = v => view.pad + v * s2
                ctx.beginPath()
                ctx.moveTo(qx(selDragMouse.lassoPts[0][0]),
                           qx(selDragMouse.lassoPts[0][1]))
                for (var k = 1; k < selDragMouse.lassoPts.length; k++)
                    ctx.lineTo(qx(selDragMouse.lassoPts[k][0]),
                               qx(selDragMouse.lassoPts[k][1]))
                strokeAnts(ctx)
                return
            }
            if (host.selTool === "ellipse") {
                const ecx = (x0 + x1) / 2
                const ecy = (y0 + y1) / 2
                const erx = Math.abs(x1 - x0) / 2
                const ery = Math.abs(y1 - y0) / 2
                if (erx < 0.5 || ery < 0.5)
                    return
                // Context2D.ellipse 的 (x,y) 是外接矩形左上角（不是圆心），
                // 用单位圆 + 缩放构造最稳（见 markPreview 的同类注释）。
                ctx.save()
                ctx.translate(ecx, ecy)
                ctx.scale(erx, ery)
                ctx.beginPath()
                ctx.arc(0, 0, 1, 0, Math.PI * 2)
                ctx.restore()
                strokeAnts(ctx)
                return
            }
            const rx = Math.min(x0, x1)
            const ry = Math.min(y0, y1)
            ctx.fillRect(rx, ry, Math.abs(x1 - x0), Math.abs(y1 - y0))
            // 描边走路径（strokeRect 只能一种颜色），才能白底 + 黑虚线。
            ctx.beginPath()
            ctx.rect(rx, ry, Math.abs(x1 - x0), Math.abs(y1 - y0))
            strokeAnts(ctx)
        }
    }

    Timer {
        id: selPreviewTimer
        interval: 16
        repeat: true
        running: selDragMouse.dragging || selPolyActive
        onTriggered: {
            // 虚线相位推进（0.25px/帧 ≈ 15px/s，和已提交选区的蚂蚁线同速）。
            selPreview.antPhase = (selPreview.antPhase + 0.25) % 8
            selPreview.requestPaint()
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
            // 同上：标记手势入口也要放开浮层输入框的焦点。
            host.dropPopoverFocus()
            // 探针：把「画布坐标 → 图像坐标」的实算结果打出来。
            // scripts/probe_mark_delete.py 用它反推合成鼠标消息与窗口像素
            // 之间的固定偏移（两者差 60~125px，不校准就会漏点小元素）。
            if (Qt.application.arguments.indexOf("--probe-keys") !== -1)
                bridge.mark("mark-press: mx=" + Math.round(mouse.x)
                            + " my=" + Math.round(mouse.y)
                            + " ix=" + Math.round(imgX(mouse.x))
                            + " iy=" + Math.round(imgY(mouse.y))
                            + " pad=" + Math.round(view.pad)
                            + " hit=" + markAt(mouse.x, mouse.y)
                            + " handle=" + handleAt(mouse.x, mouse.y)
                            + " tool=" + host.markTool)
            // 画笔按住 Alt = 临时吸管（PS 惯例）：取画布颜色作为下一笔的颜色，
            // 不落笔。光标已经由 EditorContent 换成吸管（cursorKind 5）。
            if (host.markTool === "brush"
                    && (mouse.modifiers & Qt.AltModifier) !== 0) {
                const hex = bridge.canvas_color_at(Math.floor(imgX(mouse.x)),
                                                   Math.floor(imgY(mouse.y)))
                if (hex !== "")
                    host.markColor = hex
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
            // 清焦点：隐藏后的 TextInput 若仍 activeFocus，会继续吃 Delete
            // 与 Ctrl+Z（QML Shortcut 的 enabled 也引用了 textPopup.visible，
            // 但残留焦点会让"点别处再按键"这一路失灵）。
            focus = false
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
            // 上一帧还没上屏就先等：连续换 source 会把后台解码取消重来，
            // 预览会一直停在旧帧（表现为"拖动时画面不动/闪没"）。
            // Tier 1 不产生新帧，无需等后台解码；否则帧率被旧闸门拖慢。
            if (!bridge.gpuLive && previewImage.previewBusy())
                return
            const scale = host.toImageScale()
            const dx = Math.round((selDragMouse.curX - selDragMouse.startX) * scale)
            const dy = Math.round((selDragMouse.curY - selDragMouse.startY) * scale)
            if (dx === selDragMouse.lastDx && dy === selDragMouse.lastDy)
                return
            selDragMouse.lastDx = dx
            selDragMouse.lastDy = dy
            const p = toImage(selDragMouse.curX, selDragMouse.curY)
            bridge.transform_update(0, p[0], p[1],
                                    selDragMouse.liveMods, host.toDispScale())
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
            // P1-A：斜切量 / 锚定补偿由 transform_solve_scale 解出后存在
            // host 属性上，这里原样透传（非斜切手势恒 0）。
            bridge.scale_layer_update(w, h, x, y, host.scaleFlipX, host.scaleFlipY,
                                      host.scaleSkewX, host.scaleSkewY,
                                      host.scaleCenterDx, host.scaleCenterDy)
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

    // 修补工具的轮廓：第一段画正在圈的折线，第二段画跟着拖动偏移的区域。
    // 虚线样式与选区完全一致（白实线打底 + 黑虚线 + 相位流动的蚂蚁线）：
    // 浅色照片上看得出边界，深色照片上也不会糊成一片。
    Canvas {
        id: patchViz
        anchors.fill: parent
        z: 500
        visible: host.paintTool === "patch"
                 && (patchDrawing || patchRegion.length >= 3)
        renderStrategy: Canvas.Immediate
        antialiasing: true
        // 蚂蚁线相位：由 patchAntTimer 推进，虚线看起来在流动。
        property real antPhase: 0

        function strokeAnts(ctx) {
            ctx.lineWidth = 1
            ctx.setLineDash([])
            ctx.strokeStyle = "#ffffff"
            ctx.stroke()
            ctx.setLineDash([4, 4])
            ctx.lineDashOffset = -antPhase
            ctx.strokeStyle = "rgba(0,0,0,0.9)"
            ctx.stroke()
            ctx.setLineDash([])
            ctx.lineDashOffset = 0
        }

        onPaint: {
            const ctx = getContext("2d")
            ctx.clearRect(0, 0, width, height)
            const s = Math.max(host.toDispScale(), 0.0001)
            const pts = patchDrawing ? patchPts : patchRegion
            if (!pts || pts.length < 2)
                return
            const ox = view.pad + (patchDrawing ? 0 : patchDx)
            const oy = view.pad + (patchDrawing ? 0 : patchDy)
            ctx.beginPath()
            ctx.moveTo(pts[0][0] * s + ox, pts[0][1] * s + oy)
            for (let i = 1; i < pts.length; i++)
                ctx.lineTo(pts[i][0] * s + ox, pts[i][1] * s + oy)
            if (!patchDrawing)
                ctx.closePath()
            strokeAnts(ctx)
        }
    }

    Timer {
        id: patchAntTimer
        interval: 16
        repeat: true
        running: host.paintTool === "patch"
                 && (patchDrawing || patchRegion.length >= 3)
        onTriggered: {
            patchViz.antPhase = (patchViz.antPhase + 0.25) % 8
            patchViz.requestPaint()
        }
    }

    // 快速选择的笔刷轮廓：跟着光标画一个圈，直观表示这一笔有多粗。
    // 直径 = 半径 × 2 × 显示比例（笔刷半径是画布像素，不是屏幕像素）。
    Rectangle {
        id: quickBrushRing
        visible: (host.selTool === "quick"
                  || host.paintTool === "spot" || host.paintTool === "heal")
                 && canvasHoverHandler.hovered
        readonly property real diam: Math.max(8, (host.selTool === "quick"
                                                  ? host.quickRadius * 2
                                                  : host.healSize) * host.toDispScale())
        width: diam
        height: diam
        radius: diam / 2
        x: canvasHoverHandler.point.position.x - diam / 2
        y: canvasHoverHandler.point.position.y - diam / 2
        color: "transparent"
        border.color: "#e9eaec"
        border.width: 1
        opacity: 0.8
        antialiasing: true
        z: 500
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
        onPointChanged: {
            if (host.rulerOn)
                host.noteHover(point.position.x, point.position.y)
            host.updateCursorKind(point.position.x, point.position.y)
        }
        onHoveredChanged: {
            if (!hovered) {
                host.clearHover()
                host.updateCursorKind(-1, -1)
            }
        }
    }

    // 只按住修饰键、鼠标不动时 HoverHandler 不会发 pointChanged，
    // 而 QML 也拿不到"纯修饰键"事件 —— 悬停期间只能定时问 Rust 要状态。
    // 120ms 够跟手，又不至于白烧 CPU。
    Timer {
        id: cursorModPoll
        interval: 120
        repeat: true
        running: canvasHoverHandler.hovered
        onTriggered: host.updateCursorKind(canvasHoverHandler.point.position.x,
                                           canvasHoverHandler.point.position.y)
    }
}
