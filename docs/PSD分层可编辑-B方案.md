# PSD 分层可编辑（B 方案）实施方案

状态：**设计已定稿（用户 2026-11-12 拍板「图层后台加载期间禁用全部编辑工具」），尚未写任何实现代码** ｜ 2026-11-12
影响范围：`decode` / `core` / `ui-bridge` / `QML` / 新增 `encode`；`thumbnail` **不改**
前置：统一图层模型已全量落地（`0.1.1`，commit `aaf1f23`），291 项测试全绿
关联：三档评估 A（拼合写回）/ **B（分层可编辑）** / C（结构保真）；本文档只展开 B

---

## 0. 一页结论

**B 档 = 打开 PSD 还原成多个可编辑图层 → 用现有工具改 → Ctrl+S 写回 PSD，图层仍在。**

四个环节里只有中间一层能直接复用：

| 环节 | 现状 | B 档改动 |
|---|---|---|
| ① 解码 | 只取合成图，1 张 `RgbaImage` | **新增分层解析**，吐出 N 层（名 / 可见 / 不透明度 / 位置 / 位图） |
| ② 文档 | 统一 `layers` 列表、撤销、面板、批量命令 | **可直接复用**，只需新增一个「从层集合装配文档」的入口 |
| ③ 编辑 | 全部工具已就绪 | **零改动** |
| ④ 保存 | 4 道拦截明确禁止写 `.psd` | **从零写 PSD 编码器** + 解除拦截 + 改保存格式枚举 |

> 读侧不用重造：vendored `psd 0.3.5` 已能解层名、可见性、不透明度、位置、逐层通道、混合模式、组；
> 但**写侧没有现成实现**——`image 0.25` 无 PSD 编码器，该 crate 的序列化作者明确未做。

**性能结论（详细见 §6）**：进程启动和窗口出现 **0 额外开销**（解析代码是被动调用的纯函数，首图解码本来就在 `main()` 里丢到后台线程）；非 PSD 文件零影响；唯一受影响的是「打开 PSD」，靠**两阶段渐进加载**把「画面出现」时间也保持不变，代价只是「图层可编辑」晚几秒就绪。这不靠运气，是靠 §6 的 8 条硬约束 + 可测门槛保证的。

**★ 用户定稿的交互约束**：阶段 2（图层后台加载）**完成前，不允许使用任何编辑工具**——不是「允许编辑再事后守卫」，而是从入口禁用。查看 / 缩放 / 平移 / 打开文件仍然可用；加载完成、失败或用户主动「跳过图层加载」时立即解锁。详见 §3.1 与 §6.2.1。

**工作量**：12~17 人天（原估 10~15，性能专项 +2），全职约 2.5~3.5 周。

---

## 1. 现状：PSD 是「只读预览」格式

### 1.1 已有能力

| 位置 | 能力 |
|---|---|
| `crates/decode/src/lib.rs::decode_psd_bytes` | 解析 PSD，返回**合并合成图**（`Psd::rgba()`），带 8bit / RGB·灰度 / 通道数校验、`catch_unwind` 兜底、`validate_psd_sections` 段边界预校验 |
| `vendor/psd`（`[patch.crates-io]` 指向本地 fork） | `Psd::layers()` / `groups()` / `PsdLayer::{name, visible, opacity, layer_top/left/bottom/right, blend_mode, parent_id, is_clipping_mask, rgba}` / `flatten_layers_rgba()` / `BlendMode` 约 28 种 |
| `vendor/psd/tests/fixtures/*.psd` | 20 份现成样本：RLE、3 层、负偏移、越界层、裁剪蒙版、组、中文层名、非 UTF-8 名等 |
| `crates/thumbnail` | Explorer 缩略图 COM DLL，**直接复用** `decode_psd_bytes`（`MAX_DECODE_EDGE=1024`） |
| `crates/core` | 统一 `layers` 列表 / `EditOp::Batch` / 撤销 / 多选 / 合并 / 对齐 / 栅格化 |
| `decode::encode` | PNG/JPEG/WebP/TIFF/BMP/GIF/TGA/QOI/ICO，**无 PSD** |

### 1.2 五处写回拦截（B 档要逐个解除，不能盲删）

| # | 位置 | 行为 |
|---|---|---|
| 1 | `bridge/paths.rs::save_format_for_source` | `.psd` 源强制回落 `Format::Png` |
| 2 | `bridge/session.rs::default_save_path` | 另存为默认名 `<stem>-edited.png` |
| 3 | `bridge/session.rs::save` | Ctrl+S 直接返回 `PSD_SAVE_HINT` 错误 |
| 4 | `bridge/ops/document.rs::save_as` | 手打 `.psd` 路径同样拒绝 |
| 5 | `qml/editor/EditorContent.qml::sourceIsPsd` + `PsdHintOverlay.qml` | 保存时弹「PSD 不能直接保存」浮层拦在 UI 之前 |

> ⚠️ **历史 bug 不能复发**：`image 0.25` 的扩展名映射对 `.psd` 返回 `None`，旧代码
> `format_from_path(path).unwrap_or(Format::Png)` 曾把 **PNG 字节写进 `.psd` 文件**。
> 所以解除拦截必须和新编码器**同一次提交**落地，且 `write_to` 对未知扩展名仍保持显式兜底，
> 不能改成「删掉判断让 `unwrap_or` 自己走」。

### 1.3 打开链路（现状时序）

```mermaid
flowchart LR
  A[main / argv] --> B[prewarm::spawn 后台线程]
  B --> C[decode_path → decode_psd_bytes]
  C --> D[Decoded 合成图]
  D --> E[poll_decode 等 viewport]
  E --> F[Session::from_decoded → Document::from_image]
  F --> G[1 个背景层 · 状态栏提示「图层已合并」]
```

`Document::from_image()` 只造 `layers = vec![Layer::background(image)]`，**所有 PSD 层信息在解码那一步就丢了**。

---

## 2. 目标与边界

### 2.1 做到

- 打开 `.psd` 还原成 N 个图层：层名、`visible`、`opacity(0..1)`、画布相对位置（含负偏移 / 越界裁剪）
- 用现有全部工具编辑这几层（移动 / 变换 / 选区 / 调色 / 绘制 / 合并 / 对齐 / 显隐 / 锁定 / 排序 / 撤销）
- `Ctrl+S` 覆盖写回原 `.psd`、`另存为 PSD`；Photoshop / Krita / GIMP / 本软件 / Explorer 缩略图都能正常打开
- 明确的降级提示（见 §7），**不静默改坏别人的 PSD**

### 2.2 明确不做（留给 C 档）

混合模式（非 Normal）、嵌套图层组 / 分组结构、裁剪蒙版、图层蒙版、调整层、可编辑文本层、16/32bit、CMYK / Lab / 索引色、智能对象、图层效果、切片、路径。

> 这正好对齐《图层系统重构-设计确认》里拍板的「图层组、混合模式不做」——
> **B 档不扩 `Layer` 模型**，不碰 `EditOp` / 快照 / 拖拽排序 / 脚本探针，这是它能落在 2~3 周的前提。

---

## 3. 总体架构：两阶段渐进加载

**核心决策：不改 `Decoded`、不改 `decode_psd_bytes`、不碰 `thumbnail`。**

新开一个独立的分层解析函数，由第二阶段后台线程调用；首帧仍走现有合成图路径，两者并行错开。

```mermaid
flowchart TD
  A[main: prewarm::spawn] --> B[阶段1 后台线程]
  B --> C["decode_path → 合成图 (与现在完全一致)"]
  C --> D["poll_decode → 首帧画面 + editingLocked=true"]
  D --> L["全部编辑工具禁用 · 只可查看/缩放/平移 · 显示加载进度"]
  B --> E[阶段2 后台线程: decode_psd_layers]
  E --> F[PsdLayerSet: N 层位图 + 属性 + 降级清单]
  F --> G[poll_psd_layers 50ms 轮询]
  G --> H["install_psd_layers: 替换 doc.layers + 清历史"]
  H --> K["editingLocked=false · 面板出现 N 层 · 可编辑 · Ctrl+S 可写回"]
  G -.失败 / 跳过.-> M["保持合并预览 + 立即解锁"]
  L -.用户点「跳过图层加载」.-> M
```

**为什么首帧不直接等分层解完**：`PsdLayer::rgba()` 按画布尺寸分配，10 层 4000×3000 朴素实现是 480MB + 120M 次像素循环；同步等待会让「打开 PSD」明显变慢甚至内存吃紧。渐进方案让画面出现时间**与现状完全一致**（PSD 通常自带合成图）。

**阶段 1 与阶段 2 的接线**（`DecodeMsg`，`bridge/session.rs:311`）：

```rust
pub(in crate::bridge) struct DecodeMsg {
    pub path: PathBuf,
    pub result: Result<(Decoded, i32), String>,
    /// 仅 PSD 且开启分层时 Some；阶段 2 线程在阶段 1 发送消息前就已启动。
    pub layer_job: Option<mpsc::Receiver<Result<PsdLayerSet, String>>>,
}
```

- `poll_decode` 落地首帧后把 `layer_job` 存进 `Bridge` 的 `RefCell`；
- QML 已有 50ms 轮询（后台解码 worker），挂一个 `poll_psd_layers()` 取结果，不新增 Timer；
- **`Session::open()`（自动化 / `--script` / 探针的同步路径）必须 `recv()` 等到层解析完**，保证脚本拿到的文档是分层的，否则所有既有探针的图层断言失效；
- 单实例转交、打开第二张图时，未完成的 `layer_job` 要丢弃（drop receiver 即可）。

### 3.1 编辑锁：加载完成前禁用全部编辑工具（★ 用户定稿）

**用户明确要求：阶段 2 未完成时不允许使用任何编辑工具。** 所以 B 档**不采用**
「先允许编辑、回来时靠 `edit_epoch / dirty` 放弃分层」的竞态守卫方案——那会让用户
白做几笔再被提示重开。改为**在入口禁用编辑**，竞态从根上不存在；epoch 检查只保留为
防御性兜底（脚本 op / 外部命令仍可能绕过 UI 写文档）。

#### 3.1.1 状态与生命周期

新增桥属性 `editingLocked`（`qproperty bool`，写进 `bridge/mod.rs` 的 cxx 声明块）。

| 时机 | `editingLocked` | 说明 |
|---|---|---|
| 首帧落地，且 `SourceFormat::Psd` + 开启分层 | **true** | 立刻上锁，与首帧同一次 `poll_decode` 发布 |
| 阶段 2 成功 `install_psd_layers` | **false** | 装配 + 清历史 + 刷新面板后解锁 |
| 阶段 2 失败 / 解析降级 / 无合成图兜底失败 | **false** | 保持合并预览，**必须解锁**，否则用户什么都干不了 |
| 超限只读（>8192，C8）/ 用户点「跳过图层加载」 | **false** | 保持合并预览，按普通文档编辑（写回 PSD 仍禁） |
| 打开另一张图 / 单实例转交 / 窗口关闭 | **false** | 丢弃未完成的 `layer_job` 时同步解锁，防永久锁死 |
| 同步路径（`--script=` / `--selftest` / 探针） | **恒 false** | `Session::open()` 会 `recv()` 等到层解析完成才返回，探针拿到的文档已是分层的，不应被锁影响 |

#### 3.1.2 禁止什么、放行什么

- **禁止**：全部绘制 / 选区 / 变换 / 修复 / 调色工具；图层面板所有命令按钮与右键菜单项
  （新建、复制、合并选中、删除、上下移、显隐、锁定、不透明度、对齐分布、栅格化）；
  工具快捷键、Delete、Ctrl+Z/Y、Ctrl+C/X/V；Ctrl+S 写回 PSD；数值面板 / 浮层输入框。
- **放行**（不属于「编辑工具」）：查看、滚轮缩放、平移、窗口缩放、打开文件 / 另开窗口、
  浏览菜单、关闭程序。文件菜单里「打开」不锁，「保存 / 另存为」在锁期间置灰。
- 锁期间状态栏与图层面板显示「正在加载 N 个图层…（完成前编辑工具暂不可用）」，
  并给一个**逃生按钮**「跳过图层加载，以合并预览继续编辑」——避免大文件把用户锁太久。

#### 3.1.3 QML 落点与已知坑

| 落点 | 做法 |
|---|---|
| `qml/editor/EditorContent.qml` | 所有编辑类 `Shortcut` 的 `enabled` 叠加 `&& !bridge.editingLocked`；工具按钮 `enabled` 同理 |
| `qml/panels/CanvasSurface.qml` | **不要**把 MouseArea 整个 `enabled: false`（会连滚轮缩放 / 平移一起吞掉）。在 `onPressed` / 变换手柄按下分支里判断：导航事件放行，落笔 / 拾取 / 拖柄直接忽略并给一次性状态栏提示 |
| `qml/panels/LayerPanel.qml` | 每个 `PanelButton` 的 `usable` 叠 `!editingLocked`；`MenuRow.active` 同理；标题行显示加载进度 |
| 右键 / 浮层 | 锁期间不开任何带 `TextInput` 的浮层（焦点会静默废掉 Delete / 工具键，见既有坑）；已有 `dropPopoverFocus()` 复用 |

★ 三条必须记住的既有坑：
1. `Shortcut.enabled` 引用**同文件后面才实例化**的 id → 创建期 TypeError → 绑定丢失、
   `enabled` 永久 false。`editingLocked` 只依赖 bridge 属性，但仍沿用已有的 `toolKeysReady`
   短路，别引后置 id。
2. qproperty 的 QML 名规则：带 `cxx_name=` 用驼峰，不带的按 snake_case；写错是
   `undefined` 静默失效。新增属性必须进 cxx 块，改完跑 `scripts/check_bridge_api.py`。
3. 新增 QML 文件必须加进 `crates/ui/build.rs` 的 `.qml_files([...])`。

#### 3.1.4 装配落地（成功路径）

替换 `doc.layers` 后：**重置历史栈**（加载不是一次编辑，不该能 Ctrl+Z 撤回到 1 层状态）、
保留 `zoom / fit / viewport` 避免窗口跳动、刷新图层面板与缩略图、发布降级说明、
最后置 `editingLocked = false`。落地前仍做一次 `edit_epoch / dirty` 断言：若被脚本改过，
放弃分层、保留合并预览、解锁并提示重新打开（防御路径，不是主路径）。

---

## 4. 数据结构与接口

### 4.1 新模块 `crates/decode/src/psd_layers.rs`

```rust
/// 一层 PSD 的「可编辑」表示。位图已裁到图层矩形，不是整张画布。
pub struct PsdLayerData {
    pub name: String,
    pub offset: (i32, i32),       // 画布相对，可为负
    pub image: RgbaImage,         // 只占 (right-left) × (bottom-top)
    pub visible: bool,
    pub opacity: f32,             // 0..1
    pub degraded: Vec<String>,   // "混合模式 Multiply 已按 Normal 处理" 等
}

pub struct PsdLayerSet {
    pub layers: Vec<PsdLayerData>,  // 底 → 顶，与 doc.layers 顺序一致
    pub canvas: (u32, u32),
    pub degraded: Vec<String>,      // 文件级降级（组被展平等）
    pub has_composite: bool,        // 是否自带合成图（决定首帧来源）
}

/// 只解析层，不碰 Decoded / thumbnail 路径。
pub fn decode_psd_layers(bytes: &[u8], options: DecodeOptions)
    -> Result<PsdLayerSet, DecodeError>;
```

**为什么不用 `PsdLayer::rgba()`**：`vendor/psd/src/psd_channel.rs::generate_rgba()` 里是
`let rgba_len = (psd_width * psd_height * 4)`，即**按整张画布分配**，与图层自身面积无关，
还要对整张画布做 N 次通道插入。必须自己取通道再裁。

**通道字段是 `pub(crate)`**（`vendor/psd/.../layer.rs:191`），外部 crate 读不到。给本地 vendor fork
加一个最小公开访问器即可，不动解析逻辑：

```rust
impl PsdLayer {
    /// B 档新增：让 decode crate 按图层矩形自行展开通道，避免全画布 RGBA。
    pub fn channel_bytes(&self) -> &LayerChannels { &self.channels }
}
```

`ChannelBytes::{RawData, RleCompressed}` 已是 `pub enum`，RLE 解压逻辑仓库里已有
（`insert_channel_bytes`），按其 PackBits 语义复用，只把目标缓冲从「全画布」换成「图层矩形」。

### 4.2 文档装配 `core` 新增入口

```rust
impl Document {
    /// 从 PSD 层集合装配：layers[0] 恒为透明背景，其后依次是 PSD 层（底→顶）。
    pub fn from_psd_layers(set: PsdLayerSet) -> Self;
}
```

- **背景层用透明画布位图**，所有 PSD 层都作为普通 `LayerContent::Raster` 加入，`background` 恒只在 0 号位；
  不把 PSD 最底层「提升」成背景——这样 0 号位不变量、删除/合并/对齐语义全部不用特判，行为可预测
  （代价是面板多一行「背景」，需在 UI 文案里说明）。
- `offset` 直接写 `RasterLayer.offset`（`i32`，支持负坐标）；越过画布边界的层要按画布裁剪并同步调整 offset
  （fixture `out-of-bounds-layer.psd` 回归）。
- `visible / opacity / name` 一一映射；`locked = false`。
- 若 `max_dimension` 生效（>8192），**不装配分层**，见 §6 C8。

### 4.3 保存格式枚举

`image::ImageFormat` 没有 PSD 变体，不能直接复用现有 `Format`：

```rust
// crates/decode/src/lib.rs（或新 encode crate）
pub enum SaveFormat { Image(Format), Psd }

pub fn encode_psd(doc: &Document, quality: u8) -> Result<Vec<u8>, DecodeError>;
```

`Session::write_to(&self, path, SaveFormat)` 分派：`Psd` → `encode_psd(&self.doc, …)`，其余 → 现有 `encode(&self.doc.flatten(), …)`。

---

## 5. 分模块改动清单

| # | 文件 / 位置 | 改动 | 验收 |
|---|---|---|---|
| M1a | `vendor/psd/.../layer.rs` | 加 `channel_bytes()` 访问器 | vendor 自带测试全绿 |
| M1b | `crates/decode/src/psd_layers.rs`（新） | 按层矩形展开 R/G/B/A、PackBits 解压、越界裁剪、层名（`luni` UTF-16 优先）、属性映射、降级收集 | 单测覆盖 20 份 fixture |
| M1c | `crates/decode/src/lib.rs` | 导出 `PsdLayerSet` / `decode_psd_layers` / `SaveFormat`；**`decode_psd_bytes` 签名与行为保持不变** | `thumbnail` 编译零改动 |
| M1d | `validate_psd_sections` | 现在只校验到 image data 段长度；分层解析要深入 layer record / channel 长度，补齐「每条记录的 channel data 长度、extra length、RLE 行表」越界预校验，全部包 `catch_unwind` | 损坏样本返回 `DecodeError`，不 panic |
| M2 | `crates/core/src/document/mod.rs` | `Document::from_psd_layers()`；不扩 `Layer` 结构 | core 单测：层序、offset、opacity、0 号位不变量 |
| M3a | `bridge/session.rs::DecodeMsg` | 加 `layer_job` 字段 | — |
| M3b | `bridge/prewarm.rs` + `bridge/ops/document.rs` | 阶段 2 线程启动 / `poll_psd_layers` / `install_psd_layers` / **`editingLocked` 上下锁与全部解锁路径** / 历史栈重置 | 探针跑同步与异步两条路径；锁的生命周期用例全过 |
| M3c | `bridge/mod.rs` + `qml/editor/EditorContent.qml` / `CanvasSurface.qml` / `LayerPanel.qml` | 新增 `editingLocked` qproperty；编辑类 Shortcut / 按钮 / 画布落笔 / 右键菜单全部 gate；状态栏进度 + 「跳过图层加载」逃生按钮 | 手测：锁期间任何编辑都无响应，查看缩放平移正常 |
| M3d | `bridge/paths.rs`、`session.rs::{default_save_path, save, write_to}`、`ops/document.rs::save_as` | 解除 5 处拦截，切 `SaveFormat`，PSD 走原子写；保存入口在锁期间置灰 | 「保存后图层还在」往返测试 |
| M4 | `crates/decode/src/psd_write.rs`（新） | PSD 编码器（附录 A） | Photoshop / Krita / GIMP / 自解码交叉验证 |
| M5a | `qml/editor/EditorContent.qml` | 删 `sourceIsPsd` 分支与 `confirmSaveProceed` 里的 PSD 特判；另存为过滤器加 `Photoshop (*.psd)`；PSD 源默认后缀 `psd` | 手测 Ctrl+S / 另存为 |
| M5b | `qml/panels/PsdHintOverlay.qml` | 复用改造成「降级说明 / 图层加载中 / 超限只读」提示，**不再拦保存** | 手测 |
| M5c | `qml/panels/LayerPanel.qml` | 层数多时标题/提示行显示「正在加载图层…」「已降级 N 项」 | 手测 |
| M6 | `scripts/check_bridge_api.py` 已有 | 若新增 qinvokable（`poll_psd_layers` 走现有 Timer 则不必），跑一遍防漏声明 | PASS |
| M7 | `docs/大版本重构-手动测试清单.md` | 新增 §A26（PSD 分层专项）、B 表加 `probe_psd_roundtrip.py` / `probe_psd_perf.py` | 文档计数同步 |
| M8 | `CHANGELOG.md` | 新版本段（走 `package.ps1` 自动 patch+1） | — |

> ★ 新增 QML 文件必须加进 `crates/ui/build.rs` 的 `.qml_files([...])`，否则整窗加载失败只报 `qml-load-failed`。
> ★ 新增 `#[qinvokable]` 必须写进 `bridge/mod.rs` 的 cxx extern 块；这类漏声明**不报编译错**，只有运行时 TypeError，改完跑 `scripts/check_bridge_api.py`。

---

## 6. 性能方案 ★（独立验收门槛，不是"顺手优化"）

### 6.1 先分清两种「慢」

| 场景 | 影响 | 原因 |
|---|---|---|
| 进程启动 / 窗口出现 / 打开 PNG·JPEG | **零** | PSD 编码器是被调用的纯函数，无全局初始化；首图解码本就在 `main()` → `prewarm::spawn` 后台线程，与 Qt/QML 建窗重叠；非 PSD 连分支都不多走一次 |
| 打开 PSD → **画面出现** | 目标也是**零** | 阶段 1 完全沿用现有合成图路径，不等阶段 2 |
| 打开 PSD → **图层可编辑** | 唯一新增延迟 | 阶段 2 后台解 N 层；期间**全部编辑工具禁用**（用户定稿），只可查看/缩放/平移，并显示加载进度与「跳过」按钮 |

### 6.2 八条硬约束（写进实现，逐条验收）

| ID | 约束 | 理由 / 违反后果 |
|---|---|---|
| **C1** | `decode_psd_bytes` 的签名、返回结构、「只出合成图」语义**保持不变**；分层解析放新函数 | `crates/thumbnail` 直接调它，若默认顺手解层，Explorer 每次列目录缩略图都会多解 N 层 → 资源管理器卡死 |
| **C2** | 两阶段渐进加载；首帧绝不等待阶段 2 | 同步等分层 = 10 层大图从 0.25s 涨到 3s 才出画面 |
| **C3** | 层位图**按图层矩形分配**，禁用 `PsdLayer::rgba()` | 4000×3000 画布 `vec![0; w*h*4]` = 48MB/层，10 层 480MB；裁剪到 10% 面积后约 48MB 总量，差 10 倍 |
| **C4** | `Psd::flatten_layers_rgba()` **只在合成图缺失/损坏时兜底** | 它逐像素递归向下混合、且源码 TODO 注明未处理 blend mode，是 O(层数×画布) 纯 Rust 循环，不能进默认路径 |
| **C5** | 解码与 PSD 编码都在后台线程，`catch_unwind` 全程保留 | 大 PSD 编码可能 1~2s；`panic=unwind` 是 release 硬约束，损坏文件不能拖崩主程序（更不能拖崩 explorer.exe） |
| **C6** | 缩略图改**按需懒生成 / 分批** | `bridge/ops/layer_cmds.rs::layer_thumb_url(i)` 现在在 **UI 线程同步**给每行生成 40px PNG；100 层会因 QML 逐行调用卡一下。改成只对可见行生成、一次 ≤8 行，生成完 `layerEpoch` 通知刷新（缓存 key `(sid,lid)` 沿用） |
| **C7** | 层数上限与预览提示 | `PreviewComposer` 按 `edit_epoch` 缓存缩放位图，但 `composite_for_preview` 仍是每层一次缩放，重建耗时随层数线性增长。层数 >32 或层位图总量 >256MB 时状态栏提示，并考虑暂停实时预览重建（沿用 `gpu_live` 回退策略） |
| **C8** | `DOWNSAMPLE_EDGE = 8192` 保持；超限 PSD **只做合并预览 + 禁止写回** | 降采样打开 10000px 文件后 Ctrl+S 会把 8192px 写回原文件 = 静默丢分辨率。B 档不做层坐标按比例缩放（易出 ±1px 累积误差），直接降级并明示 |

### 6.2.1 编辑锁把「加载时长」变成了硬指标 ★

原本允许用户在阶段 2 期间先编辑，所以那几秒只是「图层晚点可用」；用户定稿改为
**加载期间禁用全部编辑工具**后，这段时间变成用户**完全不能动手**的窗口，性能要求的
性质就变了——它不再只是体验优化，而是不能把人锁死：

| 追加约束 | 内容 |
|---|---|
| **C2-a** | 锁定时长 = 阶段 2 时长，12MP / 10 层 **≤3s**（与 §6.4 同一门槛）；超过 3s 必须实时显示「已加载 k/N 层」，不能只有一句静态文字 |
| **C2-b** | 必须提供「跳过图层加载，以合并预览继续编辑」按钮，点击立即丢弃 `layer_job`、解锁；这是大文件 / 慢磁盘的逃生口 |
| **C2-c** | **任何失败路径都必须解锁**：解析错误、`catch_unwind` 捕获、无合成图兜底失败、超限只读、切换文件、单实例转交、窗口关闭。把解锁放在 `finally` 式的收尾里，不要只写在成功分支 |
| **C2-d** | 解锁后立刻刷新一次面板与快捷键 enabled 状态；QML 绑定若因属性遮蔽 / 后置 id 失效，锁会变成永久的（见 §3.1.3 的坑） |

> 因此 §6.5 的 `psd-stage2-ms` 与「锁定时长」要作为**同一个指标**采样并进
> `probe_psd_perf.py` 的硬断言，而不只是打印出来看看。

### 6.3 内存预算（示例：4000×3000 / 12MP，10 层各占 10% 面积）

| 项 | 朴素实现 | 目标实现 |
|---|---|---|
| 层位图常驻 | 10 × 48MB = **480MB** | 10 × 4.8MB = **48MB** |
| 阶段 2 峰值（含通道临时） | ~1GB | ≤ **160MB** |
| 解码像素遍历 | 10 × 12M = 120M px | 12M px（各层面积之和） |
| 合成图（阶段 1） | 48MB | 48MB（一次性，装配后释放） |
| 预览合成（800px 目标） | 10 × 0.64M | 10 × 0.64M（不变，已有 epoch 缓存） |

### 6.4 时间预算与验收门槛

| 指标 | 门槛 | 怎么测 |
|---|---|---|
| 冷启动 `window-visible` / `qml-completed`（PSD 与非 PSD） | 与基线偏差 **≤3%** | 现有 `prewarm` / `window-visible` 事件埋点对比 |
| 首帧画面出现（260×400 样本） | 现状 ~2ms，目标 **≤3ms** | `dec_bench --psd-layers` 的阶段 1 耗时 |
| 分层就绪 / **编辑锁持续时长**（4000×3000 / 10 层，release） | **≤3s**，且不阻塞 UI；>3s 必须逐层报进度、可「跳过」解锁 | `psd-stage2-ms` + `editingLocked` 上下锁事件 + 主线程帧率探针 |
| 层缩略图首次展开（100 层） | 单次 UI 线程占用 **≤16ms** | `probe_psd_perf.py` 采样 |
| PSD 编码写回（12MP / 10 层） | **≤2s**，期间窗口可拖动 | `psd-encode-ms` + 主线程响应探针 |
| 缩略图 DLL 单次 `decode_psd_bytes` | 与基线**完全一致** | 复用 `crates/thumbnail` 现有单测 + 计时断言 |
| 非 PSD 打开/预览/保存 | 所有基线用例无回归 | 现有 291 项测试 + `probe_*` 全跑 |

> 现有样本 `tmp/sample-design-raw.psd` 是 260×400 / RGB / 8bit / 312KB，实测解码约 2ms。
> 上表的秒数是按像素量外推的**目标值，不是实测**；仓库里没有大尺寸多层 PSD，必须先造样本再定最终门槛（M0）。

### 6.5 基准与回归基建（M0 先做）

1. `scripts/make_psd_fixture.py`（新）：生成 **4000×3000 / 10 层**（大层 + 小层混合）、无合成图、中文层名、负偏移、越界层、非 Normal 混合、组各一份；用 PSD 最小写入格式手写字节，不依赖 Photoshop。
2. `crates/decode/examples/dec_bench.rs` 扩展 `--psd-layers`：分别打印 `read / parse / composite / per-layer / crop / total` 与峰值内存估算。
3. 结构化事件埋点（沿用现有 `{"phase":"ui","event":...}` JSON 风格）：
   `psd-stage1-ms`、`psd-stage2-ms`、`psd-layer-count`、`psd-layer-bytes`、`psd-install-ms`、`psd-encode-ms`。
4. `scripts/probe_psd_perf.py`（新）：读事件，对上表门槛做硬性 FAIL；纳入 B 表（CLI 验证）与打包自检。
5. **先跑一次基线存盘**（当前 `0.1.1` 对同一组文件），改完逐项对比，避免「感觉没变慢」。

### 6.6 明确禁止的实现方式

- ❌ 在 `decode_psd_bytes` 里同步解完所有层再返回；
- ❌ 用 `PsdLayer::rgba()` 拿全画布 RGBA 再 `crop_imm` 裁（内存峰值已经付过了）；
- ❌ 默认调 `flatten_layers_rgba()` 生成首帧；
- ❌ 把层解析放在 UI 线程 / `Component.onCompleted`；
- ❌ 阶段 2 完成后无条件 `refresh_preview()` 全片重渲染（应复用 epoch 缓存，只重建一次）。

---

## 7. 降级与兼容契约

**原则：可以丢信息，但必须让用户知道丢了什么，且不能静默写坏原文件。**

| 遇到 | B 档处理 | 提示 |
|---|---|---|
| 非 `Normal` 混合模式 / `PassThrough` | 按 Normal + opacity 合成 | 状态栏 + 面板「已降级 N 项：混合模式」 |
| 图层组 / 嵌套组 | 展平成顺序层，组名不保留 | 「组结构已展平」 |
| 文本 / 形状 / 调整 / 智能对象层 | 取其像素通道当普通位图层 | 「特殊图层已栅格化」 |
| 图层蒙版 / 裁剪蒙版 | 忽略蒙版（像素保留） | 「蒙版未保留」 |
| 16/32bit、CMYK、Lab、索引色 | **拒绝打开分层**，保持现有合成图预览（现状已拒绝，错误文案沿用） | 现有 `DecodeError::Psd` |
| 无合成图（未勾「最大化兼容」） | 阶段 1 走 `flatten_layers_rgba()` 兜底一次 | 「此 PSD 无合成图，首帧由分层合成」 |
| 矢量标记（`LayerContent::Mark`） | 写回时先栅格化（已有入口），PSD 只存位图 | 「标记已栅格化」 |
| 层越出画布 / 负偏移 | 按画布裁剪并调整 offset | 无（几何正确即可） |
| `LayerTransform`（非破坏性缩放/旋转） | 写回时按当前显示缓存烘焙（`ensure_baked` 语义） | 无（`flatten()` 已是显示结果） |
| 文件名 `.PSD` 大小写 | 全部 `eq_ignore_ascii_case` 判定 | 现有测试已覆盖 |

**切换时的画面差异**：首帧是 PSD 自带合成图，装配后是「Normal + 各层」重新合成，两者在非 Normal 混合 / 组 / 蒙版存在时**会不一样**。必须在装配完成时给一次状态栏说明，避免被当成渲染 bug 报回来。

---

## 8. 保存语义

| 动作 | 行为 |
|---|---|
| `Ctrl+S`（源为 `.psd`） | 解除拦截 → `encode_psd(doc)` **原子写**回原路径 → `dirty=false` |
| 另存为 `.psd` | 过滤器加 `Photoshop (*.psd)`；写完后 `session.source = 新路径`（与现有行为一致） |
| 另存为其它格式 | 维持现状：`doc.flatten()` → `encode()` |
| 降采样打开的 PSD（>8192） | **拒绝写回**，提示「此文件以降采样预览打开，写回会丢失分辨率；请关闭『大图优先降采样』后重新打开」 |
| 阶段 2 未完成时保存 | **全部保存入口置灰**（编辑锁期间不允许任何写文件动作）；用户可先点「跳过图层加载」解锁，之后按合并预览另存为 PNG；写 PSD 仍禁 |
| 原子写 | 写 `<target>.tmp` → `flush` → `rename` 覆盖；失败保留原文件，绝不半途截断 |

`encode_psd` 的输入是 `&Document` 而非 `RgbaImage`：逐层输出层记录与通道，同时额外输出一份 `doc.flatten()` 作为合成图（Image Data 段），这样**没勾「最大化兼容」的阅读器也能显示**。

---

## 9. 风险与对策

| # | 风险 | 对策 |
|---|---|---|
| 1 | 盲删 4 道拦截 → PNG 字节写进 `.psd`（历史 bug） | 拦截与编码器同一次提交；`write_to` 显式 `SaveFormat` 分派；回归 `paths.rs` 现有 2 个单测并改写断言 |
| 2 | 8192 降采样 + 覆盖写回 = 静默丢分辨率 | C8：超限只读，禁止写回 |
| 3 | 无合成图的 PSD 打不开 / 黑屏 | C4 兜底 `flatten_layers_rgba()`；M0 fixture 覆盖 |
| 4 | 改 `decode_psd_bytes` 签名或默认行为 → Explorer 缩略图 DLL 回归 | C1：完全不动；新函数独立；新增解析同样包 `catch_unwind`（`panic=unwind` 硬约束） |
| 5 | vendored `psd` 解析器到处是 unchecked slice | M1d 扩展 `validate_psd_sections`；所有阶段 2 调用点 `catch_unwind`；损坏 fixture 单测 |
| 6 | 写出的 PSD 第三方读不开 | 交叉验证列为**独立里程碑**（M6），不压缩；Photoshop / Krita / GIMP / 自解码 / Explorer 缩略图五个读侧 |
| 7 | 中文层名写回变乱码 | Pascal 名走 ASCII 占位 + `8BIMluni` additional layer info（UTF-16BE）；用 `green-chinese-layer-name-1x1.psd` 往返验证 |
| 8 | 编辑锁把用户卡住（加载慢 / 失败 / 忘了解锁 / QML 绑定失效导致永久锁） | §3.1 的生命周期表 + §6.2.1 的 C2-a~d：逐层进度、「跳过」按钮、所有失败路径 finally 式解锁、同步路径恒不锁；A26 手测逐项验 |
| 9 | 层数多导致面板 / 预览卡顿 | C6 / C7 + 门槛测试 |
| 10 | 新增桥方法漏 `#[qinvokable]` 声明（历史踩过：探针全绿但按钮是死的） | 改完跑 `scripts/check_bridge_api.py`；新增 QML 文件加进 `build.rs` |

---

## 10. 测试与验收

### 10.1 单元 / 集成

- `decode`：20 份 fixture 逐份解析，断言层数 / 层名（含中文、非 UTF-8 名）/ offset（含负、越界）/ opacity / visible / 像素；损坏样本全返回 `Err` 不 panic。
- `decode::psd_write`：写→自读往返，**逐像素**比对合成图；每层位图与属性往返一致（这是 B 档最核心的一条）。
- `core::from_psd_layers`：0 号位恒背景、层序、offset 不变量、与 `flatten()` 一致性。
- `paths.rs`：把现有「PSD 回落 PNG」两个断言改成「PSD → `SaveFormat::Psd`」，保留未知扩展名兜底断言。
- `bridge`：`editingLocked` 生命周期单测——首帧上锁、成功/失败/跳过/切换文件四条路径都解锁、同步 `--script=` 路径恒不锁（防探针全废）。
- `thumbnail`：现有单测全绿 + 一次「解一次只出合成图」的耗时/调用次数断言。

### 10.2 交叉验证（M6，不可用自解码代替）

| 读侧 | 检查 |
|---|---|
| Adobe Photoshop | 层名、层序、可见、不透明度、合成图正确 |
| Krita / GIMP | 能打开、不报「文件损坏」 |
| 本软件 `decode_psd_bytes` | 合成图逐像素一致 |
| Explorer 缩略图 DLL | 有缩略图、不崩 |

### 10.3 探针与脚本

- `scripts/probe_psd_roundtrip.py`（新）：`--script` 串跑「打开多层 PSD → 移动某层 → 保存 → 重开 → 断言层数/位置」；同步路径必须拿到完整分层。
- `scripts/probe_psd_perf.py`（新）：§6.4 门槛硬断言。
- `scripts/check_bridge_api.py`：已有，改完必跑。
- `cargo test --workspace`：全程绿灯（现 291 项基线）。

### 10.4 手动清单

`docs/大版本重构-手动测试清单.md` 新增 **§A26 PSD 分层专项**：打开多层 / 层名中文 / 隐藏层 / 不透明度 / 负偏移 / 越界层 / 混合模式降级提示 / 无合成图 / 超限只读 / Ctrl+S 一次撤销链 / 另存为 PSD 再用 PS 打开 / 100 层面板流畅度 / **编辑锁专项**（锁期间画一笔 / 拖图层 / 按 Delete / Ctrl+Z / Ctrl+S 全部无响应；滚轮缩放与平移仍可用；进度与「跳过」可解锁；解析失败与切换文件后自动解锁）/ 缩略图刷新 / 窗口缩放不跳。

---

## 11. 里程碑与工作量

| 阶段 | 内容 | 人天 |
|---|---|---|
| **M0** | 造大尺寸多层 fixture、扩展 `dec_bench --psd-layers`、埋点、**存基线数据** | 1.0 |
| **M1** | vendor 访问器 + `psd_layers.rs` 分层解析 + 段校验扩展 + fixture 单测 | 2.5 |
| **M2** | `Document::from_psd_layers` + core 单测 | 1.5 |
| **M3** | 两阶段接线（`DecodeMsg` / prewarm / `poll_psd_layers` / **`editingLocked` 编辑锁与全部 QML gate**）+ 同步路径 | 2.5 |
| **M4** | `psd_write.rs` 编码器（文件头 / layer&mask / RLE 通道 / 合成图 / `luni`）+ 往返单测 | 3.5 |
| **M5** | 解除 5 处拦截、`SaveFormat`、原子写、QML 对话框与提示改造 | 1.5 |
| **M6** | **性能收口**（C1~C8 逐条测、门槛回归）、五读侧交叉验证、文档与打包 | 2.5 |
| 合计 | | **15.0** （区间 12~17；编辑锁比原估多 0.5d） |

建议发版节奏：M1+M2 → M3+M4 → M5+M6 三次提交，每次都保持 `cargo test --workspace` 全绿；
M6 完成后走 `.\scripts\package.ps1`（版本号自动 patch+1，旧安装包保留）。

---

## 12. 明确不做（C 档再议）

C 档要扩 `Layer` 模型加 `blend_mode / parent / mask` / 文本，会波及所有 `EditOp`、快照、拖拽排序、合并语义和脚本探针，等于把刚完成的图层重构重来一遍，估 30~50 人天。B 档**不为此预留半成品字段**，避免把复杂度提前引进来。

---

## 13. 实施状态（M0 ~ M6 已全部落地，2026-11-12）

按本文件的里程碑顺序**串行**推进，每个模块自测通过后才进入下一个。

| 里程碑 | 状态 | 落地位置 |
| --- | --- | --- |
| **M0** 基准基建 | ✅ | `scripts/make_psd_fixture.py`（6 份样本）、`crates/decode/examples/dec_bench.rs --json --psd-layers`、`scripts/probe_psd_perf.py --baseline/--collect`、`tmp/psd-baseline.json` |
| **M1** 分层解析 | ✅ | `crates/decode/src/psd_layers.rs`（`decode_psd_layers` / `_with_progress`、按层矩形展开通道、PackBits、段边界预校验、逐层进度） |
| **M2** 文档装配 | ✅ | `core::document::Document::from_psd_layers` + `PsdLayerSeed`（0 号位透明背景、层属性映射） |
| **M3** 两阶段接线 + 编辑锁 | ✅ | `bridge/session.rs`（`spawn_psd_layer_job`、`from_decoded_with_layers`）、`bridge/ops/document.rs`（`poll_psd_layers` / `install_psd_layers` / `finish_psd_layers`）、QML 四处 gate |
| **M4** PSD 编码器 | ✅ | `crates/decode/src/psd_write.rs`（`encode_psd` / `encode_document`，8bit RGB / RAW / planar / Layer & Mask / 合成图 / `luni`） |
| **M5** 写回与 UI 改造 | ✅ | `bridge/paths.rs`（`SaveFormat`、原子写、写回守卫）、`Session::save_target`、`save_edited` / `save_as`、QML 解拦截 + `PSD (*.psd)` 过滤器 |
| **M6** 性能收口 | ✅ | 缩略图后台预渲染 + 分批落盘、`psd-notice`、`probe_psd_roundtrip.py`、`crosscheck_psd.py`（第三方 psd-tools 双向对照）、本文档与手测清单 §A26 |

### C1 ~ C8 逐条验收

| 条款 | 结果 | 证据 |
| --- | --- | --- |
| **C1** `decode_psd_bytes` 签名/行为不变 | ✅ | `git diff crates/decode/src/lib.rs` 中该函数与 `validate_psd_sections`、`encode` **零改动**；分层解析是独立模块 |
| **C2** 两阶段、首帧不等阶段 2 | ✅ | 实测 `psd-stage1-ms` = 1ms（small-layers，基线漂移 ≤3%）；阶段 2 在独立线程，`prewarm` 不等它 |
| **C3** 按图层矩形分配位图 | ✅ | 5 层 260×400 样本 `psd-layer-bytes` = 676400，而非 5×416000；完全在画布外的层跳过 |
| **C4** `flatten_layers_rgba()` 只做兜底 | ✅ | 仅在 `has_composite == false` 时走兜底；正常路径直接解 layer records |
| **C5** 编码/解码在后台 + `catch_unwind` | ✅ | `psd_layers.rs:89`、`psd_write.rs:534`、`session.rs:441`（后台编码线程）三处保留 `catch_unwind` |
| **C6** 缩略图懒生成 / 分批 | ✅ | 阶段 2 线程预渲染 40px；UI 线程每 50ms ≤6 行落盘；100 层实测 `layer-thumb-first-ms` = 0ms、`layer-thumb-batch-ms` 最大 11ms（门槛 16） |
| **C7** 层数 / 内存上限提示 | ✅ | `install_psd_layers`：`count > 32 \|\| bytes > 256MB` 时状态栏追加「图层较多（N 层 / M MB），实时预览重建可能变慢」 |
| **C8** >8192 降采样只读 | ✅ | `DOWNSAMPLE_EDGE = 8192` 未改；`save_target` 对 `downsampled_from.is_some()` + 目标是源文件时用 `DOWNSAMPLED_HINT` 拒绝；`probe_psd_roundtrip.py` 覆盖 |

### C2-a ~ C2-d 编辑锁生命周期

`scripts/probe_psd_layers.py --only all` 覆盖 5 条路径，全部 PASS：

| 路径 | 触发方式 | 断言 |
| --- | --- | --- |
| 同步自动化（`--script`） | 直接打开 | 0 个 `editing-locked`，一次性装配 5 层 |
| 正常交互 | prewarm stage 2 | `editing-locked → psd-stage2-ms → psd-layer-count=5 → psd-install-ms → editing-unlocked(done)` |
| 解析失败 | `IMAGEEDIT_PSD_STAGE2_FAIL=1` | 仍解锁（`reason=done`），保持合并预览 |
| 用户跳过 | 逐层延迟 + `AUTO_SKIP` | 进度出现后解锁（`reason=skipped`），不装配分层 |
| 慢加载 | 逐层延迟 400ms+ | 锁真实保持，进度逐层递增，仍在 3s 内完成 |

另有切文件（`finish_psd_layers("new-load")` / `"sync-load"`）、通道断开（`"channel-closed"`）、关窗口（`skip_psd_layers()`）三条 finally 式收尾。

### 与计划的偏差（已修正的真实缺陷）

1. **可见位语义写反**：vendor 解析器把 flags bit 1 当「置位=可见」，Adobe 规范是「置位=隐藏」。不是本档新增的 bug，但会同时毁掉读与写（真实 PSD 全判隐藏、写出的 PSD 在 Photoshop 里全隐藏）。已在 `vendor/psd/.../mod.rs` 与编码器同步修正，并加回归单测 + 20 份真实样本核对。
2. **透明占位背景被写进 PSD**：`layers[0]` 是为统一模型造的全画布透明层，写回时会变成真实图层，导致每保存一次多一层。现已在编码器中跳过。
3. **阶段 2 延迟钩子位置**：自动化钩子原本 sleep 在 stage 2 线程开头，大部分落在首帧之前，导致「锁时长」测不出来；改为逐层延迟后锁时长可测。

### 五读侧交叉验证

| 读侧 | 结果 |
| --- | --- |
| 自解码（`decode_psd_layers`） | `probe_psd_roundtrip.py`：写回后重开，层数/层名/显隐/opacity/offset 全一致，连续保存层数不增长 |
| 独立第三方实现（psd-tools 1.26.1） | `crosscheck_psd.py --only written`：层表逐项一致 + 合成图 4412 色；`--only vendor`：30 份第三方 fixture 双向对照 0 失败 |
| 资源管理器缩略图（`crates/thumbnail`） | `decode_psd_bytes` 零改动（C1），缩略图路径不受分层解析影响 |
| Photoshop | 本机未安装，**待用户手测**（§A26-15 / A26-18） |
| Krita / GIMP | 本机未安装，**待手测**（第三方实现对照已覆盖同一语义风险） |

---

## 附录 A：PSD 写入最小子集

只写 **8bit / RGB（`mode=3`）/ 无压缩或 RLE（`compression=0|1`）**，大端字节序。

```
File Header (26B)   '8BPS' | u16 version=1 | 6B reserved
                    | u16 channels(3 或 4) | u32 height | u32 width
                    | u16 depth=8 | u16 mode=3
Color Mode Data     u32 length=0
Image Resources     u32 length=0
Layer & Mask Info   u32 section_len
  Layer Info        u32 len | i16 layer_count | 每条 Layer Record:
      i32 top/left/bottom/right
      u16 channel_count | 每条 (i16 channel_id, u32 data_len)
      '8BIM' | 'norm' | u8 opacity | u8 clipping | u8 flags | u8 filler
      u32 extra_len
        u32 mask_data_len=0
        u32 blending_ranges_len=0
        Pascal 名: u8 len + 字节 + 补齐到 4 字节倍数
        [可选] '8BIM' + 'luni' + u32 len + UTF-16BE 名
      每条通道数据: u16 compression + (RLE: 每行 u16 len + PackBits | RAW: planar 字节)
  Global Layer Mask / Additional Info: u32 length=0
Image Data          u16 compression + 全画布合成图的 planar R/G/B/[A] 通道
```

要点：

- **Image Data 段必须写**：这是「最大化兼容」的合成图，缺了它很多阅读器显示黑图 / 拒绝打开。
- **通道是 planar**（先整层 R、再 G、再 B、再 A），不是 RGBA 交错；`encode_psd` 从 `RgbaImage` 拆平面。
- **RLE = PackBits + 每扫描行 2 字节长度表**，行表要先写完再写所有行数据；通道长度字段含行表。
- `layer_count` 与 alpha 的负号约定要跟第三方阅读器实测对齐（写 4 通道 + 正计数通常可行，M6 交叉验证定稿）。
- 层矩形可为负 / 越出画布；写回时用 `doc.layers[i].bounds()` 与 `offset`，裁剪到画布内。
- 中文层名靠 `luni`（UTF-16BE），Pascal 名只作兼容占位。
- 不做 ZIP 压缩（`compression=2/3`）、不做 16bit、不做 CMYK。

## 附录 B：关键代码位置索引

| 用途 | 位置 |
|---|---|
| PSD 合成图解码 | `crates/decode/src/lib.rs:292` `decode_psd_bytes` |
| 段边界预校验 | `crates/decode/src/lib.rs:353` `validate_psd_sections` |
| `Decoded` / `DecodeOptions` | `crates/decode/src/lib.rs:89` / `:55` |
| 编码分派 | `crates/decode/src/lib.rs:431` `encode` |
| 只读解析器 API | `vendor/psd/src/lib.rs:67` `Psd`、`:150` `layers()`、`:205` `flatten_layers_rgba()` |
| 逐层属性 / `rgba()` | `vendor/psd/src/sections/layer_and_mask_information_section/layer.rs:70-127` / `:250` |
| 全画布 RGBA 分配（禁止直接使用） | `vendor/psd/src/psd_channel.rs:33` `generate_rgba` |
| 缩略图 DLL 复用点 | `crates/thumbnail/src/lib.rs:394` |
| 后台预热 | `crates/ui/src/bridge/prewarm.rs:26` `spawn` |
| 异步落地 | `crates/ui/src/bridge/ops/document.rs:200` `poll_decode` |
| 文档装配 | `crates/ui/src/bridge/session.rs:566` `from_decoded` → `Document::from_image` (`core/document/mod.rs:126`) |
| `DecodeMsg` / 降采样上限 | `crates/ui/src/bridge/session.rs:311` / `:317` |
| 图层模型 | `crates/core/src/layer.rs:38` `LayerContent`、`:45` `Layer` |
| 全幅合成 | `crates/core/src/document/mod.rs:1084` `flatten` |
| 预览合成缓存 | `crates/render/src/lib.rs:290` `PreviewComposer` |
| 行缩略图（同步、需懒化） | `crates/ui/src/bridge/ops/layer_cmds.rs:470` `layer_thumb_url` |
| 保存拦截 1~4 | `bridge/paths.rs:110-128`、`session.rs:920/947`、`ops/document.rs:178` |
| 保存拦截 5 / 对话框 | `qml/editor/EditorContent.qml:2673` / `:808` / `:834` |
| 桥方法漏声明自检 | `scripts/check_bridge_api.py` |
| 性能基准 | `crates/decode/examples/dec_bench.rs` |
| 现成 PSD fixture | `vendor/psd/tests/fixtures/*.psd`（20 份） |

---

## 14. 图层样式（lfx2）烘焙（2026-11-12 追加）

真实 UI 稿里的阴影、描边、渐变叠加**不在图层通道里**：Photoshop 把它们存成
`lfx2` 里的描述符参数，渲染时才算。不解析就等于把按钮的描边、标题的渐变填色
全部丢掉 —— 这是「标题只剩一圈红轮廓」的根因。

### 14.1 三种键，同一份载荷

| 键 | 出现场景 |
| --- | --- |
| `lfx2` | 根目录下的图层 |
| `lmfx` / `lfxs` | **组内**图层（CS6+ 起 Photoshop 换的键） |

只认 `lfx2` 时，用户这份文件的 341 个带样式图层只读出 128 个（其余都在文件夹
里）。三键共用同一个 `DescriptorBlock2` 解析器，故在 `read_layer_record` 里合并
成一个匹配臂。

### 14.2 解析

| 文件 | 职责 |
| --- | --- |
| `vendor/psd/src/descriptor.rs` | 通用描述符读取（`Objc` / `VlLs` / `UntF` / `enum` / `TEXT` …）。**所有读取都返回 `Option`**：一个字节读错就整块丢弃，绝不让"半解析"的结构继续往下走 |
| `vendor/psd/src/effects.rs` | 描述符 → `LayerEffect { kind, enabled, blend, color, opacity, angle, distance, size, choke, stroke_position, stroke_fill, gradient, knocks_out, glow_source }`，同时把渐变（颜色站 + 透明站）与几何参数解析出来 |

两个容易踩的坑（都已修正）：

1. **`enum` 的两半都是长度前缀键**（`u32 len + 4 字节`），按裸 4 字节读会让后面
   整个描述符错位，表现为"每个块都解析失败"。
2. **描述符里的 unicode 名字不补齐 4 字节**（和 `luni` 不同），读错就会把类 ID
   当成长度。
3. **同种效果有两种容器**：单数键（`DrSh` / `FrFX` / `GrFl` / `SoFi` …）装一个
   描述符；CS6+ 的 `xxxMulti` 键装一个 `VlLs` **列表**。只认单数键会漏掉大半
   样式，详见 §14.3.1。
4. **键名**：渐变叠加是 `GrFl` 不是 `Grdn`，图案叠加是 `patternFill`。

`vendor/psd` 不是 workspace 成员且自带 dev-dependencies，之前一直无法单独跑测试，
里面还积压了三处失效的测试代码（`LayerProperties` 少字段、`enum` 少了长度前缀、
`clipping mask` 断言写反）。本轮在根 `Cargo.toml` 加 `exclude = ["vendor/psd"]`
后，可用 `cargo test --manifest-path vendor/psd/Cargo.toml` 独立跑，69 项全绿。

### 14.3 渲染（`crates/decode/src/psd_effects.rs`）

文档模型没有样式栈，所以按「拼合图层」的语义一次性烘焙进图层像素，顺序照抄
Photoshop 的样式栈：

```
投影 / 外发光        （画在内容之后面）
图层内容
描边、内阴影、内发光  （只作用于内容内部）
颜色叠加 / 渐变叠加   （盖在内容之上）
```

- **画布扩张**：投影、外描边会画到图层矩形外，按 `模糊半径×1.5 + 距离` 四周扩
  边并把 `offset` 同步左移/上移（上限 512 px），否则阴影会被图层矩形裁掉。
- **描边**：用两遍 chamfer 距离场（`distance_to`）算到边缘的距离，O(像素) 而不是
  O(像素 × 半径)；外/内/居中三种位置各自取带抗锯齿的覆盖率。
- **模糊**：三次盒滤波近似高斯，`sigma = size / 2`（Photoshop 的 size 就是 2σ）。
- **渐变**：线性/径向/角度/对称/菱形五种几何，站间线性插值，`scale` / `reverse`
  / `align` 都参与；描边用同一套 `gradient_t` 取色（`GrFl` 描边）。
- **未支持**：图案叠加、斜面和浮雕 —— 会写进该层的降级说明，不会静默丢掉。

**已知边界**：效果的混合模式是在**本层内部**合成的（投影的混合模式看不到下面
的图层）。真实图层样式的正片叠底阴影会与下层画面相乘，烘焙版不会 —— 这是烘焙
方案的固有限制，只有做实时样式栈才能消除。

### 14.3.1 多重效果栈（2026-11-13 修复）

上一版把问题记成「同一图层有三个 `FrFX`，选错了条目」——**这个判断是错的**，
两条依据都站不住：

1. 真实结构不是重复键，而是 **`xxxMulti` → `VlLs` 列表**。「矩形 5」
   `(32,1453,148,1487)` 的 `frameFXMulti` 是一个含 3 个描述符的列表
   （2 px 粉 `(248,158,190)` / 4 px 白 / 6 px 粉，均 `enab=true`、`present=true`）。
   旧解析器只接受 `DescriptorValue::Descriptor`，遇到 `VlLs` 直接 `continue`，
   **这三条描边一条都没烘焙**，不是「画错了」。
2. 扫描线选错了 y。y=1445 在按钮 bbox `(32,1453,…)` 之上，扫到的是别的层；
   那条深藏青 `#060B4A` 3 px 其实来自「图层 567」`(22,1400,233,1503)` 的**单数
   `FrFX`**（`Sz=3`、`Clr=(6,11,74)`），与多重描边无关。

普查 694 个叶层的样式块：122 个 `lfx2` + 151 个 `lmfx`（都另有兼容用的 `lrFX`），
`*Multi` 列表分布 `solidFillMulti` 233、`innerShadowMulti` 186、`dropShadowMulti`
176、`gradientFillMulti` 172、`frameFXMulti` 169。顺带查出两个漏解析的键：
渐变叠加实际叫 **`GrFl`**（旧代码写的是 `Grdn`），图案叠加叫 `patternFill`。

**规则用实测定，不靠猜**：在 `psd::effects::parse_layer_effects` 里把列表展开成
`Vec<LayerEffect>`，用 `IMAGEEDIT_FX_MODE` 切换五种排法，各自导出合成图与
`psd.topil()` 逐像素比：

| 规则 | MAE | 最差横带 |
| --- | --- | --- |
| `off`（旧行为，忽略 `*Multi` 与新键名） | 6.986 | 20.12 |
| `file`（列表原序全画） | 6.971 | 21.14 |
| `reverse`（逆序全画，首条在最上） | 6.767 | 21.09 |
| **`first`（只取首个启用项，默认）** | **6.674** | **20.31** |
| `last`（只取末个启用项） | 6.885 | 20.91 |

消融（`IMAGEEDIT_FX_SKIP`，以 `first` 为基底）证明各类样式都是正收益：
关掉描边 +1.522、颜色叠加 +0.975、渐变叠加 +0.255、阴影/发光 +0.144；
全关掉退化到 9.813。

**默认取 `first`，并明确这是近似**：Photoshop 会把列表中所有启用的条目画成同心
环。烘焙方案没有实时样式栈，逐条叠加会累积扩边与混合误差（实测 `reverse` 在按钮
区反而比 `first` 差 0.78）。这些列表里多余的条目大多是设计师留下的草稿，取栈顶
那条与成品最接近。其余排法保留为诊断开关，便于以后换成真实样式栈时对照。

配套修正：`masterFXSwitch=false` 整栈不渲染；`present=false` 优先于 `enab=true`
禁用；一条记录里出现多个样式块时**合并**而不是让后一个覆盖前一个。

**注意**：本节所有 MAE 只作用于「进入分层编辑」后的重建画面。默认路径是保真模式
（§16），直接显示 PSD 自带合并图，MAE 恒为 0.00，不受样式实现影响。

### 14.4 量化

以用户真实文件（750×1700，617 层，与 `psd.topil()` 合并图逐像素比较）：

| 阶段 | MAE | 说明 |
| --- | --- | --- |
| 修斜切之后 | 9.81 | 无混合模式、无调整层 |
| + 混合模式 + 调整层烘焙 | 9.81 | 同上（当时样式仍缺） |
| + `lfx2`（仅根层） | 8.03 | 113 层带样式 |
| + `lmfx` / `lfxs` | 6.99 | 227 层带样式 |
| + `xxxMulti` 列表 + `GrFl` 键名修正（§14.3.1） | **6.674** | 272 层带样式，P95 44.7 |

分带 MAE 从最差 28.27（按钮/文字区）降到 20.31；改善最明显的是第 2 横带
（11.53 → 8.41）。以上均为**分层重建**画面与 `psd.topil()` 的比；保真模式
（§16）走 PSD 自带合并图，MAE 0.00，不受样式实现影响。

### 14.5 验收

| 项 | 结果 |
| --- | --- |
| `cargo test --workspace` | 329 passed / 0 failed |
| `cargo test --manifest-path vendor/psd/Cargo.toml` | 69 passed / 0 failed（本轮首次跑通） |
| `scripts/crosscheck_psd.py --only all` | 30 通过 / 16 跳过 / 0 失败 |
| `scripts/check_bridge_api.py` | 158 个 QML 调用 / 0 未声明 |
| `scripts/compare_composite.py` | 输出 MAE + 8 横带 + 三栏对比图，取代肉眼判断 |
| 写回安全 | 样式烘焙会计入 `degraded`，覆盖源文件前 QML 仍会要求确认一次 |

## 15. 隐藏标注层（2026-11-12 追加）

PSD 里常有一层「标注 / 备注」文件夹放尺寸、色值、字号这些说明图层。它们不是画面
内容，做像素对照时反而挡视线。

- 判定只看**组名**（`标注 / 备注 / 注释 / 批注 / 说明 / 切图`），不看图层名 ——
  中文 UI 稿里大量文字图层叫「字号28，描边2xp #fffff」这类按样式命名，按名字匹配
  会把真正的文字也误杀。
- `decode` 在 `PsdLayerData.annotation` 上标记，`install_psd_layers` 用稳定 id
  记下来（下标会随重排变化）。
- 菜单：图层与标记 → 「隐藏标注层」（`bridge.toggle_annotation_layers()`），
  一次性提交成一条 `Batch` 历史，Ctrl+Z 一把恢复。
- 注意：本机这份文件里「标注」组本身在 PSD 里就是隐藏的（组可见性已折叠进子层），
  所以开关对它是空操作；顶部那些黄字是背景栅格图的一部分，不属于任何图层。

## 16. 保真模式（2026-11-13 追加，方向调整）

产品定位是**代替 macOS Preview 的轻量看图 / 轻编辑工具**，不是 Photoshop 复刻。
§14 那种「逐特性复刻 + 烘焙」的路线命中幂律长尾，继续投入不划算。PSD 文件本身
存了**两份画面**：一张 Photoshop 保存时渲染好的合并图，和一堆图层数据。显示一致
根本不需要任何 PS 特性 —— 直接显示合并图就是 100% 一致。

### 16.1 行为

| | 保真显示（默认） | 分层编辑（用户显式进入） |
| --- | --- | --- |
| 画面来源 | PSD 自带合并图 | 默认仍是合并图（§17 校准）；首次编辑后改为按层重建（MAE 6.674） |
| 阶段 2 图层解析 | **完全不启动** | 点菜单时才启动，约 4 s / 694 层 |
| 内存 / 层数 | 一张位图 / 1 层 | 617 层像素 + 缩略图 |
| 图层面板 | 只有背景层 | 完整图层栈 |
| Ctrl+S 覆盖 PSD | 禁止（读的是合并图） | 可用，降级项仍会先确认一次 |

入口在「视图」菜单：**保真显示（PS 合并图）** / **进入分层编辑…**。
`bridge.fidelityMode` 是当前状态，`bridge.hasPsdLayers` 决定菜单是否可用
（降采样只读 PSD 不开放切换）。

### 16.2 关键实现

- `load_path` / `prewarm::spawn` 默认不再 `spawn_psd_layer_job`，`DecodeMsg.layer_job`
  传 `None`；首帧走原有 composite-only 分支，不锁任何编辑工具。
- `enter_layered_editing()` 从 `session.source` 取路径，按需启动阶段 2；解析期间
  沿用 `editingLocked` 闸门，`poll_psd_layers` / `install_psd_layers` 一行不改地复用。
- **两份 Document 互换而不是替换**：`BridgeRust::inactive_doc` 存另一模式的
  `Document` 加 `dirty / note / psd_layered / psd_lossy`。History 在 Document 内部，
  因此来回切换不会丢任何一边的编辑、也不会把 Ctrl+Z 串到另一半。
  换文档时统一失效合成缓存、双缓冲与 GPU 快路径（否则会盖住旧纹理），
  再 `sync_layer_selection()` + `refresh_preview()` 刷新面板与画面。
- 第二次「进入分层编辑」直接换回缓存的图层栈（`mode=cache`），不重读文件。
- 写回守卫：保真态 `session.psd_layered=false`，`save_target` 依旧拒绝覆盖源 PSD。
- 回归钩子 `IMAGEEDIT_AUTO_LAYERED=1` 恢复「打开即装配分层」旧行为，
  供 `probe_psd_layers.py` 的编辑锁路径使用；`--script` 同步路径本来就直出分层，
  不参与切换。

### 16.3 验收探针

```
IMAGEEDIT_FIDELITY_PROBE=enter      # 保真 → 解析 → 停在分层态
IMAGEEDIT_FIDELITY_PROBE=roundtrip  # 装配完立刻换回保真
IMAGEEDIT_FIDELITY_PROBE=roundtrip2 # 再进一次，走缓存路径
```

结构化事件：`fidelity-state`（打开时的初始态）、`fidelity-enter`（parse / cache）、
`fidelity-mode`（每次切换后的值）。默认打开时日志里**不应**出现
`psd-stage2-ms` / `editing-locked` / `psd-layer-progress`。

## 17. 原生预览校准：编辑态与 PS 画面逐像素一致（2026-11-13 追加）

§16 解决了「看图」的一致，但用户一进分层编辑，画面就换成 §14 的按层重建
（MAE 6.674），仍然和 PS 不一样。要求「改到编辑模式与原生预览效果一致」时，
**继续补特性是到不了 0 的**：矢量蒙版、文字排版引擎、图层样式之间的实时混合
在烘焙方案里无法精确复刻，投入是无底洞。

但文件里本来就存着 Photoshop 自己渲染好的成品。于是把它同时用作**分层态的
显示基准**：

| 状态 | 画布显示 | 图层面板 | 可编辑 |
| --- | --- | --- | --- |
| 刚进入分层编辑，文档未改动 | **PS 自带合并图（MAE 0.00）** | 完整 617 层 | 是 |
| 任意一次编辑 / 手势之后 | 按层重建（MAE 6.674） | 完整 617 层 | 是 |

校准帧和保真帧走的是**同一个 `PreviewComposer` 路径 + 同一张像素**，缩放与
抗锯齿行为完全一致，所以是逐像素相等，不是「误差很小」。校准生效时连 617 层
合成都省掉了，预览反而更快。

### 17.1 关键实现

- `Session::calibration: Option<Calibration>`，其中 `reference` 是保真态那份
  1 层 Document（`install_psd_layers` 换出去时 clone 一份；`swap_display_docs`
  切回分层时重新挂上），`composer` 是**独立**合成器 —— 主 composer 的缓存键是
  `doc.edit_epoch`，两份 Document 的 epoch 都是 0，共用会串缓存。
- `install_calibration()` 只在 `psd_layered && reference.canvas == doc.canvas`、
  且分层栈仍是原始状态（`!dirty && (epoch == 0 || !can_undo())`）时激活。
  已经编辑过的图层栈**不会**被静默盖回原图。
- ★ 失效判定必须放在 `render()` 的 **GPU 快路径早退之前**。拖动期间 render 会
  直接返回上一帧 URL，若失效判定在后面，旧的合并图会永久盖住画面 —— 和
  历史上 `gpuLive` 盖旧纹理是同一类坑。失效时一次性清 `last_render` +
  重建 `composer`，强制重画成按层重建的帧。
- 失效是 **sticky** 的：任何 bump `edit_epoch` 的编辑、任何手势 / 实时面板 /
  快速蒙版，都会永久停用校准，直到切回保真再进分层（或手动开关）。
  这样撤销时画面不会在「原图」和「重建图」之间来回跳。
- `gpu_begin()` 在校准生效时禁用快路径：它的「静态底图 + QML 移动活动层」是
  按重建栈生成的，和合并图叠一起会重影。手势一开始校准已失效，这里只是兜底。
- **所见即所得**：`begin_save()` 在校准生效且目标是位图格式时，导出
  `reference.flatten()`（PS 合并图）而不是 `doc.flatten()`。`SaveFormat::Psd`
  是例外 —— 写回必须保留 617 层图层栈，用 1 层合并图会毁掉原文件。
- 菜单「视图 → PS 原合成校准」可手动关掉 / 打开。重新打开时只有分层栈仍
  处于原始状态才会恢复。`bridge.calibratedPreview` 是当前是否生效，
  `bridge.hasCalibration` 决定菜单可用性；状态栏常驻提示当前显示的是哪一种。

### 17.2 边界（不是"支持了全部 PS 特性"）

- 校准只作用于**显示**。图层栈、缩略图、历史、写回都还是真实的 617 层；
  样式依旧是烘焙过的，改不了渐变角度。这是「显示基准」，不是实时样式栈。
- 用户一旦动手改，画面切回重建结果（MAE 6.674）。此时用户已经主动改变了
  画面，「与 PS 一致」不再是目标，可编辑性才是；状态栏会同步说明。
- 撤销到干净状态**不会**自动恢复校准（`edit_epoch` 单调递增），切一次模式即可。
- 同步 `--script` 路径直出分层、没有保真态可参照，不校准；降采样只读 PSD
  不开放切换，同样不校准。

### 17.3 验收

```
python scripts/probe_calibration.py --seconds 40
```

三条独立启动路径（靠结构化事件驱动，不点击、不 SendInput）：

| 路径 | 期望事件 | 断言 |
| --- | --- | --- |
| `fidelity` | `fidelity-state:true` | 没有 `calibration-install` |
| `layered` | `calibration-install active:true`、`calibration-frame`、`fidelity-mode:false` | 预览 BMP 与 `fidelity` 帧 **MAE 0.00 / max 0** |
| `edited` | `calibration-off`、`calibration-probe-edit` | 校准 sticky 失效 |

`IMAGEEDIT_CALIBRATION_MAE=1` 会额外合成一次重建帧并打印
`calibration-mae`（约 6.67），用来证明「被替换掉的确实是有误差的那张」，
而不是校准逻辑根本没生效。`IMAGEEDIT_CALIBRATION_PROBE=edit` 在装配后模拟
一次编辑（epoch bump + dirty）以驱动失效路径。

## 18. 普通图片「另存为 PSD」（2026-11-13 追加）

### 18.1 背景

`encode_psd` 本来就接受任意 `Document`：它把 `doc.layers` 逐层写成 PSD 图层
记录、把 `doc.flatten()` 写成合并图。所以「PNG 存成 PSD」在编码层早就成立，
缺的只是入口与后缀：

1. 菜单里只有泛化的「另存为…」，PSD 只是对话框里的一个过滤器，不显眼。
2. 泛化对话框把 `defaultSuffix` 写死成 `"png"`。选了 `Photoshop (*.psd)`
   再输入不带扩展名的文件名，Qt 仍会补成 `.png` —— 用户以为存了 PSD，实际
   写出去的是 PNG 字节。
3. `validate_save_target` 对「源文件是合并预览态 PSD」一律拒绝 `.psd` 目标，
   连导出到另一个文件也一并拦掉，与它自己的注释（导出到不同路径应当允许）
   矛盾。

### 18.2 改动

| 位置 | 改动 |
| --- | --- |
| `crates/ui/qml/panels/TitleBar.qml` | 文件菜单新增「另存为 PSD…（保留图层）」，`ok` 条件为有图且不在保存中 |
| `crates/ui/qml/editor/EditorContent.qml` | 新增 `psdSaveDialog`（过滤器固定 Photoshop、`defaultSuffix: "psd"`）与 `openSaveAsPsd()`；默认落在源文件同目录、同名 `.psd` |
| 同上 | 泛化 `saveDialog` 的 `defaultSuffix` 改为空，`onAccepted` 走 `saveTargetUrl()` 按**当前选中的过滤器**补后缀 |
| `crates/ui/src/bridge/paths.rs` | 合并预览态 PSD 的 `.psd` 限制收窄为**仅覆盖源文件**；导出到其它路径放行 |
| `crates/ui/src/bridge/ops/document.rs` | 保存成功后打印 `psd-saved`（`layers` / `groups`），状态栏给出「已保存 PSD：…（N 个图层）」；有分组时补一句「M 个分组已展开，不写入 PSD」 |

### 18.3 语义与边界

- **写什么**：当前 `doc.layers` 的每一层（位图 / 标记都算，标记按其画布外接
  框栅格化），合并图用 `doc.flatten()`。普通图片只有 `layers[0]`，就是 1 层。
- **分组**：分组是本编辑器的平行元数据，PSD 图层段目前不写 `lsct` 分组标记，
  所以成员会**展开成平级图层**。这是有意为之（否则重开时反而多出虚假层级），
  状态栏会明说。
- **仍然禁止**：`另存为 PSD` 覆盖一个「合并预览态」的 PSD 源文件（会烧掉原
  图层）；降采样打开（>8192）的文档任何 PSD 写出都被拒绝。
- **校准态**：位图导出用校准参考图（所见即所得）；PSD 导出例外，必须写图层
  栈，不能用 1 层合并图顶替。

### 18.4 验收

```
python scripts/probe_psd_roundtrip.py --only saveas
```

| 用例 | 断言 |
| --- | --- |
| 普通 PNG → 新 `.psd` | `saved` 事件、无 `save-error`、文件 1 层且可见位不被置为隐藏 |
| 合并预览态 PSD → 另一个 `.psd` | 放行且写出 1 个平铺图层（覆盖源仍由 `--only composite` 验证被拒） |
| 分组文档 → `.psd` | `psd-saved.groups >= 1`，文件层记录保持平铺 5 层 |

## 19. PSD 图层分组（真实组树，2026-11-13 追加）

### 19.1 之前的问题

老实现把 PSD 的文件夹**摊平**成平级图层，并在降级清单里写「组结构已展平」。
后果是打开一份 UI 稿，617 层平铺在一个列表里，看不出层级，也无法整组显隐。
分组功能（§13 之后的图层分组）当时只是编辑器内的元数据，PSD 读入时一律
`group = None`，所以「打开 PSD 看不到分组」是必然的。

### 19.2 读：把 PSD 的文件夹树还原

PSD 用两条 `8BIMlsct` 分隔记录夹住成员（文件顺序自底向上）：

```
[lsct 3 组底边界]  成员…（自底向上）  [lsct 1 展开 / 2 折叠 的文件夹行，带组名]
```

- `vendor/psd`：`PsdGroup` 增加 `collapsed`（由文件夹行的 `lsct` 类型 2 判定），
  新增 `collapsed()` 访问器；`Psd::group_ids_in_order()` 保持 Photoshop 面板顺序。
- `crates/decode/src/psd_layers.rs`：`PsdLayerData.group`（叶子所属组）、
  `PsdLayerSet.groups: Vec<PsdGroupData>`（`psd_id / parent / name / collapsed /
  visible / count`，`count` 按子树统计）。不再产出「组结构已展平」降级项。
- 组的显隐与不透明度**仍然**折叠进子层（画布必须与合并图一致），组的眼睛
  状态单独留在组记录上给面板显示。
- `core`：`LayerGroup.parent: Option<GroupId>` 支持嵌套；`Document::
  from_psd_layers_with_groups` 负责装配并把 PSD 的 u32 组号映射成本进程的
  `GroupId`（子组可能排在父组之前，父链二次回填）。
- `GroupId` 是**进程内**自增，不写进 PSD；落盘靠重新扫描组归属，所以撤销 /
  重做 / 另存都不会串号。

### 19.3 语义变化（嵌套）

| 操作 | 语义 |
| --- | --- |
| `group_members(gid)` | **直接**成员（面板缩进用） |
| `group_subtree_layers(gid)` | 子树成员（整组显隐 / 锁定 / 点选 / 面板高亮） |
| `set_group_visible / locked` | 覆盖子树，父组一关全关 |
| `ungroup(gid)` | 只摘这一层，子组挂到它的父组（不留孤儿组） |
| `group_layers` | 同一组里的若干层再编组 = 建**子组**；跨组混选直接拒绝 |

面板显示行改为递归：按层的祖先链（根 → 叶）逐级补组标题行，链上有任意一组
折叠就整棵子树收起来；空组只在「确实没有成员、且祖先都没折叠」时补一行
（否则折叠的父组下面会漏出「X（空）」）。

### 19.4 写：分组写回 PSD

`crates/decode/src/psd_write.rs` 现在按 `doc.groups` 生成 `PsdRecord`：

- 进入一个组（遇到它的最低成员）先写 `lsct 3`（组底边界），离开（最高成员
  之后）再写 `lsct 1|2`（文件夹行，带组名，折叠态原样带上）；
- 嵌套组按祖先链的自然顺序展开，切换组时「先按内→外关、再按外→内开」；
- 空组补一对空壳记录，避免重开时组凭空消失；
- 分隔记录是空矩形 + 4 个「只有压缩标记」的通道（`channel_len = 2`），
  与 Photoshop / psd-tools 的写法一致。

### 19.5 验收

```
python scripts/probe_psd_groups.py          # 5 段：组树 / 归属 / 折叠 / 整组显隐 / 写回
python scripts/probe_psd_roundtrip.py       # 写回、隐藏位、降级拦截
python scripts/crosscheck_psd.py --only all # psd-tools 独立实现对照
```

实测（`0302-界面UI-1.psd`，117 MB）：**617 层 / 197 个组**，最深 3 层，
状态栏报「617 个可编辑图层、197 个图层组」。`nested-groups.psd`
（psd-tools 生成，Open/Closed/嵌套各一）导出后 `psd-tools` 读回的层级、
折叠态与原名一致。

### 19.6 已知边界

- 组的 `lsdk`（混合模式 / 不透明度 / 蒙版）仍未写回，组透明度按成员折叠。
- `lsct` 只写「展开/折叠」两种，`lsdk` 的「组作剪切蒙版」不支持。
- 面板拖拽排序落到组标题行时不移动（`i = -1`），要拖到具体图层行上。

### 19.7 发现性：保真态的图层面板提示条

默认打开 PSD 走保真模式（阶段 1 只解合并图），此时文档只有 **1 层**，
面板里自然看不到任何分组 —— 这正是「PSD 打开没有看见分组」的另一半原因
（组树已经解析好了，只是还没装进文档）。面板顶部因此加了一条提示条：

```qml
// crates/ui/qml/panels/LayerPanel.qml
visible: bridge.fidelityMode && bridge.hasPsdLayers
text:    "正在显示 PS 原合成（1 层）· 点此进入分层编辑，查看图层与分组"
onClicked: bridge.enter_layered_editing()
```

- 只在「保真 + 这个 PSD 确实有可解析图层」时出现，分层态不占高度；
- 点它等同菜单「视图 → 进入分层编辑」，解析期间照旧锁编辑工具；
- `listArea.anchors.top` 跟着它切换（`fidelityHint.bottom` / `header.bottom`），
  高度 44（两行 11 px 文案不挤压首行图层）。

写回状态文案同步改口：分组现在会写成 `lsct` 记录，所以保存后报
「已保存 PSD：路径（N 个图层、M 个图层组）」，不再说「分组已展开，不写入 PSD」。

---

## 20. 后续：编辑态的保真与性能（另文）

本文的「保真模式」解决的是**打开时**像不像 PS（阶段 1 合并图 + 校准）。
「进入分层编辑之后怎么继续保真、且不每帧重算 617 层」是另一个问题，方案另见
[`docs/PSD保真编辑-改造方案.md`](PSD保真编辑-改造方案.md)：

- 实测差距：重建合成 vs PS 合并图 MAE 10.67/255（14.76% 像素差 > 8），
  主因是**剪贴蒙版（210 个可见层）与图层蒙版（95 个可见层）未实现**；
- 实测成本：全幅重建 124 ms + 整幅落盘 130 ms ≈ 258 ms/帧；
- 方案：**PS 合并图当保真底图 + 64 px 瓦片脏块重合成**，单块 p50 0.22 ms；
- 阶段：P0 语义补齐（蒙版/剪贴/单次量化）→ P1 瓦片增量 → P2 前缀缓存 → P3 预览去落盘。

配套工具：`scripts/psd_fidelity_report.py`（特性画像 + 瓦片成本 + MAE 归因）。
