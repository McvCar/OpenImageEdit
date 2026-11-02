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

    // 「⋯」菜单的内容：一级只放分类，命令按类收进二级面板（OverflowMenu
    // 负责展开）。写成函数而不是 property，是为了每次打开都重新求值
    // ok（can_undo / selExists 这些），否则菜单状态会冻在启动那一刻。
    function menuTree() {
        return [
            { l: "文件", sub: [
                { l: "打开图片… (Ctrl+O)", ok: true,
                  run: function () { openDialog.open() } },
                { l: "新窗口 (Ctrl+N)", ok: true,
                  run: function () { host.requestNewWindow() } },
                { l: "", sep: true },
                { l: "保存 (Ctrl+S)", ok: bridge.image_width > 0,
                  run: function () { host.saveOrSaveAs() } },
                { l: "另存为… (Ctrl+Shift+S)", ok: true,
                  run: function () { saveDialog.open() } },
                { l: "", sep: true },
                { l: "关闭窗口 (Ctrl+W)", ok: true,
                  run: function () { host.requestQuit() } }
            ] },
            { l: "编辑", sub: [
                { l: "撤销 (Ctrl+Z)", ok: bridge.can_undo,
                  run: function () { bridge.undo() } },
                { l: "重做 (Ctrl+Shift+Z)", ok: bridge.can_redo,
                  run: function () { bridge.redo() } },
                { l: "", sep: true },
                { l: "删除选区像素 (Delete)", ok: host.selExists,
                  run: function () { host.deleteSelectionPixels() } },
                { l: "提取图层", ok: host.selExists,
                  run: function () { host.commitExtract() } }
            ] },
            { l: "图像", sub: [
                { l: "逆时针旋转 90° (Ctrl+L)", ok: true,
                  run: function () { bridge.rotate_ccw() } },
                { l: "顺时针旋转 90° (Ctrl+R)", ok: true,
                  run: function () { bridge.rotate_cw() } },
                { l: "水平翻转", ok: true,
                  run: function () { bridge.flip_horizontal() } },
                { l: "垂直翻转", ok: true,
                  run: function () { bridge.flip_vertical() } },
                { l: "", sep: true },
                { l: "应用裁剪 (Enter / Ctrl+K)", ok: host.selExists,
                  run: function () { host.applyCrop() } },
                { l: "重置裁剪", ok: bridge.image_width > 0,
                  run: function () { bridge.reset_crop() } },
                { l: "调整尺寸…", ok: bridge.image_width > 0,
                  run: function () { resizeOverlay.openResizeDialog() } }
            ] },
            { l: "选区", sub: [
                { l: "选区工具…（M / L）", ok: true,
                  run: function () { host.markBarVisible = true
                                     host.selPopoverOpen = !host.selPopoverOpen } },
                { l: "全选 (Ctrl+A)", ok: bridge.image_width > 0,
                  run: function () { bridge.sel_select_all() } },
                { l: "选择反选 (Ctrl+Shift+I)", ok: bridge.image_width > 0,
                  run: function () { bridge.sel_invert() } },
                { l: "取消选择 (Ctrl+D)", ok: host.selExists,
                  run: function () { bridge.sel_clear() } },
                { l: "快速蒙版 (Q)", ok: host.selExists,
                  run: function () { bridge.sel_toggle_quick_mask() } }
            ] },
            { l: "图层与标记", sub: [
                { l: "图层面板", ok: true,
                  run: function () { host.panelVisible = !host.panelVisible } },
                { l: "标记工具栏", ok: true,
                  run: function () { markBarBtn.clicked() } },
                { l: "调整颜色… (Ctrl+E)", ok: bridge.image_width > 0,
                  run: function () { host.toggleAdjustPanel() } },
                { l: "尺子（测量坐标）", ok: bridge.image_width > 0,
                  run: function () { host.toggleRuler() } },
                { l: "拾色器…", ok: bridge.image_width > 0,
                  run: function () { host.togglePickerPanel() } }
            ] },
            { l: "视图", sub: [
                { l: "放大 (Ctrl++)", ok: true,
                  run: function () { view.queueZoom(1.25) } },
                { l: "缩小 (Ctrl+-)", ok: true,
                  run: function () { view.queueZoom(1 / 1.25) } },
                { l: "", sep: true },
                { l: "适应窗口 (Ctrl+9)", ok: bridge.image_width > 0,
                  run: function () { bridge.zoom_fit() } },
                { l: "100% 实际大小 (Ctrl+0)", ok: bridge.image_width > 0,
                  run: function () { bridge.zoom_actual() } }
            ] },
            { l: "", sep: true },
            { l: "设置…", ok: true,
              run: function () { settingsOverlay.visible = true } }
        ]
    }

    // ---- 图标栏容量（纯绑定：窗口宽度 / 标题宽度 / 右侧按钮簇宽度变化都
    // 会自动重算）--------------------------------------------------------
    // 之前是命令式的 updateCapacity()（只在 Component.onCompleted 和
    // onWidthChanged 里跑）。窗口若在布局落地前就被定成最终宽度（新窗口按
    // 图片尺寸打开、创建时宽度还是 0），capacity 会一直停在 0 —— 整排图标
    // 消失但分隔线还在，正是"窄窗口下工具栏全被收起来"的那个状态。
    // 右侧固定簇：窗口按钮 + ⋯ + 图层面板 + 标记栏（含各自的间距/边距）。
    readonly property real rightCluster: markBarBtn.width + 2
                                         + layersBtn.width + 2
                                         + menuBtn.width + 6 + winRow.width + 4
    // 一行里能塞下多少个图标槽（30px 按钮 + 2px 间距，按 32 算）。
    function slotsFor(titleWidth) {
        const left = titleWidth > 0 ? titleWidth + 20 : 8
        const avail = Math.max(0, width - rightCluster - 8 - left)
        return Math.min(barActions.length, Math.floor(avail / 32))
    }
    // 标题文字（全称 / 只留应用名）——测量只用下面两个隐藏 Text，
    // 不能让 titleText 自己的 implicitWidth 参与判定，否则"显示→占地方→
    // 隐藏→有地方→显示"会自激振荡。
    readonly property string titleFullText: bridge.image_width > 0
        ? "ImageEdit — " + bridge.image_width + "×" + bridge.image_height
          + (bridge.dirty ? " *" : "")
        : "ImageEdit"
    readonly property string titleNameText: "ImageEdit" + (bridge.dirty ? " *" : "")
    readonly property int capacityFull: slotsFor(titleProbeFull.implicitWidth)
    readonly property int capacityName: slotsFor(titleProbeName.implicitWidth)
    // 空间优先给按钮：
    //   "full" 还不算挤（按钮没被砍掉三分之一）→ 标题带上分辨率；
    //   "name" 开始挤了 → 先让出分辨率（状态栏里还有一份）；
    //   "none" 连一个按钮都放不下 → 整块标题让出去。
    readonly property int roomySlots: Math.min(barActions.length, 8)
    readonly property string titleMode: capacityFull >= roomySlots ? "full"
                                      : (capacityName >= 1 ? "name" : "none")
    readonly property int capacity: titleMode === "full" ? capacityFull
                                   : (titleMode === "name" ? capacityName : slotsFor(0))

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

    // 标题文字用的两个"量尺"：不可见，只提供 implicitWidth 给上面判定。
    Text {
        id: titleProbeFull
        visible: false
        font.pixelSize: 12
        text: toolbar.titleFullText
    }
    Text {
        id: titleProbeName
        visible: false
        font.pixelSize: 12
        text: toolbar.titleNameText
    }

    Text {
        id: titleText
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        width: Math.min(implicitWidth, 220)
        elide: Text.ElideRight
        // 窄窗口下先丢分辨率、再整块让给图标栏（见 titleMode 的注释）。
        visible: toolbar.titleMode !== "none"
        // Same content as the (offscreen) window title: unsaved changes
        // are flagged with a trailing asterisk.
        text: toolbar.titleMode === "full" ? toolbar.titleFullText : toolbar.titleNameText
        color: "#c9cdd2"
        font.pixelSize: 12
    }

    Row {
        id: iconRow
        // 标题隐藏时图标从左边距开始，不留标题那一段空白。
        anchors.left: toolbar.titleMode !== "none" ? titleText.right : parent.left
        anchors.leftMargin: toolbar.titleMode !== "none" ? 10 : 8
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
        onClicked: overflowMenu.toggle()
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

