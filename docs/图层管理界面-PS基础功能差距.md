# 图层管理界面 vs Photoshop —— 基础功能差距

审计时间：2026-11-21。对象：`crates/ui/qml/panels/LayerPanel.qml`（1210 行）+ `crates/ui/src/bridge/ops/{layers,groups,layer_cmds}.rs` + `crates/core/src/{layer,op}.rs`。
口径：**基础功能**＝PS 图层面板日常操作；不评估图层复合、时间轴、智能滤镜等进阶模块。

## 一、已经具备（不做改动）

| 功能 | 实现 |
| --- | --- |
| 缩略图+名称+副信息（尺寸/坐标/不透明度） | LayerPanel.qml 489–677 |
| 显隐（眼睛，含组/背景） | 708–770；组有 `hidden_snapshot` 还原成员原显隐 |
| 锁定（单一 bool，含组/背景） | 772–822 |
| 多选（Ctrl 切换 / Shift 连选）、双击重命名 | 824–925 |
| 拖拽排序（全局 z 序）、背景层不参与 | 857–891 |
| 新建/复制/删除图层 | 289–305、1164–1175 |
| 编组 / 解组 / 折叠展开 / 组重命名、显隐、锁 | 317–338、1094–1126 |
| 合并选中图层（≥2，背景不参与） | 306–316、1127–1134 |
| 6 向对齐 + 水平/垂直等距分布 | 349–410 |
| 不透明度滑杆（选中层） | 943–1042 |
| 栅格化标记、旋转图层、导出单图层 PNG | 1135–1163 |

## 二、缺失的基础功能（按优先级）

### P0：后端已支持、界面/编辑链路没接 ✅ 已完成

| # | 功能 | 后端现状 | 状态 |
| --- | --- | --- | --- |
| 1 | **混合模式下拉** | `Layer::blend` / `LayerGroup::blend` 已支持 27 种（含 Blend If、挖空、组模式）；PSD 打开→保存不丢 | ✅ 面板顶部下拉，选图层改层模式、选组改组模式；进历史可撤销 |
| 2 | **组不透明度 / 组混合模式** | `LayerGroup::opacity`、`blend`、`pass_through` 均在 core 中生效 | ✅ 选中组时下拉变「组混合」，右侧出组不透明度拖动条（松手提交一条历史） |
| 3 | **组穿透开关（pass through）** | `LayerGroup::pass_through` 已有 | ✅ 组属性区「穿透」按钮（蓝色=开） |

**实现落点**：`core::blend` 的 `BLEND_MODES`（27 项，UI↔core 整数契约）+ `from_index/index/count`；`op.rs` 四个可撤销 op（`SetBlend` / `SetGroupBlend` / `SetGroupOpacity` / `SetGroupPassThrough`）；`document::set_layer_blend` 与 `group_ops` 三个组 setter；bridge `blend_mode_count/label`、`layer_blend_mode/set_layer_blend_mode`、`group_*` 六个方法（同步 cxx 块）；`session.rs` 的 `layers_json` 加 `b`、`groups_json` 加 `b/o/pt`；`LayerPanel.qml` 第三排控件 + Flickable 下拉（27 项不溢出面板）。

**验证**：`check_bridge_api` PASS（174 方法 0 缺失）；单测 core 274 / decode 84 / render 24 全绿；`probe_blend_modes`（新，设置→撤销→重做→非法索引不变）、`probe_layer_ops`、`probe_layer_groups`、`probe_layer_panel` 全过；dump-flat MAE 1.951 无回归。

### P1：PSD 只读、完全不可编辑（需要新增编辑模型）

| # | 功能 | 现状 | 说明 |
| --- | --- | --- | --- |
| 4 | **图层样式（fx）添加/编辑/复制粘贴/显隐** | 805 个样式已烘焙进位图（能正确显示） | `Layer::style: Option<StylePixels>` 只是渲染产物；无结构化样式对象、无 EditOp、无 fx 行与眼睛。双击层只触发重命名 |
| 5 | **图层蒙版 / 矢量蒙版** | PSD 导入的蒙版参与渲染（P0 保真） | `Layer::mask` 只读；不能新建/启用/停用/删除/编辑蒙版，也没有蒙版缩略图 |
| 6 | **剪贴蒙版** | PSD 导入的 `clipping` 参与合成 | 无创建/释放命令（右键菜单没有），`Layer::clipping` 无 EditOp |
| 7 | **调整图层 / 填充图层** | 导入的调整层已被烘进下方像素 | 不能新建；颜色调整面板走 `AdjustPixels` 破坏性烘焙 |
| 8 | **填充不透明度（Fill）** | `Layer` 只有 `opacity` | PS 的「不透明度」与「填充」是两个独立值；没有 Fill 字段，也没有 UI |

### P2：组织与操作类基础功能

| # | 功能 | 现状 | 说明 |
| --- | --- | --- | --- |
| 9 | **锁定细分**（透明像素/图像像素/位置/全部） | 只有一个 bool 锁 | PS 面板是四个独立锁；现在锁上就什么都做不了，无法「只锁位置」 |
| 10 | **新建空组** | `group_selected_layers()` 要求 ≥2 层 | PS 可直接建空组再拖进去；现在必须先有两层 |
| 11 | **拖入/拖出组、组内排序** | `reorder_layer(from,to)` 只改全局 z 序 | `SetLayerGroup` EditOp 有，但面板拖拽没有改 `group` 归属的路径；组结构一旦编好只能解组重编 |
| 12 | **向下合并 / 合并可见 / 盖印（Shift+Ctrl+E）** | UI 已按用户定稿删除 | 属于有意裁剪，不是遗漏；若要对齐 PS 需重新拍板 |
| 13 | **Alt+点击眼睛「只显示该层」/ 显示全部 / 隐藏其它** | 无 | 只有单层切换与组批量显隐 |
| 14 | **图层链接** | 无 | PS 传统基础功能（链图标），core 无字段 |
| 15 | **图层颜色标签** | 无 | core 无字段；PS 用于分组标记 |
| 16 | **搜索/按类型过滤图层** | 无 | 面板顶部只有计数；100+ 层文件无检索手段 |
| 17 | **缩略图选项**（小/大、图层内容 vs 整文档） | 固定 34px、只看本层 | 无面板菜单可切换 |
| 18 | **智能对象** | 无 | 需要全新的内容类型，工作量大，优先级最低 |

## 三、建议实施顺序

1. ~~先补 P0 三项~~ ✅ **已完成**（见上表实现落点）。
2. **接下来补 P1 的 Fill 与蒙版开关**：Fill 需在 `Layer` 加字段 + 写回 PSD；蒙版至少先做「启用/停用/删除已导入蒙版」，比新建蒙版简单。
3. **P2 里先做 9/10/11/13**：改起来都在面板层 + 少量 bridge，能显著接近 PS 手感；颜色标签/搜索/智能对象放最后。

## 四、相关源码位置

- 面板：`crates/ui/qml/panels/LayerPanel.qml`（头部 203–431，行 489–925，底部 943–1042，右键菜单 1044–1177）
- bridge：`crates/ui/src/bridge/ops/layers.rs`、`groups.rs`、`layer_cmds.rs`；声明处 `crates/ui/src/bridge/mod.rs`
- 数据：`crates/core/src/layer.rs`（`Layer` 201、`LayerGroup` 49）、`crates/core/src/op.rs`（`EditOp` 30）
