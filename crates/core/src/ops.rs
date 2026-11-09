// crates/core/src/ops.rs
use super::*;

pub(crate) fn apply_view(doc: &mut Document, view: Rect) {
    // 图层偏移与标记坐标都是 CANVAS 相对的。裁剪移动了画布原点，内容必须
    // 跟着走：整个合成在画布内平移 (旧原点 - 新原点)。背景层因此自动保持
    // `offset == (-view.x, -view.y)`，与其它位图图层同一套规则。
    let dx = doc.view.x - view.x;
    let dy = doc.view.y - view.y;
    doc.view = view;
    doc.canvas = Size::new(view.width, view.height);
    if dx != 0 || dy != 0 {
        for layer in &mut doc.layers {
            match &mut layer.content {
                LayerContent::Raster(r) => {
                    r.offset.0 += dx;
                    r.offset.1 += dy;
                }
                LayerContent::Mark(m) => {
                    *m = m.translated(dx as f32, dy as f32);
                }
            }
        }
    }
}

/// "Delete" the background: replace its pixels with a fully transparent
/// canvas-sized image. The layer entry itself stays (stack unchanged).
pub(crate) fn apply_delete_base(doc: &mut Document) {
    let empty = RgbaImage::new(doc.canvas.width, doc.canvas.height);
    doc.set_base_image(empty);
    doc.view = Rect::new(0, 0, doc.canvas.width, doc.canvas.height);
}

/// Punch the selection's outline out of the pixels that will remain on the
/// canvas: every pixel inside `selection.bounds` keeps `alpha * coverage`,
/// so ellipse / lasso / feathered selections leave a transparent surround
/// instead of a hard rectangular crop. Both the background and the layers are
/// touched — otherwise a layer would keep painting its out-of-selection
/// pixels over the new transparent area (looks like crop did nothing).
///
/// Only the bounding box is scanned: everything outside is dropped by the
/// crop anyway, so walking the whole image would just be slow.
pub(crate) fn clear_outside_selection(doc: &mut Document, selection: &SelectionMask) {
    let canvas_rect = Rect::new(0, 0, doc.canvas.width, doc.canvas.height);
    let Some(area) = selection.bounds.intersect(canvas_rect) else {
        return;
    };

    for layer in &mut doc.layers {
        let LayerContent::Raster(r) = &mut layer.content else {
            continue;
        };
        let lx0 = (area.x - r.offset.0).max(0) as u32;
        let ly0 = (area.y - r.offset.1).max(0) as u32;
        let lx1 = (area.right() - r.offset.0).min(r.image.width() as i32);
        let ly1 = (area.bottom() - r.offset.1).min(r.image.height() as i32);
        if lx1 <= lx0 as i32 || ly1 <= ly0 as i32 {
            continue;
        }
        for ly in ly0..ly1 as u32 {
            let cy = r.offset.1 + ly as i32;
            for lx in lx0..lx1 as u32 {
                let cx = r.offset.0 + lx as i32;
                let cov = selection.coverage_at(cx, cy);
                if cov == 255 {
                    continue;
                }
                let px = r.image.get_pixel_mut(lx, ly);
                px.0[3] = ((px.0[3] as u32 * cov as u32 + 127) / 255) as u8;
                if px.0[3] == 0 {
                    px.0 = [0, 0, 0, 0];
                }
            }
        }
    }
}

pub(crate) fn erase_base_pixels(doc: &mut Document, selection: &SelectionMask) {
    let canvas_rect = Rect::new(0, 0, doc.canvas.width, doc.canvas.height);
    let Some(clipped) = selection.bounds.intersect(canvas_rect) else {
        return;
    };
    let Some(r) = doc.layers[0].raster_mut() else {
        return;
    };
    for y in 0..clipped.height {
        for x in 0..clipped.width {
            let cx = clipped.x + x as i32;
            let cy = clipped.y + y as i32;
            let cov = selection.coverage_at(cx, cy);
            if cov == 0 {
                continue;
            }
            let bx = cx - r.offset.0;
            let by = cy - r.offset.1;
            if bx < 0 || by < 0 {
                continue;
            }
            let (bx, by) = (bx as u32, by as u32);
            if bx < r.image.width() && by < r.image.height() {
                // Partial coverage (feather) scales the pixel's alpha instead
                // of punching a hard hole.
                let px = r.image.get_pixel_mut(bx, by);
                px.0[3] = ((px.0[3] as u32 * (255 - cov as u32) + 127) / 255) as u8;
                if px.0[3] == 0 {
                    px.0 = [0, 0, 0, 0];
                }
            }
        }
    }
}

/// Canvas-only resize (no resampling): the old background pixels are copied to
/// the center of the new canvas (center anchor, cropped when shrinking), and
/// every layer is shifted by the same delta so the whole composition stays
/// centered. Layer pixels are NEVER resampled.
pub(crate) fn apply_canvas_resize(doc: &mut Document, to: Size) {
    let mut old = doc.base_image().clone();
    let dx = (to.width as i64 - old.width() as i64) / 2;
    let dy = (to.height as i64 - old.height() as i64) / 2;

    let sx0 = (-dx).max(0);
    let sy0 = (-dy).max(0);
    let tx0 = dx.max(0);
    let ty0 = dy.max(0);
    let w = (old.width() as i64 - sx0).min(to.width as i64 - tx0).max(0) as u32;
    let h = (old.height() as i64 - sy0).min(to.height as i64 - ty0).max(0) as u32;

    let mut new_base = image::RgbaImage::new(to.width, to.height);
    if w > 0 && h > 0 {
        let part = image::imageops::crop(&mut old, sx0 as u32, sy0 as u32, w, h);
        image::imageops::overlay(&mut new_base, &part.to_image(), tx0 as i64, ty0 as i64);
    }
    doc.layers[0].ensure_baked();
    if let Some(r) = doc.layers[0].raster_mut() {
        r.image = new_base;
        r.offset = (0, 0);
    }
    doc.view = Rect::new(0, 0, to.width, to.height);
    doc.canvas = to;
    // The background was re-anchored to (0, 0) above; every other layer rides
    // along with the same center-anchor shift.
    for layer in doc.layers.iter_mut().skip(1) {
        match &mut layer.content {
            LayerContent::Raster(r) => {
                r.offset.0 += dx as i32;
                r.offset.1 += dy as i32;
            }
            LayerContent::Mark(m) => {
                *m = m.translated(dx as f32, dy as f32);
            }
        }
    }
}

pub(crate) fn apply_resize(doc: &mut Document, to: Size) {
    // 以当前画布（所见）为参照缩放到 `to`：把当前可见内容重采样为新底图，
    // 缩放后不再保留裁剪窗口（view 回到全尺寸）。图层与标记跟着缩放
    // （P0 保持与统一模型之前完全一致的行为，包括「合成图 + 图层各缩放
    // 一次」这一点）。
    let sx = to.width as f64 / doc.canvas.width as f64;
    let sy = to.height as f64 / doc.canvas.height as f64;
    let visible = doc.flatten();
    let resized = image::imageops::resize(
        &visible,
        to.width,
        to.height,
        image::imageops::FilterType::Lanczos3,
    );
    doc.view = Rect::new(0, 0, to.width, to.height);
    doc.canvas = to;
    doc.layers[0].ensure_baked();
    if let Some(r) = doc.layers[0].raster_mut() {
        r.image = resized;
        r.offset = (0, 0);
    }
    for layer in doc.layers.iter_mut().skip(1) {
        match &mut layer.content {
            LayerContent::Raster(r) => {
                // 文档级 resize 直接改每层的位图：先烘焙，别让残留的原始像素
                // 在下次缩放时把缩放后的内容覆盖回去。
                r.transform = None;
                let ow = ((r.image.width() as f64) * sx).round() as u32;
                let oh = ((r.image.height() as f64) * sy).round() as u32;
                r.image = image::imageops::resize(
                    &r.image,
                    ow.max(1),
                    oh.max(1),
                    image::imageops::FilterType::Lanczos3,
                );
                r.offset = (
                    ((r.offset.0 as f64) * sx).round() as i32,
                    ((r.offset.1 as f64) * sy).round() as i32,
                );
            }
            LayerContent::Mark(m) => {
                // Marks scale with the document; the stroke width follows the
                // mean axis factor (clamped to >= 1px so hairlines survive).
                let ss = ((sx + sy) / 2.0).max(1.0 / 3.0);
                m.rect = (
                    (m.rect.0 as f64 * sx) as f32,
                    (m.rect.1 as f64 * sy) as f32,
                    (m.rect.2 as f64 * sx) as f32,
                    (m.rect.3 as f64 * sy) as f32,
                );
                for p in &mut m.points {
                    *p = ((p.0 as f64 * sx) as f32, (p.1 as f64 * sy) as f32);
                }
                m.stroke = ((m.stroke as f64 * ss).round() as i64).clamp(1, 255) as u8;
            }
        }
    }
}

/// 把一块补丁贴进目标位图的 `rect` 位置（都是位图坐标）。图片尺寸与 rect
/// 对不上时按较小的一边裁，越界部分丢弃 —— 撤销/重做用的前后两份补丁永远
/// 由同一次改动生成，理论上不会错位，这里只做防御。
pub(crate) fn apply_region(doc: &mut Document, target: AdjustTarget, rect: Rect, img: &RgbaImage) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let dst = match target {
        AdjustTarget::Base => {
            doc.layers[0].ensure_baked();
            match doc.layers[0].image_mut() {
                Some(img) => img,
                None => return,
            }
        }
        AdjustTarget::Layer(index) => match doc.layers.get_mut(index) {
            Some(layer) => {
                // 像素补丁写进 image 之前必须烘焙（D11 / §2.4）。
                layer.ensure_baked();
                match layer.image_mut() {
                    Some(img) => img,
                    None => return,
                }
            }
            None => return,
        },
    };
    let w = rect.width.min(img.width()).min(dst.width().saturating_sub(rect.x.max(0) as u32));
    let h = rect
        .height
        .min(img.height())
        .min(dst.height().saturating_sub(rect.y.max(0) as u32));
    if rect.x < 0 || rect.y < 0 || w == 0 || h == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let px = img.get_pixel(x, y);
            dst.put_pixel(rect.x as u32 + x, rect.y as u32 + y, *px);
        }
    }
}
