// crates/core/src/mark.rs
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Rect,
    Ellipse,
    Arrow,
    Brush,
    /// Mosaic: pixelates the image under the stroke path, or the whole rect
    /// when `points` is empty (drag-selection mode).
    Mosaic,
    /// Text run anchored at the rect's top-left corner.
    Text,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    /// Normalized bounds `(x0, y0, x1, y1)` with x1 >= x0, y1 >= y0 — the
    /// drag box for Rect/Ellipse and the tight bounding box of `points`
    /// otherwise. The UI uses it for hit-testing and resize handles.
    pub rect: (f32, f32, f32, f32),
    /// Arrow: exactly [start, end]. Brush: polyline vertices.
    pub points: Vec<(f32, f32)>,
    /// Stroke width in image pixels.
    pub stroke: u8,
    /// Stroke color RGBA.
    pub color: [u8; 4],
    /// Rect/Ellipse only: fill the interior with `fill_color`.
    pub fill: bool,
    pub fill_color: [u8; 4],
    /// Mosaic only: pixelation block size in image px (the "strength").
    pub cell: u8,
    /// Text only: the string (may contain '\n'), family id, size in px and
    /// faux bold / italic switches.
    pub text: String,
    pub font: String,
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    /// Text only: underline every line; alignment 0 left / 1 center / 2 right.
    pub underline: bool,
    pub align: u8,
    /// Hidden marks stay in the document (and the panel list) but are not
    /// rasterized onto the canvas.
    pub visible: bool,
}

impl Mark {
    /// A fully-defaulted mark body for the given kind; every construction
    /// site starts from this and overrides what it needs.
    pub fn new(kind: MarkKind) -> Self {
        Self {
            kind,
            rect: (0.0, 0.0, 0.0, 0.0),
            points: Vec::new(),
            stroke: 4,
            color: [226, 75, 74, 255],
            fill: false,
            fill_color: [0, 0, 0, 0],
            cell: 8,
            text: String::new(),
            font: String::new(),
            font_size: 28.0,
            bold: false,
            italic: false,
            underline: false,
            align: 0,
            visible: true,
        }
    }
}

pub(crate) fn norm_rect(r: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    (
        r.0.min(r.2),
        r.1.min(r.3),
        r.0.max(r.2),
        r.1.max(r.3),
    )
}

impl Mark {
    /// Translate every coordinate by (dx, dy).
    pub fn translated(&self, dx: f32, dy: f32) -> Mark {
        let mut m = self.clone();
        m.rect = (m.rect.0 + dx, m.rect.1 + dy, m.rect.2 + dx, m.rect.3 + dy);
        for p in &mut m.points {
            *p = (p.0 + dx, p.1 + dy);
        }
        m
    }

    /// Linearly map the mark's geometry from `from` bounds to `to` bounds
    /// (handle-driven resize: every point keeps its relative position inside
    /// the box).
    pub fn mapped(&self, from: (f32, f32, f32, f32), to: (f32, f32, f32, f32)) -> Mark {
        let (fx0, fy0, fx1, fy1) = from;
        let (tx0, ty0, tx1, ty1) = norm_rect(to);
        let sx = if fx1 - fx0 > f32::EPSILON {
            (tx1 - tx0) / (fx1 - fx0)
        } else {
            1.0
        };
        let sy = if fy1 - fy0 > f32::EPSILON {
            (ty1 - ty0) / (fy1 - fy0)
        } else {
            1.0
        };
        let mut m = self.clone();
        m.rect = (
            tx0 + (m.rect.0 - fx0) * sx,
            ty0 + (m.rect.1 - fy0) * sy,
            tx0 + (m.rect.2 - fx0) * sx,
            ty0 + (m.rect.3 - fy0) * sy,
        );
        for p in &mut m.points {
            *p = (tx0 + (p.0 - fx0) * sx, ty0 + (p.1 - fy0) * sy);
        }
        m
    }

    /// Rasterize onto a canvas-resolution image.
    pub fn draw(&self, img: &mut RgbaImage) {
        draw_mark(img, self, 1.0, 1.0);
    }

    /// Rasterize onto a preview image (`sx`/`sy` = preview px per image px).
    pub fn draw_scaled(&self, img: &mut RgbaImage, sx: f32, sy: f32) {
        draw_mark(img, self, sx, sy);
    }
}

fn blend_px(img: &mut RgbaImage, x: i32, y: i32, c: [u8; 4]) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 || c[3] == 0 {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    let a = c[3] as f32 / 255.0;
    let inv = 1.0 - a;
    for i in 0..3 {
        p.0[i] = (c[i] as f32 * a + p.0[i] as f32 * inv).round() as u8;
    }
    p.0[3] = ((a + p.0[3] as f32 / 255.0 * inv) * 255.0).round() as u8;
}

fn fill_rect_px(img: &mut RgbaImage, x0: f32, y0: f32, x1: f32, y1: f32, c: [u8; 4]) {
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for y in y0.floor() as i32..y1.ceil() as i32 {
        for x in x0.floor() as i32..x1.ceil() as i32 {
            // Pixel centers, so partial bands don't paint one extra row.
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if px >= x0 && px <= x1 && py >= y0 && py <= y1 {
                blend_px(img, x, y, c);
            }
        }
    }
}

fn stroke_rect_px(img: &mut RgbaImage, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, c: [u8; 4]) {
    let w = w.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0);
    if w <= 0.0 {
        return;
    }
    fill_rect_px(img, x0, y0, x1, y0 + w, c);
    fill_rect_px(img, x0, y1 - w, x1, y1, c);
    fill_rect_px(img, x0, y0 + w, x0 + w, y1 - w, c);
    fill_rect_px(img, x1 - w, y0 + w, x1, y1 - w, c);
}

/// Fill an ellipse inscribed in (x0,y0)-(x1,y1). With `ring > 0` only the
/// outer ring of that thickness (the stroke) is painted instead of the whole
/// interior — a per-pixel test, so strokes never overwrite the fill color.
fn fill_ellipse_px(
    img: &mut RgbaImage,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    c: [u8; 4],
    ring: f32,
) {
    let cx = (x0 + x1) / 2.0;
    let cy = (y0 + y1) / 2.0;
    let rx = ((x1 - x0) / 2.0).max(0.5);
    let ry = ((y1 - y0) / 2.0).max(0.5);
    let irx = (rx - ring).max(0.0);
    let iry = (ry - ring).max(0.0);
    for y in y0.floor() as i32..=(y1.ceil() as i32) {
        for x in x0.floor() as i32..=(x1.ceil() as i32) {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            if (dx / rx).powi(2) + (dy / ry).powi(2) > 1.0 {
                continue;
            }
            if ring > 0.0 && irx > 0.0 && iry > 0.0 {
                let d = (dx / irx).powi(2) + (dy / iry).powi(2);
                if d <= 1.0 {
                    continue;
                }
            }
            blend_px(img, x, y, c);
        }
    }
}

fn stamp_brush(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
    let r = r.max(0.5);
    let r2 = r * r;
    for y in (cy - r).floor() as i32..=(cy + r).ceil() as i32 {
        for x in (cx - r).floor() as i32..=(cx + r).ceil() as i32 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            if dx * dx + dy * dy <= r2 {
                blend_px(img, x, y, c);
            }
        }
    }
}

fn draw_thick_segment(
    img: &mut RgbaImage,
    a: (f32, f32),
    b: (f32, f32),
    w: f32,
    c: [u8; 4],
) {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt();
    let r = (w / 2.0).max(0.5);
    // Dense enough that stamps overlap into a smooth line.
    let steps = (len / (r * 0.6).max(0.4)).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        stamp_brush(img, a.0 + dx * t, a.1 + dy * t, r, c);
    }
}

fn fill_triangle_px(img: &mut RgbaImage, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), c: [u8; 4]) {
    let area = (p1.0 - p0.0) * (p2.1 - p0.1) - (p2.0 - p0.0) * (p1.1 - p0.1);
    if area.abs() < f32::EPSILON {
        return;
    }
    let min_x = p0.0.min(p1.0).min(p2.0).floor() as i32;
    let max_x = p0.0.max(p1.0).max(p2.0).ceil() as i32;
    let min_y = p0.1.min(p1.1).min(p2.1).floor() as i32;
    let max_y = p0.1.max(p1.1).max(p2.1).ceil() as i32;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let q = (x as f32 + 0.5, y as f32 + 0.5);
            let mut sign = 0.0f32;
            let mut inside = true;
            let pts = [p0, p1, p2, p0];
            for i in 0..3 {
                let a = pts[i];
                let b = pts[i + 1];
                let e = (b.0 - a.0) * (q.1 - a.1) - (b.1 - a.1) * (q.0 - a.0);
                let s = if e > 0.0 {
                    1.0
                } else if e < 0.0 {
                    -1.0
                } else {
                    0.0
                };
                if i == 0 {
                    sign = s;
                } else if s != 0.0 && s != sign {
                    inside = false;
                    break;
                }
            }
            if inside {
                blend_px(img, x, y, c);
            }
        }
    }
}

fn draw_mark(img: &mut RgbaImage, mark: &Mark, sx: f32, sy: f32) {
    let sw = (mark.stroke as f32 * (sx + sy) * 0.5).max(1.0);
    let sp = |p: (f32, f32)| (p.0 * sx, p.1 * sy);
    match mark.kind {
        MarkKind::Rect | MarkKind::Ellipse => {
            let (rx0, ry0, rx1, ry1) = mark.rect;
            let (x0, x1) = (rx0.min(rx1) * sx, rx0.max(rx1) * sx);
            let (y0, y1) = (ry0.min(ry1) * sy, ry0.max(ry1) * sy);
            if x1 - x0 < 0.5 || y1 - y0 < 0.5 {
                return;
            }
            if mark.fill {
                if matches!(mark.kind, MarkKind::Rect) {
                    fill_rect_px(img, x0, y0, x1, y1, mark.fill_color);
                } else {
                    fill_ellipse_px(img, x0, y0, x1, y1, mark.fill_color, 0.0);
                }
            }
            if matches!(mark.kind, MarkKind::Rect) {
                stroke_rect_px(img, x0, y0, x1, y1, sw, mark.color);
            } else {
                fill_ellipse_px(img, x0, y0, x1, y1, mark.color, sw);
            }
        }
        MarkKind::Arrow => {
            if mark.points.len() < 2 {
                return;
            }
            let a = sp(mark.points[0]);
            let b = sp(mark.points[1]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt();
            if len < f32::EPSILON {
                return;
            }
            let (ux, uy) = (dx / len, dy / len);
            let head = (sw * 3.5).max(10.0).min(len * 0.5);
            let base = (b.0 - ux * head, b.1 - uy * head);
            draw_thick_segment(img, a, base, sw, mark.color);
            let hw = head * 0.55;
            fill_triangle_px(
                img,
                b,
                (base.0 - uy * hw, base.1 + ux * hw),
                (base.0 + uy * hw, base.1 - ux * hw),
                mark.color,
            );
        }
        MarkKind::Brush => {
            if mark.points.is_empty() {
                return;
            }
            if mark.points.len() == 1 {
                let p = sp(mark.points[0]);
                stamp_brush(img, p.0, p.1, sw / 2.0, mark.color);
            }
            for w in mark.points.windows(2) {
                draw_thick_segment(img, sp(w[0]), sp(w[1]), sw, mark.color);
            }
        }
        MarkKind::Mosaic => {
            // Block size scales with the preview like every other mark.
            let cell = (mark.cell.max(2) as f32 * (sx + sy) * 0.5).max(2.0);
            // Snap cells to a grid: unique cells are averaged BEFORE any
            // painting so overlapping stamps never average already-pixelated
            // output.
            let mut cells: Vec<(i64, i64)> = Vec::new();
            if mark.points.is_empty() {
                // Region mode: pixelate the whole rect (drag-selection).
                let (rx0, ry0, rx1, ry1) = mark.rect;
                let (x0, y0) = (rx0.min(rx1) * sx, ry0.min(ry1) * sy);
                let (x1, y1) = (rx1.max(rx0) * sx, ry1.max(ry0) * sy);
                if x1 - x0 >= 1.0 && y1 - y0 >= 1.0 {
                    let gx0 = (x0 / cell).floor() as i64;
                    let gx1 = ((x1 / cell).ceil() as i64).max(gx0 + 1);
                    let gy0 = (y0 / cell).floor() as i64;
                    let gy1 = ((y1 / cell).ceil() as i64).max(gy0 + 1);
                    for gy in gy0..gy1 {
                        for gx in gx0..gx1 {
                            cells.push((gx, gy));
                        }
                    }
                }
            } else {
                let r = sw / 2.0;
                // Stamp along the path like the brush.
                let push_stamp = |cells: &mut Vec<(i64, i64)>, cx: f32, cy: f32| {
                    let gx0 = ((cx - r) / cell).floor() as i64;
                    let gx1 = ((cx + r) / cell).floor() as i64;
                    let gy0 = ((cy - r) / cell).floor() as i64;
                    let gy1 = ((cy + r) / cell).floor() as i64;
                    for gy in gy0..=gy1 {
                        for gx in gx0..=gx1 {
                            // Cell center within the brush radius?
                            let ccx = (gx as f32 + 0.5) * cell;
                            let ccy = (gy as f32 + 0.5) * cell;
                            if (ccx - cx).powi(2) + (ccy - cy).powi(2) <= r * r
                                && !cells.contains(&(gx, gy))
                            {
                                cells.push((gx, gy));
                            }
                        }
                    }
                };
                if mark.points.len() == 1 {
                    let p = sp(mark.points[0]);
                    push_stamp(&mut cells, p.0, p.1);
                }
                for w in mark.points.windows(2) {
                    let a = sp(w[0]);
                    let b = sp(w[1]);
                    let dx = b.0 - a.0;
                    let dy = b.1 - a.1;
                    let len = (dx * dx + dy * dy).sqrt();
                    let steps = (len / (cell * 0.4).max(0.5)).ceil().max(1.0) as i32;
                    for i in 0..=steps {
                        let t = i as f32 / steps as f32;
                        push_stamp(&mut cells, a.0 + dx * t, a.1 + dy * t);
                    }
                }
            }
            // Pass 1: average the underlying (already-composited) pixels.
            let mut avgs = Vec::with_capacity(cells.len());
            for &(gx, gy) in &cells {
                let x0 = (gx as f32 * cell).floor().max(0.0) as i32;
                let y0 = (gy as f32 * cell).floor().max(0.0) as i32;
                let x1 = ((gx as f32 + 1.0) * cell).ceil().min(img.width() as f32) as i32;
                let y1 = ((gy as f32 + 1.0) * cell).ceil().min(img.height() as f32) as i32;
                let (mut sr, mut sg, mut sb, mut sa) = (0u64, 0u64, 0u64, 0u64);
                let mut n = 0u64;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let p = img.get_pixel(x as u32, y as u32).0;
                        sr += p[0] as u64;
                        sg += p[1] as u64;
                        sb += p[2] as u64;
                        sa += p[3] as u64;
                        n += 1;
                    }
                }
                if n == 0 {
                    // Cell entirely outside the image (mark dragged past the
                    // canvas edge): push a placeholder so `avgs` stays 1:1
                    // with `cells` — pass 2 indexes avgs[idx] and the alpha-0
                    // check below skips the cell. Skipping the push instead
                    // misaligned the two lists (index-out-of-bounds abort).
                    avgs.push([0, 0, 0, 0]);
                    continue;
                }
                avgs.push([
                    (sr / n) as u8,
                    (sg / n) as u8,
                    (sb / n) as u8,
                    (sa / n) as u8,
                ]);
            }
            // Pass 2: paint every cell opaque with its average.
            for (idx, &(gx, gy)) in cells.iter().enumerate() {
                let c = avgs[idx];
                if c[3] == 0 {
                    continue;
                }
                let x0 = gx as f32 * cell;
                let y0 = gy as f32 * cell;
                fill_rect_px(img, x0, y0, x0 + cell, y0 + cell, c);
            }
        }
        MarkKind::Text => {
            if mark.text.is_empty() {
                return;
            }
            let layout = crate::font::layout_text_ex(
                &mark.text,
                &mark.font,
                mark.font_size,
                mark.bold,
                mark.italic,
                mark.underline,
                mark.align,
            );
            if layout.width == 0 || layout.height == 0 {
                return;
            }
            // The rendered text exactly fills its (possibly resized) rect:
            // blit the natural coverage buffer with per-axis scale factors.
            let (rx0, ry0, rx1, ry1) = mark.rect;
            let (x0, y0) = (rx0.min(rx1) * sx, ry0.min(ry1) * sy);
            let dw = ((rx1 - rx0).abs() * sx).round().max(1.0);
            let dh = ((ry1 - ry0).abs() * sy).round().max(1.0);
            let fx = dw / layout.width as f32;
            let fy = dh / layout.height as f32;
            for dy in 0..dh as i32 {
                let syi = ((dy as f32 + 0.5) / fy) as u32;
                let syi = syi.min(layout.height - 1);
                for dx in 0..dw as i32 {
                    let sxi = ((dx as f32 + 0.5) / fx) as u32;
                    let sxi = sxi.min(layout.width - 1);
                    let a = layout.coverage[(syi * layout.width + sxi) as usize];
                    if a == 0 {
                        continue;
                    }
                    blend_px(
                        img,
                        x0 as i32 + dx,
                        y0 as i32 + dy,
                        [mark.color[0], mark.color[1], mark.color[2], a],
                    );
                }
            }
        }
    }
}

