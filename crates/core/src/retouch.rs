// crates/core/src/retouch.rs
//! 修复类工具：污点修复、修复画笔、修补。
//!
//! 三种工具的共同点是「把一块区域的像素换掉」，区别只在**取样从哪来**：
//! * 污点修复：完全从四周环带插值，不需要用户指定源（内容识别的最简版）；
//! * 修复画笔：用户 Alt 点一下定源，按取样偏移整块复制，再做明暗/色彩匹配；
//! * 修补：先圈一块区域，再拖到取样位置，语义同修复画笔但区域是手画的。
//!
//! 全部在目标位图上工作，`origin` 是位图左上角的画布坐标（位图 = 画布 -
//! origin），笔迹 / 区域参数都是画布坐标。
use super::*;
use crate::paint::Patch;

/// 笔刷参数（PS 的画笔面板里最常用的三项）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushSpec {
    /// 直径，画布像素。
    pub size: f32,
    /// 硬度 0..1：1 = 硬边，0 = 从圆心一路羽化到边缘。
    pub hardness: f32,
    /// 间距，按直径的比例（0.05..1）。越小越连贯，越大越像一串点。
    pub spacing: f32,
}

impl Default for BrushSpec {
    fn default() -> Self {
        Self {
            size: 40.0,
            hardness: 0.7,
            spacing: 0.2,
        }
    }
}

/// 一条笔迹光栅化出来的覆盖度图（位图坐标）。
#[derive(Debug, Clone, PartialEq)]
pub struct StrokeMask {
    pub rect: Rect,
    pub alpha: Vec<u8>,
    pub radius: f32,
}

impl StrokeMask {
    pub fn coverage(&self, x: i32, y: i32) -> u8 {
        let r = self.rect;
        if x < r.x || y < r.y || x >= r.right() || y >= r.bottom() {
            return 0;
        }
        let ix = (x - r.x) as usize;
        let iy = (y - r.y) as usize;
        match self.alpha.get(iy * r.width as usize + ix) {
            Some(&a) => a,
            None => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rect.width == 0 || self.rect.height == 0
    }
}

/// 笔迹覆盖度：沿折线按 `spacing` 打圆盘，取最大值。
///
/// 单个点（点一下不拖）也打一次，这样「点一下」和「拖一笔」行为一致。
pub fn stroke_mask(
    points: &[(f32, f32)],
    origin: (i32, i32),
    canvas: Size,
    brush: BrushSpec,
) -> Option<StrokeMask> {
    if points.is_empty() || canvas.width == 0 || canvas.height == 0 {
        return None;
    }
    let radius = (brush.size.max(2.0) / 2.0).max(0.5);
    let hardness = brush.hardness.clamp(0.0, 1.0);
    // 位图坐标下的笔迹包围盒（含半径的余量）。
    let mut minx = f32::MAX;
    let mut miny = f32::MAX;
    let mut maxx = f32::MIN;
    let mut maxy = f32::MIN;
    for p in points {
        minx = minx.min(p.0);
        miny = miny.min(p.1);
        maxx = maxx.max(p.0);
        maxy = maxy.max(p.1);
    }
    let cx0 = (minx - radius - 1.0).floor() as i32 - origin.0;
    let cy0 = (miny - radius - 1.0).floor() as i32 - origin.1;
    let cx1 = (maxx + radius + 1.0).ceil() as i32 - origin.0;
    let cy1 = (maxy + radius + 1.0).ceil() as i32 - origin.1;
    // 夹到画布范围内（位图可能比画布大，这里只关心画布内的部分）。
    let bx0 = cx0.max(-origin.0);
    let by0 = cy0.max(-origin.1);
    let bx1 = cx1.min(canvas.width as i32 - origin.0);
    let by1 = cy1.min(canvas.height as i32 - origin.1);
    if bx1 <= bx0 || by1 <= by0 {
        return None;
    }
    let rect = Rect::new(bx0, by0, (bx1 - bx0) as u32, (by1 - by0) as u32);
    let mut alpha = vec![0u8; (rect.width * rect.height) as usize];

    let mut stamp = |cx: f32, cy: f32| {
        let x0 = (cx - radius).floor() as i32;
        let y0 = (cy - radius).floor() as i32;
        let x1 = (cx + radius).ceil() as i32;
        let y1 = (cy + radius).ceil() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                if x < rect.x || y < rect.y || x >= rect.right() || y >= rect.bottom() {
                    continue;
                }
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                if d > radius {
                    continue;
                }
                let cov = if radius <= 1.0 || d <= radius * hardness {
                    255.0
                } else {
                    let t = ((d - radius * hardness) / (radius * (1.0 - hardness)).max(0.001))
                        .clamp(0.0, 1.0);
                    // smoothstep，边缘过渡比线性更自然。
                    255.0 * (1.0 - t * t * (3.0 - 2.0 * t))
                };
                let i = ((y - rect.y) * rect.width as i32 + (x - rect.x)) as usize;
                if (cov as u8) > alpha[i] {
                    alpha[i] = cov as u8;
                }
            }
        }
    };

    let step = (brush.size * brush.spacing.clamp(0.02, 1.0)).max(1.0);
    if points.len() == 1 {
        stamp(points[0].0, points[0].1);
    }
    for pair in points.windows(2) {
        let (ax, ay) = pair[0];
        let (bx, by) = pair[1];
        let dx = bx - ax;
        let dy = by - ay;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 0.01 {
            stamp(bx, by);
            continue;
        }
        let n = (len / step).ceil() as i32;
        for i in 0..=n {
            let t = i as f32 / n.max(1) as f32;
            stamp(ax + dx * t, ay + dy * t);
        }
    }
    Some(StrokeMask {
        rect,
        alpha,
        radius,
    })
}

/// 把一个像素插值的混合结果写进 `dst`：只混合 RGB，alpha 保持原样（修复
/// 不该改变图层的透明形状）。
fn mix_keep_alpha(dst: [u8; 4], src: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let mut out = dst;
    for c in 0..3 {
        out[c] = (dst[c] as f32 + (src[c] as f32 - dst[c] as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    out
}

/// 位图上的取样闭包：越界返回 None。
fn at(img: &RgbaImage, x: i32, y: i32) -> Option<[u8; 4]> {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
        return None;
    }
    Some(img.get_pixel(x as u32, y as u32).0)
}

/// 污点修复：笔迹覆盖到的像素全部用四周环带的加权平均替换。
///
/// 采样用 16 个方向、由近及远找第一个「笔迹外」的像素，权重 1/d²。这比
/// 直接取环带均值更贴合结构：靠近边缘的一侧权重大，颜色过渡不会被抹平。
pub fn spot_heal(img: &RgbaImage, stroke: &StrokeMask, strength: f32) -> Option<Patch> {
    if stroke.is_empty() || strength <= 0.0 {
        return None;
    }
    let margin = (stroke.radius * 2.0) as i32 + 8;
    let rect = expand_clip(stroke.rect, margin, img.width(), img.height());
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    let mut out = image::imageops::crop_imm(
        img,
        rect.x as u32,
        rect.y as u32,
        rect.width,
        rect.height,
    )
    .to_image();

    // 16 个方向，cos/sin 预先算好。
    let dirs: Vec<(f32, f32)> = (0..16)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 16.0;
            (a.cos(), a.sin())
        })
        .collect();
    let max_d = (stroke.radius * 3.0 + 12.0).max(8.0);

    for y in 0..rect.height as i32 {
        for x in 0..rect.width as i32 {
            let (ix, iy) = (rect.x + x, rect.y + y);
            let cov = stroke.coverage(ix, iy);
            if cov == 0 {
                continue;
            }
            let mut sum = [0f32; 4];
            let mut wsum = 0f32;
            for (dx, dy) in &dirs {
                let mut d = 1.0;
                while d <= max_d {
                    let sx = ix + (dx * d).round() as i32;
                    let sy = iy + (dy * d).round() as i32;
                    if sx < rect.x || sy < rect.y || sx >= rect.right() || sy >= rect.bottom() {
                        break;
                    }
                    if stroke.coverage(sx, sy) == 0 {
                        if let Some(c) = at(img, sx, sy) {
                            let w = 1.0 / (d * d + 1.0);
                            for k in 0..4 {
                                sum[k] += c[k] as f32 * w;
                            }
                            wsum += w;
                        }
                        break;
                    }
                    d += 1.5;
                }
            }
            if wsum <= 0.0 {
                continue;
            }
            let avg = [
                (sum[0] / wsum).round() as u8,
                (sum[1] / wsum).round() as u8,
                (sum[2] / wsum).round() as u8,
                (sum[3] / wsum).round() as u8,
            ];
            let src = out.get_pixel(x as u32, y as u32).0;
            let t = (cov as f32 / 255.0) * strength.clamp(0.0, 1.0);
            out.put_pixel(x as u32, y as u32, image::Rgba(mix_keep_alpha(src, avg, t)));
        }
    }
    Some(Patch { rect, image: out })
}

/// 取样块与目标块的明暗 / 色彩差（逐通道均值），环带里逐像素比较。
fn ring_offset(
    img: &RgbaImage,
    stroke: &StrokeMask,
    delta: (i32, i32),
    rect: Rect,
) -> [f32; 3] {
    let mut sum = [0f32; 3];
    let mut n = 0f32;
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if stroke.coverage(x, y) != 0 {
                continue;
            }
            let Some(dst) = at(img, x, y) else { continue };
            let Some(src) = at(img, x + delta.0, y + delta.1) else {
                continue;
            };
            // 全透明的环带像素颜色不可信，跳过。
            if dst[3] == 0 || src[3] == 0 {
                continue;
            }
            for k in 0..3 {
                sum[k] += dst[k] as f32 - src[k] as f32;
            }
            n += 1.0;
        }
    }
    if n <= 0.0 {
        [0.0; 3]
    } else {
        [sum[0] / n, sum[1] / n, sum[2] / n]
    }
}

/// 修复画笔：从 `anchor` 取样，复制到笔迹处，并补偿明暗/色彩差。
///
/// `start` 是这一笔的起点（画布坐标）：取样偏移 = anchor - start，整笔
/// 固定，跟 PS 的「对齐」行为一致。
pub fn heal_clone(
    img: &RgbaImage,
    stroke: &StrokeMask,
    start: (f32, f32),
    anchor: (f32, f32),
    strength: f32,
) -> Option<Patch> {
    if stroke.is_empty() || strength <= 0.0 {
        return None;
    }
    let delta = (
        (anchor.0 - start.0).round() as i32,
        (anchor.1 - start.1).round() as i32,
    );
    if delta == (0, 0) {
        return None;
    }
    let margin = 2;
    let rect = expand_clip(stroke.rect, margin, img.width(), img.height());
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    let off = ring_offset(img, stroke, delta, rect);
    let mut out = image::imageops::crop_imm(
        img,
        rect.x as u32,
        rect.y as u32,
        rect.width,
        rect.height,
    )
    .to_image();
    for y in 0..rect.height as i32 {
        for x in 0..rect.width as i32 {
            let (ix, iy) = (rect.x + x, rect.y + y);
            let cov = stroke.coverage(ix, iy);
            if cov == 0 {
                continue;
            }
            let Some(src) = at(img, ix + delta.0, iy + delta.1) else {
                continue;
            };
            let dst = out.get_pixel(x as u32, y as u32).0;
            let matched = [
                (src[0] as f32 + off[0]).round().clamp(0.0, 255.0) as u8,
                (src[1] as f32 + off[1]).round().clamp(0.0, 255.0) as u8,
                (src[2] as f32 + off[2]).round().clamp(0.0, 255.0) as u8,
                src[3],
            ];
            let t = (cov as f32 / 255.0) * strength.clamp(0.0, 1.0);
            out.put_pixel(x as u32, y as u32, image::Rgba(mix_keep_alpha(dst, matched, t)));
        }
    }
    Some(Patch { rect, image: out })
}

/// 修补：把 `region`（画布坐标的选区形状）整体替换成偏移 `offset` 处的
/// 像素，边缘按 `feather` 像素羽化，并做明暗/色彩匹配。
pub fn patch_fill(
    img: &RgbaImage,
    origin: (i32, i32),
    region: &SelectionMask,
    offset: (i32, i32),
    feather: u32,
    strength: f32,
) -> Option<Patch> {
    if region.is_empty() || strength <= 0.0 {
        return None;
    }
    let b = region.bounds;
    let rect = Rect::new(
        (b.x - origin.0).max(0),
        (b.y - origin.1).max(0),
        b.width,
        b.height,
    );
    let rect = expand_clip(rect, 1, img.width(), img.height());
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    // 覆盖度：从选区取出，再按 feather 做可分离盒式模糊。
    let mut cov = vec![0f32; (rect.width * rect.height) as usize];
    for y in 0..rect.height as i32 {
        for x in 0..rect.width as i32 {
            let ix = rect.x + x;
            let iy = rect.y + y;
            cov[(y * rect.width as i32 + x) as usize] =
                region.coverage_at(ix + origin.0, iy + origin.1) as f32 / 255.0;
        }
    }
    if feather > 0 {
        cov = box_blur(&cov, rect.width as usize, rect.height as usize, feather as usize);
    }
    // 环带明暗差：用选区内部的像素与样本比较（区域外的像素往往和区域内容
    // 不同，用它算偏移会让修补结果整体偏色）。
    let mut sum = [0f32; 3];
    let mut n = 0f32;
    for y in 0..rect.height as i32 {
        for x in 0..rect.width as i32 {
            let ix = rect.x + x;
            let iy = rect.y + y;
            if region.coverage_at(ix + origin.0, iy + origin.1) < 200 {
                continue;
            }
            let Some(dst) = at(img, ix, iy) else { continue };
            let Some(src) = at(img, ix + offset.0, iy + offset.1) else {
                continue;
            };
            if dst[3] == 0 || src[3] == 0 {
                continue;
            }
            for k in 0..3 {
                sum[k] += dst[k] as f32 - src[k] as f32;
            }
            n += 1.0;
        }
    }
    let off = if n > 0.0 {
        [sum[0] / n, sum[1] / n, sum[2] / n]
    } else {
        [0.0; 3]
    };

    let mut out = image::imageops::crop_imm(
        img,
        rect.x as u32,
        rect.y as u32,
        rect.width,
        rect.height,
    )
    .to_image();
    for y in 0..rect.height as i32 {
        for x in 0..rect.width as i32 {
            let c = cov[(y * rect.width as i32 + x) as usize];
            if c <= 0.001 {
                continue;
            }
            let (ix, iy) = (rect.x + x, rect.y + y);
            let Some(src) = at(img, ix + offset.0, iy + offset.1) else {
                continue;
            };
            let dst = out.get_pixel(x as u32, y as u32).0;
            let matched = [
                (src[0] as f32 + off[0]).round().clamp(0.0, 255.0) as u8,
                (src[1] as f32 + off[1]).round().clamp(0.0, 255.0) as u8,
                (src[2] as f32 + off[2]).round().clamp(0.0, 255.0) as u8,
                src[3],
            ];
            let t = c * strength.clamp(0.0, 1.0);
            out.put_pixel(x as u32, y as u32, image::Rgba(mix_keep_alpha(dst, matched, t)));
        }
    }
    Some(Patch { rect, image: out })
}

/// 矩形向外扩 `margin` 并夹到 `0..w/0..h`。
pub(crate) fn expand_clip(rect: Rect, margin: i32, w: u32, h: u32) -> Rect {
    let x0 = (rect.x - margin).max(0);
    let y0 = (rect.y - margin).max(0);
    let x1 = (rect.right() + margin).min(w as i32);
    let y1 = (rect.bottom() + margin).min(h as i32);
    if x1 <= x0 || y1 <= y0 {
        return Rect::default();
    }
    Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
}

/// 可分离盒式模糊（两次水平 + 两次垂直近似高斯），只用于修补的软边。
fn box_blur(src: &[f32], w: usize, h: usize, radius: usize) -> Vec<f32> {
    if w == 0 || h == 0 || radius == 0 {
        return src.to_vec();
    }
    let mut a = src.to_vec();
    let mut b = vec![0f32; src.len()];
    for _ in 0..2 {
        // 水平
        for y in 0..h {
            for x in 0..w {
                let lo = x.saturating_sub(radius);
                let hi = (x + radius).min(w - 1);
                let mut s = 0.0;
                for xx in lo..=hi {
                    s += a[y * w + xx];
                }
                b[y * w + x] = s / (hi - lo + 1) as f32;
            }
        }
        // 垂直
        for y in 0..h {
            for x in 0..w {
                let lo = y.saturating_sub(radius);
                let hi = (y + radius).min(h - 1);
                let mut s = 0.0;
                for yy in lo..=hi {
                    s += b[yy * w + x];
                }
                a[y * w + x] = s / (hi - lo + 1) as f32;
            }
        }
    }
    a
}
