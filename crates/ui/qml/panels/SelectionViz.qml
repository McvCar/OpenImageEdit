import QtQuick
import com.imageedit

Item {
    id: selectionViz
    required property var host
    required property var bridge
    required property var previewImage
    z: 8
    // 暴露给 EditorContent：canvasCursor 读手柄几何算光标。
    readonly property alias layerHandles: layerHandles

    // ---- 选区轮廓（蚂蚁线）--------------------------------------------
    // 轮廓真值在 Rust（mask 的 marching squares 轮廓，图像坐标），QML 只在
    // selVersion 变化时拉一次，避免每次缩放/重绘都跨语言跑几十万像素。
    Canvas {
        id: ants
        visible: host.selExists
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        // 蚂蚁线要在静止场景里持续流动，必须用 Immediate + Image
        //（Cooperative 策略会把单次 requestPaint 丢掉）。
        renderStrategy: Canvas.Immediate
        renderTarget: Canvas.Image
        antialiasing: false

        // 缓存的轮廓（图像坐标）：[[ [x,y], ... ], ...]
        property var rings: []
        property real phase: 0

        // selVersion 变化 = 选区真的变了（标量变化才触发，避免重复拉取）。
        Connections {
            target: bridge
            function onSelVersionChanged() { ants.reload() }
            function onImage_widthChanged() { ants.reload() }
        }
        // 缩放/窗口变化只影响绘制（坐标 × 比例），不用重新拉数据。
        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()
        Component.onCompleted: reload()
        // selExists 由 bridge.selActive 驱动：出现/消失时也刷新一次
        //（selVersion 之外的第二道保险，信号名对不上也不会不刷新）。
        Connections {
            target: host
            function onSelExistsChanged() { ants.reload() }
        }

        function reload() {
            if (!host.selExists) {
                rings = []
                requestPaint()
                return
            }
            try {
                rings = JSON.parse(bridge.sel_outline())
            } catch (e) {
                bridge.mark("ANTSPARSE-ERR " + e)
                rings = []
            }
            requestPaint()
        }

        // 虚线相位：60ms 一跳，视觉上就是 PS 的蚂蚁线。
        Timer {
            interval: 60
            repeat: true
            running: ants.visible
            onTriggered: {
                ants.phase = (ants.phase + 1) % 8
                ants.requestPaint()
            }
        }

        onPaint: {
            try {
                var ctx = getContext("2d")
                ctx.clearRect(0, 0, width, height)
                if (!host.selExists || rings.length === 0)
                    return
                const s = host.toDispScale()
                if (s <= 0)
                    return
                // PS 的蚂蚁线 = 白底 + 黑虚线：白段在黑底照片上可见，黑段
                // 在白底照片上可见。**先**铺白实线，再叠黑虚线（顺序反了
                // 就只剩黑虚线，浅色照片上整圈看不见）。
                ctx.lineWidth = 1
                ctx.setLineDash([])
                ctx.strokeStyle = "#ffffff"
                ctx.beginPath()
                for (var i = 0; i < rings.length; i++) {
                    const ring = rings[i]
                    if (!ring || ring.length < 2)
                        continue
                    ctx.moveTo(ring[0][0] * s, ring[0][1] * s)
                    for (var j = 1; j < ring.length; j++)
                        ctx.lineTo(ring[j][0] * s, ring[j][1] * s)
                }
                ctx.stroke()
                // 上面再走一条黑色虚线，相位滚动 = 流动。
                ctx.setLineDash([4, 4])
                ctx.lineDashOffset = -phase
                ctx.strokeStyle = "rgba(0,0,0,0.9)"
                ctx.stroke()
                ctx.setLineDash([])
            } catch (e) {
                bridge.mark("ANTSPAINT-ERR " + e)
            }
        }
    }

    // Selection size badge: W×H only, in a small gray rounded chip so it
    // stays readable over any image content.
    Rectangle {
        id: selBadge
        visible: host.selExists && !host.rulerOn && !host.selRubberBanding
        z: 8
        // host.selX/selY/selW/selH 已经是「显示坐标」（EditorContent 里
        // 由 selImg* × toDispScale() 算好），这里不要再乘一次比例。
        x: previewImage.x
           + Math.max(0, Math.min(host.selX,
               previewImage.width - selBadge.width))
        y: previewImage.y
           + Math.min(previewImage.height - selBadge.height,
                      host.selY + host.selH + 4)
        width: selBadgeLabel.implicitWidth + 12
        height: selBadgeLabel.implicitHeight + 6
        radius: 3
        color: "#43474d"
        border.color: "#6b7076"
        border.width: 1

        Text {
            id: selBadgeLabel
            anchors.centerIn: parent
            text: host.selImgW + " × " + host.selImgH
                  + (bridge.quickMask ? "　快速蒙版" : "")
            color: "#d5d8dd"
            font.pixelSize: 11
        }
    }

    // Selected layer outline (dashed) + blue-dot handles for scaling.
    DashedRect {
        id: layerBoxViz
        visible: host.selLayer !== null && host.scaleW === 0
        z: 6
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        rx: host.selLayerBox !== null ? host.selLayerBox.x * host.toDispScale() : 0
        ry: host.selLayerBox !== null ? host.selLayerBox.y * host.toDispScale() : 0
        rw: host.selLayerBox !== null ? host.selLayerBox.w * host.toDispScale() : 0
        rh: host.selLayerBox !== null ? host.selLayerBox.h * host.toDispScale() : 0
        dashColor: "#5b9dff99"
    }

    EdgeHandles {
        id: layerHandles
        // 拾色器打开时隐藏（取色优先于缩放/移动）。
        visible: host.selLayer !== null && host.selLayer.base !== true
                 && !host.selExists && bridge.image_width > 0
                 && !bridge.pickerOpen
        z: 8
        x: previewImage.x
        y: previewImage.y
        width: previewImage.width
        height: previewImage.height
        bx: host.scaleW > 0 ? host.scaleX
                            : (host.selLayerBox !== null ? host.selLayerBox.x * host.toDispScale() : 0)
        by: host.scaleW > 0 ? host.scaleY
                            : (host.selLayerBox !== null ? host.selLayerBox.y * host.toDispScale() : 0)
        bw: host.scaleW > 0 ? host.scaleW
                            : (host.selLayerBox !== null ? host.selLayerBox.w * host.toDispScale() : 0)
        bh: host.scaleW > 0 ? host.scaleH
                            : (host.selLayerBox !== null ? host.selLayerBox.h * host.toDispScale() : 0)
        moveEnabled: false
        pxScale: host.toDispScale()
        resizeFn: function(startRect, role, dx, dy, free) {
            host.resizeLayerFrom(startRect, role, dx, dy, free)
        }
        onEngagedChanged: {
            if (layerHandles.engaged) {
                if (host.scaleW === 0)
                    bridge.scale_layer_begin()
            } else if (host.scaleW > 0) {
                host.commitLayerScale()
            }
        }
    }

    Text {
        visible: host.scaleW > 0
        z: 8
        x: previewImage.x + host.scaleX
        y: Math.min(previewImage.y + previewImage.height - 16,
                    previewImage.y + host.scaleY + host.scaleH + 4)
        text: Math.round(host.scaleW / host.toDispScale()) + " × "
              + Math.round(host.scaleH / host.toDispScale()) + "　松手应用缩放 · Esc 取消"
        color: "#7fc3ff"
        font.pixelSize: 11
    }
}
