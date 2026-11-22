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
        // Tier 1 拖动期间由 gpuLayer 接管显示；这里透明但保留双缓冲状态机。
        //
        // ★★落定时**不能**立刻恢复不透明（`gpuLive ? 0 : 1` 的老写法）：
        //   gpuLive 一变 false，这里瞬间回到 1，把**起手前那一帧**（活动层
        //   还在原位）直接露出来。gpuLayer 的 static/above 是「透明底合成」
        //   的全幅纹理 —— 活动层原来占的那块在 static/above 里本来就是透明
        //   的，所以旧帧从透明处透出来 = 用户看到的「移动前的位置闪一下
        //   当前图层」。这和淡出快慢无关，盖多少层都挡不住。
        //   所以必须等 `gpuLayer.fadeArmed` 落下（= CPU 落定帧已经上屏、
        //   交叉过渡开始的那一刻）再变不透明：那一帧的内容已经在**最终
        //   位置**，透出来只会和 GPU 纹理重合。
        //
        // ★★起手时同样不能**立刻**透明：`gpu_begin` 换掉三张纹理的 URL，
        //   而它们是 `asynchronous: true` + `cache: false` —— 换源那一瞬间
        //   pixmap 被卸掉，新纹理还在解码，屏幕上什么都还没有。这时如果
        //   CPU 预览已经透明，整块画布会空白 ~1-2 帧（实测连拍里出现
        //   「洋红像素数掉到 0」的一帧）= 用户看到的「拖动时闪一下」。
        //   所以只在三张纹理**都解好**之后才让位给 GPU（`gpuLayer.texReady`）。
        opacity: (gpuLayer.texReady && (view.bridge.gpuLive || gpuLayer.fadeArmed)) ? 0 : 1

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
        property bool frontIsB: false
        // 「上一帧还在解码」的起始时刻（见 previewBusy / pulse）。
        property double pendingSince: 0
        // 还没派给后台 Image 的最新一帧。
        property string pendingUrl: ""
        // 同一帧连续重派次数（pulse 里用；3 次以后改同步解码兜底）。
        property int retries: 0
        // 已经上屏的帧序号（每次换台 +1）。GPU 落定后的淡出要等它变化，
        // 见 gpuLayer 的 fadeBaseCount。
        property int adoptCount: 0

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
            retries = 0
            pumpPreview()
        }

        // 把待播的那一帧交给后台 Image。
        //
        // ★不能再「后台还在 Loading 就一直等」：帧是异步解码的，只要有一次
        //   加载没回来（环形临时文件正好在这一刻被删、解码线程被取消、或者
        //   后台那张当时不可见而没建纹理），`back.source === pendingUrl` 就
        //   永远成立，pumpPreview 每次都提前 return，`frameReady` 又不会再来
        //   ——画面冻在拖动前那一帧，松手也不更新，要再动一下图层才恢复
        //   （用户 11-08 报的「拖的时候图层被背景挡住、松手没变大」）。
        //   所以只在「刚派下去 200ms 内」等，超时由 pulse() 强制重开。
        function pumpPreview() {
            if (pendingUrl.length === 0 || pendingUrl === shownUrl)
                return
            const back = frontIsB ? previewA : previewB
            const backUrl = back.source.toString()
            if (backUrl === pendingUrl) {
                // 已经派下去了：要么等 Ready（frameReady 换台），要么等 pulse 重开。
                return
            }
            if (back.status === Image.Loading && Date.now() - pendingSince < 200)
                return
            dispatch(back, pendingUrl)
        }

        // 真正换 source。上一次兜底可能把这张改成同步解码，这里恢复异步。
        function dispatch(img, url) {
            if (!img.asynchronous)
                img.asynchronous = true
            img.source = url
            pendingSince = Date.now()
        }

        // 换台：后台那张已经是待播帧，直接翻到前台。
        function adopt(img) {
            const url = img.source.toString()
            if (url.length === 0 || url === shownUrl)
                return
            shownUrl = url
            frontIsB = !frontIsB
            retries = 0
            adoptCount += 1
            if (!host.imageRendered)
                host.imageRendered = true
            bridge.image_shown()
            // 换台后后台空出来了：如果期间又来了新帧，接着派。
            pumpPreview()
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

        // 定时推进（16ms）：保证「有帧待播」这件事最终一定会被处理掉。
        //
        // 三种情况：
        //  1) 后台已经解好但信号没接上（Ready 后 source 变了下一次又对不上）
        //     → 直接 adopt；
        //  2) 后台停在 Loading 超过 200ms → 清源重开（同名 URL Qt 不会自己
        //     重试，必须置空再设才会重新读文件）；
        //  3) 连试 3 次还不行 → 这张改同步解码（拖动帧都不大，宁可掉一帧
        //     也不能把画面冻死），下一帧派发时再恢复异步。
        function pulse() {
            if (pendingUrl.length === 0 || pendingUrl === shownUrl)
                return
            const back = frontIsB ? previewA : previewB
            const backUrl = back.source.toString()
            if (backUrl !== pendingUrl) {
                if (back.status !== Image.Loading)
                    pumpPreview()
                return
            }
            if (back.status === Image.Ready) {
                adopt(back)
                return
            }
            if (Date.now() - pendingSince < 200)
                return
            retries += 1
            if (retries >= 3) {
                back.asynchronous = false
                back.source = ""
                back.source = pendingUrl
            } else {
                back.source = ""
                back.source = pendingUrl
            }
            pendingSince = Date.now()
        }

        // 显示比例一变（缩放手势的预测拉伸也算）就重映射选区。
        onWidthChanged: host.remapSelectionToScale()

        // 后台那张解码完成后换台。前台自己的 Ready（角色对调时的重载）忽略。
        function frameReady(img) {
            if (img.status === Image.Error) {
                console.log("ImageEdit: preview load error");
                if (bridge.auto_quit)
                    Qt.quit();
                // ★解码失败的帧必须交出槽位。BMP 是环形临时文件，拖动帧密集
                //   （60ms 一帧）时上一帧可能刚被删掉——Qt 的异步解码这时报
                //   Error。以前这里直接 return：那张 Image 永远停在 Error，
                //   pendingUrl 里最新的一帧**再也不会被派下去**，画面停在旧帧，
                //   直到下一次渲染（用户说的「松手后画面没变大，再动一下才好」
                //   就是这个）。这里清掉源、把待播帧重新派一次；同一帧连续
                //   失败就放弃它（等下一帧），避免卡在重试循环里。
                const failed = img.source.toString()
                img.source = ""
                pendingSince = 0
                if (pendingUrl.length > 0 && pendingUrl === failed)
                    pendingUrl = ""
                else
                    pumpPreview()
                return
            }
            if (img.status !== Image.Ready)
                return
            const isFront = img === (frontIsB ? previewB : previewA)
            if (isFront)
                return
            adopt(img)
        }

        Image {
            id: previewA
            width: parent.width
            height: parent.height
            // ★用 opacity 分前后台，**不要**用 visible 隐藏后台那张：
            //   cache:false 的 Image 一旦不可见，Qt 会把它的 pixmap 卸掉
            //   （QQuickImageBase 在 ItemVisibleHasChanged 时 unload），
            //   正在进行的那次异步解码就此没了下文 —— status 永远停在
            //   Loading，frameReady 不触发，双缓冲再也换不了台。表现就是
            //   「拖动时画面冻住、松手也不更新」。opacity:0 的 item 仍然
            //   参与渲染，纹理和解码都照常。
            visible: true
            opacity: previewImage.frontIsB ? 0 : 1
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
            visible: true
            opacity: previewImage.frontIsB ? 1 : 0
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

        // 看门狗：待播帧迟迟没上屏就推动一次（见 pulse 的三种处理）。
        //
        // `pushUrl` 只在**来了新帧**时才被调用，所以「最后一帧丢了」没有任何
        // 后续事件能补救：解码失败（临时文件被环形删除）、后台那张被取消、
        // 或者松开手后不再有渲染 —— 画面会停在上一帧，只有再动一下图层
        // （触发新渲染）才恢复。16ms 推进 + 200ms 超时重开把这条链子接上。
        Timer {
            interval: 16
            repeat: true
            running: previewImage.pendingUrl.length > 0
                     && previewImage.pendingUrl !== previewImage.shownUrl
            onTriggered: previewImage.pulse()
        }

        // 打开文件时 url 在 Connections 建立之前就设好了，补一次。
        Component.onCompleted: previewImage.pushUrl(bridge.preview_url)
    }

// P0-2b2 Tier 1：拖动期间用「静态层 + 活动层」两张 retained 纹理，
// 每帧只改活动层的 x/y/w/h/opacity，Qt Quick 在 GPU 上完成合成。
// 主预览 Item 此时透明（双缓冲仍在后台保持状态，避免解码闸门误判）。
// clip 把画布外的像素裁掉（D12）；框 / 手柄 / 徽标是控件，不在这里裁。
//
// ★声明位置必须在 CanvasSurface / SelectionViz 之前：同 z 下后来者居上，
//   否则选框和手柄会被这两张纹理盖住。
Item {
    id: gpuLayer
    x: previewImage.x
    y: previewImage.y
    width: previewImage.width
    height: previewImage.height
    // P1-A（R11 观感一致）：落定帧（CPU HQ）比拖动帧（GPU 双线性）锐，
    // 直接切换会有一次「突然变锐」的跳变。落定时 GPU 层淡出 ~90ms，下面的
    // HQ 预览同时露出来 —— 交叉过渡消除突变。
    //
    // ★★可见性只跟 `gpuLive` 走，**不要**再写成 `gpuLive || 淡出计时器在跑`：
    //   11-08 用户复测「拖动时图层像被背景盖住、松手也不变大、再移动一下
    //   才正常」就是这里 —— 旧写法用 16ms 定时器逐帧减 opacity 来做淡出，
    //   一旦 `onVisibleChanged` 里把计时器 stop 掉、把 opacity 复位成 1，
    //   计时器就再也不跑，`visible` 恒为 true。于是 Tier 1 的两张**旧**纹理
    //   （静态背景 + 活动层停在旧矩形）永久压在 CPU 预览之上：拖动时看到
    //   的是背景和旧图层，落定后的新帧被完全遮住，只有再起一次移动手势
    //   （重新 gpu_begin 换纹理）才「突然正常」。淡出改用一条 NumberAnimation
    //   跑完自己结束，不可能卡住。
    visible: view.bridge.gpuLive || fadeOpacity > 0.01
    // 纹理没解好之前整层透明：三张 Image 各自异步解码，谁先好谁先上屏，
    // 先上 static 就会出现「上层栈消失」的一两帧 —— 宁可等齐了一次性换。
    opacity: view.bridge.gpuLive ? (texReady ? 1.0 : 0.0) : fadeOpacity
    property real fadeOpacity: 0.0
    // 三张纹理是否**当前这批 URL** 都解码完成（见 previewImage.opacity 的说明）。
    //
    // ★不能只看 `status === Image.Ready`：`gpu_begin` 换 URL 与 QML 求值在同一个
    //   事件回合里，这一瞬间 Image 的 status 还是**上一批纹理**的 Ready，判定会
    //   假阳 —— 于是预览让位、新纹理还在解码 → 整块画布空白一两帧（连拍里
    //   就是「洋红像素数掉到 0」那一帧，也就是用户说的「拖动时闪一下」）。
    //   所以每张 Image 在 Ready 时把「解好的是哪个 URL」记下来，只有和当前
    //   `source` 完全对上才算好。
    property string rdyStatic: ""
    property string rdyActive: ""
    property string rdyAbove: ""
    property double texSince: 0
    property int texTick: 0
    readonly property bool texReady: {
        gpuLayer.texTick           // 看门狗强制重算（不读值，只建立依赖）
        // 兜底：某张纹理解码失败（Error）或 1.2s 还没好 —— 绝不能把 CPU 预览
        // 永远顶着，否则拖动时画面是冻的（比闪一下严重得多）。
        if (gpuLayer.texSince > 0 && Date.now() - gpuLayer.texSince > 1200)
            return true
        const imgs = [gpuStatic, gpuActive, gpuAbove]
        const done = [gpuLayer.rdyStatic, gpuLayer.rdyActive, gpuLayer.rdyAbove]
        for (let i = 0; i < imgs.length; i++) {
            if (imgs[i].status === Image.Error)
                return true
            const src = imgs[i].source.toString()
            if (src.length > 0 && done[i] !== src)
                return false
        }
        return true
    }
    Timer {
        interval: 40
        repeat: true
        running: (view.bridge.gpuLive || gpuLayer.fadeArmed) && !gpuLayer.texReady
        onTriggered: gpuLayer.texTick += 1
    }
    // ★落定后**不能立刻淡出**：GPU 拖动期间 `render()` 走 `gpu-skip` 不产新帧，
    //   下面的 CPU 预览还停在**起手前那一帧**（图层在原位）。立刻淡出 =
    //   中间那几十毫秒（新帧还在解码）露出旧位置的内容 → 用户看到的
    //   「放手瞬间移动前的位置闪一下图层」。
    //   所以淡出改为「等 CPU 新帧真正上屏」再开始（或超时兜底，绝不卡住）：
    //   * fadeBaseCount = 落定那一刻已经上屏的帧序号；`previewImage.adoptCount`
    //     一变（新帧换台）就开始交叉过渡。用计数器而不是比较 URL：同一次
    //     渲染可能复用同一个临时文件，URL 不变但画面已经换了。
    //   * 兜底**不立刻淡出**：等不到新帧时（渲染被跳过 / 超大画布编码慢）
    //     宁可让 GPU 纹理（停在最终位置、只是略钝）继续顶着，也不能把那张
    //     原位旧帧露出来 —— 这才是用户报的「闪一下」。所以兜底只是「再等
    //     一轮」，最多 3 轮（~2.1s），之后才强制收掉；空文档（preview_url
    //     为空）立刻收。
    property bool fadeArmed: false
    property int fadeBaseCount: -1
    property int fallbackHits: 0
    function beginFade() {
        gpuFadeFallback.stop()
        gpuLayer.fadeArmed = false
        gpuLayer.fallbackHits = 0
        gpuLayer.fadeOpacity = 1.0
        gpuFade.restart()
    }
    NumberAnimation {
        id: gpuFade
        target: gpuLayer
        property: "fadeOpacity"
        from: 1.0
        to: 0.0
        duration: 90
        easing.type: Easing.OutCubic
    }
    Timer {
        id: gpuFadeFallback
        // 700ms 一轮：大画布一帧「合成 + 编码 + 异步解码」可能真的超过 400ms，
        // 提前淡出就会露出旧位置那一帧（正是要修的现象）。这段等待里 GPU 纹理
        // 停在**最终位置**，观感只是稍钝，不会错位。
        interval: 700
        repeat: false
        onTriggered: {
            // 还有帧要等（文档非空且没等够 3 轮）→ 继续顶着纹理再等一轮。
            if (gpuLayer.fallbackHits < 3 && view.bridge.preview_url.length > 0) {
                gpuLayer.fallbackHits += 1
                gpuFadeFallback.restart()
            } else {
                gpuLayer.beginFade()
            }
        }
    }
    Connections {
        target: previewImage
        function onAdoptCountChanged() {
            if (gpuLayer.fadeArmed
                && previewImage.adoptCount !== gpuLayer.fadeBaseCount)
                gpuLayer.beginFade()
        }
    }
    Connections {
        target: view.bridge
        function onGpuLiveChanged() {
            if (view.bridge.gpuLive) {
                gpuFade.stop()
                gpuFadeFallback.stop()
                gpuLayer.fadeArmed = false
                gpuLayer.fallbackHits = 0
                gpuLayer.fadeOpacity = 1.0
                // 记下换纹理的时刻：`texReady` 的 1.2s 兜底从这里算起。
                gpuLayer.texSince = Date.now()
                gpuLayer.texTick += 1
            } else {
                gpuLayer.fadeOpacity = 1.0
                gpuLayer.fadeBaseCount = previewImage.adoptCount
                gpuLayer.fallbackHits = 0
                gpuLayer.fadeArmed = true
                gpuFadeFallback.restart()
            }
        }
    }
    clip: true

    readonly property var rect: {
        try { return JSON.parse(view.bridge.gpuRectJson) } catch (e) { return {} }
    }

    Image {
        id: gpuStatic
        anchors.fill: parent
        source: view.bridge.gpuStaticUrl
        fillMode: Image.Stretch
        cache: false
        asynchronous: true
        smooth: true
        // 记下「解好的是哪个 URL」：`texReady` 靠它区分「新纹理还没解」和
        // 「旧纹理的 Ready 状态还没被重置」（见 texReady 的注释）。
        onStatusChanged: if (status === Image.Ready) gpuLayer.rdyStatic = source.toString()
    }

    Image {
        id: gpuActive
        x: gpuLayer.rect.x || 0
        y: gpuLayer.rect.y || 0
        width: Math.max(1, gpuLayer.rect.w || 0)
        height: Math.max(1, gpuLayer.rect.h || 0)
        opacity: gpuLayer.rect.o === undefined ? 1 : gpuLayer.rect.o
        source: view.bridge.gpuActiveUrl
        fillMode: Image.Stretch
        cache: false
        asynchronous: true
        smooth: true
        // 纯平移走 x/y；矩阵通道留给 P1-A 的缩放 / 旋转统一采样器。
        transformOrigin: Item.TopLeft
        onStatusChanged: if (status === Image.Ready) gpuLayer.rdyActive = source.toString()
    }

    // 三明治上层：活动层**之上**的所有可见层（透明底合成）。静态不跟随
    // 拖动 —— z 序由堆叠顺序保证：下层栈 → 活动层 → 上层栈。这样拖任何
    // 一层（不只是最顶层）都不必回退 CPU 每帧全幅合成。
    Image {
        id: gpuAbove
        anchors.fill: parent
        source: view.bridge.gpuAboveUrl
        fillMode: Image.Stretch
        cache: false
        asynchronous: true
        smooth: true
        onStatusChanged: if (status === Image.Ready) gpuLayer.rdyAbove = source.toString()
    }
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

// P0-4 智能参考线：像素内容之上、控件之下；不受画布外裁剪影响。
SmartGuides {
    id: smartGuides
    bridge: view.bridge
    previewImage: view.previewImage
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
