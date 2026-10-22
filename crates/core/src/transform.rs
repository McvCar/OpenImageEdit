// crates/core/src/transform.rs
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    Rotate90Cw,
    Rotate90Ccw,
    Rotate180,
    FlipHorizontal,
    FlipVertical,
}

impl Transform {
    pub const fn inverted(self) -> Self {
        match self {
            Transform::Rotate90Cw => Transform::Rotate90Ccw,
            Transform::Rotate90Ccw => Transform::Rotate90Cw,
            Transform::Rotate180 => Transform::Rotate180,
            Transform::FlipHorizontal => Transform::FlipHorizontal,
            Transform::FlipVertical => Transform::FlipVertical,
        }
    }

    const fn swaps_axes(self) -> bool {
        matches!(self, Transform::Rotate90Cw | Transform::Rotate90Ccw)
    }

    fn apply_to_image(self, image: &RgbaImage) -> RgbaImage {
        match self {
            Transform::Rotate90Cw => image::imageops::rotate90(image),
            Transform::Rotate90Ccw => image::imageops::rotate270(image),
            Transform::Rotate180 => image::imageops::rotate180(image),
            Transform::FlipHorizontal => image::imageops::flip_horizontal(image),
            Transform::FlipVertical => image::imageops::flip_vertical(image),
        }
    }
}

fn transform_rect(rect: Rect, transform: Transform, canvas: Size) -> Rect {
    let (cw, ch) = (canvas.width as i32, canvas.height as i32);
    let (lw, lh) = (rect.width as i32, rect.height as i32);
    let (ox, oy) = (rect.x, rect.y);
    let (x, y, width, height) = match transform {
        Transform::Rotate90Cw => (ch - (oy + lh), ox, lh, lw),
        Transform::Rotate90Ccw => (oy, cw - (ox + lw), lh, lw),
        Transform::Rotate180 => (cw - (ox + lw), ch - (oy + lh), lw, lh),
        Transform::FlipHorizontal => (cw - (ox + lw), oy, lw, lh),
        Transform::FlipVertical => (ox, ch - (oy + lh), lw, lh),
    };
    Rect::new(x, y, width.max(0) as u32, height.max(0) as u32)
}

pub(crate) fn transform_document(doc: &mut Document, transform: Transform) {
    let canvas = doc.canvas;
    for layer in &mut doc.layers {
        let bounds = layer.bounds();
        let (ox, oy) = (bounds.x, bounds.y);
        let (lw, lh) = (bounds.width as i32, bounds.height as i32);
        let (cw, ch) = (canvas.width as i32, canvas.height as i32);
        layer.image = transform.apply_to_image(&layer.image);
        layer.offset = match transform {
            Transform::Rotate90Cw => (ch - (oy + lh), ox),
            Transform::Rotate90Ccw => (oy, cw - (ox + lw)),
            Transform::Rotate180 => (cw - (ox + lw), ch - (oy + lh)),
            Transform::FlipHorizontal => (cw - (ox + lw), oy),
            Transform::FlipVertical => (ox, ch - (oy + lh)),
        };
    }
    // Marks live in canvas coordinates, so they follow the exact same
    // mapping as layer offsets (f32 points; the rect re-normalizes).
    let (cw, ch) = (canvas.width as f32, canvas.height as f32);
    let map_pt = |x: f32, y: f32| -> (f32, f32) {
        match transform {
            Transform::Rotate90Cw => (ch - y, x),
            Transform::Rotate90Ccw => (y, cw - x),
            Transform::Rotate180 => (cw - x, ch - y),
            Transform::FlipHorizontal => (cw - x, y),
            Transform::FlipVertical => (x, ch - y),
        }
    };
    for mark in &mut doc.marks {
        let (x0, y0) = map_pt(mark.rect.0, mark.rect.1);
        let (x1, y1) = map_pt(mark.rect.2, mark.rect.3);
        mark.rect = norm_rect((x0, y0, x1, y1));
        for p in &mut mark.points {
            *p = map_pt(p.0, p.1);
        }
    }
    // `view` lives in base coordinates, so it maps against the base dimensions
    // (layers use canvas dimensions, since their offsets are canvas-relative).
    let base_before = Size::new(doc.base.width(), doc.base.height());
    doc.view = transform_rect(doc.view, transform, base_before);
    doc.base = transform.apply_to_image(&doc.base);
    if transform.swaps_axes() {
        doc.canvas = Size::new(canvas.height, canvas.width);
    }
}
