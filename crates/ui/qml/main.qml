import QtQuick
import QtQuick.Window
import QtQuick.Dialogs
import com.imageedit

Window {
    id: shell

    width: 1100
    height: 760
    minimumWidth: 480
    minimumHeight: 320
    visible: Qt.application.arguments.indexOf("--hidden") === -1
    color: "#1b1c1e"
    // Frameless: the custom title bar (in EditorContent) provides the drag
    // area, window buttons and resize edges.
    flags: Qt.Window | Qt.FramelessWindowHint
    visibility: content.fullscreen ? Window.FullScreen : Window.Windowed
    title: content.b.image_width > 0
           ? "ImageEdit — " + content.b.image_width + "×" + content.b.image_height
             + (content.b.dirty ? " *" : "")
           : "ImageEdit"

    onAfterRendering: content.afterFrame()
    onVisibleChanged: content.b.mark("window-visible")
    onClosing: (close) => content.requestClose(close)

    WindowEdges {}

    EditorContent {
        id: content
        anchors.fill: parent
        primary: true
        // 每次装上新的图片，把窗口调成刚好装下它（内容自己判断尺寸/钳制）。
        onWindowFitRequested: content.fitWindowToImage(shell)
        onRequestNewWindow: (fileUrl) => openNewWindow(fileUrl)
    }

    property int windowCascade: 0
    // Strong references to every live secondary document window. createObject
    // with a null parent transfers ownership to the JS engine, so without
    // this array the GC may collect a visible window — it would just close
    // itself after a while (the "new window randomly disappears" bug).
    property var docWindows: []

    function openNewWindow(fileUrl) {
        const offset = shell.windowCascade * 32
        shell.windowCascade = (shell.windowCascade + 1) % 8
        const win = docWindowFactory.createObject(null, {
            x: 80 + offset, y: 60 + offset,
            newWindowFn: openNewWindow,
            fileUrl: fileUrl !== undefined ? fileUrl : ""
        })
        if (!win)
            return
        shell.docWindows.push(win)
        // DocWindow's own onClosing runs first (connected at creation), so
        // `accepted` already reflects the dirty-guard decision here: only
        // forget + destroy a window the user was really allowed to close.
        win.closing.connect(function (closeEvent) {
            if (!closeEvent.accepted)
                return
            const i = shell.docWindows.indexOf(win)
            if (i >= 0)
                shell.docWindows.splice(i, 1)
            // Deferred delete: safe during close-event processing.
            win.destroy()
        })
    }

    Component {
        id: docWindowFactory
        DocWindow {}
    }

}
