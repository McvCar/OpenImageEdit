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
        let Some(r) = layer.raster() else { return };
        match r.transform.as_ref() {
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
            None => self.stamp_image(&r.image, r.offset.0, r.offset.1, threshold),
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

// ---- 透明区域当图层：洞的边 / 中心线 --------------------------------------------
//
// 「咬合」只在内容**装得下**空窗时生效；元素比洞大、或者只想让某条边压在洞壁
// 上时它帮不上忙。这里把「被不透明内容围住的透明区域」当成一个**虚拟图层**，
// 直接产出它的外接框，交给 `snap::hole_candidates` 变成六条边 / 中心线 ——
// 和真实图层的 `bounds()` 走同一条求解路径，于是「洞」也能当对齐基准用。

/// 粗扫网格边长上限（格）。大图按步长降采样，小图 1:1 —— 每层成本恒定在
/// ~2 万格以内，手势起手扫一圈几十层也就几毫秒。
const HOLE_GRID: u32 = 160;

#[inline]
fn alpha_at(raw: &[u8], iw: u32, x: u32, y: u32) -> u8 {
    raw[(y as usize * iw as usize + x as usize) * 4 + 3]
}

/// 源图里被不透明内容**围住**的透明区域外接框（源图像素坐标）。
///
/// 只收「洞」：连通块贴到图边界的直接丢掉 —— 那是透明边距 / 画布外空白，
/// 当成对齐线会让每个带 padding 的素材凭空多出四条线。
///
/// `limit` 是返回上限（按面积从大到小取）：小碎块对对齐没用，反而把候选表
/// 撑大、把参考线画得到处都是。
pub fn hole_rects(img: &RgbaImage, threshold: u8, limit: usize) -> Vec<(u32, u32, u32, u32)> {
    let (iw, ih) = img.dimensions();
    if iw < 4 || ih < 4 || limit == 0 {
        return Vec::new();
    }
    let raw = img.as_raw();
    let step = iw.max(ih).div_ceil(HOLE_GRID).max(1);
    let gw = iw.div_ceil(step);
    let gh = ih.div_ceil(step);
    let n = (gw * gh) as usize;
    // 0 = 未分类，1 = 墙（有采样点不透明），2 = 透明
    let mut cell = vec![0u8; n];
    let mut any_clear = false;
    for gy in 0..gh {
        let y0 = gy * step;
        let y1 = (y0 + step).min(ih);
        let ym = y0 + (y1 - y0) / 2;
        for gx in 0..gw {
            let x0 = gx * step;
            let x1 = (x0 + step).min(iw);
            let xm = x0 + (x1 - x0) / 2;
            // 五采样：只要有一个点不透明就当墙（保守：宁可洞小一点，也不要
            // 把墙旁边那点透明像素算成洞，让参考线飘在半空）。
            let wall = [(xm, ym), (x0, y0), (x1 - 1, y0), (x0, y1 - 1), (x1 - 1, y1 - 1)]
                .iter()
                .any(|&(px, py)| alpha_at(raw, iw, px, py) >= threshold);
            let idx = (gy * gw + gx) as usize;
            cell[idx] = if wall { 1 } else { 2 };
            any_clear |= !wall;
        }
    }
    if !any_clear {
        return Vec::new();
    }

    let mut seen = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut found: Vec<(u64, (u32, u32, u32, u32))> = Vec::new();
    for start in 0..n {
        if cell[start] != 2 || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.clear();
        stack.push(start as u32);
        let (mut minx, mut maxx, mut miny, mut maxy) = (gw, 0u32, gh, 0u32);
        let mut border = false;
        while let Some(idx) = stack.pop() {
            let cx = idx % gw;
            let cy = idx / gw;
            minx = minx.min(cx);
            maxx = maxx.max(cx);
            miny = miny.min(cy);
            maxy = maxy.max(cy);
            if cx == 0 || cy == 0 || cx + 1 == gw || cy + 1 == gh {
                border = true;
            }
            for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let nx = cx as i32 + dx;
                let ny = cy as i32 + dy;
                if nx < 0 || ny < 0 || nx >= gw as i32 || ny >= gh as i32 {
                    continue;
                }
                let ni = ny as usize * gw as usize + nx as usize;
                if cell[ni] == 2 && !seen[ni] {
                    seen[ni] = true;
                    stack.push(ni as u32);
                }
            }
        }
        if border {
            continue;
        }
        let rough = (
            minx * step,
            miny * step,
            ((maxx + 1) * step).min(iw) - minx * step,
            ((maxy + 1) * step).min(ih) - miny * step,
        );
        let exact = refine_hole(img, threshold, rough, step + 1);
        found.push(((exact.2 as u64) * (exact.3 as u64), exact));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.truncate(limit);
    found.into_iter().map(|(_, r)| r).collect()
}

/// 把粗网格给出的外接框精修到真实像素边界。
///
/// 只做四条**边带**扫描（带宽 = 粗网格步长）：粗框一定落在真实边界外侧
/// `step` 以内，所以每条边在自己的窄带里扫一遍就能拿到精确位置，代价是
/// O(带宽 × 边长)。早期版本用 1:1 洪水填充求连通块外接框，40 万像素的洞
/// 单层就要 11ms（手势起手会明显卡一下），而这里只需要外接框 —— 边带扫描
/// 精度相同、成本低一个量级。
///
/// 「洞」的判定（贴图边界的透明区域不算）已经在粗网格阶段完成，这里只负责
/// 把框推到像素级；因此不需要再做连通性检查。
fn refine_hole(
    img: &RgbaImage,
    threshold: u8,
    rough: (u32, u32, u32, u32),
    pad: u32,
) -> (u32, u32, u32, u32) {
    let (iw, ih) = img.dimensions();
    let raw = img.as_raw();
    let (rx, ry, rw, rh) = rough;
    let y0 = ry.max(0);
    let y1 = (ry + rh).min(ih);
    let x0 = rx.max(0);
    let x1 = (rx + rw).min(iw);
    if x1 <= x0 || y1 <= y0 {
        return rough;
    }
    // 某列在行带里有没有透明像素。
    let col_clear = |x: u32, ya: u32, yb: u32| {
        (ya..yb).any(|y| alpha_at(raw, iw, x, y) < threshold)
    };
    let row_clear = |y: u32, xa: u32, xb: u32| {
        (xa..xb).any(|x| alpha_at(raw, iw, x, y) < threshold)
    };
    // 左 / 右：从粗框往里收，第一条「有透明像素」的列就是洞壁后的第一列。
    let la = x0.saturating_sub(pad);
    let lb = (x1 + pad).min(iw);
    let left = (la..x1).find(|&x| col_clear(x, y0, y1)).unwrap_or(x0);
    let right = (left + 1..lb)
        .rev()
        .find(|&x| col_clear(x, y0, y1))
        .map(|x| x + 1)
        .unwrap_or(x1);
    // 上 / 下同理（用已经修好的左右范围，拐角才不会多算）。
    let ta = y0.saturating_sub(pad);
    let tb = (y1 + pad).min(ih);
    let top = (ta..y1).find(|&y| row_clear(y, left, right)).unwrap_or(y0);
    let bottom = (top + 1..tb)
        .rev()
        .find(|&y| row_clear(y, left, right))
        .map(|y| y + 1)
        .unwrap_or(y1);
    if right <= left || bottom <= top {
        return rough;
    }
    (left, top, right - left, bottom - top)
}

/// 一张位图里的洞，换算到画布坐标（`(ox, oy)` = 位图左上角的画布坐标）。
pub fn image_holes(
    img: &RgbaImage,
    (ox, oy): (f64, f64),
    threshold: u8,
    min_side: f64,
    limit: usize,
) -> Vec<(f64, f64, f64, f64)> {
    hole_rects(img, threshold, limit.max(1) * 4)
        .into_iter()
        .filter(|r| r.2 as f64 >= min_side && r.3 as f64 >= min_side)
        .take(limit)
        .map(|(x, y, w, h)| {
            (
                ox + x as f64,
                oy + y as f64,
                w as f64,
                h as f64,
            )
        })
        .collect()
}

/// 图层里的洞（画布坐标）。旋转 / 斜切的层直接返回空：洞框转过去就不再是
/// 轴对齐矩形，拿它当对齐线会飘。
pub fn layer_holes(
    layer: &Layer,
    threshold: u8,
    min_side: f64,
    limit: usize,
) -> Vec<(f64, f64, f64, f64)> {
    if !layer.visible || layer.opacity <= 0.0 || limit == 0 {
        return Vec::new();
    }
    let Some(r) = layer.raster() else {
        return Vec::new();
    };
    let img = match r.transform.as_ref() {
        Some(t) => &t.source,
        None => &r.image,
    };
    let rects = hole_rects(img, threshold, limit * 4);
    if rects.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(rects.len().min(limit));
    match r.transform.as_ref() {
        Some(t) => {
            if !t.is_affine()
                || t.rotation.abs() > 1e-6
                || t.skew.0 != 0.0
                || t.skew.1 != 0.0
            {
                return Vec::new();
            }
            let m = t.matrix();
            for (x, y, w, h) in rects {
                let (x, y, w, h) = (x as f64, y as f64, w as f64, h as f64);
                let pts = [
                    m.apply((x, y)),
                    m.apply((x + w, y)),
                    m.apply((x + w, y + h)),
                    m.apply((x, y + h)),
                ];
                let r = crate::layer_transform::RectF::from_points(&pts);
                if r.width >= min_side && r.height >= min_side {
                    out.push((r.x, r.y, r.width, r.height));
                    if out.len() >= limit {
                        break;
                    }
                }
            }
        }
        None => {
            for (x, y, w, h) in rects {
                if w as f64 >= min_side && h as f64 >= min_side {
                    out.push((
                        r.offset.0 as f64 + x as f64,
                        r.offset.1 as f64 + y as f64,
                        w as f64,
                        h as f64,
                    ));
                    if out.len() >= limit {
                        break;
                    }
                }
            }
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
        l.set_offset((0, 0));
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
        l.set_offset((0, 0));
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

    // ---- 透明区域当图层 --------------------------------------------------------

    /// 带透明边距 + 中间开洞的素材：只有洞里那块算洞，边距不算。
    fn padded_with_hole() -> RgbaImage {
        let mut img = blank(400, 300);
        fill(&mut img, 40, 30, 320, 240); // 内容 40..360 / 30..270，四周 30~40 透明
        for py in 120..210 {
            for px in 150..250 {
                img.put_pixel(px, py, image::Rgba([0, 0, 0, 0]));
            }
        }
        img
    }

    #[test]
    fn hole_rects_finds_interior_hole_and_ignores_padding() {
        let rs = hole_rects(&padded_with_hole(), ALPHA_THRESHOLD, 8);
        assert_eq!(rs.len(), 1, "{rs:?}");
        // 粗网格 ±step 出框，精修后必须严丝合缝。
        assert_eq!(rs[0], (150, 120, 100, 90));
    }

    #[test]
    fn hole_rects_is_empty_for_opaque_or_open_images() {
        let opaque = RgbaImage::from_pixel(200, 200, image::Rgba([255, 255, 255, 255]));
        assert!(hole_rects(&opaque, ALPHA_THRESHOLD, 8).is_empty());
        // 三面围住、一面朝图边界敞开：不是洞（那是背景/留白，会漏出去）。
        let mut open = blank(200, 200);
        fill(&mut open, 40, 40, 120, 120);
        for px in 40..160 {
            for py in 80..160 {
                open.put_pixel(px, py, image::Rgba([0, 0, 0, 0]));
            }
        }
        assert!(hole_rects(&open, ALPHA_THRESHOLD, 8).is_empty());
    }

    #[test]
    fn hole_rects_keeps_biggest_ones() {
        let mut img = RgbaImage::from_pixel(300, 300, image::Rgba([255, 255, 255, 255]));
        for py in 20..60 {
            for px in 20..60 {
                img.put_pixel(px, py, image::Rgba([0, 0, 0, 0]));
            }
        }
        for py in 100..260 {
            for px in 100..260 {
                img.put_pixel(px, py, image::Rgba([0, 0, 0, 0]));
            }
        }
        let rs = hole_rects(&img, ALPHA_THRESHOLD, 1);
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0], (100, 100, 160, 160), "先按面积取大的");
    }

    #[test]
    fn layer_holes_map_through_offset() {
        let mut l = Layer::new("t", padded_with_hole());
        l.set_offset((100, 50));
        let rs = layer_holes(&l, ALPHA_THRESHOLD, 8.0, 4);
        assert_eq!(rs.len(), 1);
        assert!((rs[0].0 - 250.0).abs() < 1e-9, "{rs:?}");
        assert!((rs[0].1 - 170.0).abs() < 1e-9, "{rs:?}");
        // 小过滤：洞小于 min_side 时不给候选。
        assert!(layer_holes(&l, ALPHA_THRESHOLD, 120.0, 4).is_empty());
    }
}

