"""生成修复 / 油漆探针用的确定性测试图。

布局（200x200，RGB）：
  - 背景：浅灰 (240,240,240)
  - 红色实心圆：圆心 (60,60) 半径 30 —— 油漆桶的"封边"
  - 黑点：圆心 (150,150) 半径 6 —— 污点修复的目标
  - 竖条纹块：(130..190, 20..60) 每 8px 黑白交替 —— 修复画笔的取样源
"""
import sys
from PIL import Image, ImageDraw

out = sys.argv[1] if len(sys.argv) > 1 else "paint_test.png"
img = Image.new("RGB", (200, 200), (240, 240, 240))
d = ImageDraw.Draw(img)
d.ellipse((30, 30, 90, 90), fill=(255, 0, 0))
d.ellipse((144, 144, 156, 156), fill=(0, 0, 0))
for i, x in enumerate(range(130, 190, 8)):
    color = (0, 0, 0) if i % 2 == 0 else (255, 255, 255)
    d.rectangle((x, 20, x + 7, 60), fill=color)
img.save(out)
print(out)
