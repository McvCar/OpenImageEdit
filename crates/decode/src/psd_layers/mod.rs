//! PSD layer parsing for B-plan layered editing.
//!
//! This module is deliberately separate from `decode_psd_bytes`: the Explorer
//! thumbnail DLL calls that function for every directory listing, so it must
//! stay a "composite image only" fast path (C1). Layer parsing is only started
//! by the second-stage background job when the user actually opens a PSD.
//!
//! The vendored `psd` crate allocates `width * height * 4` bytes for **every**
//! layer in `PsdLayer::rgba()`. We instead expand the planar channels directly
//! into the layer rectangle (C3), then crop to the canvas. For a 4000x3000
//! document with ten 10%-area layers this is ~48 MB instead of ~480 MB.

use image::RgbaImage;
use psd::PsdChannelKind;

use crate::{DecodeError, DecodeOptions};

mod adjustments;
mod group_styles;
mod groups;
mod masks;
mod styles;
#[cfg(test)]
mod tests;
mod validate;

use adjustments::{adjust_scope_mode, apply_adjustment, adjustment_scope, scope_mode_name};
use group_styles::bake_group_style_layers;
use groups::{ancestor_is_annotation_group, inherited_group_state};
use masks::{channel_plane, decode_mask, mask_at, mask_probe, mask_probe_at};
pub(crate) use masks::unpack_packbits;
use styles::bake_layer_styles;
use validate::{check_header, validate_psd_layer_records};

/// One editable PSD layer. `image` only covers the layer rectangle after
/// cropping to the canvas; `offset` is where that rectangle starts.
#[derive(Debug, Clone)]
pub struct PsdLayerData {
    pub name: String,
    pub offset: (i32, i32),
    pub image: RgbaImage,
    pub visible: bool,
    pub opacity: f32,
    /// Blend mode of the layer record (already mapped onto what `core` renders).
    pub blend: editcore::BlendMode,
    /// The layer sits inside a group Photoshop's author used for annotations
    /// ("标注", "备注", ...). Those carry measurements and colour codes, not
    /// artwork, so the UI can hide them on request.
    pub annotation: bool,
    /// Per-layer "something was simplified" notes.
    pub degraded: Vec<String>,
    /// PSD 里这层所属的文件夹（u32 是 PSD 自己的组编号，装配时再映射）。
    pub group: Option<u32>,
    /// 图层蒙版（`-2` 通道），已按蒙版自己的矩形裁剪到画布范围。
    pub mask: Option<editcore::LayerMask>,
    /// 该层是剪贴蒙版（只在下层基底的不透明区域内可见）。
    pub clipping: bool,
    /// 烘焙图层样式**之前**的 alpha 平面，见 [`ContentAlpha`]。
    ///
    /// Photoshop 的剪贴调整层只受基底「内容」的 alpha 限制：样式额外画出来的
    /// 像素（外描边、投影）不算内容，所以剪到按钮上的色相/饱和度会把按钮变色、
    /// 却把它的描边留在原色。没有这一层信息就只能按烘焙后的 alpha 平均对待，
    /// 描边会被一起改色（实测把 #4a0621 的描边刷成洋红）。
    /// 只有带样式的层才需要，没有样式时是 `None`（不占内存）。
    pub content_alpha: Option<ContentAlpha>,
    /// 同一份「内容」alpha，换成剪贴基底要的覆盖率平面。
    ///
    /// 剪贴蒙版同样只认基底的内容：外描边 / 投影不算边界。拿烘焙后的 alpha
    /// 当边界，剪贴内容会铺到描边上（实测黄色斜纹越过圆角、压上 8px 粉描边）。
    pub clip_shape: Option<editcore::LayerMask>,
    /// 烘焙样式画在**内容之外**的笔迹，连同样式自己的混合模式（PSD `Md  `）。
    /// 那些像素在合成时要按样式模式与下层混合，见 [`editcore::StylePixels`]。
    pub style: Option<editcore::StylePixels>,
    /// 混合颜色带（PSD blending ranges + `lclr`），全默认 = `None`。
    pub blend_if: Option<editcore::BlendIf>,
    /// 挖空（PSD `knko`），默认关闭。
    pub knockout: editcore::Knockout,
    /// 透明形状图层（PSD `tsly`），默认开。
    pub transparency_shapes: bool,
}

/// 烘焙样式前的 alpha 平面（1 字节/像素，行优先）连同它自己的矩形。
///
/// 必须带上矩形：`bake` 会为外描边/投影把位图**扩边**，扩出来的那圈像素在
/// 原图上根本没有对应值。只记 `Vec<u8>` 再按新尺寸取下标就是越界 panic
/// （曾经真的这么挂过一次，整个 PSD 退化成合并预览）。
#[derive(Debug, Clone)]
pub struct ContentAlpha {
    pub alpha: Vec<u8>,
    pub offset: (i32, i32),
    pub size: (u32, u32),
}

impl ContentAlpha {
    /// 画布坐标处的覆盖率；落在原矩形外就是 0（那里本来什么都没有）。
    pub fn at(&self, x: i32, y: i32) -> f32 {
        let (ox, oy) = self.offset;
        let (x, y) = (x - ox, y - oy);
        if x < 0 || y < 0 || x >= self.size.0 as i32 || y >= self.size.1 as i32 {
            return 0.0;
        }
        self.alpha[y as usize * self.size.0 as usize + x as usize] as f32 / 255.0
    }

    /// 「内容」平面 = 样式烘焙前的 alpha 扣掉样式自己画上去的那一份。
    ///
    /// 两个坐标系统不一样：`content` 是烘焙**前**的矩形，`size` 是烘焙后
    /// （可能被外描边/投影扩过边）的矩形，所以逐像素按画布坐标对齐。
    fn from_baked(
        content: &ContentAlpha,
        offset: (i32, i32),
        size: (u32, u32),
        effect_coverage: &[u8],
    ) -> ContentAlpha {
        let (w, h) = size;
        let mut plane = Vec::with_capacity((w as usize) * (h as usize));
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let a = content.at(offset.0 + x, offset.1 + y);
                let c = effect_coverage[(y as usize) * (w as usize) + x as usize] as f32 / 255.0;
                plane.push((a * (1.0 - c.clamp(0.0, 1.0)) * 255.0).round().clamp(0.0, 255.0) as u8);
            }
        }
        ContentAlpha {
            alpha: plane,
            offset,
            size,
        }
    }
}

/// One PSD folder. `parent` / `psd_id` 用 PSD 自己的编号，`PsdLayerData::group`
/// 通过同样的编号指回来。
#[derive(Debug, Clone)]
pub struct PsdGroupData {
    pub psd_id: u32,
    pub parent: Option<u32>,
    pub name: String,
    pub collapsed: bool,
    pub visible: bool,
    /// 组内成员数（含嵌套组的成员），面板「N 项」与空组判定用它。
    pub count: usize,
    /// 组自己的混合模式（`pass` = 穿透，此时与 Normal 等价，见 `pass_through`）。
    pub blend: editcore::BlendMode,
    /// 组不透明度（0..=1），合成时整组一次性应用。
    pub opacity: f32,
    /// PSD 的 `pass`：成员直接与组下方的画面混合，不隔离。
    pub pass_through: bool,
}

/// Result of the second-stage parse. `layers` is bottom -> top.
#[derive(Debug, Clone)]
pub struct PsdLayerSet {
    pub layers: Vec<PsdLayerData>,
    /// 文件夹（PSD 的组）。空 = 文件里没有分组，或为兼容旧路径而忽略。
    pub groups: Vec<PsdGroupData>,
    pub canvas: (u32, u32),
    /// File-level degradation notes (groups flattened, blend modes, ...).
    pub degraded: Vec<String>,
    /// 带图层蒙版的层数 / 剪贴蒙版层数（诊断与探针用，二者现已正确参与合成）。
    pub masked: usize,
    pub clipped: usize,
    /// Whether the file carries a flattened composite (stage 1 uses it).
    pub has_composite: bool,
    /// 内嵌 ICC profile（图像资源 1039 的原始字节）。
    ///
    /// `decode` 路径在下面已经把图层位图转到 sRGB；`display` 路径把这个字节
    /// 流交给 UI 建「文档空间 -> sRGB」变换，合成完成后再转。
    pub icc_profile: Option<Vec<u8>>,
    /// Total bytes held by the layer bitmaps, for the memory budget probes.
    pub bytes: usize,
}

impl PsdLayerSet {
    /// Indices (into `layers`, bottom -> top) of the annotation layers.
    pub fn annotation_indices(&self) -> Vec<usize> {
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.annotation)
            .map(|(i, _)| i)
            .collect()
    }
}

impl PsdLayerSet {
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Convert into `core` layer descriptors. Consumes `self` so the bitmaps
    /// are moved, not cloned (C3: 16 MB instead of another 16 MB copy).
    /// Read `canvas` / `degraded` before calling this.
    pub fn into_seeds(self) -> Vec<editcore::PsdLayerSeed> {
        self.layers
            .into_iter()
            .map(|l| editcore::PsdLayerSeed {
                name: l.name,
                offset: l.offset,
                image: l.image,
                visible: l.visible,
                opacity: l.opacity,
                blend: l.blend,
                group: l.group,
                mask: l.mask,
                clipping: l.clipping,
                clip_shape: l.clip_shape,
                style: l.style,
                blend_if: l.blend_if,
                knockout: l.knockout,
                transparency_shapes: l.transparency_shapes,
            })
            .collect()
    }

    /// 文件夹树 -> core 的组种子（不含位图，先取它再 `into_seeds`）。
    pub fn group_seeds(&self) -> Vec<editcore::PsdGroupSeed> {
        self.groups
            .iter()
            .map(|g| editcore::PsdGroupSeed {
                psd_id: g.psd_id,
                parent: g.parent,
                name: g.name.clone(),
                collapsed: g.collapsed,
                visible: g.visible,
                blend: g.blend,
                opacity: g.opacity,
                pass_through: g.pass_through,
            })
            .collect()
    }
}

/// Parse the layers of a PSD. Signature and behaviour of `decode_psd_bytes`
/// are untouched.
pub fn decode_psd_layers(bytes: &[u8], options: DecodeOptions) -> Result<PsdLayerSet, DecodeError> {
    decode_psd_layers_with_progress(bytes, options, |_, _| {})
}

/// Same as [`decode_psd_layers`] but reports `(done, total)` after each layer so
/// the UI can show "已加载 k/N 层" while the editing lock is held (C2-a).
pub fn decode_psd_layers_with_progress<F>(
    bytes: &[u8],
    options: DecodeOptions,
    progress: F,
) -> Result<PsdLayerSet, DecodeError>
where
    F: FnMut(usize, usize),
{
    let (width, height, channels, depth) = check_header(bytes, options)?;
    crate::validate_psd_sections(bytes, width, height, channels, depth)?;
    let mut file_notes = validate_psd_layer_records(bytes)?;
    if depth != 8 {
        file_notes.insert(
            0,
            format!("{depth} 位/通道已降级为 8 位，像素精度按 8 位保存"),
        );
    }

    // The vendored parser still has unchecked slices. Everything it touches
    // (including our own channel expansion) stays inside catch_unwind (C5).
    let mut progress = progress;
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let t_parse = std::time::Instant::now();
        let psd = psd::Psd::from_bytes(bytes)
            .map_err(|e| format!("PSD 分层解析失败：{e:?}"))?;
        let parse_ms = t_parse.elapsed().as_millis();
        let set = build_layers(&psd, (width, height), &mut progress)?;
        if std::env::var_os("IMAGEEDIT_PSD_PROFILE").is_some() {
            println!(
                "{{\"phase\":\"ui\",\"event\":\"psd-decode-split\",\"parse_ms\":{parse_ms},\"build_ms\":{}}}",
                t_parse.elapsed().as_millis().saturating_sub(parse_ms)
            );
        }
        Ok(set)
    }));
    match built {
        Ok(Ok(mut set)) => {
            // File-level notes go first: they explain why a layer may be empty.
            set.degraded.splice(0..0, file_notes);
            dedup(&mut set.degraded);
            Ok(set)
        }
        Ok(Err(msg)) => Err(DecodeError::Psd(msg)),
        Err(_) => Err(DecodeError::Psd(
            "无法解析此 PSD 的图层数据（文件数据异常）".into(),
        )),
    }
}

fn build_layers<F: FnMut(usize, usize)>(
    psd: &psd::Psd,
    canvas: (u32, u32),
    progress: &mut F,
) -> Result<PsdLayerSet, String> {
    let (cw, ch) = (canvas.0 as i32, canvas.1 as i32);
    let source = psd.layers();
    let total = source.len();
    progress(0, total);
    let mut layers = Vec::with_capacity(total);
    // Whether each already-pushed layer is a clipping mask — needed to scope an
    // adjustment layer onto the chain it clips to.
    let mut clips: Vec<bool> = Vec::with_capacity(total);
    let mut degraded: Vec<String> = Vec::new();
    let mut total_bytes = 0usize;
    let mut baked_adjustments = 0usize;
    let mut styled_layers = 0usize;
    let mut masked_layers = 0usize;
    let mut clipped_layers = 0usize;
    let mut vector_filled = 0usize;
    let mut vector_clipped = 0usize;
    let mut vector_stroked = 0usize;
    let groups = psd.groups();
    // 图案资源（`Patt` / `Pat2` / `Pat3`）：形状填充按图案 ID 取像素。
    let patterns = crate::psd_pattern::PatternStore::from_resources(psd.resources());
    let mut inherited_hidden = false;
    // 诊断开关：把某一项特性关掉再合成，用 `dump-flat` 与 PS 合并图比对，
    // 就能量出「这一项到底贡献了多少误差」。只在排查时用，平时全开。
    let off = |k: &str| std::env::var(k).ok().as_deref() == Some("1");
    let no_styles = off("IMAGEEDIT_PSD_NO_STYLES");
    let no_masks = off("IMAGEEDIT_PSD_NO_MASKS");
    let no_clip = off("IMAGEEDIT_PSD_NO_CLIP");
    let no_adjust = off("IMAGEEDIT_PSD_NO_ADJUST");

    // Bottom -> top: the vendor hands records back top -> bottom (it reverses
    // them on read), and `PsdLayerSet::layers` is documented as bottom -> top.
    // Iterating in the target order here means an adjustment layer can be baked
    // into the layers it actually sits above, which is only knowable in this
    // direction.
    for (step, idx) in (0..total).rev().enumerate() {
        let layer = &source[idx];
        let left = layer.layer_left();
        let top = layer.layer_top();
        let right = layer.layer_right();
        let bottom = layer.layer_bottom();
        let raw_w = (right as i64 - left as i64 + 1).max(1);
        let raw_h = (bottom as i64 - top as i64 + 1).max(1);
        let mut notes: Vec<String> = Vec::new();

        // ---- adjustment layers -----------------------------------------
        // Brightness/contrast and hue/saturation are not pixels: Photoshop
        // applies them to everything below (or, when clipped, to the layer the
        // chain clips to). We have no live adjustment layer concept, so bake
        // the effect into those layers and keep the document pixel-exact.
        let bc = layer.brightness_contrast();
        let hs = layer.hue_saturation();
        if bc.is_some() || hs.is_some() {
            if !no_adjust && layer.visible() && layer.opacity() > 0 {
                let clipped = layer.is_clipping_mask();
                let start = adjustment_scope(
                    &layers,
                    &clips,
                    layer.parent_id(),
                    clipped,
                    adjust_scope_mode(),
                );
                // An adjustment's own mask decides where it applies: a hue shift
                // painted into half of a button must not recolour the other half.
                // Its pixels are irrelevant, but the `-2` channel is not.
                let mask = if no_masks {
                    None
                } else {
                    decode_mask(layer, layer.channel_bytes(), cw, ch)
                };
                // 诊断：作用域算错时（剪贴调整层改了不该改的图层）只能靠打印
                // 目标区间来定位，像素差本身分不清「范围错」还是「公式错」。
                if std::env::var("IMAGEEDIT_PSD_TRACE_ADJUST").ok().as_deref() == Some("1") {
                    let head: Vec<String> = layers[start..]
                        .iter()
                        .take(3)
                        .map(|l| format!("{}#{:?}", l.name, l.group))
                        .collect();
                    println!(
                        "{{\"phase\":\"decode\",\"event\":\"adjust\",\"name\":\"{}\",\
                         \"parent\":{:?},\"clip\":{},\"mode\":\"{}\",\"start\":{},\"len\":{},\
                         \"targets\":{:?}}}",
                        layer.name(),
                        layer.parent_id(),
                        clipped,
                        scope_mode_name(adjust_scope_mode()),
                        start,
                        layers.len(),
                        head
                    );
                }
                apply_adjustment(&mut layers[start..], bc, hs, mask.as_ref(), clipped);
                baked_adjustments += 1;
            }
            progress(step + 1, total);
            continue;
        }

        // ---- degradation contract (§7) ---------------------------------
        let blend = map_blend_mode(layer.blend_mode());
        if blend.is_none() {
            notes.push(format!("混合模式 {:?} 不支持，已按正常处理", layer.blend_mode()));
        }
        let blend = blend.unwrap_or_default();
        // 混合颜色带（`lclr` 口径位 + 每组 4 字节的范围表）。全默认时丢掉，
        // 免得每层都挂一份不起作用的表。
        let blend_if = layer_blend_if(layer);
        let knockout = match layer.knockout() {
            Some(psd::Knockout::Shallow) => editcore::Knockout::Shallow,
            Some(psd::Knockout::Deep) => editcore::Knockout::Deep,
            None => editcore::Knockout::None,
        };
        // 组不再摊平：整棵树由 `psd.groups()` 交出去（见函数末尾），这里
        // 只把「这层属于哪个组」记在层上，让面板能还原缩进与折叠。
        // A group's own visibility and opacity live on the folder record, not on
        // its children: Photoshop keeps the children's flags untouched and hides
        // them through the group. The canvas must still match the composite, so
        // every ancestor group's flag is folded into the child here; the group
        // row keeps its own state for the panel.
        let (ancestor_visible, _ancestor_opacity) =
            inherited_group_state(groups, layer.parent_id());
        if !ancestor_visible && layer.visible() {
            inherited_hidden = true;
        }
        let clipping = !no_clip && layer.is_clipping_mask();
        if clipping {
            clipped_layers += 1;
        }
        if left < 0 || top < 0 || right + 1 > cw || bottom + 1 > ch {
            notes.push("层越出画布，已按画布裁剪".into());
        }

        // ---- clip the layer rectangle to the canvas --------------------
        let x0 = left.max(0);
        let y0 = top.max(0);
        let x1 = ((right as i64 + 1).min(cw as i64)) as i32;
        let y1 = ((bottom as i64 + 1).min(ch as i64)) as i32;
        if x1 <= x0 || y1 <= y0 {
            notes.push("层完全在画布外，已跳过".into());
            degraded.extend(notes.iter().cloned());
            progress(step + 1, total);
            continue;
        }
        let src_x = (x0 - left) as u32;
        let src_y = (y0 - top) as u32;
        // A layer record may claim a rectangle larger than the pixel data we
        // keep; clamp the crop against the channel extent so crop_imm can
        // never index outside `full`.
        let avail_w = (raw_w.saturating_sub(src_x as i64)).max(0) as u32;
        let avail_h = (raw_h.saturating_sub(src_y as i64)).max(0) as u32;
        let crop_w = ((x1 - x0) as u32).min(avail_w);
        let crop_h = ((y1 - y0) as u32).min(avail_h);
        if crop_w == 0 || crop_h == 0 {
            notes.push("层位图数据不足，已跳过".into());
            degraded.extend(notes.iter().cloned());
            progress(step + 1, total);
            continue;
        }

        // Expand the channel planes at their **own** rectangle size, then crop
        // the canvas-visible part out of it.
        //
        // ★ Reading them at the cropped size instead used to shear the layer:
        //   a plane is `raw_w` bytes per row, so decoding it as `crop_w` bytes
        //   per row shifts every row further right. A shape layer that pokes a
        //   few pixels outside the canvas (a decorative border) turned into a
        //   parallelogram of diagonal red lines.
        let raw_pixels = raw_w as u64 * raw_h as u64;
        if raw_pixels > 300_000_000 {
            notes.push("层尺寸过大，已按空层处理".into());
            degraded.extend(notes.iter().cloned());
            progress(step + 1, total);
            continue;
        }
        let raw_pixels = raw_pixels as usize;

        let chans = layer.channel_bytes();
        let red = channel_plane(chans, PsdChannelKind::Red, raw_pixels);
        let green = channel_plane(chans, PsdChannelKind::Green, raw_pixels);
        let blue = channel_plane(chans, PsdChannelKind::Blue, raw_pixels);
        let alpha = channel_plane(chans, PsdChannelKind::TransparencyMask, raw_pixels);
        let Some(red) = red else {
            return Err(format!("PSD 图层「{}」缺少颜色通道", layer.name()));
        };
        // Grayscale PSDs store a single channel; mirror generate_rgba().
        let green = green.unwrap_or_else(|| red.clone());
        let blue = blue.unwrap_or_else(|| red.clone());
        let mut rgba = vec![0u8; raw_pixels * 4];
        for i in 0..raw_pixels {
            rgba[i * 4] = red[i];
            rgba[i * 4 + 1] = green[i];
            rgba[i * 4 + 2] = blue[i];
            rgba[i * 4 + 3] = alpha.as_ref().map(|a| a[i]).unwrap_or(255);
        }
        let full = RgbaImage::from_raw(raw_w as u32, raw_h as u32, rgba)
            .ok_or_else(|| "层位图尺寸异常".to_string())?;
        let image = image::imageops::crop_imm(&full, src_x, src_y, crop_w, crop_h).to_image();
        let mut image = image;
        let mut layer_offset = (x0, y0);

        // ---- 形状矢量描边的扩边 ---------------------------------------
        // 外描边 / 居中描边会落在路径之外，而形状层的通道位图止于路径
        // 包围盒 —— 不留这圈边，描边外半圈会被直接裁掉。
        if !off("IMAGEEDIT_PSD_NO_VECTOR") {
            if let Some(st) = layer.stroke_style() {
                if st.enabled && st.width > 0.0 {
                    let reach = match st.align {
                        psd::VectorStrokeAlign::Outside => st.width.ceil() as i32 + 1,
                        psd::VectorStrokeAlign::Center => (st.width / 2.0).ceil() as i32 + 1,
                        psd::VectorStrokeAlign::Inside => 0,
                    };
                    if reach > 0 {
                        let mut padded = image::RgbaImage::new(
                            image.width() + 2 * reach as u32,
                            image.height() + 2 * reach as u32,
                        );
                        image::imageops::overlay(&mut padded, &image, reach as i64, reach as i64);
                        image = padded;
                        layer_offset.0 -= reach;
                        layer_offset.1 -= reach;
                    }
                }
            }
        }

        // ---- 形状图层的矢量填充（`vmsk` / `vscg`）------------------------
        // 形状层的像素由 PS 按矢量路径实时生成。两类洞都要补：① 通道完全
        // 为空的层（实测目标稿 5 张卡片的渐变描边整圈丢失）；② 缓存接近
        // 覆盖整个路径的实心形状层（路径比缓存大一圈，边缘渐变丢失）。
        // fill 内部只补透明像素、不覆盖缓存。没有 vscg 或覆盖率低的层
        // （描边环 / 纹理）不受影响。
        if !off("IMAGEEDIT_PSD_NO_VECTOR") {
            if let Some(vm) = layer.vector_mask() {
                // 一次遍历同时算出「全空」与「缓存覆盖率」。两类层需要补洞：
                // ① 全空（通道没缓存，5 张卡片渐变描边整圈丢失）；② 缓存
                // 几乎填满路径的**实心形状**（实测 `矩形 556 拷贝` 覆盖率
                // 99.4%，矢量路径比缓存大一圈，边缘渐变整条缺失）。覆盖率
                // 低的层（描边环 18%、纹理 0.8%）透明是有意义的内容，
                // 绝不能拿 vscg 去填 —— 实测整幅填了 MAE 从 7.2 炸到 128。
                let mut opaque = 0usize;
                let mut total = 0usize;
                for p in image.pixels() {
                    total += 1;
                    if p.0[3] > 0 {
                        opaque += 1;
                    }
                }
                let empty = opaque == 0;
                let cached_shape = opaque as f32 / total.max(1) as f32 > 0.8;
                let has_fill = layer.vector_fill().is_some();
                let trace =
                    std::env::var("IMAGEEDIT_PSD_VECTOR_TRACE").ok().as_deref() == Some("1");
                if has_fill
                    && (empty || cached_shape)
                    && crate::psd_vector::fill_shape(
                        &mut image,
                        layer_offset,
                        (cw as u32, ch as u32),
                        vm,
                        layer.vector_fill(),
                        Some(&patterns),
                    )
                {
                    vector_filled += 1;
                    if trace {
                        println!(
                            "{{\"phase\":\"decode\",\"event\":\"vector-fill\",\"name\":\"{}\",\
                             \"offset\":[{},{}],\"size\":[{},{}],\"groups\":{},\"hole\":{}}}",
                            layer.name(),
                            layer_offset.0,
                            layer_offset.1,
                            image.width(),
                            image.height(),
                            vm.subpaths.len(),
                            !empty
                        );
                    }
                } else if trace && !empty {
                    println!(
                        "{{\"phase\":\"decode\",\"event\":\"vector-skip\",\"name\":\"{}\",\
                         \"why\":\"has-raster\",\"has_fill\":{},\"cov\":{:.3}}}",
                        layer.name(),
                        has_fill,
                        opaque as f32 / total.max(1) as f32
                    );
                }
                // ---- 缓存过期裁剪（仅当路径≈图层轮廓）-----------------
                // 形状层的显示 = 内容 ∩ 矢量路径。这张稿子里有两类形状层，
                // PS 合并图对它们的行为相反，必须分开对待：
                //   · 路径就是图层轮廓（`矩形 559 拷贝`：缓存是直角矩形、
                //     路径是圆角，PS 显示时圆角外被切掉）→ 按路径裁；
                //   · 路径只是层内子区域（`椭圆 623` 的气泡尾巴画在椭圆
                //     路径之外）→ 缓存才是正确内容，裁了会切掉尾巴。
                // 判据：路径包围盒四边相对图层矩形内缩都不超过 2.5px。
                // 旧的全局裁剪（psd-tools 同款）净 MAE 5.37→6.44，就是被
                // 第二类层拖累的；这个判据把两者分开。
                let path_is_outline = layer.layer_effects().is_none()
                    && crate::psd_vector::path_bounds(vm, (cw as u32, ch as u32))
                        .map(|b| {
                            b[0] - x0 as f32 <= 2.5
                                && b[1] - y0 as f32 <= 2.5
                                && x1 as f32 - b[2] <= 2.5
                                && y1 as f32 - b[3] <= 2.5
                        })
                        .unwrap_or(false);
                if path_is_outline && !off("IMAGEEDIT_PSD_NO_VCLIP") {
                    if crate::psd_vector::clip_shape(
                        &mut image,
                        layer_offset,
                        (cw as u32, ch as u32),
                        vm,
                    ) {
                        vector_clipped += 1;
                    }
                }
                // ---- 矢量描边（`vstk`）---------------------------------
                // 只补**通道全空**的层。有缓存内容的层，描边已经在像素里了
                // （PS 把形状的实时渲染烘焙进了通道）；再按路径画一遍会多出
                // 一条带 —— 实测 `矩形 13`（全画布细边框）路径的内沿比缓存
                // 靠内 1.5px，右内边多画了一列纯红，整幅 MAE 因此 +0.41。
                if !off("IMAGEEDIT_PSD_NO_VSTROKE") && empty {
                    if let Some(st) = layer.stroke_style() {
                        let painted = crate::psd_vector::stroke_shape(
                            &mut image,
                            layer_offset,
                            (cw as u32, ch as u32),
                            vm,
                            st,
                        );
                        if painted {
                            vector_stroked += 1;
                        }
                        if std::env::var_os("IMAGEEDIT_PSD_VSTROKE_TRACE").is_some() {
                            println!(
                                "{{\"phase\":\"decode\",\"event\":\"vstroke\",\"name\":\"{}\",\
                                 \"off\":[{},{}],\"size\":[{},{}],\"w\":{},\"align\":{:?},\
                                 \"op\":{},\"solid\":{:?},\"grad\":{},\"painted\":{}}}",
                                layer.name(),
                                layer_offset.0,
                                layer_offset.1,
                                image.width(),
                                image.height(),
                                st.width,
                                st.align,
                                st.opacity,
                                st.fill.solid,
                                st.fill.gradient.is_some(),
                                painted
                            );
                        }
                    }
                }
            }
        }

        // ---- 逐层像素探针 ---------------------------------------------
        // `IMAGEEDIT_PSD_TRACE_AT=x,y` 打印每个覆盖该点的图层的**解码后**像素，
        // 与 psd-tools 的 `layer.topil()` 逐层对照，就能判断差异来自解码还是
        // 合成。合成图对不上时，这一步能把问题从「整幅」缩到「某一层」。
        if let Some((px, py)) = mask_probe_at("IMAGEEDIT_PSD_TRACE_AT") {
            let lx = px - layer_offset.0;
            let ly = py - layer_offset.1;
            if lx >= 0 && ly >= 0 && lx < image.width() as i32 && ly < image.height() as i32 {
                let p = image.get_pixel(lx as u32, ly as u32).0;
                println!(
                    "{{\"phase\":\"decode\",\"event\":\"layer-at\",\"name\":\"{}\",\
                     \"px\":[{},{},{},{}],\"off\":[{},{}],\"size\":[{},{}],\"vis\":{}}}",
                    layer.name(),
                    p[0],
                    p[1],
                    p[2],
                    p[3],
                    layer_offset.0,
                    layer_offset.1,
                    image.width(),
                    image.height(),
                    layer.visible()
                );
            }
        }

        // ---- 单层位图导出 ---------------------------------------------
        // `IMAGEEDIT_PSD_DUMP_LAYER=<图层名子串>` 把解码后的位图写到临时目录，
        // 与 psd-tools 的 `layer.topil()` 逐像素对照。合成图的差异经常落在
        // 「某一层的位图本身就不对」上，这时候只比合成图是查不出来的。
        if let Ok(want) = std::env::var("IMAGEEDIT_PSD_DUMP_LAYER") {
            if !want.is_empty() && layer.name().contains(&want) {
                let safe: String = layer
                    .name()
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '_' })
                    .collect();
                let path = std::env::temp_dir()
                    .join(format!("imageedit-layer-{}-{}.png", safe, layer_offset.0));
                if image.save(&path).is_ok() {
                    println!(
                        "{{\"phase\":\"decode\",\"event\":\"dump-layer\",\"name\":\"{}\",\
                         \"path\":\"{}\",\"off\":[{},{}],\"size\":[{},{}]}}",
                        layer.name(),
                        path.display().to_string().replace('\\', "/"),
                        layer_offset.0,
                        layer_offset.1,
                        image.width(),
                        image.height()
                    );
                }
            }
        }

        // ---- layer mask (`-2`) -----------------------------------------
        // The mask channel is compressed against the mask's OWN rectangle,
        // which is usually the whole canvas while the layer is a small
        // sticker. Reading it at the layer rect shears the mask.
        let mask = if no_masks {
            None
        } else {
            decode_mask(layer, chans, cw, ch)
        };
        if mask.is_some() {
            masked_layers += 1;
        }
        // 诊断：`IMAGEEDIT_PSD_MASK_TRACE=1` 打印每个蒙版的矩形与取值分布。
        // 蒙版解错（全零 / 矩形错位）时合成图表现为「某一层整片消失」，而
        // 单看合成结果很难判断是蒙版、剪贴还是像素本身的问题。
        if std::env::var("IMAGEEDIT_PSD_MASK_TRACE").ok().as_deref() == Some("1") {
            let probe = mask_probe();
            if let Some(r) = layer.mask_rect() {
                println!(
                    "{{\"phase\":\"decode\",\"event\":\"mask-rect\",\"name\":\"{}\",\
                     \"ltrb\":[{},{},{},{}],\"wh\":[{},{}]}}",
                    layer.name(),
                    r.left,
                    r.top,
                    r.right,
                    r.bottom,
                    r.width(),
                    r.height()
                );
            }
            match &mask {
                Some(m) => {
                    let mut min = 255u8;
                    let mut max = 0u8;
                    let mut sum = 0u64;
                    let n = m.image.pixels().count() as u64;
                    for p in m.image.pixels() {
                        min = min.min(p.0[0]);
                        max = max.max(p.0[0]);
                        sum += p.0[0] as u64;
                    }
                    println!(
                        "{{\"phase\":\"decode\",\"event\":\"mask\",\"name\":\"{}\",\
                         \"rect\":\"{}x{}+{},{} \",\"off\":[{},{}],\"default\":{},\
                         \"min\":{},\"max\":{},\"mean\":{:.1},\"at\":{}}}",
                        layer.name(),
                        m.image.width(),
                        m.image.height(),
                        m.offset.0,
                        m.offset.1,
                        m.offset.0,
                        m.offset.1,
                        m.default,
                        min,
                        max,
                        sum as f64 / n.max(1) as f64,
                        mask_at(m, probe)
                    );
                }
                None => {
                    if layer.has_active_mask() {
                        println!(
                            "{{\"phase\":\"decode\",\"event\":\"mask-none\",\"name\":\"{}\",\
                             \"active\":true}}",
                            layer.name()
                        );
                    }
                }
            }
        }

        // ---- layer styles (lfx2) ---------------------------------------
        // Photoshop stores shadow / stroke / overlay as parameters; without
        // baking them the layer looks flat (a title loses its gradient fill,
        // a button loses its outline). `bake` renders them into the bitmap and
        // reports any style it could not draw.
        // 烘焙前留一份 alpha：剪贴调整层按「内容」限幅，见字段注释。
        let styled = bake_layer_styles(
            layer,
            image,
            layer_offset,
            (cw, ch),
            mask.as_ref(),
            &patterns,
            no_styles,
        );
        image = styled.image;
        layer_offset = styled.offset;
        notes.extend(styled.notes);
        let content_alpha = styled.content_alpha;
        let clip_shape = styled.clip_shape;
        let style = styled.style;
        if styled.baked {
            styled_layers += 1;
        }

        total_bytes += (image.width() as usize) * (image.height() as usize) * 4;

        degraded.extend(notes.iter().cloned());
        layers.push(PsdLayerData {
            annotation: ancestor_is_annotation_group(groups, layer.parent_id()),
            group: layer.parent_id(),
            name: layer.name().to_string(),
            offset: layer_offset,
            image,
            visible: layer.visible() && ancestor_visible,
            // 组不透明度不再乘进成员：合成时整组一次性应用（否则成员重叠处
            // 会叠两次，比 PS 更透）。
            opacity: layer.opacity() as f32 / 255.0,
            blend,
            degraded: notes,
            mask,
            clipping,
            content_alpha,
            clip_shape,
            style,
            blend_if,
            knockout,
            transparency_shapes: layer.transparency_shapes(),
        });
        clips.push(layer.is_clipping_mask());
        progress(step + 1, total);
    }

    dedup(&mut degraded);
    if inherited_hidden {
        // Say it out loud: the panel will show a layer whose own eye says
        // "visible" but which the canvas correctly hides.
        degraded.push("组已隐藏，其子层已跟随隐藏".into());
    }
    if baked_adjustments > 0 {
        degraded.push(format!(
            "{baked_adjustments} 个调整图层已烘焙进下方图层"
        ));
    }
    if styled_layers > 0 {
        degraded.push(format!(
            "{styled_layers} 个图层的样式（阴影/描边/渐变叠加等）已烘焙进图层像素"
        ));
    }
    if vector_filled > 0 {
        degraded.push(format!(
            "{vector_filled} 个形状图层按矢量路径栅格化（PSD 里这类层的通道是空的）"
        ));
    }
    if vector_clipped > 0 {
        degraded.push(format!(
            "{vector_clipped} 个形状图层按矢量路径重新裁剪（通道缓存与路径不一致）"
        ));
    }
    if vector_stroked > 0 {
        degraded.push(format!(
            "{vector_stroked} 个形状图层的描边按矢量路径栅格化（vstk，含只描边不填充的层）"
        ));
    }
    // ---- 文件夹树（PSD 的组，不再摊平）---------------------------------
    // `group_ids_in_order` 是 Photoshop 面板的顺序（底 -> 顶），照抄即可；
    // 成员数按**子树**统计（嵌套组的成员也算父组的一项），面板的「N 项」
    // 与空组判定都用它。
    let mut group_list: Vec<PsdGroupData> = Vec::with_capacity(groups.len());
    for gid in psd.group_ids_in_order() {
        let Some(group) = groups.get(gid) else {
            continue;
        };
        let mut count = 0usize;
        for layer in &layers {
            let mut next = layer.group;
            let mut guard = 0;
            while let Some(id) = next {
                if id == *gid {
                    count += 1;
                    break;
                }
                guard += 1;
                if guard > 64 {
                    break;
                }
                next = groups.get(&id).and_then(|g| g.parent_id());
            }
        }
        group_list.push(PsdGroupData {
            psd_id: *gid,
            parent: group.parent_id(),
            name: group.name().to_string(),
            collapsed: group.collapsed(),
            visible: group.visible(),
            count,
            blend: map_blend_mode(group.blend_mode()).unwrap_or(editcore::BlendMode::Normal),
            opacity: group.opacity() as f32 / 255.0,
            pass_through: matches!(group.blend_mode(), psd::BlendMode::PassThrough),
        });
    }

    // ---- 组级样式（folder record 上的 lfx2）----------------------------
    // Photoshop 对组只画一次样式（整个组合成结果的轮廓描一圈边、投一次影）。
    // 拆成两张位图层插进组的层序区间：投影插组底、描边插组顶。
    if !off("IMAGEEDIT_PSD_NO_GROUP_FX") {
        let (group_fx_count, group_fx_notes) =
            bake_group_style_layers(&mut layers, groups, (cw as u32, ch as u32));
        if group_fx_count > 0 {
            degraded.push(format!(
                "{group_fx_count} 个组的样式（投影/描边/内阴影/叠加）已烘焙进独立图层"
            ));
            // 插入的位图按实际尺寸重新计量（层循环只累加了原始层的字节）。
            total_bytes = layers
                .iter()
                .map(|l| {
                    let (w, h) = l.image.dimensions();
                    w as usize * h as usize * 4
                })
                .sum();
        }
        degraded.extend(group_fx_notes);
    }

    // ---- 内嵌 ICC -------------------------------------------------------
    // `decode` 路径：图层在这里就转成 sRGB，之后层间混合在 sRGB 里进行。
    // `display` 路径：只把 profile 原样交出去，合成完成后再转（贴近 PS 的
    // 顺序）；`off` 路径不建任何变换。
    let icc_profile = psd.icc_profile().map(|b| b.to_vec());
    if let (Some(bytes), editcore::IccMode::Decode) =
        (icc_profile.as_deref(), editcore::IccMode::from_env())
    {
        match editcore::IccTransform::new(bytes) {
            Some(xform) => {
                for layer in &mut layers {
                    xform.apply(&mut layer.image);
                }
            }
            None => degraded.push("内嵌 ICC 配置文件无法解析，已按文档色彩空间显示".into()),
        }
    }

    Ok(PsdLayerSet {
        layers,
        groups: group_list,
        canvas,
        degraded,
        masked: masked_layers,
        clipped: clipped_layers,
        // Stage 1 always has the composite from `decode_psd_bytes`; if it
        // failed the file would not have been opened at all.
        has_composite: true,
        icc_profile,
        bytes: total_bytes,
    })
}

fn dedup(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
}

/// 把 vendored 解析出的混合颜色带映射成 `core` 的 [`editcore::BlendIf`]。
///
/// 字节表的顺序是「合成源、合成目标，然后每个通道各一对（源/目标）」；
/// RGB 文件一共 10 组（2 + RGBA×2）。这里只取合成与 R/G/B 三对 —— alpha
/// 那对与逐像素的 alpha 遮罩重复，忽略即可。
///
/// PSD 没有单独存「灰度 / RGB」这个开关（`lclr` 是图层颜色标签，不是它）：
/// 灰度模式只写合成那两组，RGB 模式只写各通道那几组，所以「通道组是否
/// 非默认」就是判断依据。
fn layer_blend_if(layer: &psd::PsdLayer) -> Option<editcore::BlendIf> {
    let ranges = layer.blend_ranges()?;
    let range = |i: usize| {
        ranges
            .get(i)
            .copied()
            .map(editcore::BlendRange::from_psd)
            .unwrap_or_default()
    };
    let source_channels = [range(2), range(4), range(6)];
    let destination_channels = [range(3), range(5), range(7)];
    let rgb = source_channels
        .iter()
        .chain(destination_channels.iter())
        .any(|r| !r.is_default());
    let blend_if = editcore::BlendIf {
        source: range(0),
        destination: range(1),
        rgb,
        source_channels,
        destination_channels,
    };
    (!blend_if.is_noop()).then_some(blend_if)
}

/// Blend mode for anything outside a layer record (layer styles).
pub(crate) fn blend_of(mode: psd::BlendMode) -> editcore::BlendMode {
    map_blend_mode(mode).unwrap_or_default()
}

/// Map the parser's blend mode onto the subset `core` can render.
///
/// `None` means "not implemented": the caller falls back to Normal and records a
/// degradation note, so a file never silently changes appearance without saying
/// so. A pass-through folder record is Normal for the layer itself.
fn map_blend_mode(mode: psd::BlendMode) -> Option<editcore::BlendMode> {
    use editcore::BlendMode as M;
    Some(match mode {
        psd::BlendMode::PassThrough | psd::BlendMode::Normal => M::Normal,
        psd::BlendMode::Dissolve => M::Dissolve,
        psd::BlendMode::Multiply => M::Multiply,
        psd::BlendMode::Screen => M::Screen,
        psd::BlendMode::Overlay => M::Overlay,
        psd::BlendMode::Darken => M::Darken,
        psd::BlendMode::Lighten => M::Lighten,
        psd::BlendMode::ColorDodge => M::ColorDodge,
        psd::BlendMode::ColorBurn => M::ColorBurn,
        psd::BlendMode::LinearDodge => M::LinearDodge,
        psd::BlendMode::LinearBurn => M::LinearBurn,
        psd::BlendMode::DarkerColor => M::DarkerColor,
        psd::BlendMode::LighterColor => M::LighterColor,
        psd::BlendMode::SoftLight => M::SoftLight,
        psd::BlendMode::HardLight => M::HardLight,
        psd::BlendMode::VividLight => M::VividLight,
        psd::BlendMode::LinearLight => M::LinearLight,
        psd::BlendMode::PinLight => M::PinLight,
        psd::BlendMode::HardMix => M::HardMix,
        psd::BlendMode::Difference => M::Difference,
        psd::BlendMode::Exclusion => M::Exclusion,
        psd::BlendMode::Subtract => M::Subtract,
        psd::BlendMode::Divide => M::Divide,
        psd::BlendMode::Hue => M::Hue,
        psd::BlendMode::Saturation => M::Saturation,
        psd::BlendMode::Color => M::Color,
        psd::BlendMode::Luminosity => M::Luminosity,
        _ => return None,
    })
}
