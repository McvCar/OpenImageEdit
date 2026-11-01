// crates/core/src/selection.rs
//!
//! 选区真值：alpha 覆盖图（PS 语义）。
//!
//! 矢量形状（矩形 / 椭圆 / 多边形）只是画布上的操作输入，落地即刻光栅化。
//! 原因很直接：反选（带洞）、边界（距离场）、平滑（形态学）、羽化（模糊）
//! 全是像素域算子，用矢量表达会立刻退化 —— Photoshop 内部同样是一张
//! 8-bit mask。
//!
//! 坐标一律是「画布坐标」（= 用户看到的图像像素坐标，和 mark.rect / 图层
//! offset 同一套）。`bounds` 只覆盖有意义的那块，整幅反选才铺满画布，
//! 免得每个矩形选区都吃一张全画布内存。

use super::*;
use std::collections::HashMap;

/// 选区的矢量来源。自由套索与多边形套索在内部完全同构（都是闭合多边形），
/// 只有交互层区分它们。
#[derive(Debug, Clone, PartialEq)]
pub enum SelShape {
    Rect(Rect),
    Ellipse(Rect),
    Polygon(Vec<(f32, f32)>),
}

impl SelShape {
    pub fn bounds(&self) -> Rect {
        match self {
            SelShape::Rect(r) | SelShape::Ellipse(r) => *r,
            SelShape::Polygon(pts) => polygon_bounds(pts),
        }
    }
}

/// 选区覆盖图。`alpha[i]` 是该像素的选中程度（0..=255），逐像素按像素
/// 中心采样，和 `Selection::contains` 的几何语义保持一致。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SelectionMask {
    pub bounds: Rect,
    pub alpha: Vec<u8>,
}

/// 选区上面叠加的「可回改参数」。PS 里边界 / 平滑是点一次就烘焙的一次性
/// 操作，但右键菜单是内联数字框（可以反复上下点），所以这里存成参数、
/// 每次都从 `base` 重算 —— 否则连点两下"平滑 4"会叠加成两次平滑。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SelOps {
    /// 边界宽度（图像像素，环带总宽）。
    pub border: u32,
    /// 平滑半径（图像像素）。
    pub smooth: u32,
    /// 羽化半径（图像像素）。只影响后续编辑的软边，不改轮廓。
    pub feather: u32,
}

impl SelectionMask {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty() || self.area() == 0
    }

    /// 选中像素数（按 128 阈值算，与轮廓/命中判定一致）。
    pub fn area(&self) -> u64 {
        if self.bounds.is_empty() || self.alpha.len() < self.bounds.pixels() as usize {
            return 0;
        }
        self.alpha.iter().filter(|&&a| a >= 128).count() as u64
    }

    /// 画布坐标处的覆盖度（画布外恒为 0）。
    pub fn coverage_at(&self, x: i32, y: i32) -> u8 {
        let b = self.bounds;
        if x < b.x || y < b.y || x >= b.right() || y >= b.bottom() {
            return 0;
        }
        let ix = (x - b.x) as usize;
        let iy = (y - b.y) as usize;
        let w = b.width as usize;
        match self.alpha.get(iy * w + ix) {
            Some(&a) => a,
            None => 0,
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.coverage_at(x.floor() as i32, y.floor() as i32) >= 128
    }

    // ---- 光栅化 ----------------------------------------------------------

    pub fn from_shape(shape: &SelShape) -> Self {
        match shape {
            SelShape::Rect(r) => Self::rect(*r),
            SelShape::Ellipse(r) => Self::ellipse(*r),
            SelShape::Polygon(pts) => Self::polygon(pts),
        }
    }

    pub fn rect(r: Rect) -> Self {
        if r.is_empty() {
            return Self::empty();
        }
        Self {
            bounds: r,
            alpha: vec![255u8; r.pixels() as usize],
        }
    }

    pub fn ellipse(r: Rect) -> Self {
        if r.is_empty() {
            return Self::empty();
        }
        let (w, h) = (r.width as usize, r.height as usize);
        let mut alpha = vec![0u8; w * h];
        let cx = r.x as f32 + r.width as f32 / 2.0;
        let cy = r.y as f32 + r.height as f32 / 2.0;
        let rx = (r.width as f32 / 2.0).max(0.5);
        let ry = (r.height as f32 / 2.0).max(0.5);
        for y in 0..h {
            let py = r.y as f32 + y as f32 + 0.5;
            let dy = (py - cy) / ry;
            for x in 0..w {
                let px = r.x as f32 + x as f32 + 0.5;
                let dx = (px - cx) / rx;
                if dx * dx + dy * dy <= 1.0 {
                    alpha[y * w + x] = 255;
                }
            }
        }
        Self {
            bounds: r,
            alpha,
        }
    }

    /// 扫描线奇偶填充。取样规则与 `Selection::contains` 的射线法一致
    /// （像素中心、半开区间），所以两条路径的像素判定可以互相对照测试。
    pub fn polygon(points: &[(f32, f32)]) -> Self {
        if points.len() < 3 {
            return Self::empty();
        }
        let b = polygon_bounds(points);
        if b.is_empty() {
            return Self::empty();
        }
        let (w, h) = (b.width as usize, b.height as usize);
        let mut alpha = vec![0u8; w * h];
        let n = points.len();
        let mut xs: Vec<f32> = Vec::with_capacity(8);
        for y in 0..h {
            let py = b.y as f32 + y as f32 + 0.5;
            xs.clear();
            for i in 0..n {
                let (xi, yi) = points[i];
                let (xj, yj) = points[(i + n - 1) % n];
                if (yi > py) != (yj > py) {
                    xs.push(xj + (py - yj) * (xi - xj) / (yi - yj));
                }
            }
            if xs.len() < 2 {
                continue;
            }
            xs.sort_by(|a, c| a.partial_cmp(c).unwrap_or(std::cmp::Ordering::Equal));
            let mut k = 0;
            while k + 1 < xs.len() {
                let x0 = xs[k] - b.x as f32 - 0.5;
                let x1 = xs[k + 1] - b.x as f32 - 0.5;
                let start = (x0.ceil() as i32).max(0);
                let end = (x1.ceil() as i32).min(w as i32);
                for x in start..end {
                    alpha[y * w + x as usize] = 255;
                }
                k += 2;
            }
        }
        Self { bounds: b, alpha }
    }

    // ---- 集合运算 --------------------------------------------------------

    pub fn invert(&self, canvas: Size) -> Self {
        if canvas.width == 0 || canvas.height == 0 {
            return Self::empty();
        }
        let (w, h) = (canvas.width as usize, canvas.height as usize);
        let mut alpha = vec![255u8; w * h];
        let bw = self.bounds.width as usize;
        for y in 0..self.bounds.height as usize {
            for x in 0..bw {
                let gx = self.bounds.x + x as i32;
                let gy = self.bounds.y + y as i32;
                if gx < 0 || gy < 0 || gx >= w as i32 || gy >= h as i32 {
                    continue;
                }
                let c = self.alpha.get(y * bw + x).copied().unwrap_or(0);
                alpha[gy as usize * w + gx as usize] = 255 - c;
            }
        }
        Self {
            bounds: Rect::new(0, 0, canvas.width, canvas.height),
            alpha,
        }
    }

    fn combine(&self, other: &Self, f: impl Fn(u8, u8) -> u8) -> Self {
        let b = self.bounds.union(other.bounds);
        if b.is_empty() {
            return Self::empty();
        }
        let (w, h) = (b.width as usize, b.height as usize);
        let mut alpha = vec![0u8; w * h];
        for y in 0..h {
            let gy = b.y + y as i32;
            for x in 0..w {
                let gx = b.x + x as i32;
                alpha[y * w + x] = f(self.coverage_at(gx, gy), other.coverage_at(gx, gy));
            }
        }
        // bounds 是「两框并集」，结果却可能远小于它（减选 / 相交尤甚）。
        // 不收紧的话，QML 侧按 bounds 显示的尺寸徽标会是错的（蚂蚁线本身
        // 走 alpha，不受影响，于是更难看懂）。
        Self {
            bounds: b,
            alpha,
        }
        .crop_to_content()
    }

    pub fn union(&self, other: &Self) -> Self {
        self.combine(other, |a, b| a.max(b))
    }

    pub fn intersect(&self, other: &Self) -> Self {
        self.combine(other, |a, b| a.min(b))
    }

    pub fn subtract(&self, other: &Self) -> Self {
        self.combine(other, |a, b| a.saturating_sub(b))
    }

    /// 裁到画布内（`bounds` 里可能因为边界扩展伸到画布外）。
    pub fn clipped(&self, canvas: Size) -> Self {
        let full = Rect::new(0, 0, canvas.width, canvas.height);
        let Some(b) = self.bounds.intersect(full) else {
            return Self::empty();
        };
        if b == self.bounds {
            return self.clone();
        }
        let (w, h) = (b.width as usize, b.height as usize);
        let mut alpha = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                alpha[y * w + x] = self.coverage_at(b.x + x as i32, b.y + y as i32);
            }
        }
        Self { bounds: b, alpha }
    }

    pub fn translated(&self, dx: i32, dy: i32) -> Self {
        Self {
            bounds: self.bounds.translated(dx, dy),
            alpha: self.alpha.clone(),
        }
    }

    // ---- 算子 ------------------------------------------------------------

    /// 边界：沿原边缘生成总宽 `k` 的环（内外各 k/2），PS 的「边界」语义。
    /// 精确欧氏距离变换，环宽均匀（不随方向变窄）。
    pub fn border(&self, k: u32) -> Self {
        if k == 0 || self.is_empty() {
            return self.clone();
        }
        let m = (k as f32 / 2.0).ceil() as i32 + 1;
        let b = self.bounds;
        let (w, h) = ((b.width as i32 + 2 * m) as usize, (b.height as i32 + 2 * m) as usize);
        let bw = b.width as usize;
        let bh = b.height as usize;
        let mut inside = vec![false; w * h];
        for y in 0..bh {
            for x in 0..bw {
                if self.alpha.get(y * bw + x).copied().unwrap_or(0) >= 128 {
                    inside[(y + m as usize) * w + (x + m as usize)] = true;
                }
            }
        }
        // 到最近「未选中」像素的距离（给内部像素用）
        let mut cost = vec![FAR; w * h];
        for (i, c) in cost.iter_mut().enumerate() {
            if !inside[i] {
                *c = 0.0;
            }
        }
        let dist_to_outside = distance_transform(&cost, w, h);
        // 到最近「选中」像素的距离（给外部像素用）
        let mut cost2 = vec![FAR; w * h];
        for (i, c) in cost2.iter_mut().enumerate() {
            if inside[i] {
                *c = 0.0;
            }
        }
        let dist_to_inside = distance_transform(&cost2, w, h);

        let lim = k as f64 / 2.0;
        let lim2 = lim * lim;
        let mut alpha = vec![0u8; w * h];
        for i in 0..w * h {
            let d2 = if inside[i] {
                dist_to_outside[i]
            } else {
                dist_to_inside[i]
            };
            if d2 <= lim2 {
                alpha[i] = 255;
            }
        }
        let bounds = Rect::new(b.x - m, b.y - m, w as u32, h as u32);
        Self { bounds, alpha }.crop_to_content()
    }

    /// 平滑：抹掉边缘锯齿。模糊后阈值化（保留硬边，只圆角化）。
    pub fn smooth(&self, k: u32) -> Self {
        if k == 0 || self.is_empty() {
            return self.clone();
        }
        let (w, h) = (self.bounds.width as usize, self.bounds.height as usize);
        if w == 0 || h == 0 {
            return self.clone();
        }
        let r = k.max(1) as usize;
        // 复制边缘：贴住包围盒的边不被"啃掉"（平滑只该圆角，不该缩边）。
        let v = blur3_padded(&self.alpha, w, h, r, r, true);
        let alpha = v
            .iter()
            .map(|&x| if x >= 128.0 { 255u8 } else { 0u8 })
            .collect();
        Self {
            bounds: self.bounds,
            alpha,
        }
        .crop_to_content()
    }

    /// 羽化：软边。**不阈值化**，只把 alpha 抹成渐变；轮廓（bounds）不变，
    /// 所以蚂蚁线位置不动 —— 与 PS 一致。
    pub fn feathered(&self, r: u32) -> Self {
        if r == 0 || self.is_empty() {
            return self.clone();
        }
        let (w, h) = (self.bounds.width as usize, self.bounds.height as usize);
        if w == 0 || h == 0 {
            return self.clone();
        }
        // 半径取 r/2，让 50% 线大致落在原边缘上，过渡带落在 ±r 内。
        let rb = (r as usize / 2).max(1);
        // 补 3 倍半径的空白：过渡带必须能"淡出到 0"，否则包围盒边缘
        // 会被复制填充顶住，羽化看起来完全没生效。
        let v = blur3_padded(&self.alpha, w, h, rb, rb * 3 + 2, false);
        let alpha = v
            .iter()
            .map(|&x| x.round().clamp(0.0, 255.0) as u8)
            .collect();
        Self {
            bounds: self.bounds,
            alpha,
        }
    }

    /// 收掉四周全空的行/列（边界扩展、集合运算后必然出现）。
    fn crop_to_content(&self) -> Self {
        let bw = self.bounds.width as usize;
        let bh = self.bounds.height as usize;
        if bw == 0 || bh == 0 || self.alpha.len() < bw * bh {
            return Self::empty();
        }
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for y in 0..bh {
            let mut any = false;
            for x in 0..bw {
                if self.alpha[y * bw + x] != 0 {
                    any = true;
                    if x < x0 {
                        x0 = x;
                    }
                    if x > x1 {
                        x1 = x;
                    }
                }
            }
            if any {
                if y < y0 {
                    y0 = y;
                }
                y1 = y;
            }
        }
        if x0 == usize::MAX {
            return Self::empty();
        }
        let nw = x1 - x0 + 1;
        let nh = y1 - y0 + 1;
        if nw == bw && nh == bh && x0 == 0 && y0 == 0 {
            return self.clone();
        }
        let mut alpha = vec![0u8; nw * nh];
        for y in 0..nh {
            for x in 0..nw {
                alpha[y * nw + x] = self.alpha[(y + y0) * bw + (x + x0)];
            }
        }
        Self {
            bounds: Rect::new(
                self.bounds.x + x0 as i32,
                self.bounds.y + y0 as i32,
                nw as u32,
                nh as u32,
            ),
            alpha,
        }
    }

    // ---- 轮廓（蚂蚁线） --------------------------------------------------

    /// 提闭合轮廓，画布坐标。`step` > 1 时按格子降采样（大图省时间），
    /// 轮廓点会吸附到格子边界上。
    ///
    /// 做法：把 mask 二值化成格子，收集「选中格 ↔ 未选中格」之间的单位边，
    /// 再把边首尾相接串成闭合环（外环 + 洞都能拿到）。比 Marching Squares
    /// 好写，且天然支持多连通区域。
    pub fn outline(&self, step: u32) -> Vec<Vec<(f32, f32)>> {
        if self.is_empty() {
            return Vec::new();
        }
        let step = step.max(1) as usize;
        let b = self.bounds;
        let bw = b.width as usize;
        let bh = b.height as usize;
        let gw = (bw + step - 1) / step;
        let gh = (bh + step - 1) / step;
        if gw == 0 || gh == 0 {
            return Vec::new();
        }
        let inside = |cx: i32, cy: i32| -> bool {
            if cx < 0 || cy < 0 || cx >= gw as i32 || cy >= gh as i32 {
                return false;
            }
            let px = (cx as usize * step + step / 2).min(bw - 1);
            let py = (cy as usize * step + step / 2).min(bh - 1);
            self.alpha.get(py * bw + px).copied().unwrap_or(0) >= 128
        };

        let mut edges: Vec<((i32, i32), (i32, i32))> = Vec::new();
        for cy in 0..gh as i32 {
            for cx in 0..gw as i32 {
                if !inside(cx, cy) {
                    continue;
                }
                // 顺时针一圈：上 → 右 → 下 → 左
                if !inside(cx, cy - 1) {
                    edges.push(((cx, cy), (cx + 1, cy)));
                }
                if !inside(cx + 1, cy) {
                    edges.push(((cx + 1, cy), (cx + 1, cy + 1)));
                }
                if !inside(cx, cy + 1) {
                    edges.push(((cx + 1, cy + 1), (cx, cy + 1)));
                }
                if !inside(cx - 1, cy) {
                    edges.push(((cx, cy + 1), (cx, cy)));
                }
            }
        }
        if edges.is_empty() {
            return Vec::new();
        }

        let mut outgoing: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, (a, _)) in edges.iter().enumerate() {
            outgoing.entry(*a).or_default().push(i);
        }
        let mut used = vec![false; edges.len()];
        let mut paths: Vec<Vec<(f32, f32)>> = Vec::new();
        for start_idx in 0..edges.len() {
            if used[start_idx] {
                continue;
            }
            let start = edges[start_idx].0;
            let mut ring: Vec<(i32, i32)> = Vec::new();
            let mut cur = start;
            loop {
                let Some(list) = outgoing.get_mut(&cur) else {
                    break;
                };
                let Some(&next_idx) = list.iter().find(|&&i| !used[i]) else {
                    break;
                };
                used[next_idx] = true;
                ring.push(cur);
                cur = edges[next_idx].1;
                if cur == start {
                    break;
                }
            }
            if ring.len() < 3 {
                continue;
            }
            let simplified = simplify_ring(&ring);
            let pts: Vec<(f32, f32)> = simplified
                .iter()
                .map(|(cx, cy)| {
                    (
                        (b.x + (cx * step as i32)) as f32,
                        (b.y + (cy * step as i32)) as f32,
                    )
                })
                .collect();
            paths.push(pts);
        }
        paths
    }
}

/// 选区状态 = 矢量光栅化出来的基准 mask + 可回改的参数。
///
/// 关键点：`mask` 永远从 `base` 重新算，所以内联数字框反复改同一个参数
/// 不会叠加；换一个参数（比如先改边界再改平滑）也是从同一个基准重算。
#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    pub base: SelectionMask,
    pub ops: SelOps,
    /// `base` 应用 border/smooth 后的结果（不含羽化）。
    mask: SelectionMask,
}

impl SelectionState {
    pub fn new(base: SelectionMask) -> Self {
        let mut s = Self::default();
        s.set_base(base);
        s
    }

    /// 换一个选区：参数一并归零（PS 里新选区不继承上一次的一次性操作）。
    pub fn set_base(&mut self, base: SelectionMask) {
        self.base = base;
        self.ops = SelOps::default();
        self.recompute();
    }

    pub fn set_border(&mut self, k: u32) {
        self.ops.border = k;
        self.recompute();
    }

    pub fn set_smooth(&mut self, k: u32) {
        self.ops.smooth = k;
        self.recompute();
    }

    pub fn set_feather(&mut self, r: u32) {
        self.ops.feather = r;
    }

    /// 平移整块选区（拖动选区内部）。平移 base 再重算，边界/平滑/羽化
    /// 参数保持不变 —— 这几个算子都是平移无关的。
    pub fn translate(&mut self, dx: i32, dy: i32, canvas: Size) {
        self.base = self.base.translated(dx, dy).clipped(canvas);
        self.recompute();
    }

    fn recompute(&mut self) {
        let mut m = self.base.clone();
        if self.ops.border > 0 {
            m = m.border(self.ops.border);
        }
        if self.ops.smooth > 0 {
            m = m.smooth(self.ops.smooth);
        }
        self.mask = m;
    }

    /// 轮廓 / 手柄 / 命中判定用的 mask（不含羽化 —— 羽化后蚂蚁线不动）。
    pub fn mask(&self) -> &SelectionMask {
        &self.mask
    }

    pub fn mask_mut(&mut self) -> &mut SelectionMask {
        &mut self.mask
    }

    /// 真正做像素操作用的 mask：羽化后的软边。
    pub fn effective(&self) -> SelectionMask {
        if self.ops.feather > 0 {
            self.mask.feathered(self.ops.feather)
        } else {
            self.mask.clone()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.mask.is_empty()
    }

    /// 把 `mask` 的当前结果定为新基准（集合运算 / 平移之后调用）。
    pub fn commit_current(&mut self) {
        self.base = self.mask.clone();
        self.ops = SelOps::default();
        self.recompute();
    }
}

impl From<&Selection> for SelectionMask {
    fn from(sel: &Selection) -> Self {
        match sel {
            Selection::Rect(r) => SelectionMask::rect(*r),
            Selection::Ellipse(r) => SelectionMask::ellipse(*r),
            Selection::Lasso(pts) => SelectionMask::polygon(pts),
        }
    }
}

impl From<Selection> for SelectionMask {
    fn from(sel: Selection) -> Self {
        SelectionMask::from(&sel)
    }
}

fn polygon_bounds(points: &[(f32, f32)]) -> Rect {
    if points.is_empty() {
        return Rect::default();
    }
    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for (x, y) in points {
        min_x = min_x.min(*x);
        min_y = min_y.min(*y);
        max_x = max_x.max(*x);
        max_y = max_y.max(*y);
    }
    let x0 = min_x.floor() as i32;
    let y0 = min_y.floor() as i32;
    let x1 = max_x.ceil() as i32;
    let y1 = max_y.ceil() as i32;
    Rect::new(x0, y0, (x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32)
}

/// 去掉共线点，矩形选区就只剩 4 个角。
fn simplify_ring(ring: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let n = ring.len();
    if n < 3 {
        return ring.to_vec();
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let prev = ring[(i + n - 1) % n];
        let cur = ring[i];
        let next = ring[(i + 1) % n];
        let cross = (cur.0 - prev.0) * (next.1 - cur.1) - (cur.1 - prev.1) * (next.0 - cur.0);
        if cross != 0 {
            out.push(cur);
        }
    }
    if out.len() < 3 {
        ring.to_vec()
    } else {
        out
    }
}

// ---- 图像算法 ------------------------------------------------------------

/// 精确欧氏距离变换（Felzenszwalb & Huttenlocher, 两遍一维下包络）。
/// 输入 `cost`：源点为 0、其余 INF；输出平方距离。
fn distance_transform(cost: &[f64], w: usize, h: usize) -> Vec<f64> {
    let mut out = cost.to_vec();
    if w == 0 || h == 0 {
        return out;
    }
    let n = w.max(h);
    let mut f = vec![0.0f64; n];
    let mut d = vec![0.0f64; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];

    for x in 0..w {
        for y in 0..h {
            f[y] = out[y * w + x];
        }
        edt_1d(&f[..h], &mut d[..h], &mut v[..h], &mut z[..=h]);
        for y in 0..h {
            out[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&out[y * w..y * w + w]);
        edt_1d(&f[..w], &mut d[..w], &mut v[..w], &mut z[..=w]);
        out[y * w..y * w + w].copy_from_slice(&d[..w]);
    }
    out
}

fn edt_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        loop {
            let p = v[k];
            let num = (f[q] + (q * q) as f64) - (f[p] + (p * p) as f64);
            let den = 2.0 * q as f64 - 2.0 * p as f64;
            let s = num / den;
            if s <= z[k] {
                // z[0] 是 -inf，s 有限时不会走到 k==0 的死循环。
                k -= 1;
            } else {
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = f64::INFINITY;
                break;
            }
        }
    }
    let mut k = 0;
    for q in 0..n {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let dq = q as f64 - v[k] as f64;
        d[q] = dq * dq + f[v[k]];
    }
}

/// 距离变换里代表"无穷远"的成本。**不能**用 f64::INFINITY：两个无穷相减
/// 得到 NaN，会顺着下包络把整张图算成垃圾。
const FAR: f64 = 1.0e18;

/// 三遍盒式模糊 ≈ 高斯，带一圈 padding 后裁回原尺寸。
///
/// `replicate` 决定 padding 怎么填：
/// - `true`（平滑）：复制边缘 —— 包围盒贴边的那一面不该被啃掉；
/// - `false`（羽化）：留空 —— 过渡带必须能淡出到 0，否则羽化看着没生效。
fn blur3_padded(src: &[u8], w: usize, h: usize, r: usize, pad: usize, replicate: bool) -> Vec<f32> {
    let pw = w + 2 * pad;
    let ph = h + 2 * pad;
    if w == 0 || h == 0 || r == 0 {
        return src.iter().map(|&a| a as f32).collect();
    }
    let mut a = vec![0f32; pw * ph];
    for y in 0..ph {
        let sy = (y as i32 - pad as i32).clamp(0, h as i32 - 1) as usize;
        for x in 0..pw {
            let sx = (x as i32 - pad as i32).clamp(0, w as i32 - 1) as usize;
            let inside_src = x >= pad && x < pad + w && y >= pad && y < pad + h;
            if inside_src || replicate {
                a[y * pw + x] = src[sy * w + sx] as f32;
            }
        }
    }
    let mut tmp = vec![0f32; pw * ph];
    for _ in 0..3 {
        box_h(&a, &mut tmp, pw, ph, r);
        box_v(&tmp, &mut a, pw, ph, r);
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = a[(y + pad) * pw + (x + pad)];
        }
    }
    out
}

fn box_h(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f32;
    let last = w as i32 - 1;
    for y in 0..h {
        let row = y * w;
        let mut sum = 0.0f32;
        for k in 0..(2 * r + 1) as i32 {
            let i = (k - r as i32).clamp(0, last) as usize;
            sum += src[row + i];
        }
        for x in 0..w as i32 {
            dst[row + x as usize] = sum * norm;
            let add = (x + r as i32 + 1).clamp(0, last) as usize;
            let rem = (x - r as i32).clamp(0, last) as usize;
            sum += src[row + add] - src[row + rem];
        }
    }
}

fn box_v(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f32;
    let last = h as i32 - 1;
    for x in 0..w {
        let mut sum = 0.0f32;
        for k in 0..(2 * r + 1) as i32 {
            let i = (k - r as i32).clamp(0, last) as usize;
            sum += src[i * w + x];
        }
        for y in 0..h as i32 {
            dst[y as usize * w + x] = sum * norm;
            let add = (y + r as i32 + 1).clamp(0, last) as usize;
            let rem = (y - r as i32).clamp(0, last) as usize;
            sum += src[add * w + x] - src[rem * w + x];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Rect, Selection, Size};

    fn canvas(w: u32, h: u32) -> Size {
        Size::new(w, h)
    }

    #[test]
    fn rect_mask_matches_selection_contains() {
        let r = Rect::new(3, 4, 10, 7);
        let mask = SelectionMask::rect(r);
        let sel = Selection::Rect(r);
        for y in 0..20 {
            for x in 0..20 {
                let a = mask.contains(x as f32 + 0.5, y as f32 + 0.5);
                let b = sel.contains(x as f32 + 0.5, y as f32 + 0.5);
                assert_eq!(a, b, "mismatch at ({x},{y})");
            }
        }
        assert_eq!(mask.area(), 70);
        assert_eq!(mask.bounds, r);
    }

    #[test]
    fn ellipse_mask_excludes_corners() {
        let mask = SelectionMask::ellipse(Rect::new(0, 0, 64, 64));
        assert_eq!(mask.coverage_at(0, 0), 0);
        assert_eq!(mask.coverage_at(63, 63), 0);
        assert_eq!(mask.coverage_at(32, 32), 255);
        // 面积约为 π/4 的方框，上下各留 2% 余量。
        let a = mask.area() as f64;
        let full = 64.0 * 64.0;
        assert!(a / full > 0.74 && a / full < 0.80, "ratio {}", a / full);
    }

    #[test]
    fn polygon_scanline_matches_ray_cast() {
        // 凹多边形：能筛出"只按包围盒填充"的错误实现。
        let pts = vec![
            (2.0, 2.0),
            (18.0, 2.0),
            (18.0, 8.0),
            (8.0, 8.0),
            (8.0, 18.0),
            (2.0, 18.0),
        ];
        let mask = SelectionMask::polygon(&pts);
        let sel = Selection::Lasso(pts.clone());
        for y in 0..20 {
            for x in 0..20 {
                let a = mask.contains(x as f32 + 0.5, y as f32 + 0.5);
                let b = sel.contains(x as f32 + 0.5, y as f32 + 0.5);
                assert_eq!(a, b, "mismatch at ({x},{y})");
            }
        }
        // 凹口（12,12）不该被选中。
        assert!(!mask.contains(12.5, 12.5));
    }

    #[test]
    fn invert_twice_is_identity() {
        let mask = SelectionMask::ellipse(Rect::new(5, 5, 20, 12));
        let back = mask.invert(canvas(40, 40)).invert(canvas(40, 40));
        // 反选后 bounds 必然铺满画布，所以按覆盖度逐像素比。
        for y in 0..40 {
            for x in 0..40 {
                assert_eq!(
                    back.coverage_at(x, y),
                    mask.coverage_at(x, y),
                    "mismatch at ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn invert_bounds_is_canvas_and_punches_hole() {
        let mask = SelectionMask::rect(Rect::new(4, 4, 8, 8));
        let inv = mask.invert(canvas(20, 16));
        assert_eq!(inv.bounds, Rect::new(0, 0, 20, 16));
        assert_eq!(inv.coverage_at(5, 5), 0);
        assert_eq!(inv.coverage_at(0, 0), 255);
        assert_eq!(inv.area(), 20 * 16 - 64);
    }

    #[test]
    fn border_ring_has_expected_width() {
        let mask = SelectionMask::rect(Rect::new(20, 20, 40, 40));
        let ring = mask.border(10);
        // 圆环以原边缘为中心：向内 5px、向外 5px。
        assert_eq!(ring.coverage_at(22, 30), 255); // 内侧 2px
        assert_eq!(ring.coverage_at(16, 30), 255); // 外侧 4px
        assert_eq!(ring.coverage_at(30, 30), 0); // 环内空心
        assert_eq!(ring.coverage_at(10, 30), 0); // 环外
        assert!(ring.bounds.width >= 48 && ring.bounds.width <= 52);
    }

    #[test]
    fn smooth_rounds_stair_steps() {
        // 阶梯边：只有左下半选中。
        let mut pts = Vec::new();
        for i in 0..10 {
            let s = i as f32 * 3.0;
            pts.push((s, s));
            pts.push((s + 3.0, s));
        }
        pts.push((30.0, 30.0));
        pts.push((0.0, 30.0));
        let mask = SelectionMask::polygon(&pts);
        let before = mask.area();
        let smooth = mask.smooth(6);
        let after = smooth.area();
        // 平滑不改变量级，只削掉锯齿角。
        let ratio = after as f64 / before as f64;
        assert!(ratio > 0.85 && ratio < 1.15, "ratio {ratio}");
        // 仍然是一个整块（没有被阈值化啃出洞）。
        assert!(smooth.contains(10.5, 20.5));
    }

    #[test]
    fn feather_edges_are_soft_and_center_is_solid() {
        let mask = SelectionMask::rect(Rect::new(10, 10, 40, 40));
        let f = mask.feathered(8);
        // 中心仍然完全不透明（40px 的选区比 8px 羽化大得多）。
        assert_eq!(f.coverage_at(30, 30), 255);
        let edge = f.coverage_at(10, 30);
        assert!(edge > 0 && edge < 255, "edge {edge}");
        // 羽化 0 = 原样。
        assert_eq!(mask.feathered(0).alpha, mask.alpha);
    }

    #[test]
    fn feather_larger_than_selection_does_not_panic() {
        let mask = SelectionMask::rect(Rect::new(4, 4, 3, 2));
        let f = mask.feathered(60);
        // 羽化半径远大于选区时，内容被抹平成极低覆盖度，但不该 panic、
        // 也不该改动轮廓（bounds 决定蚂蚁线位置）。
        assert_eq!(f.bounds, mask.bounds);
        assert!(f.alpha.iter().all(|&a| a < 128));
    }

    #[test]
    fn outline_of_rect_has_four_corners() {
        let mask = SelectionMask::rect(Rect::new(4, 6, 10, 8));
        let paths = mask.outline(1);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].len(), 4);
        // 轮廓落在像素边界上：左上角 = (4,6)，右下角 = (14,14)。
        assert!(paths[0].contains(&(4.0, 6.0)));
        assert!(paths[0].contains(&(14.0, 14.0)));
    }

    #[test]
    fn outline_of_inverted_selection_has_two_rings() {
        let mask = SelectionMask::rect(Rect::new(4, 4, 8, 8));
        let inv = mask.invert(canvas(20, 20));
        let paths = inv.outline(1);
        assert_eq!(paths.len(), 2, "outer canvas edge + hole");
    }

    #[test]
    fn outline_respects_step_and_stays_closed() {
        let mask = SelectionMask::ellipse(Rect::new(0, 0, 200, 120));
        let paths = mask.outline(4);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].len() > 8);
        // 每一步都紧贴格子边界（坐标是 step 的整数倍）。
        for p in &paths[0] {
            assert_eq!(p.0 % 4.0, 0.0);
            assert_eq!(p.1 % 4.0, 0.0);
        }
    }

    #[test]
    fn union_intersect_subtract_behave() {
        let a = SelectionMask::rect(Rect::new(0, 0, 10, 10));
        let b = SelectionMask::rect(Rect::new(5, 5, 10, 10));
        assert_eq!(a.union(&b).area(), 175);
        assert_eq!(a.intersect(&b).area(), 25);
        assert_eq!(a.subtract(&b).area(), 75);
    }

    /// 布尔运算后 bounds 必须收紧到真实内容：QML 的尺寸徽标只读 bounds
    ///（蚂蚁线走 alpha），不收紧就会报出「两框并集」的错误尺寸。
    #[test]
    fn boolean_ops_tighten_bounds() {
        let a = SelectionMask::rect(Rect::new(0, 0, 10, 10));
        // 完全不重叠的框：减选后 a 原样，bounds 不能膨胀成两框并集。
        let far = SelectionMask::rect(Rect::new(20, 20, 10, 10));
        assert_eq!(a.subtract(&far).bounds, Rect::new(0, 0, 10, 10));
        assert_eq!(a.union(&far).bounds, Rect::new(0, 0, 30, 30));
        // 交集为空 → 整个选区消失。
        assert!(a.intersect(&far).is_empty());
        // 减选削掉右下角一步：bounds 跟着缩。
        let corner = SelectionMask::rect(Rect::new(5, 5, 10, 10));
        let cut = a.subtract(&corner);
        assert_eq!(cut.area(), 75);
        assert_eq!(cut.bounds, Rect::new(0, 0, 10, 10));
    }

    #[test]
    fn clipping_to_canvas_trims_outside_parts() {
        let mask = SelectionMask::rect(Rect::new(-5, -5, 20, 20));
        let clipped = mask.clipped(canvas(10, 10));
        assert_eq!(clipped.bounds, Rect::new(0, 0, 10, 10));
        assert_eq!(clipped.area(), 100);
    }

    #[test]
    fn state_applies_ops_from_base_without_stacking() {
        let mut st = SelectionState::new(SelectionMask::rect(Rect::new(10, 10, 40, 40)));
        let plain = st.mask().area();
        st.set_border(10);
        let once = st.mask().area();
        st.set_border(10); // 同一个值再来一次：不能叠加
        assert_eq!(st.mask().area(), once);
        // 40×40 的框 + 宽 10 的环 = 50² - 30² = 1600。
        assert!(once > 1400 && once < 1800, "ring area {once}");
        st.set_border(0);
        assert_eq!(st.mask().area(), plain);
    }

    #[test]
    fn state_feather_does_not_change_outline_mask() {
        let mut st = SelectionState::new(SelectionMask::rect(Rect::new(0, 0, 20, 20)));
        let outline_area = st.mask().area();
        st.set_feather(8);
        assert_eq!(st.mask().area(), outline_area);
        // 但真正消费的 effective 变了。
        assert!(st.effective().alpha.iter().any(|&a| a > 0 && a < 255));
    }

    #[test]
    fn new_selection_resets_ops() {
        let mut st = SelectionState::new(SelectionMask::rect(Rect::new(0, 0, 20, 20)));
        st.set_feather(12);
        st.set_base(SelectionMask::rect(Rect::new(5, 5, 10, 10)));
        assert_eq!(st.ops.feather, 0);
    }
}
