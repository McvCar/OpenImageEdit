// 标记纯函数（阶段 4 从 EditorContent 迁出）。

function parseMarks(s) {
    if (!s)
        return []
    try {
        return JSON.parse(s)
    } catch (e) {
        return []
    }
}

function qmlFontFamily(id) {
    return id === "simsun" ? "SimSun"
         : id === "simhei" ? "SimHei"
         : id === "simkai" ? "KaiTi"
         : id === "arial" ? "Arial"
         : "Microsoft YaHei"
}
