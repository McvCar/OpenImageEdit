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
    // Frameless: the custom title bar (in EditorContent) provides the drag
    // area, window buttons and resize edges.
    flags: Qt.Window | Qt.FramelessWindowHint
    visibility: content.fullscreen ? Window.FullScreen : Window.Windowed
    title: content.b.image_width > 0
           ? "ImageEdit — " + content.b.image_width + "×" + content.b.image_height
           : "ImageEdit"

    onAfterRendering: content.afterFrame()
    onVisibleChanged: content.b.mark("window-visible")
    onClosing: (close) => content.requestClose(close)

    WindowEdges {}

    EditorContent {
        id: content
        anchors.fill: parent
        primary: true
        onRequestNewWindow: (fileUrl) => openNewWindow(fileUrl)
    }

    property int windowCascade: 0

    function openNewWindow(fileUrl) {
        const offset = shell.windowCascade * 32
        shell.windowCascade = (shell.windowCascade + 1) % 8
        docWindowFactory.createObject(null, {
            x: 80 + offset, y: 60 + offset,
            newWindowFn: openNewWindow,
            fileUrl: fileUrl !== undefined ? fileUrl : ""
        })
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

    // Checkbox row with a real checkbox visual: filled blue square + check
    // mark when on, empty dark square when off, hover highlight on both.
    component CheckRow: Item {
        id: ck

        property string label
        property bool checked: false
        property bool active: true
        signal toggled()

        width: 14 + 8 + ckLabel.implicitWidth
        height: 18

        Rectangle {
            id: box
            x: 0
            anchors.verticalCenter: parent.verticalCenter
            width: 14
            height: 14
            radius: 3
            color: ck.checked ? "#3d5a80" : (ckHover.containsMouse && ck.active ? "#33373c" : "#1b1c1e")
            border.color: ck.checked ? "#6aa9ff"
                          : (ckHover.containsMouse && ck.active ? "#8f96a3" : "#5a6069")
            border.width: ck.checked ? 2 : 1
            opacity: ck.active ? 1.0 : 0.35

            Text {
                anchors.centerIn: parent
                visible: ck.checked
                text: "✓"
                color: "#e9eaec"
                font.pixelSize: 11
                font.bold: true
            }
        }
        Text {
            id: ckLabel
            x: 22
            anchors.verticalCenter: parent.verticalCenter
            text: ck.label
            color: ck.checked ? "#e9eaec" : "#b8bcc0"
            font.pixelSize: 12
            opacity: ck.active ? 1.0 : 0.35
        }
        MouseArea {
            id: ckHover
            anchors.fill: parent
            hoverEnabled: true
            enabled: ck.active
            cursorShape: Qt.PointingHandCursor
            onClicked: ck.toggled()
        }
    }

    // Title-bar icon button (no QtQuick.Controls dependency: the tooltip is
    // hand-rolled). `separator` renders a slim divider instead of a button.
    component IconButton: Rectangle {
        id: ib

        property string glyph
        property string tip
        property bool active: true
        property bool selected: false
        property bool separator: false
        signal clicked()

        width: separator ? 9 : 30
        height: 28
        radius: 4
        color: separator ? "transparent"
               : (selected ? "#3d5a80"
                  : (hover.containsMouse && active ? "#4a4e54" : "transparent"))
        opacity: active ? 1.0 : 0.35

        Rectangle {
            visible: ib.separator
            anchors.centerIn: parent
            width: 1
            height: 18
            color: "#3c4043"
        }
        Text {
            visible: !ib.separator
            anchors.centerIn: parent
            text: ib.glyph
            color: "#e9eaec"
            font.pixelSize: 14
        }

        Rectangle {
            id: tipBox
            visible: false
            z: 1000
            x: Math.max(-ib.x, (ib.width - width) / 2)
            y: ib.height + 5
            width: tipText.implicitWidth + 16
            height: 22
            radius: 4
            color: "#111214"
            border.color: "#3c4043"
            Text {
                id: tipText
                anchors.centerIn: parent
                text: ib.tip
                color: "#d7dade"
                font.pixelSize: 11
            }
        }
        Timer {
            id: tipDelay
            interval: 500
            onTriggered: tipBox.visible = true
        }

        MouseArea {
            id: hover
            anchors.fill: parent
            hoverEnabled: !ib.separator
            enabled: !ib.separator
            cursorShape: Qt.PointingHandCursor
            onContainsMouseChanged: {
                if (!containsMouse) {
                    tipBox.visible = false
                    tipDelay.stop()
                } else if (ib.active && ib.tip.length > 0) {
                    tipDelay.restart()
                }
            }
            onClicked: if (ib.active) ib.clicked()
        }
    }

    // Text row inside the overflow "…" menu.
    component MenuRow: Rectangle {
        id: mr

        property string label
        property bool active: true
        property bool separator: false
        signal activated()

        width: 218
        height: separator ? 9 : 26
        radius: 4
        color: separator ? "transparent"
               : (mh.containsMouse && active ? "#3d5a80" : "transparent")
        opacity: active ? 1 : 0.35

        Rectangle {
            visible: mr.separator
            anchors.centerIn: parent
            width: parent.width - 16
            height: 1
            color: "#3c4043"
        }
        Text {
            visible: !mr.separator
            x: 10
            anchors.verticalCenter: parent.verticalCenter
            text: mr.label
            color: "#e9eaec"
            font.pixelSize: 12
        }
        MouseArea {
            id: mh
            anchors.fill: parent
            hoverEnabled: !mr.separator
            enabled: !mr.separator
            cursorShape: Qt.PointingHandCursor
            onClicked: if (mr.active) mr.activated()
        }
    }

    // Single resize strip for the frameless windows (native system resize).
    component EdgeGrab: MouseArea {
        property int edges: 0
        property int shape: Qt.SizeAllCursor
        cursorShape: shape
        onPressed: (mouse) => {
            const w = Window.window
            if (w && w.startSystemResize)
                w.startSystemResize(edges)
        }
    }

    // 4px resize strips for the frameless windows (native system resize).
    component WindowEdges: Item {
        anchors.fill: parent
        z: 1000

        EdgeGrab { anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right; height: 4; edges: Qt.TopEdge; shape: Qt.SizeVerCursor }
        EdgeGrab { anchors.bottom: parent.bottom; anchors.left: parent.left; anchors.right: parent.right; height: 4; edges: Qt.BottomEdge; shape: Qt.SizeVerCursor }
        EdgeGrab { anchors.top: parent.top; anchors.bottom: parent.bottom; anchors.left: parent.left; width: 4; edges: Qt.LeftEdge; shape: Qt.SizeHorCursor }
        EdgeGrab { anchors.top: parent.top; anchors.bottom: parent.bottom; anchors.right: parent.right; width: 4; edges: Qt.RightEdge; shape: Qt.SizeHorCursor }
        EdgeGrab { anchors.top: parent.top; anchors.left: parent.left; width: 10; height: 10; edges: Qt.TopEdge | Qt.LeftEdge; shape: Qt.SizeFDiagCursor }
        EdgeGrab { anchors.top: parent.top; anchors.right: parent.right; width: 10; height: 10; edges: Qt.TopEdge | Qt.RightEdge; shape: Qt.SizeBDiagCursor }
        EdgeGrab { anchors.bottom: parent.bottom; anchors.left: parent.left; width: 10; height: 10; edges: Qt.BottomEdge | Qt.LeftEdge; shape: Qt.SizeBDiagCursor }
        EdgeGrab { anchors.bottom: parent.bottom; anchors.right: parent.right; width: 10; height: 10; edges: Qt.BottomEdge | Qt.RightEdge; shape: Qt.SizeFDiagCursor }
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
        // "Marching ants" phase: bumped periodically while visible, the
        // dash pattern slides backwards along the (clockwise) rect path,
        // which reads as a counterclockwise rotation.
        property real dashPhase: 0
        onRxChanged: requestPaint()
        onRyChanged: requestPaint()
        onRwChanged: requestPaint()
        onRhChanged: requestPaint()
        onDashColorChanged: requestPaint()
        onDashPhaseChanged: requestPaint()
        onPaint: {
            var ctx = getContext("2d")
            ctx.clearRect(0, 0, width, height)
            if (rw < 1 || rh < 1)
                return
            ctx.strokeStyle = String(dashColor)
            ctx.lineWidth = 1.5
            ctx.setLineDash([5, 4])
            ctx.lineDashOffset = dashPhase
            ctx.strokeRect(rx + 0.75, ry + 0.75, rw - 1.5, rh - 1.5)
        }
        Timer {
            interval: 90
            repeat: true
            running: dashed.visible && dashed.rw > 0 && dashed.rh > 0
            onTriggered: dashed.dashPhase = (dashed.dashPhase + 1) % 9
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
                width: hma.pressed ? 12 : 10
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
                    // hoverEnabled OFF + cursorShape undefined: hover and
                    // cursor decisions both belong to the canvas HoverHandler;
                    // this area only handles the press for resizing.
                    cursorShape: undefined
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
            hoverEnabled: false
            // undefined: without it this area's default ArrowCursor shadows
            // the canvas HoverHandler across the whole box interior.
            cursorShape: undefined
            enabled: handles.moveEnabled && handles.moveFn !== null
                     && handles.bw > 0 && handles.bh > 0
            property rect startRect
            property real px: 0
            property real py: 0

            // Only grab presses that land INSIDE the box; a press on empty
            // canvas is rejected so it falls through to the rubber-band area
            // below. NOTE: gating `enabled` on hover coordinates (the old
            // approach) deadlocks — a disabled MouseArea stops receiving
            // hover, so its mouseX/mouseY freeze and `enabled` can never
            // become true again.
            onPressed: (mouse) => {
                if (mouse.x <= handles.bx || mouse.x >= handles.bx + handles.bw
                        || mouse.y <= handles.by || mouse.y >= handles.by + handles.bh) {
                    mouse.accepted = false
                    return
                }
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

        // Four edge strips: the whole dashed border is grabbable, not only
        // the dots. Each strip straddles the box edge (half in, half out),
        // shows the matching resize cursor on hover and feeds resizeFn with
        // the same role strings the dot handles use. z:1 keeps them above
        // the center move pad (z:0) but below the dot handles (z:2), so
        // corners keep their diagonal cursors.
        Repeater {
            model: [
                { role: "n", shape: Qt.SizeVerCursor },
                { role: "s", shape: Qt.SizeVerCursor },
                { role: "w", shape: Qt.SizeHorCursor },
                { role: "e", shape: Qt.SizeHorCursor }
            ]

            delegate: MouseArea {
                required property var modelData
                readonly property string role: modelData.role
                // hoverEnabled OFF — hover must reach the canvas HoverHandler
                // (the single cursor authority); presses land here.
                // cursorShape undefined — default Arrow would shadow it.
                cursorShape: undefined
                enabled: handles.resizeFn !== null && handles.bw > 0 && handles.bh > 0
                z: 1
                x: role === "e" ? handles.bx + handles.bw - 4
                                : handles.bx - 4
                y: role === "s" ? handles.by + handles.bh - 4
                                : handles.by - 4
                width: role === "w" || role === "e" ? 8 : handles.bw + 8
                height: role === "n" || role === "s" ? 8 : handles.bh + 8

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
                    if (pressed && handles.resizeFn)
                        handles.resizeFn(startRect, role, mouse.x - px, mouse.y - py,
                                         (mouse.modifiers & Qt.ShiftModifier) !== 0)
                }
                onReleased: handles.engaged = false
            }
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
        // Frameless: same custom title bar as the shell window.
        flags: Qt.Window | Qt.FramelessWindowHint
        visibility: content.fullscreen ? Window.FullScreen : Window.Windowed
        title: content.b.image_width > 0
               ? "ImageEdit — " + content.b.image_width + "×" + content.b.image_height
               : "ImageEdit"

        onAfterRendering: content.afterFrame()
        onVisibleChanged: content.b.mark("window-visible")
        onClosing: (close) => content.requestClose(close)

        WindowEdges {}

        EditorContent {
            id: content
            anchors.fill: parent
            onRequestNewWindow: (fileUrl) => {
                if (docWin.newWindowFn)
                    docWin.newWindowFn(fileUrl)
            }
        }
    }

    component EditorContent: Item {
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
        readonly property var selLayer: findSelLayer()
        readonly property var selLayerBox: findLayerBox()
        readonly property bool probeDialog: Qt.application.arguments.indexOf("--probe-dialog") !== -1
        readonly property bool probeSettings: Qt.application.arguments.indexOf("--probe-settings") !== -1
        readonly property bool probeMenu: Qt.application.arguments.indexOf("--probe-menu") !== -1
        readonly property bool probeResize: Qt.application.arguments.indexOf("--probe-resize") !== -1
        readonly property bool probeLayers: Qt.application.arguments.indexOf("--probe-layers") !== -1
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
    // Browse images in the loaded file's folder (guard: no active gesture).
    // Dialogs block navigation so arrow keys never swap the file behind a
    // open settings/resize panel.
    Shortcut { sequence: "Left"; enabled: canNavigate(); onActivated: navigateRel(-1) }
    Shortcut { sequence: "Right"; enabled: canNavigate(); onActivated: navigateRel(1) }
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
    Shortcut { sequence: "Ctrl+Shift+R"; onActivated: bridge.rotate_cw() }
    Shortcut { sequences: ["Delete", "Backspace"]
               enabled: root.selExists || bridge.selected_layer >= 0
                        || (function () {
                            const b = findBase()
                            return b !== null && b.bd !== true && !isLocked(b)
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
              run: function () { openResizeDialog() } },
            { g: "", t: "", ok: true, sep: true },
            { g: "＋", t: "放大 (Ctrl+滚轮)", ok: true,
              run: function () { bridge.zoom_in() } },
            { g: "－", t: "缩小 (Ctrl+滚轮)", ok: true,
              run: function () { bridge.zoom_out() } }
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
            { l: "调整尺寸…", ok: bridge.image_width > 0, run: function () { openResizeDialog() } },
            { l: "", ok: true, sep: true },
            { l: "提取图层", ok: root.selExists, run: function () { commitExtract() } },
            { l: "删除选区像素 (Delete)", ok: root.selExists, run: function () { deleteSelectionPixels() } },
            { l: "图层面板", ok: true, run: function () { root.panelVisible = !root.panelVisible } },
            { l: "", ok: true, sep: true },
            { l: "放大 (Ctrl++)", ok: true, run: function () { bridge.zoom_in() } },
            { l: "缩小 (Ctrl+-)", ok: true, run: function () { bridge.zoom_out() } },
            { l: "适应窗口 (Ctrl+9)", ok: bridge.image_width > 0, run: function () { bridge.zoom_fit() } },
            { l: "100% 实际大小 (Ctrl+0)", ok: bridge.image_width > 0, run: function () { bridge.zoom_actual() } },
            { l: "", ok: true, sep: true },
            { l: "另存副本 (Ctrl+S)", ok: bridge.image_width > 0, run: function () { bridge.save_edited() } },
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
                           + layersBtn.width + 8
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
            text: "ImageEdit"
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

        anchors.top: toolbar.bottom
        anchors.bottom: statusBar.top
        anchors.left: parent.left
        anchors.right: parent.right
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
            contentX = Math.max(0, (contentWidth - width) / 2)
            contentY = Math.max(0, (contentHeight - height) / 2)
        }
        onContentWidthChanged: centerContent()
        onContentHeightChanged: centerContent()
        onWidthChanged: {
            centerContent()
            bridge.set_viewport(width, height)
        }
        onHeightChanged: {
            centerContent()
            bridge.set_viewport(width, height)
        }
        Connections {
            target: bridge
            function onFitModeChanged() { view.centerContent() }
        }

        // Clamp-and-shift the visible region by (dx, dy) viewport px.
        function panBy(dx, dy) {
            const maxX = Math.max(0, contentWidth - width)
            const maxY = Math.max(0, contentHeight - height)
            contentX = Math.min(maxX, Math.max(0, contentX + dx))
            contentY = Math.min(maxY, Math.max(0, contentY + dy))
        }

        // PS-style checkerboard behind the image: transparent pixels show
        // the grid configured in the settings panel.
        Canvas {
            id: checkerboard
            visible: bridge.preview_url.length > 0
            x: previewImage.x
            y: previewImage.y
            width: previewImage.width
            height: previewImage.height
            readonly property color cLight: bridge.checkerLight
            readonly property color cDark: bridge.checkerDark
            onCLightChanged: requestPaint()
            onCDarkChanged: requestPaint()
            onWidthChanged: requestPaint()
            onHeightChanged: requestPaint()
            onPaint: {
                var ctx = getContext("2d")
                ctx.clearRect(0, 0, width, height)
                const cell = 8
                ctx.fillStyle = cDark
                ctx.fillRect(0, 0, width, height)
                ctx.fillStyle = cLight
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

            // Authoritative canvas cursor: a HoverHandler's cursorShape beats
            // every MouseArea's (documented precedence), and one handler over
            // the whole surface means no stacking conflicts. Coordinates are
            // in canvasSurface space — image coords are offset by view.pad.
            HoverHandler {
                id: canvasHoverHandler
                cursorShape: panMa.pressed ? Qt.ClosedHandCursor
                                           : canvasCursor(point.position.x,
                                                          point.position.y)
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
            visible: root.selExists
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
                    bridge.zoom_in()
                else if (event.angleDelta.y < 0)
                    bridge.zoom_out()
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
            id: fileInfo
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left
            anchors.leftMargin: 10
            visible: bridge.fileName.length > 0
            text: bridge.fileName
                  + (bridge.image_width > 0
                     ? "   ·   " + bridge.image_width + "×" + bridge.image_height
                     : "")
                  + (bridge.fileSizeStr.length > 0 ? "   ·   " + bridge.fileSizeStr : "")
            color: "#9aa3b2"
            font.pixelSize: 12
        }

        Text {
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: fileInfo.visible ? fileInfo.right : parent.left
            anchors.leftMargin: fileInfo.visible ? 12 : 10
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
                  ? bridge.zoom_percent + "%"
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
                        onReleased: {
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
        // Canvas-only mode: change the canvas resolution WITHOUT resampling
        // the image (content is re-anchored to the new center).
        property bool canvasOnly: false
        // Clear text-field focus on close, otherwise the hidden TextInput
        // keeps consuming Ctrl+Z / Ctrl+Y as text undo and the document
        // undo/redo shortcuts appear dead (bug: dead after resize dialog).
        onVisibleChanged: if (!visible) {
            resizeW.focus = false
            resizeH.focus = false
        }

        // Blocks clicks on the dimmed background; the dialog closes via the
        // ✕ / 取消 buttons or Esc only (click-away closing felt accidental).
        MouseArea {
            anchors.fill: parent
        }

        Rectangle {
            id: resizeDialog
            width: 360
            height: resizeCol.height + 28
            x: (parent.width - width) / 2
            y: (parent.height - height) / 2
            radius: 8
            color: "#2b2e33"
            border.color: "#3c4043"

            Column {
                id: resizeCol
                anchors.top: parent.top
                anchors.topMargin: 14
                anchors.left: parent.left
                anchors.leftMargin: 14
                anchors.right: parent.right
                anchors.rightMargin: 14
                spacing: 12

                // Title bar: title + close button.
                Row {
                    width: parent.width
                    spacing: 8
                    Text {
                        id: resizeTitle
                        anchors.verticalCenter: parent.verticalCenter
                        text: "调整尺寸"
                        color: "#e9eaec"
                        font.pixelSize: 14
                        font.bold: true
                    }
                    Item { width: parent.width - resizeTitle.implicitWidth - 30 - 16; height: 1 }
                    IconButton {
                        glyph: "✕"
                        tip: "关闭"
                        onClicked: resizeOverlay.visible = false
                    }
                }
                Rectangle { width: parent.width; height: 1; color: "#3c4043" }

                Row {
                    spacing: 8

                    Text {
                        text: "宽"
                        color: "#b8bcc0"
                        font.pixelSize: 12
                        anchors.verticalCenter: parent.verticalCenter
                    }

                    Item {
                        width: 76
                        height: 26
                        anchors.verticalCenter: parent.verticalCenter

                        Rectangle {
                            anchors.fill: parent
                            color: "#1b1c1e"
                            border.color: resizeW.activeFocus ? "#6aa9ff" : "#3c4043"
                            border.width: resizeW.activeFocus ? 2 : 1
                            radius: 3
                        }

                        TextInput {
                            id: resizeW
                            anchors.fill: parent
                            anchors.margins: 5
                            text: ""
                            font.pixelSize: 13
                            color: "#e9eaec"
                            verticalAlignment: TextInput.AlignVCenter
                            validator: IntValidator { bottom: 1; top: 99999 }
                            selectByMouse: true
                            onActiveFocusChanged: if (activeFocus) selectAll()
                            onTextChanged: if (resizeOverlay.keepRatio
                                               && !resizeOverlay.canvasOnly
                                               && activeFocus)
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
                        width: 76
                        height: 26
                        anchors.verticalCenter: parent.verticalCenter

                        Rectangle {
                            anchors.fill: parent
                            color: "#1b1c1e"
                            border.color: resizeH.activeFocus ? "#6aa9ff" : "#3c4043"
                            border.width: resizeH.activeFocus ? 2 : 1
                            radius: 3
                        }

                        TextInput {
                            id: resizeH
                            anchors.fill: parent
                            anchors.margins: 5
                            text: ""
                            font.pixelSize: 13
                            color: "#e9eaec"
                            verticalAlignment: TextInput.AlignVCenter
                            validator: IntValidator { bottom: 1; top: 99999 }
                            selectByMouse: true
                            onActiveFocusChanged: if (activeFocus) selectAll()
                            onTextChanged: if (resizeOverlay.keepRatio
                                               && !resizeOverlay.canvasOnly
                                               && activeFocus)
                                resizeW.text = linkedWidth()
                            onAccepted: commitResize()
                        }
                    }

                    Text {
                        text: "px"
                        color: "#6b7076"
                        font.pixelSize: 11
                        anchors.verticalCenter: parent.verticalCenter
                    }
                }

                CheckRow {
                    id: ratioCheck
                    label: "保持宽高比"
                    checked: resizeOverlay.keepRatio
                    active: !resizeOverlay.canvasOnly
                    onToggled: resizeOverlay.keepRatio = !resizeOverlay.keepRatio
                }

                CheckRow {
                    id: canvasCheck
                    label: "只调整画布大小（图片不缩放，内容居中）"
                    checked: resizeOverlay.canvasOnly
                    onToggled: {
                        resizeOverlay.canvasOnly = !resizeOverlay.canvasOnly
                        if (resizeOverlay.canvasOnly)
                            resizeOverlay.keepRatio = false
                    }
                }

                Text {
                    text: resizeOverlay.canvasOnly
                          ? "画布变为目标分辨率，图片按原样居中放置"
                          : "图片内容缩放到目标分辨率（Lanczos）"
                    color: "#8f96a3"
                    font.pixelSize: 11
                }

                Text {
                    text: "当前：" + bridge.image_width + " × " + bridge.image_height
                    color: "#6b7076"
                    font.pixelSize: 11
                }

                Row {
                    width: parent.width
                    spacing: 8

                    Item { width: parent.width - 60 - 60 - 16; height: 1 }
                    ToolButton {
                        label: "取消"
                        active: true
                        onClicked: resizeOverlay.visible = false
                    }
                    ToolButton {
                        label: resizeOverlay.canvasOnly ? "应用画布" : "确定"
                        active: true
                        selected: true
                        onClicked: commitResize()
                    }
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

    // Canvas cursor for a point in canvasSurface coordinates. selX/selY and
    // the layer boxes live in preview-image coordinates, whose origin is
    // view.pad to the right/bottom of canvasSurface — shift before comparing.
    // topLayerAt() however expects canvasSurface coords directly.
    // Fed to the canvasHoverHandler below — HoverHandler cursorShape
    // overrides every MouseArea's cursorShape, so this is the single
    // authoritative cursor decision.
    function canvasCursor(x, y) {
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
        resizeOverlay.keepRatio = true
        resizeOverlay.canvasOnly = false
        resizeOverlay.visible = true
    }

    function commitResize() {
        let w = parseInt(resizeW.text)
        let h = parseInt(resizeH.text)
        if (resizeOverlay.keepRatio && !resizeOverlay.canvasOnly) {
            if (isNaN(h))
                h = parseInt(linkedHeight())
            if (isNaN(w))
                w = parseInt(linkedWidth())
        }
        if (isNaN(w) || w < 1 || isNaN(h) || h < 1)
            return
        if (resizeOverlay.canvasOnly)
            bridge.resize_canvas(w, h)
        else
            bridge.resize_to(w, h)
        resizeOverlay.visible = false
    }

    function onEscape() {
        if (root.renamingIndex >= 0) { root.renamingIndex = -1; return }
        if (confirmOverlay.visible) { confirmCancel(); return }
        if (resizeOverlay.visible) { resizeOverlay.visible = false; return }
        if (settingsOverlay.visible) { settingsOverlay.visible = false; return }
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

    Item {
        id: confirmOverlay
        anchors.fill: parent
        visible: false
        z: 200

        MouseArea { anchors.fill: parent }

        // Native-dialog-style window: title bar with close button, body with
        // warning icon + message, footer with right-aligned actions.
        Rectangle {
            width: 420
            height: 190
            anchors.centerIn: parent
            radius: 8
            color: "#25272b"
            border.color: "#4a4e54"
            clip: true

            // Title bar
            Item {
                id: confirmTitle
                width: parent.width
                height: 38

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    anchors.verticalCenter: parent.verticalCenter
                    text: "未保存的更改"
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
                    onClicked: confirmCancel()
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: "#3c4043"
                }
            }

            // Body: warning glyph + message
            Item {
                anchors.top: confirmTitle.bottom
                anchors.bottom: confirmFooter.top
                anchors.left: parent.left
                anchors.right: parent.right

                Text {
                    id: warnGlyph
                    anchors.left: parent.left
                    anchors.leftMargin: 18
                    anchors.verticalCenter: parent.verticalCenter
                    text: "⚠"
                    color: "#e8b339"
                    font.pixelSize: 26
                }

                Text {
                    anchors.left: warnGlyph.right
                    anchors.leftMargin: 14
                    anchors.right: parent.right
                    anchors.rightMargin: 18
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.confirmMode === "open"
                          ? "打开新文件将放弃当前图片的未保存修改。"
                          : root.confirmMode === "nav"
                          ? "切换到其他图片将放弃当前图片的未保存修改。"
                          : "关闭窗口将放弃当前图片的未保存修改。"
                    color: "#c9cdd2"
                    font.pixelSize: 13
                    wrapMode: Text.WordWrap
                }
            }

            // Footer: right-aligned action buttons
            Item {
                id: confirmFooter
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
                        onClicked: confirmCancel()
                    }
                    ToolButton {
                        label: "保存并关闭"
                        active: root.confirmMode === "close"
                        onClicked: confirmSaveAndClose()
                    }
                    ToolButton {
                        label: root.confirmMode === "open" ? "放弃并打开"
                             : root.confirmMode === "nav" ? "放弃并切换"
                             : "放弃并关闭"
                        active: true
                        selected: true
                        onClicked: confirmProceed()
                    }
                }
            }
        }
    }

    // Settings panel: transparency checkerboard colors, "open file" behavior,
    // and default-viewer registration. Every change persists immediately
    // (%APPDATA%/ImageEdit/settings.ini via the Bridge).
    Rectangle {
        id: settingsOverlay
        anchors.fill: parent
        visible: false
        z: 200
        color: "#88000000"
        onVisibleChanged: if (!visible) {
            hexLight.focus = false
            hexDark.focus = false
        }

        // Blocks clicks on the dimmed background; the panel closes via the
        // ✕ button or Esc only (click-away closing felt accidental).
        MouseArea {
            anchors.fill: parent
        }

        Rectangle {
            width: 420
            height: settingsCol.height + 28
            anchors.centerIn: parent
            radius: 8
            color: "#2b2e33"
            border.color: "#3c4043"

            Column {
                id: settingsCol
                anchors.top: parent.top
                anchors.topMargin: 14
                anchors.left: parent.left
                anchors.leftMargin: 14
                anchors.right: parent.right
                anchors.rightMargin: 14
                spacing: 12

                // Title bar: title + close button.
                Row {
                    width: parent.width
                    spacing: 8
                    Text {
                        id: setTitle
                        anchors.verticalCenter: parent.verticalCenter
                        text: "设置"
                        color: "#e9eaec"
                        font.pixelSize: 14
                        font.bold: true
                    }
                    Item { width: parent.width - setTitle.implicitWidth - 30 - 16; height: 1 }
                    IconButton {
                        glyph: "✕"
                        tip: "关闭"
                        onClicked: settingsOverlay.visible = false
                    }
                }

                Rectangle { width: parent.width; height: 1; color: "#3c4043" }

                // ---- 1. checkerboard colors ----
                Text {
                    text: "透明背景（棋盘格）颜色"
                    color: "#b8bcc0"
                    font.pixelSize: 12
                }
                Row {
                    spacing: 10

                    Canvas {
                        id: checkerPreview
                        width: 76
                        height: 26
                        anchors.verticalCenter: parent.verticalCenter
                        readonly property color cLight: bridge.checkerLight
                        readonly property color cDark: bridge.checkerDark
                        onCLightChanged: requestPaint()
                        onCDarkChanged: requestPaint()
                        onPaint: {
                            var ctx = getContext("2d")
                            const cell = 8
                            ctx.fillStyle = cDark
                            ctx.fillRect(0, 0, width, height)
                            ctx.fillStyle = cLight
                            for (var gy = 0; gy < height; gy += cell)
                                for (var gx = 0; gx < width; gx += cell)
                                    if (((gx / cell) | 0) % 2 === ((gy / cell) | 0) % 2)
                                        ctx.fillRect(gx, gy, cell, cell)
                        }
                    }

                    Column {
                        spacing: 4
                        Text { text: "亮格"; color: "#8f96a3"; font.pixelSize: 11 }
                        Item {
                            width: 72
                            height: 24
                            Rectangle { anchors.fill: parent; color: "#1b1c1e"; border.color: "#3c4043"; radius: 3 }
                            TextInput {
                                id: hexLight
                                anchors.fill: parent
                                anchors.margins: 4
                                text: bridge.checkerLight
                                font.pixelSize: 12
                                color: "#e9eaec"
                                verticalAlignment: TextInput.AlignVCenter
                                onEditingFinished: {
                                    if (/^#[0-9A-Fa-f]{6}$/.test(text))
                                        bridge.update_checker_light(text)
                                    else
                                        text = bridge.checkerLight
                                }
                            }
                        }
                    }

                    Column {
                        spacing: 4
                        Text { text: "暗格"; color: "#8f96a3"; font.pixelSize: 11 }
                        Item {
                            width: 72
                            height: 24
                            Rectangle { anchors.fill: parent; color: "#1b1c1e"; border.color: "#3c4043"; radius: 3 }
                            TextInput {
                                id: hexDark
                                anchors.fill: parent
                                anchors.margins: 4
                                text: bridge.checkerDark
                                font.pixelSize: 12
                                color: "#e9eaec"
                                verticalAlignment: TextInput.AlignVCenter
                                onEditingFinished: {
                                    if (/^#[0-9A-Fa-f]{6}$/.test(text))
                                        bridge.update_checker_dark(text)
                                    else
                                        text = bridge.checkerDark
                                }
                            }
                        }
                    }
                }
                Row {
                    spacing: 6
                    ToolButton {
                        label: "经典"
                        onClicked: {
                            bridge.update_checker_light("#ffffff")
                            bridge.update_checker_dark("#cccccc")
                            hexLight.text = "#ffffff"
                            hexDark.text = "#cccccc"
                        }
                    }
                    ToolButton {
                        label: "暗色"
                        onClicked: {
                            bridge.update_checker_light("#3c4043")
                            bridge.update_checker_dark("#202225")
                            hexLight.text = "#3c4043"
                            hexDark.text = "#202225"
                        }
                    }
                    ToolButton {
                        label: "纯白"
                        onClicked: {
                            bridge.update_checker_light("#ffffff")
                            bridge.update_checker_dark("#f2f2f2")
                            hexLight.text = "#ffffff"
                            hexDark.text = "#f2f2f2"
                        }
                    }
                    ToolButton {
                        label: "炭黑"
                        onClicked: {
                            bridge.update_checker_light("#2a2d31")
                            bridge.update_checker_dark("#141517")
                            hexLight.text = "#2a2d31"
                            hexDark.text = "#141517"
                        }
                    }
                }

                Rectangle { width: parent.width; height: 1; color: "#3c4043" }

                // ---- 2. open behavior ----
                Text {
                    text: "打开文件时"
                    color: "#b8bcc0"
                    font.pixelSize: 12
                }
                Repeater {
                    model: [
                        { label: "在当前窗口显示（替换现有图片）", value: false },
                        { label: "在单独的新窗口显示", value: true }
                    ]
                    delegate: MouseArea {
                        id: radioRow
                        required property var modelData
                        width: 16 + 8 + lbl.implicitWidth
                        height: Math.max(16, lbl.implicitHeight)
                        cursorShape: Qt.PointingHandCursor
                        onClicked: bridge.update_open_in_new_window(modelData.value)

                        Item {
                            width: 16
                            height: 16
                            anchors.verticalCenter: parent.verticalCenter
                            Rectangle {
                                anchors.fill: parent
                                radius: 8
                                color: "transparent"
                                border.color: bridge.openInNewWindow === radioRow.modelData.value ? "#6aa9ff" : "#5a6069"
                                border.width: 2
                            }
                            Rectangle {
                                anchors.centerIn: parent
                                width: 8
                                height: 8
                                radius: 4
                                color: "#6aa9ff"
                                visible: bridge.openInNewWindow === radioRow.modelData.value
                            }
                        }
                        Text {
                            id: lbl
                            x: 24
                            anchors.verticalCenter: parent.verticalCenter
                            text: radioRow.modelData.label
                            color: "#d5d8dd"
                            font.pixelSize: 12
                        }
                    }
                }

                Rectangle { width: parent.width; height: 1; color: "#3c4043" }

                // ---- 3. default viewer ----
                Text {
                    text: "默认图片查看器"
                    color: "#b8bcc0"
                    font.pixelSize: 12
                }
                Row {
                    spacing: 10
                    ToolButton { label: "注册并打开系统设置…"; onClicked: bridge.register_as_viewer() }
                    Text {
                        text: "写入「打开方式」列表后打开\n系统设置，请在其中选择 ImageEdit"
                        color: "#8f96a3"
                        font.pixelSize: 11
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

    function confirmSaveAndClose() {
        bridge.save_edited()
        root.confirmMode = "close"
        confirmProceed()
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
        onTriggered: openResizeDialog()
    }

    // Probe timer: --probe-layers shows the layer panel automatically.
    Timer {
        id: layersProbeTimer
        interval: 600
        onTriggered: root.panelVisible = true
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
        } else if (root.fileUrl.toString().length > 0) {
            // Document window spawned for a specific file ("open in separate
            // window" setting, drag-out, etc.).
            bridge.open_file(root.fileUrl)
        }
    }
}
}
