use std::sync::Arc;

use image::imageops::FilterType;
use image::RgbaImage;

use editcore::{Adjust, AdjustTarget, Document, LayerContent, Rect, Size};

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
        if !aligned {
            let mut out = RgbaImage::new(target.width, target.height);
            let (ox, oy, nw, nh) =
                Self::preview_dst(r.offset, (r.image.width(), r.image.height()), sx, sy);
            let src = self.scaled_layer(0, &r.image, nw, nh);
            Self::blend_layer(&mut out, target, &src, ox, oy, bg.opacity);
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
        Self::blend_layer(&mut out, target, &img, 0, 0, bg.opacity);
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
        for (li, layer) in doc.layers.iter().enumerate().skip(1) {
            if !layer.visible || layer.opacity <= 0.0 || skip == Some(li) {
                continue;
            }
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
                    match adjust {
                        Some((AdjustTarget::Layer(index), params)) if index == li => {
                            let mut adjusted = (*src).clone();
                            params.apply_in_place(&mut adjusted);
                            Self::blend_layer(out, target, &adjusted, ox, oy, layer.opacity);
                        }
                        _ => Self::blend_layer(out, target, &src, ox, oy, layer.opacity),
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
    fn blend(
        out_buf: &mut [u8],
        target: Size,
        src: &RgbaImage,
        ox: i32,
        oy: i32,
        opacity: f32,
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
                let sa = srow[so + 3] as f32 / 255.0 * opacity;
                if sa <= 0.0 {
                    continue;
                }
                let oo = tx as usize * 4;
                let inv = 1.0 - sa;
                for c in 0..3 {
                    orow[oo + c] =
                        (srow[so + c] as f32 * sa + orow[oo + c] as f32 * inv).round() as u8;
                }
                orow[oo + 3] = ((sa + orow[oo + 3] as f32 / 255.0 * inv) * 255.0).round() as u8;
            }
        }
    }

    fn blend_layer(out: &mut RgbaImage, target: Size, src: &RgbaImage, ox: i32, oy: i32, opacity: f32) {
        let mut raw = std::mem::take(out).into_raw();
        Self::blend(&mut raw, target, src, ox, oy, opacity);
        *out = RgbaImage::from_raw(target.width, target.height, raw)
            .expect("preview buffer size preserved");
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
        let Some((sx, sy)) = self.prepare(doc, target) else {
            return RgbaImage::new(target.width.max(1), target.height.max(1));
        };
        let mut out = self.base_buffer(doc, target, sx, sy);
        if let Some((AdjustTarget::Base, params)) = adjust {
            params.apply_in_place(&mut out);
        }
        self.blend_upper_layers(doc, &mut out, target, sx, sy, None, adjust);
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
            Self::blend_layer(&mut out, target, &stretched, ox, oy, drag_opacity);
        }
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
            Self::blend_layer(&mut out, target, &stretched, 0, 0, drag_opacity);
        }
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
            Self::blend_layer(&mut out, target, &src, ox, oy, drag_opacity);
        }
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
