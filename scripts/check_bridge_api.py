"""桥方法声明自检：QML 里 `bridge.xxx(...)` 用到的每个方法，必须在
`crates/ui/src/bridge/mod.rs` 的 cxx-qt extern 块里声明过（`#[qinvokable]`）。

为什么需要这条检查：**漏声明不会编译报错**。QML 是运行时按名字找成员的，
少了就抛 `TypeError: Property 'xxx' of object ... is not a function`，按钮
点下去毫无反应；而脚本探针（`--script=`）是在 Rust 里直接调同一个方法，
照样 PASS —— 于是「探针全绿 + 按钮点了没反应」这种最难查的组合就出现了。
（`merge_selected_layers` 就是这么漏的。）

跑法：
    python scripts/check_bridge_api.py
"""
import glob
import io
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
QML_GLOB = os.path.join(ROOT, "crates", "ui", "qml", "**", "*.qml")
BRIDGE = os.path.join(ROOT, "crates", "ui", "src", "bridge", "mod.rs")


def main():
    qml = ""
    for path in glob.glob(QML_GLOB, recursive=True):
        qml += io.open(path, encoding="utf-8").read()
    calls = set(re.findall(r"bridge\.([A-Za-z_][A-Za-z0-9_]*)\s*\(", qml))

    mod = io.open(BRIDGE, encoding="utf-8").read()
    declared = set(re.findall(r"fn\s+([a-z_][A-Za-z0-9_]*)\s*\(", mod))

    missing = sorted(c for c in calls if c not in declared)
    print("QML 调用的桥方法 %d 个，未声明 %d 个" % (len(calls), len(missing)))
    for name in missing:
        print("  FAIL 未在 cxx 块声明:", name)
    print("PASS" if not missing else "FAILED")
    return 0 if not missing else 1


if __name__ == "__main__":
    sys.exit(main())
