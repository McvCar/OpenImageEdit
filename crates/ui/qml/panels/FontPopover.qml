import QtQuick
import com.imageedit

// 文本样式面板（macOS 风格）：字体下拉 + 颜色 / 字号步进 + B I U / 对齐。
Rectangle {
    id: fontPopover
    required property var host
    required property var toolbar
    required property var markBar
    required property var colorPopover
    // 暴露给 EditorContent：stepMarkFontSize 回写字号（用户手动输入打断声明式绑定）。
    readonly property alias sizeInput: sizeInput
    visible: false
    z: 220
    width: 276
    height: 134
    color: "#26282c"
    border.color: "#3c4043"
    radius: 8
    y: toolbar.height + (markBar.visible ? markBar.height : 0) + 2
    onVisibleChanged: {
        if (visible)
            host.placePopover(fontPopover, markBar.fontBtn)
        else
            fontListOpen = false
    }

    property bool fontListOpen: false

    Column {
        anchors.fill: parent
        anchors.margins: 10
        spacing: 7

        // 行 1：字体下拉 + 文字颜色块。
        Row {
            spacing: 7

            Rectangle {
                id: fontDropBtn
                width: 202
                height: 30
                radius: 6
                color: "#2f3237"
                border.color: fontPopover.fontListOpen ? "#378add" : "#3c4043"
                border.width: 1
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 10
                    text: host.markFontName(host.markFont)
                    color: "#e9eaec"
                    font.pixelSize: 13
                    font.family: host.qmlFontFamily(host.markFont)
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: 8
                    text: "▾"
                    color: "#9aa3b2"
                    font.pixelSize: 11
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: fontPopover.fontListOpen = !fontPopover.fontListOpen
                }
            }

            Rectangle {
                width: 30
                height: 30
                radius: 6
                color: "#2f3237"
                border.color: "#3c4043"
                Rectangle {
                    anchors.centerIn: parent
                    width: 16
                    height: 16
                    radius: 3
                    color: host.markColor
                    border.color: "#e9eaec"
                    border.width: 1
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        // 文字颜色 = 描边目标；转交给颜色弹窗。
                        colorPopover.editingFill = false
                        fontPopover.visible = false
                        colorPopover.visible = true
                    }
                }
            }
        }

        // 行 2：字号步进 + B / I / U。
        Row {
            spacing: 6

            Rectangle {
                width: 88
                height: 30
                radius: 6
                color: "#2f3237"
                border.color: "#3c4043"
                // 先声明：垫底拦截空白点击（箭头声明在后、层级更高）。
                MouseArea {
                    anchors.fill: parent
                }
                // 字号：默认步进档显示 + 可输入任意自定义值（4-512），
                // Enter 或失焦提交；▲▼ 步进仍可用。
                TextInput {
                    id: sizeInput
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 12
                    width: 48
                    text: host.markFontSize
                    color: "#e9eaec"
                    font.pixelSize: 13
                    validator: IntValidator { bottom: 4; top: 512 }
                    selectByMouse: true
                    onActiveFocusChanged: if (activeFocus) selectAll()
                    onEditingFinished: {
                        const v = parseInt(text, 10)
                        if (!isNaN(v)) {
                            const clamped = Math.max(4, Math.min(512, v))
                            if (clamped !== host.markFontSize) {
                                host.markFontSize = clamped
                                host.applyTextFont()
                            }
                        }
                        text = host.markFontSize
                    }
                }
                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: 4
                    spacing: 1
                    // ▲
                    Item {
                        width: 20
                        height: 13
                        Text {
                            anchors.centerIn: parent
                            text: "▲"
                            color: sizeUpMa.containsMouse ? "#ffffff" : "#9aa3b2"
                            font.pixelSize: 8
                        }
                        MouseArea {
                            id: sizeUpMa
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: host.stepMarkFontSize(1)
                        }
                    }
                    // ▼
                    Item {
                        width: 20
                        height: 13
                        Text {
                            anchors.centerIn: parent
                            text: "▼"
                            color: sizeDownMa.containsMouse ? "#ffffff" : "#9aa3b2"
                            font.pixelSize: 8
                        }
                        MouseArea {
                            id: sizeDownMa
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: host.stepMarkFontSize(-1)
                        }
                    }
                }
            }

            Repeater {
                model: [
                    { key: "b", label: "B", get: function () { return host.markBold },
                      set: function (v) { host.markBold = v } },
                    { key: "i", label: "I", get: function () { return host.markItalic },
                      set: function (v) { host.markItalic = v } },
                    { key: "u", label: "U", get: function () { return host.markUnderline },
                      set: function (v) { host.markUnderline = v } }
                ]

                delegate: Rectangle {
                    required property var modelData
                    width: 30
                    height: 30
                    radius: 6
                    color: modelData.get() ? "#3d5a80" : "#2f3237"
                    border.color: modelData.get() ? "#5a7fb0" : "#3c4043"
                    border.width: 1
                    Text {
                        anchors.centerIn: parent
                        text: parent.modelData.label
                        color: parent.modelData.get() ? "#ffffff" : "#c8ccd2"
                        font.pixelSize: 13
                        font.bold: true
                        font.italic: parent.modelData.key === "i"
                    }
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            parent.modelData.set(!parent.modelData.get())
                            host.applyTextFont()
                        }
                    }
                }
            }
        }

        // 行 3：对齐（左 / 中 / 右）。
        Row {
            spacing: 6

            Repeater {
                model: [0, 1, 2]

                delegate: Rectangle {
                    required property int modelData
                    width: 30
                    height: 30
                    radius: 6
                    color: host.markAlign === modelData ? "#3d5a80" : "#2f3237"
                    border.color: host.markAlign === modelData ? "#5a7fb0" : "#3c4043"
                    border.width: 1

                    // 三条横线的对齐图标：锚点按对齐方式互斥设置。
                    Rectangle { width: 16; height: 2; radius: 1; y: 8
                        color: "#c8ccd2"
                        anchors.left: modelData === 0 ? parent.left : undefined
                        anchors.leftMargin: 7
                        anchors.horizontalCenter: modelData === 1 ? parent.horizontalCenter : undefined
                        anchors.right: modelData === 2 ? parent.right : undefined
                        anchors.rightMargin: 7
                    }
                    Rectangle { width: 16; height: 2; radius: 1; y: 14
                        color: "#c8ccd2"
                        anchors.left: modelData === 0 ? parent.left : undefined
                        anchors.leftMargin: 7
                        anchors.horizontalCenter: modelData === 1 ? parent.horizontalCenter : undefined
                        anchors.right: modelData === 2 ? parent.right : undefined
                        anchors.rightMargin: 7
                    }
                    Rectangle { width: 10; height: 2; radius: 1; y: 20
                        color: "#c8ccd2"
                        anchors.left: modelData === 0 ? parent.left : undefined
                        anchors.leftMargin: 7
                        anchors.horizontalCenter: modelData === 1 ? parent.horizontalCenter : undefined
                        anchors.right: modelData === 2 ? parent.right : undefined
                        anchors.rightMargin: 7
                    }
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            host.markAlign = modelData
                            host.applyTextFont()
                        }
                    }
                }
            }
        }
    }

    // 字体下拉列表：覆盖在面板下半部（不影响面板自身布局）。
    Rectangle {
        visible: fontPopover.fontListOpen
        z: 15
        x: 10
        y: 46
        width: parent.width - 20
        height: listCol.height + 8
        color: "#2f3237"
        border.color: "#3c4043"
        radius: 6

        Column {
            id: listCol
            x: 4
            y: 4
            width: parent.width - 8
            spacing: 2

            Repeater {
                model: host.fontChoices

                delegate: Rectangle {
                    required property var modelData
                    width: listCol.width
                    height: 30
                    radius: 4
                    color: host.markFont === modelData.id ? "#3d5a80"
                           : fontListHover.containsMouse ? "#3a3d42" : "transparent"
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.left: parent.left
                        anchors.leftMargin: 10
                        text: parent.modelData.name
                        color: "#e9eaec"
                        font.pixelSize: 13
                        font.family: parent.modelData.qml
                    }
                    Text {
                        visible: host.markFont === parent.modelData.id
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.right: parent.right
                        anchors.rightMargin: 10
                        text: "✓"
                        color: "#ffffff"
                        font.pixelSize: 13
                    }
                    MouseArea {
                        id: fontListHover
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            host.markFont = parent.modelData.id
                            host.applyTextFont()
                            fontPopover.fontListOpen = false
                        }
                    }
                }
            }
        }
    }
}

