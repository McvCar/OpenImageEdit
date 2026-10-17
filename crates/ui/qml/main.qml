import QtQuick
import QtQuick.Window
import QtQuick.Dialogs
import com.imageedit

Window {
    id: shell

    width: 1100
    height: 760
    minimumWidth: 480
    minimumHeight: 320
    visible: Qt.application.arguments.indexOf("--hidden") === -1
    color: "#1b1c1e"
    visibility: content.fullscreen ? Window.FullScreen : Window.Windowed
    title: content.b.image_width > 0
           ? "ImageEdit — " + content.b.image_width + "×" + content.b.image_height
           : "ImageEdit"

    onAfterRendering: content.afterFrame()
    onVisibleChanged: content.b.mark("window-visible")
    onClosing: (close) => content.requestClose(close)

    EditorContent {
        id: content
        anchors.fill: parent
        primary: true
        onRequestNewWindow: openNewWindow()
    }

    property int windowCascade: 0

    function openNewWindow() {
        const offset = shell.windowCascade * 32
        shell.windowCascade = (shell.windowCascade + 1) % 8
        docWindowFactory.createObject(null, { x: 80 + offset, y: 60 + offset, newWindowFn: openNewWindow })
    }

    Component {
        id: docWindowFactory
        DocWindow {}
    }

    component ToolButton: Rectangle {
        id: button

        property string label
        property bool active: true
        property bool selected: false
        signal clicked()

        width: Math.max(52, caption.implicitWidth + 16)
        height: 26
        radius: 4
        color: button.selected
               ? "#3d5a80"
               : (mouse.containsMouse && button.active ? "#4a4e54" : "#33373c")
        opacity: button.active ? 1.0 : 0.35

        Text {
            id: caption
            anchors.centerIn: parent
            text: button.label
            color: "#e9eaec"
            font.pixelSize: 12
        }

        MouseArea {
            id: mouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: if (button.active) button.clicked()
        }
    }

    // Dashed rectangle outline (selection / layer boxes). rx/ry/rw/rh are in
    // this item's coordinates; parent is sized to the preview area.
    component DashedRect: Canvas {
        id: dashed
        property real rx: 0
        property real ry: 0
        property real rw: 0
        property real rh: 0
        property color dashColor: "#5b9dff"
        onRxChanged: requestPaint()
        onRyChanged: requestPaint()
        onRwChanged: requestPaint()
        onRhChanged: requestPaint()
        onDashColorChanged: requestPaint()
        onPaint: {
            var ctx = getContext("2d")
            ctx.clearRect(0, 0, width, height)
            if (rw < 1 || rh < 1)
                return
            ctx.strokeStyle = String(dashColor)
            ctx.lineWidth = 1.5
            ctx.setLineDash([5, 4])
            ctx.strokeRect(rx + 0.75, ry + 0.75, rw - 1.5, rh - 1.5)
        }
    }

    // 8 resize handles (4 corners + 4 edge midpoints) for the crop / selection
    // boxes. Geometry callbacks live in the enclosing scope and are injected
    // at the instantiation site; all coordinates snap to image pixels.
    component EdgeHandles: Item {
        id: handles

        property real bx: 0
        property real by: 0
        property real bw: 0
        property real bh: 0
        property real pxScale: 1
        property var resizeFn: null
        property var moveFn: null
        property bool moveEnabled: true
        // True while any handle (or the center pad) is being dragged.
        property bool engaged: false

        Repeater {
            model: ["nw", "n", "ne", "e", "se", "s", "sw", "w"]

            delegate: Rectangle {
                readonly property string role: modelData
                readonly property bool corner: role.length === 2
                // Blue-dot handle (selected state marker), white-ringed; the
                // hit area extends past the visible dot for easy grabbing.
                width: hma.containsMouse || hma.pressed ? 12 : 10
                height: width
                radius: width / 2
                color: hma.pressed ? "#7fc3ff" : "#5b9dff"
                border.color: "#ffffff"
                border.width: 1.5
                z: 2

                x: handles.bx
                   + (role.indexOf("e") >= 0 ? handles.bw
                      : (role === "n" || role === "s" ? handles.bw / 2 : 0))
                   - width / 2
                y: handles.by
                   + (role.indexOf("s") >= 0 ? handles.bh
                      : (role === "w" || role === "e" ? handles.bh / 2 : 0))
                   - height / 2

                MouseArea {
                    id: hma
                    anchors.fill: parent
                    anchors.margins: -4
                    hoverEnabled: true
                    cursorShape: role === "n" || role === "s" ? Qt.SizeVerCursor
                               : role === "e" || role === "w" ? Qt.SizeHorCursor
                               : (role === "nw" || role === "se") ? Qt.SizeFDiagCursor
                               : Qt.SizeBDiagCursor
                    property rect startRect
                    property real px: 0
                    property real py: 0

                    // Deltas MUST be measured in a stable coordinate frame:
                    // the handle itself moves while dragging, so its local
                    // mouse coords feed back into resizeFn and the box
                    // oscillates (visible jitter). Map to the handles item,
                    // which never moves during a drag.
                    onPressed: (mouse) => {
                        const p = mapToItem(handles, mouse.x, mouse.y)
                        px = p.x
                        py = p.y
                        startRect = Qt.rect(handles.bx, handles.by, handles.bw, handles.bh)
                        handles.engaged = true
                    }
                    onPositionChanged: (mouse) => {
                        if (pressed && handles.resizeFn) {
                            const p = mapToItem(handles, mouse.x, mouse.y)
                            handles.resizeFn(startRect, role, p.x - px, p.y - py,
                                             (mouse.modifiers & Qt.ShiftModifier) !== 0)
                        }
                    }
                    onReleased: handles.engaged = false
                }
            }
        }

        MouseArea {
            id: centerMa
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.SizeAllCursor
            enabled: handles.moveEnabled && handles.moveFn !== null
                     && handles.bw > 0 && handles.bh > 0
                     && centerMa.mouseX > handles.bx && centerMa.mouseX < handles.bx + handles.bw
                     && centerMa.mouseY > handles.by && centerMa.mouseY < handles.by + handles.bh
            property rect startRect
            property real px: 0
            property real py: 0

            onPressed: (mouse) => {
                px = mouse.x
                py = mouse.y
                startRect = Qt.rect(handles.bx, handles.by, handles.bw, handles.bh)
                handles.engaged = true
            }
            onPositionChanged: (mouse) => {
                if (pressed && handles.moveFn)
                    handles.moveFn(startRect, mouse.x - px, mouse.y - py)
            }
            onReleased: handles.engaged = false
        }
    }

    // A secondary document window: same editor content, its own Bridge
    // instance, no CLI handling (that stays with the primary window).
    // newWindowFn is injected by the shell factory so the inline component
    // never needs to reference the shell (which would create a cycle).
    component DocWindow: Window {
        id: docWin

        property var newWindowFn: null

        width: 1100
        height: 760
        minimumWidth: 480
        minimumHeight: 320
        color: "#1b1c1e"
        visible: true
        visibility: content.fullscreen ? Window.FullScreen : Window.Windowed
        title: content.b.image_width > 0
               ? "ImageEdit — " + content.b.image_width + "×" + content.b.image_height
               : "ImageEdit"

        onAfterRendering: content.afterFrame()
        onVisibleChanged: content.b.mark("window-visible")
        onClosing: (close) => content.requestClose(close)

        EditorContent {
            id: content
            anchors.fill: parent
            onRequestNewWindow: if (docWin.newWindowFn) docWin.newWindowFn()
        }
    }

    component EditorContent: Item {
        id: root

        property bool primary: false
        property bool forceClose: false
        // Set right before Qt.quit() (selftest flow): in Qt 6.8 quitting
        // delivers close events to visible windows, and the dirty-guard in
        // requestClose() must not reject an intentional programmatic quit.
        property bool quitting: false
        property var pendingOpenUrl: null
        property string confirmMode: ""
        signal requestNewWindow()
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
        readonly property var selLayer: findSelLayer()
        readonly property var selLayerBox: findLayerBox()
        readonly property bool probeDialog: Qt.application.arguments.indexOf("--probe-dialog") !== -1
        // Ids inside an inline component are invisible from outside, so the
        // shell / DocWindow wrappers reach the Bridge through this alias.
        readonly property alias b: bridge

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

    FileDialog {
        id: openDialog
        title: "打开图片"
        nameFilters: [
            "图片文件 (*.jpg *.jpeg *.png *.gif *.bmp *.tif *.tiff *.webp)",
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
        nameFilters: ["PNG (*.png)", "JPEG (*.jpg)", "WebP (*.webp)", "TIFF (*.tiff)", "BMP (*.bmp)"]
        onAccepted: bridge.save_as(selectedFile)
    }

    Shortcut { sequences: [StandardKey.Open]; onActivated: openDialog.open() }
    Shortcut { sequences: [StandardKey.New]; onActivated: root.requestNewWindow() }
    Shortcut { sequences: [StandardKey.SaveAs]; onActivated: saveDialog.open() }
    Shortcut { sequence: "F11"; onActivated: root.fullscreen = !root.fullscreen }
    Shortcut { sequence: "Escape"; onActivated: onEscape() }
    Shortcut { sequences: ["Return", "Enter"]
               enabled: root.pasteFloating || root.selExists
               onActivated: onEnter() }

    Shortcut { sequence: "Left";  enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(-1, 0) }
    Shortcut { sequence: "Right"; enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(1, 0) }
    Shortcut { sequence: "Up";    enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(0, -1) }
    Shortcut { sequence: "Down";  enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(0, 1) }
    Shortcut { sequence: "Shift+Left";  enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(-10, 0) }
    Shortcut { sequence: "Shift+Right"; enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(10, 0) }
    Shortcut { sequence: "Shift+Up";    enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(0, -10) }
    Shortcut { sequence: "Shift+Down";  enabled: bridge.selected_layer >= 0; onActivated: nudgeLayer(0, 10) }

    Shortcut { sequences: [StandardKey.ZoomIn, "Ctrl+="]; onActivated: bridge.zoom_in() }
    Shortcut { sequences: [StandardKey.ZoomOut]; onActivated: bridge.zoom_out() }
    // Preview-aligned: ⌘0 actual size, ⌘9 zoom-to-fit (old keys kept as aliases).
    Shortcut { sequences: ["Ctrl+0", "Ctrl+1"]; onActivated: bridge.zoom_actual() }
    Shortcut { sequences: ["Ctrl+9"]; onActivated: bridge.zoom_fit() }
    Shortcut { sequences: [StandardKey.Undo]; onActivated: bridge.undo() }
    // Redo: explicit keys — StandardKey.Redo maps inconsistently across
    // platforms/layouts, plain strings fire reliably (Ctrl+Y and ⇧Ctrl+Z).
    Shortcut { sequences: ["Ctrl+Shift+Z", "Ctrl+Y"]; onActivated: bridge.redo() }
    Shortcut { sequences: [StandardKey.Save]; onActivated: bridge.save_edited() }
    Shortcut { sequences: [StandardKey.Copy]; onActivated: doCopy() }
    Shortcut { sequences: [StandardKey.Cut]; onActivated: doCut() }
    Shortcut { sequences: [StandardKey.Paste]; onActivated: doPaste() }
    Shortcut { sequence: "Ctrl+K"; enabled: root.selExists; onActivated: applyCrop() }
    Shortcut { sequence: "Ctrl+L"; onActivated: bridge.rotate_ccw() }
    Shortcut { sequence: "Ctrl+Shift+R"; onActivated: bridge.rotate_ccw() }
    Shortcut { sequences: ["Delete", "Backspace"]
               enabled: root.selExists || bridge.selected_layer >= 0
                        || (function () {
                            const b = findBase()
                            return b !== null && b.v === true && !isLocked(b)
                                && bridge.layer_count > 0
                        })()
               onActivated: onDeleteKey() }
    Shortcut { sequence: "Ctrl+A"
               enabled: bridge.image_width > 0 && root.renamingIndex < 0
               onActivated: selectAllCanvas() }
    // ⌘W / ⌘Q: close window / quit, both through the dirty-confirm guard.
    Shortcut { sequence: "Ctrl+W"; onActivated: requestQuit() }
    Shortcut { sequence: "Ctrl+Q"; onActivated: requestQuit() }
    Shortcut { sequence: "Ctrl+R"; onActivated: bridge.rotate_cw() }

    Rectangle {
        id: toolbar
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: root.toolbarHeight
        color: "#242628"

        Row {
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left
            anchors.leftMargin: 8
            spacing: 6

            ToolButton { label: "打开"; onClicked: openDialog.open() }
            ToolButton { label: "新窗口"; onClicked: root.requestNewWindow() }

            Rectangle { width: 1; height: 20; color: "#3c4043" }

            ToolButton { label: "撤销"; active: bridge.can_undo; onClicked: bridge.undo() }
            ToolButton { label: "重做"; active: bridge.can_redo; onClicked: bridge.redo() }

            Rectangle { width: 1; height: 20; color: "#3c4043" }

            ToolButton { label: "左转 90°"; onClicked: bridge.rotate_ccw() }
            ToolButton { label: "右转 90°"; onClicked: bridge.rotate_cw() }
            ToolButton { label: "水平翻转"; onClicked: bridge.flip_horizontal() }
            ToolButton { label: "垂直翻转"; onClicked: bridge.flip_vertical() }

            Rectangle { width: 1; height: 20; color: "#3c4043" }

            ToolButton {
                label: "裁剪"
                active: root.selExists
                onClicked: applyCrop()
            }
            ToolButton { label: "重置裁剪"; active: bridge.image_width > 0; onClicked: bridge.reset_crop() }
            ToolButton { label: "调整尺寸…"; active: bridge.image_width > 0; onClicked: openResizeDialog() }

            Rectangle { width: 1; height: 20; color: "#3c4043" }

            ToolButton { label: "提取图层"; active: root.selExists; onClicked: commitExtract() }
            ToolButton { label: "删除选区像素"; active: root.selExists; onClicked: deleteSelectionPixels() }
            ToolButton {
                label: "图层"
                active: true
                selected: root.panelVisible
                onClicked: root.panelVisible = !root.panelVisible
            }

            Rectangle { width: 1; height: 20; color: "#3c4043" }

            ToolButton { label: "-"; onClicked: bridge.zoom_out() }
            ToolButton { label: "+"; onClicked: bridge.zoom_in() }
            ToolButton { label: "适应窗口"; active: !bridge.fit_mode; onClicked: bridge.zoom_fit() }
            ToolButton { label: "100%"; active: bridge.zoom_percent !== 100; onClicked: bridge.zoom_actual() }

            Rectangle { width: 1; height: 20; color: "#3c4043" }

            ToolButton { label: "另存副本"; onClicked: bridge.save_edited() }
            ToolButton { label: "另存为…"; onClicked: saveDialog.open() }
        }
    }

    Flickable {
        id: view

        anchors.top: toolbar.bottom
        anchors.bottom: statusBar.top
        anchors.left: parent.left
        anchors.right: parent.right
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        // PAD display px of slack around the image so layers pushed partially
        // off-canvas stay visible and clickable (canvasSurface matches).
        readonly property int pad: 64
        contentWidth: Math.max(width, previewImage.width + 2 * pad)
        contentHeight: Math.max(height, previewImage.height + 2 * pad)

        onWidthChanged: bridge.set_viewport(width, height)
        onHeightChanged: bridge.set_viewport(width, height)

        // PS-style checkerboard behind the image: transparent pixels show
        // the classic gray/white grid.
        Canvas {
            id: checkerboard
            visible: bridge.preview_url.length > 0
            x: previewImage.x
            y: previewImage.y
            width: previewImage.width
            height: previewImage.height
            onWidthChanged: requestPaint()
            onHeightChanged: requestPaint()
            onPaint: {
                var ctx = getContext("2d")
                ctx.clearRect(0, 0, width, height)
                const cell = 8
                ctx.fillStyle = "#cccccc"
                ctx.fillRect(0, 0, width, height)
                ctx.fillStyle = "#ffffff"
                for (var gy = 0; gy < height; gy += cell) {
                    for (var gx = 0; gx < width; gx += cell) {
                        if (((gx / cell) | 0) % 2 === ((gy / cell) | 0) % 2)
                            ctx.fillRect(gx, gy, cell, cell)
                    }
                }
            }
        }

        Image {
            id: previewImage

            width: Math.max(1, bridge.display_width)
            height: Math.max(1, bridge.display_height)
            x: (view.contentWidth - width) / 2
            y: (view.contentHeight - height) / 2
            cache: false
            fillMode: Image.Stretch
            source: bridge.preview_url

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
                cursorShape: {
                    if (!root.selExists && bridge.image_width > 0) {
                        const hit = topLayerAt(selDragMouse.mouseX, selDragMouse.mouseY)
                        if (hit !== null && !isLocked(hit))
                            return Qt.SizeAllCursor
                    }
                    return Qt.CrossCursor
                }
                property real startX: 0
                property real startY: 0
                property real curX: 0
                property real curY: 0
                property bool dragging: false
                // Layer pick-and-move gesture (no selection active).
                property bool pickDrag: false
                property bool pickMoved: false

                onPressed: (mouse) => {
                    const hit = root.selExists ? null : topLayerAt(mouse.x, mouse.y)
                    if (hit !== null) {
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
                    if (pickDrag) {
                        curX = mouse.x
                        curY = mouse.y
                        if (!pickMoved
                            && Math.abs(curX - startX) + Math.abs(curY - startY) > 3) {
                            pickMoved = true
                            bridge.drag_translate_begin()
                        }
                        if (pickMoved)
                            liveTimer.restart()
                        return
                    }
                    if (!dragging)
                        return
                    curX = mouse.x
                    curY = mouse.y
                    const x = Math.min(startX, curX) - view.pad
                    const y = Math.min(startY, curY) - view.pad
                    root.selX = snapDisplay(x)
                    root.selY = snapDisplay(y)
                    root.selW = Math.max(snapDisplay(Math.abs(curX - startX)), toDispScale())
                    root.selH = Math.max(snapDisplay(Math.abs(curY - startY)), toDispScale())
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

            Timer {
                id: liveTimer
                interval: 33
                onTriggered: {
                    if (!selDragMouse.pickMoved)
                        return
                    const scale = toImageScale()
                    bridge.drag_translate_update(
                        Math.round((selDragMouse.curX - selDragMouse.startX) * scale),
                        Math.round((selDragMouse.curY - selDragMouse.startY) * scale))
                }
            }

            // Live scale preview throttle for the selected layer's handles.
            Timer {
                id: scaleTimer
                interval: 60
                onTriggered: {
                    if (root.scaleW <= 0)
                        return
                    const s = toDispScale()
                    bridge.scale_layer_update(
                        Math.max(1, Math.round(root.scaleW / s)),
                        Math.max(1, Math.round(root.scaleH / s)),
                        Math.round(root.scaleX / s), Math.round(root.scaleY / s))
                }
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
            dashColor: "#5b9dff"
        }

        Text {
            visible: root.selExists
            z: 8
            x: previewImage.x + root.selX
            y: Math.min(previewImage.y + previewImage.height - 16,
                        previewImage.y + root.selY + root.selH + 4)
            text: Math.round(root.selW / toDispScale()) + " × "
                  + Math.round(root.selH / toDispScale())
                  + (root.pasteFloating ? "" : "　Enter 应用裁剪 · Esc 取消")
            color: "#7fc3ff"
            font.pixelSize: 11
        }

        EdgeHandles {
            id: selHandles
            visible: root.selExists && !root.selRubberBanding && bridge.image_width > 0
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
            visible: root.selLayer !== null && root.selLayer.base !== true
                     && !root.selExists && bridge.image_width > 0
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

        WheelHandler {
            enabled: !root.modeActive
            onWheel: (event) => {
                if (event.angleDelta.y > 0)
                    bridge.zoom_in();
                else if (event.angleDelta.y < 0)
                    bridge.zoom_out();
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
            anchors.right: zoomLabel.left
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

        Text {
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left
            anchors.leftMargin: 10
            anchors.right: (root.modeActive || root.selExists) ? modeHint.left : zoomLabel.left
            anchors.rightMargin: 10
            elide: Text.ElideMiddle
            text: bridge.status
            color: "#b8bcc0"
            font.pixelSize: 12
        }

        Text {
            id: zoomLabel
            anchors.verticalCenter: parent.verticalCenter
            anchors.right: parent.right
            anchors.rightMargin: 10
            text: bridge.image_width > 0
                  ? bridge.image_width + " × " + bridge.image_height + "   " + bridge.zoom_percent + "%"
                  : ""
            color: "#b8bcc0"
            font.pixelSize: 12
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
                text: "图层（最上 = 顶层）"
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
                    model: root.layerList

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
                        cursorShape: layerRow.isBase ? Qt.ArrowCursor : Qt.PointingHandCursor
                        property bool reordering: false
                        property int fromPos: -1
                        property int toPos: -1
                        property real pressY: 0

                        onPressed: (mouse) => {
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
                            const docLen = root.layerList.length - 1  // exclude base row
                            const colY = layerRow.mapToItem(layerRepeaterColumn, 0, mouse.y).y
                            let t = Math.floor(colY / pitch + 0.5)
                            t = Math.max(0, Math.min(t, docLen - 1))
                            toPos = t
                            dropIndicator.y = t * pitch - 2
                            dropIndicator.visible = t !== fromPos
                        }
                        onReleased: {
                            if (reordering && toPos >= 0 && toPos !== fromPos) {
                                const docLen = root.layerList.length - 1
                                // Display position p ↔ doc index docLen-1-p.
                                bridge.reorder_layer(docLen - 1 - fromPos,
                                                     docLen - 1 - toPos)
                            } else if (!reordering && layerRow.row && !layerRow.isBase) {
                                // Selecting a layer takes over the canvas:
                                // drop any active selection so the layer's
                                // handles and drag become usable at once.
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
                            readonly property bool disabled: layerRow.isBase && layerRow.row && layerRow.row.v !== true
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
                            enabled: !(layerRow.isBase && layerRow.row && layerRow.row.v !== true)
                            cursorShape: Qt.PointingHandCursor
                            onClicked: if (layerRow.row)
                                bridge.set_layer_visible_at(layerRow.row.i, !layerRow.row.v)
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
                                + (layerRow.isBase
                                   ? (layerRow.row.l === true ? "（背景）" : "（已解锁，Delete 删除）")
                                   : "")
                                + (layerRow.row.o !== undefined && layerRow.row.o < 100 ? "  " + layerRow.row.o + "%" : "")
                                + (isSel ? "  (选中)" : "")
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
    }

    Item {
        id: resizeOverlay
        anchors.fill: parent
        visible: false
        z: 100
        property bool keepRatio: false

        MouseArea {
            anchors.fill: parent
            onClicked: resizeOverlay.visible = false
        }

        Rectangle {
            id: resizeDialog
            width: 320
            height: 210
            x: (parent.width - width) / 2
            y: (parent.height - height) / 2
            radius: 6
            color: "#2b2e33"
            border.color: "#3c4043"

            Column {
                anchors.centerIn: parent
                spacing: 12

                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "调整尺寸"
                    color: "#e9eaec"
                    font.pixelSize: 14
                    font.bold: true
                }

                Row {
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: 6

                    Text {
                        text: "宽"
                        color: "#b8bcc0"
                        font.pixelSize: 12
                        anchors.verticalCenter: parent.verticalCenter
                    }

                    Item {
                        width: 80
                        height: 26
                        anchors.verticalCenter: parent.verticalCenter

                        Rectangle {
                            anchors.fill: parent
                            color: "#1b1c1e"
                            border.color: "#3c4043"
                            radius: 3
                        }

                        TextInput {
                            id: resizeW
                            anchors.fill: parent
                            text: ""
                            font.pixelSize: 13
                            color: "#e9eaec"
                            onTextChanged: if (resizeOverlay.keepRatio)
                                resizeH.text = linkedHeight()
                            onAccepted: commitResize()
                        }
                    }

                    Text {
                        text: "高"
                        color: "#b8bcc0"
                        font.pixelSize: 12
                        anchors.verticalCenter: parent.verticalCenter
                    }

                    Item {
                        width: 80
                        height: 26
                        anchors.verticalCenter: parent.verticalCenter

                        Rectangle {
                            anchors.fill: parent
                            color: "#1b1c1e"
                            border.color: "#3c4043"
                            radius: 3
                        }

                        TextInput {
                            id: resizeH
                            anchors.fill: parent
                            text: ""
                            font.pixelSize: 13
                            color: "#e9eaec"
                            onTextChanged: if (resizeOverlay.keepRatio)
                                resizeW.text = linkedWidth()
                            onAccepted: commitResize()
                        }
                    }

                    ToolButton {
                        label: "保持比例" + (resizeOverlay.keepRatio ? " [x]" : "")
                        active: true
                        anchors.verticalCenter: parent.verticalCenter
                        onClicked: resizeOverlay.keepRatio = !resizeOverlay.keepRatio
                    }
                }

                Row {
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: 12
                    ToolButton { label: "确定"; active: true; onClicked: commitResize() }
                    ToolButton { label: "取消"; active: true; onClicked: resizeOverlay.visible = false }
                }

                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "当前：" + bridge.image_width + " × " + bridge.image_height
                    color: "#6b7076"
                    font.pixelSize: 11
                }
            }
        }
    }

    function toImageScale() {
        return previewImage.width > 0 ? bridge.image_width / previewImage.width : 1
    }

    function toDispScale() {
        return bridge.image_width > 0 && previewImage.width > 0
               ? previewImage.width / bridge.image_width : 0
    }

    // Snap a display coordinate to the nearest image-pixel boundary so every
    // crop / selection edge maps to an exact integer image coordinate.
    function snapDisplay(v) {
        const s = toDispScale()
        return s > 0 ? Math.round(v / s) * s : v
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
        scaleTimer.restart()
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

    // Delete / Backspace: with a selection → erase selection pixels;
    // otherwise delete the selected unlocked layer; with nothing selected and
    // an unlocked (deleted-pending) base → remove the background.
    function onDeleteKey() {
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
        if (base !== null && base.v === true && !isLocked(base)
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
        bridge.paste_clipboard()
        root.pasteFloating = true
        root.floatingOps = 1
    }

    function settlePaste() {
        root.pasteFloating = false
        root.floatingOps = 0
    }

    function linkedHeight() {
        const w = parseInt(resizeW.text)
        if (isNaN(w) || w < 1)
            return ""
        const oldW = Math.max(1, bridge.image_width)
        const oldH = Math.max(1, bridge.image_height)
        return String(Math.max(1, Math.round(w * oldH / oldW)))
    }

    function linkedWidth() {
        const h = parseInt(resizeH.text)
        if (isNaN(h) || h < 1)
            return ""
        const oldW = Math.max(1, bridge.image_width)
        const oldH = Math.max(1, bridge.image_height)
        return String(Math.max(1, Math.round(h * oldW / oldH)))
    }

    function openResizeDialog() {
        resizeW.text = String(bridge.image_width)
        resizeH.text = String(bridge.image_height)
        resizeOverlay.keepRatio = false
        resizeOverlay.visible = true
    }

    function commitResize() {
        let w = parseInt(resizeW.text)
        let h = parseInt(resizeH.text)
        if (resizeOverlay.keepRatio) {
            if (isNaN(h))
                h = parseInt(linkedHeight())
            if (isNaN(w))
                w = parseInt(linkedWidth())
        }
        if (isNaN(w) || w < 1 || isNaN(h) || h < 1)
            return
        bridge.resize_to(w, h)
        resizeOverlay.visible = false
    }

    function onEscape() {
        if (root.renamingIndex >= 0) { root.renamingIndex = -1; return }
        if (confirmOverlay.visible) { confirmCancel(); return }
        if (root.scaleW > 0) { cancelLayerScale(); return }
        if (selDragMouse.dragging) { selDragMouse.dragging = false; return }
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

    function nudgeLayer(dx, dy) {
        if (bridge.selected_layer < 0)
            return
        bridge.translate_selected(dx, dy)
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

    // Topmost visible non-base layer whose content box contains the
    // canvasSurface-space point (mx, my); null when the point is empty
    // canvas. layerList is display-ordered (index 0 = top), so the first
    // hit wins. Hidden layers and the base row are not pickable.
    function topLayerAt(mx, my) {
        const s = toDispScale()
        if (s <= 0)
            return null
        const ix = (mx - view.pad) / s
        const iy = (my - view.pad) / s
        for (var k = 0; k < root.layerList.length; k++) {
            const l = root.layerList[k]
            if (l.base === true || l.v !== true)
                continue
            if (l.w > 0 && l.h > 0
                && ix >= l.x && ix <= l.x + l.w && iy >= l.y && iy <= l.y + l.h)
                return l
        }
        return null
    }

    Item {
        id: confirmOverlay
        anchors.fill: parent
        visible: false
        z: 200

        MouseArea { anchors.fill: parent }

        Rectangle {
            width: 360
            height: 168
            anchors.centerIn: parent
            radius: 6
            color: "#2b2e33"
            border.color: "#3c4043"

            Column {
                anchors.centerIn: parent
                spacing: 14
                width: parent.width - 32

                Text {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    text: "当前文档有未保存的修改"
                    color: "#e9eaec"
                    font.pixelSize: 14
                    font.bold: true
                }

                Text {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    text: root.confirmMode === "open"
                          ? "打开新文件将放弃这些修改，是否继续？"
                          : "关闭窗口将放弃这些修改，是否继续？"
                    color: "#b8bcc0"
                    font.pixelSize: 12
                    wrapMode: Text.WordWrap
                }

                Row {
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: 8

                    ToolButton {
                        label: root.confirmMode === "open" ? "放弃并打开" : "放弃并关闭"
                        active: true
                        onClicked: confirmProceed()
                    }
                    ToolButton {
                        label: "保存并关闭"
                        active: root.confirmMode === "close"
                        onClicked: confirmSaveAndClose()
                    }
                    ToolButton {
                        label: "取消"
                        active: true
                        onClicked: confirmCancel()
                    }
                }
            }
        }
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
    function confirmThenOpen(url) {
        if (bridge.dirty) {
            root.pendingOpenUrl = url
            root.confirmMode = "open"
            confirmOverlay.visible = true
        } else {
            bridge.open_file(url)
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
                bridge.open_file(url)
        } else if (root.confirmMode === "close") {
            confirmOverlay.visible = false
            root.forceClose = true
            if (root.primary) {
                Qt.quit()
            } else {
                const w = Window.window
                if (w)
                    w.close()
            }
        }
        root.confirmMode = ""
    }

    function confirmSaveAndClose() {
        bridge.save_edited()
        root.confirmMode = "close"
        confirmProceed()
    }

    function confirmCancel() {
        root.pendingOpenUrl = null
        root.confirmMode = ""
        confirmOverlay.visible = false
    }

    Component.onCompleted: {
        bridge.mark("qml-completed");
        if (root.primary) {
            bridge.start();
            if (root.probeDialog)
                dialogProbeTimer.start();
        }
    }
}
}
