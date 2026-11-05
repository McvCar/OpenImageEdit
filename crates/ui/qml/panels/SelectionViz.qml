import QtQuick
import com.imageedit

Item {
    id: selectionViz
    required property var host
    required property var bridge
    required property var previewImage
    z: 8
    // 暴露给 EditorContent：canvasCursor 读手柄几何算光标。
    readonly property alias layerHandles: layerHandles
    readonly property alias selHandles: selHandles

    // ---- 选区手柄（拖四边/四角改选区大小）----
    // 拖动期间只画本地预览框，松手才让 Rust 按新包围盒重采样 mask：
    // 每帧重采样整块选区在大图上会明显掉帧。
    property bool resizingSel: false
    property rect liveSelBox: Qt.rect(0, 0, 0, 0)

    // ---- 角色（nw/n/ne/e/se/s/sw/w）→ 新包围盒 ----
    // 手柄拖过对边时（负宽高）的方向标记：liveSelBox 始终是正尺寸的框，
    // 松开时用它把「镜像」告诉 Rust。
    property bool liveFlipX: false
    property bool liveFlipY: false

    function resizedSelBox(r, role, dx, dy) {
        let l = r.x, t = r.y
        let rr = r.x + r.width, bb = r.y + r.height
        // 允许拖过对边：两个边各自跟手，最后取 min/abs 得到正尺寸的框。
        // 钳制在预览区域内，否则框会跑到画布外，松手后选区被裁掉一半。
        if (role.indexOf("w") >= 0)
            l = Math.max(0, Math.min(Math.round(r.x + dx), previewImage.width))
        if (role.indexOf("e") >= 0)
            rr = Math.max(0, Math.min(Math.round(r.x + r.width + dx), previewImage.width))
        if (role.indexOf("n") >= 0)
            t = Math.max(0, Math.min(Math.round(r.y + dy), previewImage.height))
        if (role.indexOf("s") >= 0)
            bb = Math.max(0, Math.min(Math.round(r.y + r.height + dy), previewImage.height))
        return Qt.rect(Math.min(l, rr), Math.min(t, bb),
                        Math.abs(rr - l), Math.abs(bb - t))
    }

    // ---- 选区轮廓（蚂蚁线）--------------------------------------------
    // 轮廓真值在 Rust（mask 的 marching squares 轮廓，图像坐标），QML 只在
    // selVersion 变化时拉一次，避免每次缩放/重绘都跨语言跑几十万像素。
    Canvas {
        id: ants
        visible: host.selExists && !selectionViz.resizingSel
        // 画布比预览图大 1px：轮廓正好落在图像边缘时（全选 / 贴边选区），
        // 1px 描边有一半在 x=0 外被裁掉，左边那一条会"看不见"。
        x: previewImage.x - 1
        y: previewImage.y - 1
        width: previewImage.width + 2
        height: previewImage.height + 2
        // 蚂蚁线要在静止场景里持续流动，必须用 Immediate + Image
        //（Cooperative 策略会把单次 requestPaint 丢掉）。
        renderStrategy: Canvas.Immediate
        renderTarget: Canvas.Image
        antialiasing: false

        // 缓存的轮廓（图像坐标）：[[ [x,y], ... ], ...]
        property var rings: []
        property real phase: 0

        // selVersion 变化 = 选区真的变了（标量变化才触发，避免重复拉取）。
        Connections {
            target: bridge
            function onSelVersionChanged() { ants.reload() }
            function onImage_widthChanged() { ants.reload() }
        }
        // 缩放/窗口变化只影响绘制（坐标 × 比例），不用重新拉数据。
        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()
        Component.onCompleted: reload()
        // selExists 由 bridge.selActive 驱动：出现/消失时也刷新一次
        //（selVersion 之外的第二道保险，信号名对不上也不会不刷新）。
        Connections {
            target: host
            function onSelExistsChanged() { ants.reload() }
            // 拖动选区期间只有本地偏移在变，重绘一次就够（不必拉轮廓）。
            function onSelOffXChanged() { ants.requestPaint() }
            function onSelOffYChanged() { ants.requestPaint() }
        }

        function reload() {
            if (!host.selExists) {
                rings = []
                requestPaint()
                return
            }
            try {
                rings = JSON.parse(bridge.sel_outline())
            } catch (e) {
                bridge.mark("ANTSPARSE-ERR " + e)
                rings = []
            }
            requestPaint()
        }

        // 虚线相位：60ms 一跳，视觉上就是 PS 的蚂蚁线。
        Timer {
            interval: 60
            repeat: true
            running: ants.visible
            onTriggered: {
                ants.phase = (ants.phase + 1) % 8
                ants.requestPaint()
            }
        }

        onPaint: {
            try {
                var ctx = getContext("2d")
                ctx.clearRect(0, 0, width, height)
                if (!host.selExists || rings.length === 0)
                    return
                const s = host.toDispScale()
                if (s <= 0)
                    return
                // 拖动选区时的临时偏移（松手后由 Rust 平移 mask 接管）。
                // +1.5 = 画布相对预览图的 1px 外扩 + 0.5px 像素中心对齐。
                const ox = (host.selOffX || 0) + 1.5
                const oy = (host.selOffY || 0) + 1.5
                // PS 的蚂蚁线 = 白底 + 黑虚线：白段在黑底照片上可见，黑段
                // 在白底照片上可见。**先**铺白实线，再叠黑虚线（顺序反了
                // 就只剩黑虚线，浅色照片上整圈看不见）。
                ctx.lineWidth = 1
                ctx.setLineDash([])
                ctx.strokeStyle = "#ffffff"
                ctx.beginPath()
                for (var i = 0; i < rings.length; i++) {
                    const ring = rings[i]
                    if (!ring || ring.length < 2)
                        continue
                    ctx.moveTo(ring[0][0] * s + ox, ring[0][1] * s + oy)
                    for (var j = 1; j < ring.length; j++)
                        ctx.lineTo(ring[j][0] * s + ox, ring[j][1] * s + oy)
                    // 轮廓点是「左上 → 顺时针」一圈，最后一点到起点那一段
                    // 得自己闭合 —— 少了它，矩形左边整条蚂蚁线不见。
                    ctx.closePath()
                }
                ctx.stroke()
                // 上面再走一条黑色虚线，相位滚动 = 流动。
                ctx.setLineDash([4, 4])
                ctx.lineDashOffset = -phase
                ctx.strokeStyle = "rgba(0,0,0,0.9)"
                ctx.stroke()
                ctx.setLineDash([])
            } catch (e) {
                bridge.mark("ANTSPAINT-ERR " + e)
            }
        }
    }

    // 拖动选区手柄时的预览框（虚线 + 手柄跟随），松手后由蚂蚁线接管。
    DashedRect {
        id: selResizeViz
        visible: selectionViz.resizingSel
        z: 7
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        rx: selectionViz.liveSelBox.x
        ry: selectionViz.liveSelBox.y
        rw: selectionViz.liveSelBox.width
        rh: selectionViz.liveSelBox.height
        dashColor: "#ffffff"
    }

    // 选区手柄：8 个蓝点 + 四边可拖。moveEnabled 关掉 —— 拖内部移动由
    // 下层 CanvasSurface 的鼠标区处理（那里能一次性提交并走抖动优化）。
    EdgeHandles {
        id: selHandles
        visible: host.selExists && !host.selRubberBanding && !host.rulerOn
                 && host.selTool !== "none" && bridge.image_width > 0
                 && !bridge.pickerOpen && !host.selMenuOpen
                 && host.paintTool === "none"
        z: 9
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        // 拖动选区时手柄要跟着**本地偏移**一起走（bridge 里的 selX 还没变），
        // 否则只有蚂蚁线在跑，8 个圆点留在原地。
        bx: selectionViz.resizingSel ? selectionViz.liveSelBox.x
                                     : host.selX + (host.selOffX || 0)
        by: selectionViz.resizingSel ? selectionViz.liveSelBox.y
                                     : host.selY + (host.selOffY || 0)
        bw: selectionViz.resizingSel ? selectionViz.liveSelBox.width : host.selW
        bh: selectionViz.resizingSel ? selectionViz.liveSelBox.height : host.selH
        moveEnabled: false
        pxScale: host.toDispScale()
        resizeFn: function(startRect, role, dx, dy, free) {
            selectionViz.liveSelBox = selectionViz.resizedSelBox(
                        startRect, role, dx, dy)
            // 拖过对边 = 镜像：记下方向，松手时以负宽高交给 Rust。
            selectionViz.liveFlipX =
                (role.indexOf("w") >= 0 && startRect.x + dx >= startRect.x + startRect.width)
                || (role.indexOf("e") >= 0 && startRect.x + startRect.width + dx <= startRect.x)
            selectionViz.liveFlipY =
                (role.indexOf("n") >= 0 && startRect.y + dy >= startRect.y + startRect.height)
                || (role.indexOf("s") >= 0 && startRect.y + startRect.height + dy <= startRect.y)
            if (!selectionViz.resizingSel)
                selectionViz.resizingSel = true
        }
        onEngagedChanged: {
            if (selHandles.engaged) {
                // 按下手柄：清掉上一次的镜像状态，这一次越过对边才能重新触发。
                bridge.sel_resize_begin()
                return
            }
            if (!selectionViz.resizingSel)
                return
            selectionViz.resizingSel = false
            const s = Math.max(host.toDispScale(), 0.0001)
            const b = selectionViz.liveSelBox
            if (b.width >= 1 && b.height >= 1) {
                // 负宽高语义：Rust 侧把 `x` 当作被拖那条边的新位置，负的
                // width 表示框向左生长（即镜像），所以镜像时传对侧锚点。
                const ax = selectionViz.liveFlipX ? b.x + b.width : b.x
                const ay = selectionViz.liveFlipY ? b.y + b.height : b.y
                bridge.sel_resize(ax / s, ay / s,
                                  (selectionViz.liveFlipX ? -b.width : b.width) / s,
                                  (selectionViz.liveFlipY ? -b.height : b.height) / s)
            }
            selectionViz.liveFlipX = false
            selectionViz.liveFlipY = false
        }
    }

    // Selection size badge: W×H only, in a small gray rounded chip so it
    // stays readable over any image content.
    Rectangle {
        id: selBadge
        // 拖 8 个手柄改大小时也显示 —— 这时最需要看数字。数字跟着正在拖的
        // 预览框走（松手前 bridge 里的值还没变），所以不能只是解除隐藏。
        visible: host.selExists && !host.rulerOn && !host.selRubberBanding
        z: 8
        // host.selX/selY/selW/selH 已经是「显示坐标」（EditorContent 里
        // 由 selImg* × toDispScale() 算好），这里不要再乘一次比例。
        readonly property var curBox: selectionViz.resizingSel
            ? selectionViz.liveSelBox
            : ({ x: host.selX + (host.selOffX || 0),
                 y: host.selY + (host.selOffY || 0),
                 width: host.selW, height: host.selH })
        // 同样不钳画布：选区贴边/出画布时徽标跟着它走，别横向滑到别处。
        x: previewImage.x + selBadge.curBox.x
        y: previewImage.y + selBadge.curBox.y + selBadge.curBox.height + 4
        width: selBadgeLabel.implicitWidth + 12
        height: selBadgeLabel.implicitHeight + 6
        radius: 3
        color: "#cc43474d"
        border.color: "#6b7076"
        border.width: 1

        Text {
            id: selBadgeLabel
            anchors.centerIn: parent
            text: {
                if (selectionViz.resizingSel) {
                    const s = Math.max(host.toDispScale(), 0.0001)
                    return Math.round(Math.abs(selectionViz.liveSelBox.width) / s)
                         + " × "
                         + Math.round(Math.abs(selectionViz.liveSelBox.height) / s)
                         + (bridge.quickMask ? "　快速蒙版" : "")
                }
                return host.selImgW + " × " + host.selImgH
                     + (bridge.quickMask ? "　快速蒙版" : "")
            }
            color: "#d5d8dd"
            font.pixelSize: 11
        }
    }

    // ---- 选中图层的尺寸徽标（图层框左下角）--------------------------
    // 位置贴包围盒左下角，**允许画到画布外**（画布四周还有 PAD 的空白区）：
    // 以前被钳在 previewImage 里，框一贴边徽标就横着滑走，看着像是跳到了
    // 别的地方。背景也改成半透明，压在图案上不会糊成一块死黑。
    Rectangle {
        id: layerBadge
        // 拖八向手柄缩放时框每帧都在变，框下沿已经有蓝色的实时数字，
        // 这里让位（否则两串数字叠在一起）；旋转中同理。
        visible: host.selLayer !== null && host.scaleW === 0 && !host.rotating
                 && host.moveMode && !host.rulerOn
                 && layerBadge.anchor !== null
        z: 8
        readonly property var anchor: {
            const b = host.selLayerBox
            if (b === null)
                return null
            const s = host.toDispScale()
            // 轴对齐包围盒的左下角。位置真值就是文档里的图层 offset，
            // 拖动时 Rust 会实时刷新 layers_json，这里不需要（也不能）
            // 再叠一份鼠标位移。
            return { x: b.x * s, y: (b.y + b.h) * s }
        }
        readonly property real anchorX: anchor !== null ? anchor.x : 0
        readonly property real anchorY: anchor !== null ? anchor.y : 0
        // 旋转提交后位图被扩成外接矩形（600×400 转 45° → 708×708），照 box
        // 报会让人以为图案被放大了；这时报旋转前的真实尺寸（rotateFrame
        // 记着那一份）。用字符串一次算清，免得多个 int 属性各自的更新时机
        // 错开、文字停在旧值上。
        readonly property string sizeText: {
            const b = host.selLayerBox
            if (b === null)
                return ""
            // 只有「框确实是那个旋转角对应的外接矩形」时才算数：缩放/撤销之后
            // rotateFrame 可能还留着，但尺寸早变了，这时以实际 bounds 为准。
            const f = host.layerFrameRotated ? host.rotateFrame : null
            const w = f ? f.w : b.w
            const h = f ? f.h : b.h
            return "图层 " + Math.round(w) + " × " + Math.round(h)
        }

        // 不钳画布：超出 previewImage 也照常画（PAD 区域里看得见）。
        x: previewImage.x + anchorX
        y: previewImage.y + anchorY + 4
        width: layerBadgeLabel.implicitWidth + 12
        height: layerBadgeLabel.implicitHeight + 6
        radius: 3
        color: "#cc43474d"
        border.color: "#6b7076"
        border.width: 1

        Text {
            id: layerBadgeLabel
            anchors.centerIn: parent
            text: layerBadge.sizeText
            color: "#d5d8dd"
            font.pixelSize: 11
        }
    }

    // Selected layer outline (dashed, axis-aligned bounding box) + handles.
    // ★只留这一圈框：旋转过之后也只是包围盒（虚线），不再叠一层"跟着
    //   内容倾斜的四边形"——两层框 + 一套圆点挤在一起，根本分不清哪个
    //   能拖、哪个只是参考线。
    DashedRect {
        id: layerBoxViz
        visible: host.selLayer !== null && host.scaleW === 0 && host.moveMode
        z: 6
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        rx: host.selLayerBox !== null ? host.selLayerBox.x * host.toDispScale() : 0
        ry: host.selLayerBox !== null ? host.selLayerBox.y * host.toDispScale() : 0
        rw: host.selLayerBox !== null ? host.selLayerBox.w * host.toDispScale() : 0
        rh: host.selLayerBox !== null ? host.selLayerBox.h * host.toDispScale() : 0
        dashColor: "#5b9dff99"
        animated: false
    }

    EdgeHandles {
        id: layerHandles
        // 拾色器打开时隐藏（取色优先于缩放/移动）。
        // ★旋转过之后手柄**照样在**（以前这里挂了 !layerFrameRotated，
        //   结果转一下 8 个圆点就全没了，用户以为工具坏了）。旋转后位图
        //   已经烘焙成外接矩形，手柄贴的就是那个矩形，上面那条淡框负责
        //   把手柄接住；倾斜虚线框仍然标出内容的真实四边。
        // 移动能力与工具**共存**：画笔 / 修补 / 油漆 / 选框激活时，只要图层
        // 还选着，手柄就在（和改造前一致）。想收掉手柄就按 V 关掉移动工具，
        // 或者取消选择（Esc / 移动模式下点空白）。
        // 只有快速选择 / 魔术棒例外：它们是「按住就涂」的手势，手柄压在
        // 图层四角会抢走按下事件；PS 里这两个工具也没有变换框。
        visible: host.selLayer !== null && host.selLayer.base !== true
                 && host.moveMode
                 && !host.selExists && bridge.image_width > 0
                 && !bridge.pickerOpen
                 && host.selTool !== "quick" && host.selTool !== "wand"
                 && !host.rotating
        z: 8
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        bx: host.scaleW > 0 ? host.scaleX
                            : (host.selLayerBox !== null ? host.selLayerBox.x * host.toDispScale() : 0)
        by: host.scaleW > 0 ? host.scaleY
                            : (host.selLayerBox !== null ? host.selLayerBox.y * host.toDispScale() : 0)
        bw: host.scaleW > 0 ? host.scaleW
                            : (host.selLayerBox !== null ? host.selLayerBox.w * host.toDispScale() : 0)
        bh: host.scaleW > 0 ? host.scaleH
                            : (host.selLayerBox !== null ? host.selLayerBox.h * host.toDispScale() : 0)
        moveEnabled: false
        pxScale: host.toDispScale()
        resizeFn: function(startRect, role, dx, dy, free) {
            host.resizeLayerFrom(startRect, role, dx, dy, free)
        }
        onEngagedChanged: {
            if (layerHandles.engaged) {
                if (host.scaleW === 0)
                    bridge.scale_layer_begin()
            } else if (host.scaleW > 0) {
                host.commitLayerScale()
            }
        }
    }

    // ---- 图层旋转手柄：框上方的小圆点，拖着绕图层中心转 ----------
    // 旋转期间位图是实时重采样的（Rust 侧每 60ms 一帧），松手才写历史。
    Item {
        id: rotateHandleBox
        // 和缩放手柄一样与工具共存；快速选择 / 魔术棒下让位（同上）。
        visible: host.selLayer !== null && host.selLayer.base !== true
                 && host.moveMode
                 && !host.selExists && bridge.image_width > 0
                 && !bridge.pickerOpen && host.scaleW === 0
                 && host.selTool !== "quick" && host.selTool !== "wand"
        z: 10
        // 手柄被藏起来时鼠标不会再发 onExited（它已经不接收 hover 了），
        // rotateKnobHover 会卡在 true，光标就永远停在旋转图标上。
        onVisibleChanged: if (!visible) host.rotateKnobHover = false
        readonly property real s: host.toDispScale()
        readonly property real bx: host.selLayerBox !== null ? host.selLayerBox.x * s : 0
        readonly property real by: host.selLayerBox !== null ? host.selLayerBox.y * s : 0
        readonly property real bw: host.selLayerBox !== null ? host.selLayerBox.w * s : 0
        readonly property real bh: host.selLayerBox !== null ? host.selLayerBox.h * s : 0
        // 旋柄**固定**挂在包围盒顶边中点，不跟着倾斜的内容走。
        // 之前试过让它贴着旋转后的顶边（中心 + 旋转后的 (0,-h/2)），结果
        // 转过之后旋柄会飘到斜边上、离手柄组越来越远，抓起来反而别扭；
        // 而且旋转过程中角度一变它就挪位置，手感发飘。回到固定上方。
        readonly property real knobX: bx + bw / 2
        readonly property real knobY: by
        x: previewImage.x + knobX
        y: previewImage.y + knobY - 22

        // 连到框顶的细线
        Rectangle {
            x: -0.5
            y: 8
            width: 1
            height: 14
            color: "#5b9dff"
        }
        Rectangle {
            id: rotateKnob
            width: 14
            height: 14
            radius: 7
            x: -width / 2
            y: -3
            color: host.rotating ? "#7fc3ff" : "#2f4a6b"
            border.color: "#cfe4ff"
            border.width: 1

            MouseArea {
                id: rotateMa
                // 命中范围比圆点大一圈，不然很难点中。
                anchors.fill: parent
                anchors.margins: -5
                // ★cursorShape 必须是 undefined：这个 MouseArea 一旦写上
                //   Qt.CrossCursor，Qt 会在鼠标进入时用 SetCursor 把它刷上去，
                //   正好盖掉 Rust 侧刚设好的旋转光标（WM_SETCURSOR 只在鼠标
                //   移动时才能改回来）—— 表现就是"第一次悬停不显示，动一下
                //   或者转过一次之后才显示"。光标统一由 canvasCursor 的
                //   Win32 通道接管，这里不参与。
                cursorShape: undefined
                hoverEnabled: true
                preventStealing: true
                property real startAngle: 0
                // 旋转中心在按下那一刻就锁死：图层一扩边界，框会跟着变大，
                // 每帧重算中心会让同一个鼠标位置算出不同角度，手感像在抖。
                property real pivotX: 0
                property real pivotY: 0
                // 悬停 → 旋转光标；拖动中即使滑出圆点也保持住。
                onEntered: {
                    host.rotateKnobHover = true
                    // Qt 的 HoverHandler 可能在同一帧里更新 cursorShape 并把
                    // 光标刷成十字，把我们的覆盖掉。next tick 再钉一次。
                    Qt.callLater(function() {
                        if (rotateMa.containsMouse)
                            bridge.cursor_refresh()
                    })
                }
                onExited: if (!pressed) host.rotateKnobHover = false
                onPressed: (mouse) => {
                    host.rotateKnobHover = true
                    pivotX = rotateHandleBox.bx + rotateHandleBox.bw / 2
                    pivotY = rotateHandleBox.by + rotateHandleBox.bh / 2
                    const p = mapToItem(previewImage, mouse.x, mouse.y)
                    startAngle = Math.atan2(p.y - pivotY, p.x - pivotX) * 180 / Math.PI
                    host.startLayerRotate()
                }
                onPositionChanged: (mouse) => {
                    if (!pressed)
                        return
                    const p = mapToItem(previewImage, mouse.x, mouse.y)
                    const a = Math.atan2(p.y - pivotY, p.x - pivotX) * 180 / Math.PI
                    let d = a - startAngle
                    // 跨 ±180 时不要跳一整圈。
                    while (d > 180) d -= 360
                    while (d < -180) d += 360
                    host.previewLayerRotate(d)
                }
                onReleased: {
                    host.finishLayerRotate(true)
                    host.rotateKnobHover = containsMouse
                }
                onCanceled: {
                    // ★丢失鼠标捕获（滑出窗口 / 焦点被抢 / 触控板丢帧）时 Qt
                    //   只发 canceled，不会发 released。以前这里按「取消」处理：
                    //   用户已经转好的角度被回滚（"放手后复位"），Rust 还会顺手
                    //   丢掉 rotate_origin 锚点 —— 下一次旋转只能从已经烘焙过
                    //   的外接矩形位图起手，框就越转越大、图越转越糊。
                    //   现在按「提交当前角度」处理：真没动过就是 neutral，
                    //   不会产生历史，也不会清锚点。
                    host.finishLayerRotate(true)
                    host.rotateKnobHover = false
                }
            }
        }
    }

    Text {
        visible: host.scaleW > 0
        z: 8
        x: previewImage.x + host.scaleX
        y: Math.min(previewImage.y + previewImage.height - 16,
                    previewImage.y + host.scaleY + host.scaleH + 4)
        // 拖动时只报当前尺寸。原来后面还挂一句「松手应用缩放 · Esc 取消」，
        // 跟着鼠标一路晃、又长又容易被形状挡住——操作说明放状态栏就够了。
        text: Math.round(Math.abs(host.scaleW) / host.toDispScale()) + " × "
              + Math.round(Math.abs(host.scaleH) / host.toDispScale())
        color: "#7fc3ff"
        font.pixelSize: 11
    }
}
