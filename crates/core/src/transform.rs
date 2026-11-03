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

    pub(crate) fn apply_to_image(self, image: &RgbaImage) -> RgbaImage {
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

/// 任意角度旋转一张位图（双线性重采样，预乘 alpha，避免边缘发黑）。
///
/// `expand` = true 时按旋转后的包围盒扩大位图（内容不裁切）；false 时
/// 保持原尺寸、四角被裁掉。返回新位图与「新左上角相对旧左上角」的偏移，
/// 调用方据此让图层绕自己的中心转（PS 的行为）。
pub fn rotate_image(img: &RgbaImage, degrees: f32, expand: bool) -> (RgbaImage, (i32, i32)) {
    let w = img.width() as f32;
    let h = img.height() as f32;
    if w < 1.0 || h < 1.0 || degrees.abs() < 0.001 {
        return (img.clone(), (0, 0));
    }
    let rad = degrees.to_radians();
    let (sin, cos) = (rad.sin(), rad.cos());
    let (nw, nh) = if expand {
        (
            (w * cos.abs() + h * sin.abs()).ceil().max(1.0) as u32,
            (w * sin.abs() + h * cos.abs()).ceil().max(1.0) as u32,
        )
    } else {
        (img.width(), img.height())
    };
    let (scx, scy) = (w / 2.0, h / 2.0);
    let (dcx, dcy) = (nw as f32 / 2.0, nh as f32 / 2.0);
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let mut out = RgbaImage::new(nw, nh);
    for dy in 0..nh {
        for dx in 0..nw {
            // 目标像素中心 → 相对新中心 → 反向旋转 → 源图坐标（像素中心）。
            let rx = dx as f32 + 0.5 - dcx;
            let ry = dy as f32 + 0.5 - dcy;
            let sx = cos * rx + sin * ry + scx - 0.5;
            let sy = -sin * rx + cos * ry + scy - 0.5;
            let x0 = sx.floor() as i32;
            let y0 = sy.floor() as i32;
            let tx = sx - x0 as f32;
            let ty = sy - y0 as f32;
            let mut acc = [0f32; 4];
            let mut asum = 0f32;
            for (ox, oy, wt) in [
                (0, 0, (1.0 - tx) * (1.0 - ty)),
                (1, 0, tx * (1.0 - ty)),
                (0, 1, (1.0 - tx) * ty),
                (1, 1, tx * ty),
            ] {
                if wt <= 0.0 {
                    continue;
                }
                let (x, y) = (x0 + ox, y0 + oy);
                if x < 0 || y < 0 || x >= iw || y >= ih {
                    continue;
                }
                let p = img.get_pixel(x as u32, y as u32).0;
                let a = p[3] as f32 / 255.0 * wt;
                for k in 0..3 {
                    acc[k] += p[k] as f32 * a;
                }
                asum += a;
            }
            if asum <= 1e-5 {
                out.put_pixel(dx, dy, image::Rgba([0, 0, 0, 0]));
                continue;
            }
            out.put_pixel(
                dx,
                dy,
                image::Rgba([
                    (acc[0] / asum).round().clamp(0.0, 255.0) as u8,
                    (acc[1] / asum).round().clamp(0.0, 255.0) as u8,
                    (acc[2] / asum).round().clamp(0.0, 255.0) as u8,
                    (asum * 255.0).round().clamp(0.0, 255.0) as u8,
                ]),
            );
        }
    }
    let delta = (
        ((w - nw as f32) / 2.0).round() as i32,
        ((h - nh as f32) / 2.0).round() as i32,
    );
    (out, delta)
}
