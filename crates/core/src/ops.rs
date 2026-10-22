// crates/core/src/ops.rs
use super::*;

pub(crate) fn apply_view(doc: &mut Document, view: Rect) {
    // Layer offsets and marks are CANVAS-relative. Cropping moves the canvas
    // origin, so their positions must ride along with the content: the whole
    // composition shifts by (old origin - new origin) inside the new canvas.
    // Without this, cropping after adding layers / marks made them jump back
    // to their pre-crop canvas position while the base content moved.
    let dx = doc.view.x - view.x;
    let dy = doc.view.y - view.y;
    doc.view = view;
    doc.canvas = Size::new(view.width, view.height);
    if dx != 0 || dy != 0 {
        for layer in &mut doc.layers {
            layer.offset.0 += dx;
            layer.offset.1 += dy;
        }
        for mark in &mut doc.marks {
            *mark = mark.translated(dx as f32, dy as f32);
        }
    }
}

/// Make every base pixel under `selection` fully transparent. Selection
/// coordinates are canvas coordinates and map into `base` through `view`.
/// "Delete" the base: replace it with a fully transparent canvas-sized image.
pub(crate) fn apply_delete_base(doc: &mut Document) {
    doc.base = RgbaImage::new(doc.canvas.width, doc.canvas.height);
    doc.base_deleted = true;
}

/// "Unlock" the base: its pixels become a regular layer at the bottom of the
/// stack (offset by `view` so the visible region matches exactly) and the
/// base becomes a transparent image of the same size. Canvas size, `view`
/// and therefore crop / rotate / canvas-resize behavior stay unchanged; the
/// composite output is pixel-identical before and after.
pub(crate) fn apply_base_to_layer(doc: &mut Document) {
    if doc.base_converted {
        return;
    }
    let mut layer = Layer::new("背景", doc.base.clone());
    // Layer offsets are canvas-relative: the converted base must show
    // base[view.x + x] at canvas x, i.e. its top-left sits at
    // (-view.x, -view.y). (view.x, view.y) was only correct for an
    // uncropped document and silently shifted the composition otherwise.
    layer.offset = (-doc.view.x, -doc.view.y);
    let empty = RgbaImage::new(doc.base.width(), doc.base.height());
    doc.base = empty;
    doc.layers.insert(0, layer);
    doc.base_deleted = false;
    doc.base_hidden = false;
    doc.base_converted = true;
}

pub(crate) fn erase_base_pixels(doc: &mut Document, selection: &Selection) {    let canvas_rect = Rect::new(0, 0, doc.canvas.width, doc.canvas.height);
    let Some(clipped) = selection.bounds().intersect(canvas_rect) else {
        return;
    };
    for y in 0..clipped.height {
        for x in 0..clipped.width {
            let cx = clipped.x + x as i32;
            let cy = clipped.y + y as i32;
            if !selection.contains(cx as f32 + 0.5, cy as f32 + 0.5) {
                continue;
            }
            let bx = doc.view.x + cx;
            let by = doc.view.y + cy;
            if bx < 0 || by < 0 {
                continue;
            }
            let (bx, by) = (bx as u32, by as u32);
            if bx < doc.base.width() && by < doc.base.height() {
                *doc.base.get_pixel_mut(bx, by) = image::Rgba([0, 0, 0, 0]);
            }
        }
    }
}

/// Canvas-only resize (no resampling): the old base pixels are copied to the
/// center of the new canvas (center anchor, cropped when shrinking), and every
/// layer is shifted by the same delta so the whole composition stays centered.
/// Layer pixels are NEVER resampled.
pub(crate) fn apply_canvas_resize(doc: &mut Document, to: Size) {
    let mut old = doc.base.clone();
    let dx = (to.width as i64 - old.width() as i64) / 2;
    let dy = (to.height as i64 - old.height() as i64) / 2;

    // Copy the overlapping region of the old base into the new canvas.
    // Source rect (old coords) and destination rect (new coords) are the same
    // content translated by (dx, dy), clipped to both images.
    let sx0 = (-dx).max(0);
    let sy0 = (-dy).max(0);
    let tx0 = dx.max(0);
    let ty0 = dy.max(0);
    let w = (old.width() as i64 - sx0).min(to.width as i64 - tx0).max(0) as u32;
    let h = (old.height() as i64 - sy0).min(to.height as i64 - ty0).max(0) as u32;

    let mut new_base = image::RgbaImage::new(to.width, to.height);
    if w > 0 && h > 0 {
        let part = image::imageops::crop(
            &mut old,
            sx0 as u32,
            sy0 as u32,
            w,
            h,
        );
        image::imageops::overlay(
            &mut new_base,
            &part.to_image(),
            tx0 as i64,
            ty0 as i64,
        );
    }
    doc.base = new_base;
    doc.view = Rect::new(0, 0, to.width, to.height);
    doc.canvas = to;
    for layer in &mut doc.layers {
        layer.offset.0 += dx as i32;
        layer.offset.1 += dy as i32;
    }
    // Marks ride along with the same center-anchor shift.
    for mark in &mut doc.marks {
        *mark = mark.translated(dx as f32, dy as f32);
    }
}

pub(crate) fn apply_resize(doc: &mut Document, to: Size) {
    // 以当前画布（所见）为参照缩放到 `to`：把当前可见内容重采样为新底图，
    // 缩放后不再保留裁剪窗口（view 回到全尺寸）。
    let sx = to.width as f64 / doc.canvas.width as f64;
    let sy = to.height as f64 / doc.canvas.height as f64;
    let visible = doc.flatten();
    doc.base = image::imageops::resize(
        &visible,
        to.width,
        to.height,
        image::imageops::FilterType::Lanczos3,
    );
    doc.view = Rect::new(0, 0, to.width, to.height);
    doc.canvas = to;
    for layer in &mut doc.layers {
        let ow = ((layer.image.width() as f64) * sx).round() as u32;
        let oh = ((layer.image.height() as f64) * sy).round() as u32;
        layer.image = image::imageops::resize(
            &layer.image,
            ow.max(1),
            oh.max(1),
            image::imageops::FilterType::Lanczos3,
        );
        layer.offset = (
            ((layer.offset.0 as f64) * sx).round() as i32,
            ((layer.offset.1 as f64) * sy).round() as i32,
        );
    }
    // Marks scale with the document; the stroke width follows the mean axis
    // factor (clamped to >= 1px so hairlines never disappear entirely).
    let ss = ((sx + sy) / 2.0).max(1.0 / 3.0);
    for mark in &mut doc.marks {
        mark.rect = (
            (mark.rect.0 as f64 * sx) as f32,
            (mark.rect.1 as f64 * sy) as f32,
            (mark.rect.2 as f64 * sx) as f32,
            (mark.rect.3 as f64 * sy) as f32,
        );
        for p in &mut mark.points {
            *p = ((p.0 as f64 * sx) as f32, (p.1 as f64 * sy) as f32);
        }
        mark.stroke = ((mark.stroke as f64 * ss).round() as i64).clamp(1, 255) as u8;
    }
}
