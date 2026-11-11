import QtQuick
import com.imageedit

// PSD information overlay (M5b).
//
// It is INFORMATION ONLY: saving is never intercepted here. It appears on a
// degraded layered load, a skipped / failed stage 2, or a downsampled
// read-only open. The Rust side publishes `bridge.psdNotice`; EditorContent
// forwards it to showNotice(). "另存为…" is a convenience shortcut for the
// cases where writing back to the source PSD is intentionally blocked.
Item {
    id: psdHintOverlay
    required property var host
    anchors.fill: parent
    visible: false
    z: 200

    property string noticeText: ""
    property string noticeTitle: "PSD 分层说明"

    function showNotice(text, title) {
        noticeText = text
        noticeTitle = title || "PSD 分层说明"
        visible = true
    }

    function dismiss() {
        visible = false
        // Let a future notice with the same text trigger onPsdNoticeChanged.
        if (typeof bridge !== "undefined" && bridge)
            bridge.clear_psd_notice()
    }

    MouseArea { anchors.fill: parent }

    Rectangle {
        width: 440
        height: 210
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
                text: psdHintOverlay.noticeTitle
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
                onClicked: psdHintOverlay.dismiss()
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
                text: psdHintOverlay.noticeText
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
                    label: "知道了"
                    active: true
                    onClicked: psdHintOverlay.dismiss()
                }
                ToolButton {
                    label: "另存为…"
                    active: true
                    selected: true
                    onClicked: {
                        psdHintOverlay.dismiss()
                        saveDialog.open()
                    }
                }
            }
        }
    }
}
