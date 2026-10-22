// 图层纯函数（阶段 4 从 EditorContent 迁出）。

function parseLayers(s) {
    if (!s)
        return []
    try {
        return JSON.parse(s)
    } catch (e) {
        return []
    }
}

function isLocked(item) {
    return item !== null && item !== undefined && item.l === true
}

function findBase(layerList) {
    for (var i = 0; i < layerList.length; i++)
        if (layerList[i].base === true)
            return layerList[i]
    return null
}
