import QtQuick
import com.imageedit

// 右上角「⋯」菜单：一级只放分类（文件/编辑/图像/选区/图层与标记/视图），
// 具体命令收进向左侧展开的二级面板。
//
// 原来是一条 40 多行的平铺长列表（≈1100px 高）。窗口一矮，下半截直接落在
// 窗口外，既看不到也点不到，而且 Rectangle 根本不会滚。现在两级面板各自
// 有高度上限、超出就滚（MenuPanel 负责），窗口再小也能翻到最后一项。
Item {
    id: overflowMenu
    required property var toolbar

    visible: false
    z: 210

    // 一级菜单条目（每次打开时从 TitleBar.menuTree() 重建，这样 ok 状态
    // 反映的是「这次打开时」的真实能力，而不是启动那一刻的）。
    property var items: []

    // 可用高度：窗口内容高度 - 标题栏 - 余量。低于 120 时按 120 算，
    // 让面板至少能露出 4 行。
    readonly property real maxMenuH:
        Math.max(120, (toolbar.host ? toolbar.host.height : 600)
                      - toolbar.height - 14)

    width: mainPanel.width
    height: mainPanel.height
    x: Math.max(4, toolbar.width - width - 10)
    y: toolbar.height + 2

    function open() {
        items = toolbar.menuTree()
        closeSub()
        visible = true
    }
    function close() {
        visible = false
        closeSub()
    }
    function toggle() {
        if (visible)
            close()
        else
            open()
    }

    function closeSub() {
        subPanel.visible = false
        mainPanel.expandedItem = null
    }
    function showSub(item, rowY) {
        subPanel.items = item.sub
        mainPanel.expandedItem = item
        // 子面板的最终高度要等 delegate 建完才准，先用行数估一个来夹位置。
        const est = Math.min(maxMenuH, item.sub.length * 26 + 10)
        subPanel.y = Math.max(0, Math.min(rowY, height - est))
        subPanel.visible = true
    }

    MenuPanel {
        id: mainPanel
        x: 0
        y: 0
        items: overflowMenu.items
        maxHeight: overflowMenu.maxMenuH
        onActivated: (item, rowY) => {
            if (item.sub) {
                overflowMenu.showSub(item, rowY)
            } else {
                overflowMenu.close()
                item.run()
            }
        }
        // 悬停到非分类行（分隔线/设置等）时收起二级面板，避免它挂在那儿
        // 跟当前高亮对不上。
        onHovered: (item, rowY) => {
            if (item.sub)
                overflowMenu.showSub(item, rowY)
            else
                overflowMenu.closeSub()
        }
    }

    MenuPanel {
        id: subPanel
        visible: false
        x: -width - 2
        y: 0
        maxHeight: overflowMenu.maxMenuH
        onActivated: (item) => {
            if (item.sub)
                return
            overflowMenu.close()
            item.run()
        }
    }
}
