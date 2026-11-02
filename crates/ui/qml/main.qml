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
    // 先不显示：等 Component.onCompleted 里按待打开的图片把尺寸/位置摆好再
    // show()。否则会先弹一个 1100×760 的空窗口，解码完再跳到图片大小 ——
    // 那一跳就是"打开大图时画面闪一下"的来源。
    // ★只能走 visibility：一旦给 visibility 赋 Windowed，QQuickWindow 内部
    // 会顺手 setVisible(true)，单写 `visible: false` 顶不住（实测窗口在
    // Component.onCompleted 之前就冒出来了）。
    property bool shown: false
    visibility: shown ? (content.fullscreen ? Window.FullScreen : Window.Windowed)
                      : Window.Hidden
    color: "#1b1c1e"
    // Frameless: the custom title bar (in EditorContent) provides the drag
    // area, window buttons and resize edges.
    flags: Qt.Window | Qt.FramelessWindowHint
    title: content.b.image_width > 0
           ? "ImageEdit — " + content.b.image_width + "×" + content.b.image_height
             + (content.b.dirty ? " *" : "")
           : "ImageEdit"

    onAfterRendering: content.afterFrame()
    onVisibleChanged: content.b.mark("window-visible")
    onClosing: (close) => content.requestClose(close)

    Component.onCompleted: {
        // 有命令行图片就按文件头量出的尺寸预置，再摆到鼠标光标中心；
        // 空启动只摆位置（保持默认大小，不超出屏幕/任务栏）。
        content.hostWindow = shell
        content.presizeWindow(shell, content.initialFileArg())
        if (Qt.application.arguments.indexOf("--hidden") === -1)
            shell.shown = true
    }

    WindowEdges {}

    EditorContent {
        id: content
        anchors.fill: parent
        primary: true
        // 每次装上新的图片，把窗口调成刚好装下它（内容自己判断尺寸/钳制）。
        onWindowFitRequested: content.fitWindowToImage(shell)
        onRequestNewWindow: (fileUrl) => openNewWindow(fileUrl)
    }

    // Strong references to every live secondary document window. createObject
    // with a null parent transfers ownership to the JS engine, so without
    // this array the GC may collect a visible window — it would just close
    // itself after a while (the "new window randomly disappears" bug).
    property var docWindows: []

    function openNewWindow(fileUrl) {
        // 每个新窗口都精确居中鼠标光标（DocWindow 在显示前自己量尺寸+定位；
        // x/y 只是自动布局不可用时的兜底）。
        const win = docWindowFactory.createObject(null, {
            x: 80, y: 60,
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
