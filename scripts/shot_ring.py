"""持续截图环：把主窗口每 300ms 抓一帧写进环（保留最近 80 帧）。

用法：python scripts/shot_ring.py <pid> <outdir> [seconds]
拖动一开始就能抓到「两份副本」那种瞬时状态（事后看预览 BMP 只能看到 CPU
合成结果，看不到 QML 叠加层，所以必须抓窗口）。
"""
import os
import subprocess
import sys
import time

PID = sys.argv[1]
OUT = sys.argv[2]
SECONDS = float(sys.argv[3]) if len(sys.argv) > 3 else 300.0
RING = 80
PY = sys.executable
SHOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "shot_window.py")

os.makedirs(OUT, exist_ok=True)
for f in os.listdir(OUT):
    try:
        os.remove(os.path.join(OUT, f))
    except OSError:
        pass

t0 = time.time()
i = 0
while time.time() - t0 < SECONDS:
    name = os.path.join(OUT, "f_%03d.png" % (i % RING))
    subprocess.run([PY, SHOT, PID, name], capture_output=True)
    os.utime(name, None)
    i += 1
    time.sleep(0.3)
