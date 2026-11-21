"""混合模式 / 组不透明度 / 组穿透探针（P0）。

fixture 是普通 PNG（只有背景），先建层编组，再用 script op 设置混合模式
与组属性，检查 layers-dump / groups-dump 的 b / o / pt 字段，并验证
撤销 / 重做能精确回到原值。

覆盖：
    图层混合模式设置 + 撤销重做；组混合模式 / 不透明度 / 穿透设置 +
    撤销重做；非法模式索引不改变状态。

跑法：
    python scripts/probe_blend_modes.py
输出：
    每个检查点一行摘要 + PASS / FAILED
"""
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, fixture, parse_events, pump  # noqa: E402

FIX = os.path.join(ROOT, "probe_blend_modes.png")
GID = 1

# 索引 3 = Screen，索引 8 = LinearBurn（见 core::BLEND_MODES）。
OPS = [
    "layers-dump", "groups-dump",                     # ck0 / gk0 初始
    "layer-new", "layer-new",
    "multi-select:1,2",
    "layer-group",                                    # GID=1
    "groups-dump",                                    # gk1 组建好
    "blend:1,3",                                      # 图层 1 -> Screen
    "layers-dump",                                    # ck1
    "undo",
    "layers-dump",                                    # ck2 撤销 -> Normal
    "redo",
    "layers-dump",                                    # ck3 重做 -> Screen
    "group-blend:%d,8" % GID,                         # 组 -> LinearBurn
    "group-opacity:%d,0.4" % GID,                     # 组 -> 40%
    "group-pass:%d,0" % GID,                          # 组 -> 非穿透
    "groups-dump",                                    # gk2
    "undo", "undo", "undo",
    "groups-dump",                                    # gk3 全撤销
    "redo", "redo", "redo",
    "groups-dump",                                    # gk4 全重做
    "blend:1,999",                                    # 非法索引
    "layers-dump",                                    # ck4 仍 Screen
    "group-blend:%d,999" % GID,                       # 非法索引
    "groups-dump",                                    # gk5 仍 LinearBurn
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
    lines, _ = pump(proc, '"event":"script-done"', timeout=90)
    time.sleep(0.6)

    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    layers = parse_events(lines, "layers-dump")
    groups = parse_events(lines, "groups-dump")
    fails = []

    def ck(events, n, tag):
        if len(events) <= n:
            fails.append("%s%d 缺失（只收到 %d 个）" % (tag, n, len(events)))
            return None
        return events[n]

    def want(cond, msg):
        if not cond:
            fails.append(msg)

    def layer_rows(n):
        ev = ck(layers, n, "ck")
        return ev["layers"] if ev else None

    def group_rows(n):
        ev = ck(groups, n, "gk")
        return ev["groups"] if ev else []

    def b_at(rows, index):
        # `blend:i,mode` 的 i 是层下标（layers-dump 的 i 字段），不是稳定 id。
        for r in rows or []:
            if r.get("i") == index:
                return r.get("b")
        return None

    # --- 图层混合模式 ---
    r1 = layer_rows(1)
    print("ck1  b=%s" % [b_at(r1, i) for i in (1, 2)])
    want(b_at(r1, 1) == 3, "ck1 下标 1 层混合模式应为 Screen(3)")
    want(b_at(r1, 2) == 0, "ck1 下标 2 层混合模式应保持 Normal(0)")

    r2 = layer_rows(2)
    print("ck2  b=%s" % b_at(r2, 1))
    want(b_at(r2, 1) == 0, "ck2 撤销后混合模式应回到 Normal(0)")

    r3 = layer_rows(3)
    print("ck3  b=%s" % b_at(r3, 1))
    want(b_at(r3, 1) == 3, "ck3 重做后混合模式应回到 Screen(3)")

    r4 = layer_rows(4)
    print("ck4  b=%s (非法索引后)" % b_at(r4, 1))
    want(b_at(r4, 1) == 3, "ck4 非法模式索引不应改变混合模式")

    # --- 组属性 ---
    g2 = group_rows(2)
    g2v = {g["id"]: g for g in g2}
    print("gk2  b=%s o=%s pt=%s" % (g2v.get(GID, {}).get("b"),
                                   g2v.get(GID, {}).get("o"),
                                   g2v.get(GID, {}).get("pt")))
    want(GID in g2v, "gk2 组应存在")
    if GID in g2v:
        want(g2v[GID].get("b") == 8, "gk2 组混合模式应为 LinearBurn(8)")
        want(g2v[GID].get("o") == 40, "gk2 组不透明度应为 40")
        want(g2v[GID].get("pt") is False, "gk2 组穿透应关闭")

    g3 = group_rows(3)
    g3v = {g["id"]: g for g in g3}
    print("gk3  b=%s o=%s pt=%s" % (g3v.get(GID, {}).get("b"),
                                   g3v.get(GID, {}).get("o"),
                                   g3v.get(GID, {}).get("pt")))
    if GID in g3v:
        want(g3v[GID].get("b") == 0, "gk3 撤销后组混合模式应回 Normal(0)")
        want(g3v[GID].get("o") == 100, "gk3 撤销后组不透明度应回 100")
        want(g3v[GID].get("pt") is True, "gk3 撤销后组穿透应回开")

    g4 = group_rows(4)
    g4v = {g["id"]: g for g in g4}
    print("gk4  b=%s o=%s pt=%s" % (g4v.get(GID, {}).get("b"),
                                   g4v.get(GID, {}).get("o"),
                                   g4v.get(GID, {}).get("pt")))
    if GID in g4v:
        want(g4v[GID].get("b") == 8, "gk4 重做后组混合模式应回 LinearBurn(8)")
        want(g4v[GID].get("o") == 40, "gk4 重做后组不透明度应回 40")
        want(g4v[GID].get("pt") is False, "gk4 重做后组穿透应回关")

    g5 = group_rows(5)
    g5v = {g["id"]: g for g in g5}
    print("gk5  b=%s (非法索引后)" % g5v.get(GID, {}).get("b"))
    if GID in g5v:
        want(g5v[GID].get("b") == 8, "gk5 非法组模式索引不应改变状态")

    if fails:
        for f in fails:
            print("FAIL " + f)
        print("FAILED 混合模式/组属性探针")
        sys.exit(1)
    print("PASS  混合模式/组属性探针全部检查点通过")


if __name__ == "__main__":
    main()
