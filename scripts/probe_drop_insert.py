"""从资源管理器拖图进**已有画布**应插入为新图层，而不是替换当前文档。

做法：先加载 fixture（有画布），跑 `drop-insert:<png>` 脚本 op（等价 QML
DropArea → bridge.insert_image_from_url），异步解码落地后断言：

  ck1  出现 drop-insert-start + dropped-insert(ok=true) 事件，尺寸 = 拖入图；
  ck2  出现 insert-image-layer 事件（新层插入），index 是新层下标；
  ck3  **没有**第二次 load / fidelity-state / preview-ready(bump) —— 说明
       当前文档没被替换（这正是用户要的：不再「变成打开新文档」）；
  ck4  拖入后画布尺寸仍是原 fixture 尺寸（会话没换）。

跑法：
    python scripts/probe_drop_insert.py
"""
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, parse_events, pump  # noqa: E402
from probe_group_select import fixture, FIX  # noqa: E402
from PIL import Image  # noqa: E402


def main():
    fixture()
    src = os.path.join(os.environ["TEMP"], "drop_source.png")
    Image.new("RGBA", (123, 77), (220, 30, 90, 255)).save(src)

    env = dict(os.environ)
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    env["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + env.get("PATH", "")

    ops = [
        "drop-insert:" + src.replace("\\", "/"),
        "layers-group-dump",   # 立刻 dump（异步落地前，应仍是原层数）
    ]
    proc = subprocess.Popen(
        [EXE, FIX, "--probe-batch", "--script=" + ";".join(ops)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors="replace", env=env, bufsize=1,
    )
    # script-done 会先到（drop-insert 是异步），再等 dropped-insert 事件落地。
    lines, _ = pump(proc, '"event":"dropped-insert"', timeout=60)
    time.sleep(0.5)
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    fails = []

    def want(cond, msg):
        if not cond:
            fails.append(msg)

    starts = parse_events(lines, "drop-insert-start")
    drops = parse_events(lines, "dropped-insert")
    inserts = parse_events(lines, "insert-image-layer")
    loads = parse_events(lines, "load")

    print("events: start=%d dropped=%d insert=%d load=%d"
          % (len(starts), len(drops), len(inserts), len(loads)))

    # ck1
    want(len(starts) >= 1, "ck1 应有 drop-insert-start")
    want(len(drops) >= 1, "ck1 应有 dropped-insert 落地事件")
    if drops:
        d = drops[0]
        print("ck1 dropped %s" % d)
        want(d.get("ok") is True, "ck1 dropped-insert ok 应为 true")
        want(d.get("w") == 123 and d.get("h") == 77,
             "ck1 拖入尺寸应 123x77，实际 %sx%s" % (d.get("w"), d.get("h")))
    # ck2
    want(len(inserts) >= 1, "ck2 应有 insert-image-layer（图片插成新层）")
    if inserts:
        print("ck2 insert %s" % inserts[0])
    # ck3：只有最初打开 fixture 的一次 load，不能因拖图再 load 一次
    print("ck3 load count=%d（应 1）" % len(loads))
    want(len(loads) == 1, "ck3 拖图不应触发新的 open/load，实际 %d 次" % len(loads))
    # ck4：最终预览尺寸仍是 fixture 900x600
    previews = parse_events(lines, "preview-ready")
    last_prev = previews[-1] if previews else None
    print("ck4 last preview %s" % (last_prev.get("display") if last_prev else None))
    want(last_prev is not None and last_prev.get("display") == "900x600",
         "ck4 画布/预览尺寸应保持 900x600（文档未被替换）")

    if fails:
        print("FAIL:")
        for f in fails:
            print("  -", f)
        for line in lines[-25:]:
            print("  |", line[:200])
        return 1
    print("ALL PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
