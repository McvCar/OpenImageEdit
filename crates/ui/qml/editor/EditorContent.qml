import QtQuick
import QtQuick.Window
import QtQuick.Dialogs
import com.imageedit
import "../js/geometry.js" as Geometry
import "../js/layers.js" as Layers
import "../js/marks.js" as Marks
import "../js/adjust.js" as Adjust
import "../js/picker.js" as Picker

Item {
    id: root

    property bool primary: false
    // File to open immediately in a non-primary document window (the
    // "open in separate window" setting routes through here).
    property url fileUrl: ""
    property bool forceClose: false
    // Set right before Qt.quit() (selftest flow): in Qt 6.8 quitting
    // delivers close events to visible windows, and the dirty-guard in
    // requestClose() must not reject an intentional programmatic quit.
    property bool quitting: false
    property var pendingOpenUrl: null
    property int pendingNavDir: 0
    property string confirmMode: ""
    // 覆盖写回会丢结构的 PSD 时，如果这次保存是「保存并打开/切换/关闭」的
    // 第一半，确认后再继续的动作记在这里（open / nav / close）。
    property string lossyAfter: ""
    signal requestNewWindow(url fileUrl)
    property bool firstFrameSeen: false
    property bool imageRendered: false
    property bool imageFrameSeen: false
    readonly property int toolbarHeight: 40
    readonly property int statusHeight: 26
    // 右侧图层面板占的竖直列宽。二级工具栏（MarkBar）要按它让位，否则
    // MarkBar 是全宽横条、z 又比面板高，会把面板头部整块盖住。
    readonly property int layerPanelWidth: 264

    // ---- 打开图片后把窗口贴合到图片大小 --------------------------------
    // automationRun：任何 -- 开关（--selftest / --script / 各种 --probe-*）
    // 都跑同步解码 + 固定窗口尺寸做几何断言，自动改窗口会打乱既有基线，
    // 所以自动化运行里一律不触发（与 shell.rs 的 automation 判定一致）。
    readonly property bool automationRun: {
        const args = Qt.application.arguments
        for (let i = 0; i < args.length; i++)
            if (args[i].indexOf("--") === 0)
                return true
        return false
    }
    // --probe-winfit：自动化里也允许"按图片调窗口尺寸/位置"，专门验证预置
    // 尺寸、居中保持这些行为（其余探针仍然固定窗口，避免打乱几何基线）。
    readonly property bool probeWinFit: Qt.application.arguments.indexOf("--probe-winfit") !== -1
    // --probe-screen：打印光标所在显示器、各屏几何/缩放、换算出的逻辑坐标，
    // 用来在多屏机器上核对"新窗口落在鼠标那块屏"。
    readonly property bool probeScreen: Qt.application.arguments.indexOf("--probe-screen") !== -1
    readonly property bool windowAutoGeometry: !automationRun || probeWinFit || probeScreen
    // 所属窗口（Window.window 这个附加属性在 EditorContent 里求值会抛异常，
    // 见 dpr 那段的注释；需要窗口对象的探针/逻辑用这个显式引用）。
    property var hostWindow: null
    // 新文档装载计数（Rust 装好 session 后自增并放在刷新之后，此时尺寸已
    // 发布）。用它而不是 image_width/h：裁剪、旋转、改画布尺寸同样会改宽高，
    // 那些操作不该动窗口。
    property int loadSerialSeen: bridge.loadSerial
    onLoadSerialSeenChanged: {
        if (root.loadSerialSeen > 0)
            root.windowFitRequested()
    }
    signal windowFitRequested()

    // 用户手动挪过窗口（拖标题栏 / 拖边框）之后，以新位置为"意图中心"，
    // 换图时围着它缩放。程序自己摆位时 adjustingWindow 挡住，不算手动。
    // 全屏/最大化不算（那套几何退出后不该污染普通窗口的中心）。
    Connections {
        target: root.hostWindow
        enabled: target !== null
        function onXChanged() {
            if (!root.adjustingWindow && root.hostWindow
                    && root.hostWindow.visibility === Window.Windowed)
                root.syncWindowCenter(root.hostWindow)
        }
        function onYChanged() {
            if (!root.adjustingWindow && root.hostWindow
                    && root.hostWindow.visibility === Window.Windowed)
                root.syncWindowCenter(root.hostWindow)
        }
    }

    // 旋转框状态按图层索引分开记（见 rotateFrames），切换图层不再清空——
    // 切走再切回来，倾斜框还在。尺寸对不上时 layerFrameRotated 会自己失效。

    // 把 win 调成"刚好装下这张图"：宽高 = 图片 100% 显示尺寸（逻辑像素）
    // + 画布以外的 chrome；装不下时图片按同一比例缩到屏幕内（保宽高比），
    // 不小于窗口最小尺寸；最大化/全屏不动。
    //
    // 位置**保持窗口中心不动**：一堆大小不一的图按方向键翻的时候，如果
    // 每次都保留左上角，窗口看上去就在左右横跳；钉住中心才稳。
    function fitWindowToImage(win) {
        if (!win || !root.windowAutoGeometry)
            return
        if (bridge.image_width <= 0 || bridge.image_height <= 0)
            return
        // 只在普通窗口下改：最大化/最小化/全屏时用户已经表达过尺寸意愿。
        if (win.visibility !== Window.Windowed)
            return
        const size = fitSizeFor(win, bridge.image_width, bridge.image_height)
        if (!size)
            return
        resizeWindowKeepingCenter(win, size.w, size.h)
    }

    // 计算"刚好装下 imgWpx × imgHpx（图像像素）"的窗口尺寸（逻辑像素）。
    // chrome（工具栏 / 状态栏 / 图层面板）是固定开销，缩图时只缩图片那部分。
    function fitSizeFor(win, imgWpx, imgHpx) {
        if (!win || imgWpx <= 0 || imgHpx <= 0)
            return null
        const dpr = root.sizeDpr()
        const imgW = Math.max(1, imgWpx / dpr)
        const imgH = Math.max(1, imgHpx / dpr)
        // 图层面板浮在画布右侧，扣掉它之后才是图片可用的宽度。
        const chromeW = root.panelVisible ? layerPanel.width : 0
        const chromeH = root.toolbarHeight + root.statusHeight
                        + (root.markBarVisible ? root.toolbarHeight : 0)
        const area = root.workArea()
        // 留 48px 边距，别把窗口贴死在屏幕边缘/任务栏上。
        const margin = 48
        const maxW = area ? Math.max(0, area.w - margin) : 0
        const maxH = area ? Math.max(0, area.h - margin) : 0
        // 装不下时按**同一比例**缩图片，而不是把宽、高分别砍到屏幕上限：
        // 8160×6120（4:3）分别砍会得到 3392×1344（2.5:1），图只有 1704 宽，
        // 左右各空 844px —— 就是"窗口宽度没有收缩到图片大小"。
        let scale = 1
        if (maxW > 0)
            scale = Math.min(scale, Math.max(0.02, (maxW - chromeW) / imgW))
        if (maxH > 0)
            scale = Math.min(scale, Math.max(0.02, (maxH - chromeH) / imgH))
        const minW = win.minimumWidth > 0 ? win.minimumWidth : 320
        const minH = win.minimumHeight > 0 ? win.minimumHeight : 240
        let targetW = Math.max(minW, Math.round(imgW * scale) + chromeW)
        let targetH = Math.max(minH, Math.round(imgH * scale) + chromeH)
        // 最小尺寸可能反倒把窗口撑出可用区域，最后再钳一次。
        if (maxW > 0)
            targetW = Math.min(targetW, Math.max(minW, maxW))
        if (maxH > 0)
            targetH = Math.min(targetH, Math.max(minH, maxH))
        return { w: Math.round(targetW), h: Math.round(targetH) }
    }

    // 当前屏的可用工作区（逻辑像素）——desktopAvailable* 已经扣掉任务栏。
    // **不能**用 Screen.availableWidth/Height：那是 Qt 5 的名字，Qt 6 上取到
    // undefined（旧代码于是静默 fallback 成整屏，大图窗口底边压在任务栏上）。
    //
    // 多屏：一律以 targetScreen（光标所在屏）为准。用 root.Screen 的话，窗口
    // 还没显示时它报的是主屏，新窗口就会被按主屏的工作区夹回第一块屏。
    function screenWorkArea(s) {
        if (!s)
            return null
        const sw = s.width
        const sh = s.height
        const w = Math.min(s.desktopAvailableWidth || sw, sw)
        const h = Math.min(s.desktopAvailableHeight || sh, sh)
        if (!(w > 0) || !(h > 0))
            return null
        return { x: s.virtualX, y: s.virtualY, w: w, h: h }
    }
    function workArea() {
        // 已显示的窗口以它**现在**所在的屏为准（用户可能把窗口拖到了另一块屏）；
        // 还没显示时 root.Screen 报的是主屏，必须用光标所在屏。
        const host = root.hostWindow
        const shown = host && host.visible
        return screenWorkArea(shown ? root.Screen
                                    : (root.targetScreen || root.Screen))
    }

    // 光标所在的那块屏 + 光标在该屏内的位置（已换算成 Qt 逻辑坐标）。
    // 由 presizeWindow 在量尺寸之前调用一次（Rust 侧的"启动光标快照"只能
    // 取一次，所以整轮摆位共用这一次结果）。
    property var targetScreen: null
    property real cursorLogicalX: -1
    property real cursorLogicalY: -1

    // 图片像素 → 逻辑像素时该用的 dpr：窗口要落在哪块屏，就用哪块屏的缩放。
    function sizeDpr() {
        const s = root.targetScreen
        if (s && s.devicePixelRatio > 0)
            return s.devicePixelRatio
        return root.dpr > 0 ? root.dpr : 1
    }

    // 把"全局物理坐标"换算成 Qt 逻辑坐标。★必须按**那块屏**分段换算：
    // 屏 A 1920 宽@100%、屏 B 3440 宽@125% 时，Qt 逻辑上 B 的原点是 1920，
    // 而不是 3440/1.25=2752。全局除一个 dpr 必然错位到别的屏。
    function resolveCursor() {
        root.targetScreen = null
        root.cursorLogicalX = -1
        root.cursorLogicalY = -1
        const info = bridge.cursor_monitor()
        if (!info || info.length === 0)
            return false
        const f = info.split("\t")
        if (f.length < 8)
            return false
        const screens = Qt.application.screens
        const friendly = f[0]          // 显示器友好名，如 S342-DP（Qt 的 QScreen.name）
        const gdi = f[1]               // \\.\DISPLAY1（有些环境 Qt 用这个）
        const offX = parseInt(f[2], 10) || 0
        const offY = parseInt(f[3], 10) || 0
        const monW = parseInt(f[4], 10) || 0
        const monH = parseInt(f[5], 10) || 0
        const monX = parseInt(f[6], 10) || 0
        const monY = parseInt(f[7], 10) || 0
        // ① 快速路径：名字能对上就直接用（有些环境下 QScreen.name 就是 GDI 名
        //    或显示器友好名；对不上也没关系，走 ② 的几何判据）。
        let s = null
        for (let i = 0; i < screens.length; ++i) {
            const n = screens[i].name
            if (n === friendly || n === gdi) {
                s = screens[i]
                break
            }
        }
        // ② 几何判据：Qt 的逻辑原点 = 该屏物理原点 / 该屏 dpr，所以
        //    「virtualX×dpr ≈ 显示器物理原点」就是同一块屏。用尺寸先筛候选，
        //    再用原点距离取最优。两台同型号显示器靠这也分得开。
        if (!s) {
            let best = null
            let bestScore = 0
            for (let i = 0; i < screens.length; ++i) {
                const c = screens[i]
                const d = c.devicePixelRatio > 0 ? c.devicePixelRatio : 1
                const sizeOk = monW > 0 && monH > 0
                    && Math.abs(Math.round(c.width * d) - monW) <= 2
                    && Math.abs(Math.round(c.height * d) - monH) <= 2
                const score = Math.abs(Math.round(c.virtualX * d) - monX)
                              + Math.abs(Math.round(c.virtualY * d) - monY)
                // 尺寸对得上的优先（score 减 1 做轻量加权，不改变量级）。
                const rank = sizeOk ? score : score + 100000
                if (!best || rank < bestScore) {
                    best = c
                    bestScore = rank
                }
            }
            s = best
        }
        if (!s)
            return false
        root.targetScreen = s
        const dpr = s.devicePixelRatio > 0 ? s.devicePixelRatio : 1
        root.cursorLogicalX = s.virtualX + offX / dpr
        root.cursorLogicalY = s.virtualY + offY / dpr
        if (root.probeScreen) {
            let dump = ""
            for (let i = 0; i < screens.length; ++i) {
                const c = screens[i]
                dump += (i > 0 ? " | " : "") + c.name + " v=" + c.virtualX + "," + c.virtualY
                        + " " + c.width + "x" + c.height
                        + " dpr=" + c.devicePixelRatio
                        + " avail=" + c.desktopAvailableWidth + "x" + c.desktopAvailableHeight
            }
            bridge.mark("cursor-monitor:" + info.replace(/\t/g, "|")
                        + " => logical " + Math.round(root.cursorLogicalX) + ","
                        + Math.round(root.cursorLogicalY)
                        + " on " + s.name
                        + " || screens: " + dump)
        }
        return true
    }

    // 把 (x, y, w, h) 夹进工作区：窗口永远整块留在屏幕上，不压任务栏。
    function clampToWorkArea(x, y, w, h) {
        const area = root.workArea()
        if (!area)
            return { x: Math.round(x), y: Math.round(y) }
        return {
            x: Math.round(Math.min(Math.max(x, area.x), Math.max(area.x, area.x + area.w - w))),
            y: Math.round(Math.min(Math.max(y, area.y), Math.max(area.y, area.y + area.h - h)))
        }
    }

    // 换图 / 换 chrome 时要守住的"意图中心"（逻辑坐标，-1 = 还没定）。
    // ★必须记意图而不是上一帧的实际中心：大图被屏幕边缘夹住之后实际中心
    // 已经偏移，下一张小图若以它为心，窗口就整体挪走了（"翻图库时窗口乱动"）。
    property real windowCenterX: -1
    property real windowCenterY: -1
    // 我们自己在改窗口几何时置位，避免把程序摆位误判成用户手动拖动。
    property bool adjustingWindow: false

    // 记下窗口当前中心作为"意图中心"（首次摆位 / 用户手动拖动后用）。
    function syncWindowCenter(win) {
        if (!win)
            return
        root.windowCenterX = win.x + win.width / 2
        root.windowCenterY = win.y + win.height / 2
    }

    // 改尺寸但钉住意图中心（换图 / 换 chrome 时不再左右横跳）。
    function resizeWindowKeepingCenter(win, w, h) {
        if (!win)
            return
        if (root.windowCenterX < 0 || root.windowCenterY < 0)
            root.syncWindowCenter(win)
        const cx = root.windowCenterX
        const cy = root.windowCenterY
        root.adjustingWindow = true
        win.width = w
        win.height = h
        const pos = root.clampToWorkArea(cx - w / 2, cy - h / 2, w, h)
        win.x = pos.x
        win.y = pos.y
        root.adjustingWindow = false
    }

    // 显示**之前**预置窗口：按文件头量出的尺寸定大小，再摆到鼠标光标中心并
    // 夹进工作区。这样窗口一出现就是最终大小/位置，不会"先弹个空窗口、
    // 解码完再跳一下"。url 为空（空窗口启动）时只按默认尺寸摆位置。
    function presizeWindow(win, url) {
        if (!win || !root.windowAutoGeometry)
            return false
        // 先解析光标在哪块屏：后面的量尺寸、夹边都要用那块屏的工作区。
        const haveCursor = root.resolveCursor()
        const s = url ? url.toString() : ""
        if (s.length > 0) {
            const probed = bridge.probe_image_size(s)
            if (probed.length > 0) {
                const parts = probed.split("x")
                const size = fitSizeFor(win, parseInt(parts[0], 10) || 0,
                                        parseInt(parts[1], 10) || 0)
                if (size) {
                    win.width = size.w
                    win.height = size.h
                }
            }
        }
        if (haveCursor && root.placeAtCursor(win)) {
            if (root.probeScreen)
                bridge.mark("window-placed:" + win.x + "," + win.y + " "
                            + win.width + "x" + win.height
                            + " on " + (root.targetScreen ? root.targetScreen.name : "?"))
            return true
        }
        // 光标接口不可用（非 Windows）：至少居中到工作区。
        const area = root.workArea()
        if (area) {
            win.x = Math.round(area.x + (area.w - win.width) / 2)
            win.y = Math.round(area.y + (area.h - win.height) / 2)
        }
        root.syncWindowCenter(win)
        return false
    }

    // 以鼠标光标为中心摆窗口（坐标由 resolveCursor 算好，是 Qt 逻辑坐标）。
    // ★所有窗口都精确居中，不做级联错开：错开 28px 会让第二张以后的图看起来
    // "没对准鼠标"。
    function placeAtCursor(win) {
        if (!win || root.cursorLogicalX < 0)
            return false
        const area = root.workArea()
        let x = root.cursorLogicalX - win.width / 2
        let y = root.cursorLogicalY - win.height / 2
        if (area) {
            x = Math.min(Math.max(x, area.x), Math.max(area.x, area.x + area.w - win.width))
            y = Math.min(Math.max(y, area.y), Math.max(area.y, area.y + area.h - win.height))
        }
        root.adjustingWindow = true
        win.x = Math.round(x)
        win.y = Math.round(y)
        root.adjustingWindow = false
        root.syncWindowCenter(win)
        return true
    }

    // 启动时命令行里的第一个待打开文件（跳过 argv[0] 和所有 -- 开关）。
    function initialFileArg() {
        const args = Qt.application.arguments
        for (let i = 1; i < args.length; ++i)
            if (args[i].length > 0 && args[i].indexOf("--") !== 0)
                return args[i]
        return ""
    }

    property bool fullscreen: false
    // Persistent-selection interaction model (no crop/selection/move mode
    // toggles): one selection lives on the canvas, crop is applied to it,
    // a freshly pasted layer enters a "floating" place-it state.
    property bool panelVisible: false
    property bool pasteFloating: false
    // How many undoable ops were committed since the paste that started the
    // floating state (the paste itself + any drag/scale commits), so Esc
    // can roll the whole paste back.
    property int floatingOps: 0
    // ---- 选区（PS 式）------------------------------------------------
    // 真值在 Rust（SelectionMask，图像像素坐标）；QML 只读它，并按当前
    // 显示比例换算成显示坐标，供绘制 / 光标 / 尺子用。
    // ★这 4 个必须保持只读：任何 `root.selW = …` 的旧写法都是 bug，改选
    //   区一律走 bridge.sel_*。
    readonly property int selImgX: bridge.image_width > 0 ? bridge.selX : 0
    readonly property int selImgY: bridge.image_width > 0 ? bridge.selY : 0
    readonly property int selImgW: bridge.image_width > 0 ? bridge.selW : 0
    readonly property int selImgH: bridge.image_width > 0 ? bridge.selH : 0
    readonly property real selX: selImgX * toDispScale()
    readonly property real selY: selImgY * toDispScale()
    readonly property real selW: selImgW * toDispScale()
    readonly property real selH: selImgH * toDispScale()
    readonly property bool selExists: bridge.selActive
    // 拖动选区时的临时显示偏移（真值仍由 Rust 在松手时平移）。
    readonly property real selOffX: view.canvasSurface ? view.canvasSurface.selOffX : 0
    readonly property real selOffY: view.canvasSurface ? view.canvasSurface.selOffY : 0
    // 当前选区工具："rect" | "ellipse" | "lasso" | "polygon" | "wand"
    // | "quick" | "none"
    // 初值来自 settings.ini（上次用的那个），切工具时再写回去。
    property string selTool: bridge.selToolSaved.length > 0 ? bridge.selToolSaved : "rect"
    // 魔术棒参数（PS 的三个关键项）：容差 / 连续 / 取样全部图层。
    // 初值同样来自 settings.ini（bridge.selWandSaved = "容差,连续,全部图层"）。
    property int wandTolerance: 32
    property bool wandContiguous: true
    property bool wandSampleAll: true
    // 快速选择参数：笔刷半径（画布像素）、容差、从笔迹向外最多扩多少像素。
    // 笔刷半径用 [ / ] 调，和 PS 一致。
    property int quickRadius: 24
    property int quickTolerance: 40
    property int quickGrow: 160
    // 选区工具浮动面板（标记栏的「选区」按钮点开；原来那行固定工具条已取消）。
    property bool selPopoverOpen: false
    // 加选/减选/相交的粘滞模式（拖动时按住的 Shift/Alt 会临时覆盖它）：
    // 0 替换 / 1 加选 / 2 减选 / 3 相交。
    property int selMode: 0
    // 右键菜单是否打开（打开时 Esc 先关它）。
    property bool selMenuOpen: false
    // 选区数值输入框正在编辑：此时 M/L/Q 快捷键必须让路（否则输入框
    // 里打的字会切成工具 / 开关快速蒙版）。
    property bool selFieldEditing: false
    // ★ 单字母工具快捷键（V/M/L/W/J/G/[/]/Q）的 enabled 绑定必须等整个
    //   组件树建完再求值：这些 Shortcut 定义在文件前部（885 行起），却要
    //   引用后面才实例化的 `view` / `fontPopover`。创建期求值抛 TypeError
    //   会让 QML 丢弃绑定，enabled 永久停在 false —— 表现就是按 V 没反应。
    //   Component.onCompleted 之后才放行，绑定此时能完整收集依赖。
    property bool toolKeysReady: false
    // Transient "rubber band in use" state; hides the handle dots and
    // disables the wheel zoom while dragging a new selection.
    readonly property bool selRubberBanding: view.canvasSurface.selDragMouse.dragging
                                             || view.canvasSurface.selPolyActive
    readonly property bool modeActive: selRubberBanding || pasteFloating
    // Scale target for the selected layer (display coords); empty when idle.
    property real scaleX: 0
    property real scaleY: 0
    property real scaleW: 0
    property real scaleH: 0
    // 图层手柄被拖过对边（负宽高）：松手时按这两个标记镜像图层位图。
    property bool scaleFlipX: false
    property bool scaleFlipY: false
    // 本次缩放拖的是哪几条边（1 左 / 4 右 / 8 上 / 32 下）。落定时交给
    // Rust：被拖的边贴目标、对侧锚点不动；旋转过的图层靠它反解 scale。
    property int scaleEdges: 0
    // P1-A：Ctrl+拖边的斜切量与对侧锚定补偿（transform_solve_scale 解出，
    // 原样透传给 scale_layer_update / scale_layer_end；非斜切手势恒 0）。
    property real scaleSkewX: 0
    property real scaleSkewY: 0
    property real scaleCenterDx: 0
    property real scaleCenterDy: 0
    // 等比（角点 + Shift）拖动：起手比例与锁定标记。落定时按它反解高度，
    // 免得 w/h 各自取整把刚保住的宽高比又打回 ±1px（25% 缩放下很明显）。
    property bool scaleRatioLocked: false
    property real scaleStartRatio: 0
    // 旋转拖动中的实时修饰键（Shift 15° 吸角，语义在 Rust 解出）。
    property int rotateMods: 0
    // 角度磁吸的合成位掩码（对应 Rust `MOD_MAGNET`）：交互式手柄才带，
    // 脚本探针 / 数值面板不带 —— 那边必须能摆出 91.4° 这种任意角度。
    readonly property int rotateMagnetBit: 0x10000000
    property int renamingIndex: -1
    readonly property var layerList: Layers.parseLayers(bridge.layers_json)
    // The layer PANEL rebuilds its Repeater whenever the model array is
    // replaced. Bounds (x/y/w/h) change on every live-drag tick, which
    // would rebuild the whole list ~30x/s; the panel only shows
    // identity/flags, so rebind it only when those actually change.
    // Canvas outlines etc. keep using layerList (live bounds).
    readonly property string layerPanelSig: {
        // 前缀是像素版本：值一变（任何可撤销编辑 / 撤销重做），面板重建一次，
        // 缩略图跟着重取。拖动帧不 bump 它，所以拖动期间列表不动。
        // groups_json 也进签名：编组 / 解组 / 重命名 / 折叠都不改像素版本，
        // 但面板的显示行必须重建，否则组标题行不会出现。
        let s = bridge.layerEpoch + "|" + bridge.groups_json + "|"
        for (var i = 0; i < layerList.length; i++) {
            const l = layerList[i]
            s += l.i + ":" + l.id + ":" + l.n + ":" + l.v + ":" + l.l + ":"
                 + Math.round(l.o) + ";"
        }
        return s
    }
    // 分组元数据（组不是图层，单独一份）。画布点选 / 选层框等仍只读
    // layerList（纯图层行），不会被组标题行污染。
    readonly property var groupList: Layers.parseLayers(bridge.groups_json)
    property var layerPanelList: buildLayerPanelRows()
    onLayerPanelSigChanged: layerPanelList = buildLayerPanelRows()
    readonly property var selLayer: findSelLayer()
    readonly property var selLayerBox: findLayerBox()
    // ---- 旋转后的图层框 ----
    // 松手提交时位图被扩成旋转后的**最小外接矩形**，四个角全是透明像素。
    // 直接把这个矩形画成虚线框，看上去就是"每转一次框就大一圈"（900×700
    // 转 45° 变 1132×1132）。这里记住旋转前的框尺寸和累计角度，由
    // SelectionViz 画成贴合内容的倾斜四边形。
    //
    // ★按图层分别记（index -> { w, h, angle }）：以前是单份全局状态，
    //   一切换图层就清空，切回来时框退回轴对齐的扩边矩形 —— 像素还转着，
    //   框却正了，就是用户说的"放手角度会复位"。
    // P0-2b2：角度与原图尺寸的真值改为 layers_json 的 a / sw / sh
    //（来自 Layer::transform 参数层）。撤销 / 重做 / 缩放 / 切换图层之后
    // 天然同步，不会再出现「按位图尺寸反推角度」失配导致框角度复位。
    // rotateFrames 仍由旧探针写入，但不再参与任何显示判定。
    property var rotateFrames: ({})
    readonly property var rotateFrame: {
        const b = selLayerBox
        if (b !== null && b.sw > 0 && b.sh > 0)
            return ({ w: b.sw, h: b.sh })
        return null
    }
    readonly property real rotateFrameAngle: {
        const b = selLayerBox
        if (b !== null && typeof b.a === "number" && Math.abs(b.a) > 0.001)
            return b.a
        return 0
    }
    readonly property bool layerFrameRotated: {
        if (Math.abs(rotateFrameAngle) <= 0.01)
            return false
        const b = selLayerBox
        return b !== null && b.w > 0 && b.h > 0 && rotateFrame !== null
    }
    readonly property real layerFrameW: rotateFrame !== null ? rotateFrame.w : 0
    readonly property real layerFrameH: rotateFrame !== null ? rotateFrame.h : 0
    // SelectionViz 统一用 layer* 前缀读这三个值（这里只是别名）。
    readonly property real layerFrameAngle: rotateFrameAngle

    function setRotateFrame(ix, w, h, angle) {
        if (ix < 0)
            return
        const m = Object.assign({}, root.rotateFrames)
        if (!(w > 0) || !(h > 0))
            delete m[ix]
        else
            m[ix] = { w: w, h: h, angle: angle }
        root.rotateFrames = m
    }

    function clearRotateFrame() {
        root.setRotateFrame(bridge.selected_layer, 0, 0, 0)
    }
    // ---- 标记工具栏（markup toolbar）----
    property bool markBarVisible: false
    // 移动工具（PS 的 V）：一个**开关**，不是互斥工具。开着时选中图层的
    // 八向手柄 / 旋柄 / 描边 / 尺寸徽标都可用，并且能和选区、画笔、标记
    // 和平共处；关掉就纯查看/绘制，碰不到图层几何。默认开。
    property bool moveMode: true
    // Active drawing tool: "none" | "rect" | "ellipse" | "arrow"
    //                   | "brush" | "text" | "mosaic"
    property string markTool: "none"
    // Style for the NEXT mark; with a mark selected the popovers restyle
    // it instead (see applyMarkStyle).
    property int markStroke: 4
    property string markColor: "#e24b4a"
    property bool markFill: false
    property string markFillColor: "#f5e14a"
    // Mosaic block size in image px (the "strength"); one of the 5 steps.
    property int markMosaic: 12
    // Text tool state (font family id from core::font::FONTS).
    property string markFont: "msyh"
    property int markFontSize: 36
    property bool markBold: false
    property bool markItalic: false
    property bool markUnderline: false
    property int markAlign: 0 // 0 left / 1 center / 2 right
    // ---- 修复 / 油漆工具族 ----------------------------------------
    // paintTool 与 markTool / selTool 三者互斥：同一时刻只有一个工具族在画布上
    // 接管手势。修复类改像素（可撤销），不产生标记。
    property string paintTool: "none" // none | spot | heal | patch | bucket | gradient
    property bool paintPopoverOpen: false
    property string paintGroup: "heal" // 参数面板当前展示的组
    // 进入修复 / 油漆工具**之前**用的选区工具：退出这组工具时还回去。
    // （以前把 selTool 置成 none 就撒手了 —— 关掉污点工具后画布按下走的是
    //   「点空白」分支，框选彻底失灵，只能重新挑一次选区工具。）
    property string selToolBeforePaint: ""
    property bool paintMenuOpen: false
    // 笔刷参数（修复类共用）：大小按画布像素，硬度/间距/强度按百分比。
    property int healSize: 40
    property int healHardness: 70
    property int healSpacing: 20
    property int healStrength: 100
    // 修复画笔是否已经有取样点（Alt + 点击设置）。
    property bool healHasSource: false
    // 修补：扩散（羽化半径，像素）。
    property int patchFeather: 2
    readonly property bool patchHasRegion: view.canvasSurface.patchRegion.length >= 3
    // 油漆桶参数。
    property int bucketTolerance: 32
    property bool bucketContiguous: true
    property int bucketOpacity: 100
    // 渐变参数（起止色直接用描边色 / 填充色）。
    property bool gradRadial: false
    property bool gradReverse: false
    property int gradOpacity: 100
    // ---- 图层旋转（画布手柄）----
    property bool rotating: false
    property real rotateAnglePreview: 0
    // 本轮旋转会话开始前已经累计的角度（连续拖手柄时非 0）。
    property real rotateBase: 0
    // 拖动节流状态：上一帧算出的角度、上一帧真实耗时、上一帧的起算时刻。
    property real rotateRenderedAngle: 0
    property int rotateFrameCost: 0
    property double rotateLastAt: 0
    readonly property var markList: Marks.parseMarks(bridge.marks_json)
    // ---- 调整颜色（macOS 预览风格）----
    // Mirrored parameters (slider positions) and the throttled live push.
    readonly property var adjustVals: Adjust.parseAdjust(bridge.adjustValues)
    property string adjustKey: ""
    property real adjustValue: 0
    property bool adjustDirty: false
    // Levels (histogram handles) pending values, throttled like sliders.
    property real pendingBlack: 0
    property real pendingGamma: 1
    property real pendingWhite: 1
    property bool levelsDirty: false
    readonly property bool probeAdjust: Qt.application.arguments.indexOf("--probe-adjust") !== -1
    readonly property bool probeClipboard: Qt.application.arguments.indexOf("--probe-clipboard") !== -1
    readonly property bool probeRuler: Qt.application.arguments.indexOf("--probe-ruler") !== -1
    readonly property bool probePicker: Qt.application.arguments.indexOf("--probe-picker") !== -1
    // Exercises the Ctrl+S / menu 保存 path (saveOrSaveAs → save_edited)
    // so QML scope errors on that route fail loudly in probe runs.
    readonly property bool probeSave: Qt.application.arguments.indexOf("--probe-save") !== -1
    // 修复 / 油漆工具面板的探针：`--probe-paint [--paint-tool=spot|heal|patch|
    // bucket|gradient] [--paint-menu]`。`--paint-menu` 走右键浮层那条路径。
    readonly property bool probePaint: Qt.application.arguments.indexOf("--probe-paint") !== -1
    readonly property string paintProbeTool: {
        const flag = "--paint-tool="
        for (let i = 0; i < Qt.application.arguments.length; ++i) {
            const a = Qt.application.arguments[i]
            if (a.indexOf(flag) === 0)
                return a.substring(flag.length)
        }
        return "heal"
    }
    // 图层旋转探针：`--probe-rotate [--rotate-dialog]`。不带 --rotate-dialog
    // 就只显示画布上的旋转手柄。
    readonly property bool probeRotate: Qt.application.arguments.indexOf("--probe-rotate") !== -1
    // ---- 尺子（测量坐标）----
    // 开关只是显示层，不动文档；坐标全部是「相对画布」的图像像素坐标。
    property bool rulerOn: false
    // 鼠标在画布上的图像坐标（可为小数；落在画布外时 hoverValid=false）。
    property real hoverFx: 0
    property real hoverFy: 0
    property bool hoverValid: false
    // ---- 拾色器 ----
    readonly property var pickerVals: Picker.parsePicker(bridge.pickerColor)
    readonly property var pickerHist: Picker.parsePickerHistory(bridge.pickerHistory)
    // 刚复制的那一行（按钮短暂显示「已复制」）。
    property string pickerCopied: ""
    // 四种色值表示（拾色器弹窗逐行复制）。
    readonly property var pickerSpecs: {
        const c = pickerVals
        if (!c)
            return []
        return [
            { k: "HEX",  v: c.hex },
            { k: "RGB",  v: c.rgb },
            { k: "HSL",  v: c.hsl },
            { k: "CMYK", v: c.cmyk }
        ]
    }
    // 选区 / 选中图层的测量框（图像像素坐标；无选中时为 null）。
    readonly property var rulerBox: {
        const s = toDispScale()
        if (!rulerOn || s <= 0 || bridge.image_width <= 0)
            return null
        if (selExists && selW > 0 && selH > 0)
            return { x: selX / s, y: selY / s, w: selW / s, h: selH / s }
        if (selLayerBox !== null && selLayerBox.w > 0 && selLayerBox.h > 0)
            return { x: selLayerBox.x, y: selLayerBox.y,
                     w: selLayerBox.w, h: selLayerBox.h }
        return null
    }
    // 选区 / 选中图层的 5 个测量点：四角 + 中心，单位是图像像素。
    readonly property var rulerPoints: {
        const box = rulerBox
        if (box === null)
            return []
        const s = toDispScale()
        const H = bridge.image_height
        const fy = function (v) { return bridge.rulerBottomUp ? H - v : v }
        const pts = [
            { x: box.x, y: box.y },
            { x: box.x + box.w, y: box.y },
            { x: box.x, y: box.y + box.h },
            { x: box.x + box.w, y: box.y + box.h },
            { x: box.x + box.w / 2, y: box.y + box.h / 2 }
        ]
        return pts.map(function (p) {
            return { px: p.x * s, py: p.y * s,
                     label: Math.round(p.x) + ", " + Math.round(fy(p.y)) }
        })
    }
    // Whether the currently selected mark is a text mark (drives the
    // Aa style button and the font panel).
    readonly property bool selMarkIsText: {
        const m = markList.find(mk => mk.i === bridge.selected_mark)
        return !!m && m.k === 5
    }
    readonly property bool probeDialog: Qt.application.arguments.indexOf("--probe-dialog") !== -1
    readonly property bool probeSettings: Qt.application.arguments.indexOf("--probe-settings") !== -1
    readonly property bool probeMenu: Qt.application.arguments.indexOf("--probe-menu") !== -1
    readonly property bool probeResize: Qt.application.arguments.indexOf("--probe-resize") !== -1
    readonly property bool probeLayers: Qt.application.arguments.indexOf("--probe-layers") !== -1
    readonly property bool probeMarks: Qt.application.arguments.indexOf("--probe-marks") !== -1
    // 选区探针：打开工具条 + 落一个椭圆选区（截图/手工验证用）。
    readonly property bool probeSelection: Qt.application.arguments.indexOf("--probe-selection") !== -1
    // 选区探针的附加项：把右键菜单也弹出来（核对数值输入框排版）。
    readonly property bool probeSelMenu: Qt.application.arguments.indexOf("--probe-selmenu") !== -1
    // 多边形套索的进行中预览（截图核对橡皮筋 + 顶点方块）。
    readonly property bool probeSelPoly: Qt.application.arguments.indexOf("--probe-selpoly") !== -1
    // 选区浮动面板（标记栏里的那个按钮弹出来的）：核对面板排版 + 画布是否
    // 正确让位。选区工具条改成浮层后没有常驻行，只能靠它截图。
    readonly property bool probeSelPop: Qt.application.arguments.indexOf("--probe-selpop") !== -1
    // 平移锁定探针：打印一次内容/可视几何，再调一次 panBy，看它有没有动。
    // 「适应窗口」下两轴都装得下，panBy 应该原样返回（before == after）。
    readonly property bool probePanLock: Qt.application.arguments.indexOf("--probe-panlock") !== -1
    // 标题栏容量探针：每秒打印宽度 / titleMode / 能放下的槽位，配合外部
    // 改窗口宽度验证"窄窗口下先收标题而不是砍按钮"。
    readonly property bool probeTitleBar: Qt.application.arguments.indexOf("--probe-titlebar") !== -1
    // 选区工具切换自检：验证"换形状不自动打开标记栏"。
    readonly property bool probeSelToolSwitch: Qt.application.arguments.indexOf("--probe-seltool") !== -1
    // Ids inside an inline component are invisible from outside, so the
    // shell / DocWindow wrappers reach the Bridge through this alias.
    readonly property alias b: bridge

    // 窗口变宽/变窄时让调色弹窗重新贴住按钮（placePopover 负责夹紧）；
    // 用户自己拖过位置的话只做边界夹紧。
    onWidthChanged: {
        if (!adjustPanel.visible)
            return
        if (adjustPanel.userMoved)
            adjustPanel.moveTo(adjustPanel.x, adjustPanel.y)
        else
            placePopover(adjustPanel, markBar.adjustBtn)
    }

    Bridge {
        id: bridge

        onLoadFailedChanged: if (bridge.loadFailed && bridge.auto_quit)
            Qt.quit()
        onImage_widthChanged: {
            // New canvas: drop the scale preview. The selection lives in
            // Rust now and is cleared there when the canvas itself changes.
            root.scaleW = 0
            root.scaleH = 0
        }
    }

    // Report the screen's device-pixel-ratio to the bridge so the preview
    // bitmap is rendered at the physical resolution (logical × dpr). Without
    // this a window at >100% Windows scaling gets a logically-sized bitmap
    // stretched by the GPU → the whole image is uniformly blurred.
    //
    // MUST use the attached `Screen` type, not `window.screen`: the attached
    // `window` property evaluates to null/throws here (EditorContent is
    // instantiated inside a Window owned by another file), so the old
    // `window ? window.screen.devicePixelRatio : 1.0` binding silently fell
    // back to 1.0 forever — the app then drew at logical size (25% too large
    // on a 125% display) with the bitmap upscaled (blurred).
    property real dpr: Screen.devicePixelRatio > 0 ? Screen.devicePixelRatio : 1.0
    onDprChanged: bridge.set_dpr(dpr)

    // ---- 单实例：接收另一个进程交过来的图片 -------------------------
    // 资源管理器里双击图片会新起一个进程；main() 发现已有实例时把路径通过
    // 本地回环 socket 交给它并退出，于是所有文档窗口都在同一个进程里（图层
    // 剪贴板自然共享）。这里由主窗口轮询取出路径，按「在单独的新窗口显示」
    // 设置决定开新窗口还是就地替换（有未保存修改时走确认对话框）。
    Timer {
        id: externalOpenTimer
        interval: 250
        repeat: true
        running: root.primary && !root.quitting
        onTriggered: {
            const url = bridge.take_external_open()
            if (url.length === 0)
                return
            // 和 confirmThenOpen/openFile 同一套规则：空窗口就地打开（否则
            // 用户双击图片后，留着的那个空窗口反而成了"废窗口"）。
            if (bridge.openInNewWindow && bridge.image_width > 0)
                root.requestNewWindow(url)
            else
                confirmThenOpen(url)
            root.raiseWindow()
        }
    }

    // ---- 后台解码：轮询应用解码结果 --------------------------------
    // 大图在 worker 线程解码，这里以 50ms 轮询取结果；结果落地后
    // bridge.loading 翻回 false，Timer 自动停。
    Timer {
        id: decodePollTimer
        interval: 50
        repeat: true
        // PSD stage 2 and the background save encoder both reuse this very
        // timer: no extra timer, no extra polling loop. psdLayersPending /
        // saving keep it alive until the worker result is drained.
        running: (bridge.loading || bridge.psdLayersPending || bridge.saving
                  || bridge.thumbsPending) && !root.quitting
        onTriggered: {
            if (bridge.loading)
                bridge.poll_decode()
            if (bridge.psdLayersPending)
                bridge.poll_psd_layers()
            if (bridge.saving)
                bridge.poll_save()
            if (bridge.thumbsPending)
                bridge.poll_thumbs()
        }
    }

    function raiseWindow() {
        const w = Window.window
        if (!w)
            return
        if (w.visibility === Window.Minimized)
            w.showNormal()
        w.requestActivate()
    }

FileDialog {
    id: openDialog
    title: "打开图片"
    nameFilters: [
        "图片文件 (*.jpg *.jpeg *.jfif *.png *.gif *.bmp *.tif *.tiff *.webp *.tga *.dds *.ico *.qoi *.pbm *.pgm *.ppm *.pnm *.hdr *.psd)",
        "所有文件 (*)"
    ]
    onAccepted: confirmThenOpen(selectedFile)
    onVisibleChanged: if (visible) {
        bridge.mark("dialog-opened");
        if (bridge.auto_quit)
            dialogQuitTimer.start();
    }
}

Timer {
    id: dialogQuitTimer
    interval: 400
    onTriggered: Qt.quit()
}

Timer {
    id: dialogProbeTimer
    interval: 500
    onTriggered: openDialog.open()
}

FileDialog {
    id: saveDialog
    title: "另存为"
    fileMode: FileDialog.SaveFile
    // 不写死 defaultSuffix：老写法固定 "png"，选了「Photoshop (*.psd)」
    // 再输入不带扩展名的文件名，Qt 仍会补成 .png —— 用户以为存了 PSD，
    // 实际写出去的是 PNG。后缀改为按当前选中的文件类型推导。
    defaultSuffix: ""
    nameFilters: ["Photoshop (*.psd)", "PNG (*.png)", "JPEG (*.jpg)", "WebP (*.webp)", "TIFF (*.tiff)", "BMP (*.bmp)", "TGA (*.tga)", "QOI (*.qoi)", "ICO (*.ico)"]
    onAccepted: bridge.save_as(root.saveTargetUrl(selectedFile))
}

// 「另存为 PSD…」：普通图片（或任何非 PSD 文档）也能把当前图层栈写成
// 一份 PSD。过滤器锁死 Photoshop、后缀固定 .psd，不依赖原生对话框的
// 默认后缀行为；分组是编辑器元数据，写进 PSD 会展开成平级图层。
FileDialog {
    id: psdSaveDialog
    title: "另存为 PSD"
    fileMode: FileDialog.SaveFile
    defaultSuffix: "psd"
    nameFilters: ["Photoshop (*.psd)"]
    onAccepted: bridge.save_as(root.withPsdSuffix(selectedFile))
}

Shortcut { sequences: [StandardKey.Open]; onActivated: openDialog.open() }
Shortcut { sequences: [StandardKey.New]; onActivated: root.requestNewWindow() }
Shortcut { sequences: [StandardKey.SaveAs]; enabled: !bridge.editingLocked && !bridge.saving; onActivated: saveDialog.open() }
Shortcut { sequence: "F11"; onActivated: root.fullscreen = !root.fullscreen }
Shortcut { sequence: "Escape"; onActivated: onEscape() }
// Browse images in the loaded file's folder (guard: no active gesture).
// Dialogs block navigation so arrow keys never swap the file behind a
// open settings/resize panel.
Shortcut { sequence: "Left"; enabled: canNavigate(); onActivated: navigateRel(-1) }
Shortcut { sequence: "Right"; enabled: canNavigate(); onActivated: navigateRel(1) }
// 回车：模态确认框 = 默认动作（保存并打开/切换/关闭）；否则裁剪/粘贴。
// 数值框正在编辑时禁用：回车属于输入框（提交数值），不能顺手把图裁了。
Shortcut { sequences: ["Return", "Enter"]
           enabled: !bridge.editingLocked && !root.selFieldEditing
                    && (confirmOverlay.visible || root.pasteFloating
                    || root.selExists || view.canvasSurface.selPolyActive)
           onActivated: onEnter() }

Shortcut { sequence: "Left";  enabled: !bridge.editingLocked && bridge.selected_layer >= 0; onActivated: nudgeLayer(-1, 0) }
Shortcut { sequence: "Right"; enabled: !bridge.editingLocked && bridge.selected_layer >= 0; onActivated: nudgeLayer(1, 0) }
Shortcut { sequence: "Up";    enabled: !bridge.editingLocked && bridge.selected_layer >= 0; onActivated: nudgeLayer(0, -1) }
Shortcut { sequence: "Down";  enabled: !bridge.editingLocked && bridge.selected_layer >= 0; onActivated: nudgeLayer(0, 1) }
Shortcut { sequence: "Shift+Left";  enabled: !bridge.editingLocked && bridge.selected_layer >= 0; onActivated: nudgeLayer(-10, 0) }
Shortcut { sequence: "Shift+Right"; enabled: !bridge.editingLocked && bridge.selected_layer >= 0; onActivated: nudgeLayer(10, 0) }
Shortcut { sequence: "Shift+Up";    enabled: !bridge.editingLocked && bridge.selected_layer >= 0; onActivated: nudgeLayer(0, -10) }
Shortcut { sequence: "Shift+Down";  enabled: !bridge.editingLocked && bridge.selected_layer >= 0; onActivated: nudgeLayer(0, 10) }

Shortcut { sequences: [StandardKey.ZoomIn, "Ctrl+="]; onActivated: view.queueZoom(1.25) }
Shortcut { sequences: [StandardKey.ZoomOut]; onActivated: view.queueZoom(1 / 1.25) }
// Preview-aligned: ⌘0 actual size, ⌘9 zoom-to-fit (old keys kept as aliases).
// Ctrl+方向键 = 平移选区（PS 的蚂蚁线平移；图层 nudge 仍归方向键）。
Shortcut { sequence: "Ctrl+Left";  enabled: root.selExists && root.toolKeysReady && root.canUseToolKeys()
           onActivated: bridge.sel_translate(-1, 0) }
Shortcut { sequence: "Ctrl+Right"; enabled: root.selExists && root.toolKeysReady && root.canUseToolKeys()
           onActivated: bridge.sel_translate(1, 0) }
Shortcut { sequence: "Ctrl+Up";    enabled: root.selExists && root.toolKeysReady && root.canUseToolKeys()
           onActivated: bridge.sel_translate(0, -1) }
Shortcut { sequence: "Ctrl+Down";  enabled: root.selExists && root.toolKeysReady && root.canUseToolKeys()
           onActivated: bridge.sel_translate(0, 1) }
Shortcut { sequences: ["Ctrl+0", "Ctrl+1"]; onActivated: bridge.zoom_actual() }
Shortcut { sequences: ["Ctrl+9"]; onActivated: bridge.zoom_fit() }
// ★ 编辑锁：以下全部是写文档 / 写文件的动作，阶段 2 未完成前一律禁用。
// 查看类（缩放、平移、打开文件、关闭）不在这里，保持可用。
Shortcut { sequences: [StandardKey.Undo]; enabled: !bridge.editingLocked; onActivated: bridge.undo() }
// Redo: explicit keys — StandardKey.Redo maps inconsistently across
// platforms/layouts, plain strings fire reliably (Ctrl+Y and ⇧Ctrl+Z).
Shortcut { sequences: ["Ctrl+Shift+Z", "Ctrl+Y"]; enabled: !bridge.editingLocked; onActivated: bridge.redo() }
Shortcut { sequences: [StandardKey.Save]; enabled: !bridge.editingLocked && !bridge.saving; onActivated: saveOrSaveAs() }
Shortcut { sequences: [StandardKey.Copy]; enabled: !bridge.editingLocked; onActivated: doCopy() }
Shortcut { sequences: [StandardKey.Cut]; enabled: !bridge.editingLocked; onActivated: doCut() }
Shortcut { sequences: [StandardKey.Paste]; enabled: !bridge.editingLocked; onActivated: doPaste() }
Shortcut { sequence: "Ctrl+K"; enabled: !bridge.editingLocked && root.selExists; onActivated: applyCrop() }
// 图层分组：普通编辑模式（随手新建的图层）与 PSD 分层态通用。
// 用 toolKeysReady 短路创建期求值，canUseToolKeys 保证重命名输入框
// 有焦点时不会误触发（和 V/M/L 那批工具键同一个闸门）。
Shortcut { sequence: "Ctrl+G"; enabled: !bridge.editingLocked && root.toolKeysReady
           && root.canUseToolKeys(); onActivated: bridge.group_selected_layers() }
Shortcut { sequence: "Ctrl+Shift+G"; enabled: !bridge.editingLocked && root.toolKeysReady
           && root.canUseToolKeys(); onActivated: bridge.ungroup_selected_layers() }
// 调整颜色面板（macOS 预览的调色入口）。
Shortcut { sequence: "Ctrl+E"; enabled: !bridge.editingLocked && bridge.image_width > 0
           onActivated: root.toggleAdjustPanel() }
Shortcut { sequence: "Ctrl+L"; enabled: !bridge.editingLocked; onActivated: bridge.rotate_ccw() }
Shortcut { sequence: "Ctrl+Shift+R"; enabled: !bridge.editingLocked; onActivated: bridge.rotate_cw() }
// Delete/Backspace 在文本输入（文本标记就地编辑 / 字号输入 / 图层
// 重命名）获得焦点时必须禁用：QML Shortcut 先于焦点项拦截按键，
// 不禁用会吞掉输入框的退格键，还会误删选中的标记/图层。
Shortcut { id: deleteShortcut
           sequences: ["Delete", "Backspace"]
           enabled: !bridge.editingLocked
                    && !view.canvasSurface.textPopup.visible
                    && !(fontPopover.visible && fontPopover.sizeInput.activeFocus)
                    && !root.selFieldEditing
                    && root.renamingIndex < 0
                    && (root.selExists || bridge.selected_mark >= 0
                    || bridge.selected_layer >= 0
                    || (function () {
                        const b = Layers.findBase(layerList)
                        return b !== null && b.bd !== true && !Layers.isLocked(b)
                            && bridge.layer_count > 0
                    })())
           onActivated: onDeleteKey() }
Shortcut { sequence: "Ctrl+A"
           enabled: !bridge.editingLocked && bridge.image_width > 0 && root.renamingIndex < 0
                    && !root.selFieldEditing
           onActivated: selectAllCanvas() }
// ---- 选区（PS 快捷键）----
// V = 移动工具开关（开：图层可拖可缩放；关：手柄收起，碰不到图层）。
Shortcut {
    id: moveShortcut
    // ★单个 sequence，**不要**用 `sequences: ["V", "v"]`：同一快捷键注册两条
    //   只差大小写的序列，Qt 的 QShortcutMap 会把它当成重载/歧义而整体不触发
    //   （实测 moveMode 一次都不翻，而 M / J 这些单 sequence 的键全正常）。
    //   大小写本来就不影响匹配 —— 相邻的 "M" 用大写、裸按 m 一样生效。
    sequence: "V"
    context: Qt.ApplicationShortcut
    enabled: root.toolKeysReady && root.canUseToolKeys()
    onActivated: root.toggleMoveTool()
}
// M / Shift+M = 矩形 / 椭圆选框；L / Shift+L = 套索 / 多边形套索。
Shortcut { sequence: "M"; enabled: root.toolKeysReady && root.canUseToolKeys()
           onActivated: root.setSelToolByShortcut("rect") }
Shortcut { sequence: "Shift+M"; enabled: root.toolKeysReady && root.canUseToolKeys()
           onActivated: root.setSelToolByShortcut("ellipse") }
Shortcut { sequence: "L"; enabled: root.toolKeysReady && root.canUseToolKeys()
           onActivated: root.setSelToolByShortcut("lasso") }
Shortcut { sequence: "Shift+L"; enabled: root.toolKeysReady && root.canUseToolKeys()
           onActivated: root.setSelToolByShortcut("polygon") }
// W：快速选择（PS 默认）；Shift+W：魔术棒（PS 里两个工具同组，W 循环）。
Shortcut { sequence: "W"; enabled: root.toolKeysReady && root.canUseToolKeys()
           onActivated: root.setSelToolByShortcut("quick") }
Shortcut { sequence: "Shift+W"; enabled: root.toolKeysReady && root.canUseToolKeys()
           onActivated: root.setSelToolByShortcut("wand") }
// J：修复组（污点修复 → 修复画笔 → 修补 循环）；G：油漆组（油漆桶 / 渐变）。
Shortcut { sequence: "J"; enabled: root.toolKeysReady && root.canUseToolKeys()
           onActivated: {
               const order = ["spot", "heal", "patch"]
               const i = order.indexOf(root.paintTool)
               root.setPaintTool(order[(i + 1) % order.length])
               root.markBarVisible = true
           } }
Shortcut { sequence: "G"; enabled: root.toolKeysReady && root.canUseToolKeys()
           onActivated: {
               const order = ["bucket", "gradient"]
               const i = order.indexOf(root.paintTool)
               root.setPaintTool(order[(i + 1) % order.length])
               root.markBarVisible = true
           } }
// [ / ]：修复笔刷大小（PS 的笔刷大小快捷键）；不在修复工具下时调快速选择。
Shortcut { sequence: "["; enabled: root.toolKeysReady && root.canUseToolKeys()
           onActivated: {
               if (root.paintTool === "spot" || root.paintTool === "heal")
                   root.setHealSize(root.healSize - 4)
               else
                   root.setQuickRadius(root.quickRadius - 4)
           } }
Shortcut { sequence: "]"; enabled: root.toolKeysReady && root.canUseToolKeys()
           onActivated: {
               if (root.paintTool === "spot" || root.paintTool === "heal")
                   root.setHealSize(root.healSize + 4)
               else
                   root.setQuickRadius(root.quickRadius + 4)
           } }
Shortcut { sequence: "Ctrl+D"; enabled: !bridge.editingLocked && root.selExists
           onActivated: bridge.sel_clear() }
Shortcut { sequence: "Ctrl+Shift+I"; enabled: !bridge.editingLocked && bridge.image_width > 0
           onActivated: bridge.sel_invert() }
// Q: 快速蒙版（要看羽化软边在哪就按它）。
Shortcut { sequence: "Q"; enabled: bridge.image_width > 0 && root.toolKeysReady && root.canUseToolKeys()
           onActivated: bridge.sel_toggle_quick_mask() }
// ⌘W / ⌘Q: close window / quit, both through the dirty-confirm guard.
Shortcut { sequence: "Ctrl+W"; onActivated: requestQuit() }
Shortcut { sequence: "Ctrl+Q"; onActivated: requestQuit() }
Shortcut { sequence: "Ctrl+R"; enabled: !bridge.editingLocked; onActivated: bridge.rotate_cw() }

// ---- Custom title bar: title + icon toolbar + overflow "…" menu + ----
// ---- window buttons. The windows are frameless; this bar doubles as ----
// ---- the drag area (see WindowEdges / drag MouseArea below).        ----
TitleBar {
    id: toolbar
    host: root
    bridge: bridge
    view: view
    openDialog: openDialog
    saveDialog: saveDialog
    settingsOverlay: settingsOverlay
    resizeOverlay: resizeOverlay
    overflowMenu: overflowMenu
    textPopup: view.canvasSurface.textPopup
    widthPopover: widthPopover
    colorPopover: colorPopover
    mosaicPopover: mosaicPopover
    fontPopover: fontPopover
}


MarkBar {
    id: markBar
    host: root
    bridge: bridge
    toolbar: toolbar
}

// P0-5：变换数值面板，停靠工作区左上角；显示状态由 settings.ini 记忆。
TransformPanel {
    id: transformPanel
    bridge: bridge
    // 初始位置注入（面板内部 x/y 绑定到这两个值；用户拖动后绑定被
    // 命令式赋值替换，位置由用户接管，本次会话内保持）。
    defaultX: 12
    defaultY: root.toolbarHeight * 2 + 12
    z: 60
}

// ---- 选区浮动面板（标记栏「选区」按钮点开）--------------------------
SelectionPopover {
    id: selPopover
    host: root
    bridge: bridge
    toolbar: toolbar
    markBar: markBar
}

// ---- 选区右键菜单 ---------------------------------------------------
SelectionMenu {
    id: selMenu
    host: root
    bridge: bridge
}

// ---- 修复 / 油漆：两个工具组面板 + 画布右键的紧凑参数面板 ----------
PaintPopover {
    id: healPop
    host: root
    bridge: bridge
    markBar: markBar
    toolbar: toolbar
    group: "heal"
}
PaintPopover {
    id: paintPop
    host: root
    bridge: bridge
    markBar: markBar
    toolbar: toolbar
    group: "paint"
}
PaintPopover {
    id: paintMenu
    host: root
    bridge: bridge
    markBar: markBar
    toolbar: toolbar
    mode: "context"
    group: "heal"
}

// 图层旋转角度对话框（图层面板右键 / 「图像」菜单打开）。
RotateLayerOverlay {
    id: rotateOverlay
    host: root
    bridge: bridge
}

// 点空白处关掉选区菜单（菜单本身在更高的 z 上，点击不会穿透到这里）。
MouseArea {
    z: 295
    anchors.fill: parent
    visible: root.selMenuOpen || root.paintMenuOpen
    enabled: root.selMenuOpen || root.paintMenuOpen
    onPressed: (mouse) => {
        if (root.selMenuOpen)
            selMenu.close()
        if (root.paintMenuOpen)
            root.closePaintMenu()
        mouse.accepted = true
    }
}

// 右键落在选区内 → 弹菜单（画布坐标已换算成 root 坐标）。
function openSelMenu(x, y) {
    selMenu.open(x, y)
}


// 弹窗定位：贴着触发按钮的正下方（不固定在窗口右侧）。
function placePopover(pop, btn) {
    const p = btn.mapToItem(root, 0, 0)
    pop.x = Math.max(4, Math.min(p.x - 8, root.width - pop.width - 4))
}

WidthPopover {
    id: widthPopover
    host: root
    toolbar: toolbar
    markBar: markBar
}


MosaicPopover {
    id: mosaicPopover
    host: root
    bridge: bridge
    toolbar: toolbar
    markBar: markBar
}

// 形状面板的"点外部收起"：面板打开时铺一层透明遮罩盖住画布，点一下先收
// 面板（不把手势透给画布）。z 压在标记栏（59）之下，所以工具栏上的按钮
// 仍然优先响应。
MouseArea {
    id: markShapeDismiss
    anchors.fill: parent
    visible: markShapePopover.visible
    z: 58
    acceptedButtons: Qt.LeftButton | Qt.RightButton
    onClicked: markShapePopover.visible = false
}

// 矩形 / 椭圆 / 箭头 / 马赛克 的合并面板（标记栏只留一个按钮）。
MarkShapePopover {
    id: markShapePopover
    host: root
    bridge: bridge
    toolbar: toolbar
    markBar: markBar
}


FontPopover {
    id: fontPopover
    host: root
    toolbar: toolbar
    markBar: markBar
    colorPopover: colorPopover
}


ColorPopover {
    id: colorPopover
    host: root
    toolbar: toolbar
    markBar: markBar
}


ZoomMenu {
    id: zoomMenu
    host: root
    bridge: bridge
    statusBar: statusBar
    view: view
    pctBtn: statusBar.pctBtnRef
}

// 弹窗打开时的点击捕获层。
MouseArea {
    z: 215
    anchors.fill: parent
    visible: widthPopover.visible || colorPopover.visible
             || mosaicPopover.visible || fontPopover.visible
             || zoomMenu.visible
    enabled: visible
    onPressed: (mouse) => {
        widthPopover.visible = false
        colorPopover.visible = false
        mosaicPopover.visible = false
        fontPopover.visible = false
        zoomMenu.visible = false
        mouse.accepted = true
    }
}

OverflowMenu {
    id: overflowMenu
    toolbar: toolbar
}

// Click-away catcher for the overflow menu.
MouseArea {
    z: 205
    anchors.fill: parent
    visible: overflowMenu.visible
    enabled: overflowMenu.visible
    onPressed: (mouse) => {
        overflowMenu.visible = false
        mouse.accepted = true
    }
}

CanvasView {
    id: view
    host: root
    bridge: bridge
    markBar: markBar
    toolbar: toolbar
    statusBar: statusBar
    layerPanel: layerPanel
}

// Viewport-fixed pan/zoom input (sits above the Flickable so it never
// scrolls with content). Wheel/touchpad = pan, Ctrl+wheel = zoom,
// Shift+wheel = horizontal pan. Middle-drag pans. Left-button events are
// NOT accepted here and pass through to the canvas gesture stack, so a
// normal click-drag can never pan the view.
MouseArea {
    id: panMa
    // 顶部对齐画布视图本身（而不是标题栏），否则中键拖动会压住
    // 标记栏 / 选区栏上的按钮。
    anchors.top: view.top
    anchors.bottom: statusBar.top
    anchors.left: parent.left
    anchors.right: parent.right
    acceptedButtons: Qt.MiddleButton
    // cursorShape: undefined is CRITICAL. A MouseArea's cursorShape
    // defaults to ArrowCursor and Qt's cursor resolution picks the
    // TOPMOST item that has any cursor — with the default, this area
    // wins everywhere and masks every cursor below it (cross, resize,
    // move). `undefined` removes it from cursor resolution entirely;
    // the pan feedback cursor comes from the canvas HoverHandler.
    cursorShape: undefined
    property real lastX: 0
    property real lastY: 0
    onPressed: (mouse) => {
        lastX = mouse.x
        lastY = mouse.y
    }
    onPositionChanged: (mouse) => {
        if (!pressed)
            return
        view.panBy(lastX - mouse.x, lastY - mouse.y)
        lastX = mouse.x
        lastY = mouse.y
    }
    onWheel: (event) => {
        if (root.modeActive) {
            event.accepted = false
            return
        }
        if ((event.modifiers & Qt.ControlModifier) !== 0) {
            if (event.angleDelta.y > 0)
                view.queueZoom(1.25)
            else if (event.angleDelta.y < 0)
                view.queueZoom(1 / 1.25)
            event.accepted = true
            return
        }
        if (event.pixelDelta.x !== 0 || event.pixelDelta.y !== 0) {
            // Touchpad pan: content follows the fingers.
            view.panBy(event.pixelDelta.x, event.pixelDelta.y)
        } else if ((event.modifiers & Qt.ShiftModifier) !== 0) {
            view.panBy(-event.angleDelta.y * 0.4, 0)
        } else if (event.angleDelta.x !== 0) {
            view.panBy(event.angleDelta.x * 0.4, 0)
        } else {
            view.panBy(0, -event.angleDelta.y * 0.4)
        }
        event.accepted = true
    }
}

// ---- 工具开关的即时反馈（V / 移动按钮）------------------------------
// 1.3s 自动消失。以前开关状态只能靠手柄是否出现来猜：选区还在时手柄
// 本来就藏着，按 V 看上去毫无反应，像是快捷键坏了。
Rectangle {
    id: toolToast
    visible: false
    z: 400
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom
    anchors.bottomMargin: root.statusHeight + 18
    width: toolToastLabel.implicitWidth + 24
    height: toolToastLabel.implicitHeight + 12
    radius: 5
    color: "#e02b2d31"
    border.color: "#5b9dff"
    border.width: 1
    Text {
        id: toolToastLabel
        anchors.centerIn: parent
        color: "#e9eaec"
        font.pixelSize: 12
    }
}
Timer {
    id: toolToastTimer
    interval: 1300
    onTriggered: toolToast.visible = false
}

StatusBar {
    id: statusBar
    host: root
    bridge: bridge
    view: view
    zoomMenu: zoomMenu
}

DropArea {
    anchors.fill: parent
    onDropped: (drop) => {
        if (drop.hasUrls && drop.urls.length > 0)
            confirmThenOpen(drop.urls[0]);
    }
}

LayerPanel {
    id: layerPanel
    host: root
    bridge: bridge
    toolbar: toolbar
    statusBar: statusBar
}

// ---- 调整颜色面板（macOS 预览「调整颜色」，见截图）------------------
// 实时预览只改参数；关闭面板（✕ / 再点工具栏按钮）才把参数烘焙成一次
// 可撤销操作，Esc 放弃（文档像素从未被改动）。
AdjustPanel {
    id: adjustPanel
    host: root
    bridge: bridge
    toolbar: toolbar
    markBar: markBar
    adjustBtn: markBar.adjustBtn
}

// ---- 拾色器弹窗 ------------------------------------------------------
// 可拖动 / 可关闭的取色面板：面板打开期间点击画布取样，逐行显示 HEX /
// RGB / HSL / CMYK 并提供复制按钮（美术同学直接取色值）。
PickerPanel {
    id: pickerPanel
    host: root
    bridge: bridge
    toolbar: toolbar
    markBar: markBar
    pickerBtn: markBar.pickerBtn
}

ResizeOverlay {
    id: resizeOverlay
    bridge: bridge
    host: root
}

function toImageScale() {
    return view.previewImage.width > 0 ? bridge.image_width / view.previewImage.width : 1
}

function toDispScale() {
    return bridge.image_width > 0 && view.previewImage.width > 0
           ? view.previewImage.width / bridge.image_width : 0
}

// 选区真值在 Rust（图像坐标），缩放 / 拖窗口都不会让它漂移——显示框由
// selX/selW 的绑定按当前比例实时换算。这个钩子保留给 CanvasView 的
// 宽度变化通知（曾经在这里把选区重映射到新比例）。
function remapSelectionToScale() {}

// Snap a display coordinate to the nearest image-pixel boundary so every
// crop / selection edge maps to an exact integer image coordinate.
function snapDisplay(v) {
    const s = toDispScale()
    return s > 0 ? Math.round(v / s) * s : v
}

// Canvas cursor for a point in canvasSurface coordinates. selX/selY and
// the layer boxes live in preview-image coordinates, whose origin is
// view.pad to the right/bottom of canvasSurface — shift before comparing.
// topLayerAt() however expects canvasSurface coords directly.
// Fed to the canvasHoverHandler below — HoverHandler cursorShape
// overrides every MouseArea's cursorShape, so this is the single
// authoritative cursor decision.
// ---- 画布光标种类 ---------------------------------------------------
// Qt 内置的 CursorShape 里没有 +/−/剪刀/旋转，QML 也构造不了自定义光标，所以
// 这几种由 Rust 侧的 Win32 光标接管（见 bridge/ops/cursor.rs）。这里只负责
// 算出"此刻该用哪种"，0 = 交回 Qt 自己的光标。
//   1 = 加选(+)  按住 Shift
//   2 = 减选(−)  按住 Alt
//   3 = 剪刀     按住 Ctrl（按下即准备把选区内容剪走）
//   4 = 旋转     悬停在图层旋转手柄上
//   5 = 吸管     修复画笔 / 画笔按住 Alt（取样）
property int cursorKindBase: 0
// 手柄悬停时强行接管：别的逻辑（画布 hover）也会改 cursorKindBase，直接拼
// 一个派生值最稳，不会互相覆盖。
readonly property int cursorKind: root.rotateKnobHover ? 4 : root.cursorKindBase
onCursorKindChanged: bridge.cursor_set_kind(cursorKind)
// 旋转手柄悬停中（SelectionViz 里的小圆点写这个）。
property bool rotateKnobHover: false

// 只在「有选区 + 鼠标在选区内」时才换光标：选区外按修饰键仍然是画新选区的
// 十字，换光标反而误导（PS 也是进到选区里才变）。
// 取样工具是例外：修复画笔 / 画笔只要按住 Alt 就整块画布都变吸管 —— 这两
// 个工具的 Alt 语义就是"取样"，不看选区位不位置。
function updateCursorKind(x, y) {
    let kind = 0
    if (x >= 0 && y >= 0 && !bridge.pickerOpen
            && !view.canvasSurface.textPopup.visible) {
        const bits = bridge.modifier_bits()
        if ((root.paintTool === "heal" || root.markTool === "brush")
                && (bits & 4) !== 0) {
            kind = 5      // Alt 取样
        }
        if (kind === 0 && root.selExists && root.selTool !== "none"
                && root.selTool !== "polygon" && root.markTool === "none"
                && root.paintTool === "none") {
            const ix = x - view.pad
            const iy = y - view.pad
            if (ix >= root.selX && ix <= root.selX + root.selW
                    && iy >= root.selY && iy <= root.selY + root.selH) {
                if ((bits & 2) !== 0)
                    kind = 3      // Ctrl
                else if ((bits & 1) !== 0)
                    kind = 1      // Shift
                else if ((bits & 4) !== 0)
                    kind = 2      // Alt
            }
        }
    }
    root.cursorKindBase = kind
}

function canvasCursor(x, y) {
    // 文本输入中：普通箭头。
    if (view.canvasSurface.textPopup.visible)
        return Qt.ArrowCursor
    // 拾色器打开：整块画布都是取色十字。
    if (bridge.pickerOpen)
        return Qt.CrossCursor
    // 修复 / 油漆工具：画布上永远只是十字 —— 它们没有移动 / 缩放语义，
    // 也不该在图层上变成 SizeAll（否则看起来像能拖图层）。
    if (root.paintTool !== "none")
        return Qt.CrossCursor
    // 标记工具激活：手柄 → 缩放光标；标记上 → 移动；文本工具 → 工字；
    // 其余 → 十字。
    if (root.markTool !== "none") {
        const h = view.canvasSurface.markMa.handleAt(x, y)
        if (h !== "")
            return view.canvasSurface.markMa.handleCursor(h)
        if (view.canvasSurface.markMa.markAt(x, y) >= 0)
            return Qt.SizeAllCursor
        return Qt.CrossCursor
    }
    // 未选绘制工具时：悬停标记仍可移动/编辑（标记不依赖工具栏）。
    if (view.canvasSurface.markMa.markAt(x, y) >= 0)
        return Qt.SizeAllCursor
    const ix = x - view.pad
    const iy = y - view.pad
    if (view.selectionViz.layerHandles.visible) {
        const c = Geometry.boxCursor(ix, iy, view.selectionViz.layerHandles.bx, view.selectionViz.layerHandles.by,
                            view.selectionViz.layerHandles.bw, view.selectionViz.layerHandles.bh)
        if (c >= 0)
            return c
    }
    // 图层"可拖动"光标：移动工具开着、没有选区、鼠标落在未锁定图层上。
    // 有选区时让位给下面的选区框光标（选区优先，不误导成"能拖图层"）。
    if (root.moveMode && !root.selExists && bridge.image_width > 0) {
        const hit = topLayerAt(x, y)
        if (hit !== null && !Layers.isLocked(hit))
            return Qt.SizeAllCursor
    }
    // 选区工具下：边/角 → 方向光标（可拖手柄改大小），框内 → 移动光标。
    // 魔术棒 / 快速选择例外：它们没有拖选区的手势语义（一个点一下取色、
    // 一个按住涂抹），始终是十字，否则悬停在旧选区上会变成"移动"箭头。
    if (root.selExists && root.selTool !== "none" && root.selTool !== "wand"
            && root.selTool !== "quick") {
        const c = Geometry.boxCursor(ix, iy, root.selX, root.selY,
                                     root.selW, root.selH)
        if (c >= 0)
            return c
    }
    return Qt.CrossCursor
}

// Live scale-target box for the selected layer; committed on release.
// ★P1-A（D14 / §9.1）：语义只在 Rust 一处求解。这里只把起手框 / role /
// 光标位移 / 修饰键原样交给 `transform_solve_scale`，拿回解算后的框
//（含锁比例 / 从中心 / 斜切 / 吸附）填进 viz 属性；拖过对边的镜像
// 与斜切量也由求解器给出，本函数不再有任何几何判断。
function resizeLayerFrom(startRect, role, dx, dy, mods) {
    const s = Math.max(toDispScale(), 0.0001)
    let r
    try {
        r = JSON.parse(bridge.transform_solve_scale(
            startRect.x / s, startRect.y / s,
            startRect.width / s, startRect.height / s,
            role, dx / s, dy / s, mods, s))
    } catch (e) {
        return
    }
    if (!r || r.error !== undefined)
        return
    // 等比（角点 + Shift）：记下起手比例，落定时用它反解另一轴（见
    // commitLayerScale）。角点 = role 有两个字母（"se"），单字母是边中点。
    if (root.scaleW <= 0)
        root.scaleStartRatio = startRect.height > 0
                             ? startRect.width / startRect.height : 0
    root.scaleRatioLocked = (mods & 0x02000000) !== 0 && role.length > 1
    // P0-3 / D3：图层变换框不钳画布（画布外不渲染像素，控件允许越界）。
    root.scaleX = r.x * s
    root.scaleY = r.y * s
    root.scaleW = Math.max(1, r.w * s)
    root.scaleH = Math.max(1, r.h * s)
    root.scaleFlipX = !!r.fx
    root.scaleFlipY = !!r.fy
    root.scaleSkewX = r.kx || 0
    root.scaleSkewY = r.ky || 0
    root.scaleCenterDx = r.cx || 0
    root.scaleCenterDy = r.cy || 0
    let edges = 0
    if (role.indexOf("w") >= 0) edges |= 1
    if (role.indexOf("e") >= 0) edges |= 4
    if (role.indexOf("n") >= 0) edges |= 8
    if (role.indexOf("s") >= 0) edges |= 32
    root.scaleEdges = edges
    if (!view.canvasSurface.scaleTimer.running) {
        view.canvasSurface.scaleTimer.lastW = -1; view.canvasSurface.scaleTimer.lastH = -1
        view.canvasSurface.scaleTimer.lastX = 0; view.canvasSurface.scaleTimer.lastY = 0
        view.canvasSurface.scaleTimer.start()
    }
}

function commitLayerScale() {
    view.canvasSurface.scaleTimer.stop()
    const s = toDispScale()
    if (root.scaleW >= 1 && root.scaleH >= 1 && s > 0) {
        // offset 始终是框的左上角（镜像后图层仍填满同一个框），镜像本身
        // 走 flip_x/flip_y；斜切量与补偿原样透传（P1-A）。
        let wImg = Math.max(1, Math.round(root.scaleW / s))
        let hImg = Math.max(1, Math.round(root.scaleH / s))
        // 等比手势：两轴各自取整会把刚保住的宽高比重新打回 ±1px（25% 缩放
        // 下就是 4 图像像素），所以高度统一由宽度反解。
        if (root.scaleRatioLocked && !root.scaleFlipX && !root.scaleFlipY
                && root.scaleStartRatio > 0)
            hImg = Math.max(1, Math.round(wImg / root.scaleStartRatio))
        bridge.scale_layer_end(
            true, wImg, hImg,
            Math.round(root.scaleX / s), Math.round(root.scaleY / s),
            root.scaleFlipX, root.scaleFlipY, root.scaleEdges,
            root.scaleSkewX, root.scaleSkewY)
    }
    root.scaleX = 0; root.scaleY = 0; root.scaleW = 0; root.scaleH = 0
    root.scaleFlipX = false; root.scaleFlipY = false
    root.scaleEdges = 0
    root.scaleSkewX = 0; root.scaleSkewY = 0
    root.scaleCenterDx = 0; root.scaleCenterDy = 0
    root.scaleRatioLocked = false; root.scaleStartRatio = 0
}

function cancelLayerScale() {
    view.canvasSurface.scaleTimer.stop()
    if (root.scaleW > 0)
        bridge.scale_layer_end(false, 0, 0, 0, 0, false, false, 0, 0, 0)
    root.scaleX = 0; root.scaleY = 0; root.scaleW = 0; root.scaleH = 0
    root.scaleFlipX = false; root.scaleFlipY = false
    root.scaleEdges = 0
    root.scaleSkewX = 0; root.scaleSkewY = 0
    root.scaleCenterDx = 0; root.scaleCenterDy = 0
    root.scaleRatioLocked = false; root.scaleStartRatio = 0
}

// ---- 选区工具 -------------------------------------------------------
// 单字母快捷键（M/L/Q）只有在没有任何文本输入获得焦点时才生效，否则会
// 把用户正在输入的字符吞成工具切换。
function canUseToolKeys() {
    // ★PSD 阶段 2 编辑锁：加载未完成时整组工具键都不放行（用户 2026-11-12
    // 定稿）。只判 bridge 属性，不引任何后置 id，避免创建期 TypeError 把
    // enabled 绑定丢掉、解锁后仍永久 false。
    const locked = (typeof bridge !== "undefined" && bridge && bridge.editingLocked)
    // 每项都判空：任何一个 id 尚未实例化时抛 TypeError，整组快捷键的
    // enabled 绑定就会被 QML 丢弃并永久停在 false（V/M/L 全按不动）。
    const tp = (typeof view !== "undefined" && view && view.canvasSurface
                && view.canvasSurface.textPopup)
               ? view.canvasSurface.textPopup : null
    const si = (typeof fontPopover !== "undefined" && fontPopover
                && fontPopover.sizeInput) ? fontPopover.sizeInput : null
    return !locked
        && !(tp !== null && tp.visible)
        && root.renamingIndex < 0
        // 菜单被别处直接藏起来时 selFieldEditing 可能卡在 true；只有菜单
        // 确实开着才认它，否则「开过一次右键菜单 → 快捷键全废」。
        && !(root.selMenuOpen && root.selFieldEditing)
        // 同理：面板已经关掉时，输入框残留的 activeFocus 不算数 —— 它只是
        // 没被清掉（QML 里点画布不会自动 blur），不该把整组工具键废掉。
        && !(si !== null && si.activeFocus && fontPopover.visible)
}

// 画布一按下就放开「浮层面板输入框」的焦点。
// ★字号输入框（字体面板）只要被点过一次，就会一直握着 activeFocus ——
//   面板本身不会因为点画布而关闭，QML 也不会自动 blur。而 Delete 和
//   V/M/L/W/J/G 这些 Shortcut 的 enabled 全都以 activeFocus 为条件，
//   于是用户看到的是「创建元素 → 拖动缩放一下 → 之后删也删不掉、工具
//   键也全按不动」，直到手动关掉面板。面板留在屏幕上（macOS 行为），
//   但焦点必须还给画布。
function dropPopoverFocus() {
    const w = root.hostWindow
    const af = w ? w.activeFocusItem : null
    if (!af)
        return
    // 就地文本编辑框有自己的提交逻辑（textPopup.close），别在这里抢。
    const tp = (typeof view !== "undefined" && view && view.canvasSurface)
               ? view.canvasSurface.textPopup : null
    if (tp !== null && tp !== undefined && af === tp)
        return
    if (af.focus !== undefined)
        af.focus = false
}

function showToolToast(text) {
    toolToastLabel.text = text
    toolToast.visible = true
    toolToastTimer.restart()
}

// 魔术棒参数：从 settings.ini 的 "容差,连续,全部图层" 读入 / 写回。
function applyWandSpec(spec) {
    const parts = String(spec).split(",")
    if (parts.length >= 1) {
        const t = parseInt(parts[0], 10)
        if (!isNaN(t))
            root.wandTolerance = Math.max(0, Math.min(255, t))
    }
    if (parts.length >= 2)
        root.wandContiguous = parts[1].trim() !== "0"
    if (parts.length >= 3)
        root.wandSampleAll = parts[2].trim() !== "0"
}
function saveWandSpec() {
    bridge.save_sel_wand(root.wandTolerance + ","
                         + (root.wandContiguous ? 1 : 0) + ","
                         + (root.wandSampleAll ? 1 : 0))
}
function setWandTolerance(v) {
    root.wandTolerance = v
    root.saveWandSpec()
}
function setWandContiguous(v) {
    root.wandContiguous = v
    root.saveWandSpec()
}
function setWandSampleAll(v) {
    root.wandSampleAll = v
    root.saveWandSpec()
}

// 快速选择的笔刷半径：画布像素，[ / ] 各调 4。上限按 8192 长边的图算，
// 200 已经能一次刷掉一大块；下限 2 保证还能点出一个小圆。
function setQuickRadius(v) {
    root.quickRadius = Math.max(2, Math.min(200, Math.round(v)))
}
function setQuickTolerance(v) {
    root.quickTolerance = Math.max(0, Math.min(255, Math.round(v)))
}
function setQuickGrow(v) {
    root.quickGrow = Math.max(0, Math.min(1000, Math.round(v)))
}

// 切换选区工具。选区工具与标记工具互斥：markMa 盖在画布按下事件之上，
// 两边同时激活时框选手势永远起不来。
function setSelTool(tool) {
    root.selTool = tool
    // 记住这次选的形状：下次启动、以及下次打开面板都用它（写 settings.ini）。
    if (tool !== "none")
        bridge.save_sel_tool(tool)
    // 只切工具本身：不碰标记栏、不碰标记类弹窗。右键菜单里换形状、或从
    // 浮动面板里换形状时，标记栏本来就是开的（后者就在栏里），静默切换。
    if (tool !== "none") {
        root.markTool = "none"
        // 选区工具与修复 / 油漆工具互斥（后者的手势会抢在选区之前）。
        if (root.paintTool !== "none") {
            view.canvasSurface.cancelPaint()
            root.paintTool = "none"
            // 用户手动挑了选区工具：不必再记住「进入前」的那个。
            root.selToolBeforePaint = ""
            root.closePaintPopovers()
            root.savePaintSpec()
        }
        widthPopover.visible = false
        mosaicPopover.visible = false
    }
    view.canvasSurface.cancelPolygon()
    view.canvasSurface.cancelQuickSelect()
}

// M / L 这类快捷键：选区入口在标记栏里，光切工具而栏是关着的话用户找不到
// 浮动面板（也就看不到边界/平滑/羽化），所以快捷键路径顺手把栏带出来。
// 和 setSelTool 分开，避免右键菜单切形状时把标记栏"自动打开"。
function setSelToolByShortcut(tool) {
    if (tool !== "none")
        root.markBarVisible = true
    setSelTool(tool)
}

// 移动工具（PS 的 V）：**开关**，不是工具族。开着时图层可拖可缩放，并且
// 和选区 / 画笔 / 标记工具并存 —— 点手柄缩放的同时照样能画选区、能涂。
// 关掉只影响「能不能碰图层几何」：手柄、描边、尺寸徽标一起收起，画布按下
// 也不再拾取图层。默认开（见 moveMode）。
function toggleMoveTool() {
    root.moveMode = !root.moveMode
    // 立刻给一句可见反馈：以前只能靠手柄是否出现来猜状态，选区还在时
    // 手柄本来就藏着，按 V 看起来像完全没反应。
    root.showToolToast(root.moveMode
                       ? "移动工具：开 · 可拖动 / 缩放 / 旋转图层"
                       : "移动工具：关 · 只绘制与选区")
    if (root.moveMode) {
        // 重新打开时顺手取消正在画的多边形/快选预览，避免半截状态。
        return
    }
    view.canvasSurface.cancelPaint()
    view.canvasSurface.cancelPolygon()
    view.canvasSurface.cancelQuickSelect()
}

// 标记栏「选区」按钮：开/关浮动面板。首次打开时顺手激活矩形选框，
// 关掉面板时不动已经选好的工具（用户还在画选区）。
function toggleSelPopover() {
    if (!root.selPopoverOpen && root.selTool === "none")
        root.selTool = "rect"
    root.selPopoverOpen = !root.selPopoverOpen
}

// 面板里选中某种选区形状：切工具并收起面板，把画布让出来。
function pickSelTool(tool) {
    setSelTool(tool)
    root.selPopoverOpen = false
}

function applyCrop() {
    if (!root.selExists)
        return
    // PS 语义：裁剪到选区**轮廓**，不是包围盒 —— 椭圆/套索/羽化选区之外的
    // 像素变透明，画布缩到选区包围盒。裁剪会移动画布原点，Rust 侧已经顺手
    // 清掉选区。
    bridge.crop_to_selection()
    cancelLayerScale()
}

function deleteSelectionPixels() {
    if (!root.selExists)
        return
    // 羽化/边界都算进"有效覆盖度"里：软边也会按 alpha 擦除。
    bridge.erase_selection_pixels()
}

// Delete / Backspace: 显式选中的标记最优先（持久选区 + 选中标记时，
// Delete 的意图是删标记——选区像素删除先按一次 Esc 取消标记选中，
// 与 Esc 的取消层级一致）；然后是选区像素 → 选中图层 → 可删的背景。
function onDeleteKey() {
    // 就地编辑文本时 Delete 属于输入框，不删标记。
    if (bridge.selected_mark >= 0 && !view.canvasSurface.textPopup.visible) {
        bridge.delete_selected_mark()
        return
    }
    if (root.selExists) {
        deleteSelectionPixels()
        return
    }
    if (bridge.selected_layer >= 0) {
        if (!Layers.isLocked(root.selLayer))
            bridge.delete_selected_layer()
        return
    }
    const base = Layers.findBase(layerList)
    // bd !== true: the base still exists (not "deleted"); visibility is
    // irrelevant for the delete flow.
    if (base !== null && base.bd !== true && !Layers.isLocked(base)
        && bridge.layer_count > 0)
        bridge.delete_base_layer()
}

// Ctrl+A: selection covering the whole canvas (Preview ⌘A).
function selectAllCanvas() {
    if (bridge.image_width < 1 || bridge.image_height < 1)
        return
    settlePaste()
    // 整块画布的选区必然「在选中图层框之外」——和 `dropLayerForSelection`
    // 同一套语义：移动工具开着时先退出图层选中，否则后续抠图 / 调色 / 擦除
    // 只会作用在那一层里（用户看到的「全选之后删不掉整幅图」）。移动工具
    // 关掉时保持原样（纯选区模式）。
    if (root.moveMode && bridge.selected_layer >= 0)
        bridge.select_layer(-1)
    bridge.sel_select_all()
}

// Ctrl+W / Ctrl+Q: close the window through the dirty-confirm guard.
function requestQuit() {
    if (bridge.dirty && !root.forceClose) {
        root.confirmMode = "close"
        confirmOverlay.visible = true
        return
    }
    const w = Window.window
    if (w)
        w.close()
}

// Ctrl+C: prefer the pending selection rect; fall back to the selected
// layer. The clipboard is process-level, so pixels survive opening a
// different document (cross-image paste).
function doCopy() {
    if (root.selExists) {
        bridge.copy_selection_pixels()
    } else if (bridge.selected_layer >= 0) {
        bridge.copy_selected_layer()
    }
}

function doCut() {
    if (root.selExists) {
        // 剪切保留选区（PS 行为）：像素走了，蚂蚁线还在原处。
        bridge.cut_selection_pixels()
    } else if (bridge.selected_layer >= 0) {
        bridge.cut_selected_layer()
    }
}

// Paste enters the floating "place it" state: the pasted layer is auto-
// selected, dragging it repositions live, Enter / clicking outside
// settles, Esc rolls the whole paste back (D3-A).
function doPaste() {
    if (root.pasteFloating)
        settlePaste()
    // Only a successful paste enters the floating placement state —
    // otherwise Esc would "roll back" an unrelated edit.
    if (bridge.paste_clipboard()) {
        root.pasteFloating = true
        root.floatingOps = 1
    }
}

function settlePaste() {
    root.pasteFloating = false
    root.floatingOps = 0
}

function onEscape() {
    // 选区右键菜单里的数值框正在编辑：Esc 先结束编辑、交出键盘焦点
    // （第二次 Esc 才关菜单），否则焦点一直卡在 TextInput 上。
    if (root.selFieldEditing) { selMenu.endEditing(); return }
    if (view.canvasSurface.textPopup.visible) { view.canvasSurface.textPopup.close(false); return }
    // 调色面板：Esc = 放弃（像素从未被改动）。
    if (bridge.adjustOpen) { root.cancelAdjust(); return }
    // 拾色器：Esc = 关面板（已取的色保留）。
    if (bridge.pickerOpen) { bridge.picker_close(); return }
    // 图层旋转对话框：Esc = 放弃（还原旋转前的位图）。
    if (rotateOverlay.visible) { rotateOverlay.close(false); return }
    // 修复 / 油漆：菜单 → 参数面板 → 进行中的手势 → 修补区域，逐层退出。
    if (root.paintMenuOpen) { root.closePaintMenu(); return }
    if (root.paintPopoverOpen) { root.closePaintPopovers(); return }
    if (view.canvasSurface.paintStroke || view.canvasSurface.patchDragging
            || view.canvasSurface.patchDrawing) {
        view.canvasSurface.cancelPaint()
        return
    }
    if (root.patchHasRegion) { view.canvasSurface.clearPatchRegion(); return }
    if (root.selPopoverOpen) { root.selPopoverOpen = false; return }
    if (widthPopover.visible || colorPopover.visible
        || mosaicPopover.visible || fontPopover.visible || zoomMenu.visible) {
        widthPopover.visible = false
        colorPopover.visible = false
        mosaicPopover.visible = false
        fontPopover.visible = false
        zoomMenu.visible = false
        return
    }
    if (layerPanel.contextVisible) { layerPanel.closeContext(); return }
    if (root.renamingIndex >= 0) { root.renamingIndex = -1; return }
    if (confirmOverlay.visible) { confirmCancel(); return }
    if (psdHintOverlay.visible) { psdHintOverlay.dismiss(); return }
    if (resizeOverlay.visible) { resizeOverlay.visible = false; return }
    if (settingsOverlay.visible) { settingsOverlay.visible = false; return }
    if (root.scaleW > 0) { cancelLayerScale(); return }
    // 多边形套索进行中：Esc = 放弃这一串顶点（不动已有选区）。
    if (view.canvasSurface.selPolyActive) { view.canvasSurface.cancelPolygon(); return }
    // 快速选择涂抹中：Esc = 撤销这一笔（回到按下前的选区，不留历史）。
    if (view.canvasSurface.quickDrag) { view.canvasSurface.cancelQuickSelect(); return }
    if (view.canvasSurface.selDragMouse.dragging) { view.canvasSurface.selDragMouse.dragging = false; return }
    if (bridge.selected_mark >= 0) { bridge.select_mark(-1); return }
    if (root.pasteFloating) {
        // D3-A: roll back everything committed since the paste.
        for (var i = 0; i < root.floatingOps; i++)
            bridge.undo()
        settlePaste()
        return
    }
    // ★ 必须走 close()：只翻 selMenuOpen 标志菜单根本不会隐藏，数值框
    // 还留着焦点（selFieldEditing 一直是 true），快捷键就全废了。
    if (root.selMenuOpen) { selMenu.close(); return }
    if (root.selExists) { bridge.sel_clear(); return }
    if (bridge.selected_layer >= 0) { bridge.select_layer(-1); return }
    if (root.panelVisible) { root.panelVisible = false; return }
    root.fullscreen = false
}

function onEnter() {
    // 模态确认框：回车 = 默认动作（保存并打开/切换/关闭）。覆盖丢结构那
    // 一档的默认动作是「另存为…」，不拿回车去覆盖原文件。
    if (confirmOverlay.visible) {
        if (root.confirmMode === "psd-lossy")
            confirmLossySaveAs()
        else
            confirmSaveProceed()
        return
    }
    // 多边形套索进行中：回车 = 闭合（不要把已有选区拿去裁剪）。
    if (view.canvasSurface.selPolyActive) {
        view.canvasSurface.commitPolygon()
        return
    }
    if (root.pasteFloating) {
        settlePaste()
        return
    }
    if (root.selExists)
        applyCrop()
}

function commitExtract() {
    if (!root.selExists)
        return
    // 「提取图层」按选区 alpha 抠出来（羽化软边原样带走）。
    bridge.extract_selection()
}

// 再点一次已选中的标记工具 = 取消（回到 none，不绘制只选择）。
function setMarkTool(tool) {
    root.markTool = (root.markTool === tool) ? "none" : tool
    if (root.markTool === "none") {
        widthPopover.visible = false
        mosaicPopover.visible = false
        markShapePopover.visible = false
    } else if (root.paintTool !== "none") {
        // 标记工具与修复 / 油漆工具互斥。
        view.canvasSurface.cancelPaint()
        root.paintTool = "none"
        // 同样把被 paintTool 抢走的选区工具还回去（见 restoreSelToolAfterPaint）。
        root.restoreSelToolAfterPaint()
        root.closePaintPopovers()
        root.savePaintSpec()
    }
}

// 从形状面板里**选中**某个工具（不是开关语义）：已经选着就保持不动，
// 免得在面板里点自己反而把工具取消了。
function chooseMarkTool(tool) {
    if (root.markTool !== tool)
        root.setMarkTool(tool)
}

// 标记栏「形状与马赛克」按钮：工具开着时再点一次 = **退出**（和移动工具
// 按钮一样的开关语义，用户报的就是「点了关不掉，只能再选别的工具」）；
// 没有工具在用时，这个按钮才用来展开/收起选择面板。
function toggleMarkShapeBtn() {
    fontPopover.visible = false
    widthPopover.visible = false
    mosaicPopover.visible = false
    const active = root.markTool === "rect" || root.markTool === "ellipse"
                || root.markTool === "arrow" || root.markTool === "brush"
                || root.markTool === "mosaic"
    if (!active) {
        markShapePopover.visible = !markShapePopover.visible
        return
    }
    // setMarkTool 是切换语义：同一个工具再设一次就回到 none。
    root.setMarkTool(root.markTool)
    markShapePopover.visible = false
    view.canvasSurface.cancelPaint()
}

// ---- 修复 / 油漆工具族 -------------------------------------------------

function paintGroupOf(tool) {
    if (tool === "spot" || tool === "heal" || tool === "patch")
        return "heal"
    if (tool === "bucket" || tool === "gradient")
        return "paint"
    return ""
}

// 切换修复 / 油漆工具。三个工具族互斥；切走时把进行中的手势撤销掉。
function setPaintTool(tool) {
    const next = (root.paintTool === tool) ? "none" : tool
    // 从「没有修复工具」进入这一族的这一刻，记住当时的选区工具。
    const entering = (next !== "none" && root.paintTool === "none")
    view.canvasSurface.cancelPaint()
    if (entering && root.selTool !== "none")
        root.selToolBeforePaint = root.selTool
    root.paintTool = next
    if (next !== "patch")
        view.canvasSurface.clearPatchRegion()
    if (next !== "none") {
        root.markTool = "none"
        root.selTool = "none"
        root.selPopoverOpen = false
        view.canvasSurface.cancelPolygon()
        view.canvasSurface.cancelQuickSelect()
        widthPopover.visible = false
        mosaicPopover.visible = false
        colorPopover.visible = false
        fontPopover.visible = false
    } else if (root.selTool === "none") {
        root.restoreSelToolAfterPaint()
    }
    root.savePaintSpec()
}

// 退出修复 / 油漆工具后把选区工具还回去。
//
// `setPaintTool` 进入这一族时会把 selTool 置成 "none"（三族互斥）。以前退出
// 时不管它 —— 画布按下就只剩「点空白」，框选整个失灵，用户以为污点工具把
// 选区功能弄坏了。这里还原：优先用进入前记下的那个，其次 settings.ini 里
// 记住的上一个选区工具，最后兜底矩形选框。
function restoreSelToolAfterPaint() {
    if (root.selTool !== "none") {
        root.selToolBeforePaint = ""
        return
    }
    root.selTool = root.selToolBeforePaint !== ""
                ? root.selToolBeforePaint
                : (bridge.selToolSaved.length > 0 ? bridge.selToolSaved : "rect")
    root.selToolBeforePaint = ""
}

// 标记栏上的两个工具组按钮：点开/收起对应的参数面板；该组工具**正在用**
// 时再点一次 = 退出这组工具（开关语义，和移动工具按钮一致）。
function togglePaintGroup(group) {
    if (group !== "heal" && group !== "paint")
        return
    root.markBarVisible = true
    const cur = root.paintTool
    const inside = group === "heal"
                   ? (cur === "spot" || cur === "heal" || cur === "patch")
                   : (cur === "bucket" || cur === "gradient")
    if (inside) {
        // 以前这里只收面板、工具还亮着 —— 按钮看着"关不掉"，只能去选别的
        // 工具才能退出。setPaintTool 是切换语义：同一个工具再设一次 = none。
        root.setPaintTool(cur)
        root.closePaintPopovers()
        return
    }
    root.paintGroup = group
    healPop.visible = (group === "heal")
    paintPop.visible = (group === "paint")
    root.paintPopoverOpen = true
    // 组里还没有激活的工具时，默认选第一个。
    root.setPaintTool(group === "heal" ? "spot" : "bucket")
}

function closePaintPopovers() {
    healPop.visible = false
    paintPop.visible = false
    root.paintPopoverOpen = false
}

// 幂等地「选中某个工具 + 展开它那一组面板」。
// ★不能复用 togglePaintGroup：那现在是开关语义（工具在用时会被关掉），
//   而这里要的是「确保打开」。显式设状态，重复调用也安全。
function openPaintPanelFor(tool) {
    const g = root.paintGroupOf(tool)
    if (g === "")
        return
    if (root.paintTool !== tool)
        root.setPaintTool(tool)
    root.paintGroup = g
    root.paintPopoverOpen = true
    healPop.visible = (g === "heal")
    paintPop.visible = (g === "paint")
}

// 画布右键：当前工具的紧凑参数面板（PS 的右键调参）。
function openPaintMenu(x, y) {
    const g = root.paintGroupOf(root.paintTool)
    if (g === "")
        return false
    paintMenu.group = g
    paintMenu.anchorX = x
    paintMenu.anchorY = y
    paintMenu.visible = true
    root.paintMenuOpen = true
    return true
}

function closePaintMenu() {
    paintMenu.visible = false
    root.paintMenuOpen = false
}

// 打开顶部工具栏的颜色面板（描边 / 填充）。
// 修复 / 油漆的参数面板里点颜色**不**走这里：那边在面板内就地展开色板
//（PaintPopover.colorSlot / ColorPalette），这条路径会收起参数面板。
function openColorFor(fill) {
    root.closePaintPopovers()
    colorPopover.editingFill = fill
    colorPopover.visible = true
}

function setHealSize(v) {
    root.healSize = Math.max(2, Math.min(400, Math.round(v)))
    root.savePaintSpec()
}
function setHealHardness(v) {
    root.healHardness = Math.max(0, Math.min(100, Math.round(v)))
    root.savePaintSpec()
}
function setHealSpacing(v) {
    root.healSpacing = Math.max(1, Math.min(100, Math.round(v)))
    root.savePaintSpec()
}
function setHealStrength(v) {
    root.healStrength = Math.max(1, Math.min(100, Math.round(v)))
    root.savePaintSpec()
}
function setPatchFeather(v) {
    root.patchFeather = Math.max(0, Math.min(40, Math.round(v)))
    root.savePaintSpec()
}
function setBucketTolerance(v) {
    root.bucketTolerance = Math.max(0, Math.min(255, Math.round(v)))
    root.savePaintSpec()
}
function setBucketContiguous(v) {
    root.bucketContiguous = v
    root.savePaintSpec()
}
function setBucketOpacity(v) {
    root.bucketOpacity = Math.max(1, Math.min(100, Math.round(v)))
    root.savePaintSpec()
}
function setGradRadial(v) {
    root.gradRadial = v
    root.savePaintSpec()
}
function toggleGradReverse() {
    root.gradReverse = !root.gradReverse
    root.savePaintSpec()
}
function setGradOpacity(v) {
    root.gradOpacity = Math.max(1, Math.min(100, Math.round(v)))
    root.savePaintSpec()
}

// 12 段：工具|大小|硬度|间距|强度|扩散|容差|连续|不透明|径向|反向|渐变不透明
function paintSpecString() {
    return [root.paintTool, root.healSize, root.healHardness, root.healSpacing,
            root.healStrength, root.patchFeather, root.bucketTolerance,
            root.bucketContiguous ? 1 : 0, root.bucketOpacity,
            root.gradRadial ? 1 : 0, root.gradReverse ? 1 : 0,
            root.gradOpacity].join("|")
}
function savePaintSpec() {
    bridge.save_paint_spec(root.paintSpecString())
}
function applyPaintSpec(spec) {
    const p = String(spec).split("|")
    if (p.length !== 12)
        return
    const num = function (i, lo, hi, dflt) {
        const v = parseInt(p[i], 10)
        return isNaN(v) ? dflt : Math.max(lo, Math.min(hi, v))
    }
    const tools = ["none", "spot", "heal", "patch", "bucket", "gradient"]
    root.paintTool = tools.indexOf(p[0]) >= 0 ? p[0] : "none"
    root.healSize = num(1, 2, 400, 40)
    root.healHardness = num(2, 0, 100, 70)
    root.healSpacing = num(3, 1, 100, 20)
    root.healStrength = num(4, 1, 100, 100)
    root.patchFeather = num(5, 0, 40, 2)
    root.bucketTolerance = num(6, 0, 255, 32)
    root.bucketContiguous = p[7] === "1"
    root.bucketOpacity = num(8, 1, 100, 100)
    root.gradRadial = p[9] === "1"
    root.gradReverse = p[10] === "1"
    root.gradOpacity = num(11, 1, 100, 100)
}

function openRotateLayer(index) {
    if (index < 0)
        return
    // 对话框是「一次性从当前样子再转」的一次性操作（keep_session = false），
    // 原来那份"旋转前框 + 累计角"的记性作废，否则关掉对话框后可能拿旧角度
    // 去画一个对不上的倾斜框。
    root.setRotateFrame(index, 0, 0, 0)
    rotateOverlay.open(index)
}

// ---- 画布上的旋转手柄 ------------------------------------------------
// 手感：按下即出第一帧，之后按 ~30fps 节流重采样（Rust 侧多线程算一层大约
// 4-16ms），松手把最终角度交给 finish 提交。
function startLayerRotate() {
    if (bridge.selected_layer < 0)
        return
    root.rotating = true
    root.rotateAnglePreview = 0
    root.rotateRenderedAngle = 0
    root.rotateFrameCost = 0
    root.rotateLastAt = 0
    // keep_session = true：连续拖手柄时沿用上次留下的原图，避免一遍遍
    // 重新插值（否则越拖越糊）。返回值是本会话已经累计的角度：连续拖着
    // 转时框要接着上次的角度画，不能从 0 重新开始。
    const base = bridge.rotate_layer_begin(bridge.selected_layer, true)
    root.rotateBase = base
    const ix = bridge.selected_layer
    const prev = root.rotateFrames[ix]
    if (Math.abs(base) < 0.001 || prev === undefined) {
        // 新起一轮：记下"还没转"的框，之后的倾斜框以它为准。
        const b = root.selLayerBox
        if (b !== null && b.w > 0 && b.h > 0)
            root.setRotateFrame(ix, b.w, b.h, base)
    } else {
        // 接着上一轮转：沿用当初那张"没转过"的框，只更新角度。
        root.setRotateFrame(ix, prev.w, prev.h, base)
    }
}

function applyRotateFrame() {
    if (!root.rotating)
        return
    // 角度没变就别重算（鼠标停在原地也会一直有 move 事件）。
    if (root.rotateAnglePreview === root.rotateRenderedAngle)
        return
    // 上一帧还没上屏就等等：否则每帧都去换 source，把后台解码取消掉重来，
    // 画面反而一直停在上一帧（见 CanvasView.previewBusy）。
    if (view.previewImage.previewBusy())
        return
    // ★节流放在「定时器网格」上，而不是改 Timer.interval：
    //   给运行中的 Timer 改 interval 会让它重启，帧间隔被抖成 16/24/33 的
    //   随机值——看起来就是"一顿一顿"。这里定时器恒定 16ms 跑，慢帧用
    //   「跳过若干格」来处理，出帧节拍始终是 16ms 的整数倍。
    const now = Date.now()
    if (root.rotateFrameCost > 0 && now - root.rotateLastAt < root.rotateFrameCost)
        return
    root.rotateLastAt = now
    root.rotateRenderedAngle = root.rotateAnglePreview
    const t0 = Date.now()
    // P1-A：修饰键透传（Shift 15° 吸角在 Rust 按累计角解出）。
    bridge.rotate_layer_update(root.rotateAnglePreview, true, root.rotateMods)
    // 记下本帧真实耗时（含同步合成/编码/写盘），下一格据此决定跳不跳。
    root.rotateFrameCost = Math.min(80, Date.now() - t0)
}

function previewLayerRotate(deg, mods) {
    if (!root.rotating)
        return
    root.rotateMods = mods || 0
    root.rotateAnglePreview = deg
    // 首帧立即出（跟着鼠标起手），之后的帧交给定时器。
    root.applyRotateFrame()
}

function finishLayerRotate(apply, mods) {
    if (!root.rotating)
        return
    // 诊断：松手时还没画出来的那一点角度。节流（16ms 网格 + 按帧耗时跳格）
    // 会让最后一帧落后光标，落定却按最新角度提交 —— 差得多时用户会看到
    // 「松手瞬间又转了一点」。日志里超 0.5° 才打，正常拖动不该出现。
    const lag = Math.abs(root.rotateAnglePreview - root.rotateRenderedAngle)
    if (lag > 0.5)
        console.log("[rotate] settle-gap", lag.toFixed(2))
    root.rotating = false
    // 松手时可能还有一帧没算：把最终角度交给 finish，由它决定复用还是重算。
    // P1-A：Shift 吸角与拖动帧同一语义（Rust 按累计角吸附）。
    bridge.rotate_layer_finish(apply, root.rotateAnglePreview, true, mods || 0)
    const ix = bridge.selected_layer
    const f = root.rotateFrames[ix]
    // 框的宽高始终沿用"旋转前"那一份，只在角度上累加；这里绝不能用
    // selLayerBox —— 提交后它已经是扩过边的外接矩形了。
    const w = f !== undefined ? f.w : 0
    const h = f !== undefined ? f.h : 0
    if (!apply) {
        root.setRotateFrame(ix, w, h, root.rotateBase)
        return
    }
    // 帧记录的角度与落定保持同一吸角规则（与 Rust `snap_angle` 相同公式，
    // 按累计角吸附；这只是 viz 簿记，几何真值仍以历史 params 为准）。
    let a = root.rotateBase + root.rotateAnglePreview
    const nearest = Math.round(a / 15) * 15
    const shift = ((mods || 0) & Qt.ShiftModifier) !== 0
    const magnet = ((mods || 0) & root.rotateMagnetBit) !== 0
    if (shift || (magnet && Math.abs(a - nearest) <= 3))
        a = nearest
    // 归一化到 (-180, 180]，转一整圈后框不该"看起来转了 360°"。
    while (a > 180) a -= 360
    while (a <= -180) a += 360
    if (w > 0 && h > 0)
        root.setRotateFrame(ix, w, h, a)
}

Timer {
    id: rotateThrottle
    interval: 16
    repeat: true
    // 只在拖动期间走；起手/松手由 rotating 自己管，省掉散落的 start/stop。
    running: root.rotating
    onTriggered: root.applyRotateFrame()
}

function nudgeLayer(dx, dy) {
    if (bridge.selected_layer < 0)
        return
    bridge.translate_selected(dx, dy)
}

// Hidden helper: TextEdit.copy() puts the (selected) text on the system
// clipboard without needing a C++ clipboard bridge.
TextEdit {
    id: clipHelper
    visible: false
    width: 0
    height: 0
}

function copyToClipboard(text) {
    clipHelper.text = text
    clipHelper.selectAll()
    clipHelper.copy()
    clipHelper.text = ""
}

// ---- 尺子 / 拾色器 ----

function toggleRuler() {
    root.rulerOn = !root.rulerOn
    if (!root.rulerOn)
        root.hoverValid = false
}

function togglePickerPanel() {
    if (bridge.pickerOpen)
        bridge.picker_close()
    else
        bridge.picker_begin()
}

// Mouse position in image coordinates (canvasSurface space input).
// Fed by every area that actually receives hover; the readout only shows
// while the pointer is inside the canvas.
function noteHover(mx, my) {
    const s = toDispScale()
    if (!root.rulerOn || s <= 0) {
        root.hoverValid = false
        return
    }
    const ix = (mx - view.pad) / s
    const iy = (my - view.pad) / s
    root.hoverFx = ix
    root.hoverFy = iy
    root.hoverValid = ix >= 0 && iy >= 0
                      && ix <= bridge.image_width && iy <= bridge.image_height
}

function clearHover() {
    root.hoverValid = false
}

// Copy one color notation and flash "已复制" on its row.
function copyPickerValue(key, value) {
    if (!value)
        return
    copyToClipboard(value)
    root.pickerCopied = key
    pickerCopiedTimer.restart()
}


function findMark(index) {
    return root.markList.find(mk => mk.i === index) || null
}

// ---- 调整颜色：参数镜像 / 节流下发 ----
// Live slider updates are throttled: a 33ms repeat timer (NOT restart())
// flushes the latest pending value, so a fast drag cannot starve it
// (see MEMORY: restart()-based throttling never fires on fast input).
function setAdjust(key, value) {
    root.adjustKey = key
    root.adjustValue = value
    root.adjustDirty = true
}

function flushAdjust() {
    if (!root.adjustDirty)
        return
    root.adjustDirty = false
    if (bridge.adjustOpen)
        bridge.adjust_set(root.adjustKey, root.adjustValue)
}

// Immediate variant for discrete actions (per-row reset, buttons).
function applyAdjustNow(key, value) {
    root.adjustKey = key
    root.adjustValue = value
    root.adjustDirty = false
    if (bridge.adjustOpen)
        bridge.adjust_set(key, value)
}

function setLevels(black, gamma, white) {
    root.pendingBlack = black
    root.pendingGamma = gamma
    root.pendingWhite = white
    root.levelsDirty = true
}

function flushLevels() {
    if (!root.levelsDirty)
        return
    root.levelsDirty = false
    if (bridge.adjustOpen)
        bridge.adjust_set_levels(root.pendingBlack, root.pendingGamma,
                                 root.pendingWhite)
}

function commitAdjust() {
    flushAdjust()
    if (bridge.adjustOpen)
        bridge.adjust_commit()
}

function cancelAdjust() {
    root.adjustDirty = false
    if (bridge.adjustOpen)
        bridge.adjust_cancel()
}

function toggleAdjustPanel() {
    if (bridge.adjustOpen) {
        commitAdjust()
    } else if (bridge.image_width > 0) {
        // 面板锚定在标记栏的 ◑ 按钮下方 → 先确保标记栏可见。
        root.markBarVisible = true
        bridge.adjust_begin()
    }
}

// Selected-mark display bounds [x, y, w, h] relative to the preview
// image origin (image coords × display scale, NO pad — callers that
// live in canvasSurface add view.pad themselves).
function selectedMarkBox() {
    if (bridge.selected_mark < 0)
        return null
    const m = findMark(bridge.selected_mark)
    if (!m)
        return null
    const s = toDispScale()
    return [m.x * s, m.y * s, m.w * s, m.h * s]
}

// Apply the current bar style to the selected mark (if any); otherwise
// the bar values simply become the defaults for the next mark.
function applyMarkStyle() {
    if (bridge.selected_mark >= 0)
        bridge.update_mark_style(root.markStroke, root.markColor,
                                 root.markFill, root.markFillColor)
}

// 颜色弹窗：把自定义 hex 应用到当前目标（描边或填充）。
function customHexApply() {
    if (!colorPopover.customHexInput.valid)
        return
    const hex = "#" + colorPopover.customHexInput.text.toLowerCase()
    if (colorPopover.editingFill) {
        root.markFillColor = hex
        root.markFill = true
    } else {
        root.markColor = hex
    }
    applyMarkStyle()
}

// 字体弹窗：把字体设置应用到选中的文本标记。
function applyTextFont() {
    if (bridge.selected_mark >= 0)
        bridge.update_text_style(root.markFont, root.markFontSize,
                                 root.markBold, root.markItalic,
                                 root.markUnderline, root.markAlign)
}

// 字体 id → QML font.family（用于输入框/面板预览）。
// 可选字体族（与 core::font::FONTS 一一对应），面板与下拉共用。
readonly property var fontChoices: [
    { id: "msyh",   name: "微软雅黑", qml: "Microsoft YaHei" },
    { id: "simsun", name: "宋体",     qml: "SimSun" },
    { id: "simhei", name: "黑体",     qml: "SimHei" },
    { id: "simkai", name: "楷体",     qml: "KaiTi" },
    { id: "arial",  name: "Arial",    qml: "Arial" }
]

function markFontName(id) {
    const f = root.fontChoices.find(f => f.id === id)
    return f ? f.name : id
}

// 字号档位步进（macOS 风格 stepper 用）：dir = +1 / -1。
// 自定义值（不在档位表内）向上/向下取相邻档；sizeInput 显式回写是
// 因为用户手动输入会打断 text 的声明式绑定。
function stepMarkFontSize(dir) {
    const steps = [12, 16, 20, 24, 28, 36, 48, 64]
    let i = 0
    for (let k = 0; k < steps.length; k++)
        if (steps[k] <= root.markFontSize)
            i = k
    i = Math.max(0, Math.min(steps.length - 1, i + dir))
    root.markFontSize = steps[i]
    fontPopover.sizeInput.text = steps[i]
    applyTextFont()
}

// 面板显示行 = 图层行 + 组标题行。
// 组不是图层：这里只在「该组最高的成员」之前插一行标题，折叠时把成员行
// 隐藏掉。真实 z 序完全不变 —— 编组不该改变画面，所以绝不重排 layerList。
// layerList 保持纯图层行，画布点选 / 选层框 / 缩略图都不受组行影响。
function buildLayerPanelRows() {
    const groups = groupList
    const byId = ({})
    for (var g = 0; g < groups.length; g++)
        byId[groups[g].id] = groups[g]
    const rows = []
    const emitted = ({})
    function groupRow(grp, empty) {
        return {
            i: -1, id: -grp.id, gid: grp.id, k: "group",
            n: empty ? grp.n + "（空）" : grp.n,
            v: grp.v, l: grp.l, o: 100,
            c: grp.c, cnt: empty ? 0 : grp.cnt, mids: grp.mids || [],
            x: 0, y: 0, w: 0, h: 0, g: 0, base: false
        }
    }
    for (var i = 0; i < layerList.length; i++) {
        const l = layerList[i]
        const gid = l.g || 0
        if (gid > 0 && byId[gid]) {
            const grp = byId[gid]
            if (!emitted[gid]) {
                emitted[gid] = true
                rows.push(groupRow(grp, false))
            }
            if (grp.c)
                continue
        }
        rows.push(l)
    }
    // 成员被删光的空组仍要留一行，否则用户没有入口把它解散。
    for (var k = 0; k < groups.length; k++)
        if (!emitted[groups[k].id])
            rows.push(groupRow(groups[k], true))
    return rows
}

function findSelLayer() {
    // 背景层已经是普通图层（layers[0]），选中它同样要有轮廓 / 数值 / 不透明度。
    for (var i = 0; i < layerList.length; i++)
        if (layerList[i].i === bridge.selected_layer)
            return layerList[i]
    return null
}

function findLayerBox() {
    for (var i = 0; i < layerList.length; i++)
        if (!layerList[i].base && layerList[i].i === bridge.selected_layer)
            return layerList[i]
    return null
}

// Number of real (non-synthetic-base) rows. The base row exists only
// while the base is still locked; after the panel unlock converts it
// into a regular layer it no longer occupies a slot here.
function nonBaseRowCount() {
    let n = 0
    for (var i = 0; i < root.layerList.length; i++)
        if (root.layerList[i].base !== true)
            n++
    return n
}

// Topmost visible UNLOCKED non-base layer whose content box contains the
// canvasSurface-space point (mx, my); null when the point is empty
// canvas. layerList is display-ordered (index 0 = top), so the first
// hit wins. Hidden and locked layers are not pickable (locked layers
// must be unlocked via the panel's lock icon first).
function topLayerAt(mx, my) {
    const s = toDispScale()
    if (s <= 0)
        return null
    const ix = (mx - view.pad) / s
    const iy = (my - view.pad) / s
    for (var k = 0; k < root.layerList.length; k++) {
        const l = root.layerList[k]
        if (l.base === true || l.v !== true || l.l === true)
            continue
        if (l.w > 0 && l.h > 0
            && ix >= l.x && ix <= l.x + l.w && iy >= l.y && iy <= l.y + l.h)
            return l
    }
    return null
}

ConfirmOverlay {
    id: confirmOverlay
    host: root
}

PsdHintOverlay {
    id: psdHintOverlay
    host: root
}

// Rust publishes a one-shot PSD notice (degraded layers / skipped stage 2 /
// downsampled read-only). This is a pure info overlay — it never blocks save.
property string psdNoticeText: bridge.psdNotice
onPsdNoticeTextChanged: {
    if (psdNoticeText.length > 0)
        psdHintOverlay.showNotice(psdNoticeText, "PSD 分层说明")
}

SettingsOverlay {
    id: settingsOverlay
    bridge: bridge
}

function afterFrame() {
    if (!firstFrameSeen) {
        firstFrameSeen = true;
        bridge.mark("first-frame");
    }
    if (imageRendered && !imageFrameSeen) {
        imageFrameSeen = true;
        bridge.mark("image-frame");
        if (bridge.auto_quit && !root.probeDialog) {
            bridge.mark("quit-requested");
            root.quitting = true
            Qt.quit();
        }
    }
}

// Unsaved-changes guard: opening a file or closing the window goes through
// here, so an edited document is never silently replaced.
// Open a file, honoring the settings-panel choice: reuse the current
// window or spawn a new document window for it.
function openFile(url) {
    // 空窗口永远就地打开：本窗口还没有图时再开一个新窗口，等于把用户刚
    // 打开的这个空窗口晾在那儿（"拖图进来却另开一个窗口"就是这么来的）。
    // "在新窗口打开"这个设置只对已经有图的窗口生效。
    if (bridge.openInNewWindow && bridge.image_width > 0)
        openNewWindow(url)
    else
        bridge.open_file(url)
}

function confirmThenOpen(url) {
    if (bridge.dirty) {
        root.pendingOpenUrl = url
        root.confirmMode = "open"
        confirmOverlay.visible = true
    } else {
        openFile(url)
    }
}

// Left/Right arrow browsing through images in the same folder. Blocked
// while a selection/layer gesture owns the keyboard; an edited document
// asks for confirmation first.
function canNavigate() {
    return bridge.image_width > 0 && !root.selExists
        && bridge.selected_layer < 0 && !root.pasteFloating
        && !root.modeActive && !confirmOverlay.visible
        && !settingsOverlay.visible && !resizeOverlay.visible
        && !bridge.adjustOpen
}

function navigateRel(dir) {
    if (bridge.dirty) {
        root.pendingNavDir = dir
        root.confirmMode = "nav"
        confirmOverlay.visible = true
    } else {
        bridge.navigate(dir)
    }
}

function requestClose(close) {
    // 窗口关闭也必须解锁（C2-c）：任何残留的锁都不该活过本次会话。
    if (bridge.editingLocked)
        bridge.skip_psd_layers()
    if (root.quitting)
        return
    if (bridge.dirty && !root.forceClose) {
        close.accepted = false
        root.confirmMode = "close"
        confirmOverlay.visible = true
    }
}

function confirmProceed() {
    if (root.confirmMode === "open") {
        const url = root.pendingOpenUrl
        root.pendingOpenUrl = null
        confirmOverlay.visible = false
        if (url)
            openFile(url)
    } else if (root.confirmMode === "nav") {
        const dir = root.pendingNavDir
        root.pendingNavDir = 0
        confirmOverlay.visible = false
        bridge.navigate(dir)
    } else if (root.confirmMode === "close") {
        confirmOverlay.visible = false
        root.forceClose = true
        // Close ONLY this window; the app quits automatically once the
        // last window goes away (Qt quitOnLastWindowClosed). Qt.quit()
        // here would kill every other open document window too.
        const w = Window.window
        if (w)
            w.close()
    }
    root.confirmMode = ""
}

// ---- 另存为：后缀按「文件类型」推导 ------------------------------------
// 原生对话框在部分平台不补后缀，另一些平台只认 defaultSuffix（老代码写死
// png，于是选了 Photoshop 也存成 .png）。这里统一按选中过滤器收尾。
function suffixForFilter(name) {
    const f = String(name || "")
    if (f.indexOf("Photoshop") >= 0) return "psd"
    if (f.indexOf("JPEG") >= 0) return "jpg"
    if (f.indexOf("WebP") >= 0) return "webp"
    if (f.indexOf("TIFF") >= 0) return "tiff"
    if (f.indexOf("BMP") >= 0) return "bmp"
    if (f.indexOf("TGA") >= 0) return "tga"
    if (f.indexOf("QOI") >= 0) return "qoi"
    if (f.indexOf("ICO") >= 0) return "ico"
    return "png"
}

// `selectedNameFilter` 只在对话框打开期间有效，缺属性时静默退化为 png。
function currentSaveFilter() {
    try {
        return String(saveDialog.selectedNameFilter || "")
    } catch (e) {
        return ""
    }
}

function saveTargetUrl(url) {
    const s = String(url || "")
    if (s.length === 0)
        return s
    const slash = s.lastIndexOf("/")
    const name = slash >= 0 ? s.slice(slash + 1) : s
    // 已经有了扩展名（用户手打的 .psd / 原生对话框补的）就原样用。
    if (name.indexOf(".") > 0)
        return s
    return s + "." + suffixForFilter(currentSaveFilter())
}

function withPsdSuffix(url) {
    const s = String(url || "")
    if (s.length === 0)
        return s
    return s.toLowerCase().endsWith(".psd") ? s : s + ".psd"
}

// 「另存为 PSD」默认落在源文件同目录、同名 .psd（没有源文件就留空）。
function psdSaveUrl() {
    const p = String(bridge.sourcePath || "").replace(/\\/g, "/")
    if (p.length === 0)
        return ""
    const slash = p.lastIndexOf("/")
    const dir = slash >= 0 ? p.slice(0, slash + 1) : ""
    let base = slash >= 0 ? p.slice(slash + 1) : p
    const dot = base.lastIndexOf(".")
    if (dot > 0)
        base = base.slice(0, dot)
    if (base.length === 0)
        base = "untitled"
    return encodeURI("file:///" + dir + base + ".psd")
}

// 普通图片也能走这条：把当前图层栈写成一个 PSD（背景 = 图层 1）。
function openSaveAsPsd() {
    if (bridge.editingLocked) {
        bridge.psd_locked_hint()
        return
    }
    if (bridge.saving)
        return
    const u = psdSaveUrl()
    if (u.length > 0)
        psdSaveDialog.currentFile = u
    psdSaveDialog.open()
}

// Ctrl+S: overwrite the file the image was loaded from. A document with
// no source file (pasted image, new window) has nothing to overwrite, so
// fall back to the Save-As dialog.
//
// PSD is a first-class save format now: no UI-level interception remains.
// Rust `Session::save_target` is the single gate (composite-only / downsampled
// read-only), and encoding runs on a worker thread.
function saveOrSaveAs() {
    // 阶段 2 未完成时保存入口整体禁用（连同菜单与 Ctrl+S）。
    if (bridge.editingLocked) {
        bridge.psd_locked_hint()
        return
    }
    // 后台编码期间不允许叠加第二次保存。
    if (bridge.saving)
        return
    if (bridge.fileName.length > 0) {
        // 这份 PSD 装载时降级过（组/蒙版/混合模式/裁剪），覆盖写回会把这些
        // 结构永久烧掉。每个文档只问一次。
        if (needsLossySaveWarn()) {
            root.lossyAfter = ""
            root.confirmMode = "psd-lossy"
            confirmOverlay.visible = true
            bridge.mark("psd-lossy-save-warn")
            return
        }
        bridge.save_edited()
    } else {
        saveDialog.open()
    }
}

// 覆盖写回会丢结构、且用户还没确认过 → 需要先弹一次警告。
function needsLossySaveWarn() {
    return bridge.psdWriteLossy && !bridge.psdLossyAck
}

// 「仍然覆盖」：记住这次确认，然后走原来的保存动作（以及被中断的后续动作）。
function confirmLossyOverwrite() {
    bridge.ack_psd_write_loss()
    const after = root.lossyAfter
    root.lossyAfter = ""
    root.confirmMode = ""
    confirmOverlay.visible = false
    bridge.save_edited()
    if (after.length > 0)
        finishPendingAction(after)
}

// 「另存为…」：不碰原文件，直接转另存为对话框。
function confirmLossySaveAs() {
    bridge.ack_psd_write_loss()
    root.lossyAfter = ""
    root.pendingOpenUrl = null
    root.pendingNavDir = 0
    root.confirmMode = ""
    confirmOverlay.visible = false
    saveDialog.open()
}

// Default (highlighted) action of the unsaved-changes dialog: save the
// edited image, then continue whatever the user originally asked for
// (open another file / navigate / close the window).
function confirmSaveProceed() {
    if (bridge.editingLocked) {
        bridge.psd_locked_hint()
        return
    }
    const mode = root.confirmMode
    // No source file: saving needs a destination first; let the user
    // pick one instead of discarding the edits silently.
    if (bridge.fileName.length === 0) {
        confirmOverlay.visible = false
        root.confirmMode = ""
        saveDialog.open()
        return
    }
    // PSD sources are saved in place just like any other document; the
    // Rust gate reports an actionable hint if write-back is unsafe.
    if (needsLossySaveWarn()) {
        // 这次保存属于「保存并打开/切换/关闭」的前半段：先确认覆盖会丢结构，
        // 确认后接着做完后面的动作。
        root.lossyAfter = mode
        root.confirmMode = "psd-lossy"
        bridge.mark("psd-lossy-save-warn")
        return
    }
    bridge.save_edited()
    confirmOverlay.visible = false
    root.confirmMode = ""
    finishPendingAction(mode)
}

// 保存完成之后要接着做的事（打开另一个文件 / 切换上下张 / 关闭窗口）。
function finishPendingAction(mode) {
    if (mode === "open") {
        const url = root.pendingOpenUrl
        root.pendingOpenUrl = null
        if (url)
            openFile(url)
    } else if (mode === "nav") {
        const dir = root.pendingNavDir
        root.pendingNavDir = 0
        bridge.navigate(dir)
    } else if (mode === "close") {
        root.forceClose = true
        // Close ONLY this window; the app quits automatically once the
        // last window goes away (Qt quitOnLastWindowClosed).
        const w = Window.window
        if (w)
            w.close()
    }
}

function confirmCancel() {
    root.pendingOpenUrl = null
    root.pendingNavDir = 0
    root.lossyAfter = ""
    root.confirmMode = ""
    confirmOverlay.visible = false
}

// Probe timer: --probe-settings opens the settings panel automatically
// (UI layout debugging / screenshots without user interaction).
Timer {
    id: settingsProbeTimer
    interval: 600
    onTriggered: settingsOverlay.visible = true
}

// Probe timer: --probe-menu opens the "…" overflow menu automatically.
Timer {
    id: menuProbeTimer
    interval: 600
    onTriggered: overflowMenu.open()
}

// Probe timer: --probe-submenu 顺带把第二个有子项的分类展开（悬停没法脚本
// 化，直接调 showSub 验二级面板的位置和滚动）。
Timer {
    id: submenuProbeTimer
    interval: 700
    onTriggered: {
        overflowMenu.open()
        const tree = overflowMenu.items
        for (let i = 0; i < tree.length; i++) {
            if (tree[i].sub && tree[i].sub.length >= 6) {
                overflowMenu.showSub(tree[i], 5 + i * 26)
                return
            }
        }
    }
}

// Probe timer: --probe-resize opens the resize dialog automatically.
Timer {
    id: resizeProbeTimer
    interval: 900
    onTriggered: resizeOverlay.openResizeDialog()
}

// Probe timer: --probe-layers shows the layer panel automatically.
Timer {
    id: layersProbeTimer
    interval: 600
    onTriggered: root.panelVisible = true
}

// Probe timer: --probe-marks opens the markup bar (rect tool preselected)
// and optionally draws sample marks via --script=add-marks.
Timer {
    id: marksProbeTimer
    interval: 600
    onTriggered: {
        root.markBarVisible = true
        root.markTool = "rect"
    }
}

// Batch-B popover probes: open one popover each for headless screenshots.
Timer {
    id: mosaicProbeTimer
    interval: 600
    onTriggered: {
        root.markBarVisible = true
        root.markTool = "mosaic"
        mosaicPopover.visible = true
    }
}

Timer {
    id: fontProbeTimer
    interval: 600
    onTriggered: {
        root.markBarVisible = true
        // 需要 Aa 按钮可见 → 选中的必须是文本标记。
        if (bridge.image_width > 0)
            bridge.add_text_mark(
                bridge.image_width / 2 - 40, bridge.image_height / 2,
                "文字", root.markFont, root.markFontSize,
                root.markBold, root.markItalic, root.markColor)
        fontPopover.visible = true
    }
}

Timer {
    id: colorProbeTimer
    interval: 600
    onTriggered: {
        root.markBarVisible = true
        root.markTool = "rect"
        colorPopover.visible = true
    }
}

// Headless repro of the ellipse drag preview: mid-drag state (mode 1)
// with the press at image (0,0) and the mouse at 90% of the image.
// Two-phase: paint at 30%, then move the cursor to 90% and requestPaint
// again — if the second frame never shows, the Canvas update pipeline
// itself is dropping paints (the frozen-preview bug).
Timer {
    id: ellipseDragProbeTimer
    interval: 1500
    onTriggered: {
        root.markBarVisible = true
        root.markTool = "ellipse"
        view.canvasSurface.markMa.startX = view.pad
        view.canvasSurface.markMa.startY = view.pad
        view.canvasSurface.markMa.curX = view.pad + view.previewImage.width * 0.3
        view.canvasSurface.markMa.curY = view.pad + view.previewImage.height * 0.3
        view.canvasSurface.markMa.mode = 1
        bridge.mark("probe phase1 requestPaint")
        view.canvasSurface.markPreview.requestPaint()
        phase2Timer.start()
    }
}
Timer {
    id: phase2Timer
    interval: 2000
    onTriggered: {
        view.canvasSurface.markMa.curX = view.pad + view.previewImage.width * 0.9
        view.canvasSurface.markMa.curY = view.pad + view.previewImage.height * 0.9
        bridge.mark("probe phase2 requestPaint")
        view.canvasSurface.markPreview.requestPaint()
    }
}

// Probe: --probe-selection 打开选区工具条并落一个椭圆选区，方便截图核对
// 蚂蚁线 / 尺寸徽标 / 工具条排版。
Timer {
    id: selectionProbeTimer
    interval: 800
    running: root.probeSelection || root.probeSelPoly
    onTriggered: {
        // 探针要截标记栏上的选中态，显式打开（setSelTool 不再顺手开栏）。
        root.markBarVisible = true
        root.setSelTool(root.probeSelPoly ? "polygon" : "ellipse")
        // 多边形探针只画预览，不要先落一个椭圆选区（会遮住判断）。
        if (!root.probeSelPoly && bridge.image_width > 0 && !root.selExists)
            bridge.sel_set_ellipse(bridge.image_width * 0.22,
                                   bridge.image_height * 0.20,
                                   bridge.image_width * 0.78,
                                   bridge.image_height * 0.75, 0)
        if (root.probeSelMenu)
            root.openSelMenu(root.width * 0.42, root.height * 0.34)
        if (root.probeSelPoly) {
            const cs = view.canvasSurface
            const s2 = Math.max(root.toDispScale(), 0.0001)
            cs.polyPts = [[120, 90], [520, 150], [460, 470], [150, 400]]
            cs.polyCurX = view.pad + 620 * s2
            cs.polyCurY = view.pad + 300 * s2
            cs.selPolyActive = true
            cs.selPreview.requestPaint()
        }
    }
}

// Probe: --probe-selpop 打开标记栏 + 选区浮动面板，并落一个矩形选区，
// 用来核对面板排版、标记栏按钮选中态，以及画布有没有被浮层挤走。
Timer {
    id: selPopProbeTimer
    interval: 800
    running: root.probeSelPop
    onTriggered: {
        root.markBarVisible = true
        if (bridge.image_width > 0 && !root.selExists)
            // 参数是「角坐标 + 布尔模式」，漏掉最后的 mode 会让 QML 直接抛
            // TypeError，把本 handler 后面的语句一起跳过（面板就打不开了）。
            bridge.sel_set_rect(bridge.image_width * 0.18,
                                bridge.image_height * 0.18,
                                bridge.image_width * 0.76,
                                bridge.image_height * 0.70, 0)
        // 不要覆盖 setSelTool：selTool 的初值来自 settings.ini（记住上次
        // 用的形状），探针要能把这个记忆状态截出来。
        if (root.selTool === "none")
            root.selTool = "rect"
        root.selPopoverOpen = true
        selPopGeomTimer.start()
    }
}

// 几何自检：--probe-selpop 时打印面板的位置/尺寸/层级，排查"面板没出来"
// 这类问题（常驻行删掉以后，光看截图分不清是没显示还是被别的层盖住）。
Timer {
    id: selPopGeomTimer
    interval: 600
    onTriggered: bridge.mark("selpop vis=" + selPopover.visible
                             + " x=" + Math.round(selPopover.x)
                             + " y=" + Math.round(selPopover.y)
                             + " w=" + Math.round(selPopover.width)
                             + " h=" + Math.round(selPopover.height)
                             + " z=" + selPopover.z
                             + " markBarZ=" + markBar.z
                             + " viewZ=" + view.z)
}

// 平移锁定自检：--probe-panlock 打印一次几何 → 调 panBy → 再打印一次。
// 「装得下」的轴必须纹丝不动（适应窗口就是这种状态）。
Timer {
    id: panLockProbeTimer
    interval: 900
    running: root.probePanLock
    onTriggered: {
        const geom = () => "contentX=" + Math.round(view.contentX)
                          + " contentY=" + Math.round(view.contentY)
                          + " img=" + Math.round(view.previewImage.width)
                          + "x" + Math.round(view.previewImage.height)
                          + " vp=" + Math.round(view.width)
                          + "x" + Math.round(view.height)
                          + " fit=" + bridge.fit_mode
                          + " zoom=" + bridge.zoom_percent
        bridge.mark("panlock before " + geom())
        view.panBy(180, 120)
        bridge.mark("panlock after  " + geom())
    }
}

// 标题栏容量自检：--probe-titlebar 每秒打一行，配合外部改窗口宽度，看
// titleMode / capacity 有没有跟着变（按钮被藏起来时到底剩几个）。
Timer {
    id: titleBarProbeTimer
    interval: 1000
    repeat: true
    running: root.probeTitleBar
    onTriggered: {
        let shown = 0
        for (let i = 0; i < toolbar.barActions.length; ++i)
            if (toolbar.barActions[i].sep !== true && i < toolbar.capacity)
                shown += 1
        bridge.mark("titlebar w=" + Math.round(root.width)
                    + " mode=" + toolbar.titleMode
                    + " cap=" + toolbar.capacity + "/" + toolbar.allSlots
                    + " buttons=" + shown + "/9"
                    + " titleW=" + Math.round(toolbar.titleNameWidth)
                    + "/" + Math.round(toolbar.titleFullWidth)
                    + " right=" + Math.round(toolbar.rightCluster))
    }
}

// 选区工具切换自检：--probe-seltool 打印一次 → 走"菜单/面板换形状"的
// setSelTool → 再打印，最后模拟 M/L 快捷键路径。换形状不该把标记栏带出来
// （那是快捷键路径才有的行为）。
Timer {
    id: selToolProbeTimer
    interval: 900
    running: root.probeSelToolSwitch
    onTriggered: {
        const orig = root.selTool
        bridge.mark("seltool before   bar=" + root.markBarVisible + " tool=" + root.selTool)
        root.setSelTool("lasso")
        bridge.mark("seltool menu     bar=" + root.markBarVisible + " tool=" + root.selTool)
        root.setSelToolByShortcut("polygon")
        bridge.mark("seltool shortcut bar=" + root.markBarVisible + " tool=" + root.selTool)
        // 探针跑完把"记忆的形状"写回原值：setSelTool 会落盘 settings.ini，
        // 不还原的话用户下次启动会莫名变成多边形套索。
        if (orig.length > 0 && orig !== "none")
            bridge.save_sel_tool(orig)
        root.selTool = orig
    }
}

// 窗口几何自检：--probe-winfit 每 1.5s 打一次窗口 x/y/宽高和中心点，然后
// 翻下一张图。用来看"换图时窗口中心有没有跑"（位置/尺寸应该围着中心变）。
Timer {
    id: winFitProbeTimer
    interval: 1500
    repeat: true
    running: root.probeWinFit
    property int step: 0
    onTriggered: {
        step += 1
        const w = root.hostWindow
        if (w)
            bridge.mark("winfit step=" + step
                        + " x=" + Math.round(w.x) + " y=" + Math.round(w.y)
                        + " w=" + Math.round(w.width) + " h=" + Math.round(w.height)
                        + " cx=" + Math.round(w.x + w.width / 2)
                        + " cy=" + Math.round(w.y + w.height / 2))
        if (step <= 4)
            bridge.navigate(1)
    }
}

// Prepare-only: enable the mark bar + ellipse tool, then let a REAL
// (posted) mouse drag drive the interaction.
Timer {
    id: markBarProbeTimer
    interval: 600
    onTriggered: {
        root.markBarVisible = true
        root.markTool = "ellipse"
        // `--probe-shape`：顺带把形状面板打开，核对 5 格布局和图标。
        if (Qt.application.arguments.indexOf("--probe-shape") !== -1)
            markShapePopover.visible = true
    }
}

// Probe: --probe-ruler opens the mark bar, turns the ruler on and seeds a
// selection (plus a hover readout, since a headless run has no real
// pointer) so a screenshot shows the 4 corner + center labels.
Timer {
    id: rulerProbeTimer
    interval: 700
    onTriggered: {
        root.markBarVisible = true
        root.rulerOn = true
        if (!root.selExists && bridge.image_width > 0) {
            bridge.sel_set_rect(bridge.image_width * 0.18,
                                bridge.image_height * 0.22,
                                bridge.image_width * 0.56,
                                bridge.image_height * 0.56, 0)
        }
        if (bridge.image_width > 0) {
            root.hoverFx = bridge.image_width * 0.72
            root.hoverFy = bridge.image_height * 0.58
            root.hoverValid = true
        }
    }
}

// Probe: --probe-picker opens the color-picker panel (colors come from
// --script="picker-pick:x,y;…").
Timer {
    id: pickerProbeTimer
    interval: 700
    onTriggered: {
        root.markBarVisible = true
        if (!bridge.pickerOpen && bridge.image_width > 0)
            bridge.picker_begin()
    }
}

// Probe: --probe-clipboard copies a layer in the FIRST window, opens a
// second window on the same file and pastes there (cross-window clipboard
// check; also covers the DocWindow `fileUrl` hand-off). Launch it with an
// ABSOLUTE image path so the second window can reopen the file.
function clipboardProbeUrl() {
    const p = bridge.sourcePath
    return p.length > 0 ? "file:///" + p.replace(/\\/g, "/") : ""
}
Timer {
    id: clipboardProbeA
    interval: 1200
    running: root.probeClipboard && root.primary
    onTriggered: {
        if (bridge.selected_layer < 0)
            bridge.select_layer(0)
        root.doCopy()
        console.log("PROBE-A window1: copied layer=" + bridge.selected_layer
                    + " layers=" + bridge.layer_count)
        clipboardProbeB.start()
    }
}
Timer {
    id: clipboardProbeB
    interval: 300
    onTriggered: {
        console.log("PROBE-URL sourcePath='" + bridge.sourcePath
                    + "' url='" + root.clipboardProbeUrl() + "'")
        root.requestNewWindow(root.clipboardProbeUrl())
    }
}
Timer {
    id: clipboardProbePaste
    interval: 3200
    running: root.probeClipboard && !root.primary
    onTriggered: {
        console.log("PROBE-B window2: open=" + (bridge.image_width > 0)
                    + " size=" + bridge.image_width + "x" + bridge.image_height
                    + " layers=" + bridge.layer_count)
        root.doPaste()
        console.log("PROBE-B window2: after paste layers=" + bridge.layer_count
                    + " selected=" + bridge.selected_layer
                    + " status=" + bridge.status)
    }
}

// Probe: --probe-save triggers the Ctrl+S path (saveOrSaveAs) on the
// primary window; a successful save logs the usual "saved" event with
// the ORIGINAL file path (overwrite semantics).
Timer {
    id: saveProbeTimer
    // 等到「图片已在、且编辑锁已解除」再按 Ctrl+S（PSD 分层加载期间保存
    // 入口本来就是灰的，早按只会拿到 psd_locked_hint）。
    interval: 700
    repeat: true
    running: root.probeSave && root.primary && bridge.image_width > 0
             && !bridge.editingLocked && !bridge.saving
    onTriggered: {
        stop()
        saveOrSaveAs()
    }
}

// Probe: --probe-accept-lossy 接在 --probe-save 之后，等「覆盖会丢结构」的
// 确认框弹出来再点「仍然覆盖」。不带这个参数时表单必须原样不动（对照组）。
Timer {
    id: lossyAcceptProbeTimer
    interval: 400
    repeat: true
    running: root.probeSave && root.primary
             && Qt.application.arguments.indexOf("--probe-accept-lossy") !== -1
    onTriggered: {
        if (confirmOverlay.visible && root.confirmMode === "psd-lossy") {
            stop()
            confirmLossyOverwrite()
        }
    }
}

// Probe timer: --probe-adjust opens the 调整颜色 panel (live preview of the
// parameters supplied via --script=adjust:...).
Timer {
    id: adjustProbeTimer
    interval: 700
    onTriggered: {
        root.markBarVisible = true
        if (!bridge.adjustOpen && bridge.image_width > 0)
            bridge.adjust_begin()
    }
}

// Probe: --probe-adjustdrag simulates a title-bar drag (moves the popover)
// so a headless screenshot can verify the drag path + window clamping.
Timer {
    id: adjustDragProbeTimer
    interval: 1300
    onTriggered: if (adjustPanel.visible)
                     adjustPanel.moveTo(adjustPanel.x + 150, adjustPanel.y + 130)
}

// Probe: --probe-paint 打开修复 / 油漆工具面板（贴工具栏 or 右键浮层）,
// 让截图能核对参数行与工具互斥态。
Timer {
    id: paintProbeTimer
    interval: 900
    onTriggered: {
        root.markBarVisible = true
        if (Qt.application.arguments.indexOf("--paint-menu") !== -1) {
            // 只拍右键浮层：先把工具选上但不展开工具组面板。
            if (root.paintTool !== root.paintProbeTool)
                root.setPaintTool(root.paintProbeTool)
            root.openPaintMenu(420, 300)
            return
        }
        root.openPaintPanelFor(root.paintProbeTool)
        // --paint-color：顺带展开就地色板，截图核对"点颜色不关面板"。
        if (Qt.application.arguments.indexOf("--paint-color") !== -1) {
            if (root.paintProbeTool === "gradient")
                paintPop.colorSlot = 2
            else
                paintPop.colorSlot = 1
            healPop.colorSlot = paintPop.colorSlot
        }
        // --probe-patch：造一块已圈好的修补区域，核对轮廓虚线样式。
        // setPaintTool 是切换语义，已经选上 patch 时不能再调（会取消掉）。
        if (Qt.application.arguments.indexOf("--probe-patch") !== -1) {
            if (root.paintTool !== "patch")
                root.setPaintTool("patch")
            const cs = view.canvasSurface
            cs.patchRegion = [[600, 400], [1200, 400], [1200, 800], [600, 800]]
            cs.patchViz.requestPaint()
            bridge.mark("patch-probe: region=" + cs.patchRegion.length)
        }
    }
}

// Probe: --probe-selhit 验证「选区工具激活时点画布不拾取图层」这一条真实
// 分支（CanvasSurface.layerPickAt 就是 onPressed 用的那个函数）。
Timer {
    id: selHitProbeTimer
    interval: 1100
    running: Qt.application.arguments.indexOf("--probe-selhit") !== -1
    onTriggered: {
        const cs = view.canvasSurface
        const cx = view.previewImage.width / 2 + view.pad
        const cy = view.previewImage.height / 2 + view.pad
        const top = root.topLayerAt(cx, cy)
        root.selTool = "rect"
        // 不带修饰键：移动工具开着时应当**拾取**（移动优先）。
        // 带 Shift：明确要加选，交回选区手势，picked 应当是 null。
        const picked = cs.layerPickAt(cx, cy, 0)
        const pickedShift = cs.layerPickAt(cx, cy, Qt.ShiftModifier)
        bridge.mark("selhit: top=" + (top === null ? "null" : top.i)
                    + " picked=" + (picked === null ? "null" : picked.i)
                    + " pickedShift=" + (pickedShift === null ? "null" : pickedShift.i)
                    + " moveMode=" + root.moveMode
                    + " selTool=" + root.selTool)
    }
}

// Probe: --probe-move 验证「移动模式（V / 移动按钮）」：切过去之后画布
// 重新拾取图层，八向手柄和旋柄都回来。配合 --script=extract:…;select-layer:0。
Timer {
    id: moveProbeTimer
    interval: 1300
    running: Qt.application.arguments.indexOf("--probe-move") !== -1
    onTriggered: {
        root.moveMode = true
        const cs = view.canvasSurface
        const cx = view.previewImage.width / 2 + view.pad
        const cy = view.previewImage.height / 2 + view.pad
        const picked = cs.layerPickAt(cx, cy)
        bridge.mark("move-probe: moveMode=" + root.moveMode
                    + " selTool=" + root.selTool
                    + " picked=" + (picked === null ? "null" : picked.i)
                    + " handles=" + view.selectionViz.layerHandles.visible
                    + " knob=" + view.selectionViz.rotateHandleBox.visible)
    }
}

// Probe: --probe-select-all / --probe-select-all-off —— 直接调用 Ctrl+A 与
// 菜单「全选」共用的 `selectAllCanvas()`，断言「有选中图层时先取消图层选中」
//（移动工具关掉时保持不动）。按键派发是既有 Shortcut，这里只验逻辑。
Timer {
    id: selectAllProbeTimer
    interval: 1400
    running: Qt.application.arguments.indexOf("--probe-select-all") !== -1
             || Qt.application.arguments.indexOf("--probe-select-all-off") !== -1
    onTriggered: {
        if (Qt.application.arguments.indexOf("--probe-select-all-off") !== -1)
            root.toggleMoveTool()
        if (bridge.layer_count > 0 && bridge.selected_layer < 0)
            bridge.select_layer(0)
        bridge.mark("selall-before: selLayer=" + bridge.selected_layer
                    + " moveMode=" + root.moveMode
                    + " selExists=" + root.selExists)
        root.selectAllCanvas()
        bridge.mark("selall-after: selLayer=" + bridge.selected_layer
                    + " moveMode=" + root.moveMode
                    + " selExists=" + root.selExists)
    }
}

// Probe: --probe-empty-click / --probe-empty-click-off —— 「点空白」在移动
// 工具关掉时必须只清选区、**保留图层选中**（纯选区模式下图层选中是调色 /
// 滤镜 / 抠图的目标，被一次画布点击悄悄清掉就会作用到整幅图）。
// 直接调用画布按下走的同一个 `clickEmptyCanvas()`，不合成鼠标事件。
Timer {
    id: emptyClickProbeTimer
    interval: 1400
    running: Qt.application.arguments.indexOf("--probe-empty-click") !== -1
             || Qt.application.arguments.indexOf("--probe-empty-click-off") !== -1
    onTriggered: {
        const off = Qt.application.arguments.indexOf("--probe-empty-click-off") !== -1
        root.moveMode = !off
        if (bridge.layer_count > 0 && bridge.selected_layer < 0)
            bridge.select_layer(0)
        // 造一个落在图层框外的选区：点空白应该清掉它（两种模式都一样）。
        bridge.sel_set_rect(2, 2, 40, 40, 0)
        bridge.mark("emptyclick-before: moveMode=" + root.moveMode
                    + " selLayer=" + bridge.selected_layer
                    + " selExists=" + root.selExists
                    + " pick=" + (view.canvasSurface.layerPickAt(200, 200) === null
                                   ? "null" : "hit"))
        view.canvasSurface.clickEmptyCanvas()
        bridge.mark("emptyclick-after: moveMode=" + root.moveMode
                    + " selLayer=" + bridge.selected_layer
                    + " selExists=" + root.selExists)
    }
}

// Probe: --probe-toggle-heal / --probe-toggle-paint —— 直接调用标记栏那两个
// 工具组按钮的处理函数（`togglePaintGroup`）：开一次、再关一次，断言关掉之后
// 选区工具回到原样（以前会停在 none，框选整个失灵）。
Timer {
    id: paintToggleProbeTimer
    interval: 1400
    running: Qt.application.arguments.indexOf("--probe-toggle-heal") !== -1
             || Qt.application.arguments.indexOf("--probe-toggle-paint") !== -1
    onTriggered: {
        const grp = Qt.application.arguments.indexOf("--probe-toggle-heal") !== -1
                  ? "heal" : "paint"
        root.markBarVisible = true
        // 探针要跟「上一次存进 settings.ini 的 paintTool」解耦：用户记住的是
        // spot/heal 时，第一次 toggle 是**关**而不是开，断言会整体反向。
        if (root.paintTool !== "none") {
            view.canvasSurface.cancelPaint()
            root.paintTool = "none"
        }
        root.togglePaintGroup(grp)
        bridge.mark("paint-toggle-open: paintTool=" + root.paintTool
                    + " selTool=" + root.selTool)
        root.togglePaintGroup(grp)
        bridge.mark("paint-toggle-close: paintTool=" + root.paintTool
                    + " selTool=" + root.selTool)
    }
}

// Probe: --probe-keys 报出单字母快捷键的 enabled 绑定是否健康。
// 修复前这里会是 vEnabled=false（创建期求值抛异常把绑定丢了）。
// repeat: 用户按键前后各打一行，才能分清「窗口没激活」「enabled 变 false」
// 「绑定都健康但按键不响应」三种完全不同的失效。
Timer {
    id: keysProbeTimer
    interval: 1200
    repeat: true
    // 只在 --probe-keys 下跑；其余时候必须闭嘴（它会临时弹出字体面板）。
    running: Qt.application.arguments.indexOf("--probe-keys") !== -1
    property int ticks: 0
    onTriggered: {
        // 整段包 try：QML 里 onTriggered 抛异常不一定进日志（之前就是
        // `Window.window` 抛错 → 探针静默不输出），catch 后再 mark 一次。
        try {
        ticks += 1
        // 焦点模拟只在 --probe-keys-focus 下做：它会弹出字体面板并抢焦点，
        // 平时跑 --probe-keys 是要观察**真实**手势之后的状态，不能被探针
        // 自己打乱（用户复现「拖动缩放后删不了」时就踩到过这个）。
        if (Qt.application.arguments.indexOf("--probe-keys-focus") !== -1) {
            // 第 3 tick：模拟「用户点过字号输入框」（面板开着、焦点在输入框）；
            // 第 8 tick：关面板 —— 之后 canUse / delEnabled 必须回到 true。
            // 留 5 个 tick 的窗口，够外部探针在「面板开着」这段时间里点一下
            // 画布、验证画布是否会把这个焦点收走。
            if (ticks === 3) {
                fontPopover.visible = true
                fontPopover.sizeInput.forceActiveFocus()
            } else if (ticks === 8) {
                fontPopover.visible = false
            }
        }
        let canUse = "ERR"
        try { canUse = root.canUseToolKeys() } catch (e) { canUse = "throw:" + e }
        const win = root.hostWindow
        const af = win ? win.activeFocusItem : null
        const afName = af ? String(af).replace(/\s*\(.*$/, "") : "null"
        bridge.mark("keys-probe: appActive=" + Qt.application.active
                    + " winActive=" + (win ? win.active : "?")
                    + " hasWin=" + (win ? "y" : "n")
                    + " focusItem=" + afName
                    + " ready=" + root.toolKeysReady
                    + " vEnabled=" + moveShortcut.enabled
                    + " delEnabled=" + deleteShortcut.enabled
                    + " selMark=" + bridge.selected_mark
                    + " selLayer=" + bridge.selected_layer
                    + " selExists=" + root.selExists
                    + " moveMode=" + root.moveMode
                    + " canUse=" + canUse
                    + " selFieldEditing=" + root.selFieldEditing
                    + " selMenuOpen=" + root.selMenuOpen
                    + " renaming=" + root.renamingIndex
                    + " textPopup=" + view.canvasSurface.textPopup.visible
                    + " sizeFocus=" + fontPopover.sizeInput.activeFocus
                    + " markTool=" + root.markTool
                    + " paintTool=" + root.paintTool
                    + " selTool=" + root.selTool
                    + " layers=" + bridge.layer_count
                    + " mbox=" + (function () {
                        const b = root.selectedMarkBox()
                        return b ? [Math.round(b[0]), Math.round(b[1]),
                                    Math.round(b[2]), Math.round(b[3])].join(",") : "null"
                    })()
                    + " lbox=" + (root.selLayerBox ? [Math.round(root.selLayerBox.x),
                                    Math.round(root.selLayerBox.y),
                                    Math.round(root.selLayerBox.w),
                                    Math.round(root.selLayerBox.h)].join(",") : "null")
                    + " mmode=" + view.canvasSurface.markMa.mode
                    + " pad=" + Math.round(view.pad)
                    + " scale=" + root.toDispScale().toFixed(5)
                    + " marks=" + (root.markList && root.markList.length !== undefined
                                    ? root.markList.length : "?"))
        } catch (e) {
            bridge.mark("keys-probe-ERR: " + e)
        }
    }
}

// Probe: --probe-session 报统一变换会话（P0-2b1）。配合 --script 的
// rotate-hold / transform-state / transform-abort 断言「同一时刻只有一个手势」。
Timer {
    id: sessionProbeTimer
    interval: 1100
    running: Qt.application.arguments.indexOf("--probe-session") !== -1
    onTriggered: bridge.mark("session-probe: " + bridge.transform_state())
}

// Probe: --probe-rotate 选中最上面的图层（配合 --script=extract:… 造图层），
// 显示画布旋转手柄；再加 --rotate-dialog 打开角度对话框。
Timer {
    id: rotateProbeTimer
    interval: 1100
    onTriggered: {
        root.markBarVisible = true
        // 旋转手柄只在「没有选区、没有绘画工具」时出现。
        if (root.paintTool !== "none") {
            root.paintTool = "none"
            root.closePaintPopovers()
        }
        if (bridge.selected_layer < 0 && bridge.layer_count > 0)
            bridge.select_layer(0)
        if (Qt.application.arguments.indexOf("--rotate-dialog") !== -1
                && bridge.selected_layer >= 0)
            root.openRotateLayer(bridge.selected_layer)
        const b = root.selLayerBox
        bridge.mark("rotbox:" + (b ? (Math.round(b.x) + "," + Math.round(b.y) + ","
                                      + Math.round(b.w) + "," + Math.round(b.h)) : "null")
                    + " scale=" + root.toDispScale().toFixed(4)
                    + " previewW=" + Math.round(view.previewImage.width)
                    + " canvas=" + bridge.image_width + "x" + bridge.image_height
                    + " scaleW=" + Math.round(root.scaleW) + "," + Math.round(root.scaleH)
                    + " rotating=" + root.rotating)
        // --probe-hand-rotate=deg：走**真实**的 QML 手势路径
        //（start → preview → finish），验证旋转框状态机；直接调 Rust 的
        // `rotate-handle:` 脚本不会经过这里。
        const args = Qt.application.arguments
        let rotFirst = true
        for (let i = 0; i < args.length; ++i) {
            if (args[i].indexOf("--probe-hand-rotate=") !== 0)
                continue
            // --probe-frame-move：两次旋转之间把图层挪一下，核对锚点还接得上
            // （以前挪动会让 base 归零，包围盒转一次大一圈）。
            if (!rotFirst && args.indexOf("--probe-frame-move") !== -1)
                bridge.translate_selected(7, 3)
            rotFirst = false
            const deg = parseFloat(args[i].substring("--probe-hand-rotate=".length)) || 45
            root.startLayerRotate()
            root.previewLayerRotate(deg, 0)
            root.finishLayerRotate(true, 0)
            bridge.mark("hand-rotate: deg=" + deg + " base=" + root.rotateBase
                        + " angle=" + root.layerFrameAngle
                        + " rotated=" + root.layerFrameRotated
                        + " fw=" + root.layerFrameW + "x" + root.layerFrameH
                        + " box=" + (root.selLayerBox
                                     ? root.selLayerBox.w + "x" + root.selLayerBox.h : "null")
                        + " handles=" + view.selectionViz.layerHandles.visible)
            // --probe-frame-keep：转完切到别的图层再切回来，核对倾斜框
            // 记忆还在（以前切一次就退回轴对齐，看着像角度复位）。
            if (args.indexOf("--probe-frame-keep") !== -1) {
                const keep = bridge.selected_layer
                bridge.select_layer(-1)
                bridge.select_layer(keep)
                bridge.mark("frame-keep: back=" + bridge.selected_layer
                            + " angle=" + root.layerFrameAngle
                            + " rotated=" + root.layerFrameRotated
                            + " fw=" + root.layerFrameW + "x" + root.layerFrameH)
            }
        }
    }
}

// Probe: --probe-dragchurn 用 30ms 的节奏连续出预览帧（模拟拖动），跑 40 帧
// 后自己退出。用来核对两件事：① 拖动期间每帧都真的上屏了（日志里
// `shown_ms` 行的条数 ≈ 帧数）；② 外部截图时画面不是空白。
Timer {
    id: dragChurnTimer
    interval: 30
    repeat: true
    property real angle: 0
    property int sent: 0
    property bool started: false
    onTriggered: {
        if (!started) {
            if (bridge.selected_layer < 0 && bridge.layer_count > 0)
                bridge.select_layer(0)
            if (bridge.selected_layer < 0) {
                bridge.mark("dragchurn:no-layer")
                Qt.quit()
                return
            }
            bridge.rotate_layer_begin(bridge.selected_layer, true)
            started = true
        }
        angle += 2
        sent += 1
        bridge.rotate_layer_update(angle, true, 0)
        if (sent >= 40) {
            bridge.rotate_layer_finish(false, 0, true, 0)
            bridge.mark("dragchurn:" + sent + " frames")
            Qt.quit()
        }
    }
}

// Probe: --probe-rotate-frame 在脚本转完之后补上「旋转框」状态，用来截图
// 核对倾斜虚线框。参数：--probe-rotate-frame=w,h,deg（默认 900,700,45）。
Timer {
    id: rotateFrameProbe
    interval: 1200
    repeat: false
    onTriggered: {
        let spec = "900,700,45"
        const args = Qt.application.arguments
        for (let i = 0; i < args.length; ++i)
            if (args[i].indexOf("--probe-rotate-frame=") === 0)
                spec = args[i].substring("--probe-rotate-frame=".length)
        const p = spec.split(",")
        const ix = bridge.selected_layer >= 0 ? bridge.selected_layer : 0
        root.setRotateFrame(ix,
                            parseInt(p[0], 10) || 900,
                            parseInt(p[1], 10) || 700,
                            parseFloat(p[2]) || 45)
        bridge.mark("rotate-frame:" + JSON.stringify(root.rotateFrame)
                    + " angle=" + root.rotateFrameAngle
                    + " rotated=" + root.layerFrameRotated)
    }
}

// Probe: --probe-rotate-bench 连着跑 N 帧「整层重采样 + 预览刷新」，量每次
// 耗时（手柄拖动的跟手程度直接由它决定），跑完自己退出。
Timer {
    id: rotateBenchTimer
    interval: 900
    onTriggered: {
        if (bridge.selected_layer < 0 && bridge.layer_count > 0)
            bridge.select_layer(0)
        if (bridge.selected_layer < 0) {
            bridge.mark("rotate-bench:no-layer")
            Qt.quit()
            return
        }
        const n = 20
        bridge.rotate_layer_begin(bridge.selected_layer, true)
        const t0 = Date.now()
        for (let i = 1; i <= n; ++i)
            bridge.rotate_layer_update(i * 1.5, true, 0)
        const t1 = Date.now()
        bridge.rotate_layer_finish(false, 0, true, 0)
        bridge.mark("rotate-bench:" + ((t1 - t0) / n).toFixed(1) + "ms/frame")
        Qt.quit()
    }
}

Component.onCompleted: {
    bridge.mark("qml-completed");
    // 此刻整棵组件树已建好，才放行单字母工具快捷键的 enabled 绑定
    //（见 toolKeysReady 的注释）。
    root.toolKeysReady = true
    // 魔术棒参数：settings.ini 里的 "容差,连续,全部图层"。
    root.applyWandSpec(bridge.selWandSaved);
    // 修复 / 油漆工具参数（含上次使用的工具），`|` 分隔 12 段。
    // ★探针要用**干净的工具状态**：上次会话留下的 spot/heal 会让「点画布 =
    //   拾取图层」变成「点画布 = 落笔」，真鼠标探针里图层一动不动，看起来像
    //   吸附坏了。只在带 `--no-restore-tool` 的探针跑里跳过恢复。
    if (Qt.application.arguments.indexOf("--no-restore-tool") === -1)
        root.applyPaintSpec(bridge.paintSpecSaved);
    // Push the screen's device-pixel-ratio so the first preview frame is
    // rendered at the physical resolution (onDprChanged alone may not fire
    // for the initial value).
    bridge.set_dpr(dpr);
    if (root.primary) {
        bridge.start();
        if (root.probeDialog)
            dialogProbeTimer.start();
        if (root.probeSettings)
            settingsProbeTimer.start();
        if (root.probeMenu)
            menuProbeTimer.start();
        if (Qt.application.arguments.indexOf("--probe-submenu") !== -1)
            submenuProbeTimer.start();
        if (root.probeResize)
            resizeProbeTimer.start();
        if (root.probeLayers)
            layersProbeTimer.start();
        if (root.probeMarks)
            marksProbeTimer.start();
        if (Qt.application.arguments.indexOf("--probe-mosaic") !== -1)
            mosaicProbeTimer.start();
        if (Qt.application.arguments.indexOf("--probe-font") !== -1)
            fontProbeTimer.start();
        if (Qt.application.arguments.indexOf("--probe-color") !== -1)
            colorProbeTimer.start();
        if (Qt.application.arguments.indexOf("--probe-ellipsedrag") !== -1)
            ellipseDragProbeTimer.start();
        if (Qt.application.arguments.indexOf("--probe-markbar") !== -1)
            markBarProbeTimer.start();
        if (root.probeAdjust)
            adjustProbeTimer.start();
        if (root.probePaint)
            paintProbeTimer.start();
        if (root.probeRotate)
            rotateProbeTimer.start();
        if (Qt.application.arguments.indexOf("--probe-rotate-bench") !== -1)
            rotateBenchTimer.start();
        if (Qt.application.arguments.indexOf("--probe-rotate-frame") !== -1)
            rotateFrameProbe.start();
        if (Qt.application.arguments.indexOf("--probe-dragchurn") !== -1)
            dragChurnTimer.start();
        if (Qt.application.arguments.indexOf("--probe-adjustdrag") !== -1)
            adjustDragProbeTimer.start();
        if (root.probeRuler)
            rulerProbeTimer.start();
        if (root.probePicker)
            pickerProbeTimer.start();
    } else if (root.fileUrl.toString().length > 0) {
        // Document window spawned for a specific file ("open in separate
        // window" setting, drag-out, etc.).
        bridge.open_file(root.fileUrl)
    }
}
}
