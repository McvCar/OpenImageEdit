import QtQuick
import com.imageedit

// 图层面板（P2 重做）。
//
// 设计要点：
// * 背景、位图图层、矢量标记共用同一个列表 —— 模型直接来自
//   `bridge.layers_json`，行顺序就是 z 序（顶部 = 最上层）。
// * 行高 44：缩略图 34 + 名称 + 一行副信息（尺寸 / 位置 / 类型）。
// * 多选：Ctrl 点击切换、Shift 连选；主选中层仍是 `bridge.selected_layer`，
//   拖动 / 变换只作用于它。批量命令走 `layerSelectionIds` 那套。
// * 命令只放在按钮条和右键菜单里，不绑快捷键。
Item {
    id: layerPanel
    required property var host
    required property var bridge
    required property var toolbar
    required property var statusBar
    anchors.top: toolbar.bottom
    // 二级工具栏（标记栏）是全宽横条、层级也比面板高：它一出现，面板就整体
    // 下移一行，两者上下排开互不重叠。以前是让工具栏在右侧缩进，窗口一窄
    // 按钮行就没地方放、被裁掉半行。
    anchors.topMargin: host.markBarVisible ? host.toolbarHeight : 0
    anchors.right: parent.right
    anchors.bottom: statusBar.top
    // 宽度单一来源在 host。
    width: host.layerPanelWidth
    visible: host.panelVisible
    z: 50
    clip: true

    readonly property int rowHeight: 44
    readonly property int rowSpacing: 3
    readonly property int multiCount: bridge.selectedLayerCount
    readonly property bool hasGroups: {
        try {
            return JSON.parse(bridge.groups_json).length > 0
        } catch (e) {
            return false
        }
    }
    // PSD 阶段 2：图层后台加载完成前，面板所有命令一律灰掉并给进度提示。
    readonly property bool editLocked: bridge.editingLocked
    // 按钮 hover 时的说明文字（面板里没有原生 tooltip，用一个固定行显示，
    // 免得浮层跑出面板边界）。
    property string hintText: ""
    // 组标题行的内联重命名（-1 = 没有）。折叠态也能改名，所以按组 id
    // 而不是显示行下标来记。
    property int renamingGroupId: -1
    // 选中的 id 集合：面板高亮用它，而不是下标（重排后下标会变）。
    readonly property var selIds: {
        try {
            return JSON.parse(bridge.layerSelectionIds)
        } catch (e) {
            return []
        }
    }
    function isSelectedRow(row) {
        if (row === null || row === undefined)
            return false
        // 组标题行：整组成员都被选中时高亮（点组标题 = 选中整组）。
        if (row.k === "group") {
            const mids = row.mids || []
            if (mids.length === 0)
                return false
            for (var i = 0; i < mids.length; i++)
                if (selIds.indexOf(mids[i]) < 0)
                    return false
            return true
        }
        return selIds.indexOf(row.id) >= 0
    }

    // ---- 小按钮（面板里到处是这种 26×24 的方形图标键）----
    //
    // 图标用 Canvas 画、不用字体符号：`⇤ ↔ ⇥` 这类符号在 Segoe UI Symbol 里
    // 字宽/基线各不相同，高 DPI 下发虚而且六个对齐键长得几乎一样。
    // `icon` 非空时走矢量路径，`glyph` 仅留给 +/⧉/✕ 这类辨识度够的字符。
    component PanelButton: Rectangle {
        id: btn
        property string glyph: ""
        property string icon: ""
        property string tip: ""
        property bool usable: true
        property bool danger: false
        // 编辑锁是全局的，按钮自身的 usable 只管「本命令当前是否可用」。
        readonly property bool effectiveUsable: usable && !layerPanel.editLocked
        signal activated()
        // 不可用时被点了一下。★默认的写法是给 MouseArea 挂 `enabled: usable`，
        // 结果按钮变灰之后连 hover 都没了：没有说明文字、点下去也毫无反馈，
        // 用户只能得出「这功能用不了」。所以 MouseArea 常开，由这里兜底解释。
        signal blocked()
        width: 26
        height: 24
        radius: 4
        color: !effectiveUsable ? "transparent"
             : (btnMa.containsMouse ? (btn.danger ? "#5d2c30" : "#414852")
                                    : "#2c3137")
        border.width: effectiveUsable ? 1 : 0
        border.color: btn.danger ? "#4d3033" : "#3a4048"
        opacity: effectiveUsable ? 1.0 : 0.35

        Text {
            anchors.centerIn: parent
            visible: btn.icon === ""
            text: btn.glyph
            color: btnMa.containsMouse && btn.effectiveUsable
                   ? (btn.danger ? "#ff9b9b" : "#ffffff") : "#c6ccd3"
            font.pixelSize: 13
            font.family: "Segoe UI Symbol"
        }

        Canvas {
            anchors.centerIn: parent
            width: 16
            height: 16
            visible: btn.icon !== ""
            property string kind: btn.icon
            property color tint: btnMa.containsMouse && btn.effectiveUsable
                                 ? (btn.danger ? "#ff9b9b" : "#ffffff") : "#c6ccd3"
            onKindChanged: requestPaint()
            onTintChanged: requestPaint()
            onPaint: {
                var ctx = getContext("2d")
                ctx.clearRect(0, 0, width, height)
                ctx.fillStyle = tint
                ctx.strokeStyle = tint
                ctx.lineWidth = 1.6
                // 参考线（对齐的那条边/中轴）：贯穿整个图标的细线。
                function railV(x) {
                    ctx.beginPath(); ctx.moveTo(x, 1.2); ctx.lineTo(x, 14.8); ctx.stroke()
                }
                function railH(y) {
                    ctx.beginPath(); ctx.moveTo(1.2, y); ctx.lineTo(14.8, y); ctx.stroke()
                }
                // 两个宽度不同的方块，贴住参考线——Photoshop 同款表达方式。
                function bar(x0, y0, x1, y1) { ctx.fillRect(x0, y0, x1 - x0, y1 - y0) }
                var k = kind
                if (k === "left") {
                    bar(4, 3, 14, 7); bar(4, 9, 10.5, 13); railV(1.6)
                } else if (k === "right") {
                    bar(2, 3, 12, 7); bar(5.5, 9, 12, 13); railV(14.4)
                } else if (k === "hcenter") {
                    bar(4, 3, 12, 7); bar(5.5, 9, 10.5, 13); railV(8)
                } else if (k === "top") {
                    bar(3, 4, 7, 14); bar(9, 4, 13, 10.5); railH(1.6)
                } else if (k === "bottom") {
                    bar(3, 2, 7, 12); bar(9, 5.5, 13, 12); railH(14.4)
                } else if (k === "vcenter") {
                    bar(3, 4, 7, 12); bar(9, 5.5, 13, 10.5); railH(8)
                } else if (k === "spread-h") {
                    bar(2, 2, 5, 14); bar(6.5, 2, 9.5, 14); bar(11, 2, 14, 14)
                } else if (k === "spread-v") {
                    bar(2, 2, 14, 5); bar(2, 6.5, 14, 9.5); bar(2, 11, 14, 14)
                } else if (k === "group" || k === "ungroup") {
                    // 文件夹轮廓；ungroup 再压一道斜杠表示「拆开」。
                    ctx.beginPath()
                    ctx.moveTo(1.5, 4); ctx.lineTo(6, 4); ctx.lineTo(7.5, 6)
                    ctx.lineTo(14.5, 6); ctx.lineTo(14.5, 13)
                    ctx.lineTo(1.5, 13); ctx.closePath(); ctx.stroke()
                    if (k === "group") {
                        railH(9.6)
                    } else {
                        ctx.beginPath()
                        ctx.moveTo(4, 1.5); ctx.lineTo(12, 14.5); ctx.stroke()
                    }
                }
            }
        }

        MouseArea {
            id: btnMa
            anchors.fill: parent
            hoverEnabled: true
            onContainsMouseChanged: {
                if (containsMouse)
                    layerPanel.hintText = btn.tip
                else if (layerPanel.hintText === btn.tip)
                    layerPanel.hintText = ""
            }
            cursorShape: btn.effectiveUsable ? Qt.PointingHandCursor : Qt.ArrowCursor
            onClicked: {
                if (layerPanel.editLocked) {
                    layerPanel.hintText = "图层仍在后台加载，可点「跳过图层加载」"
                    bridge.psd_locked_hint()
                } else if (btn.effectiveUsable) {
                    btn.activated()
                } else {
                    // 灰按钮也要能问出「为什么」：把 tip 亮在提示行里，
                    // 再让具体按钮补一句人话（如「先 Ctrl 选中两层」）。
                    layerPanel.hintText = btn.tip
                    btn.blocked()
                }
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        color: "#1d2024"
    }

    // ================= 头部 =================
    Item {
        id: header
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: headerCol.height + 16

        Column {
            id: headerCol
            anchors.top: parent.top
            anchors.topMargin: 10
            anchors.left: parent.left
            anchors.leftMargin: 10
            anchors.right: parent.right
            anchors.rightMargin: 10
            spacing: 8

            Row {
                width: parent.width
                spacing: 6

                Text {
                    text: "图层"
                    color: "#eef1f4"
                    font.pixelSize: 13
                    font.bold: true
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: bridge.layer_count > 0
                          ? (layerPanel.multiCount > 1
                             ? "已选 " + layerPanel.multiCount + " / " + bridge.layer_count
                             : "共 " + bridge.layer_count + " 个")
                          : "暂无图层"
                    color: "#7b828a"
                    font.pixelSize: 11
                }
            }

            // ---- PSD 阶段 2：逐层进度 + 「跳过图层加载」逃生口 ----
            // 用户定稿：阶段 2 期间禁用全部编辑工具，所以必须能看见进度、
            // 并且随时能一键解锁，避免大文件把人锁死（C2-a / C2-b）。
            Rectangle {
                width: parent.width
                height: 42
                visible: layerPanel.editLocked
                color: "#26313d"
                border.color: "#46617c"
                border.width: 1
                radius: 5

                Text {
                    anchors.left: parent.left
                    anchors.leftMargin: 8
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.top: parent.top
                    anchors.topMargin: 4
                    text: bridge.psdProgress.length > 0
                          ? bridge.psdProgress : "正在加载图层…"
                    color: "#dce6f0"
                    font.pixelSize: 10
                    elide: Text.ElideRight
                }
                Text {
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.bottom: parent.bottom
                    anchors.bottomMargin: 4
                    text: "跳过图层加载，以合并预览继续编辑 ›"
                    color: "#8ec5ff"
                    font.pixelSize: 10
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: bridge.skip_psd_layers()
                    }
                }
            }

            // 第一排：新建 / 复制 / 合并选中层 …… 删除（右端独立，危险色）
            Item {
                width: parent.width
                height: 24

                PanelButton {
                    id: btnNew
                    anchors.left: parent.left
                    glyph: "＋"
                    tip: "新建空图层"
                    usable: bridge.image_width > 0
                    onActivated: bridge.new_layer()
                }
                PanelButton {
                    id: btnDup
                    anchors.left: btnNew.right
                    anchors.leftMargin: 4
                    glyph: "⧉"
                    tip: "复制图层"
                    usable: bridge.selected_layer >= 0
                    onActivated: bridge.duplicate_selected_layer()
                }
                PanelButton {
                    id: btnMerge
                    anchors.left: btnDup.right
                    anchors.leftMargin: 4
                    glyph: "⤓"
                    tip: "合并选中的图层（Ctrl 点击多选，背景层不参与）"
                    usable: layerPanel.multiCount >= 2
                    onBlocked: layerPanel.hintText =
                               "合并要先选中两个以上图层：Ctrl+点击图层行（画布上的图层也可以），背景不参与"
                    onActivated: bridge.merge_selected_layers()
                }
                PanelButton {
                    id: btnGroup
                    anchors.left: btnMerge.right
                    anchors.leftMargin: 4
                    icon: "group"
                    tip: "编组（Ctrl+G）：把选中的两个以上图层收进一组，不改变 z 序"
                    usable: layerPanel.multiCount >= 2
                    onBlocked: layerPanel.hintText =
                               "编组要先选中两个以上图层：Ctrl+点击图层行，背景不参与"
                    onActivated: bridge.group_selected_layers()
                }
                PanelButton {
                    id: btnUngroup
                    anchors.left: btnGroup.right
                    anchors.leftMargin: 4
                    icon: "ungroup"
                    tip: "解散分组（Ctrl+Shift+G）：图层本身保留"
                    usable: layerPanel.hasGroups
                    onBlocked: layerPanel.hintText =
                               "还没有分组：先选中两个以上图层点「编组」"
                    onActivated: bridge.ungroup_selected_layers()
                }
                PanelButton {
                    anchors.right: parent.right
                    glyph: "✕"
                    tip: "删除选中图层"
                    danger: true
                    usable: bridge.selected_layer > 0
                    onActivated: bridge.delete_selected_layers()
                }
            }

            // 第二排：对齐（6）｜ 分布（2），两组之间一条细分隔线。
            Row {
                width: parent.width
                spacing: 4

                PanelButton {
                    icon: "left"
                    tip: "左对齐"
                    usable: layerPanel.multiCount > 0
                    onActivated: bridge.align_selected_layers("left")
                }
                PanelButton {
                    icon: "hcenter"
                    tip: "水平居中"
                    usable: layerPanel.multiCount > 0
                    onActivated: bridge.align_selected_layers("hcenter")
                }
                PanelButton {
                    icon: "right"
                    tip: "右对齐"
                    usable: layerPanel.multiCount > 0
                    onActivated: bridge.align_selected_layers("right")
                }
                PanelButton {
                    icon: "top"
                    tip: "顶对齐"
                    usable: layerPanel.multiCount > 0
                    onActivated: bridge.align_selected_layers("top")
                }
                PanelButton {
                    icon: "vcenter"
                    tip: "垂直居中"
                    usable: layerPanel.multiCount > 0
                    onActivated: bridge.align_selected_layers("vcenter")
                }
                PanelButton {
                    icon: "bottom"
                    tip: "底对齐"
                    usable: layerPanel.multiCount > 0
                    onActivated: bridge.align_selected_layers("bottom")
                }

                Rectangle {
                    width: 1
                    height: 16
                    anchors.verticalCenter: parent.verticalCenter
                    color: "#3a4048"
                }

                PanelButton {
                    icon: "spread-h"
                    tip: "水平等距分布（需 3 层以上）"
                    usable: layerPanel.multiCount >= 3
                    onActivated: bridge.distribute_selected_layers("h")
                }
                PanelButton {
                    icon: "spread-v"
                    tip: "垂直等距分布（需 3 层以上）"
                    usable: layerPanel.multiCount >= 3
                    onActivated: bridge.distribute_selected_layers("v")
                }
            }

            // 说明行：hover 按钮时显示按钮用途，平时显示多选手势提示。
            Text {
                width: parent.width
                height: 12
                text: layerPanel.hintText.length > 0
                      ? layerPanel.hintText
                      : "Ctrl 点击多选 · Shift 连选 · 双击重命名"
                color: "#6f767e"
                font.pixelSize: 10
                elide: Text.ElideRight
            }
        }

        Rectangle {
            anchors.bottom: parent.bottom
            width: parent.width
            height: 1
            color: "#31363c"
        }
    }

    // ================= 列表 =================
    Flickable {
        id: listArea
        anchors.top: header.bottom
        anchors.bottom: panelFooter.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.topMargin: 6
        anchors.bottomMargin: 6
        clip: true
        contentWidth: width
        contentHeight: layerColumn.height + 8
        boundsBehavior: Flickable.StopAtBounds

        Column {
            id: layerColumn
            x: 6
            y: 4
            width: listArea.width - 12
            spacing: layerPanel.rowSpacing

            Repeater {
                id: layerRepeater
                model: host.layerPanelList

                delegate: Rectangle {
                    id: layerRow
                    required property var modelData
                    required property int index

                    readonly property var row: modelData
                    readonly property bool isGroup: row !== null
                        && row !== undefined && row.k === "group"
                    readonly property bool isBase: row !== null && row.base === true
                    readonly property bool locked: row !== null && row.l === true
                    readonly property bool vis: row !== null && row.v === true
                    readonly property bool sel: layerPanel.isSelectedRow(row)
                    readonly property bool isMark: row !== null && row.k === "mark"
                    readonly property bool renaming: row !== null
                        && (layerRow.isGroup
                            ? layerPanel.renamingGroupId === row.gid
                            : (host.renamingIndex !== -1
                               && host.renamingIndex === row.i))
                    // 组内成员缩进一点，组标题行贴左边。
                    readonly property int indent: row && !layerRow.isGroup
                        && row.g > 0 ? 14 : 0
                    // 缩略图：layerEpoch = 像素变化，thumbEpoch = 后台分批
                    // 写入完成（C6）。两者都读一遍当依赖，避免 100 层首次
                    // 展开时 UI 线程同步缩图。组标题行没有位图，别去问
                    // layer_thumb_url(-1)。
                    readonly property string thumbUrl: {
                        if (layerRow.isGroup)
                            return ""
                        const epoch = bridge.layerEpoch
                        const batch = bridge.thumbEpoch
                        return row ? bridge.layer_thumb_url(row.i) : ""
                    }

                    width: layerColumn.width
                    height: layerPanel.rowHeight
                    radius: 5
                    color: sel ? "#31567f"
                         : (rowMa.containsMouse ? "#2b3036" : "transparent")
                    border.width: sel ? 1 : 0
                    border.color: "#4d84c4"

                    // 选中行左侧强调条。
                    Rectangle {
                        width: 3
                        height: parent.height - 10
                        anchors.left: parent.left
                        anchors.leftMargin: 1
                        anchors.verticalCenter: parent.verticalCenter
                        radius: 2
                        color: "#6fa8dc"
                        visible: layerRow.sel
                    }

                    // ---- 缩略图 ----
                    Item {
                        id: thumbBox
                        x: 9 + layerRow.indent
                        anchors.verticalCenter: parent.verticalCenter
                        width: 34
                        height: 34
                        visible: !layerRow.isGroup
                        clip: true
                        opacity: layerRow.vis ? 1.0 : 0.4

                        Rectangle {
                            anchors.fill: parent
                            color: "#14171a"
                            radius: 3
                        }
                        Image {
                            anchors.fill: parent
                            source: bridge.checkerTileUrl
                            fillMode: Image.Tile
                            opacity: 0.5
                        }
                        Image {
                            id: thumbImg
                            anchors.fill: parent
                            anchors.margins: 1
                            source: layerRow.thumbUrl
                            cache: false
                            asynchronous: true
                            fillMode: Image.PreserveAspectFit
                            smooth: true
                        }
                        Rectangle {
                            anchors.fill: parent
                            color: "transparent"
                            radius: 3
                            border.width: 1
                            border.color: layerRow.sel ? "#7fb2e5" : "#3a4048"
                        }
                        // 标记层的形状种类不再打角标：thumbBox 是 clip 的，
                        // 角标一压边就被切；而且缩略图本身已经画出了形状。
                    }

                    // ---- 组标题行的折叠箭头 ----
                    // z 要高于行本体 MouseArea：它在左侧，而 rowMa 覆盖整行
                    // （只让出右侧的眼睛/锁），不抬高会被整行点击吃掉。
                    Item {
                        id: groupArrow
                        z: 3
                        x: 9
                        anchors.verticalCenter: parent.verticalCenter
                        width: 34
                        height: 34
                        visible: layerRow.isGroup

                        Canvas {
                            anchors.fill: parent
                            property bool collapsed: layerRow.row
                                ? layerRow.row.c === true : false
                            onCollapsedChanged: requestPaint()
                            Component.onCompleted: requestPaint()
                            onPaint: {
                                var ctx = getContext("2d")
                                ctx.clearRect(0, 0, width, height)
                                ctx.fillStyle = "#8ec7ff"
                                ctx.beginPath()
                                if (collapsed) {
                                    ctx.moveTo(13, 11)
                                    ctx.lineTo(22, 17)
                                    ctx.lineTo(13, 23)
                                } else {
                                    ctx.moveTo(10, 13)
                                    ctx.lineTo(24, 13)
                                    ctx.lineTo(17, 23)
                                }
                                ctx.closePath()
                                ctx.fill()
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: if (layerRow.row)
                                bridge.set_group_collapsed(layerRow.row.gid,
                                                           !layerRow.row.c)
                        }
                    }

                    // ---- 名称 + 副信息 ----
                    //
                    // Column 里只放两行文本：重命名的 TextInput 放在 Column
                    // **外面**。放在里面时它的隐式高度会把两行文字顶上去、
                    // 副标题被行底边切掉半截（TextInput 即使空着也占一行）。
                    Column {
                        x: 50 + layerRow.indent
                        width: parent.width - 50 - 54 - layerRow.indent
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 2

                        Text {
                            width: parent.width
                            height: 16
                            verticalAlignment: Text.AlignVCenter
                            text: layerRow.row ? layerRow.row.n : ""
                            color: layerRow.isGroup ? "#a8cbef"
                                 : (layerRow.sel ? "#ffffff" : "#d3d8de")
                            font.pixelSize: 12
                            font.bold: layerRow.sel || layerRow.isGroup
                            elide: Text.ElideRight
                            visible: !layerRow.renaming
                        }

                        Text {
                            width: parent.width
                            height: 13
                            verticalAlignment: Text.AlignVCenter
                            visible: !layerRow.renaming
                            text: {
                                if (!layerRow.row)
                                    return ""
                                const r = layerRow.row
                                if (layerRow.isGroup)
                                    return r.cnt + " 个图层 · "
                                           + (r.c ? "已折叠" : "已展开")
                                if (layerRow.isBase && r.bd === true)
                                    return "背景 · 已清空"
                                const pos = layerRow.isMark ? "标记" : (r.w + "×" + r.h)
                                let s = pos + " · " + r.x + "," + r.y
                                if (r.o !== undefined && r.o < 100)
                                    s += " · " + r.o + "%"
                                return s
                            }
                            color: layerRow.sel ? "#c2d3e5" : "#79818a"
                            font.pixelSize: 10
                            elide: Text.ElideRight
                        }
                    }

                    TextInput {
                        id: renameInput
                        x: 50 + layerRow.indent
                        y: 6
                        width: parent.width - 50 - 54 - layerRow.indent
                        height: 20
                        visible: layerRow.renaming
                        text: layerRow.row ? layerRow.row.n : ""
                        color: "#ffffff"
                        font.pixelSize: 12
                        selectByMouse: true
                        focus: visible
                        onAccepted: {
                            if (text.trim().length > 0) {
                                if (layerRow.isGroup)
                                    bridge.rename_group(layerRow.row.gid, text)
                                else
                                    bridge.rename_selected_layer(text)
                            }
                            host.renamingIndex = -1
                            layerPanel.renamingGroupId = -1
                        }
                        onActiveFocusChanged: if (!activeFocus && visible) {
                            host.renamingIndex = -1
                            layerPanel.renamingGroupId = -1
                        }
                    }

                    // ---- 眼睛 ----
                    Item {
                        id: eyeSlot
                        anchors.right: parent.right
                        anchors.rightMargin: 26
                        anchors.verticalCenter: parent.verticalCenter
                        width: 22
                        height: 22

                        Canvas {
                            id: eyeIcon
                            anchors.centerIn: parent
                            width: 17
                            height: 17
                            property bool on: layerRow.vis
                            readonly property bool dim: layerRow.isBase
                                && layerRow.row && layerRow.row.bd === true
                            onOnChanged: requestPaint()
                            onDimChanged: requestPaint()
                            onPaint: {
                                var ctx = getContext("2d")
                                ctx.clearRect(0, 0, width, height)
                                ctx.lineWidth = 1.4
                                ctx.strokeStyle = dim ? "#454a51"
                                                : (on ? "#8ec7ff" : "#666c74")
                                ctx.beginPath()
                                ctx.moveTo(2, 8.5)
                                ctx.quadraticCurveTo(8.5, 2.4, 15, 8.5)
                                ctx.quadraticCurveTo(8.5, 14.6, 2, 8.5)
                                ctx.stroke()
                                if (on) {
                                    ctx.beginPath()
                                    ctx.arc(8.5, 8.5, 2.4, 0, Math.PI * 2)
                                    ctx.fillStyle = ctx.strokeStyle
                                    ctx.fill()
                                } else {
                                    ctx.beginPath()
                                    ctx.moveTo(4, 13)
                                    ctx.lineTo(13, 4)
                                    ctx.stroke()
                                }
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            enabled: !(layerRow.isBase && layerRow.row
                                       && layerRow.row.bd === true)
                            cursorShape: Qt.PointingHandCursor
                            onClicked: if (layerRow.row) {
                                if (layerRow.isGroup)
                                    bridge.set_group_visible(layerRow.row.gid,
                                                             !layerRow.row.v)
                                else if (layerRow.isBase)
                                    bridge.toggle_base_visible()
                                else
                                    bridge.set_layer_visible_at(layerRow.row.i,
                                                                 !layerRow.row.v)
                            }
                        }
                    }

                    // ---- 锁 ----
                    Item {
                        id: lockSlot
                        anchors.right: parent.right
                        anchors.rightMargin: 4
                        anchors.verticalCenter: parent.verticalCenter
                        width: 20
                        height: 22

                        Canvas {
                            id: lockIcon
                            anchors.centerIn: parent
                            width: 15
                            height: 15
                            property bool on: layerRow.locked
                            onOnChanged: requestPaint()
                            onPaint: {
                                var ctx = getContext("2d")
                                ctx.clearRect(0, 0, width, height)
                                ctx.lineWidth = 1.4
                                ctx.strokeStyle = on ? "#e8b34a" : "#5c626a"
                                ctx.fillStyle = on ? "#e8b34a" : "#5c626a"
                                ctx.beginPath()
                                if (on)
                                    ctx.arc(7.5, 6.5, 3.0, Math.PI, 0)
                                else
                                    ctx.arc(7.5, 6.5, 3.0, Math.PI * 0.85,
                                            Math.PI * 0.15, false)
                                ctx.stroke()
                                ctx.beginPath()
                                ctx.roundRect ? ctx.roundRect(3.5, 6.5, 8, 6, 1.5)
                                              : ctx.rect(3.5, 6.5, 8, 6)
                                ctx.fill()
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: if (layerRow.row) {
                                if (layerRow.isGroup)
                                    bridge.set_group_locked(layerRow.row.gid,
                                                            !layerRow.locked)
                                else if (layerRow.isBase)
                                    bridge.set_base_locked(!layerRow.locked)
                                else
                                    bridge.set_layer_locked(layerRow.row.i,
                                                            !layerRow.locked)
                            }
                        }
                    }

                    // ---- 行本体：点选 / 拖拽排序 / 右键菜单 ----
                    MouseArea {
                        id: rowMa
                        anchors.fill: parent
                        anchors.rightMargin: 50   // 让出眼睛和锁
                        hoverEnabled: true
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                        cursorShape: Qt.PointingHandCursor
                        property bool reordering: false
                        property int fromPos: -1
                        property int toPos: -1
                        property real pressY: 0

                        onPressed: (mouse) => {
                            if (mouse.button === Qt.RightButton) {
                                const pt = layerRow.mapToItem(layerPanel, mouse.x, mouse.y)
                                if (layerRow.isGroup) {
                                    openLayerContext(-1, false, pt,
                                                     layerRow.row.gid)
                                } else {
                                    if (!layerPanel.isSelectedRow(layerRow.row)
                                        && layerRow.row && !layerRow.locked)
                                        bridge.select_layer(layerRow.row.i)
                                    openLayerContext(layerRow.row
                                                     ? layerRow.row.i : -1,
                                                     layerRow.locked, pt, 0)
                                }
                                return
                            }
                            pressY = mouse.y
                            reordering = false
                        }

                        onPositionChanged: (mouse) => {
                            if (!pressed || layerRow.locked || layerRow.isBase
                                || layerRow.isGroup)
                                return
                            if (!reordering && Math.abs(mouse.y - pressY) > 6) {
                                reordering = true
                                fromPos = layerRow.index
                            }
                            if (!reordering)
                                return
                            const pitch = layerPanel.rowHeight + layerPanel.rowSpacing
                            const total = host.layerPanelList.length
                            const colY = layerRow.mapToItem(layerColumn, 0, mouse.y).y
                            let t = Math.floor(colY / pitch + 0.5)
                            t = Math.max(0, Math.min(t, total - 1))
                            if (layerRow.isBase || t === total - 1)
                                return       // 背景行不接受排序
                            toPos = t
                            dropIndicator.y = layerColumn.y + t * pitch - 2
                            dropIndicator.visible = t !== fromPos
                        }

                        onReleased: (mouse) => {
                            if (mouse.button === Qt.RightButton)
                                return
                            if (reordering && toPos >= 0 && toPos !== fromPos) {
                                // 显示行里混了组标题行，不能再用
                                // len-1-pos 反推文档下标（会算到错的层）。
                                // 起止行各自带真实 i，直接拿来排序。
                                const target = host.layerPanelList[toPos]
                                const from = layerRow.row ? layerRow.row.i : -1
                                const to = target && target.i >= 0
                                           ? target.i : from
                                if (from > 0 && to > 0 && to !== from)
                                    bridge.reorder_layer(from, to)
                            } else if (!reordering && layerRow.row && !layerRow.isBase
                                       && (!layerRow.locked || layerRow.isGroup)) {
                                if (layerRow.isGroup) {
                                    bridge.select_group(layerRow.row.gid)
                                } else {
                                    // Ctrl = 切换，Shift = 连选，普通点击 = 单选。
                                    // 事件自带的 modifiers 在「先松鼠标再松 Ctrl」时
                                    // 偶尔拿不到 Ctrl（释放事件里键盘状态已经是新的），
                                    // 所以再问一次实时键状态（bit0 Shift / bit1 Ctrl）。
                                    const bits = bridge.modifier_bits()
                                    const ctrl = (mouse.modifiers & Qt.ControlModifier) !== 0
                                                 || (bits & 2) !== 0
                                    const shift = (mouse.modifiers & Qt.ShiftModifier) !== 0
                                                  || (bits & 1) !== 0
                                    if (!ctrl && !shift)
                                        bridge.sel_clear()
                                    bridge.select_layer_ex(layerRow.row.i,
                                                           ctrl ? 1 : (shift ? 2 : 0))
                                }
                            }
                            reordering = false
                            toPos = -1
                            dropIndicator.visible = false
                        }

                        onDoubleClicked: if (layerRow.row && !layerRow.isBase) {
                            if (layerRow.isGroup)
                                layerPanel.renamingGroupId = layerRow.row.gid
                            else
                                host.renamingIndex = layerRow.row.i
                            renameInput.focus = true
                            renameInput.selectAll()
                        }
                    }
                }
            }
        }

        // 拖拽排序的插入位置提示线（放在 Column 外面：Column 会接管子项 y）。
        Rectangle {
            id: dropIndicator
            visible: false
            x: layerColumn.x
            width: layerColumn.width
            height: 2
            radius: 1
            color: "#6fa8dc"
            z: 10
        }
    }

    // ================= 底部：不透明度 =================
    Column {
        id: panelFooter
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottomMargin: 8
        anchors.leftMargin: 10
        anchors.rightMargin: 10
        spacing: 6
        visible: bridge.selected_layer >= 0

        Rectangle { width: parent.width; height: 1; color: "#31363c" }

        Row {
            width: parent.width
            spacing: 6

            Text {
                text: "不透明度"
                color: "#a9b0b8"
                font.pixelSize: 11
                anchors.verticalCenter: parent.verticalCenter
            }

            Item {
                id: opacitySlider
                width: parent.width - 108
                height: 20
                anchors.verticalCenter: parent.verticalCenter

                property real value: 1.0
                property bool dragging: false

                function setValue(x) {
                    value = Math.min(1.0, Math.max(0.0, x / width))
                    bridge.set_selected_opacity(value, false)
                }

                Binding {
                    target: opacitySlider
                    property: "value"
                    value: host.selLayer ? host.selLayer.o / 100.0 : 1.0
                    when: !opacitySlider.dragging
                }

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width
                    height: 4
                    radius: 2
                    color: "#31363c"
                }
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: opacitySlider.value * opacitySlider.width
                    height: 4
                    radius: 2
                    color: "#5b9dff"
                }
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    x: opacitySlider.value * (opacitySlider.width - 12)
                    width: 12
                    height: 12
                    radius: 6
                    color: opMa.containsMouse || opacitySlider.dragging ? "#8ec7ff" : "#e9eaec"
                }

                MouseArea {
                    id: opMa
                    anchors.fill: parent
                    anchors.margins: -6
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onPressed: (mouse) => {
                        opacitySlider.dragging = true
                        opacitySlider.setValue(mouse.x + 6)
                    }
                    onPositionChanged: (mouse) => {
                        if (opacitySlider.dragging)
                            opacitySlider.setValue(mouse.x + 6)
                    }
                    onReleased: {
                        opacitySlider.dragging = false
                        bridge.set_selected_opacity(opacitySlider.value, true)
                    }
                }
            }

            Text {
                width: 40
                text: Math.round(opacitySlider.value * 100) + "%"
                color: "#a9b0b8"
                font.pixelSize: 11
                anchors.verticalCenter: parent.verticalCenter
                horizontalAlignment: Text.AlignRight
            }
        }
    }

    // ================= 右键菜单 =================
    MouseArea {
        z: 99
        anchors.fill: parent
        visible: layerContext.visible
        enabled: layerContext.visible
        onPressed: (mouse) => {
            layerContext.visible = false
            mouse.accepted = true
        }
    }

    Rectangle {
        id: layerContext
        visible: false
        z: 100
        width: 210
        height: ctxCol.height + 10
        color: "#2a2f35"
        border.color: "#3f4650"
        border.width: 1
        radius: 7
        property int targetIndex: -1
        property bool targetLocked: false
        property bool targetIsMark: false
        // 右键点在组标题行上时 > 0；此时图层类命令全部不适用。
        property int targetGroupId: 0

        Column {
            id: ctxCol
            anchors.top: parent.top
            anchors.topMargin: 5
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 0

            MenuRow {
                label: "新建图层"
                onActivated: {
                    layerContext.visible = false
                    bridge.new_layer()
                }
            }
            MenuRow {
                label: "复制图层"
                active: layerContext.targetIndex >= 0 && !layerContext.targetLocked
                onActivated: {
                    layerContext.visible = false
                    bridge.duplicate_selected_layer()
                }
            }
            MenuRow {
                label: "编组 (Ctrl+G)"
                active: layerPanel.multiCount >= 2
                onActivated: {
                    layerContext.visible = false
                    bridge.group_selected_layers()
                }
            }
            MenuRow {
                label: "解散分组 (Ctrl+Shift+G)"
                active: layerContext.targetGroupId > 0 || layerPanel.hasGroups
                onActivated: {
                    layerContext.visible = false
                    if (layerContext.targetGroupId > 0)
                        bridge.ungroup_group(layerContext.targetGroupId)
                    else
                        bridge.ungroup_selected_layers()
                }
            }
            MenuRow {
                label: "折叠 / 展开分组"
                active: layerContext.targetGroupId > 0
                onActivated: {
                    layerContext.visible = false
                    const list = host.layerPanelList
                    for (var k = 0; k < list.length; k++)
                        if (list[k].k === "group"
                            && list[k].gid === layerContext.targetGroupId) {
                            bridge.set_group_collapsed(list[k].gid, !list[k].c)
                            break
                        }
                }
            }
            MenuRow {
                label: "合并选中的图层"
                active: layerPanel.multiCount >= 2
                onActivated: {
                    layerContext.visible = false
                    bridge.merge_selected_layers()
                }
            }
            MenuRow {
                label: "栅格化标记"
                active: layerContext.targetIsMark
                onActivated: {
                    layerContext.visible = false
                    bridge.rasterize_selected_mark()
                }
            }
            MenuRow {
                label: "旋转角度…"
                active: layerContext.targetIndex >= 0 && !layerContext.targetLocked
                          && !layerContext.targetIsMark
                onActivated: {
                    layerContext.visible = false
                    const i = layerContext.targetIndex
                    if (i >= 0)
                        host.openRotateLayer(i)
                }
            }
            MenuRow {
                label: "导出 PNG"
                active: layerContext.targetIndex >= 0 && !layerContext.targetIsMark
                onActivated: {
                    layerContext.visible = false
                    const i = layerContext.targetIndex
                    if (i >= 0)
                        bridge.export_layer_png(i)
                }
            }
            MenuRow {
                label: "删除图层"
                active: layerContext.targetIndex > 0 && !layerContext.targetLocked
                onActivated: {
                    layerContext.visible = false
                    const i = layerContext.targetIndex
                    if (i >= 0) {
                        bridge.select_layer(i)
                        bridge.delete_selected_layers()
                    }
                }
            }
        }
    }

    // 打开右键图层菜单（layerPanel 坐标），夹在面板内不溢出。
    function openLayerContext(index, locked, panelPos, groupId) {
        // 编辑锁期间不打开任何带输入框的浮层（焦点会静默废掉 Delete /
        // 工具键，见既有坑），也不给右键命令任何入口。
        if (layerPanel.editLocked) {
            bridge.psd_locked_hint()
            return
        }
        layerContext.targetIndex = index
        layerContext.targetLocked = locked
        layerContext.targetIsMark = false
        layerContext.targetGroupId = groupId || 0
        if (index >= 0) {
            const list = host.layerPanelList
            for (var i = 0; i < list.length; i++) {
                if (list[i].i === index) {
                    layerContext.targetIsMark = list[i].k === "mark"
                    break
                }
            }
        }
        layerContext.x = Math.max(0, Math.min(panelPos.x,
                                 layerPanel.width - layerContext.width))
        layerContext.y = Math.max(0, Math.min(panelPos.y,
                                 layerPanel.height - layerContext.height - 8))
        layerContext.visible = true
    }

    // Esc 处理器（EditorContent）需要感知/关闭右键菜单。
    property bool contextVisible: layerContext.visible
    function closeContext() { layerContext.visible = false }
}
