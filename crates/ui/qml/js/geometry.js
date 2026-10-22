// 几何/光标纯函数（阶段 4 从 EditorContent 迁出）。

function boxCursor(x, y, bx, by, bw, bh) {
    const tol = 7
    if (x < bx - tol || x > bx + bw + tol
            || y < by - tol || y > by + bh + tol)
        return -1
    const nearL = Math.abs(x - bx) <= tol
    const nearR = Math.abs(x - (bx + bw)) <= tol
    const nearT = Math.abs(y - by) <= tol
    const nearB = Math.abs(y - (by + bh)) <= tol
    if ((nearL || nearR) && (nearT || nearB))
        return (nearL === nearT) ? Qt.SizeFDiagCursor
                                 : Qt.SizeBDiagCursor
    if (nearL || nearR)
        return Qt.SizeHorCursor
    if (nearT || nearB)
        return Qt.SizeVerCursor
    if (x > bx && x < bx + bw && y > by && y < by + bh)
        return Qt.SizeAllCursor
    return -1
}
