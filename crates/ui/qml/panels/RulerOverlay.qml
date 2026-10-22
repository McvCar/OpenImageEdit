import QtQuick
import com.imageedit

    // ---- 尺子覆盖层（Ruler）-------------------------------------------
    // 选中图层 / 选区：四角 + 中心显示「相对画布」的绝对坐标；鼠标位置
    // 跟随显示当前坐标。Y 轴方向由设置决定（向下 / 向上），纯显示层。
    Item {
        id: rulerOverlay
    required property var host
    required property var bridge
    required property var previewImage
        visible: host.rulerOn && bridge.image_width > 0
        z: 9
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height

        // 每个测量点的十字准星。
        Repeater {
            model: host.rulerPoints
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
            model: host.rulerPoints
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
            visible: host.rulerBox !== null
            width: rulerSizeText.implicitWidth + 10
            height: 16
            radius: 3
            color: "#d91d2126"
            border.color: "#99f0b23a"
            border.width: 1
            x: Math.max(0, Math.min(rulerOverlay.width - width,
                                    (host.rulerBox !== null
                                     ? (host.rulerBox.x + host.rulerBox.w / 2) * host.toDispScale()
                                     : 0) - width / 2))
            y: Math.max(0, Math.min(rulerOverlay.height - height,
                                    (host.rulerBox !== null
                                     ? (host.rulerBox.y + host.rulerBox.h) * host.toDispScale()
                                     : 0) + 6))
            Text {
                id: rulerSizeText
                anchors.centerIn: parent
                text: host.rulerBox !== null
                      ? Math.round(host.rulerBox.w) + " × " + Math.round(host.rulerBox.h)
                      : ""
                color: "#ffcf7d"
                font.pixelSize: 10
                font.family: "Consolas"
            }
        }

        // 鼠标位置：十字线 + 坐标读数。
        Item {
            visible: host.hoverValid
            x: host.hoverFx * host.toDispScale()
            y: host.hoverFy * host.toDispScale()
            Rectangle { x: -6; y: 0; width: 13; height: 1; color: "#4ad2ff" }
            Rectangle { x: 0; y: -6; width: 1; height: 13; color: "#4ad2ff" }
        }
        Rectangle {
            id: rulerHoverChip
            visible: host.hoverValid
            width: rulerHoverText.implicitWidth + 10
            height: 16
            radius: 3
            color: "#e61d2126"
            border.color: "#994ad2ff"
            border.width: 1
            x: Math.max(0, Math.min(rulerOverlay.width - width,
                                    host.hoverFx * host.toDispScale() + 11))
            y: Math.max(0, Math.min(rulerOverlay.height - height,
                                    host.hoverFy * host.toDispScale() + 11))
            Text {
                id: rulerHoverText
                anchors.centerIn: parent
                text: {
                    const ix = Math.floor(host.hoverFx)
                    const iy = Math.floor(host.hoverFy)
                    return ix + ", "
                           + (bridge.rulerBottomUp ? bridge.image_height - iy : iy)
                }
                color: "#9fe8ff"
                font.pixelSize: 10
                font.family: "Consolas"
            }
        }
    }

