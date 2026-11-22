# -*- coding: utf-8 -*-
"""模拟面板拖动的落点换算在真实 app 里的行为（一步式 API）。

场景 A1/A2：跨组拖层 —— release 只调一次 lmove-at:index,gid,z
（归组 + MoveLayer 单条历史）。z 是按 PS 缝语义算好的插入索引：
  缝在层行下方 → to = m'（锚点 remove 后索引）；缝在组标题行下
  方 / 层行上方（组顶）→ to = m'+1。旧两步组合会因 layers_json
  同步重建销毁 delegate、吞掉第二步（用户报「换分组固定在组顶」）。

场景 D：同组内拖到组顶（用户报「88 拖到上方图层没插到最前方」）。

场景 B/B2：组块 z 排序 group-z。B2 是「组向上拖」（t 落到块成员
索引之前）—— 旧构造循环在这里产出重复索引、ReorderLayers 直接
报错（用户报「同级分组调整顺序异常」）。
"""
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from probe_layer_panel import EXE, parse_events, pump, fixture  # noqa: E402

FIX = 'probe_drag_gap.png'


def run(ops, timeout=90):
    env = dict(os.environ)
    env['PATH'] = 'D:/Qt/6.8.3/msvc2022_64/bin;' + env.get('PATH', '')
    p = subprocess.Popen(
        [EXE, FIX, '--probe-layers', '--script=' + ';'.join(ops)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors='replace', env=env, bufsize=1)
    lines, _ = pump(p, '"event":"script-done"', timeout=timeout)
    time.sleep(0.2)
    p.terminate()
    return lines


def names(lines):
    """显示序名字序列（行序恒为文档索引降序，位置变化看名字）。"""
    ld = parse_events(lines, 'layers-dump')[-1]['layers']
    return [r.get('n') for r in ld if r.get('i', -1) > 0]


def group_of_name(lines, name):
    """按层名查归属（层移动后文档索引会变，名字才稳定）。"""
    ld = parse_events(lines, 'layers-dump')[-1]['layers']
    return next((r.get('g', 0) for r in ld if r.get('n') == name), 0)


def main():
    fixture(path=FIX)
    # fixture 是 PNG（只有背景），每次 run 都要重建层 + 组。组 id 每次
    # 从 1 重新编号，所以 setup 序列产出的组 id 稳定可复用。
    SETUP = ['layer-new', 'layer-new', 'layer-new', 'layer-new', 'layer-new',
             'multi-select:1,2', 'layer-group', 'multi-select:3,4',
             'layer-group']
    lines = run(SETUP + ['groups-dump', 'layers-dump'])
    gd = parse_events(lines, 'groups-dump')[-1]['groups']
    print('初始:', [r.get('n') for r in
                    parse_events(lines, 'layers-dump')[-1]['layers']])
    # mids 是层稳定 id（= 文档索引 + 1，背景 id=1），不是文档索引。
    g1 = next(g['id'] for g in gd if 2 in (g.get('mids') or []))
    g2 = next(g['id'] for g in gd if 4 in (g.get('mids') or []))

    ok = {}

    # ---- A1：层 2（组1 内，idx2）拖到层 3（组2 内，idx3）行下方 ----
    # 缝在 L3 下方 = L2 原位（L2 就在 L3 下面相邻）→ toZ = m' = 2 =
    # from → 只归组不落位。显示序不变，归属变 g2。
    lines = run(SETUP + ['lmove-at:2,%d,2' % g2, 'layers-dump'])
    seq = names(lines)
    ok['A1'] = seq == ['5', '4', '3', '2', '1'] and group_of_name(lines, '2') == g2
    print('A1 原位归组:', seq, 'PASS' if ok['A1'] else 'FAIL')

    # ---- A2：层 2 拖到层 3 行上方 = 组 2 标题行下方 = 组顶 ----
    # toZ = mp+1 = (3-1)+1 = 3 → remove(2) 后 L2 落 idx3 > L3(idx2)。
    lines = run(SETUP + ['lmove-at:2,%d,3' % g2, 'layers-dump'])
    seq = names(lines)
    ok['A2'] = seq == ['5', '4', '2', '3', '1'] and group_of_name(lines, '2') == g2
    print('A2 跨组一步到组顶:', seq, 'PASS' if ok['A2'] else 'FAIL')

    # ---- D：同组内把层 1（组1 底，idx1）拖到组内最高成员层 2 上方 ----
    # toZ = mp+1 = (2-1)+1 = 2 → remove(1) 后 L1 落 idx2 > L2(idx1)。
    lines = run(SETUP + ['z-move:1,2', 'layers-dump'])
    seq = names(lines)
    ok['D'] = seq == ['5', '4', '3', '1', '2']
    print('D 同组拖到组顶:', seq, 'PASS' if ok['D'] else 'FAIL')

    # ---- E：跨组一步式（层 1 → 组 2 组顶），撤销一次全恢复 ----
    lines = run(SETUP + ['lmove-at:1,%d,4' % g2, 'layers-dump'])
    seq = names(lines)
    in_g2 = group_of_name(lines, '1') == g2
    lines = run(SETUP + ['lmove-at:1,%d,4' % g2, 'undo', 'layers-dump'])
    seq_undo = names(lines)
    ok['E'] = (seq == ['5', '1', '4', '3', '2'] and in_g2
               and seq_undo == ['5', '4', '3', '2', '1'])
    print('E 跨组一步+undo:', seq, '→', seq_undo,
          'PASS' if ok['E'] else 'FAIL')

    # ---- B：把组 1（块 {1,2}）排到层 4 下面（锚=4, above=False）----
    # shifted = 4-2 = 2 → t=2 → 块占 [2,3] → doc=[0,3,1,2,4,5]。
    lines = run(SETUP + ['group-z:1,4,0', 'layers-dump'])
    seq = names(lines)
    ok['B'] = seq == ['5', '4', '2', '1', '3']
    print('B 组块向下排:', seq, 'PASS' if ok['B'] else 'FAIL')

    # ---- B2：组 2（块 {3,4}）向上拖到层 1 下面（锚=1, above=False）----
    # shifted = 1 → t=1 落在块成员索引之前：修复前构造循环在这里重复
    # push 块成员 → ReorderLayers 报 LayerIndex(6)。
    lines = run(SETUP + ['group-z:%d,1,0' % g2, 'groups-dump', 'layers-dump'])
    gd = parse_events(lines, 'groups-dump')[-1]['groups']
    seq = names(lines)
    ok['B2'] = seq == ['5', '2', '1', '4', '3']
    print('B2 组块向上排:', seq, 'PASS' if ok['B2'] else 'FAIL', '|',
          [(r['i'], r.get('n'), r.get('g')) for r in
           parse_events(lines, 'layers-dump')[-1]['layers']])

    # ---- C：撤销场景 B 恢复原状 ----
    lines = run(SETUP + ['group-z:1,4,0', 'undo', 'layers-dump'])
    seq = names(lines)
    ok['C'] = seq == ['5', '4', '3', '2', '1']
    print('C undo:', seq, 'PASS' if ok['C'] else 'FAIL')

    # ---- F：to = n-1（append 位）插到全文档最顶 ----
    # 旧 cap = n-2 把它钳到 n-2 → 只落到第二名（cap 修复的回归）。
    lines = run(SETUP + ['z-move:1,5', 'layers-dump'])
    seq = names(lines)
    ok['F'] = seq == ['1', '5', '4', '3', '2']
    print('F to=n-1 插到最顶:', seq, 'PASS' if ok['F'] else 'FAIL')

    bad = [k for k, v in ok.items() if not v]
    print('RESULT:', 'PASS' if not bad else 'FAIL ' + ','.join(bad))


if __name__ == '__main__':
    main()
