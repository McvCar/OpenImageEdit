import QtQuick

Rectangle {
    id: button

    property string label
    property bool active: true
    property bool selected: false
    signal clicked()

    width: Math.max(52, caption.implicitWidth + 16)
    height: 26
    radius: 4
    color: button.selected
           ? "#3d5a80"
           : (mouse.containsMouse && button.active ? "#4a4e54" : "#33373c")
    opacity: button.active ? 1.0 : 0.35

    Text {
        id: caption
        anchors.centerIn: parent
        text: button.label
        color: "#e9eaec"
        font.pixelSize: 12
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: if (button.active) button.clicked()
    }
}