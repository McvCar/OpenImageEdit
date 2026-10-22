import QtQuick

// Text row inside the overflow "…" menu.
Rectangle {
    id: mr

    property string label
    property bool active: true
    property bool separator: false
    signal activated()

    width: 218
    height: separator ? 9 : 26
    radius: 4
    color: separator ? "transparent"
           : (mh.containsMouse && active ? "#3d5a80" : "transparent")
    opacity: active ? 1 : 0.35

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
    MouseArea {
        id: mh
        anchors.fill: parent
        hoverEnabled: !mr.separator
        enabled: !mr.separator
        cursorShape: Qt.PointingHandCursor
        onClicked: if (mr.active) mr.activated()
    }
}