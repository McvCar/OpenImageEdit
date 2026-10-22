import QtQuick
import com.imageedit

// 4px resize strips for the frameless windows (native system resize).
Item {
    anchors.fill: parent
    z: 1000

    EdgeGrab { anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right; height: 4; edges: Qt.TopEdge; shape: Qt.SizeVerCursor }
    EdgeGrab { anchors.bottom: parent.bottom; anchors.left: parent.left; anchors.right: parent.right; height: 4; edges: Qt.BottomEdge; shape: Qt.SizeVerCursor }
    EdgeGrab { anchors.top: parent.top; anchors.bottom: parent.bottom; anchors.left: parent.left; width: 4; edges: Qt.LeftEdge; shape: Qt.SizeHorCursor }
    EdgeGrab { anchors.top: parent.top; anchors.bottom: parent.bottom; anchors.right: parent.right; width: 4; edges: Qt.RightEdge; shape: Qt.SizeHorCursor }
    EdgeGrab { anchors.top: parent.top; anchors.left: parent.left; width: 10; height: 10; edges: Qt.TopEdge | Qt.LeftEdge; shape: Qt.SizeFDiagCursor }
    EdgeGrab { anchors.top: parent.top; anchors.right: parent.right; width: 10; height: 10; edges: Qt.TopEdge | Qt.RightEdge; shape: Qt.SizeBDiagCursor }
    EdgeGrab { anchors.bottom: parent.bottom; anchors.left: parent.left; width: 10; height: 10; edges: Qt.BottomEdge | Qt.LeftEdge; shape: Qt.SizeBDiagCursor }
    EdgeGrab { anchors.bottom: parent.bottom; anchors.right: parent.right; width: 10; height: 10; edges: Qt.BottomEdge | Qt.RightEdge; shape: Qt.SizeFDiagCursor }
}