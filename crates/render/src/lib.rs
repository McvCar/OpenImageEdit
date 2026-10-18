use std::sync::Arc;

use image::imageops::FilterType;
use image::RgbaImage;

use editcore::{Document, Rect, Size};

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
}

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

    pub fn composite(&mut self, doc: &Document, target: Size) -> RgbaImage {
        if target.width == 0 || target.height == 0 || doc.canvas.width == 0 || doc.canvas.height == 0 {
            return RgbaImage::new(target.width.max(1), target.height.max(1));
        }
        let key = (doc.edit_epoch, target.width, target.height);
        if (self.epoch, self.target.0, self.target.1) != key {
            self.epoch = doc.edit_epoch;
            self.target = (target.width, target.height);
            self.base = None;
            self.layers = vec![None; doc.layers.len()];
        }
        let sx = target.width as f64 / doc.canvas.width as f64;
        let sy = target.height as f64 / doc.canvas.height as f64;

        let mut out = if doc.base_hidden {
            RgbaImage::new(target.width, target.height)
        } else {
            // The base contributes doc.view (crop) scaled to the target.
            let nw = ((doc.view.width as f64 * sx).round() as u32).max(1);
            let nh = ((doc.view.height as f64 * sy).round() as u32).max(1);
            match Self::cached_hit(&self.base, nw, nh) {
                Some(img) => (*img).clone(),
                None => {
                    let img = resize_to(&crop_region(&doc.base, doc.view), target);
                    self.base = Some((nw, nh, Arc::new(img.clone())));
                    img
                }
            }
        };

        for (li, layer) in doc.layers.iter().enumerate() {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            let nw = ((layer.image.width() as f64 * sx).round() as u32).max(1);
            let nh = ((layer.image.height() as f64 * sy).round() as u32).max(1);
            let slot = self.layers.get(li).cloned().flatten();
            let src: Arc<RgbaImage> = match Self::cached_hit(&slot, nw, nh) {
                Some(img) => img,
                None => {
                    let img = Arc::new(resize_to(&layer.image, Size::new(nw, nh)));
                    if li < self.layers.len() {
                        self.layers[li] = Some((nw, nh, img.clone()));
                    }
                    img
                }
            };
            let ox = (layer.offset.0 as f64 * sx).round() as i32;
            let oy = (layer.offset.1 as f64 * sy).round() as i32;

            // Row-slice fast path: whole rows are handed out as byte slices so
            // the per-tick cost stays low even in debug builds.
            let x0 = ox.max(0) as u32;
            let x1 = (ox + nw as i32).min(target.width as i32).max(0) as u32;
            if x0 >= x1 {
                continue;
            }
            let src_stride = nw as usize * 4;
            let src_buf: &[u8] = src.as_raw();
            let out_stride = target.width as usize * 4;
            let mut out_buf = out.into_raw();
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
                    let sa = srow[so + 3] as f32 / 255.0 * layer.opacity;
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
            out = RgbaImage::from_raw(target.width, target.height, out_buf)
                .expect("preview buffer size preserved");
        }

        // Markup annotations ride on top of the preview, scaled to target px.
        for mark in &doc.marks {
            if mark.visible {
                mark.draw_scaled(&mut out, sx as f32, sy as f32);
            }
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
}
