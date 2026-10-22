import QtQuick

// Dashed rectangle outline (selection / layer boxes). rx/ry/rw/rh are in
// this item's coordinates; parent is sized to the preview area.
Canvas {
    id: dashed
    // 同 markPreview：Cooperative 策略会丢弃单次重绘请求。
    renderStrategy: Canvas.Immediate
    renderTarget: Canvas.Image
    property real rx: 0
    property real ry: 0
    property real rw: 0
    property real rh: 0
    property color dashColor: "#5b9dff"
    // "Marching ants" phase: bumped periodically while visible, the
    // dash pattern slides backwards along the (clockwise) rect path,
    // which reads as a counterclockwise rotation.
    property real dashPhase: 0
    onRxChanged: requestPaint()
    onRyChanged: requestPaint()
    onRwChanged: requestPaint()
    onRhChanged: requestPaint()
    onDashColorChanged: requestPaint()
    onDashPhaseChanged: requestPaint()
    onPaint: {
        var ctx = getContext("2d")
        ctx.clearRect(0, 0, width, height)
        if (rw < 1 || rh < 1)
            return
        ctx.strokeStyle = String(dashColor)
        ctx.lineWidth = 1.5
        ctx.setLineDash([5, 4])
        ctx.lineDashOffset = dashPhase
        ctx.strokeRect(rx + 0.75, ry + 0.75, rw - 1.5, rh - 1.5)
    }
    Timer {
        interval: 90
        repeat: true
        running: dashed.visible && dashed.rw > 0 && dashed.rh > 0
        onTriggered: dashed.dashPhase = (dashed.dashPhase + 1) % 9
    }
}