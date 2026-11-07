import QtQuick

// 8 resize handles (4 corners + 4 edge midpoints) for the crop / selection
// boxes. Geometry callbacks live in the enclosing scope and are injected
// at the instantiation site; all coordinates snap to image pixels.
Item {
    id: handles

    property real bx: 0
    property real by: 0
    property real bw: 0
    property real bh: 0
    property real pxScale: 1
    property var resizeFn: null
    property var moveFn: null
    property bool moveEnabled: true
    // True while any handle (or the center pad) is being dragged.
    property bool engaged: false

    Repeater {
        model: ["nw", "n", "ne", "e", "se", "s", "sw", "w"]

        delegate: Rectangle {
            readonly property string role: modelData
            readonly property bool corner: role.length === 2
            // Blue-dot handle (selected state marker), white-ringed; the
            // hit area extends past the visible dot for easy grabbing.
            width: hma.pressed ? 12 : 10
            height: width
            radius: width / 2
            color: hma.pressed ? "#7fc3ff" : "#5b9dff"
            border.color: "#ffffff"
            border.width: 1.5
            z: 2

            x: handles.bx
               + (role.indexOf("e") >= 0 ? handles.bw
                  : (role === "n" || role === "s" ? handles.bw / 2 : 0))
               - width / 2
            y: handles.by
               + (role.indexOf("s") >= 0 ? handles.bh
                  : (role === "w" || role === "e" ? handles.bh / 2 : 0))
               - height / 2

            MouseArea {
                id: hma
                anchors.fill: parent
                anchors.margins: -4
                // hoverEnabled OFF + cursorShape undefined: hover and
                // cursor decisions both belong to the canvas HoverHandler;
                // this area only handles the press for resizing.
                cursorShape: undefined
                property rect startRect
                property real px: 0
                property real py: 0

                // Deltas MUST be measured in a stable coordinate frame:
                // the handle itself moves while dragging, so its local
                // mouse coords feed back into resizeFn and the box
                // oscillates (visible jitter). Map to the handles item,
                // which never moves during a drag.
                onPressed: (mouse) => {
                    const p = mapToItem(handles, mouse.x, mouse.y)
                    px = p.x
                    py = p.y
                    startRect = Qt.rect(handles.bx, handles.by, handles.bw, handles.bh)
                    handles.engaged = true
                }
                onPositionChanged: (mouse) => {
                    if (pressed && handles.resizeFn) {
                        const p = mapToItem(handles, mouse.x, mouse.y)
                        // P1-A：完整修饰键位掩码原样透传，语义在 Rust 解出
                        // （Shift 锁比例 / Alt 从中心 / Ctrl 斜切）。
                        handles.resizeFn(startRect, role, p.x - px, p.y - py,
                                         mouse.modifiers)
                    }
                }
                onReleased: handles.engaged = false
            }
        }
    }

    MouseArea {
        id: centerMa
        anchors.fill: parent
        hoverEnabled: false
        // undefined: without it this area's default ArrowCursor shadows
        // the canvas HoverHandler across the whole box interior.
        cursorShape: undefined
        enabled: handles.moveEnabled && handles.moveFn !== null
                 && handles.bw > 0 && handles.bh > 0
        property rect startRect
        property real px: 0
        property real py: 0

        // Only grab presses that land INSIDE the box; a press on empty
        // canvas is rejected so it falls through to the rubber-band area
        // below. NOTE: gating `enabled` on hover coordinates (the old
        // approach) deadlocks — a disabled MouseArea stops receiving
        // hover, so its mouseX/mouseY freeze and `enabled` can never
        // become true again.
        onPressed: (mouse) => {
            if (mouse.x <= handles.bx || mouse.x >= handles.bx + handles.bw
                    || mouse.y <= handles.by || mouse.y >= handles.by + handles.bh) {
                mouse.accepted = false
                return
            }
            px = mouse.x
            py = mouse.y
            startRect = Qt.rect(handles.bx, handles.by, handles.bw, handles.bh)
            handles.engaged = true
        }
        onPositionChanged: (mouse) => {
            if (pressed && handles.moveFn)
                handles.moveFn(startRect, mouse.x - px, mouse.y - py)
        }
        onReleased: handles.engaged = false
    }

    // Four edge strips: the whole dashed border is grabbable, not only
    // the dots. Each strip straddles the box edge (half in, half out),
    // shows the matching resize cursor on hover and feeds resizeFn with
    // the same role strings the dot handles use. z:1 keeps them above
    // the center move pad (z:0) but below the dot handles (z:2), so
    // corners keep their diagonal cursors.
    Repeater {
        model: [
            { role: "n", shape: Qt.SizeVerCursor },
            { role: "s", shape: Qt.SizeVerCursor },
            { role: "w", shape: Qt.SizeHorCursor },
            { role: "e", shape: Qt.SizeHorCursor }
        ]

        delegate: MouseArea {
            required property var modelData
            readonly property string role: modelData.role
            // hoverEnabled OFF — hover must reach the canvas HoverHandler
            // (the single cursor authority); presses land here.
            // cursorShape undefined — default Arrow would shadow it.
            cursorShape: undefined
            enabled: handles.resizeFn !== null && handles.bw > 0 && handles.bh > 0
            z: 1
            x: role === "e" ? handles.bx + handles.bw - 4
                            : handles.bx - 4
            y: role === "s" ? handles.by + handles.bh - 4
                            : handles.by - 4
            width: role === "w" || role === "e" ? 8 : handles.bw + 8
            height: role === "n" || role === "s" ? 8 : handles.bh + 8

            property rect startRect
            property real px: 0
            property real py: 0

            // Deltas MUST be measured in a stable coordinate frame — the
            // SAME rule as the dot handles above: the strip itself moves
            // with the box while dragging, so its local mouse coords feed
            // back into resizeFn and the edge oscillates (visible jitter).
            // Map to the handles item, which never moves during a drag.
            onPressed: (mouse) => {
                const p = mapToItem(handles, mouse.x, mouse.y)
                px = p.x
                py = p.y
                startRect = Qt.rect(handles.bx, handles.by, handles.bw, handles.bh)
                handles.engaged = true
            }
            onPositionChanged: (mouse) => {
                if (pressed && handles.resizeFn) {
                    const p = mapToItem(handles, mouse.x, mouse.y)
                    // P1-A：完整修饰键位掩码（同上，语义在 Rust 解出）。
                    handles.resizeFn(startRect, role, p.x - px, p.y - py,
                                     mouse.modifiers)
                }
            }
            onReleased: handles.engaged = false
        }
    }
}