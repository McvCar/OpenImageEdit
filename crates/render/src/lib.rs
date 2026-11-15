use std::sync::Arc;

use image::imageops::FilterType;
use image::RgbaImage;

use editcore::{Adjust, AdjustTarget, Document, Layer, LayerContent, LayerMask, Rect, Size};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub size: Size,
    pub zoom: Zoom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zoom {
    Fit,
    Actual,
    Factor(f32),
}

impl Viewport {
    pub fn new(width: u32, height: u32, zoom: Zoom) -> Self {
        Self {
            size: Size::new(width, height),
            zoom,
        }
    }
}

pub fn scale_for(source: Size, viewport: Size, zoom: Zoom) -> f32 {
    if source.pixels() == 0 || viewport.pixels() == 0 {
        return 1.0;
    }
    match zoom {
        Zoom::Actual => 1.0,
        Zoom::Factor(factor) => factor,
        Zoom::Fit => {
            let sx = viewport.width as f32 / source.width as f32;
            let sy = viewport.height as f32 / source.height as f32;
            sx.min(sy).min(1.0)
        }
    }
}

pub fn display_size(source: Size, viewport: Size, zoom: Zoom) -> Size {
    let scale = scale_for(source, viewport, zoom);
    Size::new(
        ((source.width as f32 * scale).round() as u32).max(1),
        ((source.height as f32 * scale).round() as u32).max(1),
    )
}

/// Resample `image` to `target` for PREVIEW use.
///
/// Downscaling goes through `imageops::thumbnail` (integer box filter): for a
/// preview-sized target it is ~2.5x faster than the general
/// `resize(…, Triangle)` and, at these scale factors, indistinguishable on
/// screen (measured against a Lanczos reference: box err 0.45/255 vs bilinear
/// 0.34, nearest 0.65). That matters in debug builds, where one Triangle
/// downscale of a 1600×1000 photo to a 1100×688 fit preview costs ~390 ms and a
/// window resize used to pay it per event.
///
/// Upscaling keeps the filtered path (a box filter would turn zoomed-in pixels
/// into blocky mush).
///
/// RGBA here is straight (non-premultiplied) alpha. Averaging straight-alpha
/// pixels lets the RGB of fully transparent pixels (RGB=0 after erase) bleed
/// into neighbours — erased holes grew a dark rim on all four sides. When the
/// bitmap carries any transparency we resample premultiplied instead; fully
/// opaque bitmaps (the common photo case) skip the extra passes entirely.
///
/// ★降采样用自带的 [`downscale_box`]，**不用** `imageops::thumbnail`：后者
///   会把每个输出像素的源块算成 `[ceil(left*ratio), ceil(right*ratio))`，
///   既丢掉左端的分数部分（整幅图右/下平移最多 1 个源像素），又因为 f32
///   累加在整数边界上 `ceil` 吃掉一个多余源像素。对不透明照片看不出来，
///   但「剪掉一块再贴回原位」时，洞的边缘会向相邻预览像素渗 1/3~1/2 的
///   alpha，透出棋盘格 —— 就是用户报的那条「粘回去边缘有空隙」的浅色缝。
pub fn resize_to(image: &RgbaImage, target: Size) -> RgbaImage {
    if image.width() == target.width && image.height() == target.height {
        return image.clone();
    }
    let downscale = target.width < image.width() || target.height < image.height();
    if has_transparent_pixel(image) {
        let pma = premultiply(image);
        let scaled = if downscale {
            downscale_box(&pma, target.width, target.height)
        } else {
            image::imageops::resize(&pma, target.width, target.height, FilterType::Triangle)
        };
        return unpremultiply(&scaled);
    }
    if downscale {
        downscale_box(image, target.width, target.height)
    } else {
        image::imageops::resize(image, target.width, target.height, FilterType::Triangle)
    }
}

/// 每个输出像素覆盖哪些源像素、各占多少（权重单位 1/256，每行合计 256）。
///
/// 两端带分数权重，所以输出像素的边缘与源图的边缘严格对齐 —— 这一点是
/// 「洞的边缘不能渗到相邻像素」的前提。
fn axis_weights(src: u32, dst: u32) -> Vec<Vec<(u32, u32)>> {
    let ratio = src as f64 / dst as f64;
    let last = src.saturating_sub(1) as i64;
    let mut out = Vec::with_capacity(dst as usize);
    for i in 0..dst {
        let a = i as f64 * ratio;
        let b = (i as f64 + 1.0) * ratio;
        let mut w: Vec<(u32, u32)> = Vec::with_capacity(4);
        let mut covers: Vec<(u32, f64)> = Vec::with_capacity(4);
        let mut x = a.floor() as i64;
        let mut total = 0.0f64;
        while (x as f64) < b {
            let cov = (b.min((x + 1) as f64) - a.max(x as f64)).max(0.0);
            if cov > 0.0 {
                covers.push((x.clamp(0, last) as u32, cov));
                total += cov;
            }
            x += 1;
        }
        if covers.is_empty() {
            out.push(vec![(i.min(src - 1), 256)]);
            continue;
        }
        let scale = 256.0 / total;
        let mut sum = 0u32;
        for &(ix, cov) in &covers {
            let weight = (cov * scale).round() as u32;
            sum += weight;
            w.push((ix, weight));
        }
        // 浮点四舍五入的余差补到权重最大的一项上，保证每行合计正好 256。
        if sum != 256 {
            if let Some(idx) = (0..w.len()).max_by_key(|&k| w[k].1) {
                w[idx].1 = (w[idx].1 as i64 + 256 - sum as i64).max(1) as u32;
            }
        }
        out.push(w);
    }
    out
}

/// 精确面积平均（box）降采样。
///
/// 竖直方向先把一趟源行按权重累加到一行缓冲，再做水平加权，所以每个源
/// 像素只读一次、且按行顺序读（缓存友好）。权重合计 256×256，据此回到
/// 0..255。行为等价于「每个输出像素是其源区间的面积平均」。
fn downscale_box(image: &RgbaImage, nw: u32, nh: u32) -> RgbaImage {
    let (w, h) = image.dimensions();
    if nw == 0 || nh == 0 {
        return RgbaImage::new(nw, nh);
    }
    let xw = axis_weights(w, nw);
    let yw = axis_weights(h, nh);
    let raw = image.as_raw();
    let stride = w as usize * 4;
    let mut acc = vec![0u32; stride];
    let mut buf = vec![0u8; nw as usize * nh as usize * 4];
    for (oy, yws) in yw.iter().enumerate() {
        acc.fill(0);
        for &(sy, wy) in yws {
            let row = &raw[sy as usize * stride..][..stride];
            for (a, s) in acc.iter_mut().zip(row.iter()) {
                *a += *s as u32 * wy;
            }
        }
        let orow = &mut buf[oy * nw as usize * 4..][..nw as usize * 4];
        for (ox, xws) in xw.iter().enumerate() {
            let mut sum = [0u32; 4];
            for &(sx, wx) in xws {
                let base = sx as usize * 4;
                for c in 0..4 {
                    sum[c] += acc[base + c] * wx;
                }
            }
            for c in 0..4 {
                orow[ox * 4 + c] = ((sum[c] + 32768) / 65536).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(nw, nh, buf).expect("downscale_box buffer size")
}

/// True when at least one pixel is not fully opaque (cheapest possible scan;
/// a fully opaque bitmap never needs the premultiplied round trip).
fn has_transparent_pixel(image: &RgbaImage) -> bool {
    image.as_raw().chunks_exact(4).any(|px| px[3] != 255)
}

/// Straight alpha -> premultiplied alpha (in a fresh buffer).
fn premultiply(image: &RgbaImage) -> RgbaImage {
    let mut out = image.clone();
    for px in out.pixels_mut() {
        let [r, g, b, a] = px.0;
        if a != 255 && a != 0 {
            *px = image::Rgba([
                ((r as u32 * a as u32 + 127) / 255) as u8,
                ((g as u32 * a as u32 + 127) / 255) as u8,
                ((b as u32 * a as u32 + 127) / 255) as u8,
                a,
            ]);
        } else if a == 0 {
            *px = image::Rgba([0, 0, 0, 0]);
        }
    }
    out
}

/// Premultiplied alpha -> straight alpha.
fn unpremultiply(image: &RgbaImage) -> RgbaImage {
    let mut out = image.clone();
    for px in out.pixels_mut() {
        let [r, g, b, a] = px.0;
        if a == 255 || a == 0 {
            if a == 0 {
                *px = image::Rgba([0, 0, 0, 0]);
            }
            continue;
        }
        let f = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
        *px = image::Rgba([f(r), f(g), f(b), a]);
    }
    out
}

pub fn crop_region(image: &RgbaImage, region: Rect) -> RgbaImage {
    let bounds = Rect::new(0, 0, image.width(), image.height());
    let Some(visible) = region.intersect(bounds) else {
        return RgbaImage::new(0, 0);
    };
    image::imageops::crop_imm(
        image,
        visible.x as u32,
        visible.y as u32,
        visible.width,
        visible.height,
    )
    .to_image()
}

pub fn composite_document(doc: &Document, region: Option<Rect>) -> RgbaImage {
    let full = doc.flatten();
    match region {
        Some(rect) => crop_region(&full, rect),
        None => full,
    }
}

pub fn render_for_viewport(doc: &Document, viewport: Viewport) -> RgbaImage {
    let composite = doc.flatten();
    let target = display_size(doc.canvas, viewport.size, viewport.zoom);
    resize_to(&composite, target)
}

/// Fast path for preview generation: when no layer contributes pixels the base
/// image is resized directly (through the crop view), avoiding a full-resolution
/// flatten.
/// Preview composition done directly at `target` scale: the visible base
/// (view crop) is resampled to `target`, then each visible layer is
/// resampled by the same factor and blended at the scaled offset.
///
/// The previous implementation flattened at FULL canvas resolution and
/// downscaled afterwards, which made every live-drag tick resample every
/// layer at full size. This version keeps each tick O(preview pixels) per
/// layer, so dragging stays real-time even on large canvases. Semantics
/// mirror `Document::composite_pixel` (order, opacity, alpha math).
pub fn composite_for_preview(doc: &Document, target: Size) -> RgbaImage {
    PreviewComposer::new().composite(doc, target)
}

/// One cached, preview-scaled bitmap (base or layer) plus its scaled size.
type CachedScaled = (u32, u32, Arc<RgbaImage>);

/// Reusable preview composer.
///
/// `composite_for_preview` resamples EVERY visible layer to the preview
/// target on every call; a live layer drag calls it ~30x/s, so on large
/// canvases each tick paid a full-canvas resample per layer (brutal in
/// debug builds). This composer caches each scaled bitmap between ticks:
///
/// * cache key = `Document::edit_epoch` + target size — any committed edit
///   (execute / undo / redo) bumps the epoch and drops the cache;
/// * history-free live mutations (drag offsets, live mark moves) do NOT
///   bump the epoch, so every drag tick reuses the scaled bitmaps and only
///   pays the blend (O(preview px per layer)).
///
/// The blend applies `layer.opacity` at draw time, so opacity changes stay
/// correct even without a cache miss.
pub struct PreviewComposer {
    epoch: u64,
    target: (u32, u32),
    base: Option<CachedScaled>,
    layers: Vec<Option<CachedScaled>>,
    /// 每个有图层蒙版的层的预览尺度覆盖率（乘数），按 (ox, oy, w, h) 缓存。
    /// 蒙版编辑同样走 `edit_epoch`，`prepare()` 里统一清空。
    masks: Vec<Option<(i32, i32, u32, u32, Arc<Vec<u8>>)>>,
    /// 每个带烘焙样式、且可能当剪贴基底的层的「内容形状」覆盖率（预览尺度）。
    /// 与 `masks` 同一套缓存键与失效规则（`prepare()` 统一清空）。
    clip_shapes: Vec<Option<(i32, i32, u32, u32, Arc<Vec<u8>>)>>,
    /// 每个带「内容之外样式笔迹」的层的预览尺度掩码（见 [`editcore::StylePixels`]）。
    /// 与 `layers` 同一套缓存键（`(nw, nh)`），`prepare()` 统一清空。
    styles: Vec<Option<(u32, u32, Arc<Vec<u8>>)>>,
    /// Scaled bitmap of the layer being live-resized (kept separate from
    /// `layers` so a cancelled gesture cannot poison the layer's cache).
    live: Option<CachedScaled>,
    /// Mirror flags of the cached live bitmap. A handle dragged past the
    /// opposite edge must show the mirrored stretch, and the cache key is
    /// only (w, h) — flipping the flags has to invalidate it.
    live_flip: (bool, bool),
    /// 旋转手势里「缩到预览尺寸的那张快照」。整层缩放一次就要十几毫秒，
    /// 而拖动时快照和预览尺寸都不变，所以按 (会话号, w, h) 缓存，每帧只
    /// 剩下旋转本身。
    rotate_small: Option<(u64, u32, u32, Arc<RgbaImage>)>,
}

/// In-flight layer-resize gesture state for
/// [`PreviewComposer::composite_live_scale`]: the layer index, the layer
/// bitmap as captured when the gesture started, and the requested
/// size/offset in image pixels.

impl Default for PreviewComposer {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewComposer {
    pub fn new() -> Self {
        Self {
            epoch: u64::MAX,
            target: (0, 0),
            base: None,
            layers: Vec::new(),
            masks: Vec::new(),
            clip_shapes: Vec::new(),
            styles: Vec::new(),
            live: None,
            live_flip: (false, false),
            rotate_small: None,
        }
    }

    fn cached_hit(
        slot: &Option<CachedScaled>,
        nw: u32,
        nh: u32,
    ) -> Option<Arc<RgbaImage>> {
        match slot {
            Some((w, h, img)) if *w == nw && *h == nh => Some(img.clone()),
            _ => None,
        }
    }

    /// Cache-key check + scale factors; clears the cache when the key moved.
    fn prepare(&mut self, doc: &Document, target: Size) -> Option<(f64, f64)> {
        if target.width == 0
            || target.height == 0
            || doc.canvas.width == 0
            || doc.canvas.height == 0
        {
            return None;
        }
        let key = (doc.edit_epoch, target.width, target.height);
        if (self.epoch, self.target.0, self.target.1) != key {
            self.epoch = doc.edit_epoch;
            self.target = (target.width, target.height);
            self.base = None;
            self.layers = vec![None; doc.layers.len()];
            self.masks = vec![None; doc.layers.len()];
            self.clip_shapes = vec![None; doc.layers.len()];
            self.styles = vec![None; doc.layers.len()];
            self.live = None;
        }
        Some((
            target.width as f64 / doc.canvas.width as f64,
            target.height as f64 / doc.canvas.height as f64,
        ))
    }

    /// 背景层（`layers[0]`）的预览位图。
    ///
    /// 正常状态（背景未被移动）时走「按 `view` 裁剪 + 重采样」的快路径，避免
    /// 50MP 背景每帧整幅拷贝；被解锁拖走 / 缩放过时退回普通图层路径
    /// （`scaled_layer` + `blend_layer`），几何才是对的。
    fn base_buffer(&mut self, doc: &Document, target: Size, sx: f64, sy: f64) -> RgbaImage {
        let bg = &doc.layers[0];
        if !bg.visible || bg.opacity <= 0.0 {
            return RgbaImage::new(target.width, target.height);
        }
        let Some(r) = bg.raster() else {
            return RgbaImage::new(target.width, target.height);
        };
        let aligned = r.offset.0 == -doc.view.x && r.offset.1 == -doc.view.y;
        // 背景层带蒙版时不能走「整幅裁剪重采样」的快路径（快路径不做蒙版
        // 合成），退回普通图层路径。
        if !aligned || bg.mask.is_some() {
            let mut out = RgbaImage::new(target.width, target.height);
            let (ox, oy, nw, nh) =
                Self::preview_dst(r.offset, (r.image.width(), r.image.height()), sx, sy);
            let src = self.scaled_layer(0, &r.image, nw, nh);
            let mask = self.masked_layer(0, bg, nw, nh);
            let mask_ref = mask.as_deref().map(|m| (&m[..], nw));
            Self::blend_layer(
                &mut out,
                target,
                &src,
                ox,
                oy,
                bg.opacity,
                bg.blend,
                None,
                mask_ref,
                None,
            );
            return out;
        }
        let nw = ((doc.view.width as f64 * sx).round() as u32).max(1);
        let nh = ((doc.view.height as f64 * sy).round() as u32).max(1);
        let img = if let Some(img) = Self::cached_hit(&self.base, nw, nh) {
            (*img).clone()
        } else {
            // "Reset crop" leaves `view` equal to the whole bitmap; cropping
            // then is a pure full-size copy, so hand the base to the resampler
            // directly (a 50 MP base would otherwise be memcpy'd on every
            // render — the copy was ~35 ms of a 1100×688 preview render).
            let full_view = doc.view.x <= 0
                && doc.view.y <= 0
                && doc.view.width >= r.image.width()
                && doc.view.height >= r.image.height();
            let img = if full_view {
                resize_to(&r.image, target)
            } else {
                resize_to(&crop_region(&r.image, doc.view), target)
            };
            self.base = Some((nw, nh, Arc::new(img.clone())));
            img
        };
        if bg.opacity >= 0.999 {
            return img;
        }
        let mut out = RgbaImage::new(img.width(), img.height());
        Self::blend_layer(&mut out, target, &img, 0, 0, bg.opacity, bg.blend, None, None, None);
        out
    }

    /// 把 `layers[1..]` 按 z 序叠到 `out` 上（背景已由 `base_buffer` 画好）。
    /// 位图走缓存 + `blend_layer`，标记按同一套不透明度规则栅格化 ——
    /// 标记不再「恒置顶」，顺序就是图层顺序。
    fn blend_upper_layers(
        &mut self,
        doc: &Document,
        out: &mut RgbaImage,
        target: Size,
        sx: f64,
        sy: f64,
        skip: Option<usize>,
        adjust: Option<(AdjustTarget, Adjust)>,
    ) {
        // 剪贴蒙版基线覆盖率。只有「下一层是剪贴层」时才需要，避免每帧都为
        // 整条图层的每一层重采样一遍覆盖率。
        let mut clip: Option<Vec<u8>> = None;
        for (li, layer) in doc.layers.iter().enumerate().skip(1) {
            if !layer.clipping {
                let next_is_clip = doc
                    .layers
                    .get(li + 1)
                    .map(|next| next.clipping)
                    .unwrap_or(false);
                clip = if next_is_clip {
                    Some(self.clip_coverage(li, layer, target, sx, sy))
                } else {
                    None
                };
            }
            if !layer.visible || layer.opacity <= 0.0 || skip == Some(li) {
                continue;
            }
            let clip_alpha = if layer.clipping {
                clip.as_deref()
            } else {
                None
            };
            match &layer.content {
                LayerContent::Mark(m) => {
                    if layer.opacity >= 0.999 {
                        m.draw_scaled(out, sx as f32, sy as f32);
                    } else {
                        let mut tmp = out.clone();
                        m.draw_scaled(&mut tmp, sx as f32, sy as f32);
                        for (dst, src) in out.pixels_mut().zip(tmp.pixels()) {
                            let p = dst.0;
                            dst.0 = [
                                (p[0] as f32 * (1.0 - layer.opacity)
                                    + src.0[0] as f32 * layer.opacity)
                                    .round() as u8,
                                (p[1] as f32 * (1.0 - layer.opacity)
                                    + src.0[1] as f32 * layer.opacity)
                                    .round() as u8,
                                (p[2] as f32 * (1.0 - layer.opacity)
                                    + src.0[2] as f32 * layer.opacity)
                                    .round() as u8,
                                (p[3] as f32 * (1.0 - layer.opacity)
                                    + src.0[3] as f32 * layer.opacity)
                                    .round() as u8,
                            ];
                        }
                    }
                }
                LayerContent::Raster(r) => {
                    let (ox, oy, nw, nh) =
                        Self::preview_dst(r.offset, (r.image.width(), r.image.height()), sx, sy);
                    let src = self.scaled_layer(li, &r.image, nw, nh);
                    let mask = self.masked_layer(li, layer, nw, nh);
                    let mask_ref = mask.as_deref().map(|m| (&m[..], nw));
                    // 内容之外的样式笔迹：这些像素按样式自己的混合模式混合。
                    let style = layer.style.as_ref().map(|s| {
                        let scaled = self.scaled_style(
                            li,
                            &s.mask,
                            (r.image.width(), r.image.height()),
                            nw,
                            nh,
                        );
                        (scaled, s.blend)
                    });
                    let style_ref = style.as_ref().map(|(m, b)| (&m[..], *b));
                    match adjust {
                        Some((AdjustTarget::Layer(index), params)) if index == li => {
                            let mut adjusted = (*src).clone();
                            params.apply_in_place(&mut adjusted);
                            Self::blend_layer(
                                out,
                                target,
                                &adjusted,
                                ox,
                                oy,
                                layer.opacity,
                                layer.blend,
                                clip_alpha,
                                mask_ref,
                                style_ref,
                            );
                        }
                        _ => Self::blend_layer(
                            out,
                            target,
                            &src,
                            ox,
                            oy,
                            layer.opacity,
                            layer.blend,
                            clip_alpha,
                            mask_ref,
                            style_ref,
                        ),
                    }
                }
            }
        }
    }

    /// 位图在预览里的目标矩形 `(x, y, w, h)`（整数预览像素）。
    ///
    /// ★两端各自取整：起点 `floor`、终点 `ceil`；**不能**用「起点四舍五入 +
    ///   宽高四舍五入」。后者会让图层比真实落点少覆盖不到 1 个预览像素，而
    ///   背景上被挖走的那块洞是按面积比降采样的（洞边缘自带分数 alpha），
    ///   差出来的那半行就透出棋盘格 —— 在 13% 这类缩小档下正好是一条浅色
    ///   「缝」，看起来像「剪切出来的内容粘回去没贴严」（用户报的就是这个）。
    ///
    /// 实测：120×80 的粘贴层放在 (100,100)，缩到 13% 时真实占位是
    /// x 13.0..28.6 / y 13.0..23.4；旧算法给出 16×10 @ (13,13)，只盖到
    /// 23 行，第 23 行留下 alpha=0.62 的缺口。改成向外取整后是 16×11，
    /// 洞被完全盖住。两个相邻图层宁可重叠 <1 像素，也不该留缝。
    ///
    /// 1:1 与整数倍缩放下两端本来就落在整数上，行为完全不变。
    fn preview_dst(offset: (i32, i32), size: (u32, u32), sx: f64, sy: f64) -> (i32, i32, u32, u32) {
        const EPS: f64 = 1e-6;
        let x0 = (offset.0 as f64 * sx + EPS).floor();
        let y0 = (offset.1 as f64 * sy + EPS).floor();
        let x1 = ((offset.0 as f64 + size.0 as f64) * sx - EPS).ceil();
        let y1 = ((offset.1 as f64 + size.1 as f64) * sy - EPS).ceil();
        let w = (x1 - x0).round().max(1.0) as u32;
        let h = (y1 - y0).round().max(1.0) as u32;
        (x0 as i32, y0 as i32, w, h)
    }

    /// Cached preview-scaled copy of a layer bitmap, stored in slot `li`.
    fn scaled_layer(&mut self, li: usize, img: &RgbaImage, nw: u32, nh: u32) -> Arc<RgbaImage> {
        let slot = self.layers.get(li).cloned().flatten();
        if let Some(cached) = Self::cached_hit(&slot, nw, nh) {
            return cached;
        }
        let scaled = Arc::new(resize_to(img, Size::new(nw, nh)));
        if li < self.layers.len() {
            self.layers[li] = Some((nw, nh, scaled.clone()));
        }
        scaled
    }

    /// Alpha-over blend of `src` at `(ox, oy)` into `out_buf`, applying
    /// `opacity` at draw time. Row-slice fast path: whole rows are handed
    /// out as byte slices so the per-tick cost stays low even in debug
    /// builds.
    ///
    /// `clip` 是剪贴蒙版基线层的覆盖率（预览尺度，与 `out_buf` 同尺寸）；
    /// `mask` 是图层蒙版的覆盖率（**图层自己的矩形尺寸**，宽度一并传进来）。
    /// `style` 是「内容之外的样式笔迹」掩码（图层矩形尺寸）连同样式自己的
    /// 混合模式：那些像素改用它混合，见 [`editcore::StylePixels`]。
    /// 两者都是「src alpha 乘数」，先乘蒙版再乘剪贴 —— 与
    /// `Document::flatten_in_doc_space` 的 `composite_raster` 同序。
    /// 基底层 / 无剪贴层自己混合时对应参数传 `None`。
    fn blend(
        out_buf: &mut [u8],
        target: Size,
        src: &RgbaImage,
        ox: i32,
        oy: i32,
        opacity: f32,
        mode: editcore::BlendMode,
        clip: Option<&[u8]>,
        mask: Option<(&[u8], u32)>,
        style: Option<(&[u8], editcore::BlendMode)>,
    ) {
        let nw = src.width() as i32;
        let nh = src.height() as i32;
        let x0 = ox.max(0) as u32;
        let x1 = (ox + nw).min(target.width as i32).max(0) as u32;
        if x0 >= x1 {
            return;
        }
        let src_stride = nw as usize * 4;
        let src_buf: &[u8] = src.as_raw();
        let out_stride = target.width as usize * 4;
        // 有样式掩码时不能走 Normal 快路径：每个像素的模式可能不同。
        let normal = mode.is_normal() && style.is_none();
        for ty in 0..target.height {
            let ly = ty as i64 - oy as i64;
            if ly < 0 || ly >= nh as i64 {
                continue;
            }
            let srow = &src_buf[ly as usize * src_stride..(ly as usize + 1) * src_stride];
            let orow =
                &mut out_buf[ty as usize * out_stride..(ty as usize + 1) * out_stride];
            for tx in x0..x1 {
                let lx = tx as i64 - ox as i64;
                let so = lx as usize * 4;
                let oo = tx as usize * 4;
                if normal {
                    // 与 `composite_raster` + `blend_pixel_f32` 完全同序：
                    // alpha = (a/255 × 蒙版) × 剪贴，再乘图层不透明度。
                    let mut alpha = srow[so + 3] as f32 / 255.0;
                    if let Some((m, mw)) = mask {
                        alpha *= m[ly as usize * mw as usize + lx as usize] as f32 / 255.0;
                    }
                    if let Some(c) = clip {
                        let ci = ty as usize * target.width as usize + tx as usize;
                        alpha *= c[ci] as f32 / 255.0;
                    }
                    let sa = alpha * opacity;
                    if sa <= 0.0 {
                        continue;
                    }
                    let inv = 1.0 - sa;
                    for c in 0..3 {
                        orow[oo + c] =
                            (srow[so + c] as f32 * sa + orow[oo + c] as f32 * inv).round() as u8;
                    }
                    orow[oo + 3] = ((sa + orow[oo + 3] as f32 / 255.0 * inv) * 255.0).round() as u8;
                } else {
                    let mut dst = [
                        orow[oo],
                        orow[oo + 1],
                        orow[oo + 2],
                        orow[oo + 3],
                    ];
                    let mut px = [
                        srow[so],
                        srow[so + 1],
                        srow[so + 2],
                        srow[so + 3],
                    ];
                    if let Some((m, mw)) = mask {
                        let mcov = m[ly as usize * mw as usize + lx as usize] as u32;
                        px[3] = ((px[3] as u32 * mcov + 127) / 255) as u8;
                    }
                    if let Some(c) = clip {
                        let cov = c[ty as usize * target.width as usize + tx as usize] as u32;
                        px[3] = ((px[3] as u32 * cov + 127) / 255) as u8;
                    }
                    let px_mode = match style {
                        Some((s, smode)) if s[ly as usize * nw as usize + lx as usize] != 0 => {
                            smode
                        }
                        _ => mode,
                    };
                    // 溶解：按目标像素坐标抖动成 0/1，再走 Normal；1:1 预览下
                    // `tx` 就是文档坐标，与 flatten 的抖动图案逐像素一致。
                    let (px, op, px_mode) = if px_mode.is_dissolve() {
                        let a = px[3] as f32 / 255.0 * opacity;
                        if editcore::dissolve_hit(tx as i32, ty as i32, a) {
                            ([px[0], px[1], px[2], 255], 1.0, editcore::BlendMode::Normal)
                        } else {
                            ([0, 0, 0, 0], 0.0, editcore::BlendMode::Normal)
                        }
                    } else {
                        (px, opacity, px_mode)
                    };
                    editcore::blend_pixel(&mut dst, px, op, px_mode);
                    orow[oo..oo + 4].copy_from_slice(&dst);
                }
            }
        }
    }

    fn blend_layer(
        out: &mut RgbaImage,
        target: Size,
        src: &RgbaImage,
        ox: i32,
        oy: i32,
        opacity: f32,
        mode: editcore::BlendMode,
        clip: Option<&[u8]>,
        mask: Option<(&[u8], u32)>,
        style: Option<(&[u8], editcore::BlendMode)>,
    ) {
        let mut raw = std::mem::take(out).into_raw();
        Self::blend(&mut raw, target, src, ox, oy, opacity, mode, clip, mask, style);
        *out = RgbaImage::from_raw(target.width, target.height, raw)
            .expect("preview buffer size preserved");
    }

    /// 蒙版在画布区间 `[x0, x1) × [y0, y1)` 上的平均覆盖率（u8）。
    ///
    /// 区间端点向外取整后逐像素取**字节**平均（不是取采样点），与位图的降
    /// 采样口径一致：`downscale_box` 的一个输出像素正好覆盖源图同一段区间，
    /// 蒙版按同一段区间取平均才不会在缩略图上「透出/多挡」半行。
    ///
    /// 缩放比恰好为 1 时区间里只有一个像素，结果逐字节等于
    /// `mask.coverage()` 的 u8 值，1:1 预览才能和 flatten 逐字节一致。
    fn mask_cov_range(mask: &LayerMask, x0: f64, x1: f64, y0: f64, y1: f64) -> u8 {
        let cx0 = x0.floor() as i32;
        let cx1 = (x1.ceil() as i32).max(cx0 + 1);
        let cy0 = y0.floor() as i32;
        let cy1 = (y1.ceil() as i32).max(cy0 + 1);
        let mut sum = 0u64;
        let mut n = 0u64;
        for y in cy0..cy1 {
            for x in cx0..cx1 {
                let lx = x - mask.offset.0;
                let ly = y - mask.offset.1;
                let v = if lx < 0
                    || ly < 0
                    || lx >= mask.image.width() as i32
                    || ly >= mask.image.height() as i32
                {
                    mask.default
                } else {
                    mask.image.get_pixel(lx as u32, ly as u32).0[0]
                };
                sum += v as u64;
                n += 1;
            }
        }
        ((sum + n / 2) / n.max(1)) as u8
    }

    /// 一层位图在预览尺度上的蒙版覆盖率数组（长度 = `nw * nh`）。
    ///
    /// 输出像素 `(tx, ty)` 覆盖源图局部区间 `[tx*sw/nw, (tx+1)*sw/nw)`，
    /// 加图层 offset 后交给 [`Self::mask_cov_range`] —— 与 `resize_to` 对同一
    /// 层位图做的区间平均严格对应，缩小时不会因 `preview_dst` 的 floor/ceil
    /// 而错位。
    fn mask_cov_for_rect(
        mask: &LayerMask,
        offset: (i32, i32),
        sw: u32,
        sh: u32,
        nw: u32,
        nh: u32,
    ) -> Vec<u8> {
        let mut cov = vec![255u8; nw as usize * nh as usize];
        if nw == 0 || nh == 0 {
            return cov;
        }
        let rx = sw as f64 / nw as f64;
        let ry = sh as f64 / nh as f64;
        for ty in 0..nh {
            let y0 = offset.1 as f64 + ty as f64 * ry;
            let y1 = offset.1 as f64 + (ty as f64 + 1.0) * ry;
            for tx in 0..nw {
                let x0 = offset.0 as f64 + tx as f64 * rx;
                let x1 = offset.0 as f64 + (tx as f64 + 1.0) * rx;
                cov[ty as usize * nw as usize + tx as usize] =
                    Self::mask_cov_range(mask, x0, x1, y0, y1);
            }
        }
        cov
    }

    /// 图层蒙版在预览尺度上的 alpha 乘数（长度 = 该图层缩放后的像素数）。
    ///
    /// 每帧为每个有蒙版的图层重算一次是整块 O(像素) 的活，所以按
    /// `(offset.x, offset.y, nw, nh)` 缓存到 `self.masks[li]`：拖动只改
    /// offset 时缓存失效一次，纯重绘（offset / 尺寸都没变）直接命中；换
    /// epoch / 换预览尺寸由 `prepare()` 统一清空。全 255（蒙版没有真正遮挡）
    /// 时返回 `None`，让热路径不付任何逐像素代价。
    ///
    /// 矢量标记不在这里处理：`flatten_in_doc_space` 的 Mark 分支不应用蒙版，
    /// 预览保持同一语义。
    fn masked_layer(&mut self, li: usize, layer: &Layer, nw: u32, nh: u32) -> Option<Arc<Vec<u8>>> {
        let Some(mask) = layer.mask.as_ref() else {
            return None;
        };
        let Some(r) = layer.raster() else {
            return None;
        };
        if let Some((cx, cy, cw, ch, cov)) = self.masks.get(li).cloned().flatten() {
            if cx == r.offset.0 && cy == r.offset.1 && cw == nw && ch == nh {
                return Some(cov);
            }
        }
        let cov = Self::mask_cov_for_rect(
            mask,
            r.offset,
            r.image.width(),
            r.image.height(),
            nw,
            nh,
        );
        if cov.iter().all(|&c| c == 255) {
            if li < self.masks.len() {
                self.masks[li] = None;
            }
            return None;
        }
        let arc = Arc::new(cov);
        if li < self.masks.len() {
            self.masks[li] = Some((r.offset.0, r.offset.1, nw, nh, arc.clone()));
        }
        Some(arc)
    }

    /// 剪贴基线层的覆盖率（预览尺度 u8 alpha）。
    ///
    /// 与 `Document::flatten_in_doc_space` 里的 `fill_clip_coverage` 同语义：
    /// 覆盖率 = 基底位图 alpha × 基底自己的图层蒙版；隐藏 / 全透明的基底给出
    /// 全 0，于是整条剪贴链一起消失；基底只是「半透明」（opacity < 1）时覆盖
    /// 率仍给满，不乘不透明度。
    ///
    /// 覆盖率必须落在**预览尺度**上：图层在预览里是缩小后混合的，如果拿文
    /// 档尺度的覆盖率去索引就会错位，剪贴内容会跑到基线框外面。
    fn clip_coverage(
        &mut self,
        li: usize,
        layer: &Layer,
        target: Size,
        sx: f64,
        sy: f64,
    ) -> Vec<u8> {
        if !layer.visible || layer.opacity <= 0.0 {
            return vec![0u8; target.width as usize * target.height as usize];
        }
        // 基底带烘焙样式时，边界取它的**内容**形状：样式额外画出来的外描边 /
        // 投影不算内容，否则剪贴内容会铺到描边上（与 flatten 侧
        // `fill_clip_coverage` 同口径）。
        if let Some(shape) = layer.clip_shape.as_ref() {
            return self.clip_shape_coverage(li, layer, shape, target, sx, sy);
        }
        let mut cov = vec![0u8; target.width as usize * target.height as usize];
        let (src, ox, oy, mcov) = match &layer.content {
            LayerContent::Raster(r) => {
                let (ox, oy, nw, nh) =
                    Self::preview_dst(r.offset, (r.image.width(), r.image.height()), sx, sy);
                let src = self.scaled_layer(li, &r.image, nw, nh);
                let mcov = self.masked_layer(li, layer, nw, nh);
                (src, ox, oy, mcov)
            }
            LayerContent::Mark(_) => {
                let (plane, offset) = layer.plane();
                let (ox, oy, nw, nh) =
                    Self::preview_dst(offset, (plane.width(), plane.height()), sx, sy);
                // 与 `fill_clip_coverage` 一样，标记基底也要乘自己的蒙版。
                let mcov = layer.mask.as_ref().map(|mask| {
                    Arc::new(Self::mask_cov_for_rect(
                        mask,
                        offset,
                        plane.width(),
                        plane.height(),
                        nw,
                        nh,
                    ))
                });
                (
                    Arc::new(resize_to(&plane, Size::new(nw, nh))),
                    ox,
                    oy,
                    mcov,
                )
            }
        };
        let mw = src.width();
        let tw = target.width as i32;
        let th = target.height as i32;
        for ty in 0..src.height() as i32 {
            let py = oy + ty;
            if py < 0 || py >= th {
                continue;
            }
            for tx in 0..src.width() as i32 {
                let px = ox + tx;
                if px < 0 || px >= tw {
                    continue;
                }
                let mut a = src.get_pixel(tx as u32, ty as u32).0[3];
                if a > 0 {
                    if let Some(m) = mcov.as_deref() {
                        let m = m[ty as usize * mw as usize + tx as usize] as u32;
                        a = ((a as u32 * m + 127) / 255) as u8;
                    }
                }
                if a > 0 {
                    cov[py as usize * tw as usize + px as usize] = a;
                }
            }
        }
        cov
    }

    /// 剪贴基底的「内容形状」在预览尺度上的覆盖率。
    ///
    /// 与 [`Self::clip_coverage`] 的位图路径同语义（同样的预览矩形、同样的
    /// 蒙版顺序），只是边界换成烘焙样式**之前**的内容 alpha。形状的
    /// 区间平均借 `resize_to` 走同一条降采样路径，缩小时两者的边缘口径一致。
    fn clip_shape_coverage(
        &mut self,
        li: usize,
        layer: &Layer,
        shape: &LayerMask,
        target: Size,
        sx: f64,
        sy: f64,
    ) -> Vec<u8> {
        let mut cov = vec![0u8; target.width as usize * target.height as usize];
        let (sw, sh) = shape.image.dimensions();
        let (ox, oy, nw, nh) = Self::preview_dst(shape.offset, (sw, sh), sx, sy);
        let scaled = self.scaled_clip_shape(li, shape, nw, nh);
        let mcov = layer
            .mask
            .as_ref()
            .map(|m| Self::mask_cov_for_rect(m, shape.offset, sw, sh, nw, nh));
        let tw = target.width as i32;
        let th = target.height as i32;
        for ty in 0..nh as i32 {
            let py = oy + ty;
            if py < 0 || py >= th {
                continue;
            }
            for tx in 0..nw as i32 {
                let px = ox + tx;
                if px < 0 || px >= tw {
                    continue;
                }
                let i = ty as usize * nw as usize + tx as usize;
                let a = scaled[i];
                if a == 0 {
                    continue;
                }
                let a = match mcov.as_deref() {
                    Some(m) => ((a as u32 * m[i] as u32 + 127) / 255) as u8,
                    None => a,
                };
                if a > 0 {
                    cov[py as usize * tw as usize + px as usize] = a;
                }
            }
        }
        cov
    }

    /// 内容形状缩到预览尺度（单通道 alpha），按 `(offset, w, h)` 缓存。
    ///
    /// 单通道借用 RGBA 的重采样路径：alpha 的区间平均与同尺寸位图完全同
    /// 口径，1:1 时逐字节不变（预览与 flatten 才能逐字节对上）。
    fn scaled_clip_shape(
        &mut self,
        li: usize,
        shape: &LayerMask,
        nw: u32,
        nh: u32,
    ) -> Arc<Vec<u8>> {
        let key = (shape.offset.0, shape.offset.1, nw, nh);
        if let Some((cx, cy, cw, ch, cov)) = self.clip_shapes.get(li).cloned().flatten() {
            if (cx, cy, cw, ch) == key {
                return cov;
            }
        }
        let (sw, sh) = shape.image.dimensions();
        let mut plane = RgbaImage::from_pixel(sw, sh, image::Rgba([0, 0, 0, 0]));
        for (dst, src) in plane.pixels_mut().zip(shape.image.pixels()) {
            dst.0[3] = src.0[0];
        }
        let scaled = resize_to(&plane, Size::new(nw, nh));
        let cov = Arc::new(scaled.pixels().map(|p| p.0[3]).collect::<Vec<u8>>());
        if li < self.clip_shapes.len() {
            self.clip_shapes[li] = Some((
                shape.offset.0,
                shape.offset.1,
                nw,
                nh,
                cov.clone(),
            ));
        }
        cov
    }

    /// 「内容之外的样式笔迹」缩到预览尺度（单通道），按 `(w, h)` 缓存。
    fn scaled_style(
        &mut self,
        li: usize,
        mask: &[u8],
        size: (u32, u32),
        nw: u32,
        nh: u32,
    ) -> Arc<Vec<u8>> {
        if let Some((cw, ch, cov)) = self.styles.get(li).cloned().flatten() {
            if (cw, ch) == (nw, nh) {
                return cov;
            }
        }
        let mut plane = RgbaImage::from_pixel(size.0, size.1, image::Rgba([0, 0, 0, 0]));
        for (dst, src) in plane.as_mut().chunks_exact_mut(4).zip(mask.iter()) {
            dst[3] = *src;
        }
        let scaled = resize_to(&plane, Size::new(nw, nh));
        let cov = Arc::new(scaled.pixels().map(|p| p.0[3]).collect::<Vec<u8>>());
        if li < self.styles.len() {
            self.styles[li] = Some((nw, nh, cov.clone()));
        }
        cov
    }

    pub fn composite(&mut self, doc: &Document, target: Size) -> RgbaImage {
        self.composite_adjusted(doc, target, None)
    }

    /// Preview composite with an optional live "调整颜色" adjustment applied to
    /// the pixels of `scope` (the selected layer, or the base photo).
    ///
    /// The adjustment runs on the **preview-resolution** bitmap of the scope,
    /// never on document pixels, so a slider drag keeps the live-gesture rule
    /// (document untouched, `edit_epoch` stable, cached scaled layers reused).
    /// `Adjust::apply_in_place` short-circuits when every parameter is
    /// neutral, so an `Adjust::identity()` scope costs nothing.
    /// Markup annotations are drawn last and are never color-adjusted.
    pub fn composite_adjusted(
        &mut self,
        doc: &Document,
        target: Size,
        adjust: Option<(AdjustTarget, Adjust)>,
    ) -> RgbaImage {
        // 需要隔离的组（非穿透 / 组不透明度 < 100%）只在 core 的合成里有语义，
        // 逐层预览没法表达「整组先合成再混合」。这类文件退回整幅 flatten 再
        // 降采样：慢一点，但保证预览与导出逐像素一致。实时调色仍走下面的
        // 逐层路径（拖动期间的过渡态）。
        if adjust.is_none() && doc.groups.iter().any(|g| g.isolated()) {
            return resize_to(&doc.flatten(), target);
        }
        let Some((sx, sy)) = self.prepare(doc, target) else {
            return RgbaImage::new(target.width.max(1), target.height.max(1));
        };
        let mut out = self.base_buffer(doc, target, sx, sy);
        if let Some((AdjustTarget::Base, params)) = adjust {
            params.apply_in_place(&mut out);
        }
        self.blend_upper_layers(doc, &mut out, target, sx, sy, None, adjust);
        doc.apply_display_color(&mut out);
        out
    }

    /// Convenience wrapper around [`PreviewComposer::composite_adjusted`].
    pub fn composite_with_adjust(
        &mut self,
        doc: &Document,
        target: Size,
        scope: AdjustTarget,
        adjust: &Adjust,
    ) -> RgbaImage {
        self.composite_adjusted(doc, target, Some((scope, *adjust)))
    }

    /// Preview composite while a layer is being live-resized.
    ///
    /// The dragged layer is drawn from `state.snapshot` stretched straight
    /// to `state.to`/`state.offset` (converted to preview pixels), so the
    /// document bitmap itself is NEVER resampled during the gesture — the
    /// epoch stays put and base plus all other layers keep their cached
    /// scaled bitmaps. Each tick only pays one Nearest stretch of the
    /// snapshot into the layer's preview rect + the blend; the quality
    /// Triangle resample happens exactly once, in
    /// `Document::scale_layer` when the gesture commits.
    pub fn composite_live_scale(
        &mut self,
        doc: &Document,
        target: Size,
        layer_index: usize,
        snapshot: &RgbaImage,
        to: Size,
        offset: (i32, i32),
    ) -> RgbaImage {
        self.composite_live_scale_adjusted(
            doc,
            target,
            layer_index,
            snapshot,
            to,
            offset,
            None,
            (false, false),
        )
    }

    /// [`PreviewComposer::composite_live_scale`] plus an optional live color
    /// adjustment (the "调整颜色" panel can stay open while a layer handle is
    /// dragged) and mirroring for handles dragged past the opposite edge.
    #[allow(clippy::too_many_arguments)]
    pub fn composite_live_scale_adjusted(
        &mut self,
        doc: &Document,
        target: Size,
        layer_index: usize,
        snapshot: &RgbaImage,
        to: Size,
        offset: (i32, i32),
        adjust: Option<(AdjustTarget, Adjust)>,
        flip: (bool, bool),
    ) -> RgbaImage {
        let Some((sx, sy)) = self.prepare(doc, target) else {
            return RgbaImage::new(target.width.max(1), target.height.max(1));
        };
        let drag_opacity = doc
            .layers
            .get(layer_index)
            .map(|l| if l.visible { l.opacity } else { 0.0 })
            .unwrap_or(0.0);
        // The dragged layer keeps its own blend mode, otherwise a `Multiply`
        // layer would flash to Normal for the duration of the gesture.
        let drag_blend = doc
            .layers
            .get(layer_index)
            .map(|l| l.blend)
            .unwrap_or_default();
        let mut out = self.base_buffer(doc, target, sx, sy);
        if let Some((AdjustTarget::Base, params)) = adjust {
            params.apply_in_place(&mut out);
        }
        self.blend_upper_layers(doc, &mut out, target, sx, sy, Some(layer_index), adjust);
        if drag_opacity > 0.0 {
            let nw = ((to.width as f64 * sx).round() as u32).max(1);
            let nh = ((to.height as f64 * sy).round() as u32).max(1);
            // 缓存键只有 (w, h)：翻转标记变了就必须重算，否则镜像前后会
            // 复用同一张未翻转的位图。
            let hit = if self.live_flip == flip {
                Self::cached_hit(&self.live, nw, nh)
            } else {
                None
            };
            let src = match hit {
                Some(img) => img,
                None => {
                    // P1-A 质量分层（D14/R11）：拖动帧用双线性（Triangle），
                    // 不再 Nearest 马赛克；落定仍走 HQ（Lanczos3/sample HQ），
                    // 「不许把落定画质降到拖动水平」的反向约束照旧。
                    let mut img = image::imageops::resize(
                        snapshot,
                        nw,
                        nh,
                        FilterType::Triangle,
                    );
                    if flip.0 {
                        img = image::imageops::flip_horizontal(&img);
                    }
                    if flip.1 {
                        img = image::imageops::flip_vertical(&img);
                    }
                    let img = Arc::new(img);
                    self.live = Some((nw, nh, img.clone()));
                    self.live_flip = flip;
                    img
                }
            };
            let ox = (offset.0 as f64 * sx).round() as i32;
            let oy = (offset.1 as f64 * sy).round() as i32;
            let stretched = match adjust {
                Some((AdjustTarget::Layer(index), params)) if index == layer_index => {
                    let mut adjusted = (*src).clone();
                    params.apply_in_place(&mut adjusted);
                    adjusted
                }
                _ => (*src).clone(),
            };
            Self::blend_layer(&mut out, target, &stretched, ox, oy, drag_opacity, drag_blend, None, None, None);
        }
        doc.apply_display_color(&mut out);
        out
    }

    /// 拖动旋转时的实时合成（文档完全不动，和 `live_scale` 一个套路）。
    ///
    /// 顺序是「先把图层缩到预览尺寸，再在小图上转」：单帧代价只跟预览像素
    /// 数有关，原图多大都一样。反过来（先转整层再缩放）在 2200×1500 的图上
    /// 每帧要 40ms 以上 —— 那正是旋转手柄一顿一顿的原因。
    ///
    /// P1-A：斜切拖动帧的实时合成（Ctrl+拖边）。文档像素不动：活动层按
    /// 「起手快照 → 预览尺寸 → LayerTransform（与落定同一个 sample_affine
    /// 采样器，hq=false 双线性）」画出，其余图层照旧。`skew` 是本次手势
    /// 的累计剪切量，`center_delta` 是 W/N 缘的对侧锚定补偿（图像坐标）。
    pub fn composite_live_skew_adjusted(
        &mut self,
        doc: &Document,
        target: Size,
        layer_index: usize,
        snapshot: &RgbaImage,
        skew: (f64, f64),
        center_delta: (f64, f64),
        adjust: Option<(AdjustTarget, Adjust)>,
    ) -> RgbaImage {
        let Some((sx, sy)) = self.prepare(doc, target) else {
            return RgbaImage::new(target.width.max(1), target.height.max(1));
        };
        let drag_opacity = doc
            .layers
            .get(layer_index)
            .map(|l| if l.visible { l.opacity } else { 0.0 })
            .unwrap_or(0.0);
        // The dragged layer keeps its own blend mode, otherwise a `Multiply`
        // layer would flash to Normal for the duration of the gesture.
        let drag_blend = doc
            .layers
            .get(layer_index)
            .map(|l| l.blend)
            .unwrap_or_default();
        let mut out = self.base_buffer(doc, target, sx, sy);
        if let Some((AdjustTarget::Base, params)) = adjust {
            params.apply_in_place(&mut out);
        }
        self.blend_upper_layers(doc, &mut out, target, sx, sy, Some(layer_index), adjust);
        if drag_opacity > 0.0 {
            let Some(layer) = doc.layers.get(layer_index) else {
                return out;
            };
            // 起手参数（手势不写文档，layer.transform 就是 begin 状态）。
            let Some(lr) = layer.raster() else {
                return out;
            };
            let base = lr.transform.clone().unwrap_or_else(|| {
                editcore::LayerTransform::from_offset(lr.image.clone(), lr.offset.0, lr.offset.1)
            });
            let (sw, sh) = base.source_size();
            let nw = ((sw * base.scale.0.abs() * sx).round() as u32).max(1);
            let nh = ((sh * base.scale.1.abs() * sy).round() as u32).max(1);
            // 复用 live 缓存（键 (w,h)，flip 恒 (false,false)：斜切不镜像，
            // 若上一段缩放手势留了 flip=true 的缓存，键不匹配会自动重算）。
            let scaled = if self.live_flip == (false, false) {
                Self::cached_hit(&self.live, nw, nh)
            } else {
                None
            };
            let scaled = match scaled {
                Some(img) => img,
                None => {
                    let img = Arc::new(image::imageops::resize(
                        snapshot,
                        nw,
                        nh,
                        FilterType::Triangle,
                    ));
                    self.live = Some((nw, nh, img.clone()));
                    self.live_flip = (false, false);
                    img
                }
            };
            // 预览空间的变换：source 已是「预览尺寸的显示位图」，scale 归 1，
            // center / 补偿量按 (sx, sy) 折算，旋转 / 斜切角度原样保留。
            //（source 字段直接换绑 Arc，避免为构造再拷一次位图。）
            let mut t = editcore::LayerTransform::new(RgbaImage::new(1, 1));
            t.source = scaled;
            t.rotation = base.rotation;
            t.skew = (base.skew.0 + skew.0, base.skew.1 + skew.1);
            t.center = (
                (base.center.0 + center_delta.0) * sx,
                (base.center.1 + center_delta.1) * sy,
            );
            let active = t.render(
                Rect {
                    x: 0,
                    y: 0,
                    width: target.width,
                    height: target.height,
                },
                false,
            );
            let stretched = match adjust {
                Some((AdjustTarget::Layer(index), params)) if index == layer_index => {
                    let mut adjusted = active;
                    params.apply_in_place(&mut adjusted);
                    adjusted
                }
                _ => active,
            };
            Self::blend_layer(&mut out, target, &stretched, 0, 0, drag_opacity, drag_blend, None, None, None);
        }
        doc.apply_display_color(&mut out);
        out
    }

    pub fn composite_live_rotate_adjusted(
        &mut self,
        doc: &Document,
        target: Size,
        layer_index: usize,
        snapshot: &RgbaImage,
        center: (f64, f64),
        degrees: f32,
        expand: bool,
        token: u64,
        adjust: Option<(AdjustTarget, Adjust)>,
    ) -> RgbaImage {
        let Some((sx, sy)) = self.prepare(doc, target) else {
            return RgbaImage::new(target.width.max(1), target.height.max(1));
        };
        let drag_opacity = doc
            .layers
            .get(layer_index)
            .map(|l| if l.visible { l.opacity } else { 0.0 })
            .unwrap_or(0.0);
        // The dragged layer keeps its own blend mode, otherwise a `Multiply`
        // layer would flash to Normal for the duration of the gesture.
        let drag_blend = doc
            .layers
            .get(layer_index)
            .map(|l| l.blend)
            .unwrap_or_default();
        let mut out = self.base_buffer(doc, target, sx, sy);
        if let Some((AdjustTarget::Base, params)) = adjust {
            params.apply_in_place(&mut out);
        }
        self.blend_upper_layers(doc, &mut out, target, sx, sy, Some(layer_index), adjust);
        if drag_opacity > 0.0 {
            // 预览尺度下的图层尺寸：Triangle 是「每个输出像素读 2×2」，代价
            // 随输出走，不会因为原图巨大而爆炸。
            let pw = ((snapshot.width() as f64 * sx).round() as u32).max(1);
            let ph = ((snapshot.height() as f64 * sy).round() as u32).max(1);
            // 缩放结果按 (会话号, w, h) 缓存：拖动时这三个值都不变，每帧省掉
            // 一次整层降采样（debug 下这是最大的一笔开销）。
            let hit = self
                .rotate_small
                .as_ref()
                .is_some_and(|(t, w, h, _)| *t == token && *w == pw && *h == ph);
            if !hit {
                let small = image::imageops::resize(snapshot, pw, ph, FilterType::Triangle);
                self.rotate_small = Some((token, pw, ph, Arc::new(small)));
            }
            let small = &self.rotate_small.as_ref().expect("just filled").3;
            // ★固定输出尺寸（对角线正方形、取偶 + 2px 余量）：整段拖拽里它
            // 不变，采样网格也就恒定，每帧只有角度在动。尺寸取偶是硬要求 ——
            // 奇数尺寸会让内容中心落在半像素上，见 `expanded_size` 的说明。
            let (nw, nh) = editcore::rotated_box_size(pw, ph, expand);
            let rad = degrees.to_radians();
            // ★关键：把内容中心摆到**落定后**的中心（画布坐标 * 预览倍率）。
            // 落定同样是 `round(center - W/2)`，W 恒为偶数 → 两边都锚在
            // `round(center)` 上，拖动不抖、松手不挪。
            let cxp = center.0 * sx;
            let cyp = center.1 * sy;
            let ox = editcore::round_center(cxp - nw as f64 / 2.0) as i32;
            let oy = editcore::round_center(cyp - nh as f64 / 2.0) as i32;
            let rotated = editcore::rotate_into_centered(
                small,
                rad.sin(),
                rad.cos(),
                nw,
                nh,
                ((cxp - ox as f64) as f32, (cyp - oy as f64) as f32),
            );
            // 诊断开关：`set IMAGEEDIT_ROTATE_TRACE=1` 才打印每帧几何，
            // 正常使用不刷日志（拖动时每帧一行会淹掉别的输出）。
            if std::env::var_os("IMAGEEDIT_ROTATE_TRACE").is_some() {
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"rotate-live\",\"deg\":{degrees:.2},\"s\":[{pw}x{ph}],\
                     \"out\":[{nw}x{nh}],\"place\":[{ox},{oy}],\"c\":[{cxp:.2},{cyp:.2}],\
                     \"dc\":[{:.2},{:.2}],\"sx\":{sx:.4}}}",
                    cxp - ox as f64,
                    cyp - oy as f64
                );
            }
            let src = match adjust {
                Some((AdjustTarget::Layer(index), params)) if index == layer_index => {
                    let mut adjusted = rotated;
                    params.apply_in_place(&mut adjusted);
                    adjusted
                }
                _ => rotated,
            };
            Self::blend_layer(&mut out, target, &src, ox, oy, drag_opacity, drag_blend, None, None, None);
        }
        doc.apply_display_color(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use editcore::Layer;

    fn solid(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, image::Rgba(color))
    }

    #[test]
    fn fit_never_upscales_small_images() {
        let size = display_size(Size::new(100, 100), Size::new(1920, 1080), Zoom::Fit);
        assert_eq!(size, Size::new(100, 100));
    }

    #[test]
    fn fit_preserves_aspect_ratio() {
        let size = display_size(Size::new(4000, 2000), Size::new(1000, 1000), Zoom::Fit);
        assert_eq!(size, Size::new(1000, 500));
    }

    #[test]
    fn crop_region_clamps_to_image() {
        let image = solid(100, 50, [255, 0, 0, 255]);
        let cropped = crop_region(&image, Rect::new(90, 40, 30, 30));
        assert_eq!(cropped.dimensions(), (10, 10));
    }

    #[test]
    fn downscale_near_transparent_hole_stays_straight_white() {
        // [white, white, white, hole]: the last output pixel covers one white
        // and one transparent pixel. Straight-alpha averaging would darken it
        // to gray (the hole's RGB=0 bleeds in); premultiplied resampling must
        // keep it white at ~50% alpha — no dark rim around erased regions.
        let mut img = solid(4, 1, [255, 255, 255, 255]);
        img.put_pixel(3, 0, image::Rgba([0, 0, 0, 0]));
        let out = resize_to(&img, Size::new(2, 1));
        let edge = out.get_pixel(1, 0).0;
        assert!(
            (127..=128).contains(&edge[3]),
            "half coverage expected, got {:?}",
            edge
        );
        assert!(
            edge[0] >= 250 && edge[1] >= 250 && edge[2] >= 250,
            "edge must stay white (premultiplied resample), got {:?}",
            edge
        );
    }

    /// 剪切→粘贴回原位 (13% 缩放下「边缘一条浅色缝」的回归)。
    ///
    /// 背景挖掉 (100,100)–(220,180)，再放一张**完全盖住这个洞**的图层。
    /// 合成结果必须处处不透明：洞的分数 alpha 只要露出哪怕一行，透出来的
    /// 就是棋盘格，宏观看上去就是粘贴内容「没贴严」。
    ///
    /// 这条同时钉住 `preview_dst` 的取整策略：120×80 的图层在 0.13 倍下
    /// 真实占位是 y 13.0..23.4，按「尺寸四舍五入」只盖 10 行（到 23 行
    /// 就漏了 0.4 行 alpha），向外取整才盖满 11 行。
    #[test]
    fn pasted_layer_seals_erased_hole_at_fractional_zoom() {
        let mut doc = Document::from_image(solid(600, 400, [70, 120, 200, 255]));
        for y in 100..180 {
            for x in 100..220 {
                doc.base_image_mut().put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            }
        }
        let mut pasted = Layer::new("pasted", solid(120, 80, [70, 120, 200, 255]));
        pasted.set_offset((100, 100));
        doc.add_layer(pasted).unwrap();

        let target = display_size(doc.canvas, Size::new(1100, 700), Zoom::Factor(0.13));
        assert_eq!(target, Size::new(78, 52));
        let out = PreviewComposer::new().composite(&doc, target);
        let semi: Vec<(u32, u32, u8)> = out
            .enumerate_pixels()
            .filter(|(_, _, p)| p.0[3] != 255)
            .map(|(x, y, p)| (x, y, p.0[3]))
            .collect();
        assert!(
            semi.is_empty(),
            "预览出现半透明像素（会透出棋盘格 = 那条「缝」）：{:?}",
            &semi[..semi.len().min(8)]
        );
    }

    #[test]
    /// 降采样的输出像素必须与源图严格对齐：洞从 x=100 开始、缩到 0.37 倍
    /// 时真实边界在 37.0，所以输出第 36 列必须**全不透明**。旧实现用
    /// `imageops::thumbnail`，它在整数边界上多吃一个源像素，给出 alpha=170，
    /// 于是洞的边缘向相邻像素渗出 2/3 的半透明 —— 透出棋盘格就是那条缝。
    #[test]
    fn downscale_keeps_edge_on_exact_pixel() {
        let make = |w: u32| {
            let mut img = solid(w, 4, [255, 255, 255, 255]);
            for y in 0..4 {
                for x in 100..w {
                    img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
                }
            }
            img
        };
        // 600 -> 222（0.37 倍）：边界 37.0
        let out = resize_to(&make(600), Size::new(222, 2));
        assert_eq!(out.get_pixel(36, 1).0[3], 255, "洞左边一列必须全不透明");
        assert_eq!(out.get_pixel(37, 1).0[3], 0, "洞内部必须全透明");
        // 600 -> 450（0.75 倍）：边界 75.0
        let out = resize_to(&make(600), Size::new(450, 3));
        assert_eq!(out.get_pixel(74, 1).0[3], 255, "洞左边一列必须全不透明");
        assert_eq!(out.get_pixel(75, 1).0[3], 0, "洞内部必须全透明");
    }

    #[test]
    fn fully_opaque_image_skips_premultiply_roundtrip() {
        // Opaque bitmaps must produce byte-identical results to the plain
        // area filter (no rounding drift from premultiply/unpremultiply).
        let img = solid(64, 48, [30, 144, 255, 255]);
        let a = resize_to(&img, Size::new(16, 12));
        let b = downscale_box(&img, 16, 12);
        assert_eq!(a.as_raw(), b.as_raw());
    }

    #[test]
    fn document_render_matches_canvas_ratio() {
        let mut doc = Document::from_image(solid(2000, 1000, [10, 20, 30, 255]));
        doc.add_layer(Layer::new("overlay", solid(100, 100, [0, 255, 0, 255])))
            .unwrap();
        let rendered = render_for_viewport(&doc, Viewport::new(500, 500, Zoom::Fit));
        assert_eq!(rendered.dimensions(), (500, 250));
    }

    /// 剪贴蒙版必须在「预览合成」里生效。
    ///
    /// 回归对象：`PreviewComposer` 曾经只做逐层 alpha-over，完全不看
    /// `Layer::clipping`，于是被剪贴的纹理层（PSD 里那些 151×268 的斜条纹
    /// 智能对象）在窗口里整片铺到底，PS 合并图里却只出现在圆角按钮内 ——
    /// 屏幕上就是一条刺眼的斜条纹带，而 `dump-flat`（走 flatten）看不出来。
    #[test]
    fn preview_respects_clipping_mask() {
        let mut doc = Document::from_image(solid(40, 40, [255, 255, 255, 255]));
        let mut base = Layer::new("base", solid(10, 10, [255, 0, 0, 255]));
        base.set_offset((5, 5));
        doc.add_layer(base).unwrap();
        let mut clipped = Layer::new("clipped", solid(40, 40, [0, 0, 255, 255]));
        clipped.clipping = true;
        doc.add_layer(clipped).unwrap();

        // 0.5 倍预览：基线红块占 (2,2)–(7,7)。
        let out = PreviewComposer::new().composite(&doc, Size::new(20, 20));
        assert_eq!(
            out.get_pixel(0, 0).0,
            [255, 255, 255, 255],
            "剪贴层漏到基线框外"
        );
        assert_eq!(
            out.get_pixel(19, 19).0,
            [255, 255, 255, 255],
            "剪贴层漏到基线框外"
        );
        let inside = out.get_pixel(5, 5).0;
        assert!(
            inside[2] > 200 && inside[0] < 60,
            "框内应是剪贴层的蓝色，得到 {:?}",
            inside
        );

        // 1:1 时预览必须与 flatten（保存/保真口径）逐像素一致。
        let preview = PreviewComposer::new().composite(&doc, doc.canvas);
        let flat = doc.flatten_in_doc_space();
        assert_eq!(preview.as_raw(), flat.as_raw(), "预览与 flatten 的剪贴结果不一致");
    }

    /// 图层蒙版必须在「预览合成」里生效，且 1:1 时与 flatten 逐字节一致。
    ///
    /// 回归对象：`PreviewComposer` 曾经完全不看 `Layer::mask`，带蒙版的层在
    /// 预览里是「蒙版透明处仍然整块可见」；而保存 / 保真走
    /// `flatten_in_doc_space`，于是窗口与导出图对不上。
    ///
    /// 蒙版取 0 / 64 / 128 / 192 / 255 五档分数值：只要预览的覆盖率换算写错
    /// 一个字节（例如把 `coverage()` 的 f32 直接截断成 0/1），逐字节断言就会
    /// 失败。
    #[test]
    fn preview_matches_flatten_with_layer_mask_at_actual_size() {
        use image::{GrayImage, Luma};
        let mut doc = Document::from_image(solid(16, 8, [250, 250, 250, 255]));
        let mut layer = Layer::new("masked", solid(16, 8, [200, 30, 40, 255]));
        let mut mask = GrayImage::new(16, 8);
        for y in 0..8u32 {
            for x in 0..16u32 {
                mask.put_pixel(
                    x,
                    y,
                    Luma([match x % 5 {
                        0 => 0,
                        1 => 64,
                        2 => 128,
                        3 => 192,
                        _ => 255,
                    }]),
                );
            }
        }
        layer.mask = Some(LayerMask::new(mask, (0, 0)));
        doc.add_layer(layer).unwrap();

        let preview = PreviewComposer::new().composite(&doc, doc.canvas);
        let flat = doc.flatten_in_doc_space();
        assert_eq!(
            preview.as_raw(),
            flat.as_raw(),
            "1:1 预览与 flatten_in_doc_space 不一致（蒙版）"
        );

        // 钉住「蒙版确实乘进了 alpha」：0 处完全露出底图，255 处完全是本层。
        assert_eq!(preview.get_pixel(0, 0).0, [250, 250, 250, 255]);
        assert_eq!(preview.get_pixel(4, 0).0, [200, 30, 40, 255]);
        // 128 处应是严格按 alpha=128/255 的正片叠底结果。
        let half = preview.get_pixel(2, 0).0;
        assert_eq!(half, [225, 140, 145, 255], "蒙版半遮的混合结果不对");
    }

    /// 蒙版覆盖率按**预览像素覆盖的画布区间**取平均，而不是点采样。
    ///
    /// 3×1 的层在 2×1 预览里，第一个预览像素覆盖画布 x 0..1.5：蒙版是
    /// [0, 255, 255]，正确结果是一半覆盖（128/255），不是 0 或 255。
    #[test]
    fn preview_mask_coverage_follows_preview_scale() {
        use image::{GrayImage, Luma};
        let mut doc = Document::from_image(solid(3, 1, [255, 255, 255, 255]));
        let mut layer = Layer::new("masked", solid(3, 1, [0, 0, 0, 255]));
        let mut mask = GrayImage::new(3, 1);
        mask.put_pixel(0, 0, Luma([0]));
        mask.put_pixel(1, 0, Luma([255]));
        mask.put_pixel(2, 0, Luma([255]));
        layer.mask = Some(LayerMask::new(mask, (0, 0)));
        doc.add_layer(layer).unwrap();

        let out = PreviewComposer::new().composite(&doc, Size::new(2, 1));
        assert_eq!(out.get_pixel(1, 0).0, [0, 0, 0, 255], "右格应被完全遮住");
        let edge = out.get_pixel(0, 0).0;
        assert_eq!(edge[3], 255);
        assert_eq!(edge[0], 127, "半覆盖的预览像素应混成中灰，得到 {:?}", edge);
    }

    /// 蒙版与剪贴同时存在时：先乘蒙版、再乘剪贴覆盖率。
    ///
    /// 基底红块自己带蒙版（右半遮住），上面压一层剪贴蓝层。正确结果：
    /// - 基底可见处 → 蓝（剪贴层）；
    /// - 基底被蒙版遮住处 → 白底（剪贴层跟着消失，而不是漏出蓝）；
    /// - 基底矩形外 → 白底。
    /// 若顺序反过来（先剪贴再蒙版），右侧会错误地留下蓝色。
    #[test]
    fn preview_applies_mask_before_clip() {
        use image::{GrayImage, Luma};
        let mut doc = Document::from_image(solid(8, 8, [255, 255, 255, 255]));
        let mut base = Layer::new("base", solid(6, 6, [255, 0, 0, 255]));
        base.set_offset((1, 1));
        let mut mask = GrayImage::new(6, 6);
        for y in 0..6u32 {
            for x in 0..6u32 {
                mask.put_pixel(x, y, Luma([if x < 3 { 255 } else { 0 }]));
            }
        }
        base.mask = Some(LayerMask::new(mask, (1, 1)));
        doc.add_layer(base).unwrap();
        let mut clipped = Layer::new("clipped", solid(8, 8, [0, 0, 255, 255]));
        clipped.clipping = true;
        doc.add_layer(clipped).unwrap();

        let out = PreviewComposer::new().composite(&doc, doc.canvas);
        assert_eq!(
            out.get_pixel(2, 2).0,
            [0, 0, 255, 255],
            "基底可见处应显示剪贴层"
        );
        assert_eq!(
            out.get_pixel(5, 2).0,
            [255, 255, 255, 255],
            "基底被蒙版遮住，剪贴层必须一起消失"
        );
        assert_eq!(
            out.get_pixel(0, 0).0,
            [255, 255, 255, 255],
            "基底矩形外应露出白底"
        );
        assert_eq!(
            out.as_raw(),
            doc.flatten_in_doc_space().as_raw(),
            "1:1 预览与 flatten 的蒙版 + 剪贴叠加结果不一致"
        );
    }

    /// 剪贴基底带烘焙样式时，预览的边界必须取基底的「内容」形状。
    ///
    /// 回归对象：`矩形 5` 有 8px Outside 描边，烘焙后位图 alpha 比填充大一圈；
    /// 剪贴的斜纹层按位图 alpha 限幅，于是花纹越过圆角、压到粉色描边上
    /// （PS 里描边是硬边界）。这里钉住两件事：1:1 预览与 flatten 逐字节一致，
    /// 且放大的描边带里一点剪贴内容都不能有。
    #[test]
    fn preview_clip_base_uses_content_shape_not_baked_effects() {
        use image::{GrayImage, Luma};
        let mut doc = Document::from_image(solid(20, 20, [255, 255, 255, 255]));
        // 基底：12×12 全不透明（= 内容 + 烘焙描边），内容形状只有中间 6×6。
        let mut base = Layer::new("base", solid(12, 12, [255, 0, 0, 255]));
        base.set_offset((4, 4));
        let mut shape = GrayImage::new(12, 12);
        for y in 3..9u32 {
            for x in 3..9u32 {
                shape.put_pixel(x, y, Luma([255]));
            }
        }
        base.clip_shape = Some(LayerMask::new(shape, (4, 4)));
        doc.add_layer(base).unwrap();
        let mut clipped = Layer::new("clipped", solid(20, 20, [0, 0, 255, 255]));
        clipped.clipping = true;
        doc.add_layer(clipped).unwrap();

        // 描边带（内容形状之外、烘焙位图之内）必须是基底的红色，不能是蓝色。
        let preview = PreviewComposer::new().composite(&doc, doc.canvas);
        assert_eq!(
            preview.get_pixel(4, 4).0,
            [255, 0, 0, 255],
            "描边带里漏进了剪贴内容"
        );
        assert_eq!(preview.get_pixel(7, 7).0, [0, 0, 255, 255], "内容形状内应是剪贴层");
        assert_eq!(
            preview.as_raw(),
            doc.flatten_in_doc_space().as_raw(),
            "1:1 预览与 flatten 的剪贴基线不一致（内容形状）"
        );

        // 缩略预览同样只能落在内容形状里。
        let small = PreviewComposer::new().composite(&doc, Size::new(10, 10));
        assert_eq!(small.get_pixel(2, 2).0, [255, 0, 0, 255], "缩略预览里剪贴内容溢到描边");
    }

    /// 投影画在内容之外的部分要按样式自己的混合模式（PSD `Md  `）与下层混合。
    ///
    /// 回归对象：标题栏那条奶油色投影 `(255,231,200)`，PSD 里是 `Multiply`，
    /// 压在黄色面板上应得 `(255,208,~100)`；把它当 Normal 盖上去会变成一条
    /// 不透明奶油带。这里钉住预览与 flatten 两条路径都对。
    #[test]
    fn preview_blends_outside_style_pixels_with_the_effect_mode() {
        let mut doc = Document::from_image(solid(8, 8, [255, 230, 130, 255]));
        let mut bar = Layer::new("bar", solid(8, 8, [255, 231, 200, 255]));
        bar.style = Some(editcore::StylePixels {
            blend: editcore::BlendMode::Multiply,
            mask: vec![255; 64],
        });
        doc.add_layer(bar).unwrap();

        let preview = PreviewComposer::new().composite(&doc, doc.canvas);
        let px = preview.get_pixel(3, 3).0;
        assert!(
            (px[1] as i32 - 208).abs() <= 2 && (px[2] as i32 - 100).abs() <= 3,
            "样式像素没有按 Multiply 混合下层: {px:?}"
        );
        assert_eq!(
            preview.as_raw(),
            doc.flatten_in_doc_space().as_raw(),
            "1:1 预览与 flatten 的样式混合结果不一致"
        );

        // 掩码为 0 的像素仍走图层混合模式（Normal）：保持投影色。
        let mut plain = Layer::new("plain", solid(8, 8, [255, 231, 200, 255]));
        plain.style = None;
        let mut doc2 = Document::from_image(solid(8, 8, [255, 230, 130, 255]));
        doc2.add_layer(plain).unwrap();
        let out2 = PreviewComposer::new().composite(&doc2, doc2.canvas);
        assert_eq!(out2.get_pixel(3, 3).0, [255, 231, 200, 255]);
    }

    #[test]
    fn preview_matches_flatten_for_dissolve() {
        let mut doc = Document::from_image(solid(16, 16, [0, 0, 0, 255]));
        let mut layer = Layer::new("diss", solid(16, 16, [255, 0, 0, 128]));
        layer.blend = editcore::BlendMode::Dissolve;
        doc.add_layer(layer).unwrap();

        let flat = doc.flatten_in_doc_space();
        let preview = PreviewComposer::new().composite(&doc, doc.canvas);
        assert_eq!(
            preview.as_raw(),
            flat.as_raw(),
            "1:1 预览的溶解抖动图案与 flatten 不一致"
        );

        let kept = flat.pixels().filter(|p| p.0[0] > 200 && p.0[3] == 255).count();
        let partial = flat.pixels().filter(|p| p.0[3] == 128).count();
        assert_eq!(partial, 0, "溶解不应留下半透明像素");
        assert!((90..=166).contains(&kept), "保留像素数异常: {kept}");
    }

    #[test]
    fn preview_matches_base_when_no_layers_are_visible() {
        let mut doc = Document::from_image(solid(100, 50, [10, 20, 30, 255]));
        let mut hidden = Layer::new("hidden", solid(100, 50, [200, 0, 0, 255]));
        hidden.visible = false;
        doc.add_layer(hidden).unwrap();
        let preview = composite_for_preview(&doc, Size::new(40, 20));
        assert_eq!(preview.dimensions(), (40, 20));
        assert_eq!(preview.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn preview_fast_path_honors_the_crop_view() {
        let mut doc = Document::from_image(solid(100, 50, [10, 20, 30, 255]));
        let mut hidden = Layer::new("hidden", solid(100, 50, [200, 0, 0, 255]));
        hidden.visible = false;
        doc.add_layer(hidden).unwrap();

        doc.crop(Rect::new(40, 10, 30, 20)).unwrap();
        let preview = composite_for_preview(&doc, Size::new(30, 20));
        assert_eq!(preview.dimensions(), (30, 20));

        // 还原裁剪后回到全尺寸底图。
        doc.reset_crop().unwrap();
        let restored = composite_for_preview(&doc, Size::new(100, 50));
        assert_eq!(restored.dimensions(), (100, 50));
        assert_eq!(restored.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn preview_includes_visible_layers() {
        let mut doc = Document::from_image(solid(40, 40, [10, 20, 30, 255]));
        let mut layer = Layer::new("dot", solid(10, 10, [255, 255, 255, 255]));
        layer.set_offset((0, 0));
        doc.add_layer(layer).unwrap();
        let preview = composite_for_preview(&doc, Size::new(40, 40));
        assert_eq!(preview.get_pixel(2, 2).0, [255, 255, 255, 255]);
        assert_eq!(preview.get_pixel(20, 20).0, [10, 20, 30, 255]);
    }

    #[test]
    fn composer_cache_follows_live_drag_and_epoch_invalidation() {
        let mut doc = Document::from_image(solid(40, 40, [10, 20, 30, 255]));
        let mut layer = Layer::new("dot", solid(10, 10, [255, 0, 0, 255]));
        layer.set_offset((0, 0));
        doc.add_layer(layer).unwrap();
        let target = Size::new(40, 40);

        let mut composer = PreviewComposer::new();
        let first = composer.composite(&doc, target);
        assert_eq!(first.get_pixel(2, 2).0, [255, 0, 0, 255]);

        // Live drag tick: offset changes WITHOUT an epoch bump (history-free
        // mutation) — the cached scaled bitmap must still land at the new
        // offset.
        doc.layers[1].set_offset((20, 20));
        let second = composer.composite(&doc, target);
        assert_eq!(second.get_pixel(22, 22).0, [255, 0, 0, 255]);
        assert_eq!(second.get_pixel(2, 2).0, [10, 20, 30, 255]);

        // Committed edit bumps the epoch: cache is rebuilt and reflects the
        // new content.
        doc.set_layer_opacity(1, 0.5).unwrap();
        let third = composer.composite(&doc, target);
        let px = third.get_pixel(22, 22).0;
        // 0.5 alpha red over the base color: 255*0.5+10*0.5=132.5→133,
        // 20*0.5=10, 30*0.5=15.
        assert_eq!(px[0], 133);
        assert_eq!(px[1], 10);
        assert_eq!(px[2], 15);
    }

    #[test]
    fn live_scale_preview_stretches_snapshot_without_touching_doc() {        let mut doc = Document::from_image(solid(40, 40, [10, 20, 30, 255]));
        let mut layer = Layer::new("dot", solid(10, 10, [255, 0, 0, 255]));
        layer.set_offset((10, 10));
        doc.add_layer(layer).unwrap();
        let target = Size::new(40, 40);
        let snapshot = doc.layers[1].img().clone();

        let mut composer = PreviewComposer::new();
        // Warm the caches with a normal composite, then request a live
        // resize of the layer to 20x20 at offset (0, 0).
        let before = composer.composite(&doc, target);
        let epoch_before = doc.edit_epoch;
        let live = composer.composite_live_scale(
            &doc,
            target,
            1,
            &snapshot,
            Size::new(20, 20),
            (0, 0),
        );

        // The preview shows the snapshot stretched into the requested rect.
        assert_eq!(live.get_pixel(2, 2).0, [255, 0, 0, 255]);
        assert_eq!(live.get_pixel(18, 18).0, [255, 0, 0, 255]);
        // Outside the live rect: the untouched base.
        assert_eq!(live.get_pixel(30, 30).0, [10, 20, 30, 255]);

        // The document itself is never modified during the gesture.
        assert_eq!(doc.layers[1].img().dimensions(), (10, 10));
        assert_eq!(doc.layers[1].offset(), (10, 10));
        assert_eq!(doc.edit_epoch, epoch_before);

        // Cancelling (back to a plain composite) restores the committed
        // layout — the live slot must not poison the layer cache.
        let restored = composer.composite(&doc, target);
        assert_eq!(restored.get_pixel(15, 15).0, [255, 0, 0, 255]);
        assert_eq!(restored.get_pixel(2, 2).0, [10, 20, 30, 255]);
        // `before` and `restored` agree pixel for pixel.
        assert_eq!(before.as_raw(), restored.as_raw());
    }

    #[test]
    fn adjust_preview_is_identity_neutral_and_never_touches_the_doc() {
        let doc = Document::from_image(solid(40, 30, [90, 120, 200, 255]));
        let target = Size::new(20, 15);
        let mut composer = PreviewComposer::new();
        let plain = composer.composite(&doc, target);
        let neutral = composer.composite_with_adjust(
            &doc,
            target,
            AdjustTarget::Base,
            &Adjust::identity(),
        );
        assert_eq!(plain.as_raw(), neutral.as_raw());
        assert_eq!(doc.base_image().get_pixel(5, 5).0, [90, 120, 200, 255]);
    }

    #[test]
    fn adjust_preview_scopes_to_base_or_layer() {
        let mut doc = Document::from_image(solid(40, 30, [100, 100, 100, 255]));
        let mut layer = Layer::new("dot", solid(10, 10, [150, 150, 150, 255]));
        layer.set_offset((0, 0));
        doc.add_layer(layer).unwrap();
        let target = Size::new(40, 30);
        let mut composer = PreviewComposer::new();

        let brighter = Adjust {
            exposure: 0.8,
            ..Adjust::identity()
        };
        // Base scope: the far corner brightens, the layer stays as it was.
        let base_adjusted =
            composer.composite_with_adjust(&doc, target, AdjustTarget::Base, &brighter);
        assert!(base_adjusted.get_pixel(30, 20).0[0] > 100);
        assert_eq!(base_adjusted.get_pixel(2, 2).0, [150, 150, 150, 255]);

        // Layer scope: only the layer's rect changes. `layers[0]` is the
        // background, so the user layer lives at index 1.
        let layer_adjusted =
            composer.composite_with_adjust(&doc, target, AdjustTarget::Layer(1), &brighter);
        assert!(layer_adjusted.get_pixel(2, 2).0[0] > 150);
        assert_eq!(layer_adjusted.get_pixel(30, 20).0, [100, 100, 100, 255]);
    }

    #[test]
    fn adjust_preview_leaves_markup_annotations_untouched() {
        let mut doc = Document::from_image(solid(40, 30, [100, 100, 100, 255]));
        let mut mark = editcore::Mark::new(editcore::MarkKind::Rect);
        mark.rect = (2.0, 2.0, 20.0, 20.0);
        mark.stroke = 6;
        mark.color = [255, 0, 0, 255];
        doc.add_mark(mark).unwrap();
        let target = Size::new(40, 30);
        let mut composer = PreviewComposer::new();
        let out = composer.composite_with_adjust(
            &doc,
            target,
            AdjustTarget::Base,
            &Adjust {
                saturation: -1.0,
                ..Adjust::identity()
            },
        );
        // The red stroke stays red: marks are annotations, not photo pixels.
        assert_eq!(out.get_pixel(2, 10).0, [255, 0, 0, 255]);
        // The grey base is desaturated but unchanged (already neutral grey).
        assert_eq!(out.get_pixel(30, 25).0, [100, 100, 100, 255]);
    }
}
