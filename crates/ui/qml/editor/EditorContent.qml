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
            placePopover(adjustPanel, adjustBtn)
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
           enabled: !textPopup.visible && !sizeInput.activeFocus
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
Rectangle {
    id: toolbar
    anchors.top: parent.top
    anchors.left: parent.left
    anchors.right: parent.right
    height: root.toolbarHeight
    color: "#202225"
    // Above the canvas view and layer panel (z 50), so the hover
    // tooltips that hang below the bar are not painted over.
    z: 60

    // Icon actions shown in the bar. Whatever does not fit is hidden;
    // the "…" menu always offers every command as text.
    readonly property var barActions: [
        { g: "↶", t: "撤销 (Ctrl+Z)", ok: bridge.can_undo,
          run: function () { bridge.undo() } },
        { g: "↷", t: "重做 (Ctrl+Shift+Z)", ok: bridge.can_redo,
          run: function () { bridge.redo() } },
        { g: "", t: "", ok: true, sep: true },
        { g: "↺", t: "逆时针旋转 90° (Ctrl+L)", ok: true,
          run: function () { bridge.rotate_ccw() } },
        { g: "⇋", t: "水平翻转", ok: true,
          run: function () { bridge.flip_horizontal() } },
        { g: "⇅", t: "垂直翻转", ok: true,
          run: function () { bridge.flip_vertical() } },
        { g: "", t: "", ok: true, sep: true },
        { g: "⛶", t: "应用裁剪 (Enter / Ctrl+K)", ok: root.selExists,
          run: function () { applyCrop() } },
        { g: "⤢", t: "调整尺寸…", ok: bridge.image_width > 0,
          run: function () { resizeOverlay.openResizeDialog() } },
        { g: "", t: "", ok: true, sep: true },
        { g: "＋", t: "放大 (Ctrl+滚轮)", ok: true,
          run: function () { view.queueZoom(1.25) } },
        { g: "－", t: "缩小 (Ctrl+滚轮)", ok: true,
          run: function () { view.queueZoom(1 / 1.25) } }
    ]

    // Everything the bar shows as icons, plus the full text-only set.
    readonly property var menuActions: [
        { l: "打开图片… (Ctrl+O)", ok: true, run: function () { openDialog.open() } },
        { l: "新窗口 (Ctrl+N)", ok: true, run: function () { root.requestNewWindow() } },
        { l: "", ok: true, sep: true },
        { l: "撤销 (Ctrl+Z)", ok: bridge.can_undo, run: function () { bridge.undo() } },
        { l: "重做 (Ctrl+Shift+Z)", ok: bridge.can_redo, run: function () { bridge.redo() } },
        { l: "", ok: true, sep: true },
        { l: "逆时针旋转 90° (Ctrl+L)", ok: true, run: function () { bridge.rotate_ccw() } },
        { l: "顺时针旋转 90° (Ctrl+R)", ok: true, run: function () { bridge.rotate_cw() } },
        { l: "水平翻转", ok: true, run: function () { bridge.flip_horizontal() } },
        { l: "垂直翻转", ok: true, run: function () { bridge.flip_vertical() } },
        { l: "", ok: true, sep: true },
        { l: "应用裁剪 (Enter / Ctrl+K)", ok: root.selExists, run: function () { applyCrop() } },
        { l: "重置裁剪", ok: bridge.image_width > 0, run: function () { bridge.reset_crop() } },
        { l: "调整尺寸…", ok: bridge.image_width > 0, run: function () { resizeOverlay.openResizeDialog() } },
        { l: "", ok: true, sep: true },
        { l: "提取图层", ok: root.selExists, run: function () { commitExtract() } },
        { l: "删除选区像素 (Delete)", ok: root.selExists, run: function () { deleteSelectionPixels() } },
        { l: "图层面板", ok: true, run: function () { root.panelVisible = !root.panelVisible } },
        { l: "标记工具栏", ok: true, run: function () { markBarBtn.clicked() } },
        { l: "调整颜色… (Ctrl+E)", ok: bridge.image_width > 0,
          run: function () { root.toggleAdjustPanel() } },
        { l: "尺子（测量坐标）", ok: bridge.image_width > 0,
          run: function () { root.toggleRuler() } },
        { l: "拾色器…", ok: bridge.image_width > 0,
          run: function () { root.togglePickerPanel() } },
        { l: "", ok: true, sep: true },
        { l: "放大 (Ctrl++)", ok: true, run: function () { view.queueZoom(1.25) } },
        { l: "缩小 (Ctrl+-)", ok: true, run: function () { view.queueZoom(1 / 1.25) } },
        { l: "适应窗口 (Ctrl+9)", ok: bridge.image_width > 0, run: function () { bridge.zoom_fit() } },
        { l: "100% 实际大小 (Ctrl+0)", ok: bridge.image_width > 0, run: function () { bridge.zoom_actual() } },
        { l: "", ok: true, sep: true },
        { l: "保存 (Ctrl+S)", ok: bridge.image_width > 0, run: function () { saveOrSaveAs() } },
        { l: "另存为… (Ctrl+Shift+S)", ok: true, run: function () { saveDialog.open() } },
        { l: "", ok: true, sep: true },
        { l: "设置…", ok: true, run: function () { settingsOverlay.visible = true } },
        { l: "", ok: true, sep: true },
        { l: "关闭窗口 (Ctrl+W)", ok: true, run: function () { requestQuit() } }
    ]

    // How many icon slots fit between the title and the chrome.
    property int capacity: barActions.length
    function updateCapacity() {
        // 30px icon + 2px spacing each; reserve title, layers button,
        // "…", window buttons and margins.
        const chrome = titleText.implicitWidth + 8 + 16 + menuBtn.width
                       + layersBtn.width + markBarBtn.width + 8
                       + winRow.width + 24
        const avail = Math.max(0, toolbar.width - chrome)
        const per = 32
        capacity = Math.min(barActions.length, Math.floor(avail / per))
    }
    Component.onCompleted: updateCapacity()
    onWidthChanged: updateCapacity()

    // Drag-to-move (press anywhere on the bar that is not a button);
    // double-click toggles maximize. Move starts only after a small
    // threshold so double-click still registers.
    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
        property real pressX: 0
        property real pressY: 0
        onPressed: (mouse) => {
            pressX = mouse.x
            pressY = mouse.y
        }
        onPositionChanged: (mouse) => {
            if (!pressed)
                return
            if (Math.abs(mouse.x - pressX) + Math.abs(mouse.y - pressY) < 6)
                return
            const w = Window.window
            if (w && w.startSystemMove)
                w.startSystemMove()
        }
        onDoubleClicked: (mouse) => {
            const w = Window.window
            if (!w)
                return
            if (w.visibility === Window.Maximized)
                w.showNormal()
            else if (w.visibility === Window.Windowed)
                w.showMaximized()
        }
    }

    Text {
        id: titleText
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        width: Math.min(implicitWidth, 220)
        elide: Text.ElideRight
        // Same content as the (offscreen) window title: unsaved changes
        // are flagged with a trailing asterisk.
        text: b.image_width > 0
              ? "ImageEdit — " + b.image_width + "×" + b.image_height
                + (b.dirty ? " *" : "")
              : "ImageEdit"
        color: "#c9cdd2"
        font.pixelSize: 12
        onImplicitWidthChanged: toolbar.updateCapacity()
    }

    Row {
        id: iconRow
        anchors.left: titleText.right
        anchors.leftMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        Repeater {
            model: toolbar.barActions

            delegate: IconButton {
                required property var modelData
                required property int index
                // Read ok/sel through the model (not the captured
                // modelData) so enabled state stays reactive to
                // bridge.can_undo / panelVisible changes.
                glyph: modelData.g
                tip: modelData.t
                separator: modelData.sep === true
                active: modelData.sep === true
                        || toolbar.barActions[index].ok === true
                selected: modelData.sel === true
                          || (toolbar.barActions[index].sel === true)
                visible: modelData.sep === true || index < toolbar.capacity
                onClicked: modelData.run()
            }
        }
    }

    IconButton {
        id: markBarBtn
        anchors.right: layersBtn.left
        anchors.rightMargin: 2
        anchors.verticalCenter: parent.verticalCenter
        glyph: "✎"
        tip: "标记工具栏"
        selected: root.markBarVisible
        onClicked: {
            root.markBarVisible = !root.markBarVisible
            if (!root.markBarVisible) {
                root.markTool = "none"
                // 标记选中态保留：标记与图层统一交互，不依赖工具栏。
                widthPopover.visible = false
                colorPopover.visible = false
                mosaicPopover.visible = false
                fontPopover.visible = false
                textPopup.close(false)
                // 调色面板是从这条工具栏打开的：关栏 = 应用并关闭。
                if (bridge.adjustOpen)
                    root.commitAdjust()
            }
        }
    }

    IconButton {
        id: layersBtn
        anchors.right: menuBtn.left
        anchors.rightMargin: 2
        anchors.verticalCenter: parent.verticalCenter
        glyph: "☰"
        tip: "图层面板"
        selected: root.panelVisible
        visible: true
        onClicked: root.panelVisible = !root.panelVisible
    }

    IconButton {
        id: menuBtn
        anchors.right: winRow.left
        anchors.rightMargin: 6
        anchors.verticalCenter: parent.verticalCenter
        glyph: "⋯"
        tip: "更多操作"
        visible: true
        onClicked: overflowMenu.visible = !overflowMenu.visible
    }

    Row {
        id: winRow
        anchors.right: parent.right
        anchors.rightMargin: 4
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        IconButton {
            glyph: "─"
            tip: "最小化"
            onClicked: { const w = Window.window; if (w) w.showMinimized() }
        }
        IconButton {
            glyph: "▢"
            tip: "最大化 / 还原"
            onClicked: {
                const w = Window.window
                if (!w) return
                if (w.visibility === Window.Maximized) w.showNormal()
                else w.showMaximized()
            }
        }
        IconButton {
            glyph: "✕"
            tip: "关闭"
            onClicked: { const w = Window.window; if (w) w.close() }
        }
    }
}

// ---- 标记工具栏：主工具栏下方的第二行（开关 = 标题栏 ✎ 按钮）------
Rectangle {
    id: markBar
    visible: root.markBarVisible
    anchors.top: toolbar.bottom
    anchors.left: parent.left
    anchors.right: parent.right
    height: root.toolbarHeight
    color: "#25272b"
    z: 59

    Row {
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        IconButton {
            glyph: "▭"
            tip: "矩形标记（再点一次取消）"
            selected: root.markTool === "rect"
            onClicked: root.setMarkTool("rect")
        }
        IconButton {
            glyph: "◯"
            tip: "椭圆标记（再点一次取消）"
            selected: root.markTool === "ellipse"
            onClicked: root.setMarkTool("ellipse")
        }
        IconButton {
            glyph: "↗"
            tip: "箭头标记（再点一次取消）"
            selected: root.markTool === "arrow"
            onClicked: root.setMarkTool("arrow")
        }
        IconButton {
            glyph: "✏"
            tip: "自由画笔（再点一次取消）"
            selected: root.markTool === "brush"
            onClicked: root.setMarkTool("brush")
        }
        IconButton {
            glyph: "T"
            tip: "添加文本标记（双击文本可编辑）"
            // 普通动作按钮：不保持选中态，每次点击在图片中央放一个
            // 默认文本标记，并打开字体面板供调整。
            onClicked: {
                if (bridge.image_width <= 0)
                    return
                // 字体按钮是普通动作按钮：取消其它工具按钮的选中态，
                // 避免点了 T 还残留矩形/椭圆等绘制工具。
                root.markTool = "none"
                widthPopover.visible = false
                mosaicPopover.visible = false
                colorPopover.visible = false
                bridge.add_text_mark(
                    bridge.image_width / 2 - 40, bridge.image_height / 2,
                    "文字", root.markFont, root.markFontSize,
                    root.markBold, root.markItalic, root.markColor)
                fontPopover.visible = true
            }
        }
        IconButton {
            tip: "马赛克（划选区打码，再点一次取消）"
            selected: root.markTool === "mosaic"
            onClicked: root.setMarkTool("mosaic")
            // 像素块图标：3x3 深浅交错的小方块，比 ▦ 更直观。
            Grid {
                anchors.centerIn: parent
                columns: 3
                spacing: 1
                Repeater {
                    model: 9
                    Rectangle {
                        required property int index
                        width: 4
                        height: 4
                        radius: 1
                        color: index % 2 === 0 ? "#e9eaec" : "#8a8f96"
                    }
                }
            }
        }

        Rectangle { width: 9; height: 18; color: "transparent"
            Rectangle { anchors.centerIn: parent; width: 1; height: 18; color: "#3c4043" } }

        // 字体控制：选中文本标记时显示。
        IconButton {
            id: fontBtn
            glyph: "Aa"
            tip: "文本样式"
            visible: root.selMarkIsText
            selected: fontPopover.visible
            onClicked: {
                mosaicPopover.visible = false
                fontPopover.visible = !fontPopover.visible
            }
            Text {
                anchors.bottom: parent.bottom
                anchors.right: parent.right
                anchors.margins: 2
                text: root.markFontSize
                color: "#9aa3b2"
                font.pixelSize: 9
            }
        }

        // 马赛克强度：仅马赛克工具激活时显示。
        IconButton {
            id: mosaicBtn
            tip: "马赛克强度"
            visible: root.markTool === "mosaic"
            selected: mosaicPopover.visible
            onClicked: {
                fontPopover.visible = false
                mosaicPopover.visible = !mosaicPopover.visible
            }
            Grid {
                anchors.centerIn: parent
                anchors.verticalCenterOffset: -3
                columns: 3
                spacing: 1
                Repeater {
                    model: 9
                    Rectangle {
                        required property int index
                        width: 3
                        height: 3
                        radius: 1
                        color: index % 2 === 0 ? "#e9eaec" : "#8a8f96"
                    }
                }
            }
            Text {
                anchors.bottom: parent.bottom
                anchors.right: parent.right
                anchors.margins: 2
                text: root.markMosaic
                color: "#9aa3b2"
                font.pixelSize: 9
            }
        }

        IconButton {
            id: widthBtn
            glyph: "─"
            tip: "笔画粗细"
            selected: widthPopover.visible
            onClicked: {
                colorPopover.visible = false
                fontPopover.visible = false
                mosaicPopover.visible = false
                widthPopover.visible = !widthPopover.visible
            }
            Text {
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.margins: 2
                text: root.markStroke
                color: "#9aa3b2"
                font.pixelSize: 9
            }
        }

        IconButton {
            id: colorBtn
            tip: "颜色"
            selected: colorPopover.visible
            onClicked: {
                widthPopover.visible = false
                fontPopover.visible = false
                mosaicPopover.visible = false
                colorPopover.visible = !colorPopover.visible
            }
            Rectangle {
                anchors.centerIn: parent
                width: 14
                height: 14
                radius: 3
                color: root.markColor
                border.color: "#e9eaec"
                border.width: 1
            }
        }

        IconButton {
            glyph: "▨"
            tip: "填充色开关（矩形/椭圆）"
            active: root.markTool === "rect" || root.markTool === "ellipse"
            selected: root.markFill
                         && (root.markTool === "rect" || root.markTool === "ellipse")
            onClicked: {
                root.markFill = !root.markFill
                applyMarkStyle()
            }
            Rectangle {
                anchors.centerIn: parent
                width: 14
                height: 14
                radius: 3
                color: root.markFillColor
                border.color: root.markFill ? "#e9eaec" : "#5a5e64"
                border.width: 1
            }
        }

        Rectangle { width: 9; height: 18; color: "transparent"
            Rectangle { anchors.centerIn: parent; width: 1; height: 18; color: "#3c4043" } }

        // 调整颜色（macOS 预览风格）：打开实时调色面板；再点一次 = 应用。
        IconButton {
            id: adjustBtn
            glyph: "◑"
            tip: "调整颜色（曝光 / 对比度 / 色阶 …）"
            active: bridge.image_width > 0
            selected: bridge.adjustOpen
            onClicked: root.toggleAdjustPanel()
        }

        Rectangle { width: 9; height: 18; color: "transparent"
            Rectangle { anchors.centerIn: parent; width: 1; height: 18; color: "#3c4043" } }

        // 尺子：显示选区 / 选中图层的四角+中心坐标，并跟随鼠标读数。
        IconButton {
            id: rulerBtn
            tip: "尺子（显示坐标，用于测量）"
            active: bridge.image_width > 0
            selected: root.rulerOn
            onClicked: root.toggleRuler()
            // 直尺图标：外框 + 三根刻度。
            Canvas {
                width: 22
                height: 22
                anchors.centerIn: parent
                // selected 变化时重绘（onPaint 里的绑定不是响应式的）。
                property color ink: rulerBtn.selected ? "#ffffff" : "#e9eaec"
                onInkChanged: requestPaint()
                onPaint: {
                    const c = getContext("2d")
                    c.clearRect(0, 0, width, height)
                    c.strokeStyle = ink
                    c.lineWidth = 1.4
                    c.lineCap = "butt"
                    c.beginPath()
                    c.rect(1.5, 6.5, 19, 9)
                    c.stroke()
                    c.beginPath()
                    c.moveTo(6.0, 6.5); c.lineTo(6.0, 11.5)
                    c.moveTo(11.0, 6.5); c.lineTo(11.0, 14.0)
                    c.moveTo(16.0, 6.5); c.lineTo(16.0, 11.5)
                    c.stroke()
                }
            }
        }

        // 拾色器：打开取色弹窗，点击图片取色并复制色值。
        IconButton {
            id: pickerBtn
            tip: "拾色器（点击图片取色，可复制色值）"
            active: bridge.image_width > 0
            selected: bridge.pickerOpen
            onClicked: root.togglePickerPanel()
            // 吸管图标：斜置的管身 + 尖端 + 顶端胶囊。
            Canvas {
                width: 22
                height: 22
                anchors.centerIn: parent
                property color ink: pickerBtn.selected ? "#ffffff" : "#e9eaec"
                onInkChanged: requestPaint()
                onPaint: {
                    const c = getContext("2d")
                    c.clearRect(0, 0, width, height)
                    c.strokeStyle = ink
                    c.fillStyle = ink
                    c.lineWidth = 1.4
                    c.save()
                    c.translate(11, 11)
                    c.rotate(-Math.PI / 4)
                    c.beginPath()
                    c.rect(-2.6, -6.0, 5.2, 8.5)
                    c.stroke()
                    c.beginPath()
                    c.moveTo(-2.6, 2.5)
                    c.lineTo(2.6, 2.5)
                    c.lineTo(0, 8.0)
                    c.closePath()
                    c.fill()
                    c.beginPath()
                    c.rect(-3.8, -9.2, 7.6, 3.4)
                    c.stroke()
                    c.restore()
                }
            }
        }
    }

    Text {
        anchors.right: parent.right
        anchors.rightMargin: 12
        anchors.verticalCenter: parent.verticalCenter
        text: root.markTool === "none"
              ? "选择工具绘制 · 点击标记可移动，双击文本可编辑"
              : (root.markTool === "brush"
                 ? "按住拖拽绘制笔画"
                 : root.markTool === "mosaic"
                 ? "按住拖拽划选区打码 · 强度可调"
                 : "按住拖拽绘制 · 点击已有标记可移动/缩放")
        color: "#6b7076"
        font.pixelSize: 11
    }
}

// 弹窗定位：贴着触发按钮的正下方（不固定在窗口右侧）。
function placePopover(pop, btn) {
    const p = btn.mapToItem(root, 0, 0)
    pop.x = Math.max(4, Math.min(p.x - 8, root.width - pop.width - 4))
}

// 粗细弹窗：8 档，选中档高亮。
Rectangle {
    id: widthPopover
    visible: false
    z: 220
    width: 168
    height: 8 * 30 + 12
    color: "#2a2d31"
    border.color: "#3c4043"
    radius: 6
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    onVisibleChanged: if (visible) placePopover(widthPopover, widthBtn)

    Column {
        anchors.fill: parent
        anchors.margins: 6

        Repeater {
            model: [2, 3, 4, 6, 8, 12, 16, 24]

            delegate: Item {
                required property int modelData
                width: parent.width
                height: 30

                Rectangle {
                    anchors.fill: parent
                    radius: 4
                    color: root.markStroke === modelData ? "#3d5a80"
                           : widthHover.containsMouse ? "#3a3d42" : "transparent"
                }
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    width: 90
                    height: modelData
                    radius: modelData / 2
                    color: "#d5d8dd"
                }
                Text {
                    visible: root.markStroke === modelData
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: 12
                    text: "✓"
                    color: "#ffffff"
                    font.pixelSize: 13
                }
                MouseArea {
                    id: widthHover
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        root.markStroke = modelData
                        applyMarkStyle()
                        widthPopover.visible = false
                    }
                }
            }
        }
    }
}

// 马赛克强度弹窗：5 档块尺寸，选中档高亮。
Rectangle {
    id: mosaicPopover
    visible: false
    z: 220
    width: 190
    height: 5 * 30 + 12
    color: "#2a2d31"
    border.color: "#3c4043"
    radius: 6
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    onVisibleChanged: if (visible) placePopover(mosaicPopover, mosaicBtn)

    Column {
        anchors.fill: parent
        anchors.margins: 6

        Repeater {
            model: [
                { cell: 4,  label: "细腻" },
                { cell: 8,  label: "较细" },
                { cell: 12, label: "适中" },
                { cell: 18, label: "较粗" },
                { cell: 26, label: "强烈" }
            ]

            delegate: Item {
                required property var modelData
                width: parent.width
                height: 30

                Rectangle {
                    anchors.fill: parent
                    radius: 4
                    color: root.markMosaic === modelData.cell ? "#3d5a80"
                           : mosaicHover.containsMouse ? "#3a3d42" : "transparent"
                }
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 12
                    width: modelData.cell * 1.4 + 8
                    height: modelData.cell * 1.4 + 8
                    radius: 2
                    color: "#7a7f86"
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 70
                    text: modelData.label + "（" + modelData.cell + "px）"
                    color: "#d5d8dd"
                    font.pixelSize: 12
                }
                Text {
                    visible: root.markMosaic === modelData.cell
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: 12
                    text: "✓"
                    color: "#ffffff"
                    font.pixelSize: 13
                }
                MouseArea {
                    id: mosaicHover
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        root.markMosaic = modelData.cell
                        // 选中标记是马赛克时同步改它的强度。
                        if (bridge.selected_mark >= 0)
                            bridge.update_mark_cell(root.markMosaic)
                        mosaicPopover.visible = false
                    }
                }
            }
        }
    }
}

// 文本样式面板（macOS 风格）：字体下拉 + 颜色 / 字号步进 + B I U / 对齐。
Rectangle {
    id: fontPopover
    visible: false
    z: 220
    width: 276
    height: 134
    color: "#26282c"
    border.color: "#3c4043"
    radius: 8
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    onVisibleChanged: {
        if (visible)
            placePopover(fontPopover, fontBtn)
        else
            fontListOpen = false
    }

    property bool fontListOpen: false

    Column {
        anchors.fill: parent
        anchors.margins: 10
        spacing: 7

        // 行 1：字体下拉 + 文字颜色块。
        Row {
            spacing: 7

            Rectangle {
                id: fontDropBtn
                width: 202
                height: 30
                radius: 6
                color: "#2f3237"
                border.color: fontPopover.fontListOpen ? "#378add" : "#3c4043"
                border.width: 1
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 10
                    text: root.markFontName(root.markFont)
                    color: "#e9eaec"
                    font.pixelSize: 13
                    font.family: qmlFontFamily(root.markFont)
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    text: "▾"
                    color: "#9aa3b2"
                    font.pixelSize: 11
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: fontPopover.fontListOpen = !fontPopover.fontListOpen
                }
            }

            Rectangle {
                width: 30
                height: 30
                radius: 6
                color: "#2f3237"
                border.color: "#3c4043"
                Rectangle {
                    anchors.centerIn: parent
                    width: 16
                    height: 16
                    radius: 3
                    color: root.markColor
                    border.color: "#e9eaec"
                    border.width: 1
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        // 文字颜色 = 描边目标；转交给颜色弹窗。
                        colorPopover.editingFill = false
                        fontPopover.visible = false
                        colorPopover.visible = true
                    }
                }
            }
        }

        // 行 2：字号步进 + B / I / U。
        Row {
            spacing: 6

            Rectangle {
                width: 88
                height: 30
                radius: 6
                color: "#2f3237"
                border.color: "#3c4043"
                // 先声明：垫底拦截空白点击（箭头声明在后、层级更高）。
                MouseArea {
                    anchors.fill: parent
                }
                // 字号：默认步进档显示 + 可输入任意自定义值（4-512），
                // Enter 或失焦提交；▲▼ 步进仍可用。
                TextInput {
                    id: sizeInput
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 12
                    width: 48
                    text: root.markFontSize
                    color: "#e9eaec"
                    font.pixelSize: 13
                    validator: IntValidator { bottom: 4; top: 512 }
                    selectByMouse: true
                    onActiveFocusChanged: if (activeFocus) selectAll()
                    onEditingFinished: {
                        const v = parseInt(text, 10)
                        if (!isNaN(v)) {
                            const clamped = Math.max(4, Math.min(512, v))
                            if (clamped !== root.markFontSize) {
                                root.markFontSize = clamped
                                root.applyTextFont()
                            }
                        }
                        text = root.markFontSize
                    }
                }
                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: 4
                    spacing: 1
                    // ▲
                    Item {
                        width: 20
                        height: 13
                        Text {
                            anchors.centerIn: parent
                            text: "▲"
                            color: sizeUpMa.containsMouse ? "#ffffff" : "#9aa3b2"
                            font.pixelSize: 8
                        }
                        MouseArea {
                            id: sizeUpMa
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.stepMarkFontSize(1)
                        }
                    }
                    // ▼
                    Item {
                        width: 20
                        height: 13
                        Text {
                            anchors.centerIn: parent
                            text: "▼"
                            color: sizeDownMa.containsMouse ? "#ffffff" : "#9aa3b2"
                            font.pixelSize: 8
                        }
                        MouseArea {
                            id: sizeDownMa
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.stepMarkFontSize(-1)
                        }
                    }
                }
            }

            Repeater {
                model: [
                    { key: "b", label: "B", get: function () { return root.markBold },
                      set: function (v) { root.markBold = v } },
                    { key: "i", label: "I", get: function () { return root.markItalic },
                      set: function (v) { root.markItalic = v } },
                    { key: "u", label: "U", get: function () { return root.markUnderline },
                      set: function (v) { root.markUnderline = v } }
                ]

                delegate: Rectangle {
                    required property var modelData
                    width: 30
                    height: 30
                    radius: 6
                    color: modelData.get() ? "#3d5a80" : "#2f3237"
                    border.color: modelData.get() ? "#5a7fb0" : "#3c4043"
                    border.width: 1
                    Text {
                        anchors.centerIn: parent
                        text: parent.modelData.label
                        color: parent.modelData.get() ? "#ffffff" : "#c8ccd2"
                        font.pixelSize: 13
                        font.bold: true
                        font.italic: parent.modelData.key === "i"
                    }
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            parent.modelData.set(!parent.modelData.get())
                            applyTextFont()
                        }
                    }
                }
            }
        }

        // 行 3：对齐（左 / 中 / 右）。
        Row {
            spacing: 6

            Repeater {
                model: [0, 1, 2]

                delegate: Rectangle {
                    required property int modelData
                    width: 30
                    height: 30
                    radius: 6
                    color: root.markAlign === modelData ? "#3d5a80" : "#2f3237"
                    border.color: root.markAlign === modelData ? "#5a7fb0" : "#3c4043"
                    border.width: 1

                    // 三条横线的对齐图标：锚点按对齐方式互斥设置。
                    Rectangle { width: 16; height: 2; radius: 1; y: 8
                        color: "#c8ccd2"
                        anchors.left: modelData === 0 ? parent.left : undefined
                        anchors.leftMargin: 7
                        anchors.horizontalCenter: modelData === 1 ? parent.horizontalCenter : undefined
                        anchors.right: modelData === 2 ? parent.right : undefined
                        anchors.rightMargin: 7
                    }
                    Rectangle { width: 16; height: 2; radius: 1; y: 14
                        color: "#c8ccd2"
                        anchors.left: modelData === 0 ? parent.left : undefined
                        anchors.leftMargin: 7
                        anchors.horizontalCenter: modelData === 1 ? parent.horizontalCenter : undefined
                        anchors.right: modelData === 2 ? parent.right : undefined
                        anchors.rightMargin: 7
                    }
                    Rectangle { width: 10; height: 2; radius: 1; y: 20
                        color: "#c8ccd2"
                        anchors.left: modelData === 0 ? parent.left : undefined
                        anchors.leftMargin: 7
                        anchors.horizontalCenter: modelData === 1 ? parent.horizontalCenter : undefined
                        anchors.right: modelData === 2 ? parent.right : undefined
                        anchors.rightMargin: 7
                    }
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            root.markAlign = modelData
                            applyTextFont()
                        }
                    }
                }
            }
        }
    }

    // 字体下拉列表：覆盖在面板下半部（不影响面板自身布局）。
    Rectangle {
        visible: fontPopover.fontListOpen
        z: 15
        x: 10
        y: 46
        width: parent.width - 20
        height: listCol.height + 8
        color: "#2f3237"
        border.color: "#3c4043"
        radius: 6

        Column {
            id: listCol
            x: 4
            y: 4
            width: parent.width - 8
            spacing: 2

            Repeater {
                model: root.fontChoices

                delegate: Rectangle {
                    required property var modelData
                    width: listCol.width
                    height: 30
                    radius: 4
                    color: root.markFont === modelData.id ? "#3d5a80"
                           : fontListHover.containsMouse ? "#3a3d42" : "transparent"
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.left: parent.left
                        anchors.leftMargin: 10
                        text: parent.modelData.name
                        color: "#e9eaec"
                        font.pixelSize: 13
                        font.family: parent.modelData.qml
                    }
                    Text {
                        visible: root.markFont === parent.modelData.id
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.right: parent.right
                        anchors.rightMargin: 10
                        text: "✓"
                        color: "#ffffff"
                        font.pixelSize: 13
                    }
                    MouseArea {
                        id: fontListHover
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            root.markFont = parent.modelData.id
                            applyTextFont()
                            fontPopover.fontListOpen = false
                        }
                    }
                }
            }
        }
    }
}

// 颜色弹窗：描边/填充切换 + 预置色板。
Rectangle {
    id: colorPopover
    visible: false
    z: 220
    width: 250
    height: 208
    color: "#2a2d31"
    border.color: "#3c4043"
    radius: 6
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    onVisibleChanged: if (visible) placePopover(colorPopover, colorBtn)

    property bool editingFill: false

    Row {
        id: colorTargets
        anchors.top: parent.top
        anchors.topMargin: 8
        anchors.left: parent.left
        anchors.leftMargin: 10
        spacing: 8

        Repeater {
            model: [
                { label: "描边", bind: function () { return root.markColor } },
                { label: "填充", bind: function () { return root.markFillColor } }
            ]

            delegate: Item {
                required property var modelData
                // 固定尺寸：Row 子项的尺寸不得反向依赖 Row 自身，
                // 否则触发 polish() 循环，Qt 会放弃整轮布局（连主工具栏
                // 的图标 Row 都不再排版）。宽 = 16 色块 + 4 间距 + 文本。
                width: 16 + 4 + swatchLabel.implicitWidth
                height: 16

                Rectangle {
                    x: 0
                    y: 0
                    width: 16
                    height: 16
                    radius: 3
                    color: modelData.bind()
                    border.color: (colorPopover.editingFill === (modelData.label === "填充"))
                                  ? "#378add" : "#5a5e64"
                    border.width: 1
                }
                Text {
                    id: swatchLabel
                    x: 20
                    anchors.verticalCenter: parent.verticalCenter
                    text: modelData.label
                    color: (colorPopover.editingFill === (modelData.label === "填充"))
                           ? "#e9eaec" : "#9aa3b2"
                    font.pixelSize: 11
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: colorPopover.editingFill = modelData.label === "填充"
                }
            }
        }
    }

    Grid {
        anchors.top: colorTargets.bottom
        anchors.topMargin: 8
        anchors.left: parent.left
        anchors.leftMargin: 10
        columns: 8
        spacing: 6

        Repeater {
            model: [
                "#e24b4a", "#ef9f27", "#f5e14a", "#639922",
                "#378add", "#7f77dd", "#e94aa0", "#111111",
                "#ffffff", "#9aa3b2", "#4a4e54", "#0f6e56",
                "#993556", "#2d6a9f", "#ba7517", "#5f5e5a"
            ]

            delegate: Rectangle {
                required property string modelData
                width: 22
                height: 22
                radius: 4
                color: modelData
                border.width: 2
                border.color: {
                    const cur = colorPopover.editingFill
                                ? root.markFillColor : root.markColor
                    return cur.toLowerCase() === modelData.toLowerCase()
                           ? "#378add" : "transparent"
                }

                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        if (colorPopover.editingFill) {
                            root.markFillColor = modelData
                            root.markFill = true
                        } else {
                            root.markColor = modelData
                        }
                        applyMarkStyle()
                    }
                }
            }
        }
    }

    // 自定义颜色：hex 输入 + 实时预览色块。
    Row {
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 8
        spacing: 8

        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: "自定义"
            color: "#9aa3b2"
            font.pixelSize: 11
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 20
            height: 20
            radius: 3
            color: customHexInput.valid ? "#" + customHexInput.text : "#33373c"
            border.color: "#5a5e64"
            border.width: 1
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 96
            height: 22
            radius: 4
            color: "#222428"
            border.color: customHexInput.activeFocus ? "#378add" : "#3c4043"
            border.width: 1

            TextInput {
                id: customHexInput
                anchors.fill: parent
                anchors.leftMargin: 6
                anchors.rightMargin: 6
                verticalAlignment: TextInput.AlignVCenter
                color: "#e9eaec"
                font.pixelSize: 12
                font.family: "Consolas"
                text: "e24b4a"
                maximumLength: 6
                // 6 位十六进制才有效（不含 #，输入更顺手）。
                readonly property bool valid: /^[0-9a-fA-F]{6}$/.test(text)
                selectByMouse: true
                onAccepted: customHexApply()
            }
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 44
            height: 22
            radius: 4
            color: customHexApplyMa.containsMouse ? "#3d5a80" : "#33373c"
            Text {
                anchors.centerIn: parent
                text: "应用"
                color: "#e9eaec"
                font.pixelSize: 11
            }
            MouseArea {
                id: customHexApplyMa
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: customHexApply()
            }
        }
    }
}

// 缩放预设菜单（状态栏百分比胶囊弹出，向上弹避免越出窗口底）。
Rectangle {
    id: zoomMenu
    visible: false
    z: 220
    width: 200
    height: zoomMenuCol.height + 12
    color: "#2a2d31"
    border.color: "#3c4043"
    radius: 6
    y: statusBar.y - height - 4
    onVisibleChanged: if (visible) placePopover(zoomMenu, zoomPctBtn)

    Column {
        id: zoomMenuCol
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.topMargin: 6
        spacing: 0

        Repeater {
            model: [25, 50, 100, 200, 400, 800]

            delegate: Item {
                required property int modelData
                width: parent.width
                height: 30

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 2
                    radius: 4
                    color: Math.abs(bridge.zoom_percent - modelData) < 1
                           ? "#3d5a80"
                           : zoomItemMa.containsMouse ? "#3a3d42" : "transparent"
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    text: modelData + "%"
                    color: "#d5d8dd"
                    font.pixelSize: 12
                }
                MouseArea {
                    id: zoomItemMa
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        view.queueZoomTo(modelData)
                        zoomMenu.visible = false
                    }
                }
            }
        }

        Item { width: parent.width; height: 7
            Rectangle { anchors.centerIn: parent; width: parent.width - 16; height: 1; color: "#3c4043" }
        }

        Repeater {
            model: [
                { l: "适应窗口 (Ctrl+9)", run: function () { bridge.zoom_fit() } },
                { l: "100% 实际大小 (Ctrl+0)", run: function () { bridge.zoom_actual() } }
            ]

            delegate: Item {
                required property var modelData
                width: parent.width
                height: 30

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 2
                    radius: 4
                    color: zoomItemMa2.containsMouse ? "#3a3d42" : "transparent"
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    text: modelData.l
                    color: "#d5d8dd"
                    font.pixelSize: 12
                }
                MouseArea {
                    id: zoomItemMa2
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        modelData.run()
                        zoomMenu.visible = false
                    }
                }
            }
        }
    }
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

// Traditional text menu behind "…": every command, keyboard-hint labels.
Rectangle {
    id: overflowMenu
    visible: false
    z: 210
    width: 232
    height: menuCol.height + 10
    color: "#2a2d31"
    border.color: "#3c4043"
    radius: 6
    x: toolbar.width - width - 10
    y: toolbar.height + 2

    Column {
        id: menuCol
        anchors.top: parent.top
        anchors.topMargin: 5
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: 0

        Repeater {
            model: toolbar.menuActions

            delegate: MenuRow {
                required property var modelData
                required property int index
                // Reactive like the bar icons above.
                label: modelData.l
                separator: modelData.sep === true
                active: modelData.sep === true
                        || toolbar.menuActions[index].ok === true
                onActivated: {
                    overflowMenu.visible = false
                    modelData.run()
                }
            }
        }
    }
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

Rectangle {
    id: statusBar
    anchors.bottom: parent.bottom
    anchors.left: parent.left
    anchors.right: parent.right
    height: root.statusHeight
    color: "#242628"

    Text {
        id: modeHint
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: zoomCtl.left
        anchors.rightMargin: 10
        visible: root.modeActive || root.selExists || bridge.selected_layer >= 0
        text: root.pasteFloating
              ? "粘贴内容可拖动定位：Enter / 点击外部落定，Esc 撤销粘贴"
              : root.selRubberBanding
              ? "松手完成选区，随后可拖手柄精修、拖内部移动"
              : root.scaleW > 0
              ? "拖角/边调整图层大小（角点等比，Shift 自由）；松手应用，Esc 取消"
              : root.selExists
              ? "已选区：拖手柄缩放 / 拖内部移动；Enter 或 Ctrl+K 应用裁剪，Esc 取消"
              : "已选图层：点住拖动移动，拖手柄缩放；点击空白处或 Esc 取消选择"
        color: "#9fb6d8"
        font.pixelSize: 12
    }

    // File name / resolution / size. When a real file is loaded this is
    // a button: click copies the full path (hidden TextEdit.copy()
    // helper), hover shows a hint tooltip.
    Item {
        id: fileInfo
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: parent.left
        anchors.leftMargin: 10
        height: parent.height
        visible: bridge.fileName.length > 0
        width: fileInfoText.implicitWidth
        readonly property bool hasPath: bridge.sourcePath.length > 0
        property bool hovering: false
        property bool copied: false

        Text {
            id: fileInfoText
            anchors.verticalCenter: parent.verticalCenter
            text: fileInfo.copied
                  ? "已复制完整路径 ✓"
                  : bridge.fileName
                    + (bridge.image_width > 0
                       ? "   ·   " + bridge.image_width + "×" + bridge.image_height
                       : "")
                    + (bridge.fileSizeStr.length > 0 ? "   ·   " + bridge.fileSizeStr : "")
            color: fileInfo.copied ? "#7fc3ff"
                   : (fileInfo.hasPath && fileInfo.hovering ? "#c9dcf5" : "#9aa3b2")
            font.pixelSize: 12
            font.underline: fileInfo.hasPath && fileInfo.hovering && !fileInfo.copied
        }

        // Hand-rolled tooltip (no QtQuick.Controls dependency).
        Rectangle {
            id: fileInfoTip
            visible: false
            z: 1000
            x: 0
            y: parent.height + 4
            width: fileInfoTipText.implicitWidth + 16
            height: 22
            radius: 4
            color: "#111214"
            border.color: "#3c4043"
            Text {
                id: fileInfoTipText
                anchors.centerIn: parent
                text: "点击复制完整路径"
                color: "#d7dade"
                font.pixelSize: 11
            }
        }
        Timer {
            id: fileInfoTipDelay
            interval: 500
            onTriggered: fileInfoTip.visible = fileInfo.hasPath && fileInfo.hovering
        }

        MouseArea {
            id: fileInfoMa
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: fileInfo.hasPath ? Qt.PointingHandCursor : Qt.ArrowCursor
            onContainsMouseChanged: {
                if (containsMouse) {
                    fileInfo.hovering = true
                    if (fileInfo.hasPath && !fileInfo.copied)
                        fileInfoTipDelay.restart()
                    else
                        fileInfoTip.visible = false
                } else {
                    fileInfo.hovering = false
                    fileInfoTip.visible = false
                    fileInfoTipDelay.stop()
                }
            }
            onClicked: {
                if (!fileInfo.hasPath)
                    return
                copyToClipboard(bridge.sourcePath)
                fileInfo.copied = true
                fileInfoTip.visible = false
                fileInfoCopiedReset.restart()
            }
        }
        Timer {
            id: fileInfoCopiedReset
            interval: 2000
            onTriggered: fileInfo.copied = false
        }
    }

    Text {
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: fileInfo.visible ? fileInfo.right : parent.left
        anchors.leftMargin: fileInfo.visible ? 12 : 10
        anchors.right: (root.modeActive || root.selExists) ? modeHint.left : zoomCtl.left
        anchors.rightMargin: 10
        elide: Text.ElideMiddle
        text: bridge.status
        color: "#b8bcc0"
        font.pixelSize: 12
    }

    // 可操作缩放控件：百分比下拉（预设/适应窗口/实际大小）+ 滑杆 + 加减。
    Row {
        id: zoomCtl
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        anchors.rightMargin: 10
        spacing: 6
        visible: bridge.image_width > 0

        // 百分比胶囊按钮 → 弹预设菜单。
        Rectangle {
            id: zoomPctBtn
            width: zoomPctRow.implicitWidth + 18
            height: 20
            anchors.verticalCenter: parent.verticalCenter
            radius: 4
            color: zoomPctMa.containsMouse || zoomMenu.visible
                   ? "#3a3d42" : "#2f3236"
            border.color: "#4a4e54"
            border.width: 1

            Row {
                id: zoomPctRow
                anchors.centerIn: parent
                spacing: 4
                Text {
                    text: bridge.zoom_percent + "%"
                    color: "#d5d8dd"
                    font.pixelSize: 11
                }
                Text {
                    text: "▾"
                    color: "#9aa0a6"
                    font.pixelSize: 9
                    anchors.verticalCenter: parent.verticalCenter
                }
            }

            MouseArea {
                id: zoomPctMa
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    if (zoomMenu.visible)
                        zoomMenu.visible = false
                    else {
                        placePopover(zoomMenu, zoomPctBtn)
                        zoomMenu.visible = true
                    }
                }
            }
        }

        // 滑杆：对数刻度 2%..1600%，拖动与滚轮共用同一条节流管线。
        Item {
            id: zoomSlider
            width: 96
            height: 20
            anchors.verticalCenter: parent.verticalCenter

            property real value: 0.5
            property bool dragging: false

            function zoomToValue(pct) {
                const z = Math.min(1600, Math.max(2, pct))
                return Math.log(z / 2) / Math.log(800)
            }
            function valueToZoom(v) {
                return Math.round(2 * Math.pow(800, v))
            }
            function setValue(x) {
                value = Math.min(1, Math.max(0, x / width))
                view.queueZoomTo(valueToZoom(value))
            }

            Binding {
                target: zoomSlider
                property: "value"
                value: zoomSlider.zoomToValue(bridge.zoom_percent)
                // 等节流定时器把 zoom 落到 bridge 后再恢复跟随，否则
                // 松手瞬间会先弹回旧值、120ms 后又跳到新值。
                when: !zoomSlider.dragging && !view.zoomPending
            }

            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width
                height: 4
                radius: 2
                color: "#3c4043"
            }
            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: zoomSlider.value * zoomSlider.width
                height: 4
                radius: 2
                color: "#5b9dff"
            }
            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                x: zoomSlider.value * (zoomSlider.width - 10)
                width: 10
                height: 10
                radius: 5
                color: zoomMa.containsMouse || zoomSlider.dragging
                       ? "#7fc3ff" : "#e9eaec"
            }

            MouseArea {
                id: zoomMa
                anchors.fill: parent
                anchors.margins: -6
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onPressed: (mouse) => {
                    zoomSlider.dragging = true
                    zoomSlider.setValue(mouse.x + 6)
                }
                onPositionChanged: (mouse) => {
                    if (zoomSlider.dragging)
                        zoomSlider.setValue(mouse.x + 6)
                }
                onReleased: zoomSlider.dragging = false
            }
        }

        // − / + 按钮：与 Ctrl+滚轮同步长（×1.25）。
        Rectangle {
            width: 18
            height: 18
            anchors.verticalCenter: parent.verticalCenter
            radius: 9
            color: zoomMinusMa.containsMouse ? "#3a3d42" : "transparent"
            border.color: "#4a4e54"
            border.width: 1
            Text {
                anchors.centerIn: parent
                text: "−"
                color: "#d5d8dd"
                font.pixelSize: 12
            }
            MouseArea {
                id: zoomMinusMa
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: view.queueZoom(1 / 1.25)
            }
        }
        Rectangle {
            width: 18
            height: 18
            anchors.verticalCenter: parent.verticalCenter
            radius: 9
            color: zoomPlusMa.containsMouse ? "#3a3d42" : "transparent"
            border.color: "#4a4e54"
            border.width: 1
            Text {
                anchors.centerIn: parent
                text: "+"
                color: "#d5d8dd"
                font.pixelSize: 12
            }
            MouseArea {
                id: zoomPlusMa
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: view.queueZoom(1.25)
            }
        }
    }
}

DropArea {
    anchors.fill: parent
    onDropped: (drop) => {
        if (drop.hasUrls && drop.urls.length > 0)
            confirmThenOpen(drop.urls[0]);
    }
}

Item {
    id: layerPanel
    anchors.top: toolbar.bottom
    anchors.right: parent.right
    anchors.bottom: statusBar.top
    width: 230
    visible: root.panelVisible
    z: 50
    clip: true

    Rectangle {
        anchors.fill: parent
        color: "#202327"
    }

    Column {
        id: headerCol
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.topMargin: 8
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        spacing: 6

        Text {
            width: parent.width
            text: "图层"
            color: "#e9eaec"
            font.pixelSize: 13
            font.bold: true
        }

        Text {
            width: parent.width
            text: bridge.layer_count > 0
                  ? "共 " + bridge.layer_count + " 个图层"
                  : "暂无图层：框选后点「提取图层」"
            color: "#6b7076"
            font.pixelSize: 11
        }

        Rectangle { width: parent.width; height: 1; color: "#3c4043" }
    }

    Item {
        id: listArea
        anchors.top: headerCol.bottom
        anchors.bottom: panelFooter.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.topMargin: 8
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        clip: true

        Column {
            id: layerRepeaterColumn
            width: listArea.width
            spacing: 4

            Repeater {
                id: layerRepeater
                // Panel list updates only when layer identity/flags
                // change (not on every live-drag bounds tick) — see
                // layerPanelSig on the root item.
                model: root.layerPanelList

            delegate: Rectangle {
                id: layerRow
                width: layerRepeaterColumn.width
                height: 34
                radius: 4
                // Repeater + JS-array models expose the row either as the
                // `model` context object or as `modelData` depending on the
                // model wrapper; resolve once, defensively.
                readonly property var row: {
                    if (typeof modelData !== "undefined" && modelData !== null && modelData.n !== undefined)
                        return modelData
                    if (model !== undefined && model !== null && model.n !== undefined)
                        return model
                    return null
                }
                readonly property bool isBase: row !== null && row.base === true
                readonly property bool isSel: row !== null && row.i === bridge.selected_layer
                readonly property bool vis: row !== null && row.v === true
                readonly property int disp: index
                color: isSel ? "#3d5a80"
                     : (rowMa.containsMouse && !isBase ? "#3a3e44" : "#33373c")

                // Press = select; press + drag = reorder (insertion line
                // shows the drop slot); double-click = rename. The base
                // row is display-only.
                MouseArea {
                    id: rowMa
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: layerRow.row !== null
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    cursorShape: layerRow.isBase ? Qt.ArrowCursor : Qt.PointingHandCursor
                    property bool reordering: false
                    property int fromPos: -1
                    property int toPos: -1
                    property real pressY: 0

                    onPressed: (mouse) => {
                        if (mouse.button === Qt.RightButton) {
                            if (!layerRow.isBase && layerRow.row)
                                openLayerContext(layerRow.row.i,
                                                 layerRow.row.l === true,
                                                 layerRow.mapToItem(layerPanel,
                                                                    mouse.x, mouse.y))
                            return
                        }
                        pressY = mouse.y
                        reordering = false
                    }
                    onPositionChanged: (mouse) => {
                        if (!pressed || layerRow.isBase || (layerRow.row && layerRow.row.l === true))
                            return
                        if (!reordering && Math.abs(mouse.y - pressY) > 6) {
                            reordering = true
                            fromPos = layerRow.disp
                        }
                        if (!reordering)
                            return
                        const pitch = 38  // row 34 + spacing 4
                        // Non-base row count: the synthetic base row
                        // (present only while the base is still locked)
                        // is never a valid reorder target.
                        const docLen = nonBaseRowCount()
                        const colY = layerRow.mapToItem(layerRepeaterColumn, 0, mouse.y).y
                        let t = Math.floor(colY / pitch + 0.5)
                        t = Math.max(0, Math.min(t, docLen - 1))
                        toPos = t
                        dropIndicator.y = t * pitch - 2
                        dropIndicator.visible = t !== fromPos
                    }
                    onReleased: (mouse) => {
                        if (mouse.button === Qt.RightButton)
                            return
                        if (reordering && toPos >= 0 && toPos !== fromPos) {
                            const docLen = nonBaseRowCount()
                            // Display position p ↔ doc index docLen-1-p.
                            bridge.reorder_layer(docLen - 1 - fromPos,
                                                 docLen - 1 - toPos)
                        } else if (!reordering && layerRow.row && !layerRow.isBase
                                   && layerRow.row.l !== true) {
                            // Locked layers reject selection; unlock via
                            // the row's lock icon first. Selecting a
                            // layer takes over the canvas: drop any
                            // active selection so the layer's handles
                            // and drag become usable at once.
                            root.selW = 0
                            root.selH = 0
                            bridge.select_layer(layerRow.row.i)
                        }
                        reordering = false
                        toPos = -1
                        dropIndicator.visible = false
                    }
                    onDoubleClicked: if (layerRow.row && !layerRow.isBase) {
                        root.renamingIndex = layerRow.row.i
                        renameInput.focus = true
                        renameInput.selectAll()
                    }
                }

                // Eye icon: click toggles this layer's visibility.
                Item {
                    x: 5
                    width: 22
                    height: parent.height

                    Canvas {
                        id: eyeIcon
                        anchors.centerIn: parent
                        width: 18
                        height: 18
                        property bool on: layerRow.vis
                        readonly property bool disabled: layerRow.isBase
                            && layerRow.row && layerRow.row.bd === true
                        onOnChanged: requestPaint()
                        onDisabledChanged: requestPaint()
                        onPaint: {
                            var ctx = getContext("2d")
                            ctx.clearRect(0, 0, width, height)
                            ctx.lineWidth = 1.4
                            ctx.strokeStyle = disabled ? "#4a4e54"
                                                : (on ? "#7fc3ff" : "#5a5f66")
                            ctx.beginPath()
                            ctx.moveTo(2, 9)
                            ctx.quadraticCurveTo(9, 2.5, 16, 9)
                            ctx.quadraticCurveTo(9, 15.5, 2, 9)
                            ctx.stroke()
                            if (on) {
                                ctx.beginPath()
                                ctx.arc(9, 9, 2.6, 0, Math.PI * 2)
                                ctx.fillStyle = ctx.strokeStyle
                                ctx.fill()
                            } else {
                                ctx.beginPath()
                                ctx.moveTo(4.5, 14)
                                ctx.lineTo(13.5, 4)
                                ctx.stroke()
                            }
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        anchors.margins: -4
                        // Base: hidden via the dedicated undoable toggle
                        // (set_layer_visible_at only indexes real layers;
                        // deleted base rows have no eye at all).
                        enabled: !(layerRow.isBase && layerRow.row
                                   && layerRow.row.bd === true)
                        cursorShape: Qt.PointingHandCursor
                        onClicked: if (layerRow.row) {
                            if (layerRow.isBase)
                                bridge.toggle_base_visible()
                            else
                                bridge.set_layer_visible_at(layerRow.row.i, !layerRow.row.v)
                        }
                    }
                }

                Text {
                    x: 30
                    width: parent.width - 62
                    anchors.verticalCenter: parent.verticalCenter
                    elide: Text.ElideRight
                    visible: !renameInput.visible
                    text: layerRow.row
                          ? layerRow.row.n
                            + (layerRow.row.o !== undefined && layerRow.row.o < 100 ? "  " + layerRow.row.o + "%" : "")
                            // Selection is shown by row highlight only.
                          : ""
                    color: isSel ? "#ffffff" : "#c9cdd2"
                    font.pixelSize: 12
                }

                TextInput {
                    id: renameInput
                    x: 30
                    width: parent.width - 62
                    anchors.verticalCenter: parent.verticalCenter
                    visible: root.renamingIndex !== -1 && root.renamingIndex === (layerRow.row ? layerRow.row.i : -1)
                    text: layerRow.row ? layerRow.row.n : ""
                    color: "#ffffff"
                    font.pixelSize: 12
                    focus: visible
                    onAccepted: {
                        if (text.trim().length > 0)
                            bridge.rename_selected_layer(text)
                        root.renamingIndex = -1
                    }
                    onActiveFocusChanged: if (!activeFocus && visible)
                        root.renamingIndex = -1
                }

                // Lock icon (right side): click toggles the lock. Locked
                // layers reject move / scale / delete / reorder.
                Item {
                    anchors.right: parent.right
                    anchors.rightMargin: 4
                    width: 22
                    height: parent.height

                    Canvas {
                        id: lockIcon
                        anchors.centerIn: parent
                        width: 16
                        height: 16
                        property bool on: layerRow.row !== null && layerRow.row.l === true
                        onOnChanged: requestPaint()
                        onPaint: {
                            var ctx = getContext("2d")
                            ctx.clearRect(0, 0, width, height)
                            ctx.lineWidth = 1.4
                            ctx.strokeStyle = on ? "#e8b34a" : "#5a5f66"
                            ctx.fillStyle = on ? "#e8b34a" : "#5a5f66"
                            // Shackle.
                            ctx.beginPath()
                            if (on) {
                                ctx.arc(8, 7, 3.2, Math.PI, 0)
                            } else {
                                ctx.arc(8, 7, 3.2, Math.PI * 0.85, Math.PI * 0.15, false)
                            }
                            ctx.stroke()
                            // Body.
                            ctx.beginPath()
                            ctx.roundRect ? ctx.roundRect(4, 7, 8, 6.5, 1.5)
                                          : ctx.rect(4, 7, 8, 6.5)
                            ctx.fill()
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        anchors.margins: -4
                        cursorShape: Qt.PointingHandCursor
                        onClicked: if (layerRow.row) {
                            if (layerRow.isBase)
                                bridge.set_base_locked(!(layerRow.row.l === true))
                            else
                                bridge.set_layer_locked(layerRow.row.i, !(layerRow.row.l === true))
                        }
                    }
                }
            }
            }

            // ---- 标记分区：画布上的全部矢量标记（左键选中，Delete 删除）----
            Rectangle {
                width: parent.width
                height: 1
                color: "#3c4043"
                visible: root.markList.length > 0
            }

            Text {
                id: markSectionHdr
                width: layerRepeaterColumn.width
                text: "标记（" + root.markList.length + "）"
                color: "#e9eaec"
                font.pixelSize: 12
                font.bold: true
                visible: root.markList.length > 0
            }

            Repeater {
                model: root.markList

                delegate: Rectangle {
                    id: markRow
                    required property var modelData
                    required property int index
                    readonly property bool isSel: modelData.i === bridge.selected_mark
                    readonly property bool isVisible: modelData.v !== false
                    width: layerRepeaterColumn.width
                    height: 26
                    radius: 4
                    color: isSel ? "#3d5a80"
                         : (markRowMa.containsMouse ? "#3a3e44" : "#33373c")
                    opacity: isVisible ? 1.0 : 0.45

                    Text {
                        x: 32
                        anchors.verticalCenter: parent.verticalCenter
                        text: (markRow.modelData.k === 0 ? "▭ 矩形"
                               : markRow.modelData.k === 1 ? "◯ 椭圆"
                               : markRow.modelData.k === 2 ? "↗ 箭头"
                               : markRow.modelData.k === 4 ? "▦ 马赛克"
                               : markRow.modelData.k === 5
                               ? "T 文本：" + (markRow.modelData.t.length > 8
                                   ? markRow.modelData.t.slice(0, 8) + "…"
                                   : markRow.modelData.t)
                               : "✏ 画笔")
                              + "  " + (markRow.index + 1)
                        color: "#d5d8dd"
                        font.pixelSize: 11
                    }
                    Text {
                        anchors.right: parent.right
                        anchors.rightMargin: 8
                        anchors.verticalCenter: parent.verticalCenter
                        text: markRow.modelData.k === 5
                              ? markRow.modelData.fs + "px " + markRow.modelData.fn
                              : markRow.modelData.k === 4
                              ? "强度 " + markRow.modelData.cell + "px"
                              : markRow.modelData.s + "px  " + markRow.modelData.c
                        color: "#6b7076"
                        font.pixelSize: 10
                    }
                    // 行点击=选中。声明在眼睛之前，眼睛在其上层接收点击。
                    MouseArea {
                        id: markRowMa
                        anchors.fill: parent
                        hoverEnabled: true
                        acceptedButtons: Qt.LeftButton
                        cursorShape: Qt.PointingHandCursor
                        onClicked: bridge.select_mark(markRow.modelData.i)
                    }
                    // 眼睛：切换该标记的显示/隐藏（可撤销）。
                    Item {
                        id: markEye
                        x: 6
                        width: 22
                        height: parent.height

                        Canvas {
                            id: markEyeIcon
                            anchors.centerIn: parent
                            width: 16
                            height: 16
                            property bool on: markRow.isVisible
                            onOnChanged: requestPaint()
                            onPaint: {
                                var ctx = getContext("2d")
                                ctx.clearRect(0, 0, width, height)
                                ctx.lineWidth = 1.4
                                var c = on ? "#d5d8dd" : "#8a9099"
                                ctx.strokeStyle = c
                                ctx.fillStyle = c
                                ctx.beginPath()
                                ctx.ellipse(8, 8, 6.5, 4.2, 0, 0, Math.PI * 2)
                                ctx.stroke()
                                if (on) {
                                    ctx.beginPath()
                                    ctx.arc(8, 8, 1.8, 0, Math.PI * 2)
                                    ctx.fill()
                                } else {
                                    ctx.beginPath()
                                    ctx.moveTo(3.5, 12.5)
                                    ctx.lineTo(12.5, 3.5)
                                    ctx.stroke()
                                }
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            anchors.margins: -4
                            cursorShape: Qt.PointingHandCursor
                            onClicked: bridge.set_mark_visible(
                                markRow.modelData.i, markRow.isVisible === false)
                        }
                    }
                }
            }
        }
            // Drop-position line shown while dragging a row to reorder.
            Rectangle {
                id: dropIndicator
                visible: false
                width: parent.width
                height: 2
                color: "#5b9dff"
                radius: 1
                z: 10
            }
    }

    Column {
        id: panelFooter
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottomMargin: 8
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        spacing: 6
        visible: bridge.selected_layer >= 0

        Row {
            id: opacityRow
            width: parent.width
            spacing: 6

            Text {
                text: "不透明度"
                color: "#b8bcc0"
                font.pixelSize: 11
                anchors.verticalCenter: parent.verticalCenter
            }

            Item {
                id: opacitySlider
                width: parent.width - 104
                height: 20
                anchors.verticalCenter: parent.verticalCenter

                property real value: 1.0
                property bool dragging: false

                function setValue(x) {
                    value = Math.min(1.0, Math.max(0.0, x / width))
                    bridge.set_selected_opacity(value, false)
                }

                Binding {
                    target: opacitySlider
                    property: "value"
                    value: root.selLayer ? root.selLayer.o / 100.0 : 1.0
                    when: !opacitySlider.dragging
                }

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width
                    height: 4
                    radius: 2
                    color: "#3c4043"
                }

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: opacitySlider.value * opacitySlider.width
                    height: 4
                    radius: 2
                    color: "#5b9dff"
                }

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    x: opacitySlider.value * (opacitySlider.width - 12)
                    width: 12
                    height: 12
                    radius: 6
                    color: opMa.containsMouse || opacitySlider.dragging ? "#7fc3ff" : "#e9eaec"
                }

                MouseArea {
                    id: opMa
                    anchors.fill: parent
                    anchors.margins: -6
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onPressed: (mouse) => {
                        opacitySlider.dragging = true
                        opacitySlider.setValue(mouse.x + 6)
                    }
                    onPositionChanged: (mouse) => {
                        if (opacitySlider.dragging)
                            opacitySlider.setValue(mouse.x + 6)
                    }
                    onReleased: {
                        opacitySlider.dragging = false
                        bridge.set_selected_opacity(opacitySlider.value, true)
                    }
                }
            }

            Text {
                width: 40
                text: Math.round(opacitySlider.value * 100) + "%"
                color: "#b8bcc0"
                font.pixelSize: 11
                anchors.verticalCenter: parent.verticalCenter
                horizontalAlignment: Text.AlignRight
            }
        }
        // Reorder = drag rows; delete = Delete/Backspace on the selected
        // row; visibility = the eye icon (per §4.8 of the UX plan).
    }

    // Click-away catcher for the layer row context menu.
    MouseArea {
        z: 99
        anchors.fill: parent
        visible: layerContext.visible
        enabled: layerContext.visible
        onPressed: (mouse) => {
            layerContext.visible = false
            mouse.accepted = true
        }
    }

    // Right-click menu on a layer row: delete / export PNG.
    Rectangle {
        id: layerContext
        visible: false
        z: 100
        width: 232
        height: ctxCol.height + 10
        color: "#2a2d31"
        border.color: "#3c4043"
        radius: 6
        property int targetIndex: -1
        property bool targetLocked: false

        Column {
            id: ctxCol
            anchors.top: parent.top
            anchors.topMargin: 5
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 0

            MenuRow {
                label: "删除图层"
                active: layerContext.targetIndex >= 0 && !layerContext.targetLocked
                onActivated: {
                    layerContext.visible = false
                    const i = layerContext.targetIndex
                    if (i >= 0) {
                        bridge.select_layer(i)
                        bridge.delete_selected_layer()
                    }
                }
            }
            MenuRow {
                label: "导出 PNG"
                active: layerContext.targetIndex >= 0
                onActivated: {
                    layerContext.visible = false
                    const i = layerContext.targetIndex
                    if (i >= 0)
                        bridge.export_layer_png(i)
                }
            }
        }
    }
}

// ---- 调整颜色面板（macOS 预览「调整颜色」，见截图）------------------
// 实时预览只改参数；关闭面板（✕ / 再点工具栏按钮）才把参数烘焙成一次
// 可撤销操作，Esc 放弃（文档像素从未被改动）。
Item {
    id: adjustPanel
    visible: bridge.adjustOpen
    width: 320
    // 弹窗样式：贴着「调整颜色」按钮的正下方（与其它标记弹窗一致），
    // 窗口太矮时面板内部滚动。高度与 y 无关，拖动时不会越拖越矮。
    readonly property real maxHeight: Math.max(240, root.height - toolbar.height - 24)
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
            placePopover(adjustPanel, adjustBtn)
        }
    }
    onAnchorXChanged: if (visible && !userMoved) placePopover(adjustPanel, adjustBtn)
    // 窗口变矮 / 面板内容变高时，把用户拖过的位置重新夹回窗口内。
    onHeightChanged: if (visible && userMoved) moveTo(x, y)

    // 拖动标题栏移动弹窗（限制在窗口内，别拖丢）。
    function moveTo(nx, ny) {
        userMoved = true
        x = Math.max(4, Math.min(Math.max(4, root.width - width - 4), nx))
        const minY = toolbar.height + 2
        const maxY = Math.max(minY, root.height - height - 8)
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
                        const p = mapToItem(root, mouse.x, mouse.y)
                        pressX = p.x
                        pressY = p.y
                        fromX = adjustPanel.x
                        fromY = adjustPanel.y
                    }
                    onPositionChanged: (mouse) => {
                        if (!pressed)
                            return
                        const p = mapToItem(root, mouse.x, mouse.y)
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
                    onClicked: root.commitAdjust()
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
                        property var hist: parseHistogram(bridge.adjustHistogram)
                        onHistChanged: requestPaint()
                        // 裁剪区随色阶手柄变化重绘。
                        property real blk: root.adjustVals.bl
                        property real wht: root.adjustVals.wh
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
                        const v = root.adjustVals
                        if (kind === "black")
                            root.setLevels(t, v.ga, v.wh)
                        else if (kind === "white")
                            root.setLevels(v.bl, v.ga, t)
                        else
                            root.setLevels(v.bl, Math.max(0.05, Math.min(20,
                                           Math.log(Math.max(0.02, Math.min(0.98, t)))
                                           / Math.log(0.5))), v.wh)
                    }

                    Repeater {
                        model: [{ kind: "black" }, { kind: "mid" }, { kind: "white" }]

                        delegate: Item {
                            id: handleItem
                            required property var modelData
                            readonly property real pos: {
                                const v = root.adjustVals
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
                    text: "黑场 " + Math.round(root.adjustVals.bl * 255)
                          + "    中间 " + root.adjustVals.ga.toFixed(2)
                          + "    白场 " + Math.round(root.adjustVals.wh * 255)
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
                glyph: "☼"; label: "曝光"; key: "exposure"; value: root.adjustVals.e
                onMoved: (v) => root.setAdjust("exposure", v)
                onResetRequested: root.applyAdjustNow("exposure", 0)
            }
            AdjustRow {
                glyph: "◑"; label: "对比度"; key: "contrast"; value: root.adjustVals.c
                onMoved: (v) => root.setAdjust("contrast", v)
                onResetRequested: root.applyAdjustNow("contrast", 0)
            }
            AdjustRow {
                glyph: "◉"; label: "高光"; key: "highlights"; value: root.adjustVals.hi
                onMoved: (v) => root.setAdjust("highlights", v)
                onResetRequested: root.applyAdjustNow("highlights", 0)
            }
            AdjustRow {
                glyph: "◍"; label: "阴影"; key: "shadows"; value: root.adjustVals.sh
                onMoved: (v) => root.setAdjust("shadows", v)
                onResetRequested: root.applyAdjustNow("shadows", 0)
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 组 2：颜色 ----
            AdjustRow {
                glyph: "◈"; label: "饱和度"; key: "saturation"; value: root.adjustVals.sa
                onMoved: (v) => root.setAdjust("saturation", v)
                onResetRequested: root.applyAdjustNow("saturation", 0)
            }
            AdjustRow {
                glyph: "❄"; label: "色温"; key: "temperature"; value: root.adjustVals.te
                onMoved: (v) => root.setAdjust("temperature", v)
                onResetRequested: root.applyAdjustNow("temperature", 0)
            }
            AdjustRow {
                glyph: "◐"; label: "色调"; key: "tint"; value: root.adjustVals.ti
                onMoved: (v) => root.setAdjust("tint", v)
                onResetRequested: root.applyAdjustNow("tint", 0)
            }
            AdjustRow {
                glyph: "▤"; label: "棕褐色"; key: "sepia"; value: root.adjustVals.se
                minVal: 0; maxVal: 1
                onMoved: (v) => root.setAdjust("sepia", v)
                onResetRequested: root.applyAdjustNow("sepia", 0)
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 组 3：清晰度 ----
            AdjustRow {
                glyph: "◆"; label: "清晰度"; key: "sharpness"; value: root.adjustVals.sp
                minVal: 0; maxVal: 1
                onMoved: (v) => root.setAdjust("sharpness", v)
                onResetRequested: root.applyAdjustNow("sharpness", 0)
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
                        root.adjustDirty = false
                        root.levelsDirty = false
                        bridge.adjust_reset()
                    }
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
    running: bridge.adjustOpen && (root.adjustDirty || root.levelsDirty)
    onTriggered: {
        root.flushAdjust()
        root.flushLevels()
    }
}

// ---- 拾色器弹窗 ------------------------------------------------------
// 可拖动 / 可关闭的取色面板：面板打开期间点击画布取样，逐行显示 HEX /
// RGB / HSL / CMYK 并提供复制按钮（美术同学直接取色值）。
Item {
    id: pickerPanel
    visible: bridge.pickerOpen
    width: 316
    readonly property real maxHeight: Math.max(220, root.height - toolbar.height - 24)
    readonly property real anchorX: pickerBtn.x + pickerBtn.parent.x
    height: Math.min(pickerCol.implicitHeight + 24, maxHeight)
    x: 8
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    z: 221
    property bool userMoved: false
    onVisibleChanged: {
        if (visible) {
            userMoved = false
            placePopover(pickerPanel, pickerBtn)
        }
    }
    onAnchorXChanged: if (visible && !userMoved) placePopover(pickerPanel, pickerBtn)
    onHeightChanged: if (visible && userMoved) moveTo(x, y)

    function moveTo(nx, ny) {
        userMoved = true
        x = Math.max(4, Math.min(Math.max(4, root.width - width - 4), nx))
        const minY = toolbar.height + 2
        const maxY = Math.max(minY, root.height - height - 8)
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
        id: pickerFlick
        anchors.fill: parent
        anchors.margins: 12
        contentWidth: width
        contentHeight: pickerCol.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height

        Column {
            id: pickerCol
            width: pickerFlick.width
            spacing: 8

            // 标题栏：拖动区先声明，✕ 后声明才能点得到。
            Item {
                id: pickerTitleBar
                width: parent.width
                height: 24

                MouseArea {
                    id: pickerDragArea
                    anchors.fill: parent
                    anchors.leftMargin: -10
                    anchors.rightMargin: -2
                    cursorShape: Qt.SizeAllCursor
                    property real pressX: 0
                    property real pressY: 0
                    property real fromX: 0
                    property real fromY: 0
                    onPressed: (mouse) => {
                        const p = mapToItem(root, mouse.x, mouse.y)
                        pressX = p.x
                        pressY = p.y
                        fromX = pickerPanel.x
                        fromY = pickerPanel.y
                    }
                    onPositionChanged: (mouse) => {
                        if (!pressed)
                            return
                        const p = mapToItem(root, mouse.x, mouse.y)
                        pickerPanel.moveTo(fromX + (p.x - pressX),
                                           fromY + (p.y - pressY))
                    }
                }

                Text {
                    id: pickerTitle
                    anchors.verticalCenter: parent.verticalCenter
                    text: "拾色器"
                    color: "#e9eaec"
                    font.pixelSize: 13
                    font.bold: true
                }
                Text {
                    anchors.left: pickerTitle.right
                    anchors.leftMargin: 8
                    anchors.right: pickerClose.left
                    anchors.rightMargin: 6
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.pickerVals ? "已取样" : "点击图片取色"
                    color: "#8f96a3"
                    font.pixelSize: 10
                    elide: Text.ElideRight
                }
                IconButton {
                    id: pickerClose
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    glyph: "✕"
                    tip: "关闭拾色器（Esc）"
                    onClicked: bridge.picker_close()
                }
            }

            // ---- 当前颜色：大色块 + HEX + 坐标 ----
            Row {
                spacing: 10

                Rectangle {
                    width: 56
                    height: 56
                    radius: 5
                    color: root.pickerVals ? root.pickerVals.hex : "#1b1c1e"
                    border.color: "#5a6069"
                    border.width: 1
                }

                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 3

                    Text {
                        text: root.pickerVals ? root.pickerVals.hex : "—"
                        color: "#f2f3f5"
                        font.pixelSize: 20
                        font.family: "Consolas"
                        font.bold: true
                    }
                    Text {
                        text: root.pickerVals
                              ? "像素 (" + root.pickerVals.x + ", "
                                + root.pickerVals.y + ")"
                                + (root.pickerVals.a < 255
                                   ? "　不透明度 "
                                     + Math.round(root.pickerVals.a / 255 * 100) + "%"
                                   : "")
                              : "在图片上点一下取色"
                        color: "#8f96a3"
                        font.pixelSize: 10
                    }
                }
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            // ---- 四种色值表示，每行右侧一个复制按钮 ----
            Repeater {
                model: root.pickerSpecs
                delegate: Item {
                    required property var modelData
                    width: pickerCol.width
                    height: 26

                    Text {
                        x: 0
                        width: 44
                        anchors.verticalCenter: parent.verticalCenter
                        text: modelData.k
                        color: "#8f96a3"
                        font.pixelSize: 11
                    }
                    Text {
                        x: 48
                        width: parent.width - 48 - 66
                        anchors.verticalCenter: parent.verticalCenter
                        text: modelData.v
                        color: "#e9eaec"
                        font.pixelSize: 12
                        font.family: "Consolas"
                        elide: Text.ElideRight
                    }
                    ToolButton {
                        width: 58
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        label: root.pickerCopied === modelData.k ? "已复制" : "复制"
                        selected: root.pickerCopied === modelData.k
                        onClicked: root.copyPickerValue(modelData.k, modelData.v)
                    }
                }
            }

            // ---- 最近取色（点击切换当前色，便于反复复制）----
            Item {
                width: parent.width
                height: visible ? pickerRecentLabel.height + 4 + 24 + 4 : 0
                visible: root.pickerHist.length > 0

                Text {
                    id: pickerRecentLabel
                    text: "最近取色（点击切换）"
                    color: "#8f96a3"
                    font.pixelSize: 10
                }
                Text {
                    anchors.right: parent.right
                    anchors.top: parent.top
                    text: "清空"
                    color: "#8f96a3"
                    font.pixelSize: 10
                    MouseArea {
                        anchors.fill: parent
                        anchors.margins: -3
                        cursorShape: Qt.PointingHandCursor
                        onClicked: bridge.picker_clear()
                    }
                }
                Row {
                    anchors.top: pickerRecentLabel.bottom
                    anchors.topMargin: 4
                    spacing: 4

                    Repeater {
                        model: root.pickerHist

                        delegate: Rectangle {
                            required property var modelData
                            required property int index
                            width: 22
                            height: 22
                            radius: 3
                            color: modelData.hex
                            border.color: root.pickerVals
                                          && root.pickerVals.hex === modelData.hex
                                          ? "#6aa9ff" : "#5a6069"
                            border.width: root.pickerVals
                                          && root.pickerVals.hex === modelData.hex ? 2 : 1
                            MouseArea {
                                anchors.fill: parent
                                cursorShape: Qt.PointingHandCursor
                                onClicked: bridge.picker_select(index)
                            }
                        }
                    }
                }
            }

            Text {
                width: parent.width
                text: "点击图片取色 · Esc 关闭 · 拖动标题栏可移动\n"
                      + "CMYK 为无色彩配置的近似换算"
                color: "#6b7076"
                font.pixelSize: 10
                wrapMode: Text.WordWrap
            }
        }
    }
}

// 「已复制」提示的自动复位。
Timer {
    id: pickerCopiedTimer
    interval: 1200
    onTriggered: root.pickerCopied = ""
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
    if (layerContext.visible) { layerContext.visible = false; return }
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

// Open the right-click layer menu at `panelPos` (layerPanel coords),
// clamped so it never spills outside the panel.
function openLayerContext(index, locked, panelPos) {
    layerContext.targetIndex = index
    layerContext.targetLocked = locked
    layerContext.x = Math.max(0, Math.min(panelPos.x, layerPanel.width - layerContext.width))
    layerContext.y = Math.max(0, Math.min(panelPos.y,
                                          layerPanel.height - layerContext.height - 8))
    layerContext.visible = true
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
    if (!customHexInput.valid)
        return
    const hex = "#" + customHexInput.text.toLowerCase()
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
    sizeInput.text = steps[i]
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

// PSD is preview-only: Ctrl+S / 保存 on a .psd document opens this
// dialog instead of writing (a direct save would corrupt the file —
// image has no PSD encoder). Routes to the Save-As dialog.
Item {
    id: psdHintOverlay
    anchors.fill: parent
    visible: false
    z: 200

    MouseArea { anchors.fill: parent }

    Rectangle {
        width: 420
        height: 190
        anchors.centerIn: parent
        radius: 8
        color: "#25272b"
        border.color: "#4a4e54"
        clip: true

        Item {
            id: psdHintTitle
            width: parent.width
            height: 38

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                text: "PSD 不能直接保存"
                color: "#e9eaec"
                font.pixelSize: 13
                font.bold: true
            }

            IconButton {
                anchors.right: parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                glyph: "✕"
                tip: "关闭"
                onClicked: psdHintOverlay.visible = false
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: "#3c4043"
            }
        }

        Item {
            anchors.top: psdHintTitle.bottom
            anchors.bottom: psdHintFooter.top
            anchors.left: parent.left
            anchors.right: parent.right

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 18
                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                text: "PSD 是预览专用格式（打开时图层已合并），直接保存会损坏原文件。\n请使用「另存为」导出 PNG / JPEG 等格式。"
                color: "#c9cdd2"
                font.pixelSize: 13
                wrapMode: Text.WordWrap
            }
        }

        Item {
            id: psdHintFooter
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 58

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: 1
                color: "#3c4043"
            }

            Row {
                anchors.right: parent.right
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8

                ToolButton {
                    label: "取消"
                    active: true
                    onClicked: psdHintOverlay.visible = false
                }
                ToolButton {
                    label: "另存为…"
                    active: true
                    selected: true
                    onClicked: {
                        psdHintOverlay.visible = false
                        saveDialog.open()
                    }
                }
            }
        }
    }
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
