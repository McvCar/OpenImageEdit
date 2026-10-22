import QtQuick

// Title-bar icon button (no QtQuick.Controls dependency: the tooltip is
// hand-rolled). `separator` renders a slim divider instead of a button.
Rectangle {
    id: ib

    property string glyph
    property string tip
    property bool active: true
    property bool selected: false
    property bool separator: false
    signal clicked()

    width: separator ? 9 : 30
    height: 28
    radius: 4
    color: separator ? "transparent"
           : (selected ? "#3d5a80"
              : (hover.containsMouse && active ? "#4a4e54" : "transparent"))
    opacity: active ? 1.0 : 0.35

    Rectangle {
        visible: ib.separator
        anchors.centerIn: parent
        width: 1
        height: 18
        color: "#3c4043"
    }
    Text {
        visible: !ib.separator
        anchors.centerIn: parent
        text: ib.glyph
        color: "#e9eaec"
        font.pixelSize: 14
    }

    Rectangle {
        id: tipBox
        visible: false
        z: 1000
        x: Math.max(-ib.x, (ib.width - width) / 2)
        y: ib.height + 5
        width: tipText.implicitWidth + 16
        height: 22
        radius: 4
        color: "#111214"
        border.color: "#3c4043"
        Text {
            id: tipText
            anchors.centerIn: parent
            text: ib.tip
            color: "#d7dade"
            font.pixelSize: 11
        }
    }
    Timer {
        id: tipDelay
        interval: 500
        onTriggered: tipBox.visible = true
    }

    MouseArea {
        id: hover
        anchors.fill: parent
        hoverEnabled: !ib.separator
        enabled: !ib.separator
        cursorShape: Qt.PointingHandCursor
        onContainsMouseChanged: {
            if (!containsMouse) {
                tipBox.visible = false
                tipDelay.stop()
            } else if (ib.active && ib.tip.length > 0) {
                tipDelay.restart()
            }
        }
        onClicked: if (ib.active) ib.clicked()
    }
}