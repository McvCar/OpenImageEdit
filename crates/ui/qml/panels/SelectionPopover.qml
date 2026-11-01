import QtQuick
import com.imageedit

// ---- 选区浮动面板 ---------------------------------------------------
// 取代原来占满一行的固定选区工具条：标记工具栏里的「选区」按钮点开后
// 弹这个面板，选工具（矩形 / 椭圆 / 套索 / 多边形）后自动收起，把画布
// 让出来。全选 / 反选 / 取消 / 快速蒙版 也在这里，不用再记快捷键。
Rectangle {
    id: selPop
    required property var host
    required property var bridge
    required property var toolbar
    required property var markBar

    width: 236
    height: col.implicitHeight + 16
    visible: host.selPopoverOpen
    onVisibleChanged: if (visible) host.placePopover(selPop, markBar.selBtn)
    // 贴着标记栏下沿（和粗细/颜色弹窗同一套定位规则）。
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    color: "#2a2d31"
    border.color: "#3c4043"
    border.width: 1
    radius: 6
    z: 220

    // 面板自身吞掉点击：否则点在按钮之间的空白处会穿透到下面画布，
    // 把面板当成"没点中"直接触发一次选区手势。
    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.AllButtons
        z: -1
    }

    Column {
        id: col
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 8
        spacing: 6
        Text {
            text: "选区工具"
            color: "#8b9096"
            font.pixelSize: 10
        }

        // ---- 四种选区形状 ----
        // z: 2 是给 IconButton 的悬停提示让路：提示框是按钮的子项，竖排面板
        // 里它会被"下面几行的兄弟项"按声明顺序盖住（看起来就是提示文字被
        // 面板里更靠下的文字压住）。这行的提示还会向下伸进第二行按钮的
        // 位置，所以必须比第二行更高（第二行 z:1），否则露出半截。
        Row {
            spacing: 2
            z: 2
            IconButton {
                glyph: "▭"
                tip: "矩形选框 (M)"
                selected: host.selTool === "rect"
                onClicked: host.pickSelTool("rect")
            }
            IconButton {
                glyph: "◯"
                tip: "椭圆选框 (Shift+M)"
                selected: host.selTool === "ellipse"
                onClicked: host.pickSelTool("ellipse")
            }
            IconButton {
                glyph: "∿"
                tip: "套索 (L)：按住拖出不规则选区"
                selected: host.selTool === "lasso"
                onClicked: host.pickSelTool("lasso")
            }
            IconButton {
                glyph: "⬡"
                tip: "多边形套索 (Shift+L)：逐点点击，双击/回车闭合"
                selected: host.selTool === "polygon"
                onClicked: host.pickSelTool("polygon")
            }
        }

        Rectangle {
            width: parent.width
            height: 1
            color: "#3c4043"
        }

        // ---- 选区操作 ----
        // 同样 z: 1：这行的提示框要压住下面的分隔线和尺寸文字。
        Row {
            spacing: 2
            z: 1
            IconButton {
                glyph: "▣"
                tip: "全选 (Ctrl+A)"
                active: bridge.image_width > 0
                onClicked: bridge.sel_select_all()
            }
            IconButton {
                glyph: "⇄"
                tip: "选择反选 (Ctrl+Shift+I)"
                active: host.selExists
                onClicked: bridge.sel_invert()
            }
            IconButton {
                glyph: "✕"
                tip: "取消选择 (Ctrl+D)"
                active: host.selExists
                onClicked: bridge.sel_clear()
            }
            IconButton {
                glyph: "Q"
                tip: "快速蒙版：把选区外的像素压暗，看清羽化软边 (Q)"
                active: host.selExists
                selected: bridge.quickMask
                onClicked: bridge.sel_toggle_quick_mask()
            }
            IconButton {
                glyph: "◎"
                tip: "边界 / 平滑 / 羽化…（也可在选区上右键）"
                active: host.selExists
                onClicked: {
                    const p = selPop.mapToItem(host, 8, selPop.height)
                    host.selPopoverOpen = false
                    host.openSelMenu(p.x, p.y)
                }
            }
        }

        Rectangle {
            width: parent.width
            height: 1
            color: "#3c4043"
        }

        // ---- 当前选区尺寸 / 操作提示 ----
        Text {
            width: parent.width
            text: host.selExists
                  ? ("选区 " + host.selImgW + " × " + host.selImgH)
                  : "在画布上拖出选区"
            color: host.selExists ? "#e9eaec" : "#8b9096"
            font.pixelSize: 11
        }
        Text {
            width: parent.width
            wrapMode: Text.WordWrap
            text: host.selTool === "polygon"
                  ? "单击加点 · 双击/回车闭合 · Esc 放弃"
                  : (host.selTool === "lasso"
                     ? "按住拖出不规则选区"
                     : "Shift 加选 · Alt 减选 · Shift+Alt 交集")
            color: "#6b7076"
            font.pixelSize: 10
        }
    }
}
