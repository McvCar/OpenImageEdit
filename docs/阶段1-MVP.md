# 阶段 1 — MVP 进展报告

> 目的：记录阶段 1（快速冷启动预览 + 基础编辑）的实现进度、实测数据与遗留项。
> 更新日期：2026-10-17。数据为本机实测（Windows 10/11 x64，虚拟显示适配器环境，详见 `docs/阶段0-技术验证.md` §5.3）。
> 相关文档：`docs/预览功能规划.md`（规划）、`docs/阶段0-技术验证.md`（技术验证）。

## 1. 进度摘要

| 阶段 1 条目 | 状态 | 说明 |
| --- | --- | --- |
| 快速冷启动预览 | ✅ | 打包版 tiny 448 / small 495 / medium 670 / large 1163ms，全部 ≤2000ms |
| 查看：打开（命令行/拖放/对话框） | ✅ | 命令行参数、`DropArea` 拖放、`Ctrl+O` 文件对话框 |
| 查看：缩放（适应/实际/步进） | ✅ | `Ctrl+0`/`Ctrl+1`/滚轮/工具栏，缩放范围 2%–1600% |
| 查看：平移 | ✅ | `Flickable` 拖动，越界回弹 |
| 查看：全屏 | ✅ | `F11` 切换（`Esc` 退出） |
| 旋转 / 翻转 | ✅ | 左/右转 90°、水平/垂直翻转，含图层坐标迁移 |
| 撤销 / 重做 | ✅ | `Ctrl+Z`/`Ctrl+Y`，基于 `editcore::History`，含变换后的画布尺寸还原 |
| 导出 / 另存 | ✅ | `Ctrl+S` 另存副本（`<名>-edited.<ext>`）、`Ctrl+Shift+S` 另存为 |
| 多窗口 | ⚠️ 部分 | 目前为「单进程单窗口」，多窗口 = 多次启动（尚未做同进程多窗口） |
| 裁剪 (Crop) | ✅ | 工具栏「裁剪」进入拖框模式（交叉光标），松手按选区裁剪；「重置裁剪」还原底图；含 23 项 core 测试 |
| 调整尺寸 (Resize) | ✅ | 「调整尺寸…」对话框：宽/高输入 + 保持比例 + 回车提交，Lanczos3 重采样；撤销栈可回滚 |
| 选区图层（框选/提取/移动/图层面板） | ✅ | 「框选」拖框 →「提取图层」生成图层；右侧图层面板（显示序=顶层在前）可 选择/显隐/上移/下移/删除；方向键 ±1、Shift+方向键 ±10 微调；跨图复制粘贴未做 |
| 基础快捷键 | ✅ | 见 §3 |

## 2. 本次交付物

| 文件 | 变更 |
| --- | --- |
| `crates/core/src/lib.rs` | 新增 `Transform`（Rotate90Cw/Ccw/180、FlipH/V，含 `inverted()`/`swaps_axes()`/`apply_to_image`）、`transform_document()`、`EditOp::TransformDocument`、`Document::apply_transform` |
| `crates/render/src/lib.rs` | 新增 `composite_for_preview(doc, target)`：无可见图层时直接缩放底图（快路径） |
| `crates/ui/src/bridge.rs` | 重写为会话状态机：13 个 qproperty + 20 个 `#[qinvokable]`；缩放/平移/变换/撤销/另存/脚本化自测 |
| `crates/ui/qml/main.qml` | 完整 UI：工具栏 + 状态栏 + `Flickable` 平移 + 滚轮缩放 + 快捷键 + 拖放 + 文件对话框 |
| `crates/bench/src/main.rs` | 缺失文件校验（避免静默伪数据） |
| `scripts/package.ps1` | 新增：可复现打包（构建 → windeployqt → 裁剪 → 自检 → zip） |

### 2.1 桥接接口（QML ↔ Rust）

- 属性：`status`、`preview_url`、`image_width/height`、`display_width/height`、`zoom_percent`、`fit_mode`、`can_undo`、`can_redo`、`decode_ms`、`shown_ms`、`auto_quit`、`loadFailed`、`defaultSaveUrl`、`layer_count`、`selected_layer`、`layers_json`（`[{"i":idx,"n":name,"v":visible}]`，顶层在前，QML 直接绑定的响应式图层列表）。
- 方法：`start`、`image_shown`、`mark`、`open_file`、`set_viewport`、`zoom_in/out/fit/actual`、`rotate_cw/ccw`、`flip_horizontal/vertical`、`undo`、`redo`、`save_edited`、`save_as`、`crop_to(x,y,w,h)`、`resize_to(w,h)`、`reset_crop()`、`extract_selection(x,y,w,h)`、`select_layer(i)`、`layer_name(i)`、`layer_visible(i)`、`toggle_selected_visible()`、`move_selected_up/down()`、`translate_selected(dx,dy)`、`delete_selected_layer()`。
- 会话状态（`Session`：文档 + 源路径 + 缩放 + 适应标志 + 渲染计数）通过 `RefCell<Option<Session>>` 持有——`Pin<&mut Self>` 没有 `DerefMut`，这是 cxx-qt 0.10 的硬约束。

### 2.2 裁剪 + 调整尺寸（2026-10-17 迭代）

| 文件 | 变更 |
| --- | --- |
| `crates/core/src/lib.rs` | `Document.view: Rect`（裁剪视窗）；`EditOp::CropDocument`（非破坏：仅移动视窗 + 图层裁剪）/`EditOp::ResizeDocument`（Lanczos3 重采样 base，view 重置、图层等比缩放）；`Document::crop/resize/reset_crop`；`EditError::InvalidCanvasSize`；+5 测试（23 项全绿） |
| `crates/render/src/lib.rs` | `composite_for_preview` 快路径：裁剪区 == 全画布时跳过裁剪合成 |
| `crates/ui/src/bridge.rs` | 新增 3 个 `#[qinvokable]`：`crop_to`（钳制到画布内）/`resize_to`（≥1 校验）/`reset_crop`；`run_script` 分隔符改为 `;` 并支持参数化 op（`crop:x,y,w,h`、`resize:w,h`） |
| `crates/ui/qml/main.qml` | 裁剪模式（`cropMode`：拖框 + 高亮矩形 + 状态栏提示 + Esc 取消）、`resizeOverlay` 对话框（宽/高/保持比例/当前尺寸）、双 Escape 快捷键分发、裁剪中禁用滚轮缩放 |
| `crates/ui/src/main.rs` | `qml-load-failed` 详情输出 errorMessage（定位 QML 创建失败） |
| `scripts/env.ps1` | `CFLAGS/CXXFLAGS=/utf-8`（见 §5 第 10 条） |
| `scripts/package.ps1` | 自检链路更新为 `crop:100,100,800,600;resize:400,300;undo;redo`（`script-done` 断言） |

### 2.3 选区图层（2026-10-17 迭代）

| 文件 | 变更 |
| --- | --- |
| `crates/core/src/lib.rs` | 无改动——`Selection::Rect`、`Layer`（name/visible/opacity/offset）、`extract_selection`/`remove_layer`/`move_layer`/`set_layer_visible`/`translate_layer` 均已就绪并有测试 |
| `crates/ui/src/bridge.rs` | 新增 qproperty `layer_count`/`selected_layer`/`layers_json`；`Refresh::Ok` 增 `layer_count`+`layers_json`（`Session::layers_json()` 生成，顶层在前）；9 个 `#[qinvokable]`（extract/select/name/visible/toggle-visible/up/down/translate/delete）；`apply_layer_edit` 统一刷新；`run_script` 新增 `extract:`/`select-layer:`/`translate-selected:`/`toggle-visible-selected`/`delete-selected`/`layer-up`/`layer-down`；`script-done` 增 `layers`/`selected_layer`；`move_selected_up/down` 成功后 `selected_layer` 跟随被移动图层 |
| `crates/ui/qml/main.qml` | `selectionMode` 拖框（`enabled:`）→ `selRect` pending →「提取图层」；`moveMode` 拖动移动选中图层（松手一次性 `translate_selected`，拖框反馈）；右侧图层面板（`Repeater` 绑定 `layers_json`，显示序自顶向下）含 显隐/上移/下移/删除；三模式互斥（`toggleMode`）；统一 `Esc` 分发（`onEscape`）；方向键 ±1px / Shift+方向键 ±10px 微调 |
| `scripts/package.ps1` | 自检链路更新为 `crop:100,100,800,600;extract:100,100,300,200;resize:400,300;undo;redo`（`script-done` 断言） |

设计决策：不做破坏性「删除选区像素」（core 无此 op），只做「提取到图层」；图层 Vec 末尾 = 顶层，面板反向显示；「上移」= Vec index +1。移动图层不做逐帧重渲染（松手一次性提交），避免拖拽卡顿。

## 3. 交互清单

| 操作 | 快捷键 |
| --- | --- |
| 打开 | `Ctrl+O` |
| 另存副本 / 另存为 | `Ctrl+S` / `Ctrl+Shift+S` |
| 放大 / 缩小 | `Ctrl+=` / `Ctrl+-`、滚轮 |
| 适应窗口 / 100% | `Ctrl+0` / `Ctrl+1` |
| 撤销 / 重做 | `Ctrl+Z` / `Ctrl+Y` |
| 右转 / 左转 90° | `Ctrl+R` / `Ctrl+Shift+R` |
| 全屏 | `F11`（`Esc` 退出） |
| 裁剪模式 | 工具栏「裁剪」进入；拖框选择后松手应用；`Esc` 取消（裁剪模式下 `Esc` 不退出全屏） |
| 重置裁剪 | 工具栏「重置裁剪」，非破坏还原到底图全尺寸 |
| 调整尺寸 | 工具栏「调整尺寸…」→ 对话框输入宽/高（可选保持比例）→ 确定 / 回车 |
| 框选 | 工具栏「框选」→ 拖框松手产生选区（高亮）；点「提取图层」生成图层；`Esc` 取消选区 |
| 提取图层 | 工具栏「提取图层」（有选区时可用），选区像素提为非破坏图层并选中 |
| 移动图层 | 工具栏「移动图层」（有选中图层时可用）→ 拖动画面移动；**方向键 ±1px、Shift+方向键 ±10px** 微调；`Esc` 退出 |
| 图层面板 | 工具栏「图层」开合：行点击选中该行图层；底部 上移/下移/显隐/删除 作用于选中图层；显示序自顶向下（首行=顶层） |

状态栏显示：画布尺寸、缩放百分比、（预览降采样时）预览尺寸；工具栏按钮按可用性自动置灰（`can_undo`/`can_redo`/`fit_mode`）。

## 4. 实测数据

### 4.1 冷启动（`imageedit-bench ui <file> --repeat 5`，spawn → 含图首帧）

| 图片 | 开发树 D3D11 p50 | 打包版 p50 | 打包版 p95 | 门槛 |
| --- | --- | --- | --- | --- |
| tiny 64×64 | 439ms | 448ms | 455ms | ≤2000 ✓ |
| small 1.9MP jpg | 486ms | **495ms** | 508ms | ≤2000 ✓（目标 ≤1000 ✓） |
| medium 12MP jpg | 627ms | **670ms** | 754ms | ≤2000 ✓（目标 ≤1000 ✓） |
| large 50MP jpg | 1146ms | **1163ms** | 1305ms | ≤2000 ✓ |

- 对照阶段 0 极简版（仅看图，无工具栏/编辑）：打包版 small 493 / medium 664 / large 1202ms。
- ⇒ **新增的完整 UI 与编辑能力没有带来可测量的冷启动代价**（差异在 ±2% 噪声内）；瓶颈仍是 Qt 固定开销（窗口显示 + RHI 首帧约 290ms）+ 大图解码（50MP JPEG 约 600ms）。

### 4.2 编辑链路正确性（像素级验证）

自测脚本 `--script=<op;op;...>`（分隔符 `;`，支持参数化 op）驱动桥接的公开方法，再用 `System.Drawing` 采样像素比对：

| 用例 | 断言 | 结果 |
| --- | --- | --- |
| `rotate-cw ×4` | 尺寸回到 1600×1200（恒等） | ✓ |
| `rotate-cw, undo` | 尺寸还原且 `redo=true` | ✓ |
| `rotate-cw`（另存） | 尺寸 1200×1600；`orig(x,y) == edited(H-1-y, x)`，diff ≤14 | ✓ |
| `flip-h, flip-v`（=180°） | `orig(x,y) == edited(W-1-x, H-1-y)`，diff ≤12 | ✓ |
| `zoom-actual, zoom-in, zoom-out, zoom-fit` | 100% → 125% → 100% → 适应 58% | ✓ |
| `crop:100,100,800,600;resize:400,300;undo;redo` | 800×600 → 400×300 → 撤销 800×600 → 重做 400×300；`script-done` 400×300 | ✓ |
| `crop:100,100,800,600;reset-crop` | 还原 1600×1200，状态 `crop reset`，`undo=true` | ✓ |
| `resize:800,300;undo` | 800×300 → 撤销还原 1600×1200，`redo=true` | ✓ |
| 缺失文件 | 551ms 内快速失败（而非等看门狗 10s），退出码 0 | ✓ |
| `extract:100,100,300,200;extract:400,400,300,200;layer-down;translate-selected:10,10;toggle-visible-selected` | 两次 `layer-extracted`（idx 0、1）→ `layer-down` 后选中跟随到顶；`script-done layers=2 selected_layer=0` | ✓ |
| `extract:100,100,300,200;delete-selected;undo` | 删除后 `layers=0` → 撤销还原 `layers=1`、`selected_layer=-1` | ✓ |

（diff 为 JPEG 有损编码误差；未位移像素的 diff 在 200–475 量级，证明变换确实生效。）

### 4.3 打包体积

| 版本 | 目录体积 | 文件数 | zip |
| --- | --- | --- | --- |
| 裁剪前（`windeployqt --qmldir` 默认） | 79.2MB | 1340 | 23.1MB |
| 裁剪后（`scripts/package.ps1`） | **43.4MB** | 78 | **17.4MB** |

裁剪掉的：`opengl32sw.dll`(19.7MB 软件光栅化回退)、`QtQuick/Controls` 及 8 个 Controls2 风格 DLL(约 15MB)、Effects/Shapes/Particles/Layouts(tooling)/VectorImage/NativeStyle/LocalStorage、`Qt6LabsFolderListModel`、`qmltooling`。
**必须保留**：`Qt6QuickControls2Impl.dll`（`Qt6QuickDialogs2QuickImpl.dll` 链接它，删掉会导致文件对话框插件加载失败）、`Qt6QuickTemplates2.dll`、`Qt6QuickDialogs2*.dll`。

## 5. 关键技术发现与坑

1. **`#[qproperty]` 要求在 Rust 结构体上声明同名字段**。只写属性宏不写字段会报 `no field ... on type Pin<&mut X>`，错误指向属性宏行，误导性强。
2. **QML 属性/方法名默认不做 camelCase 转换的连带影响**：`#[qproperty(bool, load_failed)]` 的变更信号是 `load_failedChanged`，QML 处理器得写成 `onLoad_failedChanged`（可编译但反直觉）。需要驼峰时用 `#[qproperty(bool, load_failed, cxx_name = "loadFailed")]`，QML 侧就是 `loadFailed` / `onLoadFailedChanged`。
3. **Qt 6 的 `FileDialog` 没有 `opened` 信号**（`Cannot assign to non-existent property "opened"`），只有 `visible`/`accepted`/`rejected`；探测「对话框已打开」应挂 `onVisibleChanged`。
4. **`QtQuick.Dialogs` 会拖入整条 Controls 依赖链**：`windeployqt --qmldir` 见到该 import 后会把 `QtQuick/Controls`、各风格 DLL 全部复制（体积 58→79MB）。但 `QtQuick.Dialogs.quickimpl` 的 `qmldir` 实际只 `depends QtQuick / QtQuick.Templates / QtQuick.Layouts`，故可安全裁掉 Controls 栈。
5. **打包自检必须自动化**：`--probe-dialog` 在启动 500ms 后自动打开文件对话框并把 `dialog-opened` 打点到 stdout，`--selftest --probe-dialog` 可无人值守验证「对话框插件 + QML 模块 + DLL 依赖」完整（本次裁剪失败正是靠它定位到缺少 `Qt6QuickControls2Impl.dll`）。
6. **PowerShell 会毁掉 UTF-8 源文件**：用 `Get-Content -Raw` + `-replace` + `Set-Content` 改含中文/符号的 `.qml` 会把非 ASCII 字符替换成 `?`（并吞掉引号），导致 qmlcachegen 报 `Expected token ','`。**改文件必须用专用编辑工具，不要走 PowerShell 文本往返**。
7. **PowerShell 管道截断会杀掉原生进程**：`windeployqt ... | Select-Object -First 5` 会因管道提前关闭而中断 `windeployqt`，产物只剩 `Qt6Core.dll`（当时 9.7MB）。要么全量重定向到文件，要么 `| Out-Null`。
   **补充（2026-10-17）**：脚本顶部 `$ErrorActionPreference='Stop'` 时，`cargo`/`windeployqt` 的 stderr 输出（进度/warning）会以 `NativeCommandError` 中断脚本——仅靠 `2>&1 | Out-String` 不够，必须临时 `$ErrorActionPreference='Continue'` 包一层再捕获。
8. **bench 必须校验输入文件存在**：否则 UI 子进程进入「解码失败 → 永不渲染 → 看门狗超时」路径，计时器会把超时当成首帧耗时上报（本次曾产出 2197ms/10609ms 的伪数据）。
9. **缺失文件应快速失败**：`loadFailed` 属性 + QML `Qt.quit()` 让自测路径在 0.5s 内退出，而不是干等 10s 看门狗。
10. **MSVC 必须带 `/utf-8`**：`qmlcachegen` 把含中文的 `main.qml` 生成 `.cpp`（UTF-8 字节），`cl.exe` 不带 `/utf-8` 按 GBK 解析会把字符串字面量中的多字节字符切断，报 `C2001 常量中有换行符`。在 `scripts/env.ps1` 设 `CFLAGS/CXXFLAGS=/utf-8` 解决。
11. **本 Qt 6.8.3 + cxx-qt 0.10（qmlcachegen 编译 QML）的两个对象创建坑**：(a) 给内建 `MouseArea` 赋任何 `active:` 值都会 `objectCreationFailed`（exit 2），必须用 `enabled:` 替代；(b) 给 `TextInput` 赋内联 `background: Rectangle{}`（即使空对象）也会 `objectCreationFailed`，必须改用「兄弟 `Rectangle` 垫底 + 裸 `TextInput`」。两者都是合法运行时 QML，属 qmlcachegen 编译路径的缺陷，经约 15 轮 build+run 二分确认。
12. **`run_script` 分隔符 `;`**：参数化 op（`crop:x,y,w,h`）与旧的 `,` 分隔冲突，改为 `split(';')`；参数之间仍用 `,`。
13. **cxx-qt 变更信号/处理器命名沿用 snake_case**：`#[qproperty(i32, image_width)]` 的 QML 变更信号是 `image_widthChanged`，处理器写 `onImage_widthChanged`（首字母大写 + snake 原样 + `Changed`），**不做 camelCase**（同 §5 第 2 条）。
14. **match 臂是表达式体**：`match guard.as_mut() { Some(s) => { ... } None => ... }` 的臂里不能用裸 `return` 或 `?`（会报类型错/E0308）。需要在臂内用 `if/else` 表达式或嵌套 `match` 把值算出来，`return`/`?` 放到外层函数体。
15. **QML 图层数据必须走 qproperty 而非 invokable**：`Repeater` 的 delegate 若用 `bridge.layer_visible(i)`（invokable，非响应式）取值不会随状态刷新。改为暴露 `layers_json`（qproperty，JSON 数组），QML `readsonly property var layerList: JSON.parse(bridge.layers_json)` + `Repeater{model: layerList}`——qproperty 变化即重算 model、重建 delegate，天然响应。
16. **跑 imageedit.exe 必须先 `. .\scripts\env.ps1`**（把 Qt `bin` 加进 PATH）。忘记加载时进程无输出直接崩 `0xC0000135`（0xC0000005/135 视缺 DLL 组合），极易误判为代码 bug；构建（`cargo build`）同样依赖此 PATH。

## 6. 已知限制

1. **预览通道仍是「Rust 编码 PNG → 临时文件 → Qt 读回」**（阶段 0 技术债 #1）。预览长边上限 4096px，因此 50MP 图在缩放 >50% 时看到的是降采样结果，不是全分辨率像素。
2. **大图解码无降采样优先**：50MP JPEG 全量解码约 600ms，占 large 冷启动的一半以上。
3. **多窗口未实现**：同进程只能打开一个文档；打开新文件会替换当前文档（未加「未保存改动」确认）。
4. **选区图层 UI**：框选提取、图层面板（选择/显隐/上下移/删除）、移动/方向键微调已实现；**剩**：跨图复制粘贴、非破坏性「删除选区像素」（core 侧无此 op）、移动图层拖拽的实时预览（当前松手一次性提交）、图层不透明度/重命名。
5. **编辑器未提供「另存会覆盖原文件」的路径**：`save_as` 由用户在选择框里决定，覆盖原文件时无二次确认。
6. **`Image` 同步加载**：变换操作是「同步重渲染 + 同步重载预览」，大图连点旋转会有可见卡顿（主线程阻塞）。
7. **可视化验证缺失**：目前全部依赖像素采样与打点，未做截图目视确认（阶段 0 技术债 #7 仍未关闭）。
8. **`--hidden` / `--probe-dialog` 仍是诊断开关**，借 `Qt.application.arguments` 在 QML 内判断。

## 7. 下一步建议（按收益排序）

1. **预览通道直传**：`QQuickImageProvider` 或 `QQmlImageProviderBase` 交付纹理，去掉临时 PNG 与二次解码（预期大图缩放交互明显变顺、冷启动再降几十毫秒）。
2. **大图降采样优先解码**：接入 libjpeg-turbo scaled decode（`jpeg_decompress` 的 scale_num/scale_denom）与渐进显示——这是 large 突破 ≤1000ms 目标的唯一路径。
3. **裁剪手柄精修**：当前拖框裁剪已可用；可加选区缩放手柄、9 点锚点、像素吸附与「取消已选」浮层（复用 §2.2 的 `crop_to`）。
4. **选区图层增强**：框选/提取/图层面板/移动/方向键微调已交付（§2.3）；剩 **跨图复制粘贴**（`crates/core` 的跨文档剪贴板模型已就绪，缺 UI 与快捷键 `Ctrl+C/X/V`）、非破坏「删除选区像素」、图层不透明度/重命名、移动图层拖拽实时预览。
5. **主线程解耦**：解码/渲染搬进工作线程（`queued` 信号回主线程），消除大图操作卡顿。
6. **打包脚本纳入回归**：`scripts/package.ps1` 已含自检，建议连同 `imageedit-bench ui` 门槛检查一起接入 CI。
