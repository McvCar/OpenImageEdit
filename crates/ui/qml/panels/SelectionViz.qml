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
        // Selection outline: dashed frame + W×H pixel readout + blue dots.
        DashedRect {
            id: selRectViz
            visible: host.selExists
            z: 7
            x: previewImage.x
            y: previewImage.y
            width: previewImage.width
            height: previewImage.height
            rx: host.selX
            ry: host.selY
            rw: host.selW
            rh: host.selH
            dashColor: "#000000"
        }

        // Selection size badge: W×H only, in a small gray rounded chip so it
        // stays readable over any image content.
        Rectangle {
            id: selBadge
            visible: host.selExists && !host.rulerOn
            z: 8
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
                text: Math.round(host.selW / host.toDispScale()) + " × "
                      + Math.round(host.selH / host.toDispScale())
                color: "#d5d8dd"
                font.pixelSize: 11
            }
        }

        EdgeHandles {
            id: selHandles
            // 拾色器打开时隐藏：画布点击必须优先给取色，不能被选区手柄吞掉。
            visible: host.selExists && !host.selRubberBanding && bridge.image_width > 0
                     && !bridge.pickerOpen
            z: 8
            x: previewImage.x
            y: previewImage.y
            width: previewImage.width
            height: previewImage.height
            bx: host.selX
            by: host.selY
            bw: host.selW
            bh: host.selH
            pxScale: host.toDispScale()
            resizeFn: function(startRect, role, dx, dy, free) {
                host.resizeBoxFrom(startRect, role, dx, dy)
            }
            moveFn: function(startRect, dx, dy) {
                host.moveSelFrom(startRect, dx, dy)
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
            // 拾色器打开时隐藏（同 selHandles：取色优先于缩放/移动）。
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
