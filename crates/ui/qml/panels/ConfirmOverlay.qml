import QtQuick
import com.imageedit

Item {
    id: confirmOverlay
    required property var host
    anchors.fill: parent
    visible: false
    z: 200

    MouseArea { anchors.fill: parent }

    // Native-dialog-style window: title bar with close button, body with
    // warning icon + message, footer with right-aligned actions.
    Rectangle {
        width: 420
        height: host.confirmMode === "psd-lossy" ? 244 : 190
        anchors.centerIn: parent
        radius: 8
        color: "#25272b"
        border.color: "#4a4e54"
        clip: true

        // Title bar
        Item {
            id: confirmTitle
            width: parent.width
            height: 38

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                text: host.confirmMode === "psd-lossy"
                      ? "覆盖保存会丢失 PSD 结构"
                      : "未保存的更改"
                color: "#e9eaec"
                font.pixelSize: 13
                font.bold: true
            }

            IconButton {
                anchors.right: parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                glyph: "✕"
                tip: "关闭"
                onClicked: host.confirmCancel()
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: "#3c4043"
            }
        }

        // Body: warning glyph + message
        Item {
            anchors.top: confirmTitle.bottom
            anchors.bottom: confirmFooter.top
            anchors.left: parent.left
            anchors.right: parent.right

            Text {
                id: warnGlyph
                anchors.left: parent.left
                anchors.leftMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                text: "⚠"
                color: "#e8b339"
                font.pixelSize: 26
            }

            Text {
                anchors.left: warnGlyph.right
                anchors.leftMargin: 14
                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                text: host.confirmMode === "open"
                      ? "打开新文件将放弃当前图片的未保存修改。"
                      : host.confirmMode === "nav"
                      ? "切换到其他图片将放弃当前图片的未保存修改。"
                      : host.confirmMode === "psd-lossy"
                      ? "这份 PSD 装载时有图层按降级处理（组结构、蒙版、混合模式或越界像素）。覆盖保存只会写回普通位图图层，原文件里的那些结构将永久丢失。\n建议先「另存为」新文件，确认无误后再替换原文件。"
                      : "关闭窗口将放弃当前图片的未保存修改。"
                color: "#c9cdd2"
                font.pixelSize: 13
                wrapMode: Text.WordWrap
            }
        }

        // Footer: right-aligned action buttons
        Item {
            id: confirmFooter
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 58

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: 1
                color: "#3c4043"
            }

            Row {
                anchors.right: parent.right
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                visible: host.confirmMode !== "psd-lossy"

                ToolButton {
                    label: "取消"
                    active: true
                    onClicked: host.confirmCancel()
                }
                // Default action: highlighted, triggered by Enter. Saves
                // first, then continues the pending open/nav/close.
                ToolButton {
                    label: host.confirmMode === "open" ? "保存并打开"
                         : host.confirmMode === "nav" ? "保存并切换"
                         : "保存并关闭"
                    active: true
                    selected: true
                    onClicked: host.confirmSaveProceed()
                }
                ToolButton {
                    label: host.confirmMode === "open" ? "放弃并打开"
                         : host.confirmMode === "nav" ? "放弃并切换"
                         : "放弃并关闭"
                    active: true
                    onClicked: host.confirmProceed()
                }
            }

            // 覆盖写回会丢结构：只有「另存为」和「仍然覆盖」两个出口，
            // 默认高亮给「另存为」，避免顺手一个回车把结构烧掉。
            Row {
                anchors.right: parent.right
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                visible: host.confirmMode === "psd-lossy"

                ToolButton {
                    label: "取消"
                    active: true
                    onClicked: host.confirmCancel()
                }
                ToolButton {
                    label: "仍然覆盖"
                    active: true
                    onClicked: host.confirmLossyOverwrite()
                }
                ToolButton {
                    label: "另存为…"
                    active: true
                    selected: true
                    onClicked: host.confirmLossySaveAs()
                }
            }
        }
    }
}
