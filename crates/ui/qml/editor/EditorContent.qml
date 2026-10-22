import QtQuick
import QtQuick.Window
import QtQuick.Dialogs
import com.imageedit

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
    signal requestNewWindow(url fileUrl)
    property bool firstFrameSeen: false
    property bool imageRendered: false
    property bool imageFrameSeen: false
    readonly property int toolbarHeight: 40
    readonly property int statusHeight: 26

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
    property real selX: 0
    property real selY: 0
    property real selW: 0
    property real selH: 0
    readonly property bool selExists: selW >= 3 && selH >= 3
    // Transient "rubber band in use" state; hides the handle dots and
    // disables the wheel zoom while dragging a new selection.
    readonly property bool selRubberBanding: selDragMouse.dragging
    readonly property bool modeActive: selRubberBanding || pasteFloating
    // Scale target for the selected layer (display coords); empty when idle.
    property real scaleX: 0
    property real scaleY: 0
    property real scaleW: 0
    property real scaleH: 0
    property int renamingIndex: -1
    readonly property var layerList: parseLayers(bridge.layers_json)
    // The layer PANEL rebuilds its Repeater whenever the model array is
    // replaced. Bounds (x/y/w/h) change on every live-drag tick, which
    // would rebuild the whole list ~30x/s; the panel only shows
    // identity/flags, so rebind it only when those actually change.
    // Canvas outlines etc. keep using layerList (live bounds).
    readonly property string layerPanelSig: {
        let s = ""
        for (var i = 0; i < layerList.length; i++) {
            const l = layerList[i]
            s += l.i + ":" + l.n + ":" + l.v + ":" + l.l + ":" + Math.round(l.o) + ";"
        }
        return s
    }
    property var layerPanelList: layerList
    onLayerPanelSigChanged: layerPanelList = layerList
    readonly property var selLayer: findSelLayer()
    readonly property var selLayerBox: findLayerBox()
    // ---- 标记工具栏（markup toolbar）----
    property bool markBarVisible: false
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
    readonly property var markList: parseMarks(bridge.marks_json)
    // ---- 调整颜色（macOS 预览风格）----
    // Mirrored parameters (slider positions) and the throttled live push.
    readonly property var adjustVals: parseAdjust(bridge.adjustValues)
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
    // ---- 尺子（测量坐标）----
    // 开关只是显示层，不动文档；坐标全部是「相对画布」的图像像素坐标。
    property bool rulerOn: false
    // 鼠标在画布上的图像坐标（可为小数；落在画布外时 hoverValid=false）。
    property real hoverFx: 0
    property real hoverFy: 0
    property bool hoverValid: false
    // ---- 拾色器 ----
    readonly property var pickerVals: parsePicker(bridge.pickerColor)
    readonly property var pickerHist: parsePickerHistory(bridge.pickerHistory)
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
            // New canvas: drop any pending selection / scale preview.
            root.selW = 0
            root.selH = 0
            root.scaleW = 0
            root.scaleH = 0
        }
    }

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
            if (bridge.openInNewWindow)
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
        running: bridge.loading && !root.quitting
        onTriggered: bridge.poll_decode()
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
    defaultSuffix: "png"
    nameFilters: ["PNG (*.png)", "JPEG (*.jpg)", "WebP (*.webp)", "TIFF (*.tiff)", "BMP (*.bmp)", "TGA (*.tga)", "QOI (*.qoi)", "ICO (*.ico)"]
    onAccepted: bridge.save_as(selectedFile)
}

Shortcut { sequences: [StandardKey.Open]; onActivated: openDialog.open() }
Shortcut { sequences: [StandardKey.New]; onActivated: root.requestNewWindow() }
Shortcut { sequences: [StandardKey.SaveAs]; onActivated: saveDialog.open() }
Shortcut { sequence: "F11"; onActivated: root.fullscreen = !root.fullscreen }
Shortcut { sequence: "Escape"; onActivated: onEscape() }
// Browse images in the loaded file's folder (guard: no active gesture).
// Dialogs block navigation so arrow keys never swap the file behind a
// open settings/resize panel.
Shortcut { sequence: "Left"; enabled: canNavigate(); onActivated: navigateRel(-1) }
Shortcut { sequence: "Right"; enabled: canNavigate(); onActivated: navigateRel(1) }
// 回车：模态确认框 = 默认动作（保存并打开/切换/关闭）；否则裁剪/粘贴。
Shortcut { sequences: ["Return", "Enter"]
           enabled: confirmOverlay.visible || root.pasteFloating
                    || root.selExists
           onActivated: onEnter() }

Shortcut { sequence: "Left";  enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(-1, 0) }
Shortcut { sequence: "Right"; enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(1, 0) }
Shortcut { sequence: "Up";    enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(0, -1) }
Shortcut { sequence: "Down";  enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(0, 1) }
Shortcut { sequence: "Shift+Left";  enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(-10, 0) }
Shortcut { sequence: "Shift+Right"; enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(10, 0) }
Shortcut { sequence: "Shift+Up";    enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(0, -10) }
Shortcut { sequence: "Shift+Down";  enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(0, 10) }

Shortcut { sequences: [StandardKey.ZoomIn, "Ctrl+="]; onActivated: view.queueZoom(1.25) }
Shortcut { sequences: [StandardKey.ZoomOut]; onActivated: view.queueZoom(1 / 1.25) }
// Preview-aligned: ⌘0 actual size, ⌘9 zoom-to-fit (old keys kept as aliases).
Shortcut { sequences: ["Ctrl+0", "Ctrl+1"]; onActivated: bridge.zoom_actual() }
Shortcut { sequences: ["Ctrl+9"]; onActivated: bridge.zoom_fit() }
Shortcut { sequences: [StandardKey.Undo]; onActivated: bridge.undo() }
// Redo: explicit keys — StandardKey.Redo maps inconsistently across
// platforms/layouts, plain strings fire reliably (Ctrl+Y and ⇧Ctrl+Z).
Shortcut { sequences: ["Ctrl+Shift+Z", "Ctrl+Y"]; onActivated: bridge.redo() }
Shortcut { sequences: [StandardKey.Save]; onActivated: saveOrSaveAs() }
Shortcut { sequences: [StandardKey.Copy]; onActivated: doCopy() }
Shortcut { sequences: [StandardKey.Cut]; onActivated: doCut() }
Shortcut { sequences: [StandardKey.Paste]; onActivated: doPaste() }
Shortcut { sequence: "Ctrl+K"; enabled: root.selExists; onActivated: applyCrop() }
// 调整颜色面板（macOS 预览的调色入口）。
Shortcut { sequence: "Ctrl+E"; enabled: bridge.image_width > 0
           onActivated: root.toggleAdjustPanel() }
Shortcut { sequence: "Ctrl+L"; onActivated: bridge.rotate_ccw() }
Shortcut { sequence: "Ctrl+Shift+R"; onActivated: bridge.rotate_cw() }
// Delete/Backspace 在文本输入（文本标记就地编辑 / 字号输入 / 图层
// 重命名）获得焦点时必须禁用：QML Shortcut 先于焦点项拦截按键，
// 不禁用会吞掉输入框的退格键，还会误删选中的标记/图层。
Shortcut { sequences: ["Delete", "Backspace"]
           enabled: !textPopup.visible && !fontPopover.sizeInput.activeFocus
                    && root.renamingIndex < 0
                    && (root.selExists || bridge.selected_mark >= 0
                    || bridge.selected_layer >= 0
                    || (function () {
                        const b = findBase()
                        return b !== null && b.bd !== true && !isLocked(b)
                            && bridge.layer_count > 0
                    })())
           onActivated: onDeleteKey() }
Shortcut { sequence: "Ctrl+A"
           enabled: bridge.image_width > 0 && root.renamingIndex < 0
           onActivated: selectAllCanvas() }
// ⌘W / ⌘Q: close window / quit, both through the dirty-confirm guard.
Shortcut { sequence: "Ctrl+W"; onActivated: requestQuit() }
Shortcut { sequence: "Ctrl+Q"; onActivated: requestQuit() }
Shortcut { sequence: "Ctrl+R"; onActivated: bridge.rotate_cw() }

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
    textPopup: textPopup
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

Flickable {
    id: view

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

    // In fit mode the content includes the 64px work-area pad on every
    // side, so it is slightly larger than the viewport — without recentring
    // the image would sit 64px down (blank top, cut bottom). Center the
    // image in the viewport whenever fit sizing changes.
    function centerContent() {
        if (!bridge.fit_mode)
            return
        // 取整：contentX/Y 若带 .5，整块内容（含 1:1 的预览位图）都会被
        // 半像素采样糊掉，与 previewImage.x/y 取整是同一件事的两半。
        contentX = Math.round(Math.max(0, (contentWidth - width) / 2))
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
    readonly property real fitScalePreview: {
        if (!bridge.fit_mode || bridge.image_width <= 0 || bridge.image_height <= 0)
            return 0
        return Math.min(width / bridge.image_width,
                        height / bridge.image_height, 1.0)
    }
    // 画布当前应当显示的尺寸：待渲染期间用预测值，其余用 bridge 的真值。
    // 缩放手势的预测优先（fit 已被 zoom_by 置 false）。
    readonly property real shownWidth: {
        if (zoomPending && predictedZoom > 0)
            return Math.max(1, Math.round(bridge.image_width * predictedZoom))
        if (viewportPending && fitScalePreview > 0)
            return Math.max(1, Math.round(bridge.image_width * fitScalePreview))
        return Math.max(1, bridge.display_width)
    }
    readonly property real shownHeight: {
        if (zoomPending && predictedZoom > 0)
            return Math.max(1, Math.round(bridge.image_height * predictedZoom))
        if (viewportPending && fitScalePreview > 0)
            return Math.max(1, Math.round(bridge.image_height * fitScalePreview))
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
    function panBy(dx, dy) {
        const maxX = Math.max(0, contentWidth - width)
        const maxY = Math.max(0, contentHeight - height)
        // 取整：触摸板/滚轮的增量常带小数，contentX 一旦是 .5，1:1 的预览
        // 位图就会按半像素中心采样（整幅变糊）。
        contentX = Math.round(Math.min(maxX, Math.max(0, contentX + dx)))
        contentY = Math.round(Math.min(maxY, Math.max(0, contentY + dy)))
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

    Image {
        id: previewImage

        width: view.shownWidth
        height: view.shownHeight
        // 必须落在整数像素上：预览位图与显示尺寸是 1:1 的，一旦 x/y 出现
        // .5（奇数宽高居中时很常见，例如 31% 的 497×311），GPU 就会按
        // 半像素中心做双线性采样，整张画面被均匀糊掉——这正是"某些缩放
        // 比例看着模糊"的根因。居中偏移一律取整。
        x: Math.round((view.contentWidth - width) / 2)
        y: Math.round((view.contentHeight - height) / 2)
        cache: false
        fillMode: Image.Stretch
        source: bridge.preview_url

        // 显示比例一变（缩放手势的预测拉伸也算）就重映射选区。
        onWidthChanged: root.remapSelectionToScale()

        onStatusChanged: {
            if (status === Image.Ready) {
                imageRendered = true;
                bridge.image_shown();
            } else if (status === Image.Error) {
                console.log("ImageEdit: preview load error");
                if (bridge.auto_quit)
                    Qt.quit();
            }
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

    Item {
        id: canvasSurface
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
                const hit = root.selExists ? null : topLayerAt(mouse.x, mouse.y)
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
                root.selW = 0
                root.selH = 0
                settlePaste()
            }
            onPositionChanged: (mouse) => {
                if (root.rulerOn)
                    root.noteHover(mouse.x, mouse.y)
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
                root.selX = snapDisplay(x1 - pad)
                root.selY = snapDisplay(y1 - pad)
                root.selW = Math.max(snapDisplay(x2 - x1), toDispScale())
                root.selH = Math.max(snapDisplay(y2 - y1), toDispScale())
            }
            onReleased: {
                if (pickDrag) {
                    pickDrag = false
                    if (pickMoved) {
                        liveTimer.stop()
                        const scale = toImageScale()
                        bridge.drag_translate_update(
                            Math.round((curX - startX) * scale),
                            Math.round((curY - startY) * scale))
                        bridge.drag_translate_end(true)
                        if (root.pasteFloating)
                            root.floatingOps += 1
                    }
                    return
                }
                dragging = false
                if (root.selW < 3 || root.selH < 3) {
                    // Click on empty canvas: clear the selection and any
                    // layer selection with it.
                    root.selW = 0
                    root.selH = 0
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

            function imgX(mx) { return (mx - view.pad) * toImageScale() }
            function imgY(my) { return (my - view.pad) * toImageScale() }

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
                const tol = 4 / Math.max(toDispScale(), 0.0001)
                for (var i = root.markList.length - 1; i >= 0; i--) {
                    const m = root.markList[i]
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
                const b = selectedMarkBox()
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
                const m = findMark(bridge.selected_mark)
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
                if (root.markTool !== "none") {
                    mode = 1
                    const p = clampCanvas(mouse.x, mouse.y)
                    startX = p[0]
                    startY = p[1]
                    curX = p[0]
                    curY = p[1]
                    brushPts = []
                    if (root.markTool === "brush")
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
                const m = findMark(hit)
                if (m && m.k === 5) {
                    bridge.select_mark(hit)
                    textPopup.openEdit(m)
                }
            }

            onPositionChanged: (mouse) => {
                if (root.rulerOn)
                    root.noteHover(mouse.x, mouse.y)
                if (mode === 1) {
                    const p = clampCanvas(mouse.x, mouse.y)
                    curX = p[0]
                    curY = p[1]
                    if (root.markTool === "brush") {
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
                    const s = toImageScale()
                    gestureRect = Qt.rect(
                        startMark.x + (mouse.x - startX) * s,
                        startMark.y + (mouse.y - startY) * s,
                        startMark.width, startMark.height)
                } else if (mode === 3) {
                    gestureRect = resizeTarget(
                        resizeRole, startMark,
                        (mouse.x - startX) * toImageScale(),
                        (mouse.y - startY) * toImageScale())
                }
            }

            onReleased: (mouse) => {
                if (mode === 1) {
                    const s = toImageScale()
                    // 用跟踪的 curX/curY（已钳制到图片范围）而不是
                    // mouse.x/y：提交的 rect 与预览完全一致。
                    const x0 = (startX - view.pad) * s
                    const y0 = (startY - view.pad) * s
                    const x1 = (curX - view.pad) * s
                    const y1 = (curY - view.pad) * s
                    if (root.markTool === "brush") {
                        if (brushPts.length >= 2) {
                            bridge.add_brush_mark(
                                brushPts.map(p => p[0].toFixed(1) + "," + p[1].toFixed(1)).join(" "),
                                root.markStroke, root.markColor)
                        }
                    } else if (root.markTool === "mosaic") {
                        // 选区模式：整块矩形打码（core 的 region 分支）。
                        if (Math.abs(x1 - x0) >= 1 && Math.abs(y1 - y0) >= 1)
                            bridge.add_mosaic_rect(x0, y0, x1, y1, root.markMosaic)
                    } else if (Math.abs(x1 - x0) >= 1 || Math.abs(y1 - y0) >= 1) {
                        const k = root.markTool === "rect" ? 0
                                  : root.markTool === "ellipse" ? 1 : 2
                        bridge.add_shape_mark(k, x0, y0, x1, y1,
                                              root.markStroke, root.markColor,
                                              root.markFill, root.markFillColor)
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
                const s = toDispScale()
                ctx.lineWidth = Math.max(1, root.markStroke * s)
                ctx.strokeStyle = root.markColor
                ctx.fillStyle = root.markFillColor
                ctx.lineCap = "round"
                ctx.lineJoin = "round"
                const px = v => view.pad + v * s
                const x0 = markMa.startX
                const y0 = markMa.startY
                const x1 = markMa.curX
                const y1 = markMa.curY
                if (root.markTool === "mosaic") {
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
                if (root.markTool === "brush") {
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
                if (root.markTool === "rect") {
                    if (root.markFill) {
                        ctx.fillRect(x0, y0, x1 - x0, y1 - y0)
                    }
                    ctx.strokeRect(x0, y0, x1 - x0, y1 - y0)
                    return
                }
                if (root.markTool === "ellipse") {
                    // core 的椭圆描边画在边界内侧（ring），预览保持一致：
                    // 半径先内收半个线宽。
                    const shrink = Math.max(1, root.markStroke * s) / 2
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
                    if (root.markFill)
                        ctx.fill()
                    ctx.stroke()
                    return
                }
                if (root.markTool === "arrow") {
                    const dx = x1 - x0
                    const dy = y1 - y0
                    const len = Math.sqrt(dx * dx + dy * dy)
                    if (len < 2)
                        return
                    const ux = dx / len
                    const uy = dy / len
                    const head = Math.max(root.markStroke * s * 3.5, 10)
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
                    ctx.fillStyle = root.markColor
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

            property var box: selectedMarkBox()
            onBoxChanged: if (visible) requestPaint()
        }

        Repeater {
            model: bridge.selected_mark >= 0
                   && markMa.mode === 0 ? 8 : 0

            delegate: Rectangle {
                required property int index
                readonly property var roles: ["nw", "n", "ne", "e", "se", "s", "sw", "w"]
                readonly property string role: roles[index]
                readonly property var box: selectedMarkBox()
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

            font.family: editMark ? qmlFontFamily(editMark.fn) : "Microsoft YaHei"
            font.pixelSize: editMark ? Math.max(8, editMark.fs * toDispScale()) : 20
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
                const s = toDispScale()
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
                const scale = toImageScale()
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
                if (root.scaleW <= 0)
                    return
                const s = toDispScale()
                const w = Math.max(1, Math.round(root.scaleW / s))
                const h = Math.max(1, Math.round(root.scaleH / s))
                const x = Math.round(root.scaleX / s)
                const y = Math.round(root.scaleY / s)
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
                const s = toDispScale()
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
                                       : canvasCursor(point.position.x,
                                                      point.position.y)
            // 尺子读数：handler 是唯一贯穿整块画布的 hover 来源（各个
            // MouseArea 谁在上面不确定），这里补一路坐标。
            onPointChanged: if (root.rulerOn)
                                root.noteHover(point.position.x, point.position.y)
            onHoveredChanged: if (!hovered) root.clearHover()
        }
    }

    // Selection outline: dashed frame + W×H pixel readout + blue dots.
    DashedRect {
        id: selRectViz
        visible: root.selExists
        z: 7
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        rx: root.selX
        ry: root.selY
        rw: root.selW
        rh: root.selH
        dashColor: "#000000"
    }

    // Selection size badge: W×H only, in a small gray rounded chip so it
    // stays readable over any image content.
    Rectangle {
        id: selBadge
        visible: root.selExists && !root.rulerOn
        z: 8
        x: previewImage.x
           + Math.max(0, Math.min(root.selX,
               previewImage.width - selBadge.width))
        y: previewImage.y
           + Math.min(previewImage.height - selBadge.height,
                      root.selY + root.selH + 4)
        width: selBadgeLabel.implicitWidth + 12
        height: selBadgeLabel.implicitHeight + 6
        radius: 3
        color: "#43474d"
        border.color: "#6b7076"
        border.width: 1

        Text {
            id: selBadgeLabel
            anchors.centerIn: parent
            text: Math.round(root.selW / toDispScale()) + " × "
                  + Math.round(root.selH / toDispScale())
            color: "#d5d8dd"
            font.pixelSize: 11
        }
    }

    EdgeHandles {
        id: selHandles
        // 拾色器打开时隐藏：画布点击必须优先给取色，不能被选区手柄吞掉。
        visible: root.selExists && !root.selRubberBanding && bridge.image_width > 0
                 && !bridge.pickerOpen
        z: 8
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        bx: root.selX
        by: root.selY
        bw: root.selW
        bh: root.selH
        pxScale: toDispScale()
        resizeFn: function(startRect, role, dx, dy, free) {
            resizeBoxFrom(startRect, role, dx, dy)
        }
        moveFn: function(startRect, dx, dy) {
            moveSelFrom(startRect, dx, dy)
        }
    }

    // Selected layer outline (dashed) + blue-dot handles for scaling.
    DashedRect {
        id: layerBoxViz
        visible: root.selLayer !== null && root.scaleW === 0
        z: 6
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        rx: root.selLayerBox !== null ? root.selLayerBox.x * toDispScale() : 0
        ry: root.selLayerBox !== null ? root.selLayerBox.y * toDispScale() : 0
        rw: root.selLayerBox !== null ? root.selLayerBox.w * toDispScale() : 0
        rh: root.selLayerBox !== null ? root.selLayerBox.h * toDispScale() : 0
        dashColor: "#5b9dff99"
    }

    EdgeHandles {
        id: layerHandles
        // 拾色器打开时隐藏（同 selHandles：取色优先于缩放/移动）。
        visible: root.selLayer !== null && root.selLayer.base !== true
                 && !root.selExists && bridge.image_width > 0
                 && !bridge.pickerOpen
        z: 8
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        bx: root.scaleW > 0 ? root.scaleX
                            : (root.selLayerBox !== null ? root.selLayerBox.x * toDispScale() : 0)
        by: root.scaleW > 0 ? root.scaleY
                            : (root.selLayerBox !== null ? root.selLayerBox.y * toDispScale() : 0)
        bw: root.scaleW > 0 ? root.scaleW
                            : (root.selLayerBox !== null ? root.selLayerBox.w * toDispScale() : 0)
        bh: root.scaleW > 0 ? root.scaleH
                            : (root.selLayerBox !== null ? root.selLayerBox.h * toDispScale() : 0)
        moveEnabled: false
        pxScale: toDispScale()
        resizeFn: function(startRect, role, dx, dy, free) {
            resizeLayerFrom(startRect, role, dx, dy, free)
        }
        onEngagedChanged: {
            if (layerHandles.engaged) {
                if (root.scaleW === 0)
                    bridge.scale_layer_begin()
            } else if (root.scaleW > 0) {
                commitLayerScale()
            }
        }
    }

    Text {
        visible: root.scaleW > 0
        z: 8
        x: previewImage.x + root.scaleX
        y: Math.min(previewImage.y + previewImage.height - 16,
                    previewImage.y + root.scaleY + root.scaleH + 4)
        text: Math.round(root.scaleW / toDispScale()) + " × "
              + Math.round(root.scaleH / toDispScale()) + "　松手应用缩放 · Esc 取消"
        color: "#7fc3ff"
        font.pixelSize: 11
    }

    // ---- 尺子覆盖层（Ruler）-------------------------------------------
    // 选中图层 / 选区：四角 + 中心显示「相对画布」的绝对坐标；鼠标位置
    // 跟随显示当前坐标。Y 轴方向由设置决定（向下 / 向上），纯显示层。
    Item {
        id: rulerOverlay
        visible: root.rulerOn && bridge.image_width > 0
        z: 9
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height

        // 每个测量点的十字准星。
        Repeater {
            model: root.rulerPoints
            delegate: Item {
                required property var modelData
                width: 13
                height: 13
                x: modelData.px - width / 2
                y: modelData.py - height / 2
                Rectangle { anchors.centerIn: parent; width: 13; height: 1; color: "#f0b23a" }
                Rectangle { anchors.centerIn: parent; width: 1; height: 13; color: "#f0b23a" }
            }
        }

        // 每个测量点的坐标读数：角点朝框外排布，中心点居中。
        Repeater {
            model: root.rulerPoints
            delegate: Rectangle {
                id: rulerChip
                required property var modelData
                required property int index
                readonly property bool isCenter: index === 4
                readonly property bool onRight: index === 1 || index === 3
                readonly property bool onBottom: index === 2 || index === 3
                width: rulerChipText.implicitWidth + 10
                height: 16
                radius: 3
                color: "#d91d2126"
                border.color: "#99f0b23a"
                border.width: 1
                x: Math.max(0, Math.min(rulerOverlay.width - width,
                    isCenter ? modelData.px - width / 2
                             : (onRight ? modelData.px + 5 : modelData.px - width - 5)))
                y: Math.max(0, Math.min(rulerOverlay.height - height,
                    isCenter ? modelData.py - height / 2
                             : (onBottom ? modelData.py + 5 : modelData.py - height - 5)))
                Text {
                    id: rulerChipText
                    anchors.centerIn: parent
                    text: rulerChip.modelData.label
                    color: "#ffcf7d"
                    font.pixelSize: 10
                    font.family: "Consolas"
                }
            }
        }

        // 选区尺寸：贴在框的下边中点，和坐标一起读（W × H）。
        Rectangle {
            id: rulerSizeChip
            visible: root.rulerBox !== null
            width: rulerSizeText.implicitWidth + 10
            height: 16
            radius: 3
            color: "#d91d2126"
            border.color: "#99f0b23a"
            border.width: 1
            x: Math.max(0, Math.min(rulerOverlay.width - width,
                                    (root.rulerBox !== null
                                     ? (root.rulerBox.x + root.rulerBox.w / 2) * toDispScale()
                                     : 0) - width / 2))
            y: Math.max(0, Math.min(rulerOverlay.height - height,
                                    (root.rulerBox !== null
                                     ? (root.rulerBox.y + root.rulerBox.h) * toDispScale()
                                     : 0) + 6))
            Text {
                id: rulerSizeText
                anchors.centerIn: parent
                text: root.rulerBox !== null
                      ? Math.round(root.rulerBox.w) + " × " + Math.round(root.rulerBox.h)
                      : ""
                color: "#ffcf7d"
                font.pixelSize: 10
                font.family: "Consolas"
            }
        }

        // 鼠标位置：十字线 + 坐标读数。
        Item {
            visible: root.hoverValid
            x: root.hoverFx * toDispScale()
            y: root.hoverFy * toDispScale()
            Rectangle { x: -6; y: 0; width: 13; height: 1; color: "#4ad2ff" }
            Rectangle { x: 0; y: -6; width: 1; height: 13; color: "#4ad2ff" }
        }
        Rectangle {
            id: rulerHoverChip
            visible: root.hoverValid
            width: rulerHoverText.implicitWidth + 10
            height: 16
            radius: 3
            color: "#e61d2126"
            border.color: "#994ad2ff"
            border.width: 1
            x: Math.max(0, Math.min(rulerOverlay.width - width,
                                    root.hoverFx * toDispScale() + 11))
            y: Math.max(0, Math.min(rulerOverlay.height - height,
                                    root.hoverFy * toDispScale() + 11))
            Text {
                id: rulerHoverText
                anchors.centerIn: parent
                text: {
                    const ix = Math.floor(root.hoverFx)
                    const iy = Math.floor(root.hoverFy)
                    return ix + ", "
                           + (bridge.rulerBottomUp ? bridge.image_height - iy : iy)
                }
                color: "#9fe8ff"
                font.pixelSize: 10
                font.family: "Consolas"
            }
        }
    }

    Text {
        anchors.centerIn: parent
        visible: bridge.preview_url.length === 0
        text: "把图片拖到这里，或在命令行传入文件路径"
        color: "#6b7076"
        font.pixelSize: 14
    }
}

// Viewport-fixed pan/zoom input (sits above the Flickable so it never
// scrolls with content). Wheel/touchpad = pan, Ctrl+wheel = zoom,
// Shift+wheel = horizontal pan. Middle-drag pans. Left-button events are
// NOT accepted here and pass through to the canvas gesture stack, so a
// normal click-drag can never pan the view.
MouseArea {
    id: panMa
    anchors.top: toolbar.bottom
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
    return previewImage.width > 0 ? bridge.image_width / previewImage.width : 1
}

function toDispScale() {
    return bridge.image_width > 0 && previewImage.width > 0
           ? previewImage.width / bridge.image_width : 0
}

// 选区存的是显示坐标：缩放（滚轮 / 适应窗口 / 拖窗口）改变显示比例后
// 它会漂离原画面位置。按新旧比例把选区重映射，让包围盒始终钉住同一块
// 内容。图层框走图像坐标 × toDispScale() 实时换算，天生跟随，无需处理。
property real selTrackScale: 0
property int selTrackImgW: 0
function remapSelectionToScale() {
    const iw = bridge.image_width
    if (iw <= 0 || previewImage.width <= 0) {
        selTrackScale = 0
        selTrackImgW = 0
        return
    }
    const s = previewImage.width / iw
    // 图像尺寸没变且之前记过基准 → 纯比例变化，重映射。
    // 图像尺寸变了（裁剪/调整大小/新文档）→ 语义已变，只重置基准。
    if (selTrackImgW === iw && selTrackScale > 0 && root.selExists
            && !root.selRubberBanding) {
        const k = s / selTrackScale
        if (k !== 1) {
            root.selX *= k
            root.selY *= k
            root.selW *= k
            root.selH *= k
        }
    }
    selTrackScale = s
    selTrackImgW = iw
}

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
function canvasCursor(x, y) {
    // 文本输入中：普通箭头。
    if (textPopup.visible)
        return Qt.ArrowCursor
    // 拾色器打开：整块画布都是取色十字。
    if (bridge.pickerOpen)
        return Qt.CrossCursor
    // 标记工具激活：手柄 → 缩放光标；标记上 → 移动；文本工具 → 工字；
    // 其余 → 十字。
    if (root.markTool !== "none") {
        const h = markMa.handleAt(x, y)
        if (h !== "")
            return markMa.handleCursor(h)
        if (markMa.markAt(x, y) >= 0)
            return Qt.SizeAllCursor
        return Qt.CrossCursor
    }
    // 未选绘制工具时：悬停标记仍可移动/编辑（标记不依赖工具栏）。
    if (markMa.markAt(x, y) >= 0)
        return Qt.SizeAllCursor
    const ix = x - view.pad
    const iy = y - view.pad
    if (root.selExists) {
        const c = boxCursor(ix, iy, root.selX, root.selY,
                            root.selW, root.selH)
        if (c >= 0)
            return c
    }
    if (layerHandles.visible) {
        const c = boxCursor(ix, iy, layerHandles.bx, layerHandles.by,
                            layerHandles.bw, layerHandles.bh)
        if (c >= 0)
            return c
    }
    if (!root.selExists && bridge.image_width > 0) {
        const hit = topLayerAt(x, y)
        if (hit !== null && !isLocked(hit))
            return Qt.SizeAllCursor
    }
    return Qt.CrossCursor
}

// Cursor for hovering a box's border / interior (display px).
// Returns the matching Qt cursor, or -1 when the point is outside the
// grab zone. Border tolerance: corners beat edges, edges beat interior.
function boxCursor(x, y, bx, by, bw, bh) {
    const tol = 7
    if (x < bx - tol || x > bx + bw + tol
            || y < by - tol || y > by + bh + tol)
        return -1
    const nearL = Math.abs(x - bx) <= tol
    const nearR = Math.abs(x - (bx + bw)) <= tol
    const nearT = Math.abs(y - by) <= tol
    const nearB = Math.abs(y - (by + bh)) <= tol
    if ((nearL || nearR) && (nearT || nearB))
        return (nearL === nearT) ? Qt.SizeFDiagCursor
                                 : Qt.SizeBDiagCursor
    if (nearL || nearR)
        return Qt.SizeHorCursor
    if (nearT || nearB)
        return Qt.SizeVerCursor
    if (x > bx && x < bx + bw && y > by && y < by + bh)
        return Qt.SizeAllCursor
    return -1
}

function boxSet(x, y, w, h) {
    root.selX = x; root.selY = y; root.selW = w; root.selH = h
}

function resizeBoxFrom(startRect, role, dx, dy) {
    const s = Math.max(toDispScale(), 0.001)
    let x = startRect.x, y = startRect.y, w = startRect.width, h = startRect.height
    if (role.indexOf("w") >= 0) {
        let nx = snapDisplay(startRect.x + dx)
        nx = Math.max(0, Math.min(nx, startRect.x + startRect.width - s))
        w = startRect.x + startRect.width - nx
        x = nx
    }
    if (role.indexOf("e") >= 0) {
        w = Math.min(Math.max(s, snapDisplay(startRect.width + dx)),
                     previewImage.width - x)
    }
    if (role.indexOf("n") >= 0) {
        let ny = snapDisplay(startRect.y + dy)
        ny = Math.max(0, Math.min(ny, startRect.y + startRect.height - s))
        h = startRect.y + startRect.height - ny
        y = ny
    }
    if (role.indexOf("s") >= 0) {
        h = Math.min(Math.max(s, snapDisplay(startRect.height + dy)),
                     previewImage.height - y)
    }
    boxSet(x, y, w, h)
}

function moveSelFrom(startRect, dx, dy) {
    let x = snapDisplay(startRect.x + dx)
    let y = snapDisplay(startRect.y + dy)
    x = Math.max(0, Math.min(x, previewImage.width - startRect.width))
    y = Math.max(0, Math.min(y, previewImage.height - startRect.height))
    boxSet(x, y, startRect.width, startRect.height)
}

// Live scale-target box for the selected layer; committed on release.
function resizeLayerFrom(startRect, role, dx, dy, free) {
    const s = Math.max(toDispScale(), 0.001)
    let x = startRect.x, y = startRect.y, w = startRect.width, h = startRect.height
    if (role.indexOf("w") >= 0) {
        let nx = snapDisplay(startRect.x + dx)
        nx = Math.max(0, Math.min(nx, startRect.x + startRect.width - s))
        w = startRect.x + startRect.width - nx
        x = nx
    }
    if (role.indexOf("e") >= 0) {
        w = Math.min(Math.max(s, snapDisplay(startRect.width + dx)),
                     previewImage.width - x)
    }
    if (role.indexOf("n") >= 0) {
        let ny = snapDisplay(startRect.y + dy)
        ny = Math.max(0, Math.min(ny, startRect.y + startRect.height - s))
        h = startRect.y + startRect.height - ny
        y = ny
    }
    if (role.indexOf("s") >= 0) {
        h = Math.min(Math.max(s, snapDisplay(startRect.height + dy)),
                     previewImage.height - y)
    }
    if (role.length === 2 && !free) {
        // Corner handles keep the layer's aspect ratio by default
        // (D5-A); hold Shift for a free resize.
        const ar = startRect.width / Math.max(1, startRect.height)
        if (w / Math.max(1, h) > ar)
            w = h * ar
        else
            h = w / ar
        if (role.indexOf("w") >= 0)
            x = startRect.x + startRect.width - w
        if (role.indexOf("n") >= 0)
            y = startRect.y + startRect.height - h
    }
    root.scaleX = x; root.scaleY = y
    root.scaleW = Math.max(1, w); root.scaleH = Math.max(1, h)
    if (!scaleTimer.running) {
        scaleTimer.lastW = -1; scaleTimer.lastH = -1
        scaleTimer.lastX = 0; scaleTimer.lastY = 0
        scaleTimer.start()
    }
}

function commitLayerScale() {
    scaleTimer.stop()
    const s = toDispScale()
    if (root.scaleW >= 1 && root.scaleH >= 1 && s > 0) {
        bridge.scale_layer_end(
            true,
            Math.max(1, Math.round(root.scaleW / s)),
            Math.max(1, Math.round(root.scaleH / s)),
            Math.round(root.scaleX / s), Math.round(root.scaleY / s))
    }
    root.scaleX = 0; root.scaleY = 0; root.scaleW = 0; root.scaleH = 0
}

function cancelLayerScale() {
    scaleTimer.stop()
    if (root.scaleW > 0)
        bridge.scale_layer_end(false, 0, 0, 0, 0)
    root.scaleX = 0; root.scaleY = 0; root.scaleW = 0; root.scaleH = 0
}

function applyCrop() {
    if (!root.selExists)
        return
    const s = toDispScale()
    bridge.crop_to(Math.round(root.selX / s), Math.round(root.selY / s),
                   Math.max(1, Math.round(root.selW / s)),
                   Math.max(1, Math.round(root.selH / s)))
    root.selW = 0
    root.selH = 0
    cancelLayerScale()
}

function deleteSelectionPixels() {
    if (!root.selExists)
        return
    const s = toDispScale()
    bridge.erase_selection_pixels(
        Math.floor(root.selX / s), Math.floor(root.selY / s),
        Math.round(root.selW / s), Math.round(root.selH / s))
    root.selW = 0
    root.selH = 0
}

// Delete / Backspace: 显式选中的标记最优先（持久选区 + 选中标记时，
// Delete 的意图是删标记——选区像素删除先按一次 Esc 取消标记选中，
// 与 Esc 的取消层级一致）；然后是选区像素 → 选中图层 → 可删的背景。
function onDeleteKey() {
    // 就地编辑文本时 Delete 属于输入框，不删标记。
    if (bridge.selected_mark >= 0 && !textPopup.visible) {
        bridge.delete_selected_mark()
        return
    }
    if (root.selExists) {
        deleteSelectionPixels()
        return
    }
    if (bridge.selected_layer >= 0) {
        if (!isLocked(root.selLayer))
            bridge.delete_selected_layer()
        return
    }
    const base = findBase()
    // bd !== true: the base still exists (not "deleted"); visibility is
    // irrelevant for the delete flow.
    if (base !== null && base.bd !== true && !isLocked(base)
        && bridge.layer_count > 0)
        bridge.delete_base_layer()
}

// Ctrl+A: selection covering the whole canvas (Preview ⌘A).
function selectAllCanvas() {
    if (previewImage.width < 1 || previewImage.height < 1)
        return
    root.selX = 0
    root.selY = 0
    root.selW = previewImage.width
    root.selH = previewImage.height
    settlePaste()
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
        const s = toDispScale()
        bridge.copy_selection_pixels(
            Math.floor(root.selX / s), Math.floor(root.selY / s),
            Math.round(root.selW / s), Math.round(root.selH / s))
    } else if (bridge.selected_layer >= 0) {
        bridge.copy_selected_layer()
    }
}

function doCut() {
    if (root.selExists) {
        const s = toDispScale()
        bridge.cut_selection_pixels(
            Math.floor(root.selX / s), Math.floor(root.selY / s),
            Math.round(root.selW / s), Math.round(root.selH / s))
        root.selW = 0
        root.selH = 0
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
    root.selW = 0
    root.selH = 0
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
    if (textPopup.visible) { textPopup.close(false); return }
    // 调色面板：Esc = 放弃（像素从未被改动）。
    if (bridge.adjustOpen) { root.cancelAdjust(); return }
    // 拾色器：Esc = 关面板（已取的色保留）。
    if (bridge.pickerOpen) { bridge.picker_close(); return }
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
    if (psdHintOverlay.visible) { psdHintOverlay.visible = false; return }
    if (resizeOverlay.visible) { resizeOverlay.visible = false; return }
    if (settingsOverlay.visible) { settingsOverlay.visible = false; return }
    if (root.scaleW > 0) { cancelLayerScale(); return }
    if (selDragMouse.dragging) { selDragMouse.dragging = false; return }
    if (bridge.selected_mark >= 0) { bridge.select_mark(-1); return }
    if (root.pasteFloating) {
        // D3-A: roll back everything committed since the paste.
        for (var i = 0; i < root.floatingOps; i++)
            bridge.undo()
        settlePaste()
        return
    }
    if (root.selExists) { root.selW = 0; root.selH = 0; return }
    if (bridge.selected_layer >= 0) { bridge.select_layer(-1); return }
    if (root.panelVisible) { root.panelVisible = false; return }
    root.fullscreen = false
}

function onEnter() {
    // 模态确认框：回车 = 默认动作（保存并打开/切换/关闭）。
    if (confirmOverlay.visible) {
        confirmSaveProceed()
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
    const scale = toImageScale()
    bridge.extract_selection(
        Math.floor(root.selX * scale), Math.floor(root.selY * scale),
        Math.round(root.selW * scale), Math.round(root.selH * scale))
    root.selW = 0
    root.selH = 0
}

// 再点一次已选中的标记工具 = 取消（回到 none，不绘制只选择）。
function setMarkTool(tool) {
    root.markTool = (root.markTool === tool) ? "none" : tool
    if (root.markTool === "none") {
        widthPopover.visible = false
        mosaicPopover.visible = false
    }
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

function parsePicker(s) {
    if (!s)
        return null
    try {
        const v = JSON.parse(s)
        return v && v.hex !== undefined ? v : null
    } catch (e) {
        return null
    }
}

function parsePickerHistory(s) {
    if (!s)
        return []
    try {
        const v = JSON.parse(s)
        return Array.isArray(v) ? v : []
    } catch (e) {
        return []
    }
}

// Copy one color notation and flash "已复制" on its row.
function copyPickerValue(key, value) {
    if (!value)
        return
    copyToClipboard(value)
    root.pickerCopied = key
    pickerCopiedTimer.restart()
}


function parseLayers(s) {
    if (!s)
        return []
    try {
        return JSON.parse(s)
    } catch (e) {
        return []
    }
}

function parseMarks(s) {
    if (!s)
        return []
    try {
        return JSON.parse(s)
    } catch (e) {
        return []
    }
}

function findMark(index) {
    return root.markList.find(mk => mk.i === index) || null
}

// ---- 调整颜色：参数镜像 / 节流下发 ----
function parseAdjust(s) {
    const empty = { e: 0, c: 0, hi: 0, sh: 0, sa: 0, te: 0, ti: 0,
                    se: 0, sp: 0, bl: 0, ga: 1, wh: 1, mh: 0.5 }
    if (!s)
        return empty
    try {
        const v = JSON.parse(s)
        for (const k in empty)
            if (v[k] === undefined)
                v[k] = empty[k]
        return v
    } catch (e) {
        return empty
    }
}

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

function parseHistogram(s) {
    if (!s)
        return null
    try {
        const v = JSON.parse(s)
        return (v && v.r && v.g && v.b) ? v : null
    } catch (e) {
        return null
    }
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
function qmlFontFamily(id) {
    return id === "simsun" ? "SimSun"
         : id === "simhei" ? "SimHei"
         : id === "simkai" ? "KaiTi"
         : id === "arial" ? "Arial"
         : "Microsoft YaHei"
}

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

function findSelLayer() {
    for (var i = 0; i < layerList.length; i++)
        if (!layerList[i].base && layerList[i].i === bridge.selected_layer)
            return layerList[i]
    return null
}

function findLayerBox() {
    for (var i = 0; i < layerList.length; i++)
        if (!layerList[i].base && layerList[i].i === bridge.selected_layer)
            return layerList[i]
    return null
}

function isLocked(item) {
    return item !== null && item !== undefined && item.l === true
}

function findBase() {
    for (var i = 0; i < layerList.length; i++)
        if (layerList[i].base === true)
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
    if (bridge.openInNewWindow)
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

// Ctrl+S: overwrite the file the image was loaded from. A document with
// no source file (pasted image, new window) has nothing to overwrite, so
// fall back to the Save-As dialog.
//
// PSD documents are preview-only (layers were merged at open and there
// is no PSD encoder): block the direct save with an explanatory dialog
// that routes to Save-As instead.
function sourceIsPsd() {
    return String(bridge.fileName || "").toLowerCase().endsWith(".psd")
}

function saveOrSaveAs() {
    if (bridge.fileName.length > 0) {
        if (sourceIsPsd()) {
            psdHintOverlay.visible = true
            return
        }
        bridge.save_edited()
    } else
        saveDialog.open()
}

// Default (highlighted) action of the unsaved-changes dialog: save the
// edited image, then continue whatever the user originally asked for
// (open another file / navigate / close the window).
function confirmSaveProceed() {
    const mode = root.confirmMode
    // No source file: saving needs a destination first; let the user
    // pick one instead of discarding the edits silently.
    if (bridge.fileName.length === 0) {
        confirmOverlay.visible = false
        root.confirmMode = ""
        saveDialog.open()
        return
    }
    // PSD source: direct save is blocked, so the "save and proceed"
    // action degrades to Save-As (the dialog drives the continuation).
    if (sourceIsPsd()) {
        confirmOverlay.visible = false
        root.confirmMode = ""
        psdHintOverlay.visible = true
        return
    }
    bridge.save_edited()
    confirmOverlay.visible = false
    root.confirmMode = ""
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
    onTriggered: overflowMenu.visible = true
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
        markMa.startX = view.pad
        markMa.startY = view.pad
        markMa.curX = view.pad + previewImage.width * 0.3
        markMa.curY = view.pad + previewImage.height * 0.3
        markMa.mode = 1
        bridge.mark("probe phase1 requestPaint")
        markPreview.requestPaint()
        phase2Timer.start()
    }
}
Timer {
    id: phase2Timer
    interval: 2000
    onTriggered: {
        markMa.curX = view.pad + previewImage.width * 0.9
        markMa.curY = view.pad + previewImage.height * 0.9
        bridge.mark("probe phase2 requestPaint")
        markPreview.requestPaint()
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
            root.selX = previewImage.width * 0.18
            root.selY = previewImage.height * 0.22
            root.selW = previewImage.width * 0.38
            root.selH = previewImage.height * 0.34
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
    interval: 900
    running: root.probeSave && root.primary
    onTriggered: saveOrSaveAs()
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

Component.onCompleted: {
    bridge.mark("qml-completed");
    if (root.primary) {
        bridge.start();
        if (root.probeDialog)
            dialogProbeTimer.start();
        if (root.probeSettings)
            settingsProbeTimer.start();
        if (root.probeMenu)
            menuProbeTimer.start();
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
