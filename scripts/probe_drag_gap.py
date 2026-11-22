# -*- coding: utf-8 -*-
"""模拟面板拖动的落点换算（release 两步组合）在真实 app 里的行为。

场景 A（用户 88→24 报告）：层 A 在组 1（z 高），层 B 在组 2。拖 A 到
B 行下半 → release = move_group(A→组2) + z-move(A, m-1)，期望显示序
中 A 紧贴 B 下一行。

场景 B（同级分组调序）：拖组 1 到组 2 最高成员下面 → group-z，期望
组 1 标题出现在组 2 下面、组块连续、其余相对序不变。
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


def show(rows, groups):
    """按显示序打印行。"""
    gmeta = {g['id']: g for g in groups}
    out = []
    for r in rows:
        if r.get('gid'):
            out.append('[组%d %s]' % (r['gid'], gmeta.get(r['gid'], {}).get('n', '')))
        else:
            out.append('L%d(%s)' % (r['i'], r.get('n', '')))
    return ' '.join(out)


def main():
    fixture(path=FIX)
    # fixture 是 PNG（只有背景），每次 run 都要重建层 + 组。组 id 每次
    # 从 1 重新编号，所以 setup 序列产出的组 id 稳定可复用。
    SETUP = ['layer-new', 'layer-new', 'layer-new', 'layer-new', 'layer-new',
             'multi-select:1,2', 'layer-group', 'multi-select:3,4',
             'layer-group']
    lines = run(SETUP + ['groups-dump', 'layers-dump'])
    gd = parse_events(lines, 'groups-dump')[-1]['groups']
    ld = parse_events(lines, 'layers-dump')[-1]['layers']
    print('初始:', show(ld, gd))
    g2 = next(g['id'] for g in gd if 3 in (g.get('mids') or []))

    # ---- 场景 A：层 2（组1 内）拖到层 3（组3 内）行下半 ----
    # release 两步：move_group(2→组3) + z-move(2, 2)（m=3, m'-1=2）。
    # 层 2 原文档索引 2 → toZ==from 不动 z，只改归属 → 显示序不变，
    # 但 L2 显示在 L3 下一行（L2 在组 3 内、索引 2 < 3）。
    lines = run(SETUP + ['move-group:2,%d' % g2, 'z-move:2,2',
                         'groups-dump', 'layers-dump'])
    gd = parse_events(lines, 'groups-dump')[-1]['groups']
    ld = parse_events(lines, 'layers-dump')[-1]['layers']
    # 显示顺序 = dump 行序；行 i 字段恒为文档索引降序（rev），位置变化
    # 要看名字序列。
    seq = [r.get('n') for r in ld if r.get('i', -1) > 0]
    ok_a = seq == ['5', '4', '3', '2', '1']
    in_g2 = next((r['g'] for r in ld if r.get('i') == 2), 0) == g2
    print('场景A 显示层序:', seq, 'PASS' if (ok_a and in_g2) else 'FAIL',
          '|', show(ld, gd))

    # ---- 场景 B：把组 1（块 {1,2}）排到层 4 下面（锚=4, above=False）----
    # shifted = 4-2 = 2 → t=2 → 块占 [2,3] → doc=[0,3,1,2,4]
    # 显示序 4,2,1,3：组 1 标题紧贴 L4 下一行 ✓
    lines = run(SETUP + ['group-z:1,4,0', 'groups-dump', 'layers-dump'])
    gd = parse_events(lines, 'groups-dump')[-1]['groups']
    ld = parse_events(lines, 'layers-dump')[-1]['layers']
    print('场景B raw:', [(r['i'], r.get('n')) for r in ld])
    seq = [r.get('n') for r in ld if r.get('i', -1) > 0]
    ok_b = seq == ['5', '4', '2', '1', '3']
    print('场景B 显示层序:', seq, 'PASS' if ok_b else 'FAIL',
          '|', show(ld, gd))

    # ---- 场景 C：撤销场景 B 恢复原状 ----
    lines = run(SETUP + ['group-z:1,4,0', 'undo', 'layers-dump'])
    ld = parse_events(lines, 'layers-dump')[-1]['layers']
    seq = [r.get('n') for r in ld if r.get('i', -1) > 0]
    ok_c = seq == ['5', '4', '3', '2', '1']
    print('场景C undo 显示层序:', seq, 'PASS' if ok_c else 'FAIL')

    print('RESULT:', 'PASS' if (ok_a and ok_b and ok_c) else 'FAIL')


if __name__ == '__main__':
    main()
