import QtQuick
import com.imageedit

// P0-5 变换数值面板（D7 / D10 / D13）。
// * 首次默认隐藏，由工具栏按钮切换；显示状态在 settings.ini 里跨启动记忆。
// * X/Y/W/H 都是「变换后轴对齐外接框」语义，可为负、可超出画布。
// * 一次提交 = 一条 SetLayerTransform 历史；不逐键写文档。
// * 11-07 复测改版：X/Y 同一行、W/H 同一行（原来一行一个太占地方），
//   水平/垂直翻转按钮去掉（画布手柄拖过对边就能镜像；菜单里也有整幅翻转），
//   吸附 / 参考线开关移到右上角「⋯」总菜单（视图 → 智能吸附 / 显示参考线）。
Rectangle {
    id: panel
    required property var bridge

    readonly property var vals: {
        try { return JSON.parse(bridge.transformValuesJson) } catch (e) { return {has: false} }
    }

    visible: bridge.transformPanelVisible && vals.has === true
    width: 222
    // 标题 26 + 上边距 10 + 三行 22×3 + 行距 6×2 = 114；参考点网格顶到
    // 122、高 48 → 内容最低 170，再留 12 的底边距。以前写死 160，网格最后
    // 一行被裁掉（看着像「布局错乱」）。
    height: 182
    radius: 6
    color: "#f21d2024"
    border.color: "#5a6068"
    border.width: 1
    z: 20

    property bool lockRatio: true
    property int refPoint: 0
    property bool suppress: false

    // 初始位置（外部注入）；拖动一次之后 x/y 的绑定被替换，位置由用户接管。
    property real defaultX: 12
    property real defaultY: 100
    x: defaultX
    y: defaultY

    // 文档状态变化时回显；用户正在输入时不抢输入框。
    onValsChanged: syncFromBridge()
    Component.onCompleted: syncFromBridge()

    function syncFromBridge() {
        if (!vals.has)
            return
        suppress = true
        px.value = Math.round(vals.x)
        py.value = Math.round(vals.y)
        pw.value = Math.round(vals.w)
        ph.value = Math.round(vals.h)
        rot.value = Math.round(vals.rot)
        suppress = false
    }

    function apply() {
        if (suppress || !vals.has)
            return
        // 翻转一律保持当前值：按钮已经去掉，翻转过一次之后改数值不能把它清掉。
        bridge.transform_set_params(px.value, py.value, pw.value, ph.value,
                                    rot.value, vals.fx === true, vals.fy === true,
                                    lockRatio, refPoint)
    }

    // 标题栏
    Rectangle {
        id: header
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 26
        color: "#ff262a30"
        radius: 6
        // ★拖动面板：标题栏空白处按住即可移动（声明在 ✕ 之前，后声明的
        //   关闭按钮命中优先级更高，不会被拖动手势吃掉）。
        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.SizeAllCursor
            // ★位移必须用**场景坐标**算：MouseArea 挂在被拖的面板自己身上，
            //   面板一动，mouse.x/y 的参照系跟着走 —— 用局部坐标算增量会形成
            //   正反馈（拖 1px 面板移动 2px，再拖又翻倍），表现出来是「面板不跟
            //   鼠标 + 一直抖」。按下与移动都换算到 scene，增量才和鼠标一致。
            property real grabSceneX: 0
            property real grabSceneY: 0
            property real panelX: 0
            property real panelY: 0
            function scenePt(mx, my) {
                return mapToItem(null, mx, my)
            }
            onPressed: (mouse) => {
                const g = scenePt(mouse.x, mouse.y)
                grabSceneX = g.x
                grabSceneY = g.y
                panelX = panel.x
                panelY = panel.y
            }
            onPositionChanged: (mouse) => {
                if (!pressed)
                    return
                const g = scenePt(mouse.x, mouse.y)
                const pw = panel.parent ? panel.parent.width : 0
                const ph = panel.parent ? panel.parent.height : 0
                // 允许拖出左边一点（贴边收纳），但永远留 40px 可抓回来；
                // 上下不出父项可视范围。
                panel.x = Math.max(40 - panel.width,
                                   Math.min(panelX + g.x - grabSceneX, pw - 40))
                panel.y = Math.max(0,
                                   Math.min(panelY + g.y - grabSceneY, ph - 24))
            }
        }
        Text {
            anchors.left: parent.left
            anchors.leftMargin: 10
            anchors.verticalCenter: parent.verticalCenter
            text: "变换数值"
            color: "#e9ecf1"
            font.pixelSize: 12
        }
        Text {
            anchors.right: parent.right
            anchors.rightMargin: 8
            anchors.verticalCenter: parent.verticalCenter
            text: "✕"
            color: "#aeb4bd"
            font.pixelSize: 13
            MouseArea {
                anchors.fill: parent
                anchors.margins: -6
                cursorShape: Qt.PointingHandCursor
                onClicked: bridge.update_transform_panel_visible(false)
            }
        }
    }

    // ★positioner（Grid / Row / Column）的子项不能再用 anchors 定位，
    //   否则整行尺寸算不出来（表现为「面板在、输入框全不见」）。这里用
    //   Column + Row，标签给定行高、靠 verticalAlignment 垂直居中。
    Column {
        id: fields
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.top: header.bottom
        anchors.topMargin: 10
        spacing: 6

        // X / Y 同一行
        Row {
            spacing: 6
            height: 22
            Text {
                text: "X"; width: 14; height: 22
                verticalAlignment: Text.AlignVCenter
                color: "#c8cdd4"; font.pixelSize: 11
            }
            SpinBox { id: px; width: 78; height: 22; from: -99999; to: 99999; onEdited: panel.apply() }
            Text {
                text: "Y"; width: 14; height: 22
                verticalAlignment: Text.AlignVCenter
                color: "#c8cdd4"; font.pixelSize: 11
            }
            SpinBox { id: py; width: 78; height: 22; from: -99999; to: 99999; onEdited: panel.apply() }
        }
        // W / H 同一行
        Row {
            spacing: 6
            height: 22
            Text {
                text: "W"; width: 14; height: 22
                verticalAlignment: Text.AlignVCenter
                color: "#c8cdd4"; font.pixelSize: 11
            }
            SpinBox { id: pw; width: 78; height: 22; from: 1; to: 99999; onEdited: panel.apply() }
            Text {
                text: "H"; width: 14; height: 22
                verticalAlignment: Text.AlignVCenter
                color: "#c8cdd4"; font.pixelSize: 11
            }
            SpinBox { id: ph; width: 78; height: 22; from: 1; to: 99999; onEdited: panel.apply() }
        }
        Row {
            spacing: 6
            height: 22
            Text {
                text: "∠"; width: 14; height: 22
                verticalAlignment: Text.AlignVCenter
                color: "#c8cdd4"; font.pixelSize: 11
            }
            SpinBox { id: rot; width: 78; height: 22; from: -3600; to: 3600; onEdited: panel.apply() }
        }
    }

    // 锁比例
    Rectangle {
        id: lockBox
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.top: fields.bottom
        anchors.topMargin: 10
        width: 14
        height: 14
        radius: 2
        color: panel.lockRatio ? "#5b9dff" : "#202328"
        border.color: "#6b7076"
        border.width: 1
        Text { anchors.centerIn: parent; text: panel.lockRatio ? "✓" : ""; color: "white"; font.pixelSize: 10 }
        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: panel.lockRatio = !panel.lockRatio
        }
    }
    Text {
        anchors.left: lockBox.right
        anchors.leftMargin: 6
        anchors.verticalCenter: lockBox.verticalCenter
        text: "锁定比例"
        color: "#c8cdd4"
        font.pixelSize: 11
    }

    // 3×3 参考点
    Grid {
        id: refGrid
        anchors.right: parent.right
        anchors.rightMargin: 10
        anchors.top: fields.bottom
        anchors.topMargin: 8
        columns: 3
        spacing: 3
        Repeater {
            model: 9
            Rectangle {
                width: 14; height: 14; radius: 2
                readonly property int cell: 8 - index
                color: panel.refPoint === cell ? "#5b9dff" : "#2c3036"
                border.color: "#5a6068"; border.width: 1
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: { panel.refPoint = cell; panel.apply() }
                }
            }
        }
    }
}
