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
    // PSD 阶段 2：图层后台加载完成前，面板所有命令一律灰掉并给进度提示。
    readonly property bool editLocked: bridge.editingLocked

    // ---- 混合模式下拉（P0）------------------------------------------
    // 选项文字由 core::BlendMode::label 提供，避免 QML 再抄一份 27 项列表。
    readonly property var blendChoices: {
        const arr = []
        const n = bridge.blend_mode_count()
        for (var i = 0; i < n; i++)
            arr.push({ i: i, n: bridge.blend_mode_label(i) })
        return arr
    }
    property bool blendListOpen: false
    // 点组标题行 = 选中整组（「组 = 一个整体」的手势入口）：
    //   * 成员写进 bridge 多选集合 → 画布出**并集包围盒**，拖动 / 缩放 /
    //     旋转都作用于整组（批量路径）；
    //   * 面板只高亮组标题行，成员行不跟着亮（用户报的「组和图层同时被
    //     选中」）；这个 id 同时供组属性编辑（混合模式 / 不透明度）读取。
    property int selectedGroupId: -1
    // 组行高亮 / 组属性编辑的唯一判据就是这个 id（不再因为「成员恰好全在
    // 多选集合里」自动高亮 —— 单选一个成员层时会把组也点亮）。
    readonly property var activeGroup: {
        let best = null
        const list = host.layerPanelList
        for (var i = 0; i < list.length; i++) {
            const r = list[i]
            if (r && r.k === "group" && layerPanel.isSelectedRow(r))
                if (best === null || (r.d || 0) > (best.d || 0))
                    best = r
        }
        return best
    }
    readonly property bool hasBlendTarget: layerPanel.activeGroup !== null
                                           || host.selLayer !== null
    readonly property int currentBlend: {
        if (layerPanel.activeGroup)
            return layerPanel.activeGroup.b || 0
        const l = host.selLayer
        return l && l.b !== undefined ? l.b : 0
    }
    function currentBlendName() {
        const c = layerPanel.blendChoices[layerPanel.currentBlend]
        return c ? c.n : "正常"
    }
    function closeBlendList() { layerPanel.blendListOpen = false }
    // 画布上点选了图层 = 用户的心智已经转到那一层，组焦点随之作废；否则
    // 组行会一直高亮、混合下拉还改的是组。点空白（`select_layer(-1)`）同样
    // 作废 —— 否则「组一直在选中态」而画布上已经没有框了。
    // 点组行会先调 `select_group`（把主选中层写成组内最上层），这个回调
    // 先把 id 清掉，调用方随后再写回来，所以不会被误清。
    Connections {
        target: bridge
        // ★qproperty `selected_layer` 生成的信号是 `selected_layerChanged`
        //（cxx-qt 不做驼峰转换），写成 onSelectedLayerChanged 时这个回调
        // 根本不会被调用 —— 画布点选后列表不跟着滚就是这么来的。
        function onSelected_layerChanged() {
            const ix = bridge.selected_layer
            if (ix >= 0) {
                layerPanel.selectedGroupId = -1
                layerPanel.revealLayerRow(ix)
            } else {
                layerPanel.selectedGroupId = -1
            }
        }
    }

    // 画布选中图层 → 列表自动滚动到那一行（用户报的 bug：选中后列表不
    // 跟着走，看不到选的是谁）。用行下标反查显示行（组标题行 i=-1）。
    // 拖放落点用：组子树最上面成员的文档索引（z 最高）。用原始 layerList
    // （全量层，含被折叠祖先藏住的成员），按 groups_json 的 mids 匹配。
    function groupTopMemberIndex(gid) {
        let grp = null
        const groups = host.groupList
        for (var k = 0; k < groups.length; k++) {
            if (groups[k].id === gid) {
                grp = groups[k]
                break
            }
        }
        if (!grp)
            return -1
        const mids = grp.mids || []
        const layers = host.layerList
        for (var j = 0; j < layers.length; j++) {
            const l = layers[j]
            // ★跳过影子层（组样式烘焙，syn 标记）：锚点必须是「最上面的真实
            // 成员」。插到影子层上方会让新成员落到前景样式之外（描边/叠加
            // 盖不住它），跟 PS 的渲染顺序对不上。syn 是数字 1，用真值判断。
            if (l.i > 0 && !l.syn && mids.indexOf(l.id) >= 0)
                return l.i
        }
        return -1
    }

    function revealLayerRow(index) {
        const list = host.layerPanelList
        const row = rowOfLayer(list, index)
        if (row >= 0) {
            applyRevealRow(index, row, list)
            return
        }
        // 目标不在显示行里 —— 藏在折叠组。让 bridge 按文档真实组状态
        // 展开整条祖先链（QML 的 groupList 快照可能过期，不能拿它判断）。
        // 折叠是纯视图状态，不进撤销栈、不动像素。
        // bridge 会 NOTIFY groups_json，面板绑定下一帧才刷新；同时还要等
        // Repeater 建好 delegate，所以定位延到下一帧再做。
        if (layerPanel._revealPending === index)
            return
        bridge.reveal_layer_groups(index)
        layerPanel._revealPending = index
        if (layerPanel._revealTries[index] === undefined)
            layerPanel._revealTries[index] = 0
        const tries = ++layerPanel._revealTries[index]
        if (tries > 3)
            return
        Qt.callLater(function() {
            layerPanel._revealPending = -1
            layerPanel.revealLayerRow(index)
        })
    }
    property int _revealPending: -1
    property var _revealTries: ({})
    // 真正计算并应用滚动位置。行数据已刷新到这里才调用。
    function applyRevealRow(index, row, list) {
        const pitch = layerPanel.rowHeight + layerPanel.rowSpacing
        const y0 = row * pitch
        const y1 = y0 + layerPanel.rowHeight
        const viewH = listArea.height
        // ★展开组后 Repeater 的 delegate 和 layerColumn.height 要到下一帧
        // 才建好，listArea.contentHeight 此刻可能还是旧值。真实 PSD 有几
        // 百层 / 上百组，用它 clamp 会把滚动位置压回旧范围 —— 用户报的
        // 「定位不准」就是这个。用显示行数估算内容高度，不依赖滞后帧。
        const estH = list.length * pitch + 8
        const contentH = Math.max(listArea.contentHeight, estH)
        let cy = listArea.contentY
        if (y0 < cy)
            cy = y0 - 4
        else if (y1 > cy + viewH)
            cy = y1 - viewH + 4
        cy = Math.max(0, Math.min(cy, contentH - viewH))
        // 拖动中别抢滚动（会落点错乱）；只在静止时定位。
        if (listArea.interactive)
            listArea.contentY = cy
        if (host.reportReveal)
            host.reportReveal(index, row, listArea.contentY, listArea.height)
    }
    function rowOfLayer(list, index) {
        for (var k = 0; k < list.length; k++) {
            const r = list[k]
            if (r && r.i === index)
                return k
        }
        return -1
    }
    // 按钮 hover 时的说明文字（面板里没有原生 tooltip，用一个固定行显示，
    // 免得浮层跑出面板边界）。
    property string hintText: ""
    // 组标题行的内联重命名（-1 = 没有）。折叠态也能改名，所以按组 id
    // 而不是显示行下标来记。
    property int renamingGroupId: -1
    // 拖动图层时悬停到的目标组 id（-1 = 没有）。组行据此高亮，明确提示
    // 「松手会放进这个组」，比只有一条细指示线更醒目。
    property int dropTargetGroupId: -1
    // 兜底：bridge 调用会同步重建 layerPanelList，正在拖的 delegate 可能
    // 被当场销毁而跳过 onReleased → 绿线 / 组高亮残留。列表一变就清。
    Connections {
        target: host
        function onLayerPanelListChanged() {
            if (dropTargetGroupId >= 0)
                dropTargetGroupId = -1
            dropIndicator.visible = false
        }
    }
    // 落点诊断日志通道自检：console.log 在 Windows GUI 下走
    // OutputDebugString，不进 stderr 重定向，所以诊断统一走
    // bridge.dump_panel_rows（原样打印 JSON → stdout 日志可查）。
    property bool _dropLogChecked: false
    Connections {
        target: host
        function onLayerPanelListChanged() {
            if (!layerPanel._dropLogChecked) {
                layerPanel._dropLogChecked = true
                bridge.dump_panel_rows(JSON.stringify(
                    { d: "drop-log", channel: "ok",
                      rows: host.layerPanelList.length }))
            }
        }
    }
    // 选中的 id 集合：面板高亮用它，而不是下标（重排后下标会变）。
    readonly property var selIds: {
        try {
            return JSON.parse(bridge.layerSelectionIds)
        } catch (e) {
            return []
        }
    }
    // ---- 「组选中」的唯一判据 -------------------------------------------
    // 组选中 = 整组成员进了多选集合（画布出并集框、批量变换吃整组），但面板
    // **只高亮组标题行**：成员行跟着一起亮就是用户报的「组和图层同时被选中」。
    //
    // 两条来源合成一个派生值，避免面板自己记一份、跟着选择集跑偏：
    //   1) 点了组行（selectedGroupId）且成员还在选择集里；
    //   2) 选择集**恰好等于**某个 ≥2 成员组的全部成员 —— 覆盖脚本 / 探针 /
    //      别的代码路径调 `select_group` 的情况，和点组行看起来一致。
    //      要求 ≥2 是有意的：只有 1 个成员的组，单选那层时不能把组也点亮。
    function groupById(gid) {
        if (gid <= 0)
            return null
        const list = host.groupList
        for (var i = 0; i < list.length; i++)
            if (list[i] && list[i].id === gid)
                return list[i]
        return null
    }
    function membersAllSelected(g) {
        if (g === null)
            return false
        const mids = g.mids || []
        if (mids.length === 0 || mids.length !== selIds.length)
            return false
        for (var i = 0; i < mids.length; i++)
            if (selIds.indexOf(mids[i]) < 0)
                return false
        return true
    }
    readonly property int effectiveGroupId: {
        if (layerPanel.membersAllSelected(layerPanel.groupById(
                layerPanel.selectedGroupId)))
            return layerPanel.selectedGroupId
        if (bridge.selectedLayerCount >= 2) {
            // 嵌套时可能有多个组同时「成员全中」（父组的 mids 含子组成员）：
            // 取**最深**那个，才和用户点的子组一致。
            let best = -1
            let depth = -1
            const list = host.groupList
            for (var i = 0; i < list.length; i++) {
                const g = list[i]
                if (!g || (g.mids || []).length < 2)
                    continue
                if (!layerPanel.membersAllSelected(g))
                    continue
                if ((g.d || 0) > depth) {
                    depth = g.d || 0
                    best = g.id
                }
            }
            return best
        }
        return -1
    }
    function focusedGroup() {
        return layerPanel.groupById(layerPanel.effectiveGroupId)
    }
    function isSelectedRow(row) {
        if (row === null || row === undefined)
            return false
        if (row.k === "group")
            return row.gid === layerPanel.effectiveGroupId
        // 组选中态：组子树里的成员不单独高亮（框已经代表整组了）。
        const g = layerPanel.focusedGroup()
        if (g !== null) {
            const mids = g.mids || []
            if (mids.indexOf(row.id) >= 0)
                return false
        }
        return selIds.indexOf(row.id) >= 0
    }

    // 点击面板内别处时先提交正在进行的重命名并收起输入框。遍历当前可见
    // 的 delegate，找到仍处于编辑态的那个。QML 里 TextInput 默认不会因
    // 点击 MouseArea 失焦，所以必须显式收口（用户报的「点别处不关闭」）。
    function commitInlineRename() {
        if (host.renamingIndex < 0 && layerPanel.renamingGroupId < 0)
            return
        for (var k = 0; k < layerRepeater.count; k++) {
            const d = layerRepeater.itemAt(k)
            if (d && d.renameField && d.renameField.visible) {
                d.renameField.finishRename(true)
                break
            }
        }
        // 兜底：即便没找到 delegate（行正在重建），也要清掉状态，否则
        // renaming 标志会残留。
        host.renamingIndex = -1
        layerPanel.renamingGroupId = -1
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
                    tip: "拆散选中的分组（Ctrl+Shift+G）：图层本身保留"
                    // 只有先点选一个分组（组标题行高亮）才允许拆散，避免
                    // 「以为在动图层结果把整个组拆了」的误操作（用户反馈）。
                    usable: layerPanel.activeGroup !== null
                    onBlocked: layerPanel.hintText =
                               "拆散分组要先点选一个分组：点击组标题行（组行高亮后生效）"
                    onActivated: bridge.ungroup_group(layerPanel.activeGroup.gid)
                }
                // 删除只保留在右键菜单里：右上角这个 ✕ 太像「关闭面板」，
                // 误点一次就可能删掉图层（用户反馈）。
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

            // 第三排：混合模式（图层 / 选中组）。
            Row {
                width: parent.width
                height: 20
                spacing: 6
                visible: layerPanel.hasBlendTarget

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: layerPanel.activeGroup ? "组混合" : "混合"
                    color: "#a9b0b8"
                    font.pixelSize: 11
                }

                Rectangle {
                    id: blendBtn
                    anchors.verticalCenter: parent.verticalCenter
                    width: 118
                    height: 20
                    radius: 3
                    color: layerPanel.editLocked ? "#24282d" : "#2b3036"
                    border.color: layerPanel.blendListOpen ? "#6fa8dc" : "#3a4048"
                    border.width: 1

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.left: parent.left
                        anchors.leftMargin: 6
                        width: parent.width - 20
                        text: layerPanel.currentBlendName()
                        color: layerPanel.editLocked ? "#666c74" : "#e3e7eb"
                        font.pixelSize: 11
                        elide: Text.ElideRight
                    }
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.right: parent.right
                        anchors.rightMargin: 6
                        text: "▾"
                        color: "#8ec7ff"
                        font.pixelSize: 10
                    }
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            if (layerPanel.editLocked) {
                                layerPanel.hintText = "图层加载中，暂不可修改混合模式"
                                return
                            }
                            layerPanel.blendListOpen = !layerPanel.blendListOpen
                        }
                    }
                }

                Rectangle {
                    visible: layerPanel.activeGroup !== null
                    anchors.verticalCenter: parent.verticalCenter
                    width: 1
                    height: 14
                    color: "#3a4048"
                }
                Rectangle {
                    id: passBtn
                    visible: layerPanel.activeGroup !== null
                    anchors.verticalCenter: parent.verticalCenter
                    width: 32
                    height: 18
                    radius: 3
                    color: layerPanel.activeGroup && layerPanel.activeGroup.pt ? "#2f5b7d" : "#2b3036"
                    border.color: "#4a5560"
                    border.width: 1
                    Text {
                        anchors.centerIn: parent
                        text: "穿透"
                        color: "#d7e2ec"
                        font.pixelSize: 9
                    }
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            if (layerPanel.editLocked || !layerPanel.activeGroup)
                                return
                            bridge.set_group_pass_through(layerPanel.activeGroup.gid,
                                                          !layerPanel.activeGroup.pt)
                        }
                    }
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

    // 混合模式下拉：用 Flickable 装 27 项，面板再高也不溢出。
    MouseArea {
        z: 149
        anchors.fill: parent
        visible: layerPanel.blendListOpen
        onClicked: layerPanel.closeBlendList()
    }
    Rectangle {
        id: blendList
        z: 150
        visible: layerPanel.blendListOpen
        readonly property point origin: blendBtn.mapToItem(layerPanel, 0, 0)
        x: Math.max(4, Math.min(origin.x, layerPanel.width - width - 4))
        y: origin.y + blendBtn.height + 2
        width: 178
        height: listFlick.height + 8
        color: "#2a2f35"
        border.color: "#4a5560"
        border.width: 1
        radius: 6

        Flickable {
            id: listFlick
            x: 4
            y: 4
            width: parent.width - 8
            height: Math.min(276, Math.max(48, layerPanel.height - blendList.y - 12))
            contentWidth: width
            contentHeight: blendCol.height
            clip: true
            boundsBehavior: Flickable.StopAtBounds

            Column {
                id: blendCol
                width: parent.width
                spacing: 2
                Repeater {
                    model: layerPanel.blendChoices
                    delegate: Rectangle {
                        required property var modelData
                        width: blendCol.width
                        height: 24
                        radius: 3
                        color: modelData.i === layerPanel.currentBlend ? "#3d5a80"
                             : blendItemMa.containsMouse ? "#363c44" : "transparent"
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.left: parent.left
                            anchors.leftMargin: 8
                            text: modelData.n
                            color: "#e9eaec"
                            font.pixelSize: 11
                        }
                        Text {
                            visible: modelData.i === layerPanel.currentBlend
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.right: parent.right
                            anchors.rightMargin: 8
                            text: "✓"
                            color: "#ffffff"
                            font.pixelSize: 11
                        }
                        MouseArea {
                            id: blendItemMa
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                layerPanel.closeBlendList()
                                if (layerPanel.activeGroup)
                                    bridge.set_group_blend_mode(layerPanel.activeGroup.gid,
                                                                modelData.i)
                                else
                                    bridge.set_layer_blend_mode(-1, modelData.i)
                            }
                        }
                    }
                }
            }
        }
    }

    // ================= 保真态提示 =================
    // 保真显示时文档只有 1 层（PS 原合成），分组与图层都还不存在 —— 不给
    // 入口的话，用户会以为「这个 PSD 里没有分组」。
    Rectangle {
        id: fidelityHint
        visible: bridge.fidelityMode && bridge.hasPsdLayers
        anchors.top: header.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        height: visible ? 44 : 0
        color: "#2a3646"

        Text {
            anchors.left: parent.left
            anchors.leftMargin: 10
            anchors.right: parent.right
            anchors.rightMargin: 10
            anchors.verticalCenter: parent.verticalCenter
            text: "正在显示 PS 原合成（1 层）· 点此进入分层编辑，查看图层与分组"
            color: "#a8cbef"
            font.pixelSize: 11
            wrapMode: Text.WordWrap
        }

        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: bridge.enter_layered_editing()
        }
    }

    // ================= 列表 =================
    Flickable {
        id: listArea
        anchors.top: fidelityHint.visible ? fidelityHint.bottom : header.bottom
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
                    // 暴露输入框给面板级「点击别处提交」逻辑。
                    property alias renameField: renameInput

                    readonly property var row: modelData
                    readonly property bool isGroup: row !== null
                        && row !== undefined && row.k === "group"
                    readonly property bool isBase: row !== null && row.base === true
                    readonly property bool locked: row !== null && row.l === true
                    // 眼睛图标显示「自身意愿」（ev）：组隐藏时成员图标保持原样。
                    // 合成真值 v 只用于缩略图置灰等非图标语义。
                    readonly property bool vis: row !== null && row.ev === true
                    readonly property bool sel: layerPanel.isSelectedRow(row)
                    readonly property bool isMark: row !== null && row.k === "mark"
                    readonly property bool renaming: row !== null
                        && (layerRow.isGroup
                            ? layerPanel.renamingGroupId === row.gid
                            : (host.renamingIndex !== -1
                               && host.renamingIndex === row.i))
                    // 嵌套缩进：组行与层行都按 depth 排，PSD 的组套组才看得出来。
                    readonly property int indent: row && row.d ? row.d * 14 : 0
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
                    // 拖入目标组：整行高亮成绿色，明确提示「松手放进这里」。
                    readonly property bool dropTarget: layerRow.isGroup
                        && layerPanel.dropTargetGroupId === row.gid
                    color: dropTarget ? "#2f5d3f"
                         : sel ? "#31567f"
                         : (rowMa.containsMouse ? "#2b3036" : "transparent")
                    border.width: (sel || dropTarget) ? 1 : 0
                    border.color: dropTarget ? "#7ec8a0" : "#4d84c4"

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

                    // ---- 组标题行：左侧折叠三角 + 右侧小文件夹 ----
                    // z 要高于行本体 MouseArea：它在左侧，而 rowMa 覆盖整行
                    // （只让出右侧的眼睛/锁），不抬高会被整行点击吃掉。
                    // 用户定稿：三角形在文件夹**左边**，两个图标都别占宽度。
                    Item {
                        id: groupArrow
                        z: 3
                        x: 5 + layerRow.indent
                        anchors.verticalCenter: parent.verticalCenter
                        width: 36
                        height: 24
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
                                const stroke = "#8ec7ff"
                                ctx.strokeStyle = stroke
                                ctx.fillStyle = stroke
                                ctx.lineWidth = 1.1
                                // 折叠三角：折叠 → 向右，展开 → 向下。
                                ctx.beginPath()
                                if (collapsed) {
                                    ctx.moveTo(4, 8)
                                    ctx.lineTo(4, 16)
                                    ctx.lineTo(10, 12)
                                } else {
                                    ctx.moveTo(3, 9)
                                    ctx.lineTo(11, 9)
                                    ctx.lineTo(7, 15)
                                }
                                ctx.closePath()
                                ctx.fill()
                                // 文件夹：主体 15,8 → 32,19；标签页更短。
                                ctx.beginPath()
                                ctx.moveTo(15, 11)
                                ctx.lineTo(15, 19)
                                ctx.lineTo(32, 19)
                                ctx.lineTo(32, 11)
                                ctx.closePath()
                                ctx.stroke()
                                ctx.beginPath()
                                ctx.moveTo(15, 11)
                                ctx.lineTo(15, 8)
                                ctx.lineTo(22, 8)
                                ctx.lineTo(24, 11)
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
                        x: 44 + layerRow.indent
                        width: parent.width - 44 - 54 - layerRow.indent
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
                        // ★z 必须高于 rowMa（整行鼠标区在它之后声明、默认盖在上层），
                        // 否则鼠标点不进输入框、光标无法定位 —— 「重命名异常」主因。
                        z: 6
                        x: 44 + layerRow.indent
                        y: 6
                        width: parent.width - 44 - 54 - layerRow.indent
                        height: 20
                        visible: layerRow.renaming
                        color: "#ffffff"
                        font.pixelSize: 12
                        selectByMouse: true
                        focus: visible
                        // ★不能用 `text: layerRow.row.n` 持续绑定：用户输入会打断
                        // 绑定，第二次双击同一行时输入框保留上次的旧值。改为进入
                        // 编辑时显式灌入当前名字并全选。
                        onVisibleChanged: if (visible && layerRow.row) {
                            text = layerRow.row.n
                            forceActiveFocus()
                            selectAll()
                        }
                        onAccepted: finishRename(true)
                        onActiveFocusChanged: if (!activeFocus && visible)
                            finishRename(true)
                        Keys.onEscapePressed: {
                            finishRename(false)
                            event.accepted = true
                        }

                        function finishRename(commit) {
                            if (commit && text.trim().length > 0) {
                                if (layerRow.isGroup)
                                    bridge.rename_group(layerRow.row.gid,
                                                        text.trim())
                                else
                                    bridge.rename_layer_at(layerRow.row.i,
                                                           text.trim())
                            }
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
                                                             !(layerRow.row.sv === undefined
                                                               ? layerRow.row.v
                                                               : layerRow.row.sv))
                                else if (layerRow.isBase)
                                    bridge.toggle_base_visible()
                                else
                                    // 用眼睛显示态取反：组隐藏时成员 v=false 但
                                    // ev=true，点眼睛应改「自身意愿」而不是误点亮。
                                    bridge.set_layer_eye(layerRow.row.i,
                                                         !layerRow.vis)
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
                        // 层/组拖动共用「插入缝」模型：toPos = 缝（gap），
                        // toGroup = 落点容器组（0 = 顶层，-1 = 不改归属）。
                        // toZ = 层的目标文档 z 索引（-1 = 只改归属）；组拖动
                        // 用 toAnchor + toAbove 表达「块插到锚点行上/下方」。
                        property int toPos: -1
                        property int toGroup: -1
                        property int toAnchor: -1
                        property bool toAbove: false
                        property int toZ: -1
                        property real pressY: 0
                        // 诊断：最近一次悬停的「缝上那行」快照（onReleased 打日志用）。
                        property var lastAbove: null

                        onPressed: (mouse) => {
                            // 正在重命名时点别的行本体 → 先提交并收起输入框。
                            // （点输入框自身被高 z 的 renameInput 接走，不会到这里。）
                            if (layerRow.renaming !== true
                                && (host.renamingIndex >= 0
                                    || layerPanel.renamingGroupId >= 0))
                                layerPanel.commitInlineRename()
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
                            toGroup = -1
                            toZ = -1
                            toAnchor = -1
                            toAbove = false
                            // ★拖动期间关掉 Flickable 的惯性/边缘滚动：否则
                            // 鼠标移到列表上下沿时 Qt 自动滚 contentY，落点
                            // 计算跟着跳，图层怎么拖都放不到想去的位置。
                            listArea.interactive = false
                        }

                        onPositionChanged: (mouse) => {
                            // 组行也允许拖动（拖进别的组 / 拖回顶层）。
                            if (!pressed || layerRow.locked || layerRow.isBase)
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
                            const raw = colY / pitch
                            const idx = Math.max(0, Math.min(Math.floor(raw), total - 1))
                            // 「插入缝」几何量提前算好，层/组拖动共用：行的
                            // 上半 = 缝在该行上方、下半 = 缝在下方；above =
                            // 缝上面那行（决定落点容器与 z 锚点）。
                            const inUpper = (raw - Math.floor(raw)) < 0.5
                            const gap = Math.max(0, Math.min(inUpper ? idx : idx + 1,
                                                             total - 1))
                            const above = gap > 0 ? host.layerPanelList[gap - 1] : null
                            lastAbove = above
                            const draggedGroup = layerRow.row && layerRow.isGroup
                                                 ? layerRow.row.gid : 0
                            if (draggedGroup > 0) {
                                // ★组拖动同样用「插入缝」：above = 缝上面
                                // 那行。组标题行下方 = 进该组（嵌套，成为组
                                // 内最上）；层行下方 = 组块排到那行下面（同
                                // 级换位 / 跨容器）；列表顶 / 背景上方 = 顶
                                // 层最上 / 顶层最底。防环由 bridge 校验。
                                const parentGid = layerRow.row.g || 0
                                toPos = gap
                                toGroup = -1
                                toAnchor = -1
                                toAbove = false
                                if (gap === 0 || !above) {
                                    toGroup = 0
                                    const layers = host.layerList
                                    if (layers.length > 1) {
                                        toAnchor = layers[0].i
                                        toAbove = true
                                    }
                                } else if (above.k === "group") {
                                    // 缝在组标题行下方 = 嵌套进该组最上。
                                    toGroup = above.gid
                                    const m = layerPanel.groupTopMemberIndex(above.gid)
                                    if (m > 0) {
                                        toAnchor = m
                                        toAbove = true
                                    }
                                } else if (above.base) {
                                    toGroup = 0
                                    toAnchor = 1
                                    toAbove = false
                                } else {
                                    // 缝在层行下方：组块排到该行下面。
                                    toGroup = above.g || 0
                                    toAnchor = above.i
                                    toAbove = false
                                }
                                const meaningful = toGroup !== parentGid
                                                   || (toAnchor > 0)
                                dropIndicator.y = layerColumn.y + gap * pitch - 2
                                dropIndicator.visible = meaningful && gap !== fromPos
                                dropIndicator.color = toGroup > 0 ? "#7ec8a0" : "#6fa8dc"
                                layerPanel.dropTargetGroupId = toGroup > 0 ? toGroup : -1
                                return
                            }
                            // ★层拖动用「插入缝」判定（PS 语义）：行的上半 =
                            // 缝在这行上方、下半 = 缝在这行下方。以前按最近行
                            // 中心取整，拖到 图层24 下缘一点就被吸附成组标题行
                            // → 误判「放进下面的组」（用户报的问题）。
                            // 缝的容器 = 缝上面那行的容器：层行 → 它的组；
                            // 组标题行 → 该组自身；列表顶端 → 顶层。
                            const from = layerRow.row ? layerRow.row.i : -1
                            const fromGroup = layerRow.row ? (layerRow.row.g || 0) : 0
                            toPos = gap
                            toZ = -1
                            toAnchor = -1
                            toAbove = false
                            // z 索引换算（文档索引，越大越顶）：move_layer 是
                            // 先 remove 再 insert，`to` 即最终文档索引。锚点
                            // 索引先按「自己被移走」平移：m' = m > from ? m-1 : m。
                            //   缝在锚点显示位下方（更底层）→ to = m' - 1
                            //   缝在锚点显示位上方（更顶层）→ to = m' + 1
                            // 上限 = layerList.length - 1：MoveLayer 是
                            // remove 后 insert，insert 位置允许 = 移除后
                            // 的长度（append = 成为全文档最顶层）。n-2 会把
                            // 「插到最顶」钳掉一位：组顶成员就是全文档最顶
                            // 层时，拖到组顶必现落点差一位。
                            const cap = Math.max(0, host.layerList.length - 1)
                            if (gap === 0 || !above) {
                                toGroup = 0
                                const layers = host.layerList
                                const topI = layers.length ? layers[0].i : -1
                                // 自己已经是最顶层时拖到最顶 = 无事可做。
                                // to = topI：remove 后 append 位 → 自己成为
                                // 新的最顶层（topI-1 会落到第二名）。
                                if (topI > 0 && topI !== from)
                                    toZ = Math.min(topI, cap)
                            } else if (above.k === "group") {
                                // 缝在组标题行下方 = 进该组、插到子树最上面
                                // 成员上方（z 更高，成为新的最上成员）。
                                // 锚点是自己（自己已是该组最上成员）= 无事可做。
                                toGroup = above.gid
                                const m = layerPanel.groupTopMemberIndex(above.gid)
                                if (m > 0 && m !== from) {
                                    const mp = m > from ? m - 1 : m
                                    toZ = Math.min(mp + 1, cap)
                                }
                            } else if (above.base) {
                                // 缝在背景行上方 = 最底层（紧贴背景上面）。
                                toGroup = 0
                                toZ = 1
                            } else {
                                // 缝在层行下方 = 与它同容器、紧贴它下面
                                // （z 更低一位）。锚点是自己 = 无事可做。
                                // ★to = m'，不是 m'-1：insert at m' 会把
                                // 锚点顶到 m'+1，落点正好紧贴锚点下方；
                                // 用 m'-1 会和锚点下面那层换位、反而往下掉
                                // （from=1, m=5 → to=3 把 idx4 顶上去）。
                                toGroup = above.g || 0
                                const m = above.i
                                if (m > 0 && m !== from)
                                    toZ = Math.max(1, m > from ? m - 1 : m)
                            }
                            dropIndicator.y = layerColumn.y + gap * pitch - 2
                            dropIndicator.visible = toZ !== from || toGroup !== fromGroup
                            dropIndicator.color = toGroup > 0 ? "#7ec8a0" : "#6fa8dc"
                            layerPanel.dropTargetGroupId = toGroup > 0 ? toGroup : -1
                        }

                        onReleased: (mouse) => {
                            if (mouse.button === Qt.RightButton)
                                return
                            // ★先快照落点、清理视觉状态，再调 bridge：
                            // qinvokable 同步触发 layers_json NOTIFY → 列表
                            // 重建 → 本 delegate 当场销毁，放后面的清理代码
                            // 不会执行 → 绿线 / 组高亮残留（用户报的问题）。
                            const doMove = reordering && toPos >= 0
                            const dragGid = layerRow.row && layerRow.isGroup
                                            ? layerRow.row.gid : 0
                            const snapRow = layerRow.row
                            const snapToGroup = toGroup
                            const snapToZ = toZ
                            const snapAnchor = toAnchor
                            const snapAbove = toAbove
                            const snapParent = snapRow ? (snapRow.g || 0) : 0
                            // 落点诊断：真实交互里 gap/above 的实际值
                            // （dump_panel_rows → stdout → debug 日志可查）。
                            const ab = lastAbove
                            bridge.dump_panel_rows(JSON.stringify({
                                d: "drop",
                                kind: dragGid > 0 ? "group" : "layer",
                                doMove: doMove, gap: toPos,
                                row: snapRow ? { n: snapRow.n, i: snapRow.i,
                                                 g: snapRow.g || 0,
                                                 k: snapRow.k || "" } : null,
                                toG: snapToGroup, toZ: snapToZ,
                                anchor: snapAnchor, abv: snapAbove,
                                ab: ab ? { k: ab.k, i: ab.i, gid: ab.gid,
                                           n: ab.n, g: ab.g || 0 } : null
                            }))
                            reordering = false
                            toPos = -1
                            toGroup = -1
                            toZ = -1
                            toAnchor = -1
                            toAbove = false
                            layerPanel.dropTargetGroupId = -1
                            dropIndicator.visible = false
                            listArea.interactive = true
                            if (doMove) {
                                if (dragGid > 0) {
                                    // 组拖动（缝模型）：★一次调用完成归组 +
                                    // z 排序。拆两次 qinvokable 时第一步的
                                    // layers_json 同步重建会销毁本 delegate、
                                    // 第二步永远不执行（用户报「换分组后
                                    // 顺序异常 / 固定在最上面」的根因）。
                                    // target=-1 = 只排序不改归属；anchor=-1
                                    // = 只归组不排序（bridge 侧分支处理）。
                                    const needRegroup =
                                        snapToGroup >= 0 && snapToGroup !== snapParent
                                    if (needRegroup || snapAnchor > 0) {
                                        bridge.dump_panel_rows(JSON.stringify(
                                            { d: "drop-call",
                                              op: "move_group_to_group_at",
                                              gid: dragGid, g: snapToGroup,
                                              anchor: snapAnchor,
                                              above: snapAbove }))
                                        bridge.move_group_to_group_at(
                                            dragGid,
                                            needRegroup ? snapToGroup : -1,
                                            snapAnchor > 0 ? snapAnchor : -1,
                                            snapAbove)
                                    }
                                } else if (snapRow && snapRow.i > 0) {
                                    // 层拖动：★同样一次调用完成归组 + 落位。
                                    // toZ 是按 PS 缝语义算好的插入 z 索引
                                    // （见 onPositionChanged）；snapToZ<0 =
                                    // 只归组（空组 / 落点无 z 变化）。
                                    const from = snapRow.i
                                    const fromGroup = snapRow.g || 0
                                    if (snapToGroup >= 0 && snapToGroup !== fromGroup) {
                                        bridge.dump_panel_rows(JSON.stringify(
                                            { d: "drop-call",
                                              op: "move_layer_to_group_at",
                                              from: from, g: snapToGroup,
                                              z: snapToZ }))
                                        bridge.move_layer_to_group_at(
                                            from, snapToGroup, snapToZ)
                                    } else if (snapToZ >= 0 && snapToZ !== from) {
                                        bridge.dump_panel_rows(JSON.stringify(
                                            { d: "drop-call", op: "reorder_layer",
                                              from: from, z: snapToZ }))
                                        bridge.reorder_layer(from, snapToZ)
                                    } else {
                                        bridge.dump_panel_rows(JSON.stringify(
                                            { d: "drop-skip", from: from,
                                              z: snapToZ, g: snapToGroup }))
                                    }
                                }
                            } else if (snapRow && !snapRow.isBase
                                       && (!layerRow.locked || layerRow.isGroup)) {
                                if (layerRow.isGroup) {
                                    // 点组 = 选中整组：成员写进多选集合（画布出
                                    // 并集包围盒、批量变换吃整组），面板只亮组标题行。
                                    // ★顺序要紧：`select_group` 会把主选中层写成组内
                                    //   最上层，`onSelected_layerChanged` 会先把
                                    //   selectedGroupId 清成 -1，所以 id 必须在调用
                                    //   **之后**再写。
                                    bridge.select_group(snapRow.gid)
                                    layerPanel.selectedGroupId = snapRow.gid
                                } else {
                                    layerPanel.selectedGroupId = -1
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
                                    bridge.select_layer_ex(snapRow.i,
                                                           ctrl ? 1 : (shift ? 2 : 0))
                                }
                            }
                        }

                        onCanceled: {
                            // Alt+Tab / 浮层抢焦点导致丢鼠标：标志和滚动锁都要复位，
                            // 否则列表永久不可滚、下一次拖也起不来。
                            reordering = false
                            toPos = -1
                            toGroup = -1
                            toZ = -1
                            toAnchor = -1
                            toAbove = false
                            layerPanel.dropTargetGroupId = -1
                            dropIndicator.visible = false
                            listArea.interactive = true
                        }

                        onDoubleClicked: if (layerRow.row && !layerRow.isBase) {
                            // 先关掉可能正在编辑的其它行，否则两个输入框同时
                            // 抢焦点；设置状态后 renameInput 的 visible 绑定会
                            // 自动获得焦点并在 onVisibleChanged 里全选。
                            host.renamingIndex = -1
                            layerPanel.renamingGroupId = -1
                            if (layerRow.isGroup)
                                layerPanel.renamingGroupId = layerRow.row.gid
                            else
                                host.renamingIndex = layerRow.row.i
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

        // 滚动条：只在滚动 / 拖拽期间短暂出现，停手 900ms 后淡出。
        // Qt 的 Flickable 默认不画滚动条，长列表（100+ 层）时用户不知道
        // 还能往下滚；常驻又占地方，所以用「滚动时才显示」。
        // 右缘 14px 是热区：可拖 thumb、可点 track 跳转（用户反馈：
        // 滚动条不能只是个指示灯，得有正常滚动条的手感）。
        Item {
            id: scrollHot
            z: 12
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 10
            visible: listArea.contentHeight > listArea.height + 1

            function ensureVisible() {
                layerPanel.scrollBarVisible = true
                scrollBarHide.restart()
            }

            Rectangle {
                id: listScrollBar
                anchors.right: parent.right
                anchors.rightMargin: 2
                anchors.top: parent.top
                width: 4
                radius: 2
                height: Math.max(24,
                    listArea.height * listArea.height
                    / Math.max(listArea.contentHeight, 1))
                y: listArea.contentHeight > listArea.height
                   ? listArea.contentY / (listArea.contentHeight - listArea.height)
                     * (listArea.height - height) : 0
                color: scrollMa.containsMouse || scrollMa.pressed
                       ? "#8ec7ff" : "#6fa8dc"
                opacity: layerPanel.scrollBarVisible ? 0.85 : 0
                Behavior on opacity { NumberAnimation { duration: 180 } }
            }

            MouseArea {
                id: scrollMa
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                property real grabOffset: 0

                function dragTo(newThumbY) {
                    const maxY = listArea.contentHeight - listArea.height
                    const track = listArea.height - listScrollBar.height
                    if (track <= 0 || maxY <= 0)
                        return
                    listArea.contentY = Math.max(0,
                        Math.min(maxY, newThumbY / track * maxY))
                }

                onPressed: (mouse) => {
                    // 命中 thumb → 按下点相对 thumb 顶的偏移；点在 track
                    // 其它位置 → thumb 中心跳到点击处再拖。
                    const thumbY = listScrollBar.y
                    if (mouse.y >= thumbY && mouse.y <= thumbY + listScrollBar.height)
                        grabOffset = mouse.y - thumbY
                    else
                        grabOffset = listScrollBar.height / 2
                    listArea.interactive = false
                    dragTo(mouse.y - grabOffset)
                    scrollHot.ensureVisible()
                }
                onPositionChanged: (mouse) => {
                    if (pressed)
                        dragTo(mouse.y - grabOffset)
                }
                onReleased: {
                    scrollBarHide.restart()
                    listArea.interactive = true
                }
                onCanceled: {
                    scrollBarHide.restart()
                    listArea.interactive = true
                }
            }
        }
        Timer {
            id: scrollBarHide
            interval: 900
            onTriggered: layerPanel.scrollBarVisible = false
        }

        // 滚轮速度：Flickable 默认一滚只挪一点点，长列表难用。WheelHandler
        // 吃掉滚轮事件自己算 contentY，倍率来自设置（bridge.layerScrollSpeed）。
        WheelHandler {
            target: listArea
            onWheel: (event) => {
                if (listArea.contentHeight <= listArea.height) {
                    event.accepted = true
                    return
                }
                const maxY = listArea.contentHeight - listArea.height
                // pixelDelta = 触控板像素；angleDelta/120 = 鼠标一格。
                let dy = event.pixelDelta.y
                if (dy === 0)
                    dy = (event.angleDelta.y > 0 ? -1 : 1)
                         * listArea.height * 0.15 * bridge.layerScrollSpeed
                else
                    dy *= bridge.layerScrollSpeed
                listArea.contentY = Math.max(0, Math.min(maxY,
                    listArea.contentY + dy))
                layerPanel.scrollBarVisible = true
                scrollBarHide.restart()
                event.accepted = true
            }
        }
    }

    property bool scrollBarVisible: false
    Connections {
        target: listArea
        function onContentYChanged() {
            layerPanel.scrollBarVisible = true
            scrollBarHide.restart()
        }
        function onMovingChanged() {
            if (listArea.moving) {
                layerPanel.scrollBarVisible = true
                scrollBarHide.restart()
            }
        }
    }

    // ================= 底部：不透明度 =================
    // 双用途：选中图层 → 图层不透明度；聚焦分组 → 组不透明度
    // （第三排的组滑条已删，统一从这里调，用户反馈）。
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
                 || layerPanel.activeGroup !== null

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
                    if (layerPanel.activeGroup)
                        return // 组：拖动期间只刷显示，松手一次性提交（一条历史）
                    bridge.set_selected_opacity(value, false)
                }

                Binding {
                    target: opacitySlider
                    property: "value"
                    // 组焦点优先：点的是组行，滑条就该反映组不透明度。
                    value: layerPanel.activeGroup
                           ? layerPanel.activeGroup.o / 100.0
                           : (host.selLayer ? host.selLayer.o / 100.0 : 1.0)
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
                        if (layerPanel.activeGroup)
                            bridge.set_group_opacity(layerPanel.activeGroup.gid,
                                                     opacitySlider.value)
                        else
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
                label: "重命名"
                active: layerContext.targetGroupId > 0
                        || (layerContext.targetIndex >= 0
                            && !layerContext.targetLocked)
                onActivated: {
                    layerContext.visible = false
                    host.renamingIndex = -1
                    layerPanel.renamingGroupId = -1
                    if (layerContext.targetGroupId > 0)
                        layerPanel.renamingGroupId = layerContext.targetGroupId
                    else
                        host.renamingIndex = layerContext.targetIndex
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
                // 只有右键点在分组标题行上才出现（用户反馈：图层行的菜单
                // 不该有拆散项，避免误拆整组）。
                label: "拆散分组"
                active: layerContext.targetGroupId > 0
                onActivated: {
                    layerContext.visible = false
                    bridge.ungroup_group(layerContext.targetGroupId)
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
        // 右键切到另一行时先收起正在编辑的输入框。
        commitInlineRename()
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
