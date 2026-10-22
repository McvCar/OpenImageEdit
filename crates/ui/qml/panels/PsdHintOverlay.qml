import QtQuick
import com.imageedit

// PSD is preview-only: Ctrl+S / 保存 on a .psd document opens this
// dialog instead of writing (a direct save would corrupt the file —
// image has no PSD encoder). Routes to the Save-As dialog.
Item {
    id: psdHintOverlay
    required property var host
    anchors.fill: parent
    visible: false
    z: 200

    MouseArea { anchors.fill: parent }

    Rectangle {
        width: 420
        height: 190
        anchors.centerIn: parent
        radius: 8
        color: "#25272b"
        border.color: "#4a4e54"
        clip: true

        Item {
            id: psdHintTitle
            width: parent.width
            height: 38

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                text: "PSD 不能直接保存"
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
                onClicked: psdHintOverlay.visible = false
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: "#3c4043"
            }
        }

        Item {
            anchors.top: psdHintTitle.bottom
            anchors.bottom: psdHintFooter.top
            anchors.left: parent.left
            anchors.right: parent.right

            Text {
                anchors.left: parent.left
                anchors.leftMargin: 18
                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                text: "PSD 是预览专用格式（打开时图层已合并），直接保存会损坏原文件。\n请使用「另存为」导出 PNG / JPEG 等格式。"
                color: "#c9cdd2"
                font.pixelSize: 13
                wrapMode: Text.WordWrap
            }
        }

        Item {
            id: psdHintFooter
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

                ToolButton {
                    label: "取消"
                    active: true
                    onClicked: psdHintOverlay.visible = false
                }
                ToolButton {
                    label: "另存为…"
                    active: true
                    selected: true
                    onClicked: {
                        psdHintOverlay.visible = false
                        saveDialog.open()
                    }
                }
            }
        }
    }
}
