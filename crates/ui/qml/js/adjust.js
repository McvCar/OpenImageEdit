// 调色纯函数（阶段 4 从 EditorContent 迁出）。

function parseAdjust(s) {
    const empty = { e: 0, c: 0, hi: 0, sh: 0, sa: 0, te: 0, ti: 0,
                    se: 0, sp: 0, bl: 0, ga: 1, wh: 1, mh: 0.5 }
    if (!s)
        return empty
    try {
        const v = JSON.parse(s)
        for (const k in empty)
            if (v[k] === undefined)
                v[k] = empty[k]
        return v
    } catch (e) {
        return empty
    }
}

function parseHistogram(s) {
    if (!s)
        return null
    try {
        const v = JSON.parse(s)
        return (v && v.r && v.g && v.b) ? v : null
    } catch (e) {
        return null
    }
}
