use image::RgbaImage;

pub mod font;
pub use font::{measure_text, FONTS};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub const fn pixels(self) -> u64 {
        self.width as u64 * self.height as u64
    }

    pub const fn megapixels(self) -> f64 {
        self.pixels() as f64 / 1_000_000.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub const fn right(self) -> i32 {
        self.x + self.width as i32
    }

    pub const fn bottom(self) -> i32 {
        self.y + self.height as i32
    }

    pub fn intersect(self, other: Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= x || bottom <= y {
            None
        } else {
            Some(Rect::new(x, y, (right - x) as u32, (bottom - y) as u32))
        }
    }

    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Rect::new(x, y, (right - x) as u32, (bottom - y) as u32)
    }

    pub fn translated(self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.width, self.height)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    Rect(Rect),
    Ellipse(Rect),
    Lasso(Vec<(f32, f32)>),
}

impl Selection {
    pub fn bounds(&self) -> Rect {
        match self {
            Selection::Rect(r) | Selection::Ellipse(r) => *r,
            Selection::Lasso(points) => {
                if points.is_empty() {
                    return Rect::default();
                }
                let mut min_x = f32::INFINITY;
                let mut min_y = f32::INFINITY;
                let mut max_x = f32::NEG_INFINITY;
                let mut max_y = f32::NEG_INFINITY;
                for (x, y) in points {
                    min_x = min_x.min(*x);
                    min_y = min_y.min(*y);
                    max_x = max_x.max(*x);
                    max_y = max_y.max(*y);
                }
                Rect::new(
                    min_x.floor() as i32,
                    min_y.floor() as i32,
                    (max_x - min_x).ceil().max(0.0) as u32,
                    (max_y - min_y).ceil().max(0.0) as u32,
                )
            }
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            Selection::Rect(r) => {
                x >= r.x as f32
                    && y >= r.y as f32
                    && x < r.right() as f32
                    && y < r.bottom() as f32
            }
            Selection::Ellipse(r) => {
                if r.is_empty() {
                    return false;
                }
                let cx = r.x as f32 + r.width as f32 / 2.0;
                let cy = r.y as f32 + r.height as f32 / 2.0;
                let rx = r.width as f32 / 2.0;
                let ry = r.height as f32 / 2.0;
                let dx = (x - cx) / rx;
                let dy = (y - cy) / ry;
                dx * dx + dy * dy <= 1.0
            }
            Selection::Lasso(points) => {
                let n = points.len();
                if n < 3 {
                    return false;
                }
                let mut inside = false;
                let mut j = n - 1;
                for i in 0..n {
                    let (xi, yi) = points[i];
                    let (xj, yj) = points[j];
                    if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                        inside = !inside;
                    }
                    j = i;
                }
                inside
            }
        }
    }
}

// ---- 标记（markup annotations）------------------------------------------
//
// Vector annotations drawn ON TOP of the flattened image at render / export
// time. They never touch pixels until composition, so adding / moving /
// restyling one is cheap and fully undoable. Coordinates are canvas-space
// f32; stroke widths are image pixels at full resolution.

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

fn norm_rect(r: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
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

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub image: RgbaImage,
    pub offset: (i32, i32),
    pub opacity: f32,
    pub visible: bool,
    /// UI-only guard: locked layers reject move / scale / delete in the
    /// bridge. Plain flag, not part of undo history.
    pub locked: bool,
}

impl Layer {
    pub fn new(name: impl Into<String>, image: RgbaImage) -> Self {
        Self {
            name: name.into(),
            image,
            offset: (0, 0),
            opacity: 1.0,
            visible: true,
            locked: false,
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(
            self.offset.0,
            self.offset.1,
            self.image.width(),
            self.image.height(),
        )
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    pub source: Option<std::path::PathBuf>,
    pub canvas: Size,
    pub base: RgbaImage,
    /// Visible region of `base`, in `base` coordinates. Cropping only shrinks
    /// this rect (no pixel copy), so crop is O(1) and reversible.
    pub view: Rect,
    pub layers: Vec<Layer>,
    /// True while the base layer has been "deleted" (fully transparent
    /// canvas). Tracked so the panel can grey out the base row.
    pub base_deleted: bool,
    /// True while the base layer is hidden by the user (eye toggle in the
    /// panel). Non-destructive and undoable.
    pub base_hidden: bool,
    /// True once the base image has been converted into a regular layer
    /// ("unlock" in the panel): it becomes movable / scalable / deletable
    /// like any other layer, and the synthetic base row disappears. The
    /// base field stays as a transparent canvas of the same size so crop /
    /// rotate / canvas-resize keep working unchanged.
    pub base_converted: bool,
    /// Vector markup annotations (rect / ellipse / arrow / brush / …) drawn
    /// on top of the composite at render time. Canvas-space coordinates.
    pub marks: Vec<Mark>,
    /// Monotonic counter bumped by every committed / structural change
    /// (execute, undo, redo). Preview caches key on it: any real edit
    /// invalidates them, while history-free live mutations (layer offset
    /// during a drag, live mark moves) deliberately do NOT bump it so each
    /// drag tick can reuse cached scaled layers.
    pub edit_epoch: u64,
    /// Auto-naming counter: new layers (extract/paste) are named by their
    /// creation order ("1", "2", ...). Not part of snapshots on purpose, so
    /// names keep increasing across undo/redo.
    next_layer_seq: u32,
    history: History,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("layer index {0} out of range")]
    LayerIndex(usize),
    #[error("mark index {0} out of range")]
    MarkIndex(usize),
    #[error("selection is empty")]
    EmptySelection,
    #[error("crop rect is empty or outside the image")]
    EmptyCrop,
    #[error("target size must be at least 1x1")]
    InvalidCanvasSize,
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
    #[error("the base layer needs at least one other layer")]
    BaseRequired,
}

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

fn transform_document(doc: &mut Document, transform: Transform) {
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

/// Full document state taken before a pixel-modifying op, so undo can restore
/// it exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSnapshot {
    pub canvas: Size,
    pub base: RgbaImage,
    pub view: Rect,
    pub layers: Vec<Layer>,
    /// True while the base has been "deleted" (fully transparent canvas).
    pub base_deleted: bool,
    /// Base hidden by the user (eye toggle).
    pub base_hidden: bool,
    /// Base already converted into a regular layer (panel unlock).
    pub base_converted: bool,
    /// Vector markup annotations.
    pub marks: Vec<Mark>,
}

impl Document {
    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            canvas: self.canvas,
            base: self.base.clone(),
            view: self.view,
            layers: self.layers.clone(),
            base_deleted: self.base_deleted,
            base_hidden: self.base_hidden,
            base_converted: self.base_converted,
            marks: self.marks.clone(),
        }
    }

    /// Replace everything except history (used when undoing a pixel op).
    pub fn restore(&mut self, snapshot: DocumentSnapshot) {
        self.canvas = snapshot.canvas;
        self.base = snapshot.base;
        self.view = snapshot.view;
        self.layers = snapshot.layers;
        self.base_deleted = snapshot.base_deleted;
        self.base_hidden = snapshot.base_hidden;
        self.base_converted = snapshot.base_converted;
        self.marks = snapshot.marks;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditOp {
    TransformDocument { transform: Transform },
    CropDocument { from: Rect, to: Rect },
    ResizeDocument { from: DocumentSnapshot, to: Size },
    AddLayer { layer: Layer, index: usize },
    RemoveLayer { layer: Layer, index: usize },
    MoveLayer { from: usize, to: usize },
    SetVisible { index: usize, from: bool, to: bool },
    SetOpacity { index: usize, from: f32, to: f32 },
    TranslateLayer { index: usize, from: (i32, i32), to: (i32, i32) },
    RenameLayer { index: usize, from: String, to: String },
    /// Non-destructive "delete selection pixels": the base pixels under the
    /// selection are made fully transparent while layers stay untouched. The
    /// pre-op snapshot makes undo exact (same pattern as `ResizeDocument`).
    EraseBasePixels { from: DocumentSnapshot, selection: Selection },
    /// Scale a layer's image to `to` (Lanczos3) and move its top-left corner
    /// to `offset`, so handle-driven resizes anchor at the dragged corner /
    /// edge instead of always growing toward the bottom-right. The pre-op
    /// snapshot makes undo exact (same pattern as `ResizeDocument`).
    ScaleLayer {
        index: usize,
        from: DocumentSnapshot,
        to: Size,
        offset: (i32, i32),
    },
    /// "Delete" the base layer: the canvas becomes fully transparent while
    /// layers stay untouched. Undo restores the previous base pixels.
    DeleteBase { from: DocumentSnapshot },
    /// Hide/show the base layer (eye toggle in the panel). Cheap flag flip,
    /// no pixels involved; snapshot-free undo via from/to.
    SetBaseVisible { from: bool, to: bool },
    /// Change the canvas resolution WITHOUT resampling any pixels: content
    /// keeps its size and is re-anchored to the center of the new canvas
    /// (clipped when shrinking, transparent padding when growing). The
    /// pre-op snapshot makes undo exact (same pattern as `ResizeDocument`).
    CanvasResize { from: DocumentSnapshot, to: Size },
    /// "Unlock" the base layer: its pixels become a regular layer at the
    /// bottom of the stack (movable / scalable / deletable like any other
    /// layer) and the base itself becomes a transparent canvas of the same
    /// size. The pre-op snapshot makes undo exact.
    BaseToLayer { from: DocumentSnapshot },
    /// Add a markup annotation on top of the composition. `index` is always
    /// the current list length (marks stack in creation order).
    AddMark { index: usize, mark: Mark },
    /// Restyle / move / resize an existing annotation. `from` is the mark
    /// before the change (undo restores it verbatim).
    UpdateMark { index: usize, from: Mark, to: Mark },
    /// Remove an annotation; the removed mark is kept for undo.
    DeleteMark { index: usize, mark: Mark },
}

impl EditOp {
    pub fn apply(&self, doc: &mut Document) -> Result<(), EditError> {
        match self {
            EditOp::TransformDocument { transform } => {
                transform_document(doc, *transform);
                Ok(())
            }
            EditOp::CropDocument { to, .. } => {
                apply_view(doc, *to);
                Ok(())
            }
            EditOp::ResizeDocument { to, .. } => {
                apply_resize(doc, *to);
                Ok(())
            }
            EditOp::AddLayer { layer, index } => {
                if *index > doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.insert(*index, layer.clone());
                Ok(())
            }
            EditOp::RemoveLayer { index, .. } => {
                if *index >= doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.remove(*index);
                Ok(())
            }
            EditOp::MoveLayer { from, to } => {
                if *from >= doc.layers.len() || *to >= doc.layers.len() {
                    return Err(EditError::LayerIndex((*from).max(*to)));
                }
                let layer = doc.layers.remove(*from);
                doc.layers.insert(*to, layer);
                Ok(())
            }
            EditOp::SetVisible { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.visible = *to;
                Ok(())
            }
            EditOp::SetOpacity { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.opacity = to.clamp(0.0, 1.0);
                Ok(())
            }
            EditOp::TranslateLayer { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.offset = *to;
                Ok(())
            }
            EditOp::RenameLayer { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.name = to.clone();
                Ok(())
            }
            EditOp::EraseBasePixels { selection, .. } => {
                erase_base_pixels(doc, selection);
                Ok(())
            }
            EditOp::ScaleLayer { index, to, offset, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                if to.width < 1 || to.height < 1 {
                    return Err(EditError::InvalidCanvasSize);
                }
                layer.image = image::imageops::resize(
                    &layer.image,
                    to.width,
                    to.height,
                    image::imageops::FilterType::Lanczos3,
                );
                layer.offset = *offset;
                Ok(())
            }
            EditOp::DeleteBase { .. } => {
                apply_delete_base(doc);
                Ok(())
            }
            EditOp::SetBaseVisible { to, .. } => {
                doc.base_hidden = *to;
                Ok(())
            }
            EditOp::CanvasResize { to, .. } => {
                apply_canvas_resize(doc, *to);
                Ok(())
            }
            EditOp::BaseToLayer { .. } => {
                apply_base_to_layer(doc);
                Ok(())
            }
            EditOp::AddMark { index, mark } => {
                if *index > doc.marks.len() {
                    return Err(EditError::MarkIndex(*index));
                }
                doc.marks.insert(*index, mark.clone());
                Ok(())
            }
            EditOp::UpdateMark { index, to, .. } => {
                let m = doc
                    .marks
                    .get_mut(*index)
                    .ok_or(EditError::MarkIndex(*index))?;
                *m = to.clone();
                Ok(())
            }
            EditOp::DeleteMark { index, .. } => {
                if *index >= doc.marks.len() {
                    return Err(EditError::MarkIndex(*index));
                }
                doc.marks.remove(*index);
                Ok(())
            }
        }
    }

    pub fn revert(&self, doc: &mut Document) -> Result<(), EditError> {
        match self {
            EditOp::TransformDocument { transform } => {
                transform_document(doc, transform.inverted());
                Ok(())
            }
            EditOp::CropDocument { from, .. } => {
                apply_view(doc, *from);
                Ok(())
            }
            EditOp::ResizeDocument { from, .. } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::AddLayer { index, .. } => {
                if *index >= doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.remove(*index);
                Ok(())
            }
            EditOp::RemoveLayer { layer, index } => {
                if *index > doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.insert(*index, layer.clone());
                Ok(())
            }
            EditOp::MoveLayer { from, to } => {
                if *from >= doc.layers.len() || *to >= doc.layers.len() {
                    return Err(EditError::LayerIndex((*from).max(*to)));
                }
                let layer = doc.layers.remove(*to);
                doc.layers.insert(*from, layer);
                Ok(())
            }
            EditOp::SetVisible { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.visible = *from;
                Ok(())
            }
            EditOp::SetOpacity { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.opacity = *from;
                Ok(())
            }
            EditOp::TranslateLayer { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.offset = *from;
                Ok(())
            }
            EditOp::RenameLayer { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.name = from.clone();
                Ok(())
            }
            EditOp::EraseBasePixels { from, .. } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::ScaleLayer { from, .. } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::DeleteBase { from } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::SetBaseVisible { from, .. } => {
                doc.base_hidden = *from;
                Ok(())
            }
            EditOp::CanvasResize { from, .. } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::BaseToLayer { from } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::AddMark { index, .. } => {
                if *index >= doc.marks.len() {
                    return Err(EditError::MarkIndex(*index));
                }
                doc.marks.remove(*index);
                Ok(())
            }
            EditOp::UpdateMark { index, from, .. } => {
                let m = doc
                    .marks
                    .get_mut(*index)
                    .ok_or(EditError::MarkIndex(*index))?;
                *m = from.clone();
                Ok(())
            }
            EditOp::DeleteMark { index, mark } => {
                if *index > doc.marks.len() {
                    return Err(EditError::MarkIndex(*index));
                }
                doc.marks.insert(*index, mark.clone());
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct History {
    undo: Vec<EditOp>,
    redo: Vec<EditOp>,
    limit: usize,
}

impl History {
    fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit,
        }
    }

    fn push(&mut self, op: EditOp) {
        self.undo.push(op);
        if self.limit > 0 && self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

fn apply_view(doc: &mut Document, view: Rect) {
    doc.view = view;
    doc.canvas = Size::new(view.width, view.height);
}

/// Make every base pixel under `selection` fully transparent. Selection
/// coordinates are canvas coordinates and map into `base` through `view`.
/// "Delete" the base: replace it with a fully transparent canvas-sized image.
fn apply_delete_base(doc: &mut Document) {
    doc.base = RgbaImage::new(doc.canvas.width, doc.canvas.height);
    doc.base_deleted = true;
}

/// "Unlock" the base: its pixels become a regular layer at the bottom of the
/// stack (offset by `view` so the visible region matches exactly) and the
/// base becomes a transparent image of the same size. Canvas size, `view`
/// and therefore crop / rotate / canvas-resize behavior stay unchanged; the
/// composite output is pixel-identical before and after.
fn apply_base_to_layer(doc: &mut Document) {
    if doc.base_converted {
        return;
    }
    let mut layer = Layer::new("背景", doc.base.clone());
    layer.offset = (doc.view.x, doc.view.y);
    let empty = RgbaImage::new(doc.base.width(), doc.base.height());
    doc.base = empty;
    doc.layers.insert(0, layer);
    doc.base_deleted = false;
    doc.base_hidden = false;
    doc.base_converted = true;
}

fn erase_base_pixels(doc: &mut Document, selection: &Selection) {    let canvas_rect = Rect::new(0, 0, doc.canvas.width, doc.canvas.height);
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
fn apply_canvas_resize(doc: &mut Document, to: Size) {
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

fn apply_resize(doc: &mut Document, to: Size) {
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

impl Document {
    pub fn from_image(image: RgbaImage) -> Self {
        let canvas = Size::new(image.width(), image.height());
        Self {
            source: None,
            canvas,
            base: image,
            view: Rect::new(0, 0, canvas.width, canvas.height),
            layers: Vec::new(),
            marks: Vec::new(),
            base_deleted: false,
            base_hidden: false,
            base_converted: false,
            edit_epoch: 0,
            next_layer_seq: 0,
            history: History::new(256),
        }
    }

    pub fn execute(&mut self, op: EditOp) -> Result<(), EditError> {
        op.apply(self)?;
        self.history.push(op);
        self.edit_epoch = self.edit_epoch.wrapping_add(1);
        Ok(())
    }

    /// Crop to `rect`, given in canvas coordinates. The rect is clamped to the
    /// current canvas; a rect that falls outside it is rejected. Use
    /// [`Document::reset_crop`] to reveal the full base image again.
    pub fn crop(&mut self, rect: Rect) -> Result<(), EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let clamped = rect.intersect(canvas_rect).ok_or(EditError::EmptyCrop)?;
        if clamped.is_empty() {
            return Err(EditError::EmptyCrop);
        }
        self.execute(EditOp::CropDocument {
            from: self.view,
            to: Rect::new(
                self.view.x + clamped.x,
                self.view.y + clamped.y,
                clamped.width,
                clamped.height,
            ),
        })
    }

    /// Undo every crop, showing the full base image again.
    pub fn reset_crop(&mut self) -> Result<(), EditError> {
        if self.view.x == 0
            && self.view.y == 0
            && self.view.width == self.base.width()
            && self.view.height == self.base.height()
        {
            return Ok(());
        }
        self.execute(EditOp::CropDocument {
            from: self.view,
            to: Rect::new(0, 0, self.base.width(), self.base.height()),
        })
    }

    pub fn apply_transform(&mut self, transform: Transform) -> Result<(), EditError> {
        self.execute(EditOp::TransformDocument { transform })
    }

    /// Resize the document to exactly `to`: the current canvas (what the user
    /// sees) is resampled (Lanczos3) into a new base, any crop is dropped, and
    /// layers are scaled proportionally. Full previous state is snapshotted for
    /// undo.
    pub fn resize(&mut self, to: Size) -> Result<(), EditError> {
        if to.width < 1 || to.height < 1 {
            return Err(EditError::InvalidCanvasSize);
        }
        let from = self.snapshot();
        self.execute(EditOp::ResizeDocument { from, to })
    }

    /// Change only the canvas resolution: no pixel is resampled. Content is
    /// re-anchored to the center of the new canvas (see `apply_canvas_resize`).
    pub fn resize_canvas(&mut self, to: Size) -> Result<(), EditError> {
        if to.width < 1 || to.height < 1 {
            return Err(EditError::InvalidCanvasSize);
        }
        let from = self.snapshot();
        self.execute(EditOp::CanvasResize { from, to })
    }

    pub fn add_layer(&mut self, layer: Layer) -> Result<usize, EditError> {
        let index = self.layers.len();
        self.execute(EditOp::AddLayer { layer, index })?;
        Ok(index)
    }

    /// Append a markup annotation (drawn on top of the composite). Returns
    /// its index (creation order, topmost last).
    pub fn add_mark(&mut self, mark: Mark) -> Result<usize, EditError> {
        let index = self.marks.len();
        self.execute(EditOp::AddMark { index, mark })?;
        Ok(index)
    }

    /// Replace a markup annotation (move / resize / restyle), undoable.
    pub fn update_mark(&mut self, index: usize, to: Mark) -> Result<(), EditError> {
        let from = self
            .marks
            .get(index)
            .cloned()
            .ok_or(EditError::MarkIndex(index))?;
        self.execute(EditOp::UpdateMark { index, from, to })
    }

    /// History-free live preview of [`Document::update_mark`] for handle
    /// drags; the committing call records the true pre-drag mark.
    pub fn update_mark_live(&mut self, index: usize, mark: Mark) -> Result<(), EditError> {
        let m = self
            .marks
            .get_mut(index)
            .ok_or(EditError::MarkIndex(index))?;
        *m = mark;
        Ok(())
    }

    /// Remove a markup annotation, undoable.
    pub fn delete_mark(&mut self, index: usize) -> Result<(), EditError> {
        let mark = self
            .marks
            .get(index)
            .cloned()
            .ok_or(EditError::MarkIndex(index))?;
        self.execute(EditOp::DeleteMark { index, mark })
    }

    pub fn remove_layer(&mut self, index: usize) -> Result<(), EditError> {
        let layer = self
            .layers
            .get(index)
            .cloned()
            .ok_or(EditError::LayerIndex(index))?;
        self.execute(EditOp::RemoveLayer { layer, index })
    }

    pub fn move_layer(&mut self, from: usize, to: usize) -> Result<(), EditError> {
        self.execute(EditOp::MoveLayer { from, to })
    }

    pub fn set_layer_visible(&mut self, index: usize, visible: bool) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .visible;
        self.execute(EditOp::SetVisible {
            index,
            from,
            to: visible,
        })
    }

    pub fn set_layer_opacity(&mut self, index: usize, opacity: f32) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .opacity;
        self.execute(EditOp::SetOpacity {
            index,
            from,
            to: opacity.clamp(0.0, 1.0),
        })
    }

    pub fn translate_layer(&mut self, index: usize, dx: i32, dy: i32) -> Result<(), EditError> {
        let layer = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?;
        let from = layer.offset;
        let to = (from.0 + dx, from.1 + dy);
        self.execute(EditOp::TranslateLayer { index, from, to })
    }

    pub fn rename_layer(&mut self, index: usize, name: impl Into<String>) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .name
            .clone();
        self.execute(EditOp::RenameLayer {
            index,
            from,
            to: name.into(),
        })
    }

    /// Scale a layer's image to exactly `to` (Lanczos3) and place its
    /// top-left corner at `offset`, so handle-driven resizes anchor at the
    /// dragged corner / edge. Undo restores the full previous state via
    /// snapshot.
    pub fn scale_layer(
        &mut self,
        index: usize,
        to: Size,
        offset: (i32, i32),
    ) -> Result<(), EditError> {
        if to.width < 1 || to.height < 1 {
            return Err(EditError::InvalidCanvasSize);
        }
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        let from = self.snapshot();
        self.execute(EditOp::ScaleLayer { index, from, to, offset })
    }

    /// History-free live preview of [`Document::scale_layer`], using a fast
    /// (Triangle) filter so handle drags render in real time. The committing
    /// call re-does the work with Lanczos3 and full undo support.
    pub fn scale_layer_live(
        &mut self,
        index: usize,
        to: Size,
        offset: (i32, i32),
    ) -> Result<(), EditError> {
        if to.width < 1 || to.height < 1 {
            return Err(EditError::InvalidCanvasSize);
        }
        let layer = self
            .layers
            .get_mut(index)
            .ok_or(EditError::LayerIndex(index))?;
        layer.image = image::imageops::resize(
            &layer.image,
            to.width,
            to.height,
            image::imageops::FilterType::Triangle,
        );
        layer.offset = offset;
        // Pixels changed without going through execute(): bump the epoch so
        // preview caches never serve a stale bitmap during live-scale drags.
        self.edit_epoch = self.edit_epoch.wrapping_add(1);
        Ok(())
    }

    /// "Delete" the base layer: the canvas becomes fully transparent while
    /// layers stay untouched. Requires at least one layer to remain.
    pub fn delete_base(&mut self) -> Result<(), EditError> {
        if self.layers.is_empty() {
            return Err(EditError::BaseRequired);
        }
        let from = self.snapshot();
        self.execute(EditOp::DeleteBase { from })
    }

    /// Hide/show the base layer. No-op (no history entry) when the state
    /// already matches.
    pub fn set_base_visible(&mut self, visible: bool) -> Result<(), EditError> {
        let from = self.base_hidden;
        let to = !visible;
        if from == to {
            return Ok(());
        }
        self.execute(EditOp::SetBaseVisible { from, to })
    }

    /// "Unlock" the base layer: convert it into a regular layer at the
    /// bottom of the stack so it can be moved / scaled / deleted / renamed
    /// like any other layer. Idempotent: converting twice is a no-op (the
    /// second call adds no history entry).
    pub fn convert_base_to_layer(&mut self) -> Result<(), EditError> {
        if self.base_converted {
            return Ok(());
        }
        let from = self.snapshot();
        self.execute(EditOp::BaseToLayer { from })
    }

    pub fn undo(&mut self) -> Result<(), EditError> {
        let op = self.history.undo.pop().ok_or(EditError::NothingToUndo)?;
        op.revert(self)?;
        self.history.redo.push(op);
        self.edit_epoch = self.edit_epoch.wrapping_add(1);
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), EditError> {
        let op = self.history.redo.pop().ok_or(EditError::NothingToRedo)?;
        op.apply(self)?;
        self.history.undo.push(op);
        self.edit_epoch = self.edit_epoch.wrapping_add(1);
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    pub fn selection_layer(&self, selection: &Selection) -> Result<Layer, EditError> {
        let bounds = selection.bounds();
        if bounds.is_empty() {
            return Err(EditError::EmptySelection);
        }
        let mut image = RgbaImage::new(bounds.width, bounds.height);
        for y in 0..bounds.height {
            for x in 0..bounds.width {
                let sx = bounds.x + x as i32;
                let sy = bounds.y + y as i32;
                if !selection.contains(sx as f32 + 0.5, sy as f32 + 0.5) {
                    continue;
                }
                image.put_pixel(x, y, self.composite_pixel(sx, sy));
            }
        }
        let mut layer = Layer::new("Selection", image);
        layer.offset = (bounds.x, bounds.y);
        Ok(layer)
    }

    /// Next auto name for a created layer ("1", "2", ...), by creation order.
    pub fn next_layer_name(&mut self) -> String {
        self.next_layer_seq += 1;
        self.next_layer_seq.to_string()
    }

    pub fn extract_selection(&mut self, selection: &Selection) -> Result<usize, EditError> {
        let mut layer = self.selection_layer(selection)?;
        layer.name = self.next_layer_name();
        let index = self.layers.len();
        self.execute(EditOp::AddLayer { layer, index })?;
        Ok(index)
    }

    /// Non-destructive "delete selection pixels": base pixels under the
    /// selection become transparent; layers are untouched; undo restores the
    /// exact previous state via snapshot.
    pub fn erase_selection(&mut self, selection: &Selection) -> Result<(), EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let clipped = selection
            .bounds()
            .intersect(canvas_rect)
            .ok_or(EditError::EmptySelection)?;
        if clipped.is_empty() {
            return Err(EditError::EmptySelection);
        }
        let from = self.snapshot();
        self.execute(EditOp::EraseBasePixels {
            from,
            selection: selection.clone(),
        })
    }

    pub fn composite_pixel(&self, x: i32, y: i32) -> image::Rgba<u8> {
        if x < 0 || y < 0 || x >= self.canvas.width as i32 || y >= self.canvas.height as i32 {
            return image::Rgba([0, 0, 0, 0]);
        }
        let bx = (self.view.x + x) as u32;
        let by = (self.view.y + y) as u32;
        let mut out = if !self.base_hidden
            && bx < self.base.width()
            && by < self.base.height()
        {
            self.base.get_pixel(bx, by).0
        } else {
            [0, 0, 0, 0]
        };
        for layer in &self.layers {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            let lx = x - layer.offset.0;
            let ly = y - layer.offset.1;
            if lx < 0
                || ly < 0
                || lx >= layer.image.width() as i32
                || ly >= layer.image.height() as i32
            {
                continue;
            }
            let src = layer.image.get_pixel(lx as u32, ly as u32).0;
            let a = (src[3] as f32 / 255.0) * layer.opacity;
            if a <= 0.0 {
                continue;
            }
            let inv = 1.0 - a;
            for c in 0..3 {
                out[c] = (src[c] as f32 * a + out[c] as f32 * inv).round() as u8;
            }
            out[3] = ((a + out[3] as f32 / 255.0 * inv) * 255.0).round() as u8;
        }
        image::Rgba(out)
    }

    pub fn flatten(&self) -> RgbaImage {
        let mut out = RgbaImage::new(self.canvas.width, self.canvas.height);
        for y in 0..self.canvas.height {
            for x in 0..self.canvas.width {
                out.put_pixel(x, y, self.composite_pixel(x as i32, y as i32));
            }
        }
        // Markup annotations always sit on top of every layer.
        for mark in &self.marks {
            if mark.visible {
                mark.draw(&mut out);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            if (x / 8 + y / 8) % 2 == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        })
    }

    #[test]
    fn extract_rect_selection_produces_layer_of_selection_bounds() {
        let mut doc = Document::from_image(checker(64, 32));
        let index = doc
            .extract_selection(&Selection::Rect(Rect::new(8, 4, 16, 12)))
            .unwrap();
        let layer = &doc.layers[index];
        assert_eq!(layer.image.width(), 16);
        assert_eq!(layer.image.height(), 12);
        assert_eq!(layer.offset, (8, 4));
        assert_eq!(layer.image.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(layer.image.get_pixel(0, 4).0, [255, 0, 0, 255]);
    }

    #[test]
    fn ellipse_selection_masks_corners() {
        let doc = Document::from_image(checker(64, 64));
        let layer = doc
            .selection_layer(&Selection::Ellipse(Rect::new(0, 0, 64, 64)))
            .unwrap();
        assert_eq!(layer.image.get_pixel(0, 0).0[3], 0);
        assert_eq!(layer.image.get_pixel(32, 32).0[3], 255);
    }

    #[test]
    fn lasso_selection_contains_polygon_interior() {
        let sel = Selection::Lasso(vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ]);
        assert!(sel.contains(5.0, 5.0));
        assert!(!sel.contains(15.0, 5.0));
        assert_eq!(sel.bounds(), Rect::new(0, 0, 10, 10));
    }

    #[test]
    fn history_roundtrips_layer_ops() {
        let mut doc = Document::from_image(checker(16, 16));
        doc.add_layer(Layer::new("clip", checker(4, 4))).unwrap();
        assert_eq!(doc.layers.len(), 1);
        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 0);
        doc.redo().unwrap();
        assert_eq!(doc.layers.len(), 1);
    }

    #[test]
    fn undo_restores_scalar_layer_properties() {
        let mut doc = Document::from_image(checker(16, 16));
        let index = doc.add_layer(Layer::new("clip", checker(4, 4))).unwrap();
        doc.set_layer_opacity(index, 0.4).unwrap();
        doc.set_layer_visible(index, false).unwrap();
        doc.translate_layer(index, 3, -2).unwrap();
        doc.rename_layer(index, "renamed").unwrap();

        doc.undo().unwrap();
        assert_eq!(doc.layers[index].name, "clip");
        doc.undo().unwrap();
        assert_eq!(doc.layers[index].offset, (0, 0));
        doc.undo().unwrap();
        assert!(doc.layers[index].visible);
        doc.undo().unwrap();
        assert_eq!(doc.layers[index].opacity, 1.0);
    }

    #[test]
    fn cross_document_paste_keeps_pixels_and_offset() {
        let mut src = Document::from_image(checker(32, 32));
        let index = src
            .extract_selection(&Selection::Rect(Rect::new(4, 6, 8, 8)))
            .unwrap();
        let layer = src.layers[index].clone();

        let mut dst = Document::from_image(checker(16, 16));
        let pasted = dst.add_layer(layer).unwrap();
        assert_eq!(dst.layers[pasted].offset, (4, 6));
        assert_eq!(dst.layers[pasted].image.width(), 8);
        assert_eq!(dst.composite_pixel(4, 6).0, [255, 0, 0, 255]);
    }

    #[test]
    fn erase_selection_makes_base_pixels_transparent_and_keeps_layers() {
        let mut doc = Document::from_image(checker(32, 16));
        let index = doc
            .extract_selection(&Selection::Rect(Rect::new(20, 2, 8, 8)))
            .unwrap();

        doc.erase_selection(&Selection::Rect(Rect::new(4, 4, 8, 8)))
            .unwrap();
        // Erased base area is transparent (no layer above it there).
        assert_eq!(doc.composite_pixel(4, 4).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(11, 11).0, [0, 0, 0, 0]);
        // Outside the selection the base is untouched.
        assert_eq!(doc.composite_pixel(0, 0).0[3], 255);
        // The layer extracted before erasing is untouched.
        assert_eq!(doc.layers[index].image.get_pixel(0, 0).0[3], 255);
        assert_eq!(doc.composite_pixel(20, 2).0[3], 255);
    }

    #[test]
    fn erase_selection_clips_to_canvas_and_roundtrips_undo_redo() {
        let mut doc = Document::from_image(checker(16, 16));
        let original = doc.flatten();

        // Selection partially outside the canvas: only the inside part erases.
        doc.erase_selection(&Selection::Rect(Rect::new(12, 12, 20, 20)))
            .unwrap();
        assert_eq!(doc.composite_pixel(12, 12).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(15, 15).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(11, 11).0[3], 255);

        doc.undo().unwrap();
        assert_eq!(doc.flatten(), original);
        doc.redo().unwrap();
        assert_eq!(doc.composite_pixel(12, 12).0, [0, 0, 0, 0]);
    }

    #[test]
    fn erase_selection_rejects_empty_and_outside_selections() {
        let mut doc = Document::from_image(checker(16, 16));
        assert_eq!(
            doc.erase_selection(&Selection::Rect(Rect::new(0, 0, 0, 0))),
            Err(EditError::EmptySelection)
        );
        assert_eq!(
            doc.erase_selection(&Selection::Rect(Rect::new(40, 40, 4, 4))),
            Err(EditError::EmptySelection)
        );
        assert_eq!(doc.can_undo(), false);
    }

    #[test]
    fn erase_selection_composes_with_crop_view_mapping() {
        // Erase must map canvas coords through `view` after a crop.
        let mut doc = Document::from_image(ramp(32, 32));
        doc.crop(Rect::new(8, 8, 16, 16)).unwrap();
        doc.erase_selection(&Selection::Rect(Rect::new(0, 0, 4, 4)))
            .unwrap();
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 0]);
        // base pixel at (8,8) is what canvas (0,0) maps to.
        assert_eq!(doc.base.get_pixel(8, 8).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(4, 4).0, [12, 12, 0, 255]);
    }

    #[test]
    fn scale_layer_resamples_content_and_keeps_offset() {
        let mut doc = Document::from_image(checker(32, 32));
        let index = doc
            .add_layer(Layer::new("clip", checker(8, 4)))
            .unwrap();
        doc.translate_layer(index, 5, 3).unwrap();

        doc.scale_layer(index, Size::new(16, 8), (5, 3)).unwrap();
        let layer = &doc.layers[index];
        assert_eq!(layer.image.width(), 16);
        assert_eq!(layer.image.height(), 8);
        // Offset (top-left anchor) is untouched by scaling.
        assert_eq!(layer.offset, (5, 3));

        doc.undo().unwrap();
        assert_eq!(doc.layers[index].image.width(), 8);
        assert_eq!(doc.layers[index].image.height(), 4);
        assert_eq!(doc.layers[index].offset, (5, 3));
        doc.redo().unwrap();
        assert_eq!(doc.layers[index].image.width(), 16);
    }

    #[test]
    fn scale_layer_rejects_bad_size_and_index() {
        let mut doc = Document::from_image(checker(16, 16));
        let index = doc.add_layer(Layer::new("clip", checker(4, 4))).unwrap();
        let undo_depth = doc.history.undo.len();
        assert_eq!(
            doc.scale_layer(index, Size::new(0, 4), (0, 0)),
            Err(EditError::InvalidCanvasSize)
        );
        assert_eq!(
            doc.scale_layer(9, Size::new(4, 4), (0, 0)),
            Err(EditError::LayerIndex(9))
        );
        // Rejected calls must not touch the history stack.
        assert_eq!(doc.history.undo.len(), undo_depth);
        assert_eq!(doc.layers[index].image.width(), 4);
    }

    #[test]
    fn set_base_visible_roundtrips_and_affects_composite() {
        let mut doc = Document::from_image(checker(8, 8));
        doc.set_base_visible(false).unwrap();
        assert!(doc.base_hidden);
        // Hidden base composites as fully transparent.
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 0]);

        doc.undo().unwrap();
        assert!(!doc.base_hidden);
        assert_eq!(doc.composite_pixel(0, 0).0[3], 255);
        doc.redo().unwrap();
        assert!(doc.base_hidden);

        // Setting the same state again must not push a history entry.
        let depth = doc.history.undo.len();
        doc.set_base_visible(false).unwrap();
        assert_eq!(doc.history.undo.len(), depth);
        doc.set_base_visible(true).unwrap();
        assert_eq!(doc.history.undo.len(), depth + 1);
        assert!(!doc.base_hidden);
    }

    #[test]
    fn delete_base_requires_layers_and_roundtrips() {
        let mut doc = Document::from_image(checker(16, 16));
        // No layers yet: deleting the base is rejected.
        assert_eq!(doc.delete_base(), Err(EditError::BaseRequired));

        let index = doc.add_layer(Layer::new("top", checker(4, 4))).unwrap();
        doc.delete_base().unwrap();
        assert!(doc.base_deleted);
        assert_eq!(doc.base.width(), 16);
        assert_eq!(doc.base.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(doc.layers[index].image.width(), 4);

        doc.undo().unwrap();
        assert!(!doc.base_deleted);
        assert_eq!(doc.base.get_pixel(0, 0).0, [255, 0, 0, 255]);
        doc.redo().unwrap();
        assert!(doc.base_deleted);
        assert_eq!(doc.base.get_pixel(15, 15).0, [0, 0, 0, 0]);
    }

    #[test]
    fn convert_base_to_layer_roundtrips_and_composites_identically() {
        let mut doc = Document::from_image(checker(16, 16));
        let before = doc.flatten();

        doc.convert_base_to_layer().unwrap();
        assert!(doc.base_converted);
        // The base pixels now live in a regular bottom layer named 背景.
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].name, "背景");
        assert_eq!(doc.layers[0].offset, (0, 0));
        assert_eq!(doc.layers[0].image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert!(!doc.layers[0].locked);
        // The base itself is a transparent canvas of the same size; the
        // composite output is pixel-identical.
        assert_eq!(doc.base.width(), 16);
        assert_eq!(doc.base.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(doc.canvas.width, 16);
        assert_eq!(doc.flatten(), before);

        // Converting twice must not push a second history entry.
        let depth = doc.history.undo.len();
        doc.convert_base_to_layer().unwrap();
        assert_eq!(doc.history.undo.len(), depth);

        // The converted layer behaves like any other layer: movable and
        // deletable. After deletion no trace remains in the layer list.
        doc.translate_layer(0, 4, 2).unwrap();
        assert_eq!(doc.layers[0].offset, (4, 2));
        doc.remove_layer(0).unwrap();
        assert!(doc.layers.is_empty());
        assert_eq!(doc.flatten(), RgbaImage::new(16, 16));

        // Undo the deletion: the layer (still converted) comes back.
        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].offset, (4, 2));
        assert!(doc.base_converted);

        // Undo past translate AND the conversion restores the original
        // locked-in-base state exactly.
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert!(!doc.base_converted);
        assert!(doc.layers.is_empty());
        assert_eq!(doc.base.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(doc.flatten(), before);

        // Redo re-applies the conversion.
        doc.redo().unwrap();
        assert!(doc.base_converted);
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn move_layer_is_single_reorder_op() {        // Drag-reorder in the panel relies on move_layer(from, to) being one
        // undoable step that lands the layer exactly at `to`.
        let mut doc = Document::from_image(checker(16, 16));
        doc.add_layer(Layer::new("a", checker(2, 2))).unwrap();
        doc.add_layer(Layer::new("b", checker(2, 2))).unwrap();
        doc.add_layer(Layer::new("c", checker(2, 2))).unwrap();

        doc.move_layer(2, 0).unwrap();
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["c", "a", "b"]);

        doc.undo().unwrap();
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn rotate_four_times_is_identity() {
        let mut doc = Document::from_image(corner_marked(20, 10));
        let original = doc.flatten();
        for _ in 0..4 {
            doc.apply_transform(Transform::Rotate90Cw).unwrap();
        }
        assert_eq!(doc.canvas, Size::new(20, 10));
        assert_eq!(doc.flatten(), original);
    }

    #[test]
    fn rotate90_swaps_canvas_and_moves_marked_corner() {
        let mut doc = Document::from_image(corner_marked(20, 10));
        assert_eq!(doc.composite_pixel(0, 0).0, [255, 255, 255, 255]);

        doc.apply_transform(Transform::Rotate90Cw).unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.composite_pixel(9, 0).0, [255, 255, 255, 255]);
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 255]);

        doc.apply_transform(Transform::Rotate90Ccw).unwrap();
        assert_eq!(doc.canvas, Size::new(20, 10));
        assert_eq!(doc.composite_pixel(0, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn flips_are_self_inverse_and_move_marked_corner() {
        for transform in [Transform::FlipHorizontal, Transform::FlipVertical] {
            let mut doc = Document::from_image(corner_marked(16, 8));
            doc.apply_transform(transform).unwrap();
            let expected = match transform {
                Transform::FlipHorizontal => (15, 0),
                _ => (0, 7),
            };
            assert_eq!(doc.composite_pixel(expected.0, expected.1).0, [255, 255, 255, 255]);
            doc.apply_transform(transform).unwrap();
            assert_eq!(doc.composite_pixel(0, 0).0, [255, 255, 255, 255]);
        }
    }

    #[test]
    fn transform_moves_layers_and_undo_restores_everything() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            20,
            10,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (1, 1);
        doc.add_layer(layer).unwrap();

        doc.apply_transform(Transform::Rotate90Cw).unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[0].offset, (10 - (1 + 2), 1));
        assert_eq!(doc.composite_pixel(7, 1).0, [255, 255, 255, 255]);

        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(20, 10));
        assert_eq!(doc.layers[0].offset, (1, 1));
        assert_eq!(doc.composite_pixel(1, 1).0, [255, 255, 255, 255]);

        doc.redo().unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[0].offset, (7, 1));
    }

    #[test]
    fn rotate_ccw_moves_layer_to_expected_place() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            20,
            10,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (1, 1);
        doc.add_layer(layer).unwrap();

        doc.apply_transform(Transform::Rotate90Ccw).unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[0].offset, (1, 20 - 1 - 2));
        assert_eq!(doc.composite_pixel(1, 17).0, [255, 255, 255, 255]);
    }

    fn corner_marked(width: u32, height: u32) -> RgbaImage {
        let mut image = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
        image.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
        image
    }

    fn ramp(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([x as u8, y as u8, 0, 255])
        })
    }

    #[test]
    fn crop_shrinks_canvas_and_shifts_visible_region() {
        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();

        assert_eq!(doc.canvas, Size::new(20, 12));
        assert_eq!(doc.view, Rect::new(10, 6, 20, 12));
        assert_eq!(doc.base.width(), 64);
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 6, 0, 255]);
        assert_eq!(doc.composite_pixel(19, 11).0, [29, 17, 0, 255]);
    }

    #[test]
    fn crop_undo_and_redo_restore_canvas_and_pixels() {
        let mut doc = Document::from_image(ramp(64, 48));
        let original = doc.flatten();

        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(64, 48));
        assert_eq!(doc.flatten(), original);

        doc.redo().unwrap();
        assert_eq!(doc.canvas, Size::new(20, 12));
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 6, 0, 255]);
    }

    #[test]
    fn crop_composes_and_clamps_to_the_image() {
        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.crop(Rect::new(2, 2, 100, 100)).unwrap();

        assert_eq!(doc.canvas, Size::new(18, 10));
        assert_eq!(doc.view, Rect::new(12, 8, 18, 10));
        assert_eq!(doc.composite_pixel(0, 0).0, [12, 8, 0, 255]);

        let mut doc = Document::from_image(ramp(16, 16));
        assert_eq!(doc.crop(Rect::new(20, 20, 4, 4)), Err(EditError::EmptyCrop));
        assert_eq!(doc.crop(Rect::new(0, 0, 0, 4)), Err(EditError::EmptyCrop));
    }

    #[test]
    fn reset_crop_shows_the_full_image_again() {
        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.reset_crop().unwrap();

        assert_eq!(doc.canvas, Size::new(64, 48));
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(doc.composite_pixel(63, 47).0, [63, 47, 0, 255]);
        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(20, 12));
    }

    #[test]
    fn crop_moves_layers_relative_to_the_new_origin() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            40,
            30,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (2, 3);
        doc.add_layer(layer).unwrap();

        doc.crop(Rect::new(5, 5, 20, 20)).unwrap();
        assert_eq!(doc.canvas, Size::new(20, 20));
        // 图层坐标未改动，但视口左上角移到了 (5,5)，所以它落在画布外左侧/上方。
        assert_eq!(doc.layers[0].offset, (2, 3));
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 255]);

        doc.reset_crop().unwrap();
        assert_eq!(doc.composite_pixel(2, 3).0, [255, 255, 255, 255]);
    }

    #[test]
    fn crop_then_rotate_keeps_the_view_on_the_same_content() {
        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.apply_transform(Transform::Rotate90Cw).unwrap();

        assert_eq!(doc.canvas, Size::new(12, 20));
        assert_eq!(doc.view, Rect::new(48 - 6 - 12, 10, 12, 20));
        // 裁剪区左上角的内容在旋转后落到右上角。
        assert_eq!(doc.composite_pixel(11, 0).0, [10, 6, 0, 255]);
        assert_eq!(doc.composite_pixel(0, 19).0, [29, 17, 0, 255]);

        doc.apply_transform(Transform::Rotate90Ccw).unwrap();
        assert_eq!(doc.canvas, Size::new(20, 12));
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 6, 0, 255]);
    }

    fn solid(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, image::Rgba(color))
    }

    #[test]
    fn resize_sets_canvas_and_drops_crop() {
        let mut doc = Document::from_image(solid(64, 48, [10, 20, 30, 255]));
        doc.crop(Rect::new(8, 6, 32, 24)).unwrap();
        assert_eq!(doc.canvas, Size::new(32, 24));

        doc.resize(Size::new(16, 12)).unwrap();
        assert_eq!(doc.canvas, Size::new(16, 12));
        assert_eq!(doc.view, Rect::new(0, 0, 16, 12));
        assert_eq!(doc.base.dimensions(), (16, 12));
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn resize_upscale_grows_dimensions() {
        let mut doc = Document::from_image(solid(10, 5, [1, 2, 3, 255]));
        doc.resize(Size::new(30, 15)).unwrap();
        assert_eq!(doc.canvas, Size::new(30, 15));
        assert_eq!(doc.base.dimensions(), (30, 15));
        assert_eq!(doc.composite_pixel(29, 14).0, [1, 2, 3, 255]);
    }

    #[test]
    fn resize_undo_restores_base_view_and_layers() {
        let mut doc = Document::from_image(ramp(64, 48));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (6, 6);
        doc.add_layer(layer).unwrap();
        let original = doc.flatten();
        let original_layers = doc.layers.clone();

        doc.resize(Size::new(32, 24)).unwrap();
        assert_eq!(doc.canvas, Size::new(32, 24));

        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(64, 48));
        assert_eq!(doc.base.dimensions(), (64, 48));
        assert_eq!(doc.layers, original_layers);
        assert_eq!(doc.flatten(), original);

        doc.redo().unwrap();
        assert_eq!(doc.canvas, Size::new(32, 24));
    }

    #[test]
    fn resize_rejects_zero_dimensions() {
        let mut doc = Document::from_image(solid(16, 16, [0, 0, 0, 255]));
        assert_eq!(
            doc.resize(Size::new(0, 16)),
            Err(EditError::InvalidCanvasSize)
        );
        assert_eq!(
            doc.resize(Size::new(16, 0)),
            Err(EditError::InvalidCanvasSize)
        );
        assert_eq!(doc.canvas, Size::new(16, 16));
    }

    #[test]
    fn resize_scales_layer_offset_and_image() {
        let mut doc = Document::from_image(solid(40, 20, [0, 0, 0, 255]));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (10, 5);
        doc.add_layer(layer).unwrap();

        doc.resize(Size::new(20, 10)).unwrap();
        let l = &doc.layers[0];
        assert_eq!(l.offset, (5, 3));
        assert_eq!(l.image.dimensions(), (2, 2));
        assert_eq!(doc.composite_pixel(5, 3).0, [255, 255, 255, 255]);
    }

    #[test]
    fn canvas_resize_centers_content_without_resampling() {
        let mut doc = Document::from_image(ramp(8, 8));
        let original = doc.flatten();

        doc.resize_canvas(Size::new(12, 12)).unwrap();
        assert_eq!(doc.canvas, Size::new(12, 12));
        assert_eq!(doc.view, Rect::new(0, 0, 12, 12));
        assert_eq!(doc.base.dimensions(), (12, 12));
        // Old content is re-anchored to the center (offset +2/+2) and every
        // pixel is copied verbatim (no resampling).
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    doc.composite_pixel(x + 2, y + 2).0,
                    original.get_pixel(x as u32, y as u32).0
                );
            }
        }
        // Corners are transparent padding.
        assert_eq!(doc.composite_pixel(0, 0).0[3], 0);
        assert_eq!(doc.composite_pixel(11, 11).0[3], 0);
    }

    #[test]
    fn canvas_resize_shrink_clips_to_center_and_undoes() {
        let mut doc = Document::from_image(ramp(8, 8));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (0, 0);
        doc.add_layer(layer).unwrap();
        let original = doc.flatten();
        let original_layers = doc.layers.clone();

        doc.resize_canvas(Size::new(4, 4)).unwrap();
        assert_eq!(doc.canvas, Size::new(4, 4));
        // Composition re-anchored to the center: old (x+2, y+2) → new (x, y).
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(
                    doc.composite_pixel(x, y).0,
                    original.get_pixel((x + 2) as u32, (y + 2) as u32).0
                );
            }
        }
        // Layer shifted by (-2, -2); its pixels are untouched (still 2x2).
        assert_eq!(doc.layers[0].offset, (-2, -2));
        assert_eq!(doc.layers[0].image.dimensions(), (2, 2));

        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(8, 8));
        assert_eq!(doc.layers, original_layers);
        assert_eq!(doc.flatten(), original);
    }

    #[test]
    fn flatten_composites_translated_layer() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            16,
            16,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (5, 5);
        doc.add_layer(layer).unwrap();
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(5, 5).0, [255, 255, 255, 255]);
        assert_eq!(flat.get_pixel(0, 0).0, [0, 0, 0, 255]);
    }

    fn rect_mark(fill: bool) -> Mark {
        let mut m = Mark::new(MarkKind::Rect);
        m.rect = (2.0, 2.0, 10.0, 8.0);
        m.stroke = 2;
        m.color = [255, 0, 0, 255];
        m.fill = fill;
        m.fill_color = [0, 255, 0, 255];
        m
    }

    fn blank(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]))
    }

    #[test]
    fn rect_mark_renders_stroke_fill_and_history_roundtrips() {
        let mut doc = Document::from_image(blank(16, 16));
        let index = doc.add_mark(rect_mark(true)).unwrap();
        assert_eq!(index, 0);

        let flat = doc.flatten();
        // Stroke bands (2px) on the border, fill color in the interior.
        assert_eq!(flat.get_pixel(2, 2).0, [255, 0, 0, 255]);
        assert_eq!(flat.get_pixel(9, 3).0, [255, 0, 0, 255]);
        assert_eq!(flat.get_pixel(5, 5).0, [0, 255, 0, 255]);
        // Outside the mark: untouched.
        assert_eq!(flat.get_pixel(0, 0).0, [0, 0, 0, 255]);

        // Move + undo + redo roundtrip.
        let moved = doc.marks[0].translated(3.0, 1.0);
        doc.update_mark(0, moved.clone()).unwrap();
        assert_eq!(doc.marks[0].rect, (5.0, 3.0, 13.0, 9.0));
        doc.undo().unwrap();
        assert_eq!(doc.marks[0].rect, (2.0, 2.0, 10.0, 8.0));
        doc.redo().unwrap();
        assert_eq!(doc.marks[0].rect, (5.0, 3.0, 13.0, 9.0));

        // Delete + undo restores the exact mark.
        doc.delete_mark(0).unwrap();
        assert!(doc.marks.is_empty());
        doc.undo().unwrap();
        assert_eq!(doc.marks.len(), 1);
        assert_eq!(doc.marks[0], moved);
        // add→update→undo→redo left undo = [add, update]; three undos walk
        // back delete → update → add, leaving an empty (pre-add) state with
        // the whole chain replayable via redo.
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert!(doc.marks.is_empty() && doc.can_redo());
    }

    #[test]
    fn rect_mark_without_fill_keeps_interior_transparent() {
        let mut doc = Document::from_image(blank(16, 16));
        doc.add_mark(rect_mark(false)).unwrap();
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(5, 5).0, [0, 0, 0, 255]);
        assert_eq!(flat.get_pixel(2, 4).0, [255, 0, 0, 255]);
    }

    #[test]
    fn hidden_mark_is_skipped_and_undo_restores_it() {
        let mut doc = Document::from_image(blank(32, 32));
        let mark = rect_mark(false);
        doc.add_mark(mark.clone()).unwrap();
        // (3,3) sits in the 2px stroke ring of rect (2,2)-(10,8).
        assert_eq!(doc.flatten().get_pixel(3, 3).0, [255, 0, 0, 255]);

        let mut hidden = mark.clone();
        hidden.visible = false;
        doc.update_mark(0, hidden).unwrap();
        assert_eq!(doc.flatten().get_pixel(3, 3).0, [0, 0, 0, 255]);

        doc.undo().unwrap();
        assert_eq!(doc.flatten().get_pixel(3, 3).0, [255, 0, 0, 255]);
        assert!(doc.marks[0].visible);
    }

    #[test]
    fn ellipse_mark_paints_ring_and_interior() {
        let mut doc = Document::from_image(blank(32, 32));
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Ellipse);
            m.rect = (4.0, 4.0, 28.0, 28.0);
            m.stroke = 2;
            m.color = [255, 0, 0, 255];
            m.fill = true;
            m.fill_color = [0, 0, 255, 255];
            m
        })
        .unwrap();
        let flat = doc.flatten();
        // Center = fill; just inside the outline = stroke; outside = base.
        assert_eq!(flat.get_pixel(16, 16).0, [0, 0, 255, 255]);
        assert_eq!(flat.get_pixel(16, 5).0, [255, 0, 0, 255]);
        assert_eq!(flat.get_pixel(1, 1).0, [0, 0, 0, 255]);
    }

    #[test]
    fn arrow_mark_draws_shaft_and_head() {
        let mut doc = Document::from_image(blank(48, 24));
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Arrow);
            m.rect = (4.0, 12.0, 40.0, 12.0);
            m.points = vec![(4.0, 12.0), (40.0, 12.0)];
            m.stroke = 2;
            m.color = [255, 255, 0, 255];
            m
        })
        .unwrap();
        let flat = doc.flatten();
        // Shaft midpoint and inside the head triangle near the end point.
        assert_eq!(flat.get_pixel(20, 12).0, [255, 255, 0, 255]);
        assert_eq!(flat.get_pixel(38, 12).0, [255, 255, 0, 255]);
        assert_eq!(flat.get_pixel(20, 2).0, [0, 0, 0, 255]);
    }

    #[test]
    fn brush_mark_stamps_a_thick_line() {
        let mut doc = Document::from_image(blank(32, 32));
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Brush);
            m.rect = (4.0, 16.0, 28.0, 16.0);
            m.points = vec![(4.0, 16.0), (28.0, 16.0)];
            m.stroke = 4;
            m.color = [0, 255, 255, 255];
            m
        })
        .unwrap();
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(16, 16).0, [0, 255, 255, 255]);
        // 4px stroke → ±2px around the path (pixel centers at .5 offsets).
        assert_eq!(flat.get_pixel(16, 17).0, [0, 255, 255, 255]);
        assert_eq!(flat.get_pixel(16, 21).0, [0, 0, 0, 255]);
    }

    #[test]
    fn mosaic_mark_averages_block_content() {
        // Black base with a 2x2 white square at (8..10, 8..10): the cell
        // (8..12, 8..12) averages to floor((255*4 + 0*12) / 16) = 63.
        let mut base = blank(32, 32);
        for y in 8..10 {
            for x in 8..10 {
                base.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let mut doc = Document::from_image(base);
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Mosaic);
            m.rect = (8.0, 8.0, 12.0, 12.0);
            m.points = vec![(10.0, 10.0)];
            m.stroke = 8;
            m.cell = 4;
            m
        })
        .unwrap();
        let flat = doc.flatten();
        for y in 8..12 {
            for x in 8..12 {
                assert_eq!(flat.get_pixel(x, y).0, [63, 63, 63, 255]);
            }
        }
        // Outside the mosaic band: untouched.
        assert_eq!(flat.get_pixel(5, 5).0, [0, 0, 0, 255]);
        assert_eq!(flat.get_pixel(20, 20).0, [0, 0, 0, 255]);
    }

    #[test]
    fn mosaic_region_mode_pixelates_whole_rect() {
        // Empty points = region mode: the entire rect is pixelated, not a
        // stamp path. A 2px checkerboard averages to ~127 in every 4px cell.
        let mut base = blank(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let v = if ((x / 2 + y / 2) % 2) == 0 { 255u8 } else { 0u8 };
                base.put_pixel(x, y, image::Rgba([v, v, v, 255]));
            }
        }
        let mut doc = Document::from_image(base);
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Mosaic);
            m.rect = (8.0, 8.0, 24.0, 24.0);
            m.points = Vec::new();
            m.cell = 4;
            m
        })
        .unwrap();
        let flat = doc.flatten();
        // Every pixel inside the rect is a flat cell average (~127), i.e.
        // the hard checkerboard texture is gone.
        for y in 8..24 {
            for x in 8..24 {
                let p = flat.get_pixel(x, y).0;
                assert_eq!(p[0], p[1]);
                assert_eq!(p[1], p[2]);
                assert!(p[0] > 100 && p[0] < 155, "cell average at ({x},{y}): {p:?}");
            }
        }
        // Outside the rect: untouched (raw checkerboard extremes remain).
        assert_eq!(flat.get_pixel(4, 4).0, [255, 255, 255, 255]);
        assert_eq!(flat.get_pixel(30, 32).0, [0, 0, 0, 255]);
    }

    #[test]
    fn text_mark_rasterizes_colored_pixels() {
        let mut doc = Document::from_image(blank(160, 80));
        let (tw, th) = editcore_measures();
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Text);
            m.text = "测试Test".into();
            m.font = "msyh".into();
            m.font_size = 32.0;
            m.color = [0, 255, 0, 255];
            m.rect = (4.0, 4.0, 4.0 + tw, 4.0 + th);
            m
        })
        .unwrap();
        let flat = doc.flatten();
        let mut greens = 0;
        for y in 0..80 {
            for x in 0..160 {
                if flat.get_pixel(x, y).0 == [0, 255, 0, 255] {
                    greens += 1;
                }
            }
        }
        assert!(greens > 20, "expected anti-aliased ink, got {greens} px");
    }

    // Helper so the text test stays readable: natural size at 32px.
    fn editcore_measures() -> (f32, f32) {
        crate::font::measure_text("测试Test", "msyh", 32.0, false, false)
    }

    #[test]
    fn marks_follow_rotate_and_canvas_resize() {
        let mut doc = Document::from_image(blank(20, 10));
        doc.add_mark(rect_mark(false)).unwrap();

        doc.apply_transform(Transform::Rotate90Cw).unwrap();
        // Canvas is 10x20 now; the mark maps with the same point transform.
        let m = &doc.marks[0];
        assert_eq!(m.rect.0.min(m.rect.2), 2.0); // old x0 == new y0 side
        assert_eq!(doc.canvas, Size::new(10, 20));

        doc.undo().unwrap();
        let before = doc.marks[0].clone();
        doc.resize_canvas(Size::new(30, 20)).unwrap();
        // Center anchor: dx = (30-20)/2 = 5, dy = (20-10)/2 = 5.
        assert_eq!(doc.marks[0].rect, (7.0, 7.0, 15.0, 13.0));
        doc.undo().unwrap();
        assert_eq!(doc.marks[0], before);
    }

    #[test]
    fn resize_document_scales_marks() {
        let mut doc = Document::from_image(blank(20, 10));
        doc.add_mark(rect_mark(false)).unwrap();
        doc.resize(Size::new(40, 20)).unwrap();
        assert_eq!(doc.marks[0].rect, (4.0, 4.0, 20.0, 16.0));
        assert_eq!(doc.marks[0].stroke, 4);
    }
}
