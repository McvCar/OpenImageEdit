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

    // ---- 旋转后的图层框（倾斜四边形）----
    // 提交旋转时位图被扩成最小外接矩形，四角是透明像素；画轴对齐矩形就会
    // 显得"框一下变大了"（900×700 转 45° → 1132×1132）。这里按"旋转前的
    // 尺寸 + 累计角度"算出四个角，虚线框正好贴着旋转后的内容边走。
    readonly property var layerQuad: {
        if (!host.layerFrameRotated || host.selLayerBox === null)
            return null
        const w = host.layerFrameW
        const h = host.layerFrameH
        if (!(w > 0) || !(h > 0))
            return null
        const s = host.toDispScale()
        const b = host.selLayerBox
        const cx = (b.x + b.w / 2) * s
        const cy = (b.y + b.h / 2) * s
        const hw = w * s / 2
        const hh = h * s / 2
        const a = host.layerFrameAngle * Math.PI / 180
        const ca = Math.cos(a)
        const sa = Math.sin(a)
        const out = []
        const px = [-hw, hw, hw, -hw]
        const py = [-hh, -hh, hh, hh]
        for (let i = 0; i < 4; ++i) {
            out.push(cx + px[i] * ca - py[i] * sa)
            out.push(cy + px[i] * sa + py[i] * ca)
        }
        return out
        return out
    }

    // 角色（nw/n/ne/e/se/s/sw/w）→ 新包围盒（显示坐标，钳在画布内）。
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
        bx: selectionViz.resizingSel ? selectionViz.liveSelBox.x : host.selX
        by: selectionViz.resizingSel ? selectionViz.liveSelBox.y : host.selY
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
        x: previewImage.x
           + Math.max(0, Math.min(selBadge.curBox.x,
               previewImage.width - selBadge.width))
        y: previewImage.y
           + Math.min(previewImage.height - selBadge.height,
                      selBadge.curBox.y + selBadge.curBox.height + 4)
        width: selBadgeLabel.implicitWidth + 12
        height: selBadgeLabel.implicitHeight + 6
        radius: 3
        color: "#43474d"
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

    // ---- 选中图层的尺寸徽标（图层描边左下角）--------------------------
    // 和选区尺寸徽标同一套视觉：灰底小圆角 chip，图片上任何内容都读得清。
    // 位置贴图层框的左下角：框底还有余量就画在框外 4px，贴到画布底部时
    // 翻到框内，避免被视口切掉。旋转过的框是倾斜四边形，取四角里最靠下
    // 的那个角（并列取更靠左的），跟着内容走而不是跟着大包围盒走。
    Rectangle {
        id: layerBadge
        // 拖八向手柄缩放时框每帧都在变，框下沿已经有蓝色的实时数字，
        // 这里让位（否则两串数字叠在一起）；旋转中同理。
        visible: host.selLayer !== null && host.scaleW === 0 && !host.rotating
                 && !host.rulerOn && host.paintTool === "none"
                 && layerBadge.anchor !== null
        z: 8
        readonly property var anchor: {
            const b = host.selLayerBox
            if (b === null)
                return null
            const s = host.toDispScale()
            const q = selectionViz.layerQuad
            if (q !== null && q.length === 8) {
                // 取四个角里最靠下的那个（并列取更靠左的）：轴对齐时正好是
                // 左下角，旋转后是内容的下尖角——视觉上始终贴着框的下沿。
                let bx = q[0]
                let by = q[1]
                for (let i = 1; i < 4; ++i) {
                    const x = q[i * 2]
                    const y = q[i * 2 + 1]
                    if (y > by + 0.5 || (Math.abs(y - by) <= 0.5 && x < bx)) {
                        bx = x
                        by = y
                    }
                }
                return { x: bx, y: by }
            }
            return { x: b.x * s, y: (b.y + b.h) * s }
        }
        readonly property real anchorX: anchor !== null ? anchor.x : 0
        readonly property real anchorY: anchor !== null ? anchor.y : 0
        // 倾斜框时那个角是尖的，徽标以它为中心才不会压住半边框。
        readonly property bool centered: selectionViz.layerQuad !== null
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

        x: previewImage.x + Math.max(0, Math.min(anchorX - (centered ? width / 2 : 0),
                                                previewImage.width - width))
        y: previewImage.y + (anchorY + 4 + height <= previewImage.height
                             ? anchorY + 4
                             : Math.max(0, anchorY - height - 4))
        width: layerBadgeLabel.implicitWidth + 12
        height: layerBadgeLabel.implicitHeight + 6
        radius: 3
        color: "#43474d"
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

    // Selected layer outline (dashed) + blue-dot handles for scaling.
    DashedRect {
        id: layerBoxViz
        visible: host.selLayer !== null && host.scaleW === 0
                 && host.paintTool === "none"
        z: 6
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        // 旋转过就画倾斜四边形（贴合内容），否则画普通的轴对齐框。
        quad: selectionViz.layerQuad
        rx: host.selLayerBox !== null ? host.selLayerBox.x * host.toDispScale() : 0
        ry: host.selLayerBox !== null ? host.selLayerBox.y * host.toDispScale() : 0
        rw: host.selLayerBox !== null ? host.selLayerBox.w * host.toDispScale() : 0
        rh: host.selLayerBox !== null ? host.selLayerBox.h * host.toDispScale() : 0
        dashColor: "#5b9dff99"
    }

    EdgeHandles {
        id: layerHandles
        // 拾色器打开时隐藏（取色优先于缩放/移动）。
        // 旋转过之后框变成倾斜四边形，八向缩放手柄还是轴对齐的，两者一起
        // 显示会互相"打架"，这时只留倾斜框和旋柄。
        // 快速选择 / 魔术棒激活时不显示：它们是「按住就涂」的手势，手柄压在
        // 图层四角会抢走按下事件；PS 里这两个工具也没有变换框。
        // （画布上的按下本身在 CanvasSurface.layerPickAt 里就不再拾取图层，
        //  选区工具点空白不会冒出包围盒。）
        visible: host.selLayer !== null && host.selLayer.base !== true
                 && !host.selExists && bridge.image_width > 0
                 && !bridge.pickerOpen && !host.layerFrameRotated
                 && host.paintTool === "none" && host.markTool === "none"
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
        visible: host.selLayer !== null && host.selLayer.base !== true
                 && !host.selExists && bridge.image_width > 0
                 && !bridge.pickerOpen && host.scaleW === 0
                 && host.paintTool === "none" && host.markTool === "none"
                 && host.selTool !== "quick" && host.selTool !== "wand"
        z: 10
        readonly property real s: host.toDispScale()
        readonly property real bx: host.selLayerBox !== null ? host.selLayerBox.x * s : 0
        readonly property real by: host.selLayerBox !== null ? host.selLayerBox.y * s : 0
        readonly property real bw: host.selLayerBox !== null ? host.selLayerBox.w * s : 0
        readonly property real bh: host.selLayerBox !== null ? host.selLayerBox.h * s : 0
        // 旋柄挂在框的顶边中点。旋转过之后顶边是斜的，用「中心 + 旋转后的
        // (0,-h/2)」算，旋柄才会跟着内容走，而不是钉在大包围盒的顶上。
        readonly property real knobX: {
            if (!host.layerFrameRotated || host.selLayerBox === null)
                return bx + bw / 2
            const b = host.selLayerBox
            const hh = host.layerFrameH * s / 2
            const a = host.layerFrameAngle * Math.PI / 180
            return (b.x + b.w / 2) * s + hh * Math.sin(a)
        }
        readonly property real knobY: {
            if (!host.layerFrameRotated || host.selLayerBox === null)
                return by
            const b = host.selLayerBox
            const hh = host.layerFrameH * s / 2
            const a = host.layerFrameAngle * Math.PI / 180
            return (b.y + b.h / 2) * s - hh * Math.cos(a)
        }
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
                // Qt 没有旋转光标，这里让 Rust 侧的 Win32 光标接管（kind 4）。
                cursorShape: Qt.CrossCursor
                hoverEnabled: true
                preventStealing: true
                property real startAngle: 0
                // 旋转中心在按下那一刻就锁死：图层一扩边界，框会跟着变大，
                // 每帧重算中心会让同一个鼠标位置算出不同角度，手感像在抖。
                property real pivotX: 0
                property real pivotY: 0
                // 悬停 → 旋转光标；拖动中即使滑出圆点也保持住。
                onEntered: host.rotateKnobHover = true
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
                    host.finishLayerRotate(false)
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
