import QtQuick
import QtQuick.Window
import QtQuick.Dialogs
import com.imageedit

Window {
    id: root

    property bool firstFrameSeen: false
    property bool imageRendered: false
    property bool imageFrameSeen: false
    readonly property int toolbarHeight: 40
    readonly property int statusHeight: 26

    property bool fullscreen: false
    property bool cropMode: false
    property bool selectionMode: false
    property bool moveMode: false
    property bool panelVisible: false
    readonly property bool modeActive: cropMode || selectionMode || moveMode
    property real selX: 0
    property real selY: 0
    property real selW: 0
    property real selH: 0
    readonly property bool selExists: selW >= 3 && selH >= 3
    readonly property var layerList: parseLayers(bridge.layers_json)
    readonly property var selLayer: findSelLayer()
    readonly property bool probeDialog: Qt.application.arguments.indexOf("--probe-dialog") !== -1

    width: 1100
    height: 760
    minimumWidth: 480
    minimumHeight: 320
    visible: Qt.application.arguments.indexOf("--hidden") === -1
    color: "#1b1c1e"
    visibility: root.fullscreen ? Window.FullScreen : Window.Windowed
    title: bridge.image_width > 0
           ? "ImageEdit — " + bridge.image_width + "×" + bridge.image_height
           : "ImageEdit"

    Bridge {
        id: bridge

        onLoadFailedChanged: if (bridge.loadFailed && bridge.auto_quit)
            Qt.quit()
        onImage_widthChanged: if (!cropMouse.dragging)
            root.cropMode = false
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

    FileDialog {
        id: openDialog
        title: "打开图片"
        nameFilters: [
            "图片文件 (*.jpg *.jpeg *.png *.gif *.bmp *.tif *.tiff *.webp)",
            "所有文件 (*)"
        ]
        onAccepted: bridge.open_file(selectedFile)
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
    Shortcut { sequences: [StandardKey.SaveAs]; onActivated: saveDialog.open() }
    Shortcut { sequence: "F11"; onActivated: root.fullscreen = !root.fullscreen }
    Shortcut { sequence: "Escape"; onActivated: onEscape() }

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
    Shortcut { sequence: "Ctrl+0"; onActivated: bridge.zoom_fit() }
    Shortcut { sequence: "Ctrl+1"; onActivated: bridge.zoom_actual() }
    Shortcut { sequences: [StandardKey.Undo]; onActivated: bridge.undo() }
    Shortcut { sequences: [StandardKey.Redo]; onActivated: bridge.redo() }
    Shortcut { sequences: [StandardKey.Save]; onActivated: bridge.save_edited() }
    Shortcut { sequence: "Ctrl+R"; onActivated: bridge.rotate_cw() }
    Shortcut { sequence: "Ctrl+Shift+R"; onActivated: bridge.rotate_ccw() }

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
                active: bridge.image_width > 0
                selected: root.cropMode
                onClicked: toggleMode("crop")
            }
            ToolButton { label: "重置裁剪"; active: bridge.image_width > 0; onClicked: bridge.reset_crop() }
            ToolButton { label: "调整尺寸…"; active: bridge.image_width > 0; onClicked: openResizeDialog() }

            Rectangle { width: 1; height: 20; color: "#3c4043" }

            ToolButton {
                label: "框选"
                active: bridge.image_width > 0
                selected: root.selectionMode
                onClicked: toggleMode("selection")
            }
            ToolButton { label: "提取图层"; active: root.selExists; onClicked: commitExtract() }
            ToolButton {
                label: "移动图层"
                active: bridge.selected_layer >= 0
                selected: root.moveMode
                onClicked: toggleMode("move")
            }
            ToolButton {
                label: "图层"
                active: bridge.layer_count > 0
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
        contentWidth: Math.max(width, previewImage.width)
        contentHeight: Math.max(height, previewImage.height)

        onWidthChanged: bridge.set_viewport(width, height)
        onHeightChanged: bridge.set_viewport(width, height)

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

        Item {
            id: cropSurface
            visible: root.cropMode && bridge.image_width > 0
            z: 5
            x: previewImage.x
            y: previewImage.y
            width: previewImage.width
            height: previewImage.height

            MouseArea {
                id: cropMouse
                anchors.fill: parent
                enabled: cropSurface.visible
                cursorShape: Qt.CrossCursor
                property real startX: 0
                property real startY: 0
                property real curX: 0
                property real curY: 0
                property bool dragging: false

                onPressed: (mouse) => {
                    startX = mouse.x
                    startY = mouse.y
                    curX = mouse.x
                    curY = mouse.y
                    dragging = true
                }
                onPositionChanged: (mouse) => {
                    if (dragging) {
                        curX = mouse.x
                        curY = mouse.y
                    }
                }
                onReleased: {
                    dragging = false
                    if (root.cropMode)
                        finishCrop()
                }
            }

            Rectangle {
                id: cropRect
                visible: cropMouse.dragging
                color: "#224f8ef7"
                border.color: "#5b9dff"
                border.width: 1
                x: Math.min(cropMouse.startX, cropMouse.curX)
                y: Math.min(cropMouse.startY, cropMouse.curY)
                width: Math.abs(cropMouse.curX - cropMouse.startX)
                height: Math.abs(cropMouse.curY - cropMouse.startY)
            }
        }

        Item {
            id: selectionSurface
            visible: root.selectionMode && bridge.image_width > 0
            z: 6
            x: previewImage.x
            y: previewImage.y
            width: previewImage.width
            height: previewImage.height

            MouseArea {
                id: selectionMouse
                anchors.fill: parent
                enabled: selectionSurface.visible
                cursorShape: Qt.CrossCursor
                property real startX: 0
                property real startY: 0
                property real curX: 0
                property real curY: 0
                property bool dragging: false

                onPressed: (mouse) => {
                    startX = mouse.x
                    startY = mouse.y
                    curX = mouse.x
                    curY = mouse.y
                    dragging = true
                }
                onPositionChanged: (mouse) => {
                    if (dragging) {
                        curX = mouse.x
                        curY = mouse.y
                    }
                }
                onReleased: {
                    dragging = false
                    const x = Math.min(startX, curX)
                    const y = Math.min(startY, curY)
                    const w = Math.abs(curX - startX)
                    const h = Math.abs(curY - startY)
                    if (w < 3 || h < 3) {
                        root.selW = 0
                        root.selH = 0
                    } else {
                        root.selX = x
                        root.selY = y
                        root.selW = w
                        root.selH = h
                    }
                    root.selectionMode = false
                }
            }
        }

        Rectangle {
            id: selRectViz
            visible: root.selExists && !root.selectionMode
            z: 7
            x: previewImage.x + root.selX
            y: previewImage.y + root.selY
            width: root.selW
            height: root.selH
            color: "transparent"
            border.color: "#5b9dff"
            border.width: 2
        }

        Item {
            id: moveSurface
            visible: root.moveMode && bridge.selected_layer >= 0
            z: 6
            x: previewImage.x
            y: previewImage.y
            width: previewImage.width
            height: previewImage.height

            Rectangle {
                id: moveRect
                visible: moveMouse.dragging
                color: "#5b9dff33"
                border.color: "#5b9dff"
                border.width: 1
                x: Math.min(moveMouse.startX, moveMouse.curX)
                y: Math.min(moveMouse.startY, moveMouse.curY)
                width: Math.abs(moveMouse.curX - moveMouse.startX)
                height: Math.abs(moveMouse.curY - moveMouse.startY)
            }

            MouseArea {
                id: moveMouse
                anchors.fill: parent
                enabled: moveSurface.visible
                cursorShape: Qt.SizeAllCursor
                property real startX: 0
                property real startY: 0
                property real curX: 0
                property real curY: 0
                property bool dragging: false

                onPressed: (mouse) => {
                    startX = mouse.x
                    startY = mouse.y
                    curX = mouse.x
                    curY = mouse.y
                    dragging = true
                }
                onPositionChanged: (mouse) => {
                    if (dragging) {
                        curX = mouse.x
                        curY = mouse.y
                    }
                }
                onReleased: {
                    dragging = false
                    const dx = curX - startX
                    const dy = curY - startY
                    if (Math.abs(dx) < 1 && Math.abs(dy) < 1)
                        return
                    const scale = toImageScale()
                    bridge.translate_selected(Math.round(dx * scale), Math.round(dy * scale))
                }
            }
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
            visible: root.modeActive || root.selExists
            text: root.cropMode
                    ? "拖框选择裁剪区域（Esc 取消）"
                    : root.selectionMode
                    ? "拖框选择区域，松手后点「提取图层」（Esc 取消）"
                    : root.moveMode
                    ? "拖动画面移动选中图层，方向键微调（Esc 退出）"
                    : "已选区：点「提取图层」生成图层，Esc 取消"
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
                bridge.open_file(drop.urls[0]);
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
                        width: layerRepeaterColumn.width
                        height: 34
                        radius: 4
                        readonly property bool isSel: model !== undefined && model.i === bridge.selected_layer
                        readonly property bool vis: model !== undefined && model.v === true
                        color: isSel ? "#3d5a80" : "#33373c"

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: if (model !== undefined) bridge.select_layer(model.i)
                        }

                        Text {
                            x: 8
                            anchors.verticalCenter: parent.verticalCenter
                            text: vis ? "●" : "○"
                            color: vis ? "#7fc3ff" : "#5a5f66"
                            font.pixelSize: 12
                        }

                        Text {
                            x: 28
                            width: parent.width - 28
                            anchors.verticalCenter: parent.verticalCenter
                            elide: Text.ElideRight
                            text: model !== undefined ? model.n + (isSel ? "  (选中)" : "") : ""
                            color: isSel ? "#ffffff" : "#c9cdd2"
                            font.pixelSize: 12
                        }
                    }
                }
            }
        }

        Row {
            id: panelFooter
            anchors.bottom: parent.bottom
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottomMargin: 8
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            spacing: 6
            visible: bridge.selected_layer >= 0

            ToolButton {
                label: "上移"
                active: true
                onClicked: bridge.move_selected_up()
            }
            ToolButton {
                label: "下移"
                active: true
                onClicked: bridge.move_selected_down()
            }
            ToolButton {
                label: root.selLayer && !root.selLayer.v ? "显示" : "隐藏"
                active: bridge.selected_layer >= 0
                onClicked: bridge.toggle_selected_visible()
            }
            ToolButton {
                label: "删除"
                active: true
                onClicked: bridge.delete_selected_layer()
            }
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

    function finishCrop() {
        const sx = Math.min(cropMouse.startX, cropMouse.curX)
        const sy = Math.min(cropMouse.startY, cropMouse.curY)
        const sw = Math.abs(cropMouse.curX - cropMouse.startX)
        const sh = Math.abs(cropMouse.curY - cropMouse.startY)
        root.cropMode = false
        if (sw < 3 || sh < 3)
            return
        const scale = toImageScale()
        bridge.crop_to(Math.floor(sx * scale), Math.floor(sy * scale),
                       Math.round(sw * scale), Math.round(sh * scale))
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
        if (root.moveMode) { root.moveMode = false; return }
        if (root.selectionMode) { root.selectionMode = false; return }
        if (root.selExists) { root.selW = 0; root.selH = 0; return }
        if (root.panelVisible) { root.panelVisible = false; return }
        if (root.cropMode) { root.cropMode = false; return }
        root.fullscreen = false
    }

    function toggleMode(m) {
        const wasActive = m === "crop" ? root.cropMode
                        : (m === "selection" ? root.selectionMode : root.moveMode)
        root.cropMode = false
        root.selectionMode = false
        root.moveMode = false
        if (!wasActive) {
            if (m === "crop") root.cropMode = true
            else if (m === "selection") root.selectionMode = true
            else root.moveMode = true
        }
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
            if (layerList[i].i === bridge.selected_layer)
                return layerList[i]
        return null
    }

    onVisibleChanged: bridge.mark("window-visible")

    onAfterRendering: {
        if (!firstFrameSeen) {
            firstFrameSeen = true;
            bridge.mark("first-frame");
        }
        if (imageRendered && !imageFrameSeen) {
            imageFrameSeen = true;
            bridge.mark("image-frame");
            if (bridge.auto_quit && !root.probeDialog)
                Qt.quit();
        }
    }

    Component.onCompleted: {
        bridge.mark("qml-completed");
        bridge.start();
        if (root.probeDialog)
            dialogProbeTimer.start();
    }
}