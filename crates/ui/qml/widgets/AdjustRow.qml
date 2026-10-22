import QtQuick

// One "调整颜色" slider row: label + bipolar/unipolar track + readout +
// per-row reset, mirroring the macOS Preview panel. `minVal`/`maxVal`
// drive both the knob math and the readout sign.
Item {
    id: arow

    property string label: ""
    property string glyph: ""
    property string key: ""
    property real value: 0
    property real minVal: -1
    property real maxVal: 1
    // 0..1 position of the knob; while dragging the local override wins so
    // the knob never fights the throttled round-trip through Rust.
    property bool dragging: false
    property real dragNorm: 0
    readonly property real norm: {
        const span = arow.maxVal - arow.minVal
        return span > 0 ? Math.max(0, Math.min(1, (arow.value - arow.minVal) / span)) : 0
    }
    readonly property real knobPos: arow.dragging ? arow.dragNorm : arow.norm
    // Bipolar readouts show a signed percentage, unipolar a plain one.
    readonly property string readout: {
        const v = arow.dragging
            ? arow.minVal + arow.dragNorm * (arow.maxVal - arow.minVal)
            : arow.value
        if (arow.minVal < 0)
            return (v > 0 ? "+" : "") + Math.round(v * 100)
        return Math.round(v * 100)
    }
    readonly property bool neutral: Math.abs(arow.value) < 0.005

    signal moved(real value)
    signal resetRequested()

    width: parent.width
    height: 22

    Text {
        id: arowIcon
        x: 2
        width: 14
        anchors.verticalCenter: parent.verticalCenter
        text: arow.glyph
        color: "#9aa3b2"
        font.pixelSize: 11
        horizontalAlignment: Text.AlignHCenter
    }
    Text {
        id: arowLabel
        x: 20
        width: 52
        anchors.verticalCenter: parent.verticalCenter
        text: arow.label
        color: "#c9ccd1"
        font.pixelSize: 11
        elide: Text.ElideRight
    }

    Item {
        id: arowTrack
        anchors.left: arowLabel.right
        anchors.leftMargin: 4
        anchors.right: arowValue.left
        anchors.rightMargin: 6
        anchors.verticalCenter: parent.verticalCenter
        height: 18

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: parent.width
            height: 3
            radius: 2
            color: "#3c4043"
        }

        // Filled part: from the neutral point (bipolar) or the left end.
        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            x: arow.minVal < 0
               ? arowTrack.width * Math.min(0.5, arow.knobPos)
               : 0
            width: arow.minVal < 0
                   ? Math.abs(arow.knobPos - 0.5) * arowTrack.width
                   : arow.knobPos * arowTrack.width
            height: 3
            radius: 2
            color: "#5b9dff"
        }

        // Neutral tick for bipolar rows.
        Rectangle {
            visible: arow.minVal < 0
            anchors.verticalCenter: parent.verticalCenter
            x: arowTrack.width * 0.5 - 0.5
            width: 1
            height: 9
            color: "#5a6069"
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            x: Math.max(0, Math.min(arowTrack.width - 10,
                                   arow.knobPos * arowTrack.width - 5))
            width: 10
            height: 10
            radius: 5
            color: arowMa.containsMouse || arow.dragging ? "#7fc3ff" : "#e9eaec"
        }

        MouseArea {
            id: arowMa
            anchors.fill: parent
            anchors.margins: -6
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor

            function applyAt(x) {
                const t = Math.max(0, Math.min(1, (x + 6) / arowTrack.width))
                arow.dragging = true
                arow.dragNorm = t
                arow.moved(arow.minVal + t * (arow.maxVal - arow.minVal))
            }
            onPressed: (mouse) => applyAt(mouse.x)
            onPositionChanged: (mouse) => { if (pressed) applyAt(mouse.x) }
            onReleased: arow.dragging = false
            onCanceled: arow.dragging = false
            onDoubleClicked: arow.resetRequested()
        }
    }

    Text {
        id: arowValue
        anchors.right: arowReset.left
        anchors.rightMargin: 5
        anchors.verticalCenter: parent.verticalCenter
        width: 30
        text: arow.readout
        color: arow.neutral ? "#7c828b" : "#e9eaec"
        font.pixelSize: 10
        horizontalAlignment: Text.AlignRight
    }

    // Per-row reset (the circular arrow at the right of every row).
    Rectangle {
        id: arowReset
        anchors.right: parent.right
        anchors.rightMargin: 2
        anchors.verticalCenter: parent.verticalCenter
        width: 16
        height: 16
        radius: 8
        color: arowResetMa.containsMouse && !arow.neutral ? "#3d5a80" : "transparent"
        Text {
            anchors.centerIn: parent
            text: "↺"
            color: arow.neutral ? "#5a6069" : "#b8bcc0"
            font.pixelSize: 11
        }
        MouseArea {
            id: arowResetMa
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: arow.resetRequested()
        }
    }
}