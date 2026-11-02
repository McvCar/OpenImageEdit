import QtQuick
import com.imageedit

// 可滚动的文字菜单面板（一级菜单与二级菜单共用同一套）。
//
// 为什么要有高度上限：命令现在有 40 多条，平铺是 1100px 高的一整列。
// Rectangle 不滚动，窗口一矮，下半截就落在窗口外——既看不到也点不到。
// 现在面板高度被 maxHeight 夹住，超出部分用 Flickable 滚（滚轮/拖动都行），
// 右侧还有一条细滚动条提示「下面还有」。
//
// 条目结构：
//   { l: "显示文字", run: function(){}, ok: bool, sep: true, sub: [ ... ] }
//   sep: true          分隔线（忽略其它字段）
//   sub: [ ... ]       有二级菜单（右侧画 ‹，由宿主决定展开位置）
//   ok: false          置灰且点击无效
Rectangle {
    id: panel

    property var items: []
    // 面板最大高度；内容更矮时按内容收缩。
    property real maxHeight: 600
    // 当前展开了子菜单的那一条（对象本身，null = 无），只为让它保持高亮。
    property var expandedItem: null

    signal activated(var item, real rowY)
    // 悬停到某一行：rowY 是该行在面板坐标系里的顶部位置（含滚动偏移），
    // 宿主据此把二级面板对齐到这一行。
    signal hovered(var item, real rowY)

    width: 210
    height: Math.min(menuCol.implicitHeight + 10, maxHeight)
    radius: 6
    color: "#2a2d31"
    border.color: "#3c4043"
    border.width: 1

    Flickable {
        id: flick
        x: 4
        y: 5
        width: panel.width - 8
        height: panel.height - 10
        contentWidth: width
        contentHeight: menuCol.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick

        Column {
            id: menuCol
            width: flick.width
            spacing: 0

            Repeater {
                model: panel.items

                delegate: MenuRow {
                    id: row
                    required property var modelData
                    required property int index
                    width: menuCol.width
                    label: modelData.l !== undefined && modelData.l !== null
                           ? modelData.l : ""
                    separator: modelData.sep === true
                    hasSub: modelData.sub !== undefined && modelData.sub !== null
                    expanded: panel.expandedItem === modelData
                    active: modelData.sep === true || modelData.ok !== false
                    // 滚过一段以后 row.y 还是内容坐标，得减掉 contentY 才是
                    // 视觉位置，否则二级面板会跑偏。
                    readonly property real rowY: 5 + row.y - flick.contentY
                    onHoveredChanged: if (row.hovered)
                        panel.hovered(modelData, row.rowY)
                    onActivated: panel.activated(modelData, row.rowY)
                }
            }
        }
    }

    // 细滚动条：只在内容超出时出现。
    Rectangle {
        visible: flick.contentHeight > flick.height + 1
        x: panel.width - 7
        width: 3
        radius: 1.5
        color: "#5b6168"
        height: Math.max(18, flick.height * flick.height / flick.contentHeight)
        y: 5 + (flick.height - height)
           * (flick.contentY / Math.max(1, flick.contentHeight - flick.height))
    }
}
