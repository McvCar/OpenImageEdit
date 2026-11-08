import re, json

path = "crates/ui/src/bridge/mod.rs"
src = open(path, encoding="utf-8").read()
lines = src.split("\n")

methods = json.load(open("scripts/bridge_methods2.json"))
# build name -> method
byname = {m["name"]: m for m in methods}

# Grouping: pub (qinvokable) methods -> ops file. Private helpers stay in mod.rs.
groups = {
    "shell": ["take_external_open"],
    "settings": ["register_as_viewer", "update_checker_light", "update_checker_dark",
                 "update_open_in_new_window", "update_large_downsample",
                 "update_save_quality", "update_ruler_bottom_up"],
    "document": ["image_shown", "mark", "open_file", "navigate", "export_layer_png",
                 "save_edited", "save_as", "poll_decode"],
    "view": ["set_viewport", "zoom_in", "zoom_out", "zoom_step", "zoom_fit", "zoom_actual"],
    "canvas": ["rotate_cw", "rotate_ccw", "flip_horizontal", "flip_vertical", "undo", "redo",
               "crop_to", "resize_to", "resize_canvas", "reset_crop", "extract_selection"],
    "layers": ["layer_name", "layer_visible", "select_layer", "toggle_selected_visible",
               "set_layer_visible_at", "delete_selected_layer", "move_selected_up",
               "move_selected_down", "reorder_layer", "translate_selected",
               "set_selected_opacity", "rename_selected_layer", "scale_selected_layer",
               "scale_layer_begin", "scale_layer_update", "scale_layer_end",
               "set_layer_locked", "set_base_locked", "toggle_base_visible",
               "delete_base_layer", "drag_translate_begin", "drag_translate_update",
               "drag_translate_end"],
    "adjust": ["adjust_begin", "adjust_set", "adjust_set_levels", "adjust_auto_levels",
               "adjust_reset", "adjust_commit", "adjust_cancel"],
    "picker": ["picker_begin", "picker_close", "picker_pick", "picker_select", "picker_clear"],
    "clipboard": ["copy_selection_pixels", "cut_selection_pixels", "erase_selection_pixels",
                  "copy_selected_layer", "cut_selected_layer", "paste_clipboard"],
    "marks": ["select_mark", "add_shape_mark", "add_brush_mark", "add_mosaic_mark",
              "add_mosaic_rect", "add_text_mark", "update_mark_cell", "update_text_style",
              "set_editing_mark", "update_text_content", "mark_drag_begin",
              "mark_drag_update", "mark_drag_end", "delete_selected_mark",
              "set_mark_visible", "update_mark_style"],
}

# sanity: every grouped name must be a pub method
for g, names in groups.items():
    for n in names:
        assert n in byname, "missing %s" % n
        assert byname[n]["pub"], "%s expected pub" % n
# start already moved; ensure not double-listed
assert "start" not in [n for ns in groups.values() for n in ns]

moved_names = set(n for ns in groups.values() for n in ns)
print("moving %d pub methods across %d files" % (len(moved_names), len(groups)))

# ---- extract method text for each name (with leading attributes) ----
def method_text(name):
    m = byname[name]
    return "\n".join(lines[m["a"]-1:m["b"]])

# ---- generate each ops file ----
# Rust 子模块不继承父模块的 use 导入，所以每个 ops 文件自带完整 import 块。
# 外部 crate（cxx_qt_lib/decode/editcore/image）经 extern prelude 全模块可见；
# qobject 是本 crate 内的，用 super::super 取。#![allow(unused_imports)] 压制
# 每个文件没用到那几个 import 的告警。
HEADER = (
    "#![allow(unused_imports)]\n"
    "use core::pin::Pin;\n"
    "use std::path::{Path, PathBuf};\n"
    "use cxx_qt_lib::{QString, QUrl};\n"
    "use decode::{decode_path, encode, format_from_path, DecodeOptions, Decoded, Format};\n"
    "use editcore::{\n"
    "    adjust_json, Adjust, AdjustTarget, Document, Histogram, Layer, Mark, MarkKind, PickedColor,\n"
    "    Rect, Selection, Size, Transform,\n"
    "};\n"
    "use image::RgbaImage;\n"
    "use super::super::qobject;\n"
    "\n"
)
doclines = {
    "shell": "窗口级外部打开事件。",
    "settings": "配置项 / 棋盘格 / 注册查看器。",
    "document": "载入 / 解码 / 保存 / 文件信息。",
    "view": "视口 / 缩放 / 预览刷新。",
    "canvas": "裁剪 / 画布缩放 / 旋转翻转 / 撤销重做 / 选区提取。",
    "layers": "全部图层操作。",
    "adjust": "色彩调整。",
    "picker": "拾色器。",
    "clipboard": "剪贴板。",
    "marks": "标记 / 批注。",
}
for g, names in groups.items():
    body = []
    for n in names:
        body.append(method_text(n))
    content = "// %s\n" % doclines[g]
    content += HEADER
    content += "impl qobject::Bridge {\n"
    content += "\n\n".join(body)
    content += "\n}\n"
    open("crates/ui/src/bridge/ops/%s.rs" % g, "w", encoding="utf-8").write(content)
    print("  wrote ops/%s.rs (%d methods)" % (g, len(names)))

# ---- remove moved methods from mod.rs impl block ----
# Collect line ranges to delete (1-based a..b inclusive). Sort desc to splice safely.
ranges = sorted([byname[n] for n in moved_names], key=lambda m: m["a"], reverse=True)
new_lines = lines[:]
for m in ranges:
    a, b = m["a"], m["b"]
    # also drop a trailing blank line after the method if present
    end = b
    while end < len(new_lines) and new_lines[end].strip() == "":
        end += 1
    del new_lines[a-1:end]
new_src = "\n".join(new_lines)
open(path, "w", encoding="utf-8").write(new_src)
print("mod.rs now %d lines (was %d)" % (len(new_lines), len(lines)))

# ---- update ops/mod.rs ----
mods = "pub(super) mod %s;\n"
ops_mod = ("//! `impl qobject::Bridge` 的方法按功能拆到各子模块。\n"
           "//!\n"
           "//! 可见性事实：私有助手方法留在 `bridge/mod.rs`（父模块），本目录所有\n"
           "//! 后代模块可直接访问（Rust 后代可见祖先私有项），故 qinvokable 方法调用\n"
           "//! 助手方法无需改可见性。`#[qproperty]/#[qinvokable]` 声明全在 mod.rs 的\n"
           "//! `qobject` 块里，这里只是实现体。\n\n")
for g in ["shell", "settings", "document", "view", "canvas", "layers",
          "adjust", "picker", "clipboard", "marks"]:
    ops_mod += mods % g
open("crates/ui/src/bridge/ops/mod.rs", "w", encoding="utf-8").write(ops_mod)
print("updated ops/mod.rs")
print("DONE")