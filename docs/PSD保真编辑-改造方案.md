# PSD 保真编辑改造方案

> 目标文件：`D:\_APP_CODE\_XhCompany\FontGame\Res\0302-界面UI-1.psd`（750×1700，117 MB）
> 基线提交：`3a42b56`（PSD 组树 / 分组写回）
> 实测脚本：`scripts/psd_fidelity_report.py`（本次新增，纯 psd-tools 只读）
> 参考实现：`D:\_APP_CODE\photocraft`（PhotoCraft，MIT OR Apache-2.0，纯 Rust 的 PS 重实现）
> 日期：2026-11-13

---

## 0. 一页结论

**能打开，但画得不像 PS；而且在编辑态下每帧都按 617 层重算，做不到「保真 + 流畅」同时成立。**

三组实测数字（debug 构建，本机 i7-8700）：

| 指标 | 实测值 | 说明 |
| --- | --- | --- |
| 首帧（同步装配 617 层 + 样式烘焙 + 首帧合成） | **3.25 s** | `shown_ms` |
| 分层编辑态内存 | **434 MB** | 617 层按 bbox 存位图，面积合计 41.9 Mpx |
| 全幅重建一帧（750×1700，617 层） | **124 ms** | `render-profile.composite`，`Document::flatten` |
| 预览尺寸重建（383×868） | **93 ms** | 同上，缩放后仍是 93 ms |
| 同一帧的「保真显示」（1 层 PS 合并图） | **8 ms** | 校准路径 |
| 重建合成 vs PS 自带合并图 | **MAE 10.67 / 255**，14.76% 像素差 > 8，9.24% > 32，最差 32px 块 **114.6** | alpha 完全一致（MAE 0.000） |

**差距不是「某个混合模式没实现」，而是三层**：

1. **合成语义缺两块地基**：剪贴蒙版（**210 个可见层**）和图层蒙版（**95 个可见层**）在文档模型里根本不存在 —— `Layer` 没有 `mask` / `clipping` 字段，`Document::flatten` 也不认。PS 保存的 PSD 里，被剪贴层的像素是「完整」的，剪贴只影响合成时的 alpha，所以我们把 210 个层都画满了。这是 MAE 10.67 的主因，也和最差区域完全吻合（见 §2.2 归因）。
2. **特性只能烘焙、不能live**：73 个调整层被逐层烘进下方像素，805 个图层样式被烘进图层位图（其中 775 个在可见层上）—— 几何基本对，但模糊半径、每效果的混合模式/等高线/杂色、调整的作用顺序都是近似。
3. **渲染架构是「整幅重算 + 整幅落盘」**：任何一次编辑都重合成整幅（124 ms），再把整幅编码成 BMP 写进临时文件让 QML 解码（`write` 约 130 ms/帧，5.1 MB/帧）。这条路上加任何特性都救不了交互延迟。

**保真编辑的解法（本方案主张）**：

- **不去重建没被编辑过的像素** —— PS 合并图本身就是逐像素真值，让它当「保真底图」；
- 画布切 **64 px 瓦片**，编辑只重合成**脏瓦片**：实测单块成本 **p50 0.22 ms / p95 1.16 ms**（对比全幅 124 ms，约 100–500×）；
- 脏瓦片内必须**语义正确**（P0 补剪贴/蒙版/组蒙版 + 单次量化），把块内 MAE 从 10.67 压到 ≤ 2 —— 否则编辑处会和周围保真像素出现**接缝**，这比整幅都偏一点更难接受。

分四步：**P0 语义补齐 → P1 瓦片增量 → P2 前缀缓存/遮挡裁剪 → P3 预览链路去落盘 → P4 通用特性**（§4）。

---

## 1. 目标文件的特性画像（实测）

`python scripts/psd_fidelity_report.py "<psd>"` 输出：

| 维度 | 数值 |
| --- | --- |
| 画布 / 位深 / 色彩 | 750×1700 / 8 bit / RGB（无 ICC，按 sRGB） |
| 图层记录（非组） | **694** |
| 图层组 | **197**（全部 `PASS_THROUGH`，23 个隐藏，9 个带组蒙版，0 个非 100% 不透明度） |
| 最深嵌套 | **5 层** |
| 图层类型 | shape **235**、pixel 161、smartobject **131**、type **93**、brightnesscontrast 37、huesaturation 37（可见合计 73 个调整层） |
| 混合模式 | Normal 650、**LinearDodge 31**、Multiply 10、Color 1、SoftLight 1、Screen 1 |
| 图层样式 | **259 个可见层 / 775 个样式实例**：Stroke 238、InnerShadow 217、DropShadow 170、GradientOverlay 100、ColorOverlay 40、InnerGlow 10 |
| 剪贴蒙版 | **212（可见 210）** |
| 图层蒙版 | **108（可见 95，其中 4 个黑底=默认隐藏）** |
| 矢量蒙版 | 234（形状层自带，栅格已按形状裁好） |
| 调整层 | **73**（Brightness/Contrast 37 + Hue/Saturation 36，**全部是剪贴型**） |
| 不透明度 < 100% | 72 个层 |
| 每像素平均堆叠 | **32.9 层**（最多 107 层） |
| 图层面积合计 | **41.9 Mpx = 32.9 × 画布** |

一句话画像：**这是一张 UI 素材稿** —— 矢量形状 + 智能对象 + 文字 + 大面积剪贴叠样式，靠"每个像素 33 层"堆出质感。它不考验位深/CMYK/3D，考验的是**合成语义**（剪贴、蒙版、样式的精确落地）。

---

## 2. 现状与差距

### 2.1 逐特性矩阵

| 特性 | 用量 | 当前处理 | 保真影响 | 代码位置 |
| --- | --- | --- | --- | --- |
| 混合模式 | 44 个非 Normal | 全部映射到 18 种，逐像素 f32 计算 | 无（已对齐 PS 公式） | `crates/core/src/blend.rs` |
| **剪贴蒙版** | 210 可见层 | **丢弃**（只记一句降级项） | **高**：被剪贴层画满整层 | `psd_layers.rs:339` |
| **图层蒙版** | 95 可见层 | **完全没读**（只读 RGBA + alpha 通道） | **高**：蒙版区域全显 | `psd_layers.rs:390-394` |
| 组蒙版 | 9 个组 | **丢弃** | 中 | 同上 |
| 组不透明度 / 组混合 | 0 / 197 穿透 | 组状态折进子层（本文件全 100% 穿透，等价） | 无（本文件）；通用缺失 | `psd_layers.rs:335` |
| 调整层 | 73（全剪贴） | **烘焙进下方层像素**（逐层，不是合成后） | 中：数学近似 + 顺序近似 | `psd_layers.rs:309-320` |
| 图层样式 | 775 实例 | **烘焙进图层位图**，6 类支持（阴影/内阴影/外发光/内发光/描边/颜色叠加/渐变叠加） | 中：模糊 σ、每效果混合模式、等高线、杂色未实现；图案叠加/斜面和浮雕直接跳过 | `psd_effects.rs` |
| 矢量形状 / 矢量蒙版 | 235 | 只用栅格（PS 存了渲染结果） | 显示无影响；缩放不重渲染 | — |
| 智能对象 | 131 | 只用栅格代理 | 显示无影响；替换内容/智能滤镜缺 | — |
| 文字 | 93 | 只用栅格 | 显示无影响；不可重排 | — |
| 位深 / 色彩模式 | 8 bit RGB | 支持；16/32 bit 与 CMYK 直接明确报错 | 无 | `decode/src/lib.rs:318` |
| 逐层量化 | 平均 33 层堆叠 | **每混一层就 round 到 u8** | 低-中：随机游走估计 1–2/255 | `blend.rs:74` |

### 2.2 保真差距实测与归因

`--flat` 对比（我们 `dump-flat` 导出的重建图 vs PSD 里的合并图）：

```
MAE(最大通道)=10.667  p99=219  max=255
差>8 的像素 14.76%   差>32 的像素 9.24%
alpha MAE=0.000

最差 8 个 32px 块:
   114.61  ( 160,1312)
   101.44  ( 160,1376)
   100.16  (  32,1376)
    96.05  (  64,1376)
    ...
```

对 `(160,1312)` 做图层归因：该块内堆了 **8 个 clip 层**（含 `LINEAR_DODGE op=64`、`MULTIPLY`、`ColorOverlay` 的智能对象）+ **3 个带蒙版层** + 多层 `InnerShadow/Stroke/DropShadow`，还有 1 个 `COLOR` 混合的像素层。

→ 结论明确：**先把剪贴与蒙版做对，MAE 才会掉一个量级**；样式/调整层的近似是第二梯队（预计 1–3/255 量级，需要实测确认）。

### 2.3 性能实测（编辑态每帧的成本）

| 环节 | 耗时 | 备注 |
| --- | --- | --- |
| `Document::flatten`（617 层，全幅） | **124–125 ms** | 工作 ∝ 图层面积和 41.9 Mpx |
| 预览合成（383×868） | **93 ms** | 缩放后仍按层叠加，没省多少 |
| BMP 编码 | 4 ms | 每帧整幅 5.1 MB |
| 写盘 + QML 重新加载（`write` 字段） | **~130 ms** | 每帧一个唯一文件名，环形保留 |
| 首帧（装配 617 层 + 775 个样式烘焙） | 3.25 s | 只在打开时付一次 |

也就是说：**现在一次编辑 ≈ 258 ms/帧**（124 合成 + 4 编码 + 130 写读），约 **3.9 fps**。这个量级下"编辑态保真"无从谈起 —— 先得把"每帧全幅"变成"每帧脏块"。

---

## 3. 编辑态保真架构：保真底图 + 瓦片账本 + 脏块重合成

### 3.1 三条不变式

1. **保真不变式**：任何没有被本次编辑触及的像素，必须逐字节等于 PSD 自带合并图。
2. **最小不变式**：一次编辑的计算量 ∝ 脏区域面积 × 该区域内的图层数，**与文档总层数无关**。
3. **可解释不变式**：任何一块像素从"保真"变成"重建"都要能说出来（哪次编辑、涉及哪些特性、重建后与 PS 差多少）。

### 3.2 数据模型

新增 `crates/core/src/fidelity.rs`（core 不依赖 Qt/image 之外的东西；`RgbaImage` 已在用）：

```rust
pub const TILE: u32 = 64;              // 实测成本与块数的最佳折中（见 §3.5）

pub struct TileGrid { cols: u32, rows: u32 }

pub enum TileState {
    /// 仍是 PSD 合并图的原始像素（逐字节保真）
    Pristine,
    /// 被本编辑器重合成过；epoch = 最后一次重合成的编辑代数
    Rebuilt { epoch: u64 },
}

pub struct FidelityView {
    grid: TileGrid,
    /// PSD 自带的合并图（Document::from_image，1 层）。只读共享。
    base: Arc<RgbaImage>,
    /// 工作合成：初值 = base；被编辑过的瓦片换成自己的 Arc（COW）
    tiles: Vec<Arc<RgbaImage>>,
    state: Vec<TileState>,
    /// 瓦片 → 与该瓦片相交的图层下标（按 z 序，加载时建一次）
    index: Vec<Vec<u16>>,
    /// 瓦片 → 最高的「完全遮挡下方」的层下标（可选，P2）
    occluder: Vec<Option<u16>>,
}
```

关键点：

- `base` 直接复用现有 `Calibration.reference`（保真态 Document 的 layers[0]），**不额外解一次文件**；
- 自动化（任何 `--` 参数）走的是 `load_path_sync`：它同步装配完整分层栈、**把合并图丢掉了**
  （`fidelityMode`/`hasPsdLayers` 全置 false）。瓦片方案要在探针里可测，`Session::open` 必须
  把阶段 1 的合并图一并留下（**+5.1 MB/文档**），否则「打开 → 全瓦片 == PS」这条断言没法验；
- 未编辑瓦片不复制像素，`tiles[i]` 就是 `base` 的一个子块视图（或懒复制，见实现备注）；
- `index` 在装配时用层 bbox 建：324 个瓦片 × 平均 29 层 = 9.4k 条 `u16`，**约 19 KB**，可忽略。

### 3.3 脏矩形从哪来

现在编辑链路里完全没有"脏区"概念，`EditOp` 只有语义。补一个不侵入的方法：

```rust
impl EditOp {
    /// 这次操作会改变画布的哪一块（画布坐标，保守取并集）。
    pub fn dirty_rect(&self, doc: &Document) -> Option<Rect>;
}
```

| 操作 | 脏矩形 |
| --- | --- |
| 画笔 / 橡皮 / 涂抹 / 修复 / 修补 | 笔画 bbox（`stroke_mask` 已有 bounds） |
| 图层移动 / 缩放 / 旋转 | 变换前 bbox ∪ 变换后 bbox |
| 不透明度 / 混合模式 / 填充 / 显隐 / 锁定 | 该层 bbox |
| 新建 / 删除 / 复制 / 合并 / 重排 | 受影响图层的 bbox 并集（保守） |
| 调整（亮度/对比度/色相）面板 | 目标作用范围（整层或选区） |
| 调整层参数（未来） | 被作用层的 bbox 并集 |
| 画布裁剪 / 旋转 90° | 整幅（清空账本，回到全量重建一次） |
| 撤销 / 重做 | 该历史项自己的脏矩形（`EditOp` 直接带） |

### 3.4 重合成内核

```rust
impl FidelityView {
    /// 只重合成 rect 覆盖的瓦片。
    pub fn patch(&mut self, doc: &Document, rect: Rect) -> PatchStats {
        let mut stats = PatchStats::default();
        for tile in self.grid.tiles_of(rect) {
            let trect = self.grid.rect_of(tile, doc.canvas);   // 瓦片 ∩ 画布
            let list = &self.index[tile];

            // ① 遮挡裁剪：从最高的"完全不透明覆盖本块"的层开始
            let start = self.occluder[tile].map(|i| i as usize).unwrap_or(0);
            // ② 前缀缓存（P2）：从最近的断点开始，复用 acc 的初始内容
            let (mut acc, from) = self.resume(tile, start);

            for &i in &list[from..] {
                let layer = &doc.layers[i as usize];
                if !layer.visible || layer.opacity <= 0.0 { continue; }
                blend_layer_into(&mut acc, layer, trect);      // 蒙版/剪贴见 P0
            }

            self.tiles[tile] = Arc::new(acc.finish());        // ★只在这里量化到 u8
            self.state[tile] = TileState::Rebuilt { epoch: doc.edit_epoch };
            stats.tiles += 1;
        }
        stats
    }
}
```

三点与现有代码的差别，正是"保真"的来源：

1. **累积缓冲用 `f32`（或 `u16`）**，整块算完再量化一次。现在 `blend_pixel` 每层 `round` 到 u8，33 层堆叠下来是随机游走 1–2/255 的误差，且会随层序变化跳变。
2. **起点可跳过**：`occluder` 记录"这一块最高的不透明层"，它下面的层不必参与（photocraft 的 `occludes_below` 就是这个判据）。
3. **前缀缓存**：瓦片内按层序存若干"安全断点"的累积结果，编辑第 i 层时从 ≤ i 的最近断点恢复，只算断点之上 —— 这才是"改上层几乎零成本"。

### 3.5 成本模型（本文件实测换算）

折算基准：全幅 124 ms / 41.9 Mpx ≈ **2.96 µs / 千像素**（debug，单线程，图层 bbox 裁剪已生效）。

| tile | 块数 | 单块 p50 | 单块 p95 | 单块 max | 200×200 笔刷（4–9 块） |
| --- | --- | --- | --- | --- | --- |
| 32 | 1296 | 0.06 ms | 0.29 ms | 0.31 ms | 0.3–2.6 ms |
| **64** | **324** | **0.22 ms** | **1.16 ms** | **1.22 ms** | **0.9–10 ms** |
| 128 | 84 | 0.94 ms | 4.68 ms | 4.74 ms | 3.8–42 ms |
| 256 | 21 | 4.74 ms | 16.5 ms | 17.5 ms | 19–148 ms |

选 **64 px**：p95 单块 1.16 ms，大笔刷也在 10 ms 内；324 块的拼装是 324 次 `copy_from_slice`（合计 5.1 MB），不到 2 ms。

对照现状：同一笔刷 = 124 ms 合成 + 130 ms 落盘 ≈ **258 ms/帧**（约 3.9 fps）→ 瓦片方案 ≈ **10 ms/帧**（含拼装），**20× 以上**，且未编辑区域完全不参与。

内存增量：`base` 5.1 MB（已在）+ 工作瓦片 5.1 MB + 账本/index 约 0.2 MB ⇒ **+10 MB 量级**。

### 3.6 保真护栏

1. **接缝门限**：脏瓦片重建后与 PS 合并图逐块比对，`MAE ≤ 2 且 p99 ≤ 8` 才算"可用"。超过就打 `Risk` 标记（见下）。
2. **Risk 标记**：块里含当前实现无法精确复现的特性（P0 之前：任何 clip/mask 层；P0 之后：斜面浮雕/图案叠加/未支持调整层）时，该块记 `Risk`。用户在**没编辑**它时看到的仍是 PS 像素（完美保真）；一旦编辑命中，状态栏提示"该区域含 X 特性，已按近似重建"。
3. **回到保真**：撤销到打开时的状态（`edit_epoch == 0` 且历史为空）时，所有瓦片复位 `Pristine`，画面重新逐字节等于 PS —— 补上现在 calibration "sticky 失效后永不回来"的遗憾。
4. **接缝自检探针**：编辑命中瓦片后，检查其**相邻未编辑瓦片**边界 1px 带的差值，> 3/255 就报告（说明合成器有系统性偏差，不是内容差异）。

### 3.7 与现有 calibration 的关系

现在的 `Calibration`（`crates/ui/src/bridge/session.rs:723`）做的是"整幅二选一"：未编辑 → 整幅 PS 合并图；一旦编辑 → 整幅换成重建（sticky）。本方案是把它**降级成"逐瓦片选择"**：

| | 现在 | 改造后 |
| --- | --- | --- |
| 未编辑 | 整幅 PS 像素 | 未编辑瓦片 = PS 像素（其余 = 重建） |
| 第一次编辑 | 整幅重建（sticky，不可逆） | 只有命中瓦片重建 |
| 撤销回原位 | 仍显示重建（sticky） | 复位成 PS 像素 |
| 显示成本 | 1 层 / 8 ms | 未编辑区 1 层；编辑区 O(块) |

`Calibration.reference` 继续存在，作为 `FidelityView.base` 的来源，改动是"从整幅开关变成瓦片账本"，不推翻现有语义。

---

## 4. 分阶段实施

### P0 — 合成语义补齐（保真的前提，必须先做）

| 项 | 改动 |
| --- | --- |
| 读图层蒙版 | `crates/decode/src/psd_layers.rs`：取 `PsdChannelKind::UserMask`（-2）与 `RealMask`（-3），按**蒙版自己的矩形**（`mask_len` 里已解析出行数）展开；默认值来自蒙版块的 `background_color`（黑=0 隐藏 / 白=255 显示）。写进 `PsdLayerData.mask` |
| 读剪贴标记 | 同上：`layer.is_clipping_mask()` 落到 `PsdLayerData.clipping`（现在只用于调整层作用域） |
| 文档模型 | `crates/core/src/layer.rs` 的 `Layer` 增加 `mask: Option<LayerMask { image, offset, enabled, default }>` 与 `clipping: bool`；`LayerGroup` 增加 `mask` / `opacity` / `blend`（后两个本文件为默认值） |
| 合成 | `crates/core/src/document/mod.rs::flatten`：① 每层 alpha ×= 蒙版取样；② 连续 `clipping` 层与其 base 组成"剪贴单元"，单元内先按 Normal 叠进一个临时缓冲，再乘 base 的 alpha，最后按 base 的混合模式/不透明度整体合成；③ 组区间内先合成再把组蒙版乘上去；④ 累积缓冲 f32，最后量化一次 |
| 量化 | `crates/core/src/blend.rs` 增加 `blend_pixel_f32`（不 round），`flatten` / `FidelityView` 走它 |
| 验收 | `psd_fidelity_report.py --flat`：MAE ≤ 2、差>8 像素 < 1%、最差块 < 12；`cargo test -p imageedit-core` 补蒙版/剪贴单测（复制自 PSD 边界语义）；`probe_psd_roundtrip.py` 不变绿不能提交 |
| 风险 | 剪贴单元跨在"调整层烘焙"逻辑上，需要统一成"先按 z 序建单元，再烘焙" |

预计收益：把 10.67 的 MAE 打到 2 以内（本文件 305 个受影响的可见层全部落地）。这也是唯一能让"接缝门限"成立的一步。

### P1 — 瓦片账本 + 脏块重合成（性能）

| 项 | 改动 |
| --- | --- |
| 新模块 | `crates/core/src/fidelity.rs`（`TileGrid` / `TileState` / `FidelityView` / `PatchStats`） |
| 脏矩形 | `crates/core/src/op.rs`：`EditOp::dirty_rect(&Document)`；`Document::execute/undo/redo` 后把 rect 累积到 `Document.dirty: Option<Rect>`（`take_dirty()` 取走） |
| 渲染接线 | `crates/ui/src/bridge/session.rs`：`Session` 持有 `FidelityView`；`refresh_preview` 改成「`take_dirty()` 有值 → `patch()` + 只重编码受影响预览区域」，无值 → 直接用上一帧 |
| 拼装 | `FidelityView::image()` 返回整幅（未编辑瓦片 = base 子块）；预览缩放仍复用 `render::downscale_box`（不要改用 thumbnail，见旧坑） |
| 验收 | 新探针 `scripts/probe_fidelity_tiles.py`：① 打开后全瓦片逐字节 == PS 合并图；② 画一笔只有相交瓦片变化；③ 撤销后全瓦片回到 PS 像素；④ 每瓦片 MAE ≤ 2；⑤ 单帧 patch 时间 p95 ≤ 5 ms（debug） |
| 风险 | `EditOp::dirty_rect` 漏报 → 画面残留。兜底：所有 op 默认返回 `Some(整幅)`，只对已测过的 op 收紧；这样漏报退化成现在的行为，不会错 |

### P2 — 前缀缓存 / 遮挡裁剪（大文件才需要）

- 每瓦片存"完全不透明断点"：`occluder[tile]` = 最高的一层 `visible && opacity=1 && blend=Normal && 无蒙版/样式/剪贴 && 不透明覆盖整块`（photocraft `compose::occludes_below`）。本文件有 7 个层覆盖 > 50% 画布，背景层不透明 ⇒ 大部分瓦片能砍掉下面几十层。
- 每瓦片按层序存 1–2 个累积断点（`Vec<(u16, Arc<RgbaImage>)>`，只对"改上层"的高频操作有价值）。参考 `photocraft/docs/gpu-composite-reuse.md` 的"chunk grid + 前缀复用 + 失效判定"。
- 验收：在本文件上，**只改最上层**时单块 patch ≤ 0.2 ms；`probe_psd_perf.py --check` 的门槛全部不变。

### P3 — 预览链路去落盘（现在每帧 130 ms 在这）

现状：每帧把整幅编码成 BMP（5.1 MB）写临时文件，QML `Image.source` 换 URL 重新解码（`session.rs:1170-1200`）。改造：

- 优先：`QQuickImageProvider` 直接给 `QImage`（零落盘、零解码），或 `QQuickWindow::updateTexture` 类共享内存路径；
- 次选：只对被编辑的矩形区域重编码，QML 侧用「底层整幅图 + 若干瓦片图层」叠加（改动小，但要处理双缓冲一致性）；
- 目标：单帧提交 ≤ 15 ms（debug），2560×1440 预览不掉帧。

### P4 — 通用特性补齐（见 §5，按需排序）

---

## 5. 通用特性补齐清单（本文件之外）

「能不能开」和「开得像不像」是两件事。下表按"通用价值 / 实现成本"排序，`现状` 指本项目：

| 特性 | 现状 | 优先级 | 说明 |
| --- | --- | --- | --- |
| 图层蒙版（含羽化/密度/黑底默认值） | ❌ 未读 | **P0** | 通用水印、渐变蒙版全靠它 |
| 剪贴蒙版（base + clipping 链） | ❌ 丢弃 | **P0** | PS 最常用的"上色"手法 |
| 组蒙版 / 组不透明度 / 组混合模式 | ❌ 组透明度和混合未建模 | P1 | 本文件 197 组全穿透，通用稿常见 |
| 调整层（Curves/Levels/ColorBalance/BlackWhite/GradientMap/PhotoFilter/ChannelMixer/Invert/Posterize/Threshold/Exposure/Vibrance/SelectiveColor/ColorLookup） | ❌ 只烘焙 B/C + H/S | P1 | 做成分级：先支持"作用到合成后"的正确语义，再逐个补公式 |
| 调整层作用于"合成结果"而非逐层 | ❌ 逐层烘焙 | P1 | 非剪贴调整层必须作用于下面的合成，否则整幅偏色 |
| 图层样式 live（不烘焙） | ❌ 烘焙 | P2 | 编辑性 + 可撤销；烘焙只能保显示 |
| 样式缺项：斜面和浮雕、图案叠加、缎光 | ❌ 跳过 | P2 | 若做烘焙，先补这三类的几何 |
| 每效果属性：混合模式、等高线、杂色、渐变描边、内阴影"图层挖空" | ❌ | P2 | 决定样式"像不像"的细节 |
| 16 bit / 32 bit 通道 | ❌ 明确拒绝（报错文案友好） | P2 | 摄影稿常见 16 bit |
| CMYK / Lab / 灰度 / 索引色 | ❌ 拒绝 | P3 | 印刷稿；参考 photocraft 的 `compose/src/cmyk.rs` 原生墨水路径 |
| Blend If / 填充不透明度 / 挖空 / 通道限制 | ❌ | P2 | "高级混合"整组缺失 |
| 混合模式补全（VividLight/LinearLight/PinLight/HardMix/Subtract/Divide/DarkerColor/LighterColor） | ❌ 18 种 / PS 约 30 种 | P2 | 公式短，补齐成本低 |
| ICC 色彩管理 | ❌ 忽略 | P3 | 本文件无 ICC；广色域稿会偏 |
| 智能对象（替换/重渲染/智能滤镜） | ❌ 只用栅格 | P4 | 需要把内嵌 PSD 当子文档 |
| 文字引擎（重排/编辑） | ❌ 只用栅格 | P4 | 与 `crates/text` 之类的工作量级 |
| 矢量路径编辑 | ❌ 只用栅格 | P4 | 形状层的 234 个矢量蒙版 |
| Layer Comps / 艺术板 / 时间轴 / 3D / Alpha 通道 / 专色 | ❌ | P4 | 按真实需求再说 |

---

## 6. 参考实现索引（Photocraft）

> PhotoCraft 是 MIT OR Apache-2.0 的纯 Rust PS 重实现（约 53.9 万行）。**只借鉴行为与结构，不拷代码**；若真要复制片段，需按其许可证在 `NOTICE` 里署名。

| 我们要做的事 | 参考位置 | 可借鉴的要点 |
| --- | --- | --- |
| 蒙版合成 | `crates/compose/src/masks.rs::combined_mask` + `crates/doc/src/lib.rs:248 LayerMask{surface, enabled, linked, density, feather}` | 蒙版是**灰度 Surface**，`value()` 里把 density 与 feather 一起算；`has_feather` 决定要不要走慢路径 |
| 剪贴蒙版 | `crates/compose/src/composite.rs::composite_layer`（`while layers[j].clipped` 收集剪贴组）+ `advanced::clips_individually` | 剪贴组从 base 向上连续收集；"把剪贴层当组"与"每层各自混"是**两个开关**（PS 的「将剪贴图层作为组混合」） |
| 穿透组 | `crates/compose/src/composite.rs` 的组分支 + `cmyk.rs:150` | 穿透组不建立隔离缓冲，直接混到下层 backdrop；非穿透组才起临时缓冲 |
| PS 精确混合 | `crates/compose/src/psblend.rs::composite` / `composite_fill`（f32 + gamma + fill） | 混合在 **f32** 里做，`fill opacity` 参与 source alpha，gamma 显式传入 |
| 层级合成入口 | `crates/compose/src/render.rs::composite_to_surface(doc, rect, fmt, background)` | **接口天生带 rect** —— 脏区合成是一等公民，不是后加优化 |
| 图层外接框 | `crates/compose/src/extents.rs::composite_bounds(layer, canvas)` | 每层合成范围（含样式外扩）先算好，避免整幅遍历 |
| 遮挡裁剪 | `crates/compose/src/composite.rs::occludes_below` | 判据清单：Normal + 100% 不透明 + 无蒙版/样式/剪贴/通道限制/Blend If + fill 不透明 |
| 脏块缓存与前缀复用 | `docs/gpu-composite-reuse.md` + `crates/gpu/src/plan.rs` | ① 脏矩形对齐到 chunk 网格，缓存按网格位置键；② 只在前缀"安全边界"（base + 剪贴链结束）设断点；③ 失效靠 COW 快照结构比较，不靠指纹 |
| 不可逆降级 | `crates/compose/src/native.rs`（模式原生合成）+ GPU planner 返回 `Unsupported` → CPU 兜底 | 做不到就明确回退到 CPU 路径，而不是静默出错 |
| 调整层 | `crates/compose/src/adjust/{tone,color,channels}.rs` + `adjustment_quantum(depth)` | 调整按位深量化（8 bit 文档里 PS 也是 8 bit 步进） |
| 图层样式 | `crates/compose/src/effects/`（`composite_with_effects` + `FxMaps`） | 样式是**合成期**的实时效果，不是烘焙；`FxMaps` 缓存距离场/模糊结果 |
| 瓦片与快照 | `docs/architecture.md` §5（256² 瓦片、`Arc` 共享、COW 快照） | 撤销/后台任务/UI 读取都靠快照，不做整幅复制 |
| 色彩管理 | `crates/cms` | 自带 ICC 引擎；显示用矩阵/TRC 或 3D LUT |

---

## 7. 不承诺的边界

- **不承诺"任何 PSD 都逐像素一致"**。能做到的是：① 没被编辑的像素逐字节等于 PS；② 被编辑的瓦片与 PS 的差在门限内（P0 之后 MAE ≤ 2）；③ 做不到时明确告诉你哪块、因为什么特性。
- **不把"重算"当成保真手段**。PS 的排版/矢量/智能对象渲染是闭源引擎，追平它没有终点；用合并图当锚点才是有限成本。
- **不做无损往返的假承诺**：写回 PSD 时样式/调整层已经被烘焙（这是现状的既定策略，doc `PSD分层可编辑-B方案.md` §19.6 已列），改造后依然如此，直到 P4 做 live 样式。
- **性能数字是 debug 构建**。release 会快 3–5×，但**相对关系**（脏块 vs 全幅）不变，验收门槛按 debug 定，留余量。

---

## 附录 A：复现本文所有数字

```powershell
# 1) 特性画像 + 瓦片成本（不依赖本项目）
python scripts/psd_fidelity_report.py "D:\_APP_CODE\_XhCompany\FontGame\Res\0302-界面UI-1.psd"

# 2) 导出本项目的重建合成（写 tmp/flat.png）
.\target\debug\imageedit.exe "D:\...\0302-界面UI-1.psd" --probe-layers --script=dump-flat

# 3) 重建 vs PS 合并图的差距 + 最差块归因
python scripts/psd_fidelity_report.py "D:\...\0302-界面UI-1.psd" --flat tmp/flat.png --attribute 3

# 4) 单帧合成耗时（看 render-profile 的 composite 字段）
.\target\debug\imageedit.exe "D:\...\0302-界面UI-1.psd" --probe-layers
```

`render-profile` 三个字段：`composite`（合成）、`encode`（BMP 编码）、`write`（落盘 + 交给 QML 的总耗时）。

## 附录 B：一句话验收标准

| 阶段 | 验收 |
| --- | --- |
| P0 | `--flat` 对比 MAE ≤ 2、差>8 像素 < 1%；蒙版/剪贴单测覆盖 4 个边界（黑底蒙版、蒙版自适应矩形、剪贴 3 层链、穿组剪贴） |
| P1 | 打开后全瓦片逐字节 == PS；一笔只有相交瓦片变；撤销全复原；patch p95 ≤ 5 ms |
| P2 | 只改最上层时 patch ≤ 0.2 ms；`probe_psd_perf.py --check` 全绿 |
| P3 | 单帧提交 ≤ 15 ms（debug），无临时文件增长 |
| P4 | 按特性逐条给"读得进 / 画得像 / 能写回"三档结论 |
