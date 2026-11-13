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

use image::{GrayImage, Luma, RgbaImage};
use psd::{ChannelBytes, PsdChannelKind};
use std::collections::HashMap;

use crate::{DecodeError, DecodeOptions};

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
    let (width, height, channels) = check_header(bytes, options)?;
    crate::validate_psd_sections(bytes, width, height, channels)?;
    let file_notes = validate_psd_layer_records(bytes)?;

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

/// Mirror the header checks of `decode_psd_bytes` without touching it.
fn check_header(bytes: &[u8], options: DecodeOptions) -> Result<(u32, u32, u16), DecodeError> {
    let (width, height, depth, mode) = crate::psd_header(bytes).ok_or(DecodeError::UnsupportedFormat)?;
    let channels = bytes
        .get(12..14)
        .map(|s| u16::from_be_bytes([s[0], s[1]]))
        .ok_or(DecodeError::UnsupportedFormat)?;
    if width == 0 || height == 0 {
        return Err(DecodeError::Psd("画布尺寸为 0".into()));
    }
    if depth != 8 {
        return Err(DecodeError::Psd(format!(
            "仅支持 8 位/通道（此文件为 {depth} 位），无法加载分层"
        )));
    }
    if !matches!(mode, 1 | 3) {
        return Err(DecodeError::Psd(format!(
            "仅支持 RGB / 灰度模式（此文件为 {}），无法加载分层",
            crate::psd_mode_name(mode)
        )));
    }
    if let Some(limit) = options.max_pixels {
        if width as u64 * height as u64 > limit {
            return Err(DecodeError::TooLarge {
                width,
                height,
                pixels: width as u64 * height as u64,
                limit,
            });
        }
    }
    // C8: a downsampled document must never be written back with lost pixels.
    if let Some(edge) = options.max_dimension {
        if width.max(height) > edge {
            return Err(DecodeError::Psd(
                "此文件以降采样预览打开，不加载分层且禁止写回原文件".into(),
            ));
        }
    }
    Ok((width, height, channels))
}

/// Multiply a layer's own state by every ancestor group's, returning
/// `(visible, opacity)`.
///
/// Flattening the group tree loses the folder records, and a folder is where
/// Photoshop puts the "this whole variant is hidden" flag. Walking the chain
/// (`group_id -> parent group id`) keeps that information on the children.
/// The depth guard is there because a corrupt file could describe a cycle.
fn inherited_group_state(
    groups: &HashMap<u32, psd::PsdGroup>,
    parent: Option<u32>,
) -> (bool, f32) {
    let mut visible = true;
    let mut opacity = 1.0f32;
    let mut next = parent;
    for _ in 0..64 {
        let Some(group) = next.and_then(|id| groups.get(&id)) else {
            break;
        };
        visible &= group.visible();
        opacity *= group.opacity() as f32 / 255.0;
        next = group.parent_id();
    }
    (visible, opacity)
}

/// Group names designers use for annotation / spec layers.
///
/// These folders hold measurements, colour codes and font specs — data for the
/// implementer, not part of the screen. They are matched on the *group* name so
/// a real text layer called "字号28，描边2xp" (very common in Chinese UI files,
/// the name records the styling) is never mistaken for a note.
const ANNOTATION_GROUP_KEYWORDS: &[&str] = &["标注", "备注", "注释", "批注", "说明", "切图"];

fn is_annotation_name(name: &str) -> bool {
    let name = name.trim();
    ANNOTATION_GROUP_KEYWORDS
        .iter()
        .any(|kw| name.contains(kw))
}

/// Whether any ancestor group of a layer is an annotation folder.
fn ancestor_is_annotation_group(
    groups: &HashMap<u32, psd::PsdGroup>,
    parent: Option<u32>,
) -> bool {
    let mut next = parent;
    for _ in 0..64 {
        let Some(group) = next.and_then(|id| groups.get(&id)) else {
            break;
        };
        if is_annotation_name(group.name()) {
            return true;
        }
        next = group.parent_id();
    }
    false
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
    let groups = psd.groups();
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
        // 组不再摊平：整棵树由 `psd.groups()` 交出去（见函数末尾），这里
        // 只把「这层属于哪个组」记在层上，让面板能还原缩进与折叠。
        // A group's own visibility and opacity live on the folder record, not on
        // its children: Photoshop keeps the children's flags untouched and hides
        // them through the group. The canvas must still match the composite, so
        // every ancestor group's flag is folded into the child here; the group
        // row keeps its own state for the panel.
        let (ancestor_visible, ancestor_opacity) = inherited_group_state(groups, layer.parent_id());
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

        // ---- 形状图层的矢量填充（`vmsk` / `vscg`）------------------------
        // 形状层的通道数据是**空的**：像素由 PS 按矢量路径实时生成。只读通道
        // 时这类层是一片透明，连它自己的描边/内阴影都画不出来（实测目标稿
        // 5 张卡片的渐变描边整圈丢失）。只在位图确实为空时才栅格化 —— 有位图
        // 的形状层说明缓存里已经有渲染结果，再填一遍反而会盖掉它。
        if !off("IMAGEEDIT_PSD_NO_VECTOR") {
            if let Some(vm) = layer.vector_mask() {
                let empty = image.pixels().all(|p| p.0[3] == 0);
                let trace =
                    std::env::var("IMAGEEDIT_PSD_VECTOR_TRACE").ok().as_deref() == Some("1");
                if empty
                    && crate::psd_vector::fill_shape(
                        &mut image,
                        layer_offset,
                        (cw as u32, ch as u32),
                        vm,
                        layer.vector_fill(),
                    )
                {
                    vector_filled += 1;
                    if trace {
                        println!(
                            "{{\"phase\":\"decode\",\"event\":\"vector-fill\",\"name\":\"{}\",\
                             \"offset\":[{},{}],\"size\":[{},{}],\"groups\":{}}}",
                            layer.name(),
                            layer_offset.0,
                            layer_offset.1,
                            image.width(),
                            image.height(),
                            vm.subpaths.len()
                        );
                    }
                } else if trace && !empty {
                    println!(
                        "{{\"phase\":\"decode\",\"event\":\"vector-skip\",\"name\":\"{}\",\
                         \"why\":\"has-raster\"}}",
                        layer.name()
                    );
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
        let pre_alpha = if layer.layer_effects().is_some() {
            Some(ContentAlpha {
                alpha: image.pixels().map(|p| p.0[3]).collect::<Vec<u8>>(),
                offset: layer_offset,
                size: (image.width(), image.height()),
            })
        } else {
            None
        };
        let mut content_alpha = None;
        if let Some(effects) = layer.layer_effects().filter(|_| !no_styles) {
            // 诊断：`IMAGEEDIT_PSD_DUMP_FX=<图层名子串>` 打印解析到的样式参数。
            // 保真差异按块归因到某个图层后，用它确认「是解析错了还是画错了」。
            if let Ok(want) = std::env::var("IMAGEEDIT_PSD_DUMP_FX") {
                if !want.is_empty() && layer.name().contains(&want) {
                    for e in effects.iter().filter(|e| e.enabled) {
                        println!(
                            "{{\"fx\":\"{}\",\"kind\":\"{:?}\",\"blend\":\"{:?}\",\"op\":{:.3},\
                             \"ang\":{:.1},\"dist\":{:.2},\"size\":{:.2},\"choke\":{:.1},\
                             \"color\":[{},{},{}],\"pos\":\"{:?}\",\"fill\":\"{:?}\",\
                             \"grad\":{},\"knock\":{}}}",
                            layer.name(),
                            e.kind,
                            e.blend,
                            e.opacity,
                            e.angle,
                            e.distance,
                            e.size,
                            e.choke,
                            e.color[0],
                            e.color[1],
                            e.color[2],
                            e.stroke_position,
                            e.stroke_fill,
                            e.gradient.is_some(),
                            e.knocks_out
                        );
                    }
                }
            }
            crate::psd_effects::set_trace_label(layer.name());
            if let Some(baked) = crate::psd_effects::bake(&image, layer_offset, effects) {
                if let Some(pre) = pre_alpha.as_ref() {
                    content_alpha = Some(ContentAlpha::from_baked(
                        pre,
                        baked.offset,
                        baked.image.dimensions(),
                        &baked.effect_coverage,
                    ));
                }
                image = baked.image;
                layer_offset = baked.offset;
                notes.extend(baked.notes);
                styled_layers += 1;
            }
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
            opacity: (layer.opacity() as f32 / 255.0) * ancestor_opacity,
            blend,
            degraded: notes,
            mask,
            clipping,
            content_alpha,
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
        });
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
        bytes: total_bytes,
    })
}

fn dedup(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
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
        psd::BlendMode::Multiply => M::Multiply,
        psd::BlendMode::Screen => M::Screen,
        psd::BlendMode::Overlay => M::Overlay,
        psd::BlendMode::Darken => M::Darken,
        psd::BlendMode::Lighten => M::Lighten,
        psd::BlendMode::ColorDodge => M::ColorDodge,
        psd::BlendMode::ColorBurn => M::ColorBurn,
        psd::BlendMode::LinearDodge => M::LinearDodge,
        psd::BlendMode::LinearBurn => M::LinearBurn,
        psd::BlendMode::SoftLight => M::SoftLight,
        psd::BlendMode::HardLight => M::HardLight,
        psd::BlendMode::Difference => M::Difference,
        psd::BlendMode::Exclusion => M::Exclusion,
        psd::BlendMode::Hue => M::Hue,
        psd::BlendMode::Saturation => M::Saturation,
        psd::BlendMode::Color => M::Color,
        psd::BlendMode::Luminosity => M::Luminosity,
        _ => return None,
    })
}

/// Which already-pushed layers an adjustment layer applies to.
///
/// A clipped adjustment (Photoshop's "剪贴蒙版", the usual way a designer
/// recolours one button graphic) only touches the chain it clips to. Whether
/// that means "the whole clip chain plus its base" or "just the layer directly
/// below" is measurable, not obvious — Photoshop chains adjustments through each
/// other, which a flat baking pass cannot do exactly — so both are available
/// under `IMAGEEDIT_PSD_ADJUST_SCOPE` for the fidelity harness to decide.
///
/// The result is always clamped to the adjustment's own group: a group is a
/// separate composite in Photoshop, so layers outside it are never in scope.
fn adjustment_scope(
    layers: &[PsdLayerData],
    clips: &[bool],
    group: Option<u32>,
    clipped: bool,
    mode: ScopeMode,
) -> usize {
    let len = layers.len();
    let mut start = match mode {
        ScopeMode::All => 0,
        ScopeMode::Last => len.saturating_sub(1),
        ScopeMode::Chain => {
            if !clipped {
                0
            } else {
                let mut s = len;
                while s > 0 && clips[s - 1] {
                    s -= 1;
                }
                // Include the base layer the chain clips to.
                s.saturating_sub(1)
            }
        }
    };
    if !cross_group() {
        // Back up to the first layer of this group's contiguous run, so the
        // adjustment never leaks into a sibling group's pixels.
        let mut g = len;
        while g > 0 && layers[g - 1].group == group {
            g -= 1;
        }
        start = start.max(g);
    }
    start.min(len)
}

#[derive(Clone, Copy, PartialEq)]
enum ScopeMode {
    /// Everything below (the pre-2026-11 fallback).
    All,
    /// Only the layer directly below (Photoshop's literal clip target).
    Last,
    /// The trailing clip run plus the base it clips to.
    Chain,
}

fn adjust_scope_mode() -> ScopeMode {
    match std::env::var("IMAGEEDIT_PSD_ADJUST_SCOPE").ok().as_deref() {
        Some("all") => ScopeMode::All,
        Some("last") => ScopeMode::Last,
        _ => ScopeMode::Chain,
    }
}

fn scope_mode_name(mode: ScopeMode) -> &'static str {
    match mode {
        ScopeMode::All => "all",
        ScopeMode::Last => "last",
        ScopeMode::Chain => "chain",
    }
}

fn cross_group() -> bool {
    static CROSS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CROSS.get_or_init(|| std::env::var("IMAGEEDIT_PSD_ADJUST_CROSS_GROUP").ok().as_deref() == Some("1"))
}

/// Bake one adjustment layer's effect into the pixels of `targets`.
///
/// Adjustment layers have no bitmap of their own, so the only way to show what
/// Photoshop shows is to apply the operation to the layers below — exactly what
/// "拼合图层" would do. Alpha is never touched: an adjustment changes colour,
/// not coverage.
fn apply_adjustment(
    targets: &mut [PsdLayerData],
    bc: Option<psd::BrightnessContrast>,
    hs: Option<psd::HueSaturation>,
    mask: Option<&editcore::LayerMask>,
    clipped: bool,
) {
    // 剪贴调整层只吃基底的「内容」alpha；非剪贴的调整层（作用于整条链）没有
    // 这个限制。可用 `IMAGEEDIT_PSD_ADJUST_FULL_ALPHA=1` 关掉做消融。
    let content_alpha = clipped && !full_alpha_adjust();
    for layer in targets.iter_mut() {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        let pre = if content_alpha {
            layer.content_alpha.as_ref()
        } else {
            None
        };
        // Fast path: no mask and no content-alpha limit means every pixel takes
        // the full adjustment, which is the common case and must stay as cheap
        // as the original loop.
        if mask.is_none() && pre.is_none() {
            for px in layer.image.pixels_mut() {
                if px.0[3] == 0 {
                    continue;
                }
                let mut rgb = [px.0[0], px.0[1], px.0[2]];
                if let Some(hs) = hs {
                    rgb = hue_saturation(rgb, hs);
                }
                if let Some(bc) = bc {
                    rgb = brightness_contrast(rgb, bc);
                }
                px.0[0] = rgb[0];
                px.0[1] = rgb[1];
                px.0[2] = rgb[2];
            }
            continue;
        }
        let (ox, oy) = layer.offset;
        let (w, h) = (layer.image.width() as i32, layer.image.height() as i32);
        for y in 0..h {
            for x in 0..w {
                // How much of the adjustment reaches this pixel: all of it where
                // the adjustment's mask is white, none where it is black, times
                // the base layer's content alpha for a clipped adjustment.
                let mut k = match mask {
                    Some(mask) => mask.coverage(ox + x, oy + y),
                    None => 1.0,
                };
                if let Some(pre) = pre {
                    k *= pre.at(ox + x, oy + y);
                }
                let k = k.clamp(0.0, 1.0);
                if k <= 0.0 {
                    continue;
                }
                let px = layer.image.get_pixel_mut(x as u32, y as u32);
                if px.0[3] == 0 {
                    continue;
                }
                let src = px.0;
                let mut rgb = [src[0], src[1], src[2]];
                if let Some(hs) = hs {
                    rgb = hue_saturation(rgb, hs);
                }
                if let Some(bc) = bc {
                    rgb = brightness_contrast(rgb, bc);
                }
                if k < 1.0 {
                    rgb = [
                        (src[0] as f32 + (rgb[0] as f32 - src[0] as f32) * k).round() as u8,
                        (src[1] as f32 + (rgb[1] as f32 - src[1] as f32) * k).round() as u8,
                        (src[2] as f32 + (rgb[2] as f32 - src[2] as f32) * k).round() as u8,
                    ];
                }
                px.0[0] = rgb[0];
                px.0[1] = rgb[1];
                px.0[2] = rgb[2];
            }
        }
    }
}

fn full_alpha_adjust() -> bool {
    static FULL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FULL.get_or_init(|| std::env::var("IMAGEEDIT_PSD_ADJUST_FULL_ALPHA").ok().as_deref() == Some("1"))
}

/// Photoshop's brightness/contrast sliders (legacy range -100..=100).
fn brightness_contrast(rgb: [u8; 3], bc: psd::BrightnessContrast) -> [u8; 3] {
    if bc.brightness == 0 && bc.contrast == 0 {
        return rgb;
    }
    let bright = bc.brightness as f32 / 100.0;
    let contrast = 1.0 + bc.contrast as f32 / 100.0;
    let mut out = rgb;
    for c in 0..3 {
        let v = rgb[c] as f32 / 255.0;
        let v = (v - 0.5) * contrast + 0.5 + bright;
        out[c] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Photoshop's hue/saturation adjustment in HSL space.
fn hue_saturation(rgb: [u8; 3], hs: psd::HueSaturation) -> [u8; 3] {
    let (mut h, mut s, mut l) = rgb_to_hsl(rgb);
    if hs.colorize {
        // "着色": flatten the colour to a single hue first, then apply.
        s = 0.0;
        l = l * 0.5;
        h = 0.0;
        h = (h + hs.colorize_hue as f32 / 360.0).rem_euclid(1.0);
        s = apply_slider(s, hs.colorize_saturation as f32);
        l = apply_slider(l, hs.colorize_lightness as f32);
    } else {
        h = (h + hs.hue as f32 / 360.0).rem_euclid(1.0);
        s = apply_slider(s, hs.saturation as f32);
        l = apply_slider(l, hs.lightness as f32);
    }
    hsl_to_rgb(h, s, l)
}

/// Photoshop's -100..=100 sliders move a 0..1 value towards the corresponding
/// end of its range instead of scaling it linearly (so +100 really is "max").
fn apply_slider(v: f32, slider: f32) -> f32 {
    let t = (slider / 100.0).clamp(-1.0, 1.0);
    if t >= 0.0 {
        v + (1.0 - v) * t
    } else {
        v * (1.0 + t)
    }
}

fn rgb_to_hsl(rgb: [u8; 3]) -> (f32, f32, f32) {
    let r = rgb[0] as f32 / 255.0;
    let g = rgb[1] as f32 / 255.0;
    let b = rgb[2] as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    if s <= 0.0 {
        let v = (l * 255.0).round().clamp(0.0, 255.0) as u8;
        return [v, v, v];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |t: f32| -> f32 {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [
        (f(h + 1.0 / 3.0) * 255.0).round().clamp(0.0, 255.0) as u8,
        (f(h) * 255.0).round().clamp(0.0, 255.0) as u8,
        (f(h - 1.0 / 3.0) * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// Expand one planar channel to exactly `pixels` bytes. RLE data from the
/// vendored reader has the 2-byte-per-scanline length table already stripped.
fn channel_plane(
    chans: &psd::LayerChannels,
    kind: PsdChannelKind,
    pixels: usize,
) -> Option<Vec<u8>> {
    match chans.get(&kind)? {
        ChannelBytes::RawData(v) => {
            let mut out = v.clone();
            out.resize(pixels, 0);
            Some(out)
        }
        ChannelBytes::RleCompressed(v) => Some(unpack_packbits(v, pixels)),
    }
}

/// 解出一层的图层蒙版（`-2` 通道），按**蒙版自己的矩形**裁剪到画布范围。
///
/// 蒙版矩形通常就是整幅画布，而图层往往只是画布上的一个小贴片 —— 用图层
/// 矩形去解释蒙版通道会把蒙版错位。取不到通道 / 蒙版被 PS 关掉时返回 `None`
/// （等价于「无蒙版」，不会误把整层遮住）。
/// 诊断用：`IMAGEEDIT_PSD_MASK_AT=x,y` 指定要打印蒙版取值的画布坐标。
fn mask_probe() -> Option<(i32, i32)> {
    mask_probe_at("IMAGEEDIT_PSD_MASK_AT")
}

fn mask_probe_at(key: &str) -> Option<(i32, i32)> {
    let raw = std::env::var(key).ok()?;
    let (x, y) = raw.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

fn mask_at(m: &editcore::LayerMask, probe: Option<(i32, i32)>) -> i32 {
    match probe {
        Some((x, y)) => (m.coverage(x, y) * 255.0).round() as i32,
        None => -1,
    }
}

fn decode_mask(
    layer: &psd::PsdLayer,
    chans: &psd::LayerChannels,
    cw: i32,
    ch: i32,
) -> Option<editcore::LayerMask> {
    if !layer.has_active_mask() {
        return None;
    }
    let m = layer.mask_rect()?;
    let (mw, mh) = (m.width(), m.height());
    if mw == 0 || mh == 0 {
        return None;
    }
    let pixels = mw as u64 * mh as u64;
    if pixels > 300_000_000 {
        return None;
    }
    let mut plane = channel_plane(
        chans,
        PsdChannelKind::UserSuppliedLayerMask,
        pixels as usize,
    )?;
    // 通道数据比矩形短（截断文件）时按 0 补齐：蒙版缺数据 = 遮住，绝不能
    // 让下面的平面索引越界。
    plane.resize(pixels as usize, 0);
    let x0 = m.left.max(0);
    let y0 = m.top.max(0);
    let x1 = ((m.right as i64 + 1).min(cw as i64)) as i32;
    let y1 = ((m.bottom as i64 + 1).min(ch as i64)) as i32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut img = GrayImage::new((x1 - x0) as u32, (y1 - y0) as u32);
    for y in y0..y1 {
        let sy = (y - m.top) as usize;
        for x in x0..x1 {
            let sx = (x - m.left) as usize;
            let v = plane[sy * mw as usize + sx];
            img.put_pixel((x - x0) as u32, (y - y0) as u32, Luma([v]));
        }
    }
    Some(editcore::LayerMask::with_default(
        img,
        (x0, y0),
        m.default_color,
    ))
}

fn unpack_packbits(data: &[u8], expected: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0usize;
    while i < data.len() && out.len() < expected {
        let header = data[i] as i8 as i16;
        i += 1;
        if header == -128 {
            continue;
        }
        if header >= 0 {
            let count = (header + 1) as usize;
            let end = (i + count).min(data.len());
            out.extend_from_slice(&data[i..end]);
            i = end;
        } else {
            let count = (1 - header) as usize;
            if i >= data.len() {
                break;
            }
            let value = data[i];
            i += 1;
            out.resize((out.len() + count).min(expected), value);
        }
    }
    out.resize(expected, 0);
    out
}

/// Pre-validate the layer&mask section down to per-record / per-channel
/// lengths (M1d). `catch_unwind` stays as the last line of defence, but a
/// corrupt file should produce a normal `DecodeError` instead of a panic.
///
/// Returns non-fatal degradation notes; `Ok(vec![])` means "clean".
fn validate_psd_layer_records(bytes: &[u8]) -> Result<Vec<String>, DecodeError> {
    let err = || DecodeError::Psd("图层记录区段被截断，文件已损坏".into());
    let u32at = |o: usize| -> Option<u32> {
        let s = bytes.get(o..o + 4)?;
        Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    };
    let u16at = |o: usize| -> Option<u16> {
        let s = bytes.get(o..o + 2)?;
        Some(u16::from_be_bytes([s[0], s[1]]))
    };

    let mut off = 26usize;
    for _ in 0..2 {
        let len = u32at(off).ok_or_else(err)? as usize;
        off = off.checked_add(4 + len).ok_or_else(err)?;
    }
    let section_len = u32at(off).ok_or_else(err)? as usize;
    let section_start = off + 4;
    let section_end = section_start.checked_add(section_len).ok_or_else(err)?;
    if section_end > bytes.len() {
        return Err(err());
    }
    // A flat PSD has an empty Layer & Mask section (length 0); there is
    // nothing to pre-validate and the parser will report zero layers.
    if section_len == 0 {
        return Ok(Vec::new());
    }
    let info_len = u32at(section_start).ok_or_else(err)? as usize;
    if info_len == 0 {
        return Ok(Vec::new());
    }
    let info_start = section_start + 4;
    // Some encoders write a Layer Info length that does not cover the channel
    // data (and odd-length Pascal names have different padding). The vendored
    // reader works off the SECTION slice, so use the section boundary as the
    // hard limit and keep the declared info_len only for the "no layers" case.
    let info_end = section_end;
    if info_start + 2 > info_end {
        return Err(err());
    }
    let count = i16::from_be_bytes([bytes[info_start], bytes[info_start + 1]]).unsigned_abs() as usize;
    if count > 4096 {
        return Err(DecodeError::Psd("图层数量异常".into()));
    }
    let mut p = info_start + 2;
    // (layer height, user-mask height, real-mask height, (channel id, length))
    let mut records: Vec<(usize, Option<usize>, Option<usize>, Vec<(i16, u32)>)> =
        Vec::with_capacity(count);
    for _ in 0..count {
        let top = u32at(p).ok_or_else(err)? as i32;
        let bottom = u32at(p + 8).ok_or_else(err)? as i32;
        p += 16;
        let nch = u16at(p).ok_or_else(err)? as usize;
        p += 2;
        if nch > 56 || p + nch * 6 > info_end {
            return Err(err());
        }
        let mut lengths = Vec::with_capacity(nch);
        for _ in 0..nch {
            let cid = u16at(p).ok_or_else(err)? as i16;
            let l = u32at(p + 2).ok_or_else(err)?;
            if l < 2 {
                return Err(DecodeError::Psd("图层通道长度异常".into()));
            }
            lengths.push((cid, l));
            p += 6;
        }
        p += 12; // '8BIM' + blend key + opacity/clipping/flags/filler
        let extra_len = u32at(p).ok_or_else(err)? as usize;
        p += 4;
        let extra_end = p.checked_add(extra_len).ok_or_else(err)?;
        if extra_end > info_end {
            return Err(err());
        }
        // Extra data: mask length, blending ranges length, Pascal name,
        // then a chain of 8BIM/8B64 additional-layer-info blocks.
        //
        // Layer mask data carries its OWN rectangle, and its channel (`-2`) is
        // compressed against that rectangle — not against the layer rectangle.
        // Reading the mask rect here is what makes mask channels validate
        // correctly. Layout: top/left/bottom/right (4x i32), default colour,
        // flags, 2 pad bytes; a real user mask (channel `-3`) follows at +18.
        let mask_len = u32at(p).ok_or_else(err)? as usize;
        let mut mask_rows = None;
        let mut real_mask_rows = None;
        if mask_len >= 20 {
            let m_top = u32at(p + 4).ok_or_else(err)? as i32;
            let m_bottom = u32at(p + 12).ok_or_else(err)? as i32;
            mask_rows = Some((m_bottom as i64 - m_top as i64 + 1).clamp(0, 65_535) as usize);
        }
        if mask_len >= 38 {
            let r_top = u32at(p + 22).ok_or_else(err)? as i32;
            let r_bottom = u32at(p + 30).ok_or_else(err)? as i32;
            real_mask_rows = Some((r_bottom as i64 - r_top as i64 + 1).clamp(0, 65_535) as usize);
        }
        p = p.checked_add(4 + mask_len).ok_or_else(err)?;
        let ranges_len = u32at(p).ok_or_else(err)? as usize;
        p = p.checked_add(4 + ranges_len).ok_or_else(err)?;
        let name_len = *bytes.get(p).ok_or_else(err)? as usize;
        let pad = (4 - ((name_len + 1) % 4)) % 4;
        p = p.checked_add(1 + name_len + pad).ok_or_else(err)?;
        while p + 12 <= extra_end {
            let sig = bytes.get(p..p + 4).unwrap_or(&[]);
            if sig != b"8BIM" && sig != b"8B64" {
                break;
            }
            let l = u32at(p + 8).ok_or_else(err)? as usize;
            p = p.checked_add(12 + l).ok_or_else(err)?;
            if p > extra_end {
                return Err(err());
            }
        }
        p = extra_end;
        if p > info_end {
            return Err(err());
        }
        let scanlines = (bottom as i64 - top as i64 + 1).max(0).min(65_535) as usize;
        records.push((scanlines, mask_rows, real_mask_rows, lengths));
    }
    let mut short_channels = 0usize;
    for (scanlines, mask_rows, real_mask_rows, lengths) in records {
        for (cid, l) in lengths {
            let l = l as usize;
            let end = p.checked_add(l).ok_or_else(err)?;
            if end > info_end {
                return Err(err());
            }
            let compression = u16at(p).ok_or_else(err)?;
            if compression == 1 {
                // Vendor skips 2 rows-of-table bytes. Mask channels (`-2` /
                // `-3`) hold their OWN rectangle in the mask block, so their
                // scanline count is not the layer height; using the layer
                // height there flagged perfectly good Photoshop files.
                let rows = match cid {
                    -2 => mask_rows.unwrap_or(scanlines),
                    -3 => real_mask_rows.or(mask_rows).unwrap_or(scanlines),
                    _ => scanlines,
                };
                // A short row table is NOT fatal: the vendored reader drops
                // that single channel (the layer shows as transparent) and both
                // it and our PackBits unpacker are bounds-safe. Real files do
                // hit this (empty layers, unusual encoders), and refusing the
                // whole document loses every other layer as well — degrade and
                // report instead.
                // The row counts themselves are also not validated against the
                // remaining bytes for the same reason.
                if l.saturating_sub(2) < rows * 2 && cid >= -1 {
                    short_channels += 1;
                }
            } else if compression > 1 {
                return Err(DecodeError::Psd(format!(
                    "压缩方式 {compression} 不受支持，无法加载分层"
                )));
            }
            p = end;
        }
    }
    let mut notes = Vec::new();
    if short_channels > 0 {
        notes.push(format!(
            "{short_channels} 个图层通道数据不完整，已按空层处理"
        ));
    }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    struct TestLayer {
        name: &'static str,
        top: i32,
        left: i32,
        w: u32,
        h: u32,
        rgba: Vec<u8>,
        opacity: u8,
        visible: bool,
    }

    fn push_u16(v: &mut Vec<u8>, n: u16) {
        v.extend_from_slice(&n.to_be_bytes());
    }
    fn push_i16(v: &mut Vec<u8>, n: i16) {
        v.extend_from_slice(&n.to_be_bytes());
    }
    fn push_u32(v: &mut Vec<u8>, n: u32) {
        v.extend_from_slice(&n.to_be_bytes());
    }
    fn push_i32(v: &mut Vec<u8>, n: i32) {
        v.extend_from_slice(&n.to_be_bytes());
    }
    fn pad4(v: &mut Vec<u8>) {
        while v.len() % 4 != 0 {
            v.push(0);
        }
    }

    /// `layers` is given in file order, i.e. bottom -> top: that is how a PSD
    /// stores its records, and what `decode_psd_layers` must hand back.
    fn make_psd(w: u32, h: u32, layers: &[TestLayer]) -> Vec<u8> {
        let mut records = Vec::new();
        let mut channels = Vec::new();
        for layer in layers.iter() {
            let n = (layer.w * layer.h) as usize;
            let planes: Vec<Vec<u8>> = (0..4)
                .map(|c| (0..n).map(|i| layer.rgba[i * 4 + c]).collect::<Vec<u8>>())
                .collect();
            let mut data = Vec::new();
            for plane in &planes {
                push_u16(&mut data, 0);
                data.extend_from_slice(plane);
            }
            let mut rec = Vec::new();
            push_i32(&mut rec, layer.top);
            push_i32(&mut rec, layer.left);
            push_i32(&mut rec, layer.top + layer.h as i32);
            push_i32(&mut rec, layer.left + layer.w as i32);
            push_u16(&mut rec, 4);
            for (id, plane) in [0i16, 1, 2, -1].iter().zip(planes.iter()) {
                push_i16(&mut rec, *id);
                push_u32(&mut rec, (2 + plane.len()) as u32);
            }
            rec.extend_from_slice(b"8BIMnorm");
            rec.push(layer.opacity);
            rec.push(0);
            // Adobe spec: bit 3 = Photoshop 5+, bit 1 SET = HIDDEN.
            rec.push(if layer.visible { 8 } else { 8 | 2 });
            rec.push(0);
            let mut extra = Vec::new();
            push_u32(&mut extra, 0);
            push_u32(&mut extra, 0);
            let name = layer.name.as_bytes();
            extra.push(name.len() as u8);
            extra.extend_from_slice(name);
            pad4(&mut extra);
            push_u32(&mut rec, extra.len() as u32);
            rec.extend_from_slice(&extra);
            records.extend_from_slice(&rec);
            channels.extend_from_slice(&data);
        }
        let mut info = Vec::new();
        push_u32(&mut info, (2 + records.len() + channels.len()) as u32);
        push_i16(&mut info, layers.len() as i16);
        info.extend_from_slice(&records);
        info.extend_from_slice(&channels);

        let mut out = Vec::new();
        out.extend_from_slice(b"8BPS");
        push_u16(&mut out, 1);
        out.extend_from_slice(&[0; 6]);
        push_u16(&mut out, 4);
        push_u32(&mut out, h);
        push_u32(&mut out, w);
        push_u16(&mut out, 8);
        push_u16(&mut out, 3);
        push_u32(&mut out, 0);
        push_u32(&mut out, 0);
        push_u32(&mut out, (info.len() + 4) as u32);
        out.extend_from_slice(&info);
        push_u32(&mut out, 0);
        push_u16(&mut out, 0);
        for c in 0..4 {
            out.extend(std::iter::repeat((c * 40 + 17) as u8).take((w * h) as usize));
        }
        out
    }

    fn solid(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            v.extend_from_slice(&color);
        }
        v
    }

    #[test]
    fn parses_layers_with_attributes_and_pixels() {
        let bytes = make_psd(
            8,
            6,
            &[
                TestLayer {
                    name: "bottom",
                    top: 0,
                    left: 0,
                    w: 4,
                    h: 3,
                    rgba: solid(4, 3, [10, 20, 30, 255]),
                    opacity: 255,
                    visible: true,
                },
                TestLayer {
                    name: "top",
                    top: 1,
                    left: 2,
                    w: 3,
                    h: 2,
                    rgba: solid(3, 2, [200, 100, 50, 128]),
                    opacity: 128,
                    visible: false,
                },
            ],
        );
        let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("parse");
        assert_eq!(set.canvas, (8, 6));
        assert_eq!(set.layers.len(), 2);
        assert_eq!(set.layers[0].name, "bottom");
        assert_eq!(set.layers[0].offset, (0, 0));
        assert!((set.layers[0].opacity - 1.0).abs() < 1e-6);
        assert!(set.layers[0].visible);
        assert_eq!(set.layers[0].image.get_pixel(0, 0), &Rgba([10, 20, 30, 255]));
        assert_eq!(set.layers[1].name, "top");
        assert_eq!(set.layers[1].offset, (2, 1));
        assert!(!set.layers[1].visible);
        assert!((set.layers[1].opacity - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(set.layers[1].image.get_pixel(0, 0), &Rgba([200, 100, 50, 128]));
        assert_eq!(set.bytes, (4 * 3 + 3 * 2) * 4);
    }

    #[test]
    fn clips_negative_offsets_and_out_of_bounds_layers() {
        let bytes = make_psd(
            6,
            4,
            &[
                TestLayer {
                    name: "clipped",
                    top: -2,
                    left: -1,
                    w: 5,
                    h: 5,
                    rgba: solid(5, 5, [7, 8, 9, 255]),
                    opacity: 255,
                    visible: true,
                },
                TestLayer {
                    name: "outside",
                    top: 20,
                    left: 20,
                    w: 2,
                    h: 2,
                    rgba: solid(2, 2, [1, 1, 1, 255]),
                    opacity: 255,
                    visible: true,
                },
            ],
        );
        let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("parse");
        // The fully-outside layer is dropped; the negative one is cropped.
        let clipped = set
            .layers
            .iter()
            .find(|l| l.name == "clipped")
            .expect("clipped layer");
        assert_eq!(clipped.offset, (0, 0));
        assert_eq!(clipped.image.dimensions(), (4, 3));
        assert!(!set.degraded.is_empty());
    }

    #[test]
    fn memory_budget_is_per_layer_rectangle() {
        let small: Vec<TestLayer> = (0..10)
            .map(|i| TestLayer {
                name: "s",
                top: i * 9,
                left: i * 9,
                w: 10,
                h: 10,
                rgba: solid(10, 10, [i as u8, 0, 0, 255]),
                opacity: 255,
                visible: true,
            })
            .collect();
        let bytes = make_psd(100, 100, &small);
        let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("parse");
        assert_eq!(set.layer_count(), 10);
        // 10 * 10x10 layers = 4KB, NOT 10 * full canvas (400KB).
        assert_eq!(set.bytes, 10 * 10 * 10 * 4);
        assert!(set.bytes < 100 * 100 * 4);
    }

    #[test]
    fn progress_reports_every_layer() {
        let bytes = make_psd(
            4,
            4,
            &[TestLayer {
                name: "a",
                top: 0,
                left: 0,
                w: 4,
                h: 4,
                rgba: solid(4, 4, [1, 2, 3, 255]),
                opacity: 255,
                visible: true,
            }],
        );
        let mut seen = Vec::new();
        decode_psd_layers_with_progress(&bytes, DecodeOptions::default(), |d, t| seen.push((d, t)))
            .expect("parse");
        assert_eq!(seen, vec![(0, 1), (1, 1)]);
    }

    #[test]
    fn corrupt_section_length_is_an_error_not_a_panic() {
        let bytes = make_psd(
            4,
            4,
            &[TestLayer {
                name: "a",
                top: 0,
                left: 0,
                w: 4,
                h: 4,
                rgba: solid(4, 4, [1, 2, 3, 255]),
                opacity: 255,
                visible: true,
            }],
        );
        let mut bad = bytes.clone();
        bad[34..38].copy_from_slice(&0x7fff_ffffu32.to_be_bytes());
        assert!(decode_psd_layers(&bad, DecodeOptions::default()).is_err());
        // The composite-only path is unchanged (C1).
        let decoded = crate::decode_psd_bytes(&bytes, DecodeOptions::default()).expect("decode");
        assert_eq!(decoded.image.dimensions(), (4, 4));
        assert!(decoded.note.unwrap().contains("图层已合并"));
    }

    #[test]
    fn vendored_fixtures_parse_without_panicking() {
        const FIXTURES: &[(&str, &[u8])] = &[
            ("green-1x1", include_bytes!("../../../vendor/psd/tests/fixtures/green-1x1.psd")),
            (
                "chinese-name",
                include_bytes!("../../../vendor/psd/tests/fixtures/green-chinese-layer-name-1x1.psd"),
            ),
            (
                "negative-offset",
                include_bytes!("../../../vendor/psd/tests/fixtures/negative-top-left-layer.psd"),
            ),
            (
                "out-of-bounds",
                include_bytes!("../../../vendor/psd/tests/fixtures/out-of-bounds-layer.psd"),
            ),
            (
                "rle-3-layer",
                include_bytes!("../../../vendor/psd/tests/fixtures/rle-3-layer-8x8.psd"),
            ),
            (
                "one-channel",
                include_bytes!("../../../vendor/psd/tests/fixtures/one-channel-1x1.psd"),
            ),
            (
                "non-utf8-name",
                include_bytes!("../../../vendor/psd/tests/fixtures/non-utf8-pascal-string.psd"),
            ),
            (
                "two-layers",
                include_bytes!("../../../vendor/psd/tests/fixtures/two-layers-red-green-1x1.psd"),
            ),
            (
                "transparent-above",
                include_bytes!("../../../vendor/psd/tests/fixtures/transparent-above-opaque.psd"),
            ),
            (
                "odd-pascal",
                include_bytes!("../../../vendor/psd/tests/fixtures/odd-length-pascal-string.psd"),
            ),
        ];
        // Collect every problem in one run instead of stopping at the first
        // fixture, so a single test run shows all compatibility gaps.
        let mut problems = Vec::new();
        for (name, bytes) in FIXTURES {
            let result = std::panic::catch_unwind(|| {
                decode_psd_layers(bytes, DecodeOptions::default())
            });
            match result {
                Err(_) => problems.push(format!("{name}: PANIC")),
                Ok(Err(e)) => {
                    // 16-bit files are deliberately refused as layered
                    // documents (§7 degradation contract).
                    if *name != "one-channel" {
                        problems.push(format!("{name}: {e}"));
                    }
                }
                Ok(Ok(set)) => {
                    // Flat fixtures (no Layer & Mask section) legitimately
                    // report zero layers; the caller keeps the composite.
                    let flat_ok = matches!(*name, "non-utf8-name" | "odd-pascal");
                    if set.layer_count() < 1 && !flat_ok {
                        problems.push(format!("{name}: no layers"));
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn unicode_layer_name_survives() {
        let bytes = include_bytes!("../../../vendor/psd/tests/fixtures/green-chinese-layer-name-1x1.psd");
        let set = decode_psd_layers(bytes, DecodeOptions::default()).expect("parse");
        assert!(set.layers.iter().any(|l| l.name.chars().any(|c| c as u32 > 127)));
    }

    #[test]
    fn into_seeds_assembles_an_editable_document() {
        let bytes = make_psd(
            6,
            5,
            &[
                TestLayer {
                    name: "bottom",
                    top: 0,
                    left: 0,
                    w: 6,
                    h: 5,
                    rgba: solid(6, 5, [10, 20, 30, 255]),
                    opacity: 255,
                    visible: true,
                },
                TestLayer {
                    name: "top",
                    top: 1,
                    left: 2,
                    w: 2,
                    h: 2,
                    rgba: solid(2, 2, [90, 80, 70, 255]),
                    opacity: 128,
                    visible: false,
                },
            ],
        );
        let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("parse");
        let canvas = set.canvas;
        let doc = editcore::Document::from_psd_layers(canvas, set.into_seeds());
        assert_eq!(doc.layers.len(), 3);
        assert!(doc.layers[0].background);
        assert_eq!(doc.layers[1].name, "bottom");
        assert_eq!(doc.layers[2].name, "top");
        assert_eq!(doc.layers[2].offset(), (2, 1));
        assert!(!doc.layers[2].visible);
        assert!((doc.layers[2].opacity - 128.0 / 255.0).abs() < 1e-6);
    }

    /// One RLE channel: `rows` scanlines of `width` pixels. `truncate` keeps
    /// only that many bytes of the body, which is how a broken row-length
    /// table looks on disk.
    fn packbits_channel(rows: u32, width: u32, value: u8, truncate: Option<usize>) -> Vec<u8> {
        let mut body = Vec::new();
        for _ in 0..rows {
            push_u16(&mut body, 2);
        }
        for _ in 0..rows {
            body.push((1i16 - width as i16) as i8 as u8);
            body.push(value);
        }
        if let Some(n) = truncate {
            body.truncate(n);
        }
        let mut out = Vec::new();
        push_u16(&mut out, 1); // RLE
        out.extend_from_slice(&body);
        out
    }

    /// 每行取不同值的通道：用来验证「行表按谁的高度剥」和「平面 stride」两件事。
    /// 值 = 行号 * 10，任何错位都会在断言里立刻显形（均匀值则查不出来）。
    fn packbits_channel_rows(rows: u32, width: u32) -> Vec<u8> {
        let mut body = Vec::new();
        for _ in 0..rows {
            push_u16(&mut body, 2);
        }
        for r in 0..rows {
            body.push((1i16 - width as i16) as i8 as u8);
            body.push((r * 10) as u8);
        }
        let mut out = Vec::new();
        push_u16(&mut out, 1); // RLE
        out.extend_from_slice(&body);
        out
    }

    /// A one-layer PSD with an optional user-mask channel. Photoshop stores the
    /// mask rectangle separately from the layer rectangle, so the mask channel
    /// legitimately has fewer scanlines than the layer — that combination used
    /// to reject the whole file ("RLE 行长度表被截断").
    fn make_psd_with_mask(side: u32, mask_side: Option<u32>, truncate_red: bool) -> Vec<u8> {
        let red = if truncate_red { Some(2) } else { None };
        let mut chans: Vec<(i16, Vec<u8>)> = vec![
            (-1, packbits_channel(side, side, 255, None)),
            (0, packbits_channel(side, side, 200, red)),
            (1, packbits_channel(side, side, 100, None)),
            (2, packbits_channel(side, side, 50, None)),
        ];
        let mask_len = match mask_side {
            Some(m) => {
                chans.push((-2, packbits_channel_rows(m, m)));
                20usize
            }
            None => 0,
        };

        let mut rec = Vec::new();
        push_i32(&mut rec, 0);
        push_i32(&mut rec, 0);
        push_i32(&mut rec, side as i32);
        push_i32(&mut rec, side as i32);
        push_u16(&mut rec, chans.len() as u16);
        for (id, data) in &chans {
            push_i16(&mut rec, *id);
            push_u32(&mut rec, data.len() as u32);
        }
        rec.extend_from_slice(b"8BIMnorm");
        rec.push(255);
        rec.push(0);
        rec.push(8); // bit 3 = Photoshop 5+, bit 1 clear = visible
        rec.push(0);
        let mut extra = Vec::new();
        push_u32(&mut extra, mask_len as u32);
        if let Some(m) = mask_side {
            push_i32(&mut extra, 0);
            push_i32(&mut extra, 0);
            // 与图层矩形同一约定：右下角存的是**边界**（宽 = right - left）。
            // 写 `m - 1` 会让解码端按「最后一个像素」理解，整块蒙版错一行一列
            // ——真实文件（`0302-界面UI-1.psd`）里 right 就是 left + 宽度。
            push_i32(&mut extra, m as i32);
            push_i32(&mut extra, m as i32);
            extra.push(0); // default colour
            extra.push(0); // flags
            extra.extend_from_slice(&[0, 0]);
        }
        push_u32(&mut extra, 0);
        extra.push(5);
        extra.extend_from_slice(b"masks");
        pad4(&mut extra);
        push_u32(&mut rec, extra.len() as u32);
        rec.extend_from_slice(&extra);

        let mut channels = Vec::new();
        for (_, data) in &chans {
            channels.extend_from_slice(data);
        }
        let mut info = Vec::new();
        push_u32(&mut info, (2 + rec.len() + channels.len()) as u32);
        push_i16(&mut info, 1);
        info.extend_from_slice(&rec);
        info.extend_from_slice(&channels);

        let mut out = Vec::new();
        out.extend_from_slice(b"8BPS");
        push_u16(&mut out, 1);
        out.extend_from_slice(&[0; 6]);
        push_u16(&mut out, 4);
        push_u32(&mut out, side);
        push_u32(&mut out, side);
        push_u16(&mut out, 8);
        push_u16(&mut out, 3);
        push_u32(&mut out, 0);
        push_u32(&mut out, 0);
        push_u32(&mut out, (info.len() + 4) as u32);
        out.extend_from_slice(&info);
        push_u32(&mut out, 0);
        push_u16(&mut out, 0);
        for c in 0..4u8 {
            out.extend(std::iter::repeat(c * 40 + 17).take((side * side) as usize));
        }
        out
    }

    #[test]
    fn mask_channel_with_its_own_rect_is_accepted() {
        let bytes = make_psd_with_mask(8, Some(2), false);
        let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("mask channel parses");
        assert_eq!(set.layer_count(), 1);
        // A 2x2 mask on an 8x8 layer must not be reported as corruption.
        assert!(
            !set.degraded.iter().any(|n| n.contains("不完整")),
            "unexpected degrade notes: {:?}",
            set.degraded
        );
        // The mask itself must come out at its own size (2x2), not at the
        // layer's 8x8: the channel is compressed against the mask rectangle.
        let mask = set.layers[0].mask.as_ref().expect("用户蒙版应被保留");
        assert_eq!(mask.image.dimensions(), (2, 2));
        assert_eq!(mask.offset, (0, 0));
        // 行取值各不相同，所以「行表剥错 2 字节」或「stride 多一列」都会让
        // 这里对不上（实测真实文件的蒙版就错在这两处）。
        for y in 0..2u32 {
            assert_eq!(
                mask.image.get_pixel(0, y).0[0],
                (y * 10) as u8,
                "mask row {y} decoded at the wrong offset"
            );
        }
        assert_eq!(set.masked, 1);
    }

    /// A layer without `-2` must not sprout an empty mask (which would hide it).
    #[test]
    fn layer_without_mask_channel_has_no_mask() {
        let bytes = make_psd_with_mask(8, None, false);
        let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("parses");
        assert!(set.layers[0].mask.is_none());
        assert_eq!(set.masked, 0);
    }

    #[test]
    fn truncated_channel_degrades_instead_of_failing_the_file() {
        let bytes = make_psd_with_mask(8, None, true);
        let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("其他图层仍可加载");
        assert_eq!(set.layer_count(), 1);
        assert!(
            set.degraded.iter().any(|n| n.contains("通道数据不完整")),
            "expected a degrade note, got {:?}",
            set.degraded
        );
    }

    /// Diagnostic benchmark for real user files (ignored by default):
    ///
    /// ```text
    /// set IMAGEEDIT_PROFILE_PSD=<file.psd>
    /// cargo test -p imageedit-decode profile_real_psd -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn profile_real_psd() {
        let Ok(path) = std::env::var("IMAGEEDIT_PROFILE_PSD") else {
            return;
        };
        let t = std::time::Instant::now();
        let bytes = std::fs::read(&path).expect("read");
        println!("read      {:>8.1} ms  ({} MB)", ms(t), bytes.len() / 1_048_576);
        let t = std::time::Instant::now();
        let psd = psd::Psd::from_bytes(&bytes).expect("parse");
        println!("vendor    {:>8.1} ms  ({} layers)", ms(t), psd.layers().len());
        let t = std::time::Instant::now();
        let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("layers");
        println!(
            "full      {:>8.1} ms  ({} layers, {} MB)",
            ms(t),
            set.layer_count(),
            set.bytes / 1_048_576
        );
        let visible_masked = set.layers.iter().filter(|l| l.mask.is_some() && l.visible).count();
        let visible_clipped = set.layers.iter().filter(|l| l.clipping && l.visible).count();
        println!(
            "masks     {:>8} (visible {}), clipping {:>4} (visible {})",
            set.masked, visible_masked, set.clipped, visible_clipped
        );
        let t = std::time::Instant::now();
        let _ = decode_psd_layers(&bytes, DecodeOptions::default()).expect("layers");
        println!("full #2   {:>8.1} ms  (warm)", ms(t));
    }

    /// Per-layer transparency census for real user files (ignored by default).
    ///
    /// A canvas that comes out blank while the panel lists hundreds of layers
    /// almost always means the layers decoded to fully transparent bitmaps
    /// (vector / text / smart-object layers carry no channel data of their
    /// own). This prints the numbers instead of guessing.
    ///
    /// ```text
    /// set IMAGEEDIT_INSPECT_PSD=<file.psd>
    /// cargo test -p imageedit-decode inspect_real_psd -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn inspect_real_psd() {
        let Ok(path) = std::env::var("IMAGEEDIT_INSPECT_PSD") else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("layers");
        println!(
            "canvas {:?}  layers {}  composite {}",
            set.canvas,
            set.layer_count(),
            set.has_composite
        );
        if let Ok(psd) = psd::Psd::from_bytes(&bytes) {
            let groups = psd.groups();
            let mut in_group = 0usize;
            let mut hidden_inherited = 0usize;
            for l in psd.layers() {
                if let Some(gid) = l.parent_id() {
                    in_group += 1;
                    let (vis, op) = inherited_group_state(groups, Some(gid));
                    if !vis || op < 0.999 {
                        hidden_inherited += 1;
                    }
                }
            }
            println!(
                "groups {}  layers_in_group {}  inherited_hidden_or_faded {}",
                groups.len(),
                in_group,
                hidden_inherited
            );
        }
        let mut empty = 0usize;
        let mut visible_empty = 0usize;
        let mut rows: Vec<(u64, String, (u32, u32), (i32, i32), bool, f32)> = Vec::new();
        for l in &set.layers {
            let nz = l.image.pixels().filter(|p| p[3] > 8).count() as u64;
            if nz == 0 {
                empty += 1;
                if l.visible {
                    visible_empty += 1;
                }
            }
            rows.push((nz, l.name.clone(), l.image.dimensions(), l.offset, l.visible, l.opacity));
        }
        rows.sort_by(|a, b| b.0.cmp(&a.0));
        println!("全透明层 {empty} / {}（其中可见 {visible_empty}）", set.layers.len());
        for r in rows.iter().take(12) {
            println!(
                "  {:>9} px  {}  {:?} off={:?} vis={} op={:.2}",
                r.0, r.1, r.2, r.3, r.4, r.5
            );
        }
        println!("degraded: {:?}", set.degraded);
        println!(
            "前5: {:?}",
            set.layers.iter().take(5).map(|l| l.name.clone()).collect::<Vec<_>>()
        );
        println!(
            "后5: {:?}",
            set.layers.iter().rev().take(5).map(|l| l.name.clone()).collect::<Vec<_>>()
        );

        // Colour census for the biggest layers: all-white RGBA would composite
        // to a blank canvas even though the alpha mask is perfectly fine.
        for r in rows.iter().take(4) {
            if let Some(l) = set.layers.iter().find(|l| l.name == r.1 && l.image.dimensions() == r.2) {
                let mut min = [255u8; 4];
                let mut max = [0u8; 4];
                let mut sum = [0u64; 4];
                let n = l.image.pixels().count() as u64;
                for p in l.image.pixels() {
                    for c in 0..4 {
                        min[c] = min[c].min(p[c]);
                        max[c] = max[c].max(p[c]);
                        sum[c] += p[c] as u64;
                    }
                }
                let mean: Vec<u64> = (0..4).map(|c| sum[c] / n.max(1)).collect();
                let center = l.image.get_pixel(l.image.width() / 2, l.image.height() / 2);
                println!(
                    "  [{}] {:?} min={:?} max={:?} mean={:?} center={:?}",
                    l.name, l.image.dimensions(), min, max, mean, center
                );
            }
        }

        // TEMP diagnostic: dump one layer by name so its pixels can be compared
        // with the reference renderer.
        if let Ok(want) = std::env::var("IMAGEEDIT_DUMP_LAYER") {
            for l in &set.layers {
                if l.name == want {
                    let name = format!("C:/_APP_CODE/our_{}.png", want.replace(' ', "_"));
                    l.image.save(&name).expect("dump layer");
                    println!("dump -> {name} {:?} offset {:?}", l.image.dimensions(), l.offset);
                }
            }
        }

        // Composite through the **real** pipeline (core flatten, blend modes
        // included) so this file is directly comparable with the composite
        // Photoshop baked into the PSD. A hand-rolled source-over loop here used
        // to hide every blend-mode problem.
        let doc = editcore::Document::from_psd_layers(set.canvas, set.clone().into_seeds());
        let out = doc.flatten();
        let path = std::path::Path::new(&path)
            .file_stem()
            .map(|s| std::path::PathBuf::from(format!("C:/_APP_CODE/composite-{}.png", s.to_string_lossy())))
            .unwrap_or_else(|| std::path::PathBuf::from("C:/_APP_CODE/composite-check.png"));
        out.save(&path).expect("save composite");
        println!("composite -> {}", path.display());
    }

    fn ms(t: std::time::Instant) -> f64 {
        t.elapsed().as_secs_f64() * 1000.0
    }
}
