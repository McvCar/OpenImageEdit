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
    // 弹窗位置：初始居中，之后允许拖标题栏移动。用 x/y 而不是
    // anchors.centerIn，才能在 onCompleted 里摆一次中心、再由鼠标改。
    property bool positioned: false

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
        id: noticeCard
        width: Math.min(520, parent.width - 40)
        // 高度随正文行数增长（长说明不再被截断），上限 80% 窗口高，
        // 超出部分正文区自己可滚。
        height: Math.min(parent.height * 0.8,
                         psdHintTitle.height + bodyText.implicitHeight + 44
                         + psdHintFooter.height)
        x: 0
        y: 0
        Component.onCompleted: centerCard()
        function centerCard() {
            x = Math.max(0, (parent.width - width) / 2)
            y = Math.max(0, (parent.height - height) / 2)
            positioned = true
        }
        radius: 8
        color: "#25272b"
        border.color: "#4a4e54"
        clip: true

        // 标题栏拖动：改弹窗 x/y，不动内容布局。
        MouseArea {
            anchors.left: psdHintTitle.left
            anchors.top: psdHintTitle.top
            anchors.bottom: psdHintTitle.bottom
            anchors.right: psdHintTitle.right
            anchors.rightMargin: 44   // 让出右上角关闭按钮
            z: 1
            property real dragX: 0
            property real dragY: 0
            onPressed: {
                dragX = mouse.x
                dragY = mouse.y
            }
            onPositionChanged: {
                noticeCard.x = Math.max(0, Math.min(parent.width - noticeCard.width,
                                                    noticeCard.x + mouse.x - dragX))
                noticeCard.y = Math.max(0, Math.min(parent.height - noticeCard.height,
                                                    noticeCard.y + mouse.y - dragY))
            }
        }

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

            Flickable {
                anchors.fill: parent
                anchors.margins: 4
                contentWidth: width
                contentHeight: bodyText.implicitHeight
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                Text {
                    id: bodyText
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    anchors.right: parent.right
                    anchors.rightMargin: 14
                    anchors.top: parent.top
                    anchors.topMargin: 10
                    text: psdHintOverlay.noticeText
                    color: "#c9cdd2"
                    font.pixelSize: 13
                    wrapMode: Text.WordWrap
                }
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
