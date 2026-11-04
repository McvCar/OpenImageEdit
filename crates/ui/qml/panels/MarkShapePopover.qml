import QtQuick
import com.imageedit

// 形状 / 马赛克合并面板：矩形、椭圆、箭头、马赛克四个标记工具选一个。
// 工具栏原来摆四个按钮，现在只留一个，点开这里选，选完自动收起。
Rectangle {
    id: markShapePopover
    required property var host
    required property var bridge
    required property var toolbar
    required property var markBar
    visible: false
    z: 221
    width: 5 * 46 + 14
    height: 66
    color: "#2a2d31"
    border.color: "#3c4043"
    radius: 6
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    onVisibleChanged: if (visible) host.placePopover(markShapePopover, markBar.markShapeBtn)

    // 和工具栏按钮同一套图形，视觉上能对上号。
    component ShapeIcon: Canvas {
        id: ico
        property string kind: "rect"
        property color ink: "#e9eaec"
        width: 22
        height: 22
        onKindChanged: requestPaint()
        onInkChanged: requestPaint()
        onPaint: {
            const c = getContext("2d")
            c.clearRect(0, 0, width, height)
            c.strokeStyle = ink
            c.fillStyle = ink
            c.lineWidth = 1.5
            if (kind === "ellipse") {
                c.beginPath()
                c.ellipse(11, 11, 7.5, 5.5, 0, 0, Math.PI * 2)
                c.stroke()
            } else if (kind === "arrow") {
                c.beginPath()
                c.moveTo(5, 17); c.lineTo(16.5, 5.5)
                c.stroke()
                c.beginPath()
                c.moveTo(17, 5); c.lineTo(11.5, 6.5); c.lineTo(15.5, 10.5)
                c.closePath(); c.fill()
            } else if (kind === "brush") {
                // 自由画笔：一条手绘 S 形曲线 + 笔尖。
                c.beginPath()
                c.moveTo(4.5, 16.5)
                c.bezierCurveTo(8, 5.5, 14, 17.5, 18.5, 6.5)
                c.stroke()
            } else if (kind === "mosaic") {
                for (let y = 0; y < 3; ++y)
                    for (let x = 0; x < 3; ++x) {
                        c.fillStyle = (x + y) % 2 === 0 ? ink : "#8a8f96"
                        c.fillRect(4 + x * 5, 4 + y * 5, 4.4, 4.4)
                    }
            } else {
                c.strokeRect(4, 6, 14, 10)
            }
        }
    }

    Row {
        anchors.centerIn: parent
        spacing: 4

        Repeater {
            model: [
                { tool: "rect",    label: "矩形" },
                { tool: "ellipse", label: "椭圆" },
                { tool: "arrow",   label: "箭头" },
                { tool: "brush",   label: "画笔" },
                { tool: "mosaic",  label: "马赛克" }
            ]

            delegate: Item {
                required property var modelData
                width: 46
                height: 54

                Rectangle {
                    anchors.fill: parent
                    radius: 4
                    color: host.markTool === modelData.tool ? "#3d5a80"
                           : shapeHover.containsMouse ? "#3a3d42" : "transparent"
                }
                ShapeIcon {
                    anchors.horizontalCenter: parent.horizontalCenter
                    y: 8
                    kind: modelData.tool
                    ink: host.markTool === modelData.tool ? "#ffffff" : "#e9eaec"
                }
                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.bottom: parent.bottom
                    anchors.bottomMargin: 5
                    text: modelData.label
                    color: host.markTool === modelData.tool ? "#ffffff" : "#aab0b8"
                    font.pixelSize: 10
                }
                MouseArea {
                    id: shapeHover
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        host.chooseMarkTool(modelData.tool)
                        markShapePopover.visible = false
                    }
                }
            }
        }
    }
}
