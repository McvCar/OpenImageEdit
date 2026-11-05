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
    // 可选：四点折线（[x1,y1,…,x4,y4]，本项坐标）。给了就画四边形而不是
    // 轴对齐矩形。
    property var quad: null
    property color dashColor: "#5b9dff"
    // 蚂蚁线相位。图层框是静态的（animated: false），选区预览框仍然流动。
    property real dashPhase: 0
    property bool animated: true
    onRxChanged: requestPaint()
    onRyChanged: requestPaint()
    onRwChanged: requestPaint()
    onRhChanged: requestPaint()
    onQuadChanged: requestPaint()
    onDashColorChanged: requestPaint()
    onDashPhaseChanged: requestPaint()
    onPaint: {
        var ctx = getContext("2d")
        ctx.clearRect(0, 0, width, height)
        ctx.strokeStyle = String(dashColor)
        ctx.lineWidth = 1.5
        ctx.setLineDash([5, 4])
        ctx.lineDashOffset = dashPhase
        if (quad !== null && quad.length >= 8) {
            ctx.beginPath()
            ctx.moveTo(quad[0], quad[1])
            for (var i = 2; i + 1 < quad.length; i += 2)
                ctx.lineTo(quad[i], quad[i + 1])
            ctx.closePath()
            ctx.stroke()
            return
        }
        if (rw < 1 || rh < 1)
            return
        ctx.strokeRect(rx + 0.75, ry + 0.75, rw - 1.5, rh - 1.5)
    }
    Timer {
        interval: 90
        repeat: true
        running: dashed.animated && dashed.visible
                 && (dashed.quad !== null && dashed.quad.length >= 8
                     || dashed.rw > 0 && dashed.rh > 0)
        onTriggered: dashed.dashPhase = (dashed.dashPhase + 1) % 9
    }
}
