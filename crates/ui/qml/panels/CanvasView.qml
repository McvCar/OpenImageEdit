import QtQuick
import com.imageedit

Flickable {
    id: view
    required property var host
    required property var bridge
    required property var markBar
    required property var toolbar
    required property var statusBar
    required property var layerPanel
    // 暴露给 EditorContent：根级缩放/选区函数需读 previewImage 几何；
    // 光标/探针/选区逻辑访问 canvasSurface 与 selectionViz 内部 id。
    readonly property alias previewImage: previewImage
    readonly property alias canvasSurface: canvasSurface
    readonly property alias selectionViz: selectionViz

    // 两行工具栏可能同时出现（标题栏 / 标记栏），取最下面那行的底。
    // 选区工具已经是标记栏里的浮层，不再占一行。
    anchors.top: markBar.visible ? markBar.bottom : toolbar.bottom
    anchors.bottom: statusBar.top
    anchors.left: parent.left
    // 图层面板悬浮在右侧：fit/居中必须按扣掉面板后的可见区域算，
    // 否则适应窗口时图片右边会被面板盖住。
    anchors.right: layerPanel.visible ? layerPanel.left : parent.right
    clip: true
    boundsBehavior: Flickable.StopAtBounds
    // Panning happens ONLY via wheel/touchpad and middle-drag (panMa);
    // left-drag belongs to selection/layer gestures.
    interactive: false
    // PAD display px of slack around the image so layers pushed partially
    // off-canvas stay visible and clickable (canvasSurface matches).
    readonly property int pad: 64
    contentWidth: Math.max(width, previewImage.width + 2 * pad)
    contentHeight: Math.max(height, previewImage.height + 2 * pad)

    // 画布显示尺寸没超出可视区的轴 = 不需要滚动。适应窗口时两轴都装得下
    // （fit 保证不超过可视区），此时任何平移手势都只是把画面拖出可视区、
    // 露出空白，所以这一轴要吃掉手势。
    readonly property bool canPanX: previewImage.width > width
    readonly property bool canPanY: previewImage.height > height

    // In fit mode the content includes the 64px work-area pad on every
    // side, so it is slightly larger than the viewport — without recentring
    // the image would sit 64px down (blank top, cut bottom). Center the
    // image in the viewport whenever fit sizing changes.
    // 装得下的轴一律居中（不只是 fit 模式）：缩小到比可视区小时 pad 也让
    // contentWidth 大出 128，不居中就会偏出半个 pad。
    function centerContent() {
        if (bridge.fit_mode || !canPanX)
            // 取整：contentX/Y 若带 .5，整块内容（含 1:1 的预览位图）都会被
            // 半像素采样糊掉，与 previewImage.x/y 取整是同一件事的两半。
            contentX = Math.round(Math.max(0, (contentWidth - width) / 2))
        if (bridge.fit_mode || !canPanY)
            contentY = Math.round(Math.max(0, (contentHeight - height) / 2))
    }
    onContentWidthChanged: centerContent()
    onContentHeightChanged: centerContent()
    onWidthChanged: {
        centerContent()
        requestViewport()
    }
    onHeightChanged: {
        centerContent()
        requestViewport()
    }
    // Startup safety net: when the first layout lands directly on the
    // final size, onWidthChanged/onHeightChanged never fire (no "change")
    // and the bridge would keep a 0x0 viewport — fit zoom then opens at
    // 100%. Reporting once at completion closes that race.
    Component.onCompleted: requestViewport()

    // ---- 窗口缩放节流 ------------------------------------------------
    // 适应窗口模式下每次 viewport 变化都要重跑一遍全预览重合成 + BMP
    // 编码写盘；拖动窗口边框时这类事件每秒几十次，debug 构建下界面直接
    // 卡死（实测每步 ~500ms）。对策：拖动期间一个像素都不渲染，只用
    // 「预测显示尺寸」把现有预览拉伸（视觉上跟手），停手 120ms 后再渲染
    // 一次真图。预测公式与 Rust 的 fit_zoom / display_size 一致，因此
    // 渲染落地时不会跳变。
    readonly property bool viewportPending: viewportTimer.running
    // host.dpr = 屏幕 devicePixelRatio（OS 缩放，如 125% = 1.25）。Rust 端
    // zoom 定义为「物理像素/图像像素」，逻辑显示尺寸 = 画布 × zoom / dpr；
    // 这里的预测公式必须与 fit_zoom / display_size 严格一致，否则滚轮/拖窗
    // 停手瞬间落地渲染会与预测值跳变。dpr = 1 时这些除/乘都是恒等。
    readonly property real dpr: host ? host.dpr : 1.0
    readonly property real fitScalePreview: {
        if (!bridge.fit_mode || bridge.image_width <= 0 || bridge.image_height <= 0)
            return 0
        return Math.min(width * dpr / bridge.image_width,
                        height * dpr / bridge.image_height, 1.0)
    }
    // 画布当前应当显示的尺寸：待渲染期间用预测值，其余用 bridge 的真值。
    // 缩放手势的预测优先（fit 已被 zoom_by 置 false）。
    readonly property real shownWidth: {
        if (zoomPending && predictedZoom > 0)
            return Math.max(1, Math.round(bridge.image_width * predictedZoom / dpr))
        if (viewportPending && fitScalePreview > 0)
            return Math.max(1, Math.round(bridge.image_width * fitScalePreview / dpr))
        return Math.max(1, bridge.display_width)
    }
    readonly property real shownHeight: {
        if (zoomPending && predictedZoom > 0)
            return Math.max(1, Math.round(bridge.image_height * predictedZoom / dpr))
        if (viewportPending && fitScalePreview > 0)
            return Math.max(1, Math.round(bridge.image_height * fitScalePreview / dpr))
        return Math.max(1, bridge.display_height)
    }

    function requestViewport() {
        if (!bridge.fit_mode) {
            // 非适应窗口：显示尺寸与 viewport 无关，同步告知即可（不渲染）。
            bridge.set_viewport(width, height)
            return
        }
        viewportTimer.restart()
    }

    // 拖动结束后才真正重渲染（Qt 在系统的缩放循环里通常不派发定时器，
    // 所以拖动全程零渲染）。
    Timer {
        id: viewportTimer
        interval: 120
        onTriggered: bridge.set_viewport(view.width, view.height)
    }

    // ---- 缩放节流 ------------------------------------------------------
    // Ctrl+滚轮每一格原来都同步跑一遍全量重合成 + BMP 编码，连滚直接卡
    // 死。对策与窗口缩放相同：手势期间只累积倍率，用「当前 zoom × 累积
    // 倍率」预测显示尺寸把现有预览拉伸（视觉跟手），停手 120ms 后渲染一
    // 次真图。100% 以上预览目标钳制在画布原生分辨率且不再变化，落地渲染
    // 大多直接复用上一帧（Rust 端 render-skip），滚轮全程无重绘。
    readonly property bool zoomPending: zoomTimer.running
    property real pendingZoom: 1.0
    readonly property real predictedZoom: {
        if (!zoomPending || bridge.zoom_percent <= 0)
            return 0
        const z = Math.min(16.0, Math.max(0.02,
            (bridge.zoom_percent / 100.0) * pendingZoom))
        // 与 Rust display_size 一致：显示最长边 clamp 32767。
        const longest = Math.max(bridge.image_width, bridge.image_height)
        return Math.min(z, 32767.0 / longest)
    }
    function queueZoom(factor) {
        pendingZoom = Math.min(16.0, Math.max(0.02, pendingZoom * factor))
        zoomTimer.restart()
    }
    // 绝对值入口（滑杆/预设菜单用）：换算成相对倍率走同一条节流管线。
    function queueZoomTo(pct) {
        if (bridge.zoom_percent <= 0)
            return
        const target = Math.min(1600, Math.max(2, pct))
        const base = Math.min(16.0, Math.max(0.02,
            bridge.zoom_percent / 100.0))
        const f = target / 100.0 / base
        if (f > 0.999 && f < 1.001)
            return
        queueZoom(f)
    }
    Timer {
        id: zoomTimer
        interval: 120
        onTriggered: {
            const f = view.pendingZoom
            view.pendingZoom = 1.0
            bridge.zoom_step(f)
        }
    }

    // Clamp-and-shift the visible region by (dx, dy) viewport px.
    // 装得下的轴不响应平移：适应窗口（或缩小到比可视区小）时画面是铺满
    // 的，滚轮/触摸板/中键拖动只会把它拖偏、露出空白，这一轴直接忽略，
    // 并顺手把已经偏掉的轴拉回居中（从放大状态缩回来时 contentX 还停在
    // 旧值上）。
    function panBy(dx, dy) {
        const overX = Math.max(0, contentWidth - width)
        const overY = Math.max(0, contentHeight - height)
        if (canPanX) {
            // 取整：触摸板/滚轮的增量常带小数，contentX 一旦是 .5，1:1 的
            // 预览位图就会按半像素中心采样（整幅变糊）。
            contentX = Math.round(Math.min(overX, Math.max(0, contentX + dx)))
        } else {
            contentX = Math.round(overX / 2)
        }
        if (canPanY) {
            contentY = Math.round(Math.min(overY, Math.max(0, contentY + dy)))
        } else {
            contentY = Math.round(overY / 2)
        }
    }

    // PS-style checkerboard behind the image: transparent pixels show
    // the grid configured in the settings panel. The 16×16 pattern tile
    // is generated by the bridge (checkerTileUrl, a BMP next to
    // settings.ini) and tiled by the GPU via `Image.fillMode Tile` —
    // zero repaint on zoom/resize, which a full-size Canvas could not
    // avoid (canvas resizes force synchronous repaints; that was the
    // main wheel-zoom jank on large previews). A Canvas + toDataURL
    // route was tried first and crashed: toDataURL re-enters the
    // synchronous Immediate paint path from onPainted.
    Image {
        id: checkerboard
        visible: bridge.preview_url.length > 0
                 && bridge.checkerTileUrl.length > 0
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        fillMode: Image.Tile
        smooth: false
        cache: false
        source: bridge.checkerTileUrl
    }

    // 预览画布：**双缓冲**。
    //
    // 拖动/旋转时每一帧都换 `source`，而 Image 换源的瞬间会把当前贴图丢掉
    // （status 变 Loading、cache:false 直接清图）——异步解码没完成之前画面
    // 是空白的。表现就是"一动图层整窗图片消失、松手才回来"。
    //
    // 所以放两个 Image 交替用：后台那张跟着 bridge.preview_url 预加载下一帧，
    // 前台那张继续显示已经解好的上一帧；后台 Ready 时才"换台"（只翻一个
    // bool，两张的 source 绑定随之对调）。全程纯声明式绑定，不需要额外的
    // 变更信号，也不会出现空白帧。
    Item {
        id: previewImage

        width: view.shownWidth
        height: view.shownHeight
        // 必须落在整数像素上：预览位图与显示尺寸是 1:1 的，一旦 x/y 出现
        // .5（奇数宽高居中时很常见，例如 31% 的 497×311），GPU 就会按
        // 半像素中心做双线性采样，整张画面被均匀糊掉——这正是"某些缩放
        // 比例看着模糊"的根因。居中偏移一律取整。
        x: Math.round((view.contentWidth - width) / 2)
        y: Math.round((view.contentHeight - height) / 2)

        // 当前正在显示的那一帧的 URL。
        property string shownUrl: ""
        // false = previewA 在前台（显示 shownUrl），true = previewB 在前台。
        property bool frontIsB: bool
        // 「上一帧还在解码」的起始时刻（见 previewBusy）。
        property double pendingSince: 0
        // 还没派给后台 Image 的最新一帧。
        property string pendingUrl: ""

        // 新的一帧到了：记下来交给后台那张去解（同一时刻只有一次解码在跑）。
        function pushUrl(url) {
            if (url.length === 0) {
                // 没有图了（新空窗口 / 关闭文档）：两张都清掉，别留着旧帧。
                previewA.source = ""
                previewB.source = ""
                shownUrl = ""
                pendingUrl = ""
                return
            }
            pendingUrl = url
            pumpPreview()
        }

        // 把待播的那一帧交给后台 Image。★后台还在解码就先不动它：换 source
        // 会取消上一次解码，改到最后谁也不上屏（实测不设这道闸时 115 次渲染
        // 只有 35 帧真正上屏）。等这次解码完（换台时再调一次）补上最新帧。
        function pumpPreview() {
            if (pendingUrl.length === 0 || pendingUrl === shownUrl)
                return
            const back = frontIsB ? previewA : previewB
            if (back.status === Image.Loading)
                return
            if (back.source.toString() === pendingUrl)
                return
            back.source = pendingUrl
            pendingSince = Date.now()
        }

        // 上一帧还没上屏（后台仍在解码）时返回 true。
        //
        // 拖动类手势用它限流：帧还没显示出来就再算一帧，只会把后台那次加载
        // 取消掉重来——预览反而一直停在上一帧，看着像"卡住不动"。给解码留
        // 150ms 的宽限，超时就不再等（万一某次加载失败，别把拖动拖死）。
        function previewBusy() {
            const back = frontIsB ? previewA : previewB
            if (back.status !== Image.Loading)
                return false
            // 卡太久（文件被占用/解码失败）就不再等，让上层照常出帧。
            return pendingSince === 0 || Date.now() - pendingSince < 400
        }

        // 显示比例一变（缩放手势的预测拉伸也算）就重映射选区。
        onWidthChanged: host.remapSelectionToScale()

        // 后台那张解码完成后换台。前台自己的 Ready（角色对调时的重载）忽略。
        function frameReady(img) {
            if (img.status === Image.Error) {
                console.log("ImageEdit: preview load error");
                if (bridge.auto_quit)
                    Qt.quit();
                return
            }
            if (img.status !== Image.Ready)
                return
            const isFront = img === (frontIsB ? previewB : previewA)
            if (isFront)
                return
            const url = img.source.toString()
            if (url.length === 0 || url === shownUrl)
                return
            shownUrl = url
            frontIsB = !frontIsB
            if (!host.imageRendered)
                host.imageRendered = true
            bridge.image_shown()
            // 换台后后台空出来了：如果期间又来了新帧，接着派。
            pumpPreview()
        }

        Image {
            id: previewA
            width: parent.width
            height: parent.height
            visible: !previewImage.frontIsB
            cache: false
            // ★异步解码。Image 默认是同步的：`source` 一变就在 setSource 里
            // 当场解码整张 BMP（1917×1278 约 10ms），这笔开销直接叠在拖动
            // 帧耗上，比合成+编码还贵。异步后解码走线程池，拖动帧只剩合成/
            // 编码；迟到的旧帧由 Qt 自己丢弃（request id 不匹配）。
            asynchronous: true
            fillMode: Image.Stretch
            // 平滑只在"真的在缩放"时开（两张同规则，换台时不会突然变糊）。
            smooth: Math.abs(implicitWidth - width * dpr) > 0.5
                    || Math.abs(implicitHeight - height * dpr) > 0.5
            onStatusChanged: previewImage.frameReady(previewA)
        }

        Image {
            id: previewB
            width: parent.width
            height: parent.height
            visible: previewImage.frontIsB
            cache: false
            asynchronous: true
            fillMode: Image.Stretch
            smooth: Math.abs(implicitWidth - width * dpr) > 0.5
                    || Math.abs(implicitHeight - height * dpr) > 0.5
            onStatusChanged: previewImage.frameReady(previewB)
        }

        // 新帧到达 → 派给后台那张。用属性变更信号而不是把 source 绑到
        // bridge.preview_url：绑定会在换台时把刚显示的帧重新加载一遍
        // （一次解码变成两次，还会互相取消）。
        Connections {
            target: bridge
            function onPreview_advanced() {
                previewImage.pushUrl(bridge.preview_url)
            }
        }

        // 打开文件时 url 在 Connections 建立之前就设好了，补一次。
        Component.onCompleted: previewImage.pushUrl(bridge.preview_url)
    }

    // ---- Persistent-selection interaction stack ----------------------
    // Hit priority: selected layer content drag (no active selection) →
    // selection move (center of the selection) → new selection rubber
    // band. Crop is applied to the live selection (Enter / Ctrl+K).
    //
    // The surface extends PAD display px beyond the image on every side
    // (and the Flickable content grows to match), so layers pushed
    // partially off-canvas remain reachable.

    // 子面板的 host/bridge 属性名会遮蔽父级同名属性（QML 名称解析里属性
    // 优先于 id），直接写 host: host 会成自引用绑定循环。用 Flickable 的
    // id `view` 显式取父级属性，绕开遮蔽。view/previewImage 是 id，无此问题。
    CanvasSurface {
        id: canvasSurface
        host: view.host
        bridge: view.bridge
        view: view
        previewImage: previewImage
    }

SelectionViz {
    id: selectionViz
    host: view.host
    bridge: view.bridge
    previewImage: previewImage
}


RulerOverlay {
    id: rulerOverlay
    host: view.host
    bridge: view.bridge
    previewImage: previewImage
}


    Text {
        anchors.centerIn: parent
        visible: bridge.preview_url.length === 0
        text: "把图片拖到这里，或在命令行传入文件路径"
        color: "#6b7076"
        font.pixelSize: 14
    }
}
