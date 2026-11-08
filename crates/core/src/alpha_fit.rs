//! 「拼图咬合」吸附：把图层内容卡进附近图层内容之间的透明空档。
//!
//! 和 `snap` 的区别：`snap` 用的是**几何外框**（`layer.bounds()`），
//! 对带透明边距的素材（抠图、文字、图标）会吸到看不见的边上。这里改用
//! 真实像素占位：
//!
//! 1. 在移动图层内容框四周铺一张 1:1 的**不透明占位栅格**（窗口很小，
//!    只覆盖「内容框 + 阈值」），把附近图层的 alpha 采样进去；
//! 2. 按拖拽方向做投影：X 轴取内容高度带 `[y, y+h)`，逐列判断「有没有
//!    不透明像素」，连续的空列就是一条**空窗**；
//! 3. 空窗长度 ≥ 内容尺寸时才有意义：贴左墙 / 贴右墙 / 居中，取离当前
//!    位置最近的那个；只有偏差 ≤ 阈值（8 显示像素）才吸。
//!
//! 这样「两个字符之间刚好能塞下一个竖条」这类排版动作会自己咬住，
//! 而不需要手工对齐；空窗两侧都堵住时还能居中。
//!
//! 约定：移动图层只有**未旋转**（且无斜切 / 透视）时才走这条路 —— 旋转
//! 后的内容框不再是轴对齐矩形，投影会失去意义，调用方退回外框吸附。

use image::RgbaImage;

use crate::layer::Layer;
use crate::snap::{GuideLine, SnapAxis, SnapKind};

/// 不透明判定阈值（与 `LayerTransform::hit` 一致）。
pub const ALPHA_THRESHOLD: u8 = 8;

/// 窗口内的一张占位栅格（图像坐标，1:1 像素）。
///
/// `cells[i] != 0` 表示该像素被某个可见图层的不透明内容占据。
#[derive(Debug, Clone, Default)]
pub struct Occupancy {
    pub x0: i32,
    pub y0: i32,
    pub width: usize,
    pub height: usize,
    cells: Vec<u8>,
}

impl Occupancy {
    /// 按图像坐标矩形建（或复用）一张栅格。
    pub fn reset(&mut self, rect: (f64, f64, f64, f64)) {
        let x0 = rect.0.floor() as i32;
        let y0 = rect.1.floor() as i32;
        let w = (rect.2.ceil() as i64).clamp(1, 1 << 14) as usize;
        let h = (rect.3.ceil() as i64).clamp(1, 1 << 14) as usize;
        self.x0 = x0;
        self.y0 = y0;
        self.width = w;
        self.height = h;
        let need = w * h;
        if self.cells.len() < need {
            self.cells.resize(need, 0);
        }
        self.cells[..need].fill(0);
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    #[inline]
    fn set(&mut self, gx: i32, gy: i32) {
        let ix = gx - self.x0;
        let iy = gy - self.y0;
        if ix < 0 || iy < 0 || ix as usize >= self.width || iy as usize >= self.height {
            return;
        }
        self.cells[iy as usize * self.width + ix as usize] = 1;
    }

    /// 把一个图层的不透明像素盖进来。
    ///
    /// 有变换参数的层用 `inverse()` 逐像素反采样（旋转 / 缩放 / 镜像都对）；
    /// 已烘焙的层直接按 `offset` 1:1 拷贝。
    pub fn stamp(&mut self, layer: &Layer, threshold: u8) {
        if !layer.visible || layer.opacity <= 0.0 {
            return;
        }
        match layer.transform.as_ref() {
            Some(t) => {
                let Some(inv) = t.inverse() else { return };
                let b = t.bounds();
                let x0 = (b.x.floor() as i32).max(self.x0);
                let y0 = (b.y.floor() as i32).max(self.y0);
                let x1 = ((b.x + b.width).ceil() as i32).min(self.x0 + self.width as i32);
                let y1 = ((b.y + b.height).ceil() as i32).min(self.y0 + self.height as i32);
                if x1 <= x0 || y1 <= y0 {
                    return;
                }
                let (sw, sh) = (t.source.width(), t.source.height());
                for gy in y0..y1 {
                    for gx in x0..x1 {
                        let (u, v) = inv.apply((gx as f64 + 0.5, gy as f64 + 0.5));
                        if !(u >= 0.0 && v >= 0.0) {
                            continue;
                        }
                        let (iu, iv) = (u as u32, v as u32);
                        if iu >= sw || iv >= sh {
                            continue;
                        }
                        if t.source.get_pixel(iu, iv)[3] >= threshold {
                            self.set(gx, gy);
                        }
                    }
                }
            }
            None => self.stamp_image(&layer.image, layer.offset.0, layer.offset.1, threshold),
        }
    }

    /// 把一张位图的不透明像素盖进来，`(ox, oy)` 是它在画布坐标下的左上角。
    ///
    /// 背景（base）走这条：它的 `view` 原点就是画布原点，所以传
    /// `(-view.x, -view.y)`；被裁掉的部分不会被采样，裁切边自然成为一堵墙。
    pub fn stamp_image(&mut self, img: &RgbaImage, ox: i32, oy: i32, threshold: u8) {
        let (sw, sh) = img.dimensions();
        if sw == 0 || sh == 0 {
            return;
        }
        let x0 = ox.max(self.x0);
        let y0 = oy.max(self.y0);
        let x1 = (ox + sw as i32).min(self.x0 + self.width as i32);
        let y1 = (oy + sh as i32).min(self.y0 + self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        for gy in y0..y1 {
            for gx in x0..x1 {
                if img.get_pixel((gx - ox) as u32, (gy - oy) as u32)[3] >= threshold {
                    self.set(gx, gy);
                }
            }
        }
    }

    /// 窗口是否**整块被这张位图的不透明像素盖满**（只看 `area` 之内）。
    ///
    /// 用来判断「默认背景」该不该当墙：背景盖满整块画布时它只是垫底，
    /// 无条件盖进来会让空窗全部消失（每列都有像素），等于把咬合关掉；
    /// 只有背景真的透出空白（抠出的洞、画布留白、透明 PNG 的底）时才参与。
    ///
    /// `area` = 画布矩形：画布之外没有可咬的东西，窗口溢出的那部分不算
    /// 「露白」，否则「内容贴着画布边」会莫名其妙把咬合关掉。
    /// 按 4 像素步长采样（早退）：判断只是「有没有空白」，不需要逐像素。
    pub fn covered_by(
        &self,
        img: &RgbaImage,
        ox: i32,
        oy: i32,
        area: (f64, f64, f64, f64),
        threshold: u8,
    ) -> bool {
        const STEP: i32 = 4;
        let (sw, sh) = img.dimensions();
        if sw == 0 || sh == 0 {
            return false;
        }
        let x0 = self.x0.max(area.0.floor() as i32);
        let y0 = self.y0.max(area.1.floor() as i32);
        let x1 = (self.x0 + self.width as i32).min((area.0 + area.2).ceil() as i32);
        let y1 = (self.y0 + self.height as i32).min((area.1 + area.3).ceil() as i32);
        if x1 <= x0 || y1 <= y0 {
            return false;
        }
        // 画布内还有一块落在背景范围之外 → 那里是真实空白。
        if x0 < ox || y0 < oy || x1 > ox + sw as i32 || y1 > oy + sh as i32 {
            return false;
        }
        let mut gy = y0;
        while gy < y1 {
            let mut gx = x0;
            while gx < x1 {
                if img.get_pixel((gx - ox) as u32, (gy - oy) as u32)[3] < threshold {
                    return false;
                }
                gx += STEP;
            }
            gy += STEP;
        }
        true
    }

    #[inline]
    fn row_at(&self, gy: i32) -> Option<usize> {
        let iy = gy - self.y0;
        if iy < 0 || iy as usize >= self.height {
            None
        } else {
            Some(iy as usize)
        }
    }

    /// 某一列在行带 `[y0, y1)` 内是否有不透明像素。
    fn column_hit(&self, ix: usize, ya: usize, yb: usize) -> bool {
        let row = self.width;
        (ya..yb).any(|y| self.cells[y * row + ix] != 0)
    }

    /// 某一行在列带 `[x0, x1)` 内是否有不透明像素。
    fn row_hit(&self, iy: usize, xa: usize, xb: usize) -> bool {
        let row = self.width;
        self.cells[iy * row + xa..iy * row + xb].iter().any(|v| *v != 0)
    }

    /// 某个矩形内被占的像素数（用于「这块真的空着吗」的复核）。
    pub fn occupied_in(&self, rect: (f64, f64, f64, f64)) -> u32 {
        let x0 = (rect.0.floor() as i32).max(self.x0);
        let y0 = (rect.1.floor() as i32).max(self.y0);
        let x1 = ((rect.0 + rect.2).ceil() as i32).min(self.x0 + self.width as i32);
        let y1 = ((rect.1 + rect.3).ceil() as i32).min(self.y0 + self.height as i32);
        let mut n = 0;
        for gy in y0..y1 {
            let Some(iy) = self.row_at(gy) else { continue };
            for gx in x0..x1 {
                let ix = gx - self.x0;
                if ix >= 0 && (ix as usize) < self.width && self.cells[iy * self.width + ix as usize] != 0 {
                    n += 1;
                }
            }
        }
        n
    }
}

/// 一条空窗：图像坐标 `[start, end)`，两侧是否被不透明内容堵住。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gap {
    pub start: f64,
    pub end: f64,
    pub left_wall: bool,
    pub right_wall: bool,
}

impl Gap {
    pub fn len(self) -> f64 {
        self.end - self.start
    }
}

/// X 轴空窗：在行带 `[y0, y1)` 内按列投影。
pub fn gaps_x(occ: &Occupancy, y0: f64, y1: f64) -> Vec<Gap> {
    if occ.is_empty() {
        return Vec::new();
    }
    let ya = (y0.floor() as i32).max(occ.y0);
    let yb = ((y1.ceil() as i32).min(occ.y0 + occ.height as i32)).max(ya);
    let (ra, rb) = match (occ.row_at(ya), occ.row_at(yb - 1)) {
        (Some(a), Some(b)) => (a, b + 1),
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut run: Option<usize> = None;
    for ix in 0..occ.width {
        if occ.column_hit(ix, ra, rb) {
            if let Some(s) = run.take() {
                out.push(Gap {
                    start: (occ.x0 + s as i32) as f64,
                    end: (occ.x0 + ix as i32) as f64,
                    left_wall: s > 0,
                    right_wall: true,
                });
            }
        } else if run.is_none() {
            run = Some(ix);
        }
    }
    if let Some(s) = run {
        out.push(Gap {
            start: (occ.x0 + s as i32) as f64,
            end: (occ.x0 + occ.width as i32) as f64,
            left_wall: s > 0,
            right_wall: false,
        });
    }
    out
}

/// Y 轴空窗：在列带 `[x0, x1)` 内按行投影。
pub fn gaps_y(occ: &Occupancy, x0: f64, x1: f64) -> Vec<Gap> {
    if occ.is_empty() {
        return Vec::new();
    }
    let xa = (x0.floor() as i32).max(occ.x0);
    let xb = ((x1.ceil() as i32).min(occ.x0 + occ.width as i32)).max(xa);
    let ia = (xa - occ.x0).max(0) as usize;
    let ib = ((xb - occ.x0) as usize).min(occ.width);
    if ib <= ia {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut run: Option<usize> = None;
    for iy in 0..occ.height {
        if occ.row_hit(iy, ia, ib) {
            if let Some(s) = run.take() {
                out.push(Gap {
                    start: (occ.y0 + s as i32) as f64,
                    end: (occ.y0 + iy as i32) as f64,
                    left_wall: s > 0,
                    right_wall: true,
                });
            }
        } else if run.is_none() {
            run = Some(iy);
        }
    }
    if let Some(s) = run {
        out.push(Gap {
            start: (occ.y0 + s as i32) as f64,
            end: (occ.y0 + occ.height as i32) as f64,
            left_wall: s > 0,
            right_wall: false,
        });
    }
    out
}

/// 咬合结果。`dx/dy` 是要叠到当前位移上的修正量。
#[derive(Debug, Clone, Default)]
pub struct FitResult {
    pub dx: f64,
    pub dy: f64,
    pub guides: Vec<GuideLine>,
    pub snapped_x: bool,
    pub snapped_y: bool,
}

impl FitResult {
    pub fn is_empty(&self) -> bool {
        !self.snapped_x && !self.snapped_y
    }
}

/// 某一轴的候选位移：离当前最近的「贴墙 / 居中」位置。
fn axis_best(gaps: &[Gap], size: f64, cur: f64) -> Option<(f64, Vec<f64>)> {
    let mut best: Option<(f64, Vec<f64>)> = None;
    for g in gaps {
        // 装不下的空窗不参与：咬合的前提是内容真的能塞进去。
        if g.len() + 0.5 < size {
            continue;
        }
        let mut cands: Vec<(f64, Vec<f64>)> = Vec::with_capacity(3);
        if g.left_wall {
            cands.push((g.start, vec![g.start]));
        }
        if g.right_wall {
            cands.push((g.end - size, vec![g.end]));
        }
        if g.left_wall && g.right_wall {
            let mid = g.start + (g.len() - size) * 0.5;
            cands.push((mid, vec![g.start, g.end]));
        }
        for (p, walls) in cands {
            let d = (p - cur).abs();
            let better = match &best {
                None => true,
                Some((bp, _)) => d < (bp - cur).abs() - 1e-9,
            };
            if better {
                best = Some((p, walls));
            }
        }
    }
    best
}

/// 求解一次咬合。
///
/// * `moving` = 当前**内容框**（不是外框），图像坐标；
/// * `occ` = 附近图层的不透明占位；
/// * `threshold` = 已换算到图像像素的吸附阈值（`8 / disp_scale`）。
pub fn fit_into_gaps(moving: (f64, f64, f64, f64), occ: &Occupancy, threshold: f64) -> FitResult {
    let (x, y, w, h) = moving;
    if occ.is_empty() || !(threshold > 0.0) || !threshold.is_finite() || w <= 0.0 || h <= 0.0 {
        return FitResult::default();
    }

    let gx = gaps_x(occ, y, y + h);
    let gy = gaps_y(occ, x, x + w);
    let mut bx = axis_best(&gx, w, x);
    let mut by = axis_best(&gy, h, y);
    if let Some((p, _)) = &bx {
        if (*p - x).abs() > threshold {
            bx = None;
        }
    }
    if let Some((p, _)) = &by {
        if (*p - y).abs() > threshold {
            by = None;
        }
    }

    // 复核：投影只是在「行带 / 列带」上近似，两个轴同时落位后可能压到
    // 交叉区域的实体像素（L 形拐角）。真压到了就把偏差大的那个轴退掉。
    loop {
        if bx.is_none() && by.is_none() {
            return FitResult::default();
        }
        let dx = bx.as_ref().map(|(p, _)| p - x).unwrap_or(0.0);
        let dy = by.as_ref().map(|(p, _)| p - y).unwrap_or(0.0);
        let rect = (x + dx, y + dy, w, h);
        let area = (w.ceil() * h.ceil()).max(1.0);
        let tol = (area * 0.01).max(4.0);
        if (occ.occupied_in(rect) as f64) <= tol {
            break;
        }
        if dx.abs() >= dy.abs() {
            bx = None;
        } else {
            by = None;
        }
    }

    let mut out = FitResult::default();
    if let Some((p, walls)) = bx {
        out.dx = p - x;
        out.snapped_x = true;
        for wp in walls {
            out.guides.push(GuideLine {
                axis: SnapAxis::X,
                pos: wp,
                from: y,
                to: y + h,
                kind: SnapKind::GapEdge,
            });
        }
    }
    if let Some((p, walls)) = by {
        out.dy = p - y;
        out.snapped_y = true;
        for wp in walls {
            out.guides.push(GuideLine {
                axis: SnapAxis::Y,
                pos: wp,
                from: x,
                to: x + w,
                kind: SnapKind::GapEdge,
            });
        }
    }
    out
}

/// 不透明内容的最小包围盒（图像像素坐标）：`(x, y, w, h)`；全透明返回 `None`。
///
/// 16×16 分块先粗扫、再在边缘一圈块内精扫，避免每帧全图遍历大图层。
pub fn alpha_bbox(img: &RgbaImage, threshold: u8) -> Option<(u32, u32, u32, u32)> {
    const B: u32 = 16;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let bw = w.div_ceil(B);
    let bh = h.div_ceil(B);
    let mut min = (u32::MAX, u32::MAX);
    let mut max = (0u32, 0u32);
    let mut any = false;
    for by in 0..bh {
        let y0 = by * B;
        let y1 = (y0 + B).min(h);
        for bx in 0..bw {
            let x0 = bx * B;
            let x1 = (x0 + B).min(w);
            let mut hit = false;
            'block: for yy in y0..y1 {
                for xx in x0..x1 {
                    if img.get_pixel(xx, yy)[3] >= threshold {
                        hit = true;
                        break 'block;
                    }
                }
            }
            if hit {
                any = true;
                min = (min.0.min(bx), min.1.min(by));
                max = (max.0.max(bx), max.1.max(by));
            }
        }
    }
    if !any {
        return None;
    }
    // 边缘一圈块精扫：内部块必然完全落在包围盒里，不影响边界。
    let mut exact = (u32::MAX, u32::MAX, 0u32, 0u32);
    for by in min.1..=max.1 {
        for bx in min.0..=max.0 {
            let ring = bx == min.0 || bx == max.0 || by == min.1 || by == max.1;
            if !ring {
                continue;
            }
            let y0 = by * B;
            let y1 = (y0 + B).min(h);
            let x0 = bx * B;
            let x1 = (x0 + B).min(w);
            for yy in y0..y1 {
                for xx in x0..x1 {
                    if img.get_pixel(xx, yy)[3] >= threshold {
                        exact.0 = exact.0.min(xx);
                        exact.1 = exact.1.min(yy);
                        exact.2 = exact.2.max(xx);
                        exact.3 = exact.3.max(yy);
                    }
                }
            }
        }
    }
    Some((exact.0, exact.1, exact.2 - exact.0 + 1, exact.3 - exact.1 + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snap::SnapAxis;

    fn blank(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 0]))
    }

    fn fill(img: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32) {
        for py in y..y + h {
            for px in x..x + w {
                img.put_pixel(px, py, image::Rgba([255, 255, 255, 255]));
            }
        }
    }

    fn occ_with(img: &RgbaImage, rect: (f64, f64, f64, f64)) -> Occupancy {
        let mut occ = Occupancy::default();
        occ.reset(rect);
        let mut l = Layer::new("t", img.clone());
        l.offset = (0, 0);
        occ.stamp(&l, ALPHA_THRESHOLD);
        occ
    }

    #[test]
    fn alpha_bbox_ignores_transparent_padding() {
        let mut img = blank(100, 80);
        fill(&mut img, 30, 20, 10, 6);
        assert_eq!(alpha_bbox(&img, ALPHA_THRESHOLD), Some((30, 20, 10, 6)));
        assert_eq!(alpha_bbox(&blank(10, 10), ALPHA_THRESHOLD), None);
    }

    #[test]
    fn alpha_bbox_survives_block_edges() {
        // 正好压在 16 的整数倍上，验证分块粗扫 + 边缘精扫不会漏。
        let mut img = blank(64, 64);
        fill(&mut img, 15, 31, 2, 2);
        assert_eq!(alpha_bbox(&img, ALPHA_THRESHOLD), Some((15, 31, 2, 2)));
    }

    #[test]
    fn gaps_split_at_opaque_columns() {
        let mut img = blank(200, 100);
        fill(&mut img, 0, 40, 40, 20); // 左墙 0..40
        fill(&mut img, 120, 40, 40, 20); // 右墙 120..160
        let occ = occ_with(&img, (0.0, 0.0, 200.0, 100.0));
        let g = gaps_x(&occ, 45.0, 55.0);
        // 窗口两端也各算一条（无墙）
        let mid = g.iter().find(|g| g.start == 40.0).expect("应有 40..120 的空窗");
        assert_eq!(mid.end, 120.0);
        assert!(mid.left_wall && mid.right_wall);
    }

    #[test]
    fn fits_flush_when_only_one_wall_is_visible() {
        let mut img = blank(200, 100);
        fill(&mut img, 0, 40, 40, 20);
        let occ = occ_with(&img, (0.0, 0.0, 200.0, 100.0));
        // 内容 20 宽，当前左边在 44（离墙 4px），右墙在窗口外 → 贴左墙。
        let r = fit_into_gaps((44.0, 45.0, 20.0, 10.0), &occ, 8.0);
        assert!(r.snapped_x, "{r:?}");
        assert!((r.dx + 4.0).abs() < 1e-6, "{r:?}");
        assert_eq!(r.guides[0].axis, SnapAxis::X);
        assert!((r.guides[0].pos - 40.0).abs() < 1e-6);
    }

    #[test]
    fn centers_between_two_walls() {
        let mut img = blank(200, 100);
        fill(&mut img, 0, 0, 40, 100);
        fill(&mut img, 100, 0, 40, 100);
        let occ = occ_with(&img, (0.0, 0.0, 200.0, 100.0));
        // 空窗 40..100 = 60 宽，内容 20 → 居中放在 60。
        let r = fit_into_gaps((58.0, 10.0, 20.0, 20.0), &occ, 8.0);
        assert!(r.snapped_x);
        assert!((r.dx - 2.0).abs() < 1e-6, "{r:?}");
        assert_eq!(r.guides.len(), 2, "两侧墙都要出参考线");
    }

    #[test]
    fn refuses_when_content_does_not_fit() {
        let mut img = blank(200, 100);
        fill(&mut img, 0, 0, 40, 100);
        fill(&mut img, 60, 0, 40, 100);
        let occ = occ_with(&img, (0.0, 0.0, 200.0, 100.0));
        // 空窗只有 20 宽，内容 30 → 不咬合。
        let r = fit_into_gaps((42.0, 10.0, 30.0, 20.0), &occ, 8.0);
        assert!(!r.snapped_x, "{r:?}");
    }

    #[test]
    fn ignores_far_away_gaps() {
        let mut img = blank(400, 100);
        fill(&mut img, 0, 0, 40, 100);
        let occ = occ_with(&img, (0.0, 0.0, 400.0, 100.0));
        // 距离 40，远超阈值 8 → 不吸。
        let r = fit_into_gaps((84.0, 10.0, 20.0, 20.0), &occ, 8.0);
        assert!(!r.snapped_x, "{r:?}");
    }

    #[test]
    fn transparent_layer_occupies_nothing() {
        let img = blank(50, 50);
        let mut occ = Occupancy::default();
        occ.reset((0.0, 0.0, 50.0, 50.0));
        let mut l = Layer::new("t", img.clone());
        l.offset = (0, 0);
        occ.stamp(&l, ALPHA_THRESHOLD);
        assert_eq!(occ.occupied_in((0.0, 0.0, 50.0, 50.0)), 0);
    }

    /// 背景只剩「一块不透明的底 + 一个透明洞」时，洞壁要能当墙。
    fn base_with_hole() -> RgbaImage {
        let mut img = RgbaImage::from_pixel(400, 200, image::Rgba([255, 255, 255, 255]));
        for py in 0..200 {
            for px in 100..260 {
                img.put_pixel(px, py, image::Rgba([0, 0, 0, 0]));
            }
        }
        img
    }

    #[test]
    fn covered_by_distinguishes_backdrop_from_holed_base() {
        let opaque = RgbaImage::from_pixel(400, 200, image::Rgba([255, 255, 255, 255]));
        let canvas = (0.0, 0.0, 400.0, 200.0);
        let mut occ = Occupancy::default();
        occ.reset((120.0, 40.0, 60.0, 60.0));
        assert!(
            occ.covered_by(&opaque, 0, 0, canvas, ALPHA_THRESHOLD),
            "整块不透明的背景算垫底"
        );
        assert!(
            !occ.covered_by(&base_with_hole(), 0, 0, canvas, ALPHA_THRESHOLD),
            "洞落在窗口里不算垫底"
        );
        // 画布比背景宽 / 高：背景之外的那条边是真空白，要当墙。
        let mut out = Occupancy::default();
        out.reset((380.0, 40.0, 60.0, 60.0));
        assert!(
            !out.covered_by(&opaque, 0, 0, (0.0, 0.0, 500.0, 200.0), ALPHA_THRESHOLD),
            "画布留白不算垫底"
        );
        // 反过来，窗口溢出画布之外的部分不算露白（否则贴画布边就没吸附了）。
        assert!(
            out.covered_by(&opaque, 0, 0, canvas, ALPHA_THRESHOLD),
            "画布之外的部分忽略"
        );
    }

    #[test]
    fn base_hole_walls_are_snappable() {
        let mut occ = Occupancy::default();
        occ.reset((40.0, 0.0, 260.0, 200.0));
        occ.stamp_image(&base_with_hole(), 0, 0, ALPHA_THRESHOLD);
        // 洞 100..260，内容 60 宽，左边放在 98（离洞左壁 2px）→ 咬到 100。
        let r = fit_into_gaps((98.0, 20.0, 60.0, 60.0), &occ, 8.0);
        assert!(r.snapped_x, "{r:?}");
        assert!((r.dx - 2.0).abs() < 1e-6, "{r:?}");
        assert!((r.guides[0].pos - 100.0).abs() < 1e-6, "{r:?}");
    }
}
