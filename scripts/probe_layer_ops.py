"""图层命令矩阵探针（P1/P3）。

一条 `--script=` 串跑完面板上能点的所有图层命令，每个检查点打一次
`layers-dump`，然后按顺序断言。因为脚本 op 调的都是 QML 按钮点下去的同
一条桥方法（`layer_cmds.rs`），这里通过 = 面板按钮通过。

覆盖：
    新建 / 复制 / 向下合并 / 标记 / 栅格化 / 多选 / 顶对齐 /
    批量显隐（含撤销重做）/ 背景不可删 / 拼合 / 拼合可撤销 /
    合并选中层 / 画布 Ctrl+点（并入 → 松手补摘）

跑法：
    python scripts/probe_layer_ops.py
输出：
    每个检查点一行的摘要 + PASS / FAILED（不截图，纯断言，可进 CI）
"""
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, fixture, parse_events, pump  # noqa: E402

FIX = os.path.join(ROOT, "probe_layer_ops.png")

# 脚本 = 操作 + 检查点交替。注释写「跑完这一步应该是什么样」。
OPS = [
    "layers-dump",           # ck0 只有背景
    "layer-new",             # + 空图层
    "layers-dump",           # ck1 两层
    "layer-dup",             # 复制空图层（副本插在原件上面）
    "layers-dump",           # ck2 三层，顶层名字带「副本」
    "layer-merge-down",      # 副本合回原件
    "layers-dump",           # ck3 回到两层
    "mark-live",             # 画一个矩形标记（进 z 序最上面）
    "layers-dump",           # ck4 三层，顶层是标记
    "select-layer:2",
    "rasterize",             # 标记原地栅格化
    "layers-dump",           # ck5 三层，顶层变位图
    "multi-select:1,2",
    "align:top",             # 两层顶对齐（参照 = 两层的并集包围盒）
    "layers-dump",           # ck6 非背景层的 y 一致
    "layers-visible:0",      # 批量隐藏
    "layers-dump",           # ck7 选中的层 v=false
    "layers-visible:1",
    "undo",                  # 撤销「批量显示」→ 回到隐藏
    "layers-dump",           # ck8 非背景层 v=false
    "redo",                  # 重做 → 又显示
    "select-layer:0",
    "layers-delete",         # 背景不可删，条目必须留
    "layers-dump",           # ck9 层数不变、背景还在
    "flatten",               # 全部合进背景
    "layers-dump",           # ck10 只剩背景一层
    "undo",                  # 拼合可撤销
    "layers-dump",           # ck11 层数还原
    "multi-select:1,2",
    "layer-merge",           # 合并选中的两层（PS「合并图层」）
    "layers-dump",           # ck12 三层 → 两层，顶层是 raster
    # 画布 Ctrl+点那条路径（CanvasSurface）：按下 `select-mode:i,3` 只并入，
    # 松手无位移时补一次 `select-mode:i,1` 摘掉。两半都在这里盯住。
    "layer-new",             # 再补两层：0=背景，1/2 已选，3 留作「Ctrl 点它」
    "layer-new",
    "multi-select:1,2",
    "select-mode:3,3",       # Ctrl+点第 4 层（下标 3）
    "layer-sel-dump",        # ck13 三选，主选中层 = 3
    "select-mode:3,1",       # 松手补摘
    "layer-sel-dump",        # ck14 回到两选
]


def main():
    print("[1] fixture", flush=True)
    fixture(path=FIX)

    env = dict(os.environ)
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    env["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + env.get("PATH", "")

    print("[2] launch", flush=True)
    proc = subprocess.Popen(
        [EXE, FIX, "--probe-layers", "--script=" + ";".join(OPS)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors="replace", env=env, bufsize=1,
    )
    lines, _ = pump(proc, '"event":"script-done"', timeout=60)
    time.sleep(0.6)

    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    dumps = parse_events(lines, "layers-dump")
    fails = []

    def ck(n):
        if len(dumps) <= n:
            fails.append("ck%d 缺失（只收到 %d 个 layers-dump）" % (n, len(dumps)))
            return None
        return dumps[n]["layers"]

    def want(cond, msg):
        if not cond:
            fails.append(msg)

    def summary(n, rows):
        if rows is None:
            print("ck%-2d  ——" % n)
            return
        print("ck%-2d  %s" % (n, "  ".join(
            "%s[%s]" % (r["n"], r["k"]) for r in rows)))

    rows0 = ck(0)
    summary(0, rows0)
    if rows0 is not None:
        want(len(rows0) == 1, "ck0 初始应只有背景，实际 %d 层" % len(rows0))
        want(rows0 and rows0[0].get("bg") is True, "ck0 唯一那层应是背景")

    rows1 = ck(1)
    summary(1, rows1)
    if rows1 is not None:
        want(len(rows1) == 2, "ck1 新建后应 2 层，实际 %d" % len(rows1))
        want(rows1 and rows1[-1].get("bg") is True, "ck1 背景不在列表底部")

    rows2 = ck(2)
    summary(2, rows2)
    if rows2 is not None:
        want(len(rows2) == 3, "ck2 复制后应 3 层，实际 %d" % len(rows2))
        want(rows2 and "副本" in rows2[0]["n"],
             "ck2 顶层应是副本，实际名 %r" % (rows2[0]["n"] if rows2 else None))

    rows3 = ck(3)
    summary(3, rows3)
    if rows3 is not None:
        want(len(rows3) == 2, "ck3 向下合并后应 2 层，实际 %d" % len(rows3))

    rows4 = ck(4)
    summary(4, rows4)
    if rows4 is not None:
        want(len(rows4) == 3, "ck4 画标记后应 3 层，实际 %d" % len(rows4))
        want(rows4 and rows4[0]["k"] == "mark", "ck4 顶层应是 mark")

    rows5 = ck(5)
    summary(5, rows5)
    if rows5 is not None:
        want(len(rows5) == 3, "ck5 栅格化不该改层数，实际 %d" % len(rows5))
        want(rows5 and rows5[0]["k"] == "raster", "ck5 顶层应变成 raster")
        want(rows5 and rows5[0]["w"] > 0 and rows5[0]["h"] > 0,
             "ck5 栅格化结果尺寸为 0")

    rows6 = ck(6)
    summary(6, rows6)
    if rows6 is not None:
        body = [r for r in rows6 if not r.get("bg")]
        want(len(body) == 2, "ck6 非背景层应 2 个，实际 %d" % len(body))
        # 对齐语义（用户定）：选中两层时参照的是**两层自己的并集包围盒**，
        # 也就是「谁最靠上谁不动」，所以断言的是「两层的 y 一致」，不是 y=0。
        want(len({r["y"] for r in body}) == 1,
             "ck6 顶对齐后 y 不一致: %s" % [(r["n"], r["y"]) for r in body])

    rows7 = ck(7)
    summary(7, rows7)
    if rows7 is not None:
        body = [r for r in rows7 if not r.get("bg")]
        want(body and all(r["v"] is False for r in body),
             "ck7 批量隐藏没生效: %s" % [(r["n"], r["v"]) for r in body])

    rows8 = ck(8)
    summary(8, rows8)
    if rows8 is not None:
        body = [r for r in rows8 if not r.get("bg")]
        want(body and all(r["v"] is False for r in body),
             "ck8 撤销「批量显示」后应回到隐藏: %s"
             % [(r["n"], r["v"]) for r in body])

    rows9 = ck(9)
    summary(9, rows9)
    if rows9 is not None:
        want(rows9[-1].get("bg") is True, "ck9 背景被删掉了")
        want(len(rows9) == len(rows8 or rows9),
             "ck9 删背景不该改层数: %d → %d" % (len(rows8 or rows9), len(rows9)))

    rows10 = ck(10)
    summary(10, rows10)
    if rows10 is not None:
        want(len(rows10) == 1, "ck10 拼合后应只剩 1 层，实际 %d" % len(rows10))
        want(rows10 and rows10[0].get("bg") is True, "ck10 拼合结果应在背景层")

    rows11 = ck(11)
    summary(11, rows11)
    if rows11 is not None:
        want(len(rows11) == len(rows9 or rows11),
             "ck11 撤销拼合后层数应还原: %d → %d"
             % (len(rows9 or rows11), len(rows11)))

    rows12 = ck(12)
    summary(12, rows12)
    if rows12 is not None:
        before = len(rows11 or rows12)
        want(len(rows12) == before - 1,
             "ck12 合并选中层后应少一层: %d → %d" % (before, len(rows12)))
        want(rows12 and rows12[0]["k"] == "raster",
             "ck12 合并结果应是位图层（顶层 %r）"
             % (rows12[0]["k"] if rows12 else None))
        want(rows12 and rows12[-1].get("bg") is True, "ck12 背景被合并掉")

    # ck13/ck14：画布 Ctrl+点的两步（并入 → 松手补摘）。
    sels = parse_events(lines, "layer-sel-dump")
    for n, want_count, want_primary in ((0, 3, 3), (1, 2, None)):
        if len(sels) <= n:
            fails.append("ck%d 缺失（只收到 %d 个 layer-sel-dump）" % (13 + n, len(sels)))
            continue
        s = sels[n]
        print("ck%-2d  ids=%s primary=%s count=%s"
              % (13 + n, s.get("ids"), s.get("primary"), s.get("count")))
        want(s.get("count") == want_count,
             "ck%d 选中数应 %d，实际 %s" % (13 + n, want_count, s.get("count")))
        if want_primary is not None:
            want(s.get("primary") == want_primary,
                 "ck%d 主选中层应 %d，实际 %s" % (13 + n, want_primary, s.get("primary")))
        if n == 1:
            want(len(s.get("ids") or []) == 2,
                 "ck14 摘掉后集合应剩 2 层，实际 %s" % (s.get("ids"),))

    done = parse_events(lines, "script-done")
    if done:
        print("script-done:", {k: done[-1].get(k) for k in
                               ("layers", "selected_layer", "undo", "redo")})
    else:
        fails.append("没有 script-done，脚本可能中途 panic")

    for f in fails:
        print("FAIL", f)
    print("PASS" if not fails else "FAILED (%d)" % len(fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
