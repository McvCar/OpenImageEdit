import QtQuick
import com.imageedit

// ---- 修复 / 油漆的参数面板 -----------------------------------------
// group 模式：贴在标记栏两个「工具组」按钮下面，列工具 + 参数。
// context 模式：画布右键弹在光标处，只列当前工具的参数（PS 的右键调参）。
Rectangle {
    id: pop
    required property var host
    required property var bridge
    required property var markBar
    // 与 FontPopover 一致：y 自己算（贴住工具栏下沿），placePopover 只负责 x。
    property var toolbar: null

    // "heal" = 修复组（污点修复/修复画笔/修补），"paint" = 油漆组（油漆桶/渐变）
    property string group: "heal"
    // "group" 贴着工具栏；"context" 跟着右键位置。
    property string mode: "group"
    property real anchorX: 0
    property real anchorY: 0

    visible: false
    width: 250
    height: col.implicitHeight + 16
    y: (toolbar ? toolbar.height : 0) + (markBar.visible ? markBar.height : 0) + 2
    color: "#2a2d31"
    border.color: "#3c4043"
    border.width: 1
    radius: 6
    z: 240

    onVisibleChanged: {
        if (!visible)
            return
        if (mode === "context") {
            x = Math.max(4, Math.min(anchorX, host.width - width - 4))
            y = Math.max(4, Math.min(anchorY, host.height - height - 4))
        } else {
            host.placePopover(pop, group === "heal" ? markBar.healBtn : markBar.paintBtn)
        }
    }

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
            text: pop.mode === "context"
                  ? (pop.group === "heal" ? "修复参数" : "油漆参数")
                  : (pop.group === "heal" ? "修复工具" : "油漆工具")
            color: "#8b9096"
            font.pixelSize: 10
        }

        // ---- 工具列表（仅 group 模式）----
        Column {
            width: parent.width
            spacing: 1
            visible: pop.mode === "group" && pop.group === "heal"

            ToolRow {
                glyph: "◍"
                label: "污点修复画笔"
                active: host.paintTool === "spot"
                tip: "涂一下，自动用周边像素盖掉瑕疵"
                onClicked: host.setPaintTool("spot")
            }
            ToolRow {
                glyph: "✚"
                label: "修复画笔"
                active: host.paintTool === "heal"
                tip: "Alt + 点击取样，再涂抹复制（保纹理、配明暗）"
                onClicked: host.setPaintTool("heal")
            }
            ToolRow {
                glyph: "◫"
                label: "修补"
                active: host.paintTool === "patch"
                tip: "先圈一块区域，再把它拖到取样位置"
                onClicked: host.setPaintTool("patch")
            }
        }
        Column {
            width: parent.width
            spacing: 1
            visible: pop.mode === "group" && pop.group === "paint"

            ToolRow {
                glyph: "◧"
                label: "油漆桶"
                active: host.paintTool === "bucket"
                tip: "按颜色容差填一片（可限制在选区内）"
                onClicked: host.setPaintTool("bucket")
            }
            ToolRow {
                glyph: "▤"
                label: "渐变"
                active: host.paintTool === "gradient"
                tip: "拖一条线定义方向，起止色 = 描边色 → 填充色"
                onClicked: host.setPaintTool("gradient")
            }
        }

        Rectangle {
            width: parent.width
            height: 1
            color: "#3c4043"
            visible: pop.mode === "group"
        }

        // ---- 修复参数 ----
        Column {
            width: parent.width
            spacing: 5
            visible: pop.group === "heal" && host.paintTool !== "patch"
                    && host.paintTool !== "bucket" && host.paintTool !== "gradient"

            ParamRow {
                label: "大小"
                value: host.healSize
                from: 2; to: 400; step: 2
                onEdited: (v) => host.setHealSize(v)
            }
            ParamRow {
                label: "硬度"
                value: host.healHardness
                from: 0; to: 100; step: 5
                onEdited: (v) => host.setHealHardness(v)
            }
            ParamRow {
                label: "间距"
                value: host.healSpacing
                from: 1; to: 100; step: 5
                onEdited: (v) => host.setHealSpacing(v)
            }
            ParamRow {
                label: "强度"
                value: host.healStrength
                from: 1; to: 100; step: 5
                onEdited: (v) => host.setHealStrength(v)
            }
            Text {
                width: parent.width
                wrapMode: Text.WordWrap
                text: host.paintTool === "heal"
                      ? (host.healHasSource
                         ? "已取样：在画布上按住涂抹。Alt + 点击可重新取样"
                         : "先 Alt + 点击一处干净的地方取样")
                      : "按住涂抹，松手才记一条撤销（[ / ] 调笔刷）"
                color: "#6b7076"
                font.pixelSize: 10
            }
        }

        // ---- 修补参数 ----
        Column {
            width: parent.width
            spacing: 5
            visible: host.paintTool === "patch"

            ParamRow {
                label: "扩散"
                value: host.patchFeather
                from: 0; to: 40; step: 1
                onEdited: (v) => host.setPatchFeather(v)
            }
            Text {
                width: parent.width
                wrapMode: Text.WordWrap
                text: host.patchHasRegion
                      ? "按住区域内拖动到取样处松手；Esc 放弃这块区域"
                      : "先拖出一个区域，再把它拖到干净的取样处"
                color: "#6b7076"
                font.pixelSize: 10
            }
        }

        // ---- 油漆桶参数 ----
        Column {
            width: parent.width
            spacing: 5
            visible: host.paintTool === "bucket"

            ParamRow {
                label: "容差"
                value: host.bucketTolerance
                from: 0; to: 255; step: 4
                onEdited: (v) => host.setBucketTolerance(v)
            }
            ParamRow {
                label: "不透明"
                value: host.bucketOpacity
                from: 1; to: 100; step: 5
                onEdited: (v) => host.setBucketOpacity(v)
            }
            Row {
                spacing: 4
                Pill {
                    label: "连续"
                    checked: host.bucketContiguous
                    onClicked: host.setBucketContiguous(true)
                }
                Pill {
                    label: "全图同色"
                    checked: !host.bucketContiguous
                    onClicked: host.setBucketContiguous(false)
                }
            }
            Row {
                spacing: 6
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "颜色"
                    color: "#e9eaec"
                    font.pixelSize: 11
                }
                Rectangle {
                    width: 16; height: 16; radius: 3
                    color: host.markColor
                    border.color: "#e9eaec"
                    border.width: 1
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: host.openColorFor(false)
                    }
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "点击换色 · 选区内的填充"
                    color: "#6b7076"
                    font.pixelSize: 10
                }
            }
        }

        // ---- 渐变参数 ----
        Column {
            width: parent.width
            spacing: 5
            visible: host.paintTool === "gradient"

            Row {
                spacing: 4
                Pill {
                    label: "线性"
                    checked: !host.gradRadial
                    onClicked: host.setGradRadial(false)
                }
                Pill {
                    label: "径向"
                    checked: host.gradRadial
                    onClicked: host.setGradRadial(true)
                }
                Pill {
                    label: "反向"
                    checked: host.gradReverse
                    onClicked: host.toggleGradReverse()
                }
            }
            ParamRow {
                label: "不透明"
                value: host.gradOpacity
                from: 1; to: 100; step: 5
                onEdited: (v) => host.setGradOpacity(v)
            }
            Row {
                spacing: 8
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "起止色"
                    color: "#e9eaec"
                    font.pixelSize: 11
                }
                Rectangle {
                    width: 22; height: 16; radius: 3
                    color: host.markColor
                    border.color: "#e9eaec"
                    border.width: 1
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: host.openColorFor(false)
                    }
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "→"
                    color: "#8b9096"
                    font.pixelSize: 12
                }
                Rectangle {
                    width: 22; height: 16; radius: 3
                    color: host.markFillColor
                    border.color: "#e9eaec"
                    border.width: 1
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: host.openColorFor(true)
                    }
                }
            }
            Text {
                width: parent.width
                wrapMode: Text.WordWrap
                text: "在画布上拖一条线：起点 → 终点。有选区时只铺在选区里"
                color: "#6b7076"
                font.pixelSize: 10
            }
        }
    }

    // ---- 内部小组件 ----

    // 一行工具：图标 + 名称，选中态高亮。
    component ToolRow: Rectangle {
        property string glyph: ""
        property string label: ""
        property string tip: ""
        property bool active: false
        signal clicked()
        width: parent ? parent.width : 0
        height: 24
        radius: 4
        color: active ? "#3d5a80" : (rowHover.containsMouse ? "#3a3d42" : "transparent")
        Text {
            anchors.left: parent.left
            anchors.leftMargin: 4
            anchors.verticalCenter: parent.verticalCenter
            width: 20
            horizontalAlignment: Text.AlignHCenter
            text: parent.glyph
            color: parent.active ? "#ffffff" : "#e9eaec"
            font.pixelSize: 13
        }
        Text {
            anchors.left: parent.left
            anchors.leftMargin: 28
            anchors.verticalCenter: parent.verticalCenter
            text: parent.label
            color: parent.active ? "#cfe4ff" : "#b9bec5"
            font.pixelSize: 11
        }
        MouseArea {
            id: rowHover
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: parent.clicked()
        }
    }

    // 数值行：标签 + 数字框。
    component ParamRow: Row {
        property string label: ""
        property int value: 0
        property int from: 0
        property int to: 100
        property int step: 1
        signal edited(int v)
        spacing: 6
        height: 22
        Text {
            anchors.verticalCenter: parent.verticalCenter
            width: 40
            text: parent.label
            color: "#e9eaec"
            font.pixelSize: 11
        }
        SpinBox {
            width: 64
            from: parent.from
            to: parent.to
            step: parent.step
            value: parent.value
            onEdited: (v) => parent.edited(v)
        }
    }

    // 两态胶囊。
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
