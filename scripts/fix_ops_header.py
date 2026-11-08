import re, glob

# 把每个 ops 文件的显式 import 块换成 `use super::*;`。
# glob 导入会拉入祖先 bridge 模块里对后代可见的全部私有项：
# 自由函数(file_url/elapsed_ms/...)、私有类型(Session/LiveScaleState/...)、
# 常量(ZOOM_STEP/PICKER_HISTORY/...)、以及 mod.rs 里各 use 绑定的外部类型，
# 还有 qobject 模块本身。
files = glob.glob("crates/ui/src/bridge/ops/*.rs")
for f in files:
    if f.endswith("mod.rs"):
        continue
    s = open(f, encoding="utf-8").read()
    # 从 #![allow(unused_imports)] 到 `use super::super::qobject;` 整段替换
    pat = re.compile(
        r"#!\[allow\(unused_imports\)\].*?use super::super::qobject;\n",
        re.DOTALL,
    )
    new = pat.sub("use super::super::*;\n", s, count=1)
    if new == s:
        print("  WARN no match in %s" % f)
    else:
        open(f, "w", encoding="utf-8").write(new)
        print("  fixed %s" % f)
print("DONE")