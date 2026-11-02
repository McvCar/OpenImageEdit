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
    // 先不显示：EditorContent 已经把文件头量好了尺寸，等 presizeWindow()
    // 把窗口摆成最终大小/位置再 show()，避免"空窗口闪一下再跳"。
    // ★必须用 visibility 而不是 visible：赋 Windowed 会让 QQuickWindow 顺手
    // setVisible(true)，窗口会抢在 Component.onCompleted 之前冒出来。
    property bool shown: false
    visibility: shown ? (content.fullscreen ? Window.FullScreen : Window.Windowed)
                      : Window.Hidden
    // Frameless: same custom title bar as the shell window.
    flags: Qt.Window | Qt.FramelessWindowHint
    // 任务栏/系统标题用文件名，不再挂分辨率（分辨率在状态栏）。
    title: content.b.fileName.length > 0
           ? content.b.fileName + " — ImageEdit" + (content.b.dirty ? " *" : "")
           : "ImageEdit" + (content.b.dirty ? " *" : "")

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
            // 画布上的 +/−/剪刀光标（Win32 自定义光标，见 ops/cursor.rs）。
            content.b.cursor_install()
        }
    }
    Timer {
        id: frontTimer
        interval: 300
        onTriggered: {
            docWin.flags = Qt.Window | Qt.FramelessWindowHint
            // ★改 flags 会重建原生窗口，旧 HWND 上的 WndProc 子类化随之
            // 作废 —— 必须在新窗口上重新挂一次，否则剪刀光标在这个窗口里
            // 永远不出现。
            content.b.cursor_install()
        }
    }
    onClosing: (close) => content.requestClose(close)

    Component.onCompleted: {
        // 子项先完成（EditorContent 已经在解码），这里再按文件头量出的尺寸
        // 把窗口摆好，然后才显示。
        content.hostWindow = docWin
        content.presizeWindow(docWin, docWin.fileUrl)
        docWin.shown = true
    }

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