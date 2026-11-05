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
    // 轴对齐矩形——旋转后的图层框要用它，否则画的是"扩边后的位图包围盒"，
    // 四个角全是透明像素，看起来就像框平白变大了一圈。
    property var quad: null
    property color dashColor: "#5b9dff"
    // 深/花哨的图片上纯彩色虚线基本看不见（虚线本身留了空隙，透出的全
    // 是背景）。halo = true 时先铺一条半透明白实线打底，再叠虚线 —— 和
    // 蚂蚁线同一个思路，任何背景上都读得出来。
    property bool halo: false
    // "Marching ants" phase: bumped periodically while visible, the
    // dash pattern slides backwards along the (clockwise) rect path,
    // which reads as a counterclockwise rotation.
    property real dashPhase: 0
    onRxChanged: requestPaint()
    onRyChanged: requestPaint()
    onRwChanged: requestPaint()
    onRhChanged: requestPaint()
    onQuadChanged: requestPaint()
    onDashColorChanged: requestPaint()
    onDashPhaseChanged: requestPaint()
    onHaloChanged: requestPaint()
    onPaint: {
        var ctx = getContext("2d")
        ctx.clearRect(0, 0, width, height)
        if (quad !== null && quad.length >= 8) {
            ctx.beginPath()
            ctx.moveTo(quad[0], quad[1])
            for (var i = 2; i + 1 < quad.length; i += 2)
                ctx.lineTo(quad[i], quad[i + 1])
            ctx.closePath()
            ctx.setLineDash([5, 4])
            ctx.lineDashOffset = dashPhase
            if (halo) {
                // 白色**宽虚线**打底（同一相位），蓝线细一点压在上面：留空
                // 的地方还是空的，虚线节奏保住了；实线打底会让整条变成实线。
                ctx.lineWidth = 3
                ctx.strokeStyle = "#ffffff77"
                ctx.stroke()
            }
            ctx.strokeStyle = String(dashColor)
            ctx.lineWidth = 2
            ctx.stroke()
            return
        }
        if (rw < 1 || rh < 1)
            return
        var dx = rx + 0.75
        var dy = ry + 0.75
        var dw = rw - 1.5
        var dh = rh - 1.5
        ctx.setLineDash([5, 4])
        ctx.lineDashOffset = dashPhase
        if (halo) {
            ctx.lineWidth = 3
            ctx.strokeStyle = "#ffffff77"
            ctx.strokeRect(dx, dy, dw, dh)
        }
        ctx.strokeStyle = String(dashColor)
        ctx.lineWidth = 2
        ctx.strokeRect(dx, dy, dw, dh)
    }
    Timer {
        interval: 90
        repeat: true
        running: dashed.visible
                 && (dashed.quad !== null && dashed.quad.length >= 8
                     || dashed.rw > 0 && dashed.rh > 0)
        onTriggered: dashed.dashPhase = (dashed.dashPhase + 1) % 9
    }
}