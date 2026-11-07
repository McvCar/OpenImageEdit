import QtQuick
import com.imageedit

// P0-5 变换数值面板（D7 / D10 / D13）。
// * 首次默认隐藏，由工具栏按钮切换；显示状态在 settings.ini 里跨启动记忆。
// * X/Y/W/H 都是「变换后轴对齐外接框」语义，可为负、可超出画布。
// * 一次提交 = 一条 SetLayerTransform 历史；不逐键写文档。
Rectangle {
    id: panel
    required property var bridge

    readonly property var vals: {
        try { return JSON.parse(bridge.transformValuesJson) } catch (e) { return {has: false} }
    }

    visible: bridge.transformPanelVisible && vals.has === true
    width: 236
    height: 266
    radius: 6
    color: "#f21d2024"
    border.color: "#5a6068"
    border.width: 1
    z: 20

    property bool lockRatio: true
    property int refPoint: 0
    property bool flipX: false
    property bool flipY: false
    property bool suppress: false

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
        flipX = vals.fx === true
        flipY = vals.fy === true
        suppress = false
    }

    function apply() {
        if (suppress || !vals.has)
            return
        bridge.transform_set_params(px.value, py.value, pw.value, ph.value,
                                    rot.value, flipX, flipY, lockRatio, refPoint)
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

        Row {
            spacing: 6
            height: 22
            Text {
                text: "X"; width: 14; height: 22
                verticalAlignment: Text.AlignVCenter
                color: "#c8cdd4"; font.pixelSize: 11
            }
            SpinBox { id: px; width: 92; height: 22; from: -99999; to: 99999; onEdited: panel.apply() }
        }
        Row {
            spacing: 6
            height: 22
            Text {
                text: "Y"; width: 14; height: 22
                verticalAlignment: Text.AlignVCenter
                color: "#c8cdd4"; font.pixelSize: 11
            }
            SpinBox { id: py; width: 92; height: 22; from: -99999; to: 99999; onEdited: panel.apply() }
        }
        Row {
            spacing: 6
            height: 22
            Text {
                text: "W"; width: 14; height: 22
                verticalAlignment: Text.AlignVCenter
                color: "#c8cdd4"; font.pixelSize: 11
            }
            SpinBox { id: pw; width: 92; height: 22; from: 1; to: 99999; onEdited: panel.apply() }
        }
        Row {
            spacing: 6
            height: 22
            Text {
                text: "H"; width: 14; height: 22
                verticalAlignment: Text.AlignVCenter
                color: "#c8cdd4"; font.pixelSize: 11
            }
            SpinBox { id: ph; width: 92; height: 22; from: 1; to: 99999; onEdited: panel.apply() }
        }
        Row {
            spacing: 6
            height: 22
            Text {
                text: "∠"; width: 14; height: 22
                verticalAlignment: Text.AlignVCenter
                color: "#c8cdd4"; font.pixelSize: 11
            }
            SpinBox { id: rot; width: 92; height: 22; from: -3600; to: 3600; onEdited: panel.apply() }
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
    Text {
        anchors.right: refGrid.right
        anchors.top: refGrid.bottom
        anchors.topMargin: 2
        text: "参考点"
        color: "#8b929b"
        font.pixelSize: 10
    }

    // 翻转 + 参考线开关
    Row {
        id: toggles
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.top: lockBox.bottom
        anchors.topMargin: 12
        spacing: 6
        Repeater {
            model: [
                {t: "水平翻转", v: panel.flipX, k: 0},
                {t: "垂直翻转", v: panel.flipY, k: 1}
            ]
            Rectangle {
                width: 66; height: 20; radius: 3
                color: modelData.v ? "#3b5c8f" : "#2c3036"
                border.color: "#5a6068"; border.width: 1
                Text { anchors.centerIn: parent; text: modelData.t; color: "#dfe3e9"; font.pixelSize: 10 }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        if (modelData.k === 0) panel.flipX = !panel.flipX
                        else panel.flipY = !panel.flipY
                        panel.apply()
                    }
                }
            }
        }
    }

    Row {
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.top: toggles.bottom
        anchors.topMargin: 10
        spacing: 10
        Repeater {
            model: [
                {t: "显示参考线", v: bridge.showGuides, k: 0},
                {t: "智能吸附", v: bridge.snapEnabled, k: 1}
            ]
            Rectangle {
                width: 12; height: 12; radius: 2
                color: modelData.v ? "#5b9dff" : "#202328"
                border.color: "#6b7076"; border.width: 1
                Text { anchors.centerIn: parent; text: modelData.v ? "✓" : ""; color: "white"; font.pixelSize: 9 }
                MouseArea {
                    anchors.fill: parent
                    anchors.margins: -4
                    cursorShape: Qt.PointingHandCursor
                    onClicked: modelData.k === 0
                        ? bridge.update_show_guides(!bridge.showGuides)
                        : bridge.update_snap_enabled(!bridge.snapEnabled)
                }
            }
        }
        Text { text: "参考线"; color: "#8b929b"; font.pixelSize: 10; anchors.verticalCenter: parent.verticalCenter }
        Text { text: "吸附"; color: "#8b929b"; font.pixelSize: 10; anchors.verticalCenter: parent.verticalCenter }
    }
}
