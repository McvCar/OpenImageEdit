import QtQuick
import QtQuick.Window

// Single resize strip for the frameless windows (native system resize).
MouseArea {
    property int edges: 0
    property int shape: Qt.SizeAllCursor
    cursorShape: shape
    onPressed: (mouse) => {
        const w = Window.window
        if (w && w.startSystemResize)
            w.startSystemResize(edges)
    }
}