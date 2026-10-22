import QtQuick

// Checkbox row with a real checkbox visual: filled blue square + check
// mark when on, empty dark square when off, hover highlight on both.
Item {
    id: ck

    property string label
    property bool checked: false
    property bool active: true
    signal toggled()

    width: 14 + 8 + ckLabel.implicitWidth
    height: 18

    Rectangle {
        id: box
        x: 0
        anchors.verticalCenter: parent.verticalCenter
        width: 14
        height: 14
        radius: 3
        color: ck.checked ? "#3d5a80" : (ckHover.containsMouse && ck.active ? "#33373c" : "#1b1c1e")
        border.color: ck.checked ? "#6aa9ff"
                      : (ckHover.containsMouse && ck.active ? "#8f96a3" : "#5a6069")
        border.width: ck.checked ? 2 : 1
        opacity: ck.active ? 1.0 : 0.35

        Text {
            anchors.centerIn: parent
            visible: ck.checked
            text: "✓"
            color: "#e9eaec"
            font.pixelSize: 11
            font.bold: true
        }
    }
    Text {
        id: ckLabel
        x: 22
        anchors.verticalCenter: parent.verticalCenter
        text: ck.label
        color: ck.checked ? "#e9eaec" : "#b8bcc0"
        font.pixelSize: 12
        opacity: ck.active ? 1.0 : 0.35
    }
    MouseArea {
        id: ckHover
        anchors.fill: parent
        hoverEnabled: true
        enabled: ck.active
        cursorShape: Qt.PointingHandCursor
        onClicked: ck.toggled()
    }
}