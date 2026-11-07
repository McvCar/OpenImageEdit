import QtQuick
import com.imageedit

// P0-4 智能参考线：静态洋红轴线 + 距离数值，无动效。
// 数据源是 Rust 每帧随 refresh 下发的 guides_json（显示像素坐标），
// 这里只做绘制，不参与求解、不写文档、不触发额外重绘。
Item {
    id: guides
    required property var bridge
    required property var previewImage

    readonly property var lines: {
        try { return JSON.parse(bridge.guidesJson) } catch (e) { return [] }
    }

    x: previewImage.x
    y: previewImage.y
    width: previewImage.width
    height: previewImage.height
    z: 7
    visible: bridge.showGuides && lines.length > 0
    // 控件层在画布之外照常绘制（D12 裁的是像素内容，不是控件）。
    clip: false

    Repeater {
        model: guides.lines

        Item {
            id: lineItem
            width: guides.width
            height: guides.height

            readonly property bool vertical: modelData.v === true
            readonly property real p: modelData.p
            readonly property string label: modelData.t || ""

            // 细洋红线贯穿画布（PS 的提示线也是通栏，不是只画一段）。
            Rectangle {
                x: lineItem.vertical ? Math.round(lineItem.p) : 0
                y: lineItem.vertical ? 0 : Math.round(lineItem.p)
                width: lineItem.vertical ? 1 : guides.width
                height: lineItem.vertical ? guides.height : 1
                color: "#ffe040fb"
                // 端点小方块，便于看清吸附到的是哪条边。
                border.width: 0
            }

            Rectangle {
                visible: lineItem.label.length > 0
                x: lineItem.vertical
                   ? Math.min(guides.width - 52, Math.round(lineItem.p) + 5)
                   : Math.min(guides.width - 52, guides.width * 0.5)
                y: lineItem.vertical
                   ? Math.min(guides.height - 20, guides.height * 0.5)
                   : Math.min(guides.height - 20, Math.round(lineItem.p) + 5)
                width: 48
                height: 16
                radius: 3
                color: "#cc2a0d2a"
                border.color: "#ffe040fb"
                border.width: 1

                Text {
                    anchors.centerIn: parent
                    text: lineItem.label
                    color: "#ffd8ff"
                    font.pixelSize: 10
                }
            }
        }
    }
}
