import QtQuick
import com.imageedit

Item {
    id: layerPanel
    required property var host
    required property var bridge
    required property var toolbar
    required property var statusBar
    anchors.top: toolbar.bottom
    anchors.right: parent.right
    anchors.bottom: statusBar.top
    width: 230
    visible: host.panelVisible
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
                model: host.layerPanelList

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
                        const docLen = host.nonBaseRowCount()
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
                            const docLen = host.nonBaseRowCount()
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
                            bridge.sel_clear()
                            bridge.select_layer(layerRow.row.i)
                        }
                        reordering = false
                        toPos = -1
                        dropIndicator.visible = false
                    }
                    onDoubleClicked: if (layerRow.row && !layerRow.isBase) {
                        host.renamingIndex = layerRow.row.i
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
                    visible: host.renamingIndex !== -1 && host.renamingIndex === (layerRow.row ? layerRow.row.i : -1)
                    text: layerRow.row ? layerRow.row.n : ""
                    color: "#ffffff"
                    font.pixelSize: 12
                    focus: visible
                    onAccepted: {
                        if (text.trim().length > 0)
                            bridge.rename_selected_layer(text)
                        host.renamingIndex = -1
                    }
                    onActiveFocusChanged: if (!activeFocus && visible)
                        host.renamingIndex = -1
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
                visible: host.markList.length > 0
            }

            Text {
                id: markSectionHdr
                width: layerRepeaterColumn.width
                text: "标记（" + host.markList.length + "）"
                color: "#e9eaec"
                font.pixelSize: 12
                font.bold: true
                visible: host.markList.length > 0
            }

            Repeater {
                model: host.markList

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
                    value: host.selLayer ? host.selLayer.o / 100.0 : 1.0
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
                label: "旋转角度…"
                active: layerContext.targetIndex >= 0 && !layerContext.targetLocked
                onActivated: {
                    layerContext.visible = false
                    const i = layerContext.targetIndex
                    if (i >= 0)
                        host.openRotateLayer(i)
                }
            }
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
    // Esc 处理器（EditorContent）需要感知/关闭右键菜单。
    property bool contextVisible: layerContext.visible
    function closeContext() { layerContext.visible = false }
}
