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
    // 屏幕 devicePixelRatio（OS 缩放）。1 逻辑像素在 125% 屏上是 1.25
    // 设备像素，线宽得取 1/dpr 才是一根实心线；不传就是 100% 缩放。
    property real dpr: 1
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
        // ★线宽是 1 设备像素、并且描边落在**像素中心**（整数 + 0.5/dpr）。
        //   以前是 lineWidth 1.5 画在 rx+0.75 —— 描边正好骑在像素边界上，
        //   Qt 会把它摊成两条 50% 半透明的线：在 100% 缩放下只是有点发虚，
        //   在 64%/75% 这类非整数缩放下就变成一条"浅色细缝"，看起来像
        //   粘贴回去的图层没贴严（用户报的"边缘有空隙"就是这个）。
        const d = dpr > 0 ? dpr : 1
        const lw = 1 / d
        function snapDev(v) {
            return (Math.round(v * d) + 0.5) / d
        }
        ctx.lineWidth = lw
        ctx.setLineDash([5, 4])
        ctx.lineDashOffset = dashPhase
        if (quad !== null && quad.length >= 8) {
            ctx.beginPath()
            ctx.moveTo(snapDev(quad[0]), snapDev(quad[1]))
            for (var i = 2; i + 1 < quad.length; i += 2)
                ctx.lineTo(snapDev(quad[i]), snapDev(quad[i + 1]))
            ctx.closePath()
            ctx.stroke()
            return
        }
        if (rw < 1 || rh < 1)
            return
        // 四条边各自贴到最近的**像素中心**（设备像素），宽高按两端算差：
        // 这样 1px 描边正好吃掉一行像素，不会跨行摊成半透明。
        var x = (Math.round(rx * d) + 0.5) / d
        var y = (Math.round(ry * d) + 0.5) / d
        var w = (Math.round((rx + rw) * d) - 0.5) / d - x
        var h = (Math.round((ry + rh) * d) - 0.5) / d - y
        if (w < 0 || h < 0)
            return
        ctx.strokeRect(x, y, w, h)
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
