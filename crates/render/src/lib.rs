use std::sync::Arc;

use image::imageops::FilterType;
use image::RgbaImage;

use editcore::{Adjust, AdjustTarget, Document, Rect, Size};

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

pub fn resize_to(image: &RgbaImage, target: Size) -> RgbaImage {
    if image.width() == target.width && image.height() == target.height {
        return image.clone();
    }
    image::imageops::resize(image, target.width, target.height, FilterType::Triangle)
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

    /// Cached preview-scaled visible base (or a transparent buffer).
    fn base_buffer(&mut self, doc: &Document, target: Size, sx: f64, sy: f64) -> RgbaImage {
        if doc.base_hidden {
            return RgbaImage::new(target.width, target.height);
        }
        let nw = ((doc.view.width as f64 * sx).round() as u32).max(1);
        let nh = ((doc.view.height as f64 * sy).round() as u32).max(1);
        if let Some(img) = Self::cached_hit(&self.base, nw, nh) {
            return (*img).clone();
        }
        let img = resize_to(&crop_region(&doc.base, doc.view), target);
        self.base = Some((nw, nh, Arc::new(img.clone())));
        img
    }

    /// Cached preview-scaled copy of a layer bitmap, stored in slot `li`.
    fn scaled_layer(&mut self, li: usize, img: &RgbaImage, sx: f64, sy: f64) -> Arc<RgbaImage> {
        let nw = ((img.width() as f64 * sx).round() as u32).max(1);
        let nh = ((img.height() as f64 * sy).round() as u32).max(1);
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

    fn draw_marks(doc: &Document, out: &mut RgbaImage, sx: f64, sy: f64) {
        for mark in &doc.marks {
            if mark.visible {
                mark.draw_scaled(out, sx as f32, sy as f32);
            }
        }
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
        for (li, layer) in doc.layers.iter().enumerate() {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            let src = self.scaled_layer(li, &layer.image, sx, sy);
            let ox = (layer.offset.0 as f64 * sx).round() as i32;
            let oy = (layer.offset.1 as f64 * sy).round() as i32;
            match adjust {
                Some((AdjustTarget::Layer(index), params)) if index == li => {
                    let mut adjusted = (*src).clone();
                    params.apply_in_place(&mut adjusted);
                    Self::blend_layer(&mut out, target, &adjusted, ox, oy, layer.opacity);
                }
                _ => Self::blend_layer(&mut out, target, &src, ox, oy, layer.opacity),
            }
        }
        Self::draw_marks(doc, &mut out, sx, sy);
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
        )
    }

    /// [`PreviewComposer::composite_live_scale`] plus an optional live color
    /// adjustment (the "调整颜色" panel can stay open while a layer handle is
    /// dragged).
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
        for (li, layer) in doc.layers.iter().enumerate() {
            if !layer.visible || layer.opacity <= 0.0 || li == layer_index {
                continue;
            }
            let src = self.scaled_layer(li, &layer.image, sx, sy);
            let ox = (layer.offset.0 as f64 * sx).round() as i32;
            let oy = (layer.offset.1 as f64 * sy).round() as i32;
            match adjust {
                Some((AdjustTarget::Layer(index), params)) if index == li => {
                    let mut adjusted = (*src).clone();
                    params.apply_in_place(&mut adjusted);
                    Self::blend_layer(&mut out, target, &adjusted, ox, oy, layer.opacity);
                }
                _ => Self::blend_layer(&mut out, target, &src, ox, oy, layer.opacity),
            }
        }
        if drag_opacity > 0.0 {
            let nw = ((to.width as f64 * sx).round() as u32).max(1);
            let nh = ((to.height as f64 * sy).round() as u32).max(1);
            let src = match Self::cached_hit(&self.live, nw, nh) {
                Some(img) => img,
                None => {
                    let img = Arc::new(image::imageops::resize(
                        snapshot,
                        nw,
                        nh,
                        FilterType::Nearest,
                    ));
                    self.live = Some((nw, nh, img.clone()));
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
        Self::draw_marks(doc, &mut out, sx, sy);
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
        layer.offset = (0, 0);
        doc.add_layer(layer).unwrap();
        let preview = composite_for_preview(&doc, Size::new(40, 40));
        assert_eq!(preview.get_pixel(2, 2).0, [255, 255, 255, 255]);
        assert_eq!(preview.get_pixel(20, 20).0, [10, 20, 30, 255]);
    }

    #[test]
    fn composer_cache_follows_live_drag_and_epoch_invalidation() {
        let mut doc = Document::from_image(solid(40, 40, [10, 20, 30, 255]));
        let mut layer = Layer::new("dot", solid(10, 10, [255, 0, 0, 255]));
        layer.offset = (0, 0);
        doc.add_layer(layer).unwrap();
        let target = Size::new(40, 40);

        let mut composer = PreviewComposer::new();
        let first = composer.composite(&doc, target);
        assert_eq!(first.get_pixel(2, 2).0, [255, 0, 0, 255]);

        // Live drag tick: offset changes WITHOUT an epoch bump (history-free
        // mutation) — the cached scaled bitmap must still land at the new
        // offset.
        doc.layers[0].offset = (20, 20);
        let second = composer.composite(&doc, target);
        assert_eq!(second.get_pixel(22, 22).0, [255, 0, 0, 255]);
        assert_eq!(second.get_pixel(2, 2).0, [10, 20, 30, 255]);

        // Committed edit bumps the epoch: cache is rebuilt and reflects the
        // new content.
        doc.set_layer_opacity(0, 0.5).unwrap();
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
        layer.offset = (10, 10);
        doc.add_layer(layer).unwrap();
        let target = Size::new(40, 40);
        let snapshot = doc.layers[0].image.clone();

        let mut composer = PreviewComposer::new();
        // Warm the caches with a normal composite, then request a live
        // resize of the layer to 20x20 at offset (0, 0).
        let before = composer.composite(&doc, target);
        let epoch_before = doc.edit_epoch;
        let live = composer.composite_live_scale(
            &doc,
            target,
            0,
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
        assert_eq!(doc.layers[0].image.dimensions(), (10, 10));
        assert_eq!(doc.layers[0].offset, (10, 10));
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
        assert_eq!(doc.base.get_pixel(5, 5).0, [90, 120, 200, 255]);
    }

    #[test]
    fn adjust_preview_scopes_to_base_or_layer() {
        let mut doc = Document::from_image(solid(40, 30, [100, 100, 100, 255]));
        let mut layer = Layer::new("dot", solid(10, 10, [150, 150, 150, 255]));
        layer.offset = (0, 0);
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

        // Layer scope: only the layer's rect changes.
        let layer_adjusted =
            composer.composite_with_adjust(&doc, target, AdjustTarget::Layer(0), &brighter);
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
