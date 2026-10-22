import QtQuick
import com.imageedit

// 颜色弹窗：描边/填充切换 + 预置色板。
Rectangle {
    id: colorPopover
    required property var host
    required property var toolbar
    required property var markBar
    // 暴露给 EditorContent：customHexApply 读取用户输入的自定义色值。
    readonly property alias customHexInput: customHexInput
    visible: false
    z: 220
    width: 250
    height: 208
    color: "#2a2d31"
    border.color: "#3c4043"
    radius: 6
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    onVisibleChanged: if (visible) host.placePopover(colorPopover, markBar.colorBtn)

    property bool editingFill: false

    Row {
        id: colorTargets
        anchors.top: parent.top
        anchors.topMargin: 8
        anchors.left: parent.left
        anchors.leftMargin: 10
        spacing: 8

        Repeater {
            model: [
                { label: "描边", bind: function () { return host.markColor } },
                { label: "填充", bind: function () { return host.markFillColor } }
            ]

            delegate: Item {
                required property var modelData
                // 固定尺寸：Row 子项的尺寸不得反向依赖 Row 自身，
                // 否则触发 polish() 循环，Qt 会放弃整轮布局（连主工具栏
                // 的图标 Row 都不再排版）。宽 = 16 色块 + 4 间距 + 文本。
                width: 16 + 4 + swatchLabel.implicitWidth
                height: 16

                Rectangle {
                    x: 0
                    y: 0
                    width: 16
                    height: 16
                    radius: 3
                    color: modelData.bind()
                    border.color: (colorPopover.editingFill === (modelData.label === "填充"))
                                  ? "#378add" : "#5a5e64"
                    border.width: 1
                }
                Text {
                    id: swatchLabel
                    x: 20
                    anchors.verticalCenter: parent.verticalCenter
                    text: modelData.label
                    color: (colorPopover.editingFill === (modelData.label === "填充"))
                           ? "#e9eaec" : "#9aa3b2"
                    font.pixelSize: 11
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: colorPopover.editingFill = modelData.label === "填充"
                }
            }
        }
    }

    Grid {
        anchors.top: colorTargets.bottom
        anchors.topMargin: 8
        anchors.left: parent.left
        anchors.leftMargin: 10
        columns: 8
        spacing: 6

        Repeater {
            model: [
                "#e24b4a", "#ef9f27", "#f5e14a", "#639922",
                "#378add", "#7f77dd", "#e94aa0", "#111111",
                "#ffffff", "#9aa3b2", "#4a4e54", "#0f6e56",
                "#993556", "#2d6a9f", "#ba7517", "#5f5e5a"
            ]

            delegate: Rectangle {
                required property string modelData
                width: 22
                height: 22
                radius: 4
                color: modelData
                border.width: 2
                border.color: {
                    const cur = colorPopover.editingFill
                                ? host.markFillColor : host.markColor
                    return cur.toLowerCase() === modelData.toLowerCase()
                           ? "#378add" : "transparent"
                }

                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        if (colorPopover.editingFill) {
                            host.markFillColor = modelData
                            host.markFill = true
                        } else {
                            host.markColor = modelData
                        }
                        applyMarkStyle()
                    }
                }
            }
        }
    }

    // 自定义颜色：hex 输入 + 实时预览色块。
    Row {
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 8
        spacing: 8

        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: "自定义"
            color: "#9aa3b2"
            font.pixelSize: 11
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 20
            height: 20
            radius: 3
            color: customHexInput.valid ? "#" + customHexInput.text : "#33373c"
            border.color: "#5a5e64"
            border.width: 1
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 96
            height: 22
            radius: 4
            color: "#222428"
            border.color: customHexInput.activeFocus ? "#378add" : "#3c4043"
            border.width: 1

            TextInput {
                id: customHexInput
                anchors.fill: parent
                anchors.leftMargin: 6
                anchors.rightMargin: 6
                verticalAlignment: TextInput.AlignVCenter
                color: "#e9eaec"
                font.pixelSize: 12
                font.family: "Consolas"
                text: "e24b4a"
                maximumLength: 6
                // 6 位十六进制才有效（不含 #，输入更顺手）。
                readonly property bool valid: /^[0-9a-fA-F]{6}$/.test(text)
                selectByMouse: true
                onAccepted: host.customHexApply()
            }
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 44
            height: 22
            radius: 4
            color: customHexApplyMa.containsMouse ? "#3d5a80" : "#33373c"
            Text {
                anchors.centerIn: parent
                text: "应用"
                color: "#e9eaec"
                font.pixelSize: 11
            }
            MouseArea {
                id: customHexApplyMa
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: host.customHexApply()
            }
        }
    }
}

