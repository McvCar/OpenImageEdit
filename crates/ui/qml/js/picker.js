// 拾色器纯函数（阶段 4 从 EditorContent 迁出）。

function parsePicker(s) {
    if (!s)
        return null
    try {
        const v = JSON.parse(s)
        return v && v.hex !== undefined ? v : null
    } catch (e) {
        return null
    }
}

function parsePickerHistory(s) {
    if (!s)
        return []
    try {
        const v = JSON.parse(s)
        return Array.isArray(v) ? v : []
    } catch (e) {
        return []
    }
}
