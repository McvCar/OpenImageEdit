import QtQuick
import com.imageedit

// ---- 图层旋转角度（图层面板右键 / 「图像」菜单打开）------------------
// 非 90° 的旋转必须重采样位图，所以这里给的是「输入角度 + 实时预览」：
// 每次改数值都从旋转前的状态重算，取消就原样还原。
Item {
    id: rotateOverlay
    required property var host
    required property var bridge
    anchors.fill: parent
    visible: false
    z: 100

    // 当前输入的角度（整数度；负数逆时针）。
    property int angle: 0
    // 扩展图层边界（默认开）：内容不被裁切；关掉则保持原尺寸。
    property bool expand: true
    property int targetIndex: -1

    function open(index) {
        if (index < 0)
            return
        targetIndex = index
        angle = 0
        expand = true
        // ★keep_session = true：连续多次打开对话框输入不同角度时，沿用上一次
        //   留下的**未插值原图**累加角度。以前用 false，每次都拿「已经烘焙成
        //   外接矩形的位图」再转一遍 —— 转几次随机角度，四角透明区域反复
        //   叠外接矩形，框就一路变大、图也越来越糊。
        //   角度 0 仍然等于现状（total = base + 0，重采样结果与当前相同）。
        bridge.rotate_layer_begin(index, true)
        visible = true
        previewTimer.restart()
    }

    function close(apply) {
        previewTimer.stop()
        if (targetIndex >= 0 && bridge.rotate_layer_state().length > 0)
            bridge.rotate_layer_finish(apply, angle, expand)
        targetIndex = -1
        visible = false
    }

    // 数值每改一下就重算一次代价不小（整层重采样），防抖 120ms。
    Timer {
        id: previewTimer
        interval: 120
        repeat: false
        onTriggered: {
            if (rotateOverlay.visible && rotateOverlay.targetIndex >= 0)
                bridge.rotate_layer_update(rotateOverlay.angle,
                                           rotateOverlay.expand)
        }
    }

    MouseArea { anchors.fill: parent }

    Rectangle {
        id: dialog
        width: 320
        height: col.implicitHeight + 28
        x: (parent.width - width) / 2
        y: (parent.height - height) / 2
        radius: 8
        color: "#2b2e33"
        border.color: "#3c4043"

        Column {
            id: col
            anchors.top: parent.top
            anchors.topMargin: 14
            anchors.left: parent.left
            anchors.leftMargin: 14
            anchors.right: parent.right
            anchors.rightMargin: 14
            spacing: 12

            Row {
                width: parent.width
                spacing: 8
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "旋转图层"
                    color: "#e9eaec"
                    font.pixelSize: 14
                    font.bold: true
                }
                Item { width: parent.width - 120; height: 1 }
                IconButton {
                    glyph: "✕"
                    tip: "取消 (Esc)"
                    onClicked: rotateOverlay.close(false)
                }
            }

            Rectangle { width: parent.width; height: 1; color: "#3c4043" }

            Row {
                spacing: 8
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 40
                    text: "角度"
                    color: "#b8bcc0"
                    font.pixelSize: 12
                }
                SpinBox {
                    width: 74
                    from: -180
                    to: 180
                    step: 1
                    value: rotateOverlay.angle
                    onEdited: (v) => {
                        rotateOverlay.angle = v
                        previewTimer.restart()
                    }
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "度（负数 = 逆时针）"
                    color: "#6b7076"
                    font.pixelSize: 10
                }
            }

            Row {
                spacing: 6
                ToolButton { label: "−90°"; onClicked: rotateOverlay.nudge(-90) }
                ToolButton { label: "−15°"; onClicked: rotateOverlay.nudge(-15) }
                ToolButton { label: "+15°"; onClicked: rotateOverlay.nudge(15) }
                ToolButton { label: "+90°"; onClicked: rotateOverlay.nudge(90) }
            }

            CheckRow {
                width: parent.width
                label: "扩展图层边界（不裁掉旋转后的四角）"
                checked: rotateOverlay.expand
                onToggled: {
                    rotateOverlay.expand = !rotateOverlay.expand
                    previewTimer.restart()
                }
            }

            Text {
                width: parent.width
                wrapMode: Text.WordWrap
                text: "任意角度都会重采样一次位图（轻微软化）。\n"
                      + "画布上的图层框上方也有旋转手柄，可以拖着转。"
                color: "#6b7076"
                font.pixelSize: 10
            }

            Row {
                width: parent.width
                spacing: 8
                Item { width: parent.width - 160; height: 1 }
                ToolButton {
                    label: "取消"
                    onClicked: rotateOverlay.close(false)
                }
                ToolButton {
                    label: "应用"
                    selected: true
                    onClicked: rotateOverlay.close(true)
                }
            }
        }
    }

    function nudge(delta) {
        let v = rotateOverlay.angle + delta
        if (v > 180) v -= 360
        if (v < -180) v += 360
        rotateOverlay.angle = v
        previewTimer.restart()
    }
}
