import QtQuick

// Text row inside the overflow "…" menu.
Rectangle {
    id: mr

    property string label
    property bool active: true
    property bool separator: false
    // 有二级菜单：右侧画一个 ▸，并且被展开时保持高亮。
    property bool hasSub: false
    property bool expanded: false
    signal activated()

    width: 218
    height: separator ? 9 : 26
    radius: 4
    color: separator ? "transparent"
           : ((mh.containsMouse || mr.expanded) && active ? "#3d5a80" : "transparent")
    opacity: active ? 1 : 0.35
    readonly property bool hovered: mh.containsMouse

    Rectangle {
        visible: mr.separator
        anchors.centerIn: parent
        width: parent.width - 16
        height: 1
        color: "#3c4043"
    }
    Text {
        visible: !mr.separator
        x: 10
        anchors.verticalCenter: parent.verticalCenter
        text: mr.label
        color: "#e9eaec"
        font.pixelSize: 12
    }
    // 二级菜单指示：菜单靠右边缘，子面板向左展开，所以箭头指向左侧。
    Text {
        visible: mr.hasSub && !mr.separator
        anchors.right: parent.right
        anchors.rightMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        text: "‹"
        color: mr.expanded ? "#cfe4ff" : "#8b9096"
        font.pixelSize: 13
    }
    MouseArea {
        id: mh
        anchors.fill: parent
        hoverEnabled: !mr.separator
        enabled: !mr.separator
        cursorShape: Qt.PointingHandCursor
        onClicked: if (mr.active) mr.activated()
    }
}