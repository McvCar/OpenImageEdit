# PSD 保真渲染 — 交接说明

> 更新：2026-11-15。目标稿：`D:/_APP_CODE/_XhCompany/FontGame/Res/0302-界面UI-1.psd`（750×1700，805 个图层样式，PS 存的合并图为参考真值 `tmp/ref.png`）。
>
> 最新 commit：`2652e5f`（P0.9，已推 hub/master），工作区干净。

## 1. 现状

MAE 轨迹（vs PS 合并图，`tmp/mae.py` 测）：

```
10.67（分层编辑刚通）→ 9.24（P0 保真：蒙版+剪贴）→ … → 7.216（P0.8 蒙版矩形）
→ 5.370（P0.9 组级样式烘焙）；差>32 像素 6.56% → 5.49%
```

- 路线图与每步归因：`docs/PSD保真编辑-改造方案.md`（§4.1 归因表、P0.x 节）。
- 测量：`tmp/mae.py <psd> <flat.png>`、`tmp/probe_px.py <psd> <x> <y> <flat.png>`（单点层栈）、`scripts/psd_fidelity_report.py --attribute 3`（块归因+样式实例统计）。

## 2. 架构一页纸

- **解码**：`crates/decode/psd_layers.rs`（分层装配）→ `psd_effects.rs`（样式烘焙，吃独立 alpha 平面）→ `psd_vector.rs`（vmsk 栅格化 + 矢量填充）。
- **vendor**：`vendor/psd/`（自维护 fork）。`PsdGroup` 保留 folder record 的 `layer_effects`；SoCo/GdFl sheet fill 已解析。
- **核心**：`Layer.mask: Option<LayerMask>`、`Layer.clipping`；样式全部烘焙进位图（不 live）。
- **组级样式**（P0.9）：组联合 alpha（成员 over 累积，**剪贴成员必须跳过**）→ 投影/描边两张纯样式位图插进组层序区间；名字带「(烘焙)」，不写回 PSD。开关 `IMAGEEDIT_PSD_NO_GROUP_FX=1`。
- **写回**：`psd_write.rs` + `bridge/paths.rs`；透明占位背景不写；分组用 `PsdRecord::Divider` 自底向上。

## 3. 剩余工作清单（按预期收益排序）

### A. 层级样式近似误差 —— 剩余 MAE 绝对主项
把全部样式关掉 MAE 5.37→约 8+（P0 时测过 9.24→13.98）。已实现的是几何近似：
- 模糊半径、每效果混合模式/等高线（contour）/杂色目前近似或忽略；
- 调整层作用顺序近似。
方向：对 `psd_effects.rs` 逐效果补 contour 曲线映射（描述符里 `Trns` 等高线已能拿到）、InnerGlow 实时渐变（EDGE 来源 + blur=13 这类）重算而非用烘焙缓存。

### B. 蒙版/剪贴写回 PSD（编辑闭环缺口）
读已支持（P0），**写回不支持**：保存后蒙版/剪贴状态丢失，状态栏有明示。需在 `psd_write.rs` 输出 `&8` 蒙版通道 + `clipping` 位，并跑 `scripts/crosscheck_psd.py` 双向对照。

### C. 组级效果的其余类型
P0.9 只做了组投影 + 组描边。组级内阴影/内发光/颜色叠加/渐变叠加暂不支持（代码有注记）。基础设施（联合 alpha）已就位，扩展 `bake_group_effects` 的效果枚举即可。

### D. 矢量填充缺口（P0.7 遗留）
- 开放子路径（不闭合不填充——PS 语义待确认，当前直接跳过）；
- 奇偶填充（even-odd，双绕组当前按非零处理）；
- 图案填充（`PtFl`，本目标稿无实例）。
- vmsk 裁剪缓存过期层：**规则不可调和**（按钮按路径、气泡尾巴按缓存，行为相反），`clip_shape` 只留实验开关 `IMAGEEDIT_PSD_VCLIP=1`，默认关。除非找到可判别的元数据，不要轻易全局启用。

### E. 图案叠加、斜面和浮雕
不支持（本目标稿 805 个样式里没有这两类，优先级低，但换素材就会出现）。

## 4. 验证流程（改码必跑）

```
构建：. .\scripts\env.ps1; cargo build -p imageedit-ui   （别 --workspace）
测试：cargo test -p imageedit-decode / -p imageedit-core / -p imageedit-render
      cargo test --manifest-path vendor/psd/Cargo.toml --lib
探针：scripts/crosscheck_psd.py（改 vendor 必跑，30 份双向对照）
      scripts/probe_psd_groups.py（分组读写）
      scripts/probe_psd_layers.py（编辑锁 5 条路径）
      scripts/probe_psd_roundtrip.py、probe_psd_perf.py --check
测 MAE：export PATH="/d/Qt/6.8.3/msvc2022_64/bin:$PATH"
      ./target/debug/imageedit.exe <psd> --probe-layers --script=dump-flat
      python tmp/mae.py <psd> tmp/flat.png
```

- 改 PSD 读写语义 → 必跑 `crosscheck_psd.py`（读写共用错误语义的坑自测查不出）。
- 改分组读写 → 必跑 `probe_psd_groups.py`。
- vendor 的 PSD visible bit 是反的（bit1 置位=隐藏），已修，别再踩。

## 5. 已知坑速查

1. **剪贴成员不能进组联合 alpha**（首版没跳，MAE 炸到 9.902）。
2. `decode_psd_bytes` 签名/行为不能改（thumbnail DLL 复用）。
3. vendor 蒙版 `-2` 通道 RLE 行表按**蒙版矩形**高度剥，不是图层高度。
4. SoCo/GdFl 负载 = u32 版本号 + 描述符本体（先跳 4 字节）。
5. psd-tools 对组级效果、缓存过期 shape 层都渲染不出来——它**不是**这类问题的参照，别再拿它对答案。
6. 1px 描边环边缘像素半覆盖（band 语义），透明底上 RGB 预乘减半，单测用 alpha 判据。
7. `--script=` 同步路径恒不锁编辑；探针里截不到保真态面板提示条（自动化启动不走保真模式）。
8. 单测/沙箱拦测试 exe 时用 Bash 直跑 `cargo test`。
