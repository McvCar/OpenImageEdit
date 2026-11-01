import QtQuick
import com.imageedit

Item {
    id: resizeOverlay
    required property var bridge
    required property var host
    anchors.fill: parent
    visible: false
    z: 100
    property bool keepRatio: false
    // Canvas-only mode: change the canvas resolution WITHOUT resampling
    // the image (content is re-anchored to the new center).
    property bool canvasOnly: false
    // Percent mode: scale by a percentage of the current resolution
    // instead of typing absolute pixel dimensions.
    property bool percentMode: false
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

            // Mode switch: absolute pixels vs percentage of current size.
            Row {
                spacing: 8

                ToolButton {
                    label: "按分辨率"
                    selected: !resizeOverlay.percentMode
                    onClicked: resizeOverlay.percentMode = false
                }
                ToolButton {
                    label: "按百分比"
                    selected: resizeOverlay.percentMode
                    onClicked: resizeOverlay.percentMode = true
                }
            }

            Row {
                spacing: 8
                visible: !resizeOverlay.percentMode

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

            // Percent input: one field, width/height scale together.
            Row {
                spacing: 8
                visible: resizeOverlay.percentMode

                Text {
                    text: "缩放"
                    color: "#b8bcc0"
                    font.pixelSize: 12
                    anchors.verticalCenter: parent.verticalCenter
                }

                Item {
                    width: 64
                    height: 26
                    anchors.verticalCenter: parent.verticalCenter

                    Rectangle {
                        anchors.fill: parent
                        color: "#1b1c1e"
                        border.color: resizePct.activeFocus ? "#6aa9ff" : "#3c4043"
                        border.width: resizePct.activeFocus ? 2 : 1
                        radius: 3
                    }

                    TextInput {
                        id: resizePct
                        anchors.fill: parent
                        anchors.margins: 5
                        text: "100"
                        font.pixelSize: 13
                        color: "#e9eaec"
                        verticalAlignment: TextInput.AlignVCenter
                        validator: IntValidator { bottom: 1; top: 1000 }
                        selectByMouse: true
                        onActiveFocusChanged: if (activeFocus) selectAll()
                        onAccepted: commitResize()
                    }
                }

                Text {
                    text: "%"
                    color: "#6b7076"
                    font.pixelSize: 11
                    anchors.verticalCenter: parent.verticalCenter
                }

                Text {
                    text: {
                        const p = parseInt(resizePct.text)
                        if (isNaN(p) || p < 1)
                            return "目标：—"
                        const w = Math.max(1, Math.round(bridge.image_width * p / 100))
                        const h = Math.max(1, Math.round(bridge.image_height * p / 100))
                        return "目标：" + w + " × " + h
                    }
                    color: "#8f96a3"
                    font.pixelSize: 11
                    anchors.verticalCenter: parent.verticalCenter
                }
            }

            CheckRow {
                id: ratioCheck
                label: "保持宽高比"
                visible: !resizeOverlay.percentMode
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
    resizePct.text = "100"
    resizeOverlay.keepRatio = true
    resizeOverlay.canvasOnly = false
    resizeOverlay.percentMode = false
    resizeOverlay.visible = true
}

function commitResize() {
    let w = parseInt(resizeW.text)
    let h = parseInt(resizeH.text)
    if (resizeOverlay.percentMode) {
        // Uniform scale: width and height move together, keepRatio has
        // no meaning here.
        const p = parseInt(resizePct.text)
        if (isNaN(p) || p < 1)
            return
        w = Math.max(1, Math.round(bridge.image_width * p / 100))
        h = Math.max(1, Math.round(bridge.image_height * p / 100))
    } else if (resizeOverlay.keepRatio && !resizeOverlay.canvasOnly) {
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
    // 画布尺寸变了，选区（画布坐标）就不再指向原来的内容：直接丢掉。
    // Rust 侧的 crop/resize 也会清，这里补一刀覆盖 QML 直接改尺寸的路径。
    bridge.sel_clear()
    host.cancelLayerScale()
    resizeOverlay.visible = false
}
}
