import QtQuick
import com.imageedit

// ---- 内嵌色板 -------------------------------------------------------
// 给 PaintPopover 这类"就地调参"的浮层用：点色块后在参数面板里原地展开。
// 以前的做法是调 openColorFor() 把用户带到顶部工具栏的 ColorPopover，顺手
// 把参数面板关掉 —— 调完颜色还得重新打开面板，非常打断。
//
// 只有色板本身（16 色 + 自定义 hex），不含描边/填充切换：调用方已经在
// 自己的标题里说明点的是哪个颜色。
Item {
    id: palette
    // 当前颜色（#rrggbb），用来画选中框。
    property string current: "#ffffff"
    signal picked(string color)

    width: parent ? parent.width : 234
    implicitHeight: grid.implicitHeight + 6 + hexRow.height

    Grid {
        id: grid
        columns: 8
        spacing: 5

        Repeater {
            model: [
                "#e24b4a", "#ef9f27", "#f5e14a", "#639922",
                "#378add", "#7f77dd", "#e94aa0", "#111111",
                "#ffffff", "#9aa3b2", "#4a4e54", "#0f6e56",
                "#993556", "#2d6a9f", "#ba7517", "#5f5e5a"
            ]

            delegate: Rectangle {
                required property string modelData
                width: 20
                height: 20
                radius: 4
                color: modelData
                border.width: 2
                // 由 palette 记账的当前色高亮；transparent 时 Qt 不描边。
                // String(...) 兜底：调用方绑定的颜色属性在极早期可能还是
                // undefined，直接 .toLowerCase() 会每帧抛 TypeError 刷屏。
                border.color: String(palette.current || "").toLowerCase() === modelData.toLowerCase()
                              ? "#378add" : "transparent"

                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: palette.picked(modelData)
                }
            }
        }
    }

    Row {
        id: hexRow
        anchors.top: grid.bottom
        anchors.topMargin: 6
        spacing: 6

        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: "自定义"
            color: "#9aa3b2"
            font.pixelSize: 10
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 84
            height: 22
            radius: 4
            color: "#222428"
            border.color: hexInput.activeFocus ? "#378add" : "#3c4043"
            border.width: 1

            TextInput {
                id: hexInput
                anchors.fill: parent
                anchors.leftMargin: 6
                anchors.rightMargin: 6
                verticalAlignment: TextInput.AlignVCenter
                color: "#e9eaec"
                font.pixelSize: 11
                font.family: "Consolas"
                maximumLength: 6
                // 6 位十六进制才有效（不含 #，输入更顺手）。
                readonly property bool valid: /^[0-9a-fA-F]{6}$/.test(text)
                selectByMouse: true
                onAccepted: if (valid) palette.picked("#" + text.toLowerCase())
            }
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 40
            height: 22
            radius: 4
            color: applyMa.containsMouse ? "#3d5a80" : "#33373c"
            Text {
                anchors.centerIn: parent
                text: "应用"
                color: "#e9eaec"
                font.pixelSize: 10
            }
            MouseArea {
                id: applyMa
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: if (hexInput.valid) palette.picked("#" + hexInput.text.toLowerCase())
            }
        }
    }
}
