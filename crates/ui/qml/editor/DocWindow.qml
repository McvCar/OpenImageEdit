import QtQuick
import QtQuick.Window
import com.imageedit

// A secondary document window: same editor content, its own Bridge
// instance, no CLI handling (that stays with the primary window).
// newWindowFn is injected by the shell factory so the inline component
// never needs to reference the shell (which would create a cycle).
Window {
    id: docWin

    property var newWindowFn: null
    // File to open on first show. The shell factory passes it through when a
    // second image is opened in its own window ("在新窗口打开" setting, a
    // dropped file, ...). Without this property Qt rejected the initial
    // property assignment and the new window stayed BLANK, which also made
    // "copy a layer in window A, paste in window B" look broken.
    property url fileUrl: ""

    width: 1100
    height: 760
    minimumWidth: 480
    minimumHeight: 320
    color: "#1b1c1e"
    visible: true
    // Frameless: same custom title bar as the shell window.
    flags: Qt.Window | Qt.FramelessWindowHint
    visibility: content.fullscreen ? Window.FullScreen : Window.Windowed
    title: content.b.image_width > 0
           ? "ImageEdit — " + content.b.image_width + "×" + content.b.image_height
             + (content.b.dirty ? " *" : "")
           : "ImageEdit"

    onAfterRendering: content.afterFrame()
    onVisibleChanged: {
        content.b.mark("window-visible")
        // Frameless windows created via createObject don't take focus on
        // their own. raise()+requestActivate() alone only works while our
        // process already owns the foreground (menu-triggered opens);
        // Windows denies focus stealing from a background process, so a
        // file-manager double-click opened the window BEHIND other apps.
        // Temporarily pinning above-all re-creates the native window with
        // topmost z-order — no foreground permission needed. The pin is
        // dropped again right after so the window is not always-on-top.
        if (visible) {
            flags = Qt.Window | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint
            raise()
            requestActivate()
            frontTimer.restart()
        }
    }
    Timer {
        id: frontTimer
        interval: 300
        onTriggered: docWin.flags = Qt.Window | Qt.FramelessWindowHint
    }
    onClosing: (close) => content.requestClose(close)

    WindowEdges {}

    EditorContent {
        id: content
        anchors.fill: parent
        primary: false
        fileUrl: docWin.fileUrl
        // 新窗口按图片大小定位（内容自己做尺寸钳制/屏幕内收边）。
        onWindowFitRequested: content.fitWindowToImage(docWin)
        onRequestNewWindow: (fileUrl) => {
            if (docWin.newWindowFn)
                docWin.newWindowFn(fileUrl)
        }
    }
}