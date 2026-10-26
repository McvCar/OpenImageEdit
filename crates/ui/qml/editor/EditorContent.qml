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
    readonly property bool selRubberBanding: view.canvasSurface.selDragMouse.dragging
    readonly property bool modeActive: selRubberBanding || pasteFloating
    // Scale target for the selected layer (display coords); empty when idle.
    property real scaleX: 0
    property real scaleY: 0
    property real scaleW: 0
    property real scaleH: 0
    property int renamingIndex: -1
    readonly property var layerList: Layers.parseLayers(bridge.layers_json)
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

    // Report the screen's device-pixel-ratio to the bridge so the preview
    // bitmap is rendered at the physical resolution (logical × dpr). Without
    // this a window at >100% Windows scaling gets a logically-sized bitmap
    // stretched by the GPU → the whole image is uniformly blurred. `window`
    // is this Item's top-level window (main Window or a DocWindow); its
    // screen's devicePixelRatio is the Windows scaling factor. The binding is
    // fully reactive: it fires on start, on scaling changes, and when the
    // window moves to a different-density monitor.
    property real dpr: window ? window.screen.devicePixelRatio : 1.0
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
           enabled: !view.canvasSurface.textPopup.visible && !fontPopover.sizeInput.activeFocus
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
    return view.previewImage.width > 0 ? bridge.image_width / view.previewImage.width : 1
}

function toDispScale() {
    return bridge.image_width > 0 && view.previewImage.width > 0
           ? view.previewImage.width / bridge.image_width : 0
}

// 选区存的是显示坐标：缩放（滚轮 / 适应窗口 / 拖窗口）改变显示比例后
// 它会漂离原画面位置。按新旧比例把选区重映射，让包围盒始终钉住同一块
// 内容。图层框走图像坐标 × toDispScale() 实时换算，天生跟随，无需处理。
property real selTrackScale: 0
property int selTrackImgW: 0
function remapSelectionToScale() {
    const iw = bridge.image_width
    if (iw <= 0 || view.previewImage.width <= 0) {
        selTrackScale = 0
        selTrackImgW = 0
        return
    }
    const s = view.previewImage.width / iw
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
    if (view.canvasSurface.textPopup.visible)
        return Qt.ArrowCursor
    // 拾色器打开：整块画布都是取色十字。
    if (bridge.pickerOpen)
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
    if (root.selExists) {
        const c = Geometry.boxCursor(ix, iy, root.selX, root.selY,
                            root.selW, root.selH)
        if (c >= 0)
            return c
    }
    if (view.selectionViz.layerHandles.visible) {
        const c = Geometry.boxCursor(ix, iy, view.selectionViz.layerHandles.bx, view.selectionViz.layerHandles.by,
                            view.selectionViz.layerHandles.bw, view.selectionViz.layerHandles.bh)
        if (c >= 0)
            return c
    }
    if (!root.selExists && bridge.image_width > 0) {
        const hit = topLayerAt(x, y)
        if (hit !== null && !Layers.isLocked(hit))
            return Qt.SizeAllCursor
    }
    return Qt.CrossCursor
}

// Cursor for hovering a box's border / interior (display px).
// Returns the matching Qt cursor, or -1 when the point is outside the
// grab zone. Border tolerance: corners beat edges, edges beat interior.
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
                     view.previewImage.width - x)
    }
    if (role.indexOf("n") >= 0) {
        let ny = snapDisplay(startRect.y + dy)
        ny = Math.max(0, Math.min(ny, startRect.y + startRect.height - s))
        h = startRect.y + startRect.height - ny
        y = ny
    }
    if (role.indexOf("s") >= 0) {
        h = Math.min(Math.max(s, snapDisplay(startRect.height + dy)),
                     view.previewImage.height - y)
    }
    boxSet(x, y, w, h)
}

function moveSelFrom(startRect, dx, dy) {
    let x = snapDisplay(startRect.x + dx)
    let y = snapDisplay(startRect.y + dy)
    x = Math.max(0, Math.min(x, view.previewImage.width - startRect.width))
    y = Math.max(0, Math.min(y, view.previewImage.height - startRect.height))
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
                     view.previewImage.width - x)
    }
    if (role.indexOf("n") >= 0) {
        let ny = snapDisplay(startRect.y + dy)
        ny = Math.max(0, Math.min(ny, startRect.y + startRect.height - s))
        h = startRect.y + startRect.height - ny
        y = ny
    }
    if (role.indexOf("s") >= 0) {
        h = Math.min(Math.max(s, snapDisplay(startRect.height + dy)),
                     view.previewImage.height - y)
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
        bridge.scale_layer_end(
            true,
            Math.max(1, Math.round(root.scaleW / s)),
            Math.max(1, Math.round(root.scaleH / s)),
            Math.round(root.scaleX / s), Math.round(root.scaleY / s))
    }
    root.scaleX = 0; root.scaleY = 0; root.scaleW = 0; root.scaleH = 0
}

function cancelLayerScale() {
    view.canvasSurface.scaleTimer.stop()
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
    if (view.previewImage.width < 1 || view.previewImage.height < 1)
        return
    root.selX = 0
    root.selY = 0
    root.selW = view.previewImage.width
    root.selH = view.previewImage.height
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
    if (view.canvasSurface.textPopup.visible) { view.canvasSurface.textPopup.close(false); return }
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
    if (view.canvasSurface.selDragMouse.dragging) { view.canvasSurface.selDragMouse.dragging = false; return }
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
            root.selX = view.previewImage.width * 0.18
            root.selY = view.previewImage.height * 0.22
            root.selW = view.previewImage.width * 0.38
            root.selH = view.previewImage.height * 0.34
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
