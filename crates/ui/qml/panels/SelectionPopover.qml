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
            IconButton {
                glyph: "✦"
                tip: "魔术棒 (W)：按颜色自动选取相连（或全图同色）区域"
                selected: host.selTool === "wand"
                onClicked: host.pickSelTool("wand")
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

        // ---- 魔术棒参数（只在魔术棒工具下出现）----
        // PS 的魔棒就三个关键项：容差、连续、取样范围。容差实时写回
        // settings.ini，下次启动继续用同一组数。
        Column {
            id: wandBox
            width: parent.width
            spacing: 5
            visible: host.selTool === "wand"

            Row {
                spacing: 6
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "容差"
                    color: "#e9eaec"
                    font.pixelSize: 11
                }
                SpinBox {
                    id: wandTol
                    width: 62
                    from: 0
                    to: 255
                    step: 4
                    value: host.wandTolerance
                    onEdited: (v) => host.setWandTolerance(v)
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: host.wandTolerance === 0 ? "只选完全同色" : "越大吃得越宽"
                    color: "#6b7076"
                    font.pixelSize: 10
                }
            }
            Row {
                spacing: 4
                Pill {
                    label: "连续"
                    checked: host.wandContiguous
                    onClicked: host.setWandContiguous(true)
                }
                Pill {
                    label: "全图同色"
                    checked: !host.wandContiguous
                    onClicked: host.setWandContiguous(false)
                }
            }
            Row {
                spacing: 4
                Pill {
                    label: "当前图层"
                    checked: !host.wandSampleAll
                    onClicked: host.setWandSampleAll(false)
                }
                Pill {
                    label: "全部图层"
                    checked: host.wandSampleAll
                    onClicked: host.setWandSampleAll(true)
                }
            }
        }

        Rectangle {
            width: parent.width
            height: 1
            color: "#3c4043"
            visible: wandBox.visible
        }

        // ---- 当前选区尺寸 / 操作提示 ----
        Text {
            width: parent.width
            text: host.selExists
                  ? ("选区 " + host.selImgW + " × " + host.selImgH)
                  : (host.selTool === "wand" ? "在画布上点击取色" : "在画布上拖出选区")
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
                     : (host.selTool === "wand"
                        ? "点击取色选中一片 · Shift 加选 · Alt 减选"
                        : "Shift 加选 · Alt 减选 · Shift+Alt 交集"))
            color: "#6b7076"
            font.pixelSize: 10
        }
    }

    // 魔术棒参数用的两态小胶囊（连续/全图、当前层/全部层）。
    component Pill: Rectangle {
        property string label: ""
        property bool checked: false
        signal clicked()

        width: labelText.implicitWidth + 16
        height: 20
        radius: 4
        color: checked ? "#3d5a80" : (pillHover.containsMouse ? "#3a3d42" : "transparent")
        border.color: checked ? "#5b9dff" : "#45484d"
        border.width: 1

        Text {
            id: labelText
            anchors.centerIn: parent
            text: parent.label
            color: parent.checked ? "#cfe4ff" : "#b9bec5"
            font.pixelSize: 10
        }
        MouseArea {
            id: pillHover
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: parent.clicked()
        }
    }
}
