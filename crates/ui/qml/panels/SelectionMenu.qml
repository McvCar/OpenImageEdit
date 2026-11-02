import QtQuick
import com.imageedit

// ---- 选区右键菜单（PS 的 marquee 右键）------------------------------
// 命中选区内才弹出（Rust 判断 alpha>127，含羽化软边）。三项数值算子直接
// 带输入框：改完立刻作用于当前选区（Rust 侧是像素域算子，快），不做
// 「确定/取消」的模态弹窗——比 PS 少一步。
Rectangle {
    id: selMenu
    required property var host
    required property var bridge
    // 弹出锚点（root 坐标，右键位置）。
    property real anchorX: 0
    property real anchorY: 0

    visible: false
    width: 256
    height: body.implicitHeight + 12
    radius: 6
    color: "#2b2d31"
    border.color: "#4a4d52"
    border.width: 1
    z: 300
    x: Math.max(4, Math.min(anchorX, host.width - width - 4))
    y: Math.max(4, Math.min(anchorY, host.height - height - 4))

    // 数值框是否正在编辑（会临时禁用 M/L/Q 这类单字母快捷键）。
    readonly property bool editing: borderSpin.editing || smoothSpin.editing
                                    || featherSpin.editing
    onEditingChanged: host.selFieldEditing = editing

    function open(x, y) {
        anchorX = x
        anchorY = y
        visible = true
        host.selMenuOpen = true
    }
    // 结束三个数值框的编辑：丢弃未提交的输入 + 交出键盘焦点。焦点留在
    // TextInput 上时 Ctrl+Z 会被它当「文字撤销」吃掉，Del 也只删字符，
    // 文档级快捷键看起来全死了。
    function endEditing() {
        borderSpin.endEdit()
        smoothSpin.endEdit()
        featherSpin.endEdit()
    }
    function close() {
        endEditing()
        visible = false
        host.selMenuOpen = false
        host.selFieldEditing = false
    }

    // 菜单自身吃掉点击。原来菜单背景只是个 Rectangle、没有 MouseArea，
    // 点在菜单内的空白处（行与行之间、右侧 px 单位附近）会穿透到下面那层
    // 「点空白处关菜单」的全屏 MouseArea，于是看起来像误触关闭。
    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.AllButtons
        z: -1
    }

    Column {
        id: body
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.topMargin: 6
        spacing: 0

        // ---- 选区类型（单选）----
        // 形状原本只能在标记栏的浮动面板里切：想改边界/平滑就得先关菜单、
        // 开面板、再右键一次。这里放一组单选按钮就地切换；切换后菜单保持
        // 打开，方便接着调下面的数值。
        Text {
            x: 12
            width: body.width - 24
            height: 20
            verticalAlignment: Text.AlignVCenter
            text: "选区类型"
            color: "#8b9096"
            font.pixelSize: 10
        }
        Grid {
            width: body.width
            columns: 2
            rowSpacing: 2
            columnSpacing: 2
            bottomPadding: 4
            Repeater {
                model: [
                    { key: "rect", glyph: "▭", label: "矩形选框" },
                    { key: "ellipse", glyph: "◯", label: "椭圆选框" },
                    { key: "lasso", glyph: "∿", label: "套索" },
                    { key: "polygon", glyph: "⬡", label: "多边形" }
                ]
                delegate: Item {
                    id: selTypeCell
                    required property var modelData
                    readonly property bool picked: host.selTool === modelData.key
                    width: (body.width - 8) / 2
                    height: 26

                    Rectangle {
                        x: 4
                        width: parent.width - 4
                        height: parent.height
                        radius: 4
                        color: cellMa.containsMouse ? "#3d5a80" : "transparent"
                    }
                    // 单选圆点：空心圈 + 选中时填实心
                    Rectangle {
                        x: 9
                        anchors.verticalCenter: parent.verticalCenter
                        width: 12
                        height: 12
                        radius: 6
                        color: "transparent"
                        border.width: 1
                        border.color: selTypeCell.picked ? "#7fc3ff" : "#6b7076"
                        Rectangle {
                            anchors.centerIn: parent
                            width: 6
                            height: 6
                            radius: 3
                            color: "#7fc3ff"
                            visible: selTypeCell.picked
                        }
                    }
                    Text {
                        anchors.left: parent.left
                        anchors.leftMargin: 27
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        elide: Text.ElideRight
                        text: selTypeCell.modelData.glyph + "  " + selTypeCell.modelData.label
                        color: selTypeCell.picked ? "#cfe4ff" : "#e9eaec"
                        font.pixelSize: 11
                    }
                    MouseArea {
                        id: cellMa
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        // 只切工具、不关菜单（也不能走 pickSelTool，那个会顺手
                        // 收起浮动面板）。
                        onClicked: host.setSelTool(selTypeCell.modelData.key)
                    }
                }
            }
        }

        // ---- 分隔线 ----
        Item {
            width: body.width
            height: 9
            Rectangle {
                anchors.centerIn: parent
                width: parent.width - 24
                height: 1
                color: "#3c4043"
            }
        }

        // ---- 取消选择 ----
        Item {
            width: body.width
            height: 28
            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 4
                anchors.rightMargin: 4
                radius: 4
                color: m1.containsMouse ? "#3d5a80" : "transparent"
                Text {
                    x: 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: "取消选择"
                    color: "#e9eaec"
                    font.pixelSize: 12
                }
                Text {
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Ctrl+D"
                    color: "#8b9096"
                    font.pixelSize: 10
                }
            }
            MouseArea {
                id: m1
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    selMenu.close()
                    bridge.sel_clear()
                }
            }
        }

        // ---- 选择反选 ----
        Item {
            width: body.width
            height: 28
            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 4
                anchors.rightMargin: 4
                radius: 4
                color: m2.containsMouse ? "#3d5a80" : "transparent"
                Text {
                    x: 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: "选择反选"
                    color: "#e9eaec"
                    font.pixelSize: 12
                }
                Text {
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Ctrl+Shift+I"
                    color: "#8b9096"
                    font.pixelSize: 10
                }
            }
            MouseArea {
                id: m2
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                    selMenu.close()
                    bridge.sel_invert()
                }
            }
        }

        // ---- 分隔线 ----
        Item {
            width: body.width
            height: 9
            Rectangle {
                anchors.centerIn: parent
                width: parent.width - 24
                height: 1
                color: "#3c4043"
            }
        }

        // ---- 边界（PS: 往内外扩 k/2 px，取环带）----
        Item {
            width: body.width
            height: 32
            Text {
                x: 12
                anchors.verticalCenter: parent.verticalCenter
                text: "边界"
                color: "#e9eaec"
                font.pixelSize: 12
            }
            SpinBox {
                id: borderSpin
                width: 78
                anchors.right: parent.right
                anchors.rightMargin: 40
                anchors.verticalCenter: parent.verticalCenter
                from: 0
                to: 400
                step: 5
                value: bridge.selBorder
                undoHook: () => bridge.undo()
                redoHook: () => bridge.redo()
                onEdited: (v) => bridge.sel_set_border(v)
            }
            Text {
                anchors.left: borderSpin.right
                anchors.leftMargin: 5
                anchors.verticalCenter: parent.verticalCenter
                text: "px"
                color: "#8b9096"
                font.pixelSize: 11
            }
        }

        // ---- 平滑 ----
        Item {
            width: body.width
            height: 32
            Text {
                x: 12
                anchors.verticalCenter: parent.verticalCenter
                text: "平滑"
                color: "#e9eaec"
                font.pixelSize: 12
            }
            SpinBox {
                id: smoothSpin
                width: 78
                anchors.right: parent.right
                anchors.rightMargin: 40
                anchors.verticalCenter: parent.verticalCenter
                from: 0
                to: 400
                step: 2
                value: bridge.selSmooth
                undoHook: () => bridge.undo()
                redoHook: () => bridge.redo()
                onEdited: (v) => bridge.sel_set_smooth(v)
            }
            Text {
                anchors.left: smoothSpin.right
                anchors.leftMargin: 5
                anchors.verticalCenter: parent.verticalCenter
                text: "px"
                color: "#8b9096"
                font.pixelSize: 11
            }
        }

        // ---- 羽化 ----
        Item {
            width: body.width
            height: 32
            Text {
                x: 12
                anchors.verticalCenter: parent.verticalCenter
                text: "羽化"
                color: "#e9eaec"
                font.pixelSize: 12
            }
            SpinBox {
                id: featherSpin
                width: 78
                anchors.right: parent.right
                anchors.rightMargin: 40
                anchors.verticalCenter: parent.verticalCenter
                from: 0
                to: 250
                step: 2
                value: bridge.selFeather
                undoHook: () => bridge.undo()
                redoHook: () => bridge.redo()
                onEdited: (v) => bridge.sel_set_feather(v)
            }
            Text {
                anchors.left: featherSpin.right
                anchors.leftMargin: 5
                anchors.verticalCenter: parent.verticalCenter
                text: "px"
                color: "#8b9096"
                font.pixelSize: 11
            }
        }

        Item {
            width: body.width
            height: 9
            Rectangle {
                anchors.centerIn: parent
                width: parent.width - 24
                height: 1
                color: "#3c4043"
            }
        }

        // ---- 快捷键提示 ----
        Item {
            width: body.width
            height: 22
            Text {
                x: 12
                anchors.verticalCenter: parent.verticalCenter
                text: "Shift 加选 · Alt 减选 · Q 快速蒙版"
                color: "#8b9096"
                font.pixelSize: 10
            }
        }
    }
}
