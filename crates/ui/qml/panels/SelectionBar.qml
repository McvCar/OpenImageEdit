import QtQuick
import com.imageedit

// ---- 选区工具条（PS 的 options bar）----------------------------------
// 只在选区工具激活时出现（标题栏 "▭" 按钮 / M · L 快捷键）。
// 与标记工具栏同高、同色，叠在它下面一行。
Rectangle {
    id: selBar
    required property var host
    required property var bridge
    required property var toolbar
    required property var markBar

    visible: host.selBarVisible && host.selTool !== "none"
    anchors.top: markBar.visible ? markBar.bottom : toolbar.bottom
    anchors.left: parent.left
    anchors.right: parent.right
    height: host.toolbarHeight
    color: "#25272b"
    z: 58

    Row {
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        IconButton {
            glyph: "▭"
            tip: "矩形选框 (M)"
            selected: host.selTool === "rect"
            onClicked: host.setSelTool("rect")
        }
        IconButton {
            glyph: "◯"
            tip: "椭圆选框 (Shift+M)"
            selected: host.selTool === "ellipse"
            onClicked: host.setSelTool("ellipse")
        }
        IconButton {
            glyph: "∿"
            tip: "套索 (L)：按住拖出不规则选区"
            selected: host.selTool === "lasso"
            onClicked: host.setSelTool("lasso")
        }
        IconButton {
            glyph: "⬡"
            tip: "多边形套索 (Shift+L)：逐点点击，双击/回车闭合"
            selected: host.selTool === "polygon"
            onClicked: host.setSelTool("polygon")
        }

        IconButton { separator: true }

        IconButton {
            glyph: "▣"
            tip: "全选 (Ctrl+A)"
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

        IconButton { separator: true }

        IconButton {
            glyph: "Q"
            tip: "快速蒙版：把选区外的像素压暗，看清羽化软边 (Q)"
            selected: bridge.quickMask
            active: host.selExists
            onClicked: bridge.sel_toggle_quick_mask()
        }

        IconButton { separator: true }

        // 数值算子的快捷入口：点一下直接把光标送进右键菜单里的同一个
        // 输入框不现实，这里改成"展开菜单"的锚点（右键菜单在画布上）。
        IconButton {
            glyph: "◎"
            tip: "边界 / 平滑 / 羽化…（也可在选区上右键）"
            active: host.selExists
            onClicked: host.openSelMenu(selBar.mapToItem(host, 16, selBar.height).x,
                                        selBar.mapToItem(host, 16, selBar.height).y)
        }
    }

    Text {
        anchors.right: parent.right
        anchors.rightMargin: 12
        anchors.verticalCenter: parent.verticalCenter
        text: {
            if (host.selPolyActive)
                return "多边形套索：单击加点 · 双击/回车闭合 · Esc 放弃"
            if (host.selTool === "polygon")
                return "多边形套索：单击加点 · 双击/回车闭合 · Esc 放弃"
            if (host.selTool === "lasso")
                return "套索：按住拖出不规则选区 · Shift 加选 · Alt 减选"
            return host.selExists
                   ? ("选区 " + host.selImgW + " × " + host.selImgH
                      + "　Shift 加选 · Alt 减选 · Shift+Alt 交集")
                   : "拖出选区 · Shift 加选 · Alt 减选 · Shift+Alt 交集"
        }
        color: "#8b9096"
        font.pixelSize: 11
    }
}
