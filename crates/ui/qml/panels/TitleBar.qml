import QtQuick
import QtQuick.Window
import com.imageedit

Rectangle {
    id: toolbar
    required property var host
    required property var bridge
    required property var view
    required property var openDialog
    required property var saveDialog
    required property var settingsOverlay
    required property var resizeOverlay
    required property var overflowMenu
    required property var textPopup
    required property var widthPopover
    required property var colorPopover
    required property var mosaicPopover
    required property var fontPopover
    anchors.top: parent.top
    anchors.left: parent.left
    anchors.right: parent.right
    height: host.toolbarHeight
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
        { g: "⛶", t: "应用裁剪 (Enter / Ctrl+K)", ok: host.selExists,
          run: function () { host.applyCrop() } },
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
        { l: "新窗口 (Ctrl+N)", ok: true, run: function () { host.requestNewWindow() } },
        { l: "", ok: true, sep: true },
        { l: "撤销 (Ctrl+Z)", ok: bridge.can_undo, run: function () { bridge.undo() } },
        { l: "重做 (Ctrl+Shift+Z)", ok: bridge.can_redo, run: function () { bridge.redo() } },
        { l: "", ok: true, sep: true },
        { l: "逆时针旋转 90° (Ctrl+L)", ok: true, run: function () { bridge.rotate_ccw() } },
        { l: "顺时针旋转 90° (Ctrl+R)", ok: true, run: function () { bridge.rotate_cw() } },
        { l: "水平翻转", ok: true, run: function () { bridge.flip_horizontal() } },
        { l: "垂直翻转", ok: true, run: function () { bridge.flip_vertical() } },
        { l: "", ok: true, sep: true },
        { l: "应用裁剪 (Enter / Ctrl+K)", ok: host.selExists, run: function () { host.applyCrop() } },
        { l: "重置裁剪", ok: bridge.image_width > 0, run: function () { bridge.reset_crop() } },
        { l: "调整尺寸…", ok: bridge.image_width > 0, run: function () { resizeOverlay.openResizeDialog() } },
        { l: "", ok: true, sep: true },
        { l: "提取图层", ok: host.selExists, run: function () { host.commitExtract() } },
        { l: "删除选区像素 (Delete)", ok: host.selExists, run: function () { host.deleteSelectionPixels() } },
        { l: "图层面板", ok: true, run: function () { host.panelVisible = !host.panelVisible } },
        { l: "标记工具栏", ok: true, run: function () { markBarBtn.clicked() } },
        { l: "调整颜色… (Ctrl+E)", ok: bridge.image_width > 0,
          run: function () { host.toggleAdjustPanel() } },
        { l: "尺子（测量坐标）", ok: bridge.image_width > 0,
          run: function () { host.toggleRuler() } },
        { l: "拾色器…", ok: bridge.image_width > 0,
          run: function () { host.togglePickerPanel() } },
        { l: "", ok: true, sep: true },
        { l: "放大 (Ctrl++)", ok: true, run: function () { view.queueZoom(1.25) } },
        { l: "缩小 (Ctrl+-)", ok: true, run: function () { view.queueZoom(1 / 1.25) } },
        { l: "适应窗口 (Ctrl+9)", ok: bridge.image_width > 0, run: function () { bridge.zoom_fit() } },
        { l: "100% 实际大小 (Ctrl+0)", ok: bridge.image_width > 0, run: function () { bridge.zoom_actual() } },
        { l: "", ok: true, sep: true },
        { l: "保存 (Ctrl+S)", ok: bridge.image_width > 0, run: function () { host.saveOrSaveAs() } },
        { l: "另存为… (Ctrl+Shift+S)", ok: true, run: function () { saveDialog.open() } },
        { l: "", ok: true, sep: true },
        { l: "设置…", ok: true, run: function () { settingsOverlay.visible = true } },
        { l: "", ok: true, sep: true },
        { l: "关闭窗口 (Ctrl+W)", ok: true, run: function () { host.requestQuit() } }
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
        text: bridge.image_width > 0
              ? "ImageEdit — " + bridge.image_width + "×" + bridge.image_height
                + (bridge.dirty ? " *" : "")
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
        selected: host.markBarVisible
        onClicked: {
            host.markBarVisible = !host.markBarVisible
            if (!host.markBarVisible) {
                host.markTool = "none"
                // 标记选中态保留：标记与图层统一交互，不依赖工具栏。
                widthPopover.visible = false
                colorPopover.visible = false
                mosaicPopover.visible = false
                fontPopover.visible = false
                textPopup.close(false)
                // 调色面板是从这条工具栏打开的：关栏 = 应用并关闭。
                if (bridge.adjustOpen)
                    host.commitAdjust()
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
        selected: host.panelVisible
        visible: true
        onClicked: host.panelVisible = !host.panelVisible
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

