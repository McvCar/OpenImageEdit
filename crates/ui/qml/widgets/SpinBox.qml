import QtQuick

// 数值输入 + 右侧上下箭头（PS 弹窗里那种小 steps）。
// 用于选区的「边界 / 平滑 / 羽化」：拖动/点击箭头即时生效，直接改数值
// 回车（或失焦）才提交，避免每敲一个数字就整幅重算。
Rectangle {
    id: sb

    property int value: 0
    property int from: 0
    property int to: 400
    property int step: 1
    // 后缀单位（"px"）——只用于宽度估算，输入框里不显示。
    property string unit: "px"
    // 编辑期间按 Ctrl+Z / Ctrl+Y：TextInput 会先把它们当「文字撤销」吃掉
    // （它的撤销栈里只有刚敲的两个字符），文档撤销看起来就没反应。宿主
    // 挂上这两个钩子，输入框里的撤销键也照样作用于文档。
    property var undoHook: null
    property var redoHook: null
    // 正在编辑时把快捷键让路（M/L/Q 不能被吞成工具切换）。
    // editingChanged 由属性自动生成，不用再声明。
    property bool editing: input.activeFocus
    signal edited(int v)

    implicitWidth: 74
    height: 22
    radius: 3
    color: "#1b1d20"
    border.color: input.activeFocus ? "#5b9dff" : "#45484d"
    border.width: 1

    onValueChanged: {
        if (!input.activeFocus)
            input.text = String(value)
    }
    Component.onCompleted: input.text = String(value)

    // 只提交，不动焦点（↑↓ 键连续微调时还要继续能打字）。
    function commit(raw) {
        let v = parseInt(raw, 10)
        if (isNaN(v))
            v = sb.value
        v = Math.max(sb.from, Math.min(sb.to, v))
        // 只在需要时改写文本：每敲一个字都重设 text 会把光标顶到末尾。
        if (input.text !== String(v))
            input.text = String(v)
        if (v !== sb.value) {
            // ★先把 value 同步过去再发信号：宿主（变换数值面板）是在
            //   onEdited 里读 `px.value` 的，只发信号不改 value 的话，宿主拿到的
            //   永远是上一版的旧数值 —— 表现就是「改数值完全没反应」。
            sb.value = v
            sb.edited(v)
        }
    }

    // 提交并交出键盘。★ 必须交：TextInput 拿着焦点时 Ctrl+Z 会被它自己的
    // 「文字撤销」吃掉、Delete/Backspace 也只是删字符，用户看到的就是
    // 「输入完数值以后 Del / 撤销全没反应」（ResizeOverlay 踩过同一个坑）。
    function finish(raw) {
        sb.commit(raw)
        if (input.activeFocus)
            input.focus = false
    }

    // 外部关菜单 / Esc：把框里的值提交掉再交焦点。
    // 这里绝不能「丢弃输入」—— 用户点一下菜单外面菜单就关了，丢掉等于
    // 白敲（这正是"输入数值没效果、只有箭头有效"的来源）。
    function endEdit() {
        sb.commit(input.text)
        if (input.activeFocus)
            input.focus = false
    }

    // 当前值：编辑中优先用框里已经敲的内容，箭头微调才不会把它顶掉。
    function currentValue() {
        let v = parseInt(input.text, 10)
        if (isNaN(v))
            v = sb.value
        return Math.max(sb.from, Math.min(sb.to, v))
    }

    // 边打边生效（防抖 260ms）：数值框是即时预览的，等回车才应用的话，
    // 用户点一下别处就白输了；连续敲三位数也只会重算一次。
    Timer {
        id: typeDebounce
        interval: 260
        repeat: false
        onTriggered: sb.commit(input.text)
    }

    TextInput {
        id: input
        anchors.left: parent.left
        anchors.leftMargin: 6
        anchors.verticalCenter: parent.verticalCenter
        width: parent.width - 24
        text: String(sb.value)
        color: "#e9eaec"
        font.pixelSize: 12
        selectByMouse: true
        horizontalAlignment: TextInput.AlignLeft
        validator: IntValidator { bottom: sb.from; top: sb.to }
        // 输入框抢焦点时禁用单字母快捷键（父级把 editing 汇总到
        // EditorContent.selFieldEditing）。editing 是 activeFocus 的绑定，
        // editingChanged 会自动发。
        onActiveFocusChanged: {
            if (!activeFocus)
                sb.commit(text)
        }
        onAccepted: sb.finish(text)
        // 用户每敲一个字都重新计时，停下来 260ms 才真正算一次。
        onTextEdited: typeDebounce.restart()
        Keys.onUpPressed: { sb.commit(String(sb.currentValue() + sb.step)); event.accepted = true }
        Keys.onDownPressed: { sb.commit(String(sb.currentValue() - sb.step)); event.accepted = true }
        Keys.onPressed: (event) => {
            if ((event.modifiers & Qt.ControlModifier) === 0)
                return
            const shift = (event.modifiers & Qt.ShiftModifier) !== 0
            if (event.key === Qt.Key_Z && !shift && sb.undoHook) {
                sb.endEdit()
                sb.undoHook()
                event.accepted = true
            } else if ((event.key === Qt.Key_Y || (event.key === Qt.Key_Z && shift))
                       && sb.redoHook) {
                sb.endEdit()
                sb.redoHook()
                event.accepted = true
            }
        }
    }

    // 上下箭头（右边一列，各占一半高）。
    Column {
        anchors.right: parent.right
        anchors.rightMargin: 2
        anchors.verticalCenter: parent.verticalCenter
        spacing: 1

        Rectangle {
            width: 16
            height: 9
            radius: 2
            color: up.containsMouse ? "#4a4e54" : "transparent"
            Text {
                anchors.centerIn: parent
                text: "▲"
                color: "#c9cdd2"
                font.pixelSize: 6
            }
            MouseArea {
                id: up
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: sb.finish(String(sb.currentValue() + sb.step))
            }
        }
        Rectangle {
            width: 16
            height: 9
            radius: 2
            color: dn.containsMouse ? "#4a4e54" : "transparent"
            Text {
                anchors.centerIn: parent
                text: "▼"
                color: "#c9cdd2"
                font.pixelSize: 6
            }
            MouseArea {
                id: dn
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: sb.finish(String(sb.currentValue() - sb.step))
            }
        }
    }
}
