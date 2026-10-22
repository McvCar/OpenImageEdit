import QtQuick
import com.imageedit

Rectangle {
    id: overflowMenu
    required property var toolbar
    visible: false
    z: 210
    width: 232
    height: menuCol.height + 10
    color: "#2a2d31"
    border.color: "#3c4043"
    radius: 6
    x: toolbar.width - width - 10
    y: toolbar.height + 2

    Column {
        id: menuCol
        anchors.top: parent.top
        anchors.topMargin: 5
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: 0

        Repeater {
            model: toolbar.menuActions

            delegate: MenuRow {
                required property var modelData
                required property int index
                // Reactive like the bar icons above.
                label: modelData.l
                separator: modelData.sep === true
                active: modelData.sep === true
                        || toolbar.menuActions[index].ok === true
                onActivated: {
                    overflowMenu.visible = false
                    modelData.run()
                }
            }
        }
    }
}
