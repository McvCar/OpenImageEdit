use std::sync::Arc;

use image::RgbaImage;

use editcore::{Adjust, AdjustTarget, Document, Rect, Size};

use crate::scale::{crop_region, display_size, resize_to, Viewport};

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
pub(crate) type CachedScaled = (u32, u32, Arc<RgbaImage>);

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
    pub(crate) epoch: u64,
    pub(crate) target: (u32, u32),
    pub(crate) base: Option<CachedScaled>,
    pub(crate) layers: Vec<Option<CachedScaled>>,
    /// 每个有图层蒙版的层的预览尺度覆盖率（乘数），按 (ox, oy, w, h) 缓存。
    /// 蒙版编辑同样走 `edit_epoch`，`prepare()` 里统一清空。
    pub(crate) masks: Vec<Option<(i32, i32, u32, u32, Arc<Vec<u8>>)>>,
    /// 每个带烘焙样式、且可能当剪贴基底的层的「内容形状」覆盖率（预览尺度）。
    /// 与 `masks` 同一套缓存键与失效规则（`prepare()` 统一清空）。
    pub(crate) clip_shapes: Vec<Option<(i32, i32, u32, u32, Arc<Vec<u8>>)>>,
    /// 每个带「内容之外样式笔迹」的层的预览尺度掩码（见 [`editcore::StylePixels`]）。
    /// 与 `layers` 同一套缓存键（`(nw, nh)`），`prepare()` 统一清空。
    pub(crate) styles: Vec<Option<(u32, u32, Arc<Vec<u8>>)>>,
    /// Scaled bitmap of the layer being live-resized (kept separate from
    /// `layers` so a cancelled gesture cannot poison the layer's cache).
    pub(crate) live: Option<CachedScaled>,
    /// Mirror flags of the cached live bitmap. A handle dragged past the
    /// opposite edge must show the mirrored stretch, and the cache key is
    /// only (w, h) — flipping the flags has to invalidate it.
    pub(crate) live_flip: (bool, bool),
    /// 旋转手势里「缩到预览尺寸的那张快照」。整层缩放一次就要十几毫秒，
    /// 而拖动时快照和预览尺寸都不变，所以按 (会话号, w, h) 缓存，每帧只
    /// 剩下旋转本身。
    pub(crate) rotate_small: Option<(u64, u32, u32, Arc<RgbaImage>)>,
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

    pub(crate) fn cached_hit(
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
    pub(crate) fn prepare(&mut self, doc: &Document, target: Size) -> Option<(f64, f64)> {
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
}
