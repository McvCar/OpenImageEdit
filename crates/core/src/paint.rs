// crates/core/src/paint.rs
//! 油漆类工具：油漆桶（按颜色填充）与渐变。
//!
//! 所有函数都在**目标位图**上工作（base 或某个图层的 `image`），`origin`
//! 是这位图左上角在画布中的坐标 —— 换算关系 `位图坐标 = 画布坐标 - origin`。
//! 返回值是一块 `Patch`（矩形 + 位图内容），由调用方决定写进文档（实时
//! 预览）还是包成一条撤销记录。
use super::*;

/// 目标位图的一块改动：`rect` 是位图坐标。
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    pub rect: Rect,
    pub image: RgbaImage,
}

impl Patch {
    pub fn is_empty(&self) -> bool {
        self.rect.width == 0 || self.rect.height == 0
    }
}

/// 位图左上角在画布中的位置。
pub type Origin = (i32, i32);

/// 从位图里裁一块（坐标已钳到边界内）。用于取撤销记录的前后两份。
pub fn crop_region(img: &RgbaImage, rect: Rect) -> (Rect, RgbaImage) {
    let w = img.width();
    let h = img.height();
    let x = rect.x.max(0) as u32;
    let y = rect.y.max(0) as u32;
    let cw = rect.width.min(w.saturating_sub(x));
    let ch = rect.height.min(h.saturating_sub(y));
    if cw == 0 || ch == 0 {
        return (Rect::default(), RgbaImage::new(0, 0));
    }
    let sub = image::imageops::crop_imm(img, x, y, cw, ch).to_image();
    (Rect::new(x as i32, y as i32, cw, ch), sub)
}

/// straight-alpha 的 over 合成，`a` 是 0..1 的额外不透明度。
pub(crate) fn over(dst: [u8; 4], src: [u8; 4], a: f32) -> [u8; 4] {
    let sa = (src[3] as f32 / 255.0) * a.clamp(0.0, 1.0);
    if sa <= 0.0 {
        return dst;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= f32::EPSILON {
        return [0, 0, 0, 0];
    }
    let mut out = [0u8; 4];
    for c in 0..3 {
        let v = (src[c] as f32 * sa + dst[c] as f32 * da * (1.0 - sa)) / out_a;
        out[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    out[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    out
}

pub(crate) fn lerp_rgba(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let mut out = [0u8; 4];
    for c in 0..4 {
        out[c] = (a[c] as f32 + (b[c] as f32 - a[c] as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    out
}

/// 两像素是否「同色」：两边都全透明时视为同色（残留 RGB 不可信，和魔术棒
/// 的判定保持一致），否则逐通道最大差 <= tol。
fn near(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    if a[3] == 0 && b[3] == 0 {
        return true;
    }
    (0..4).all(|k| (a[k] as i32 - b[k] as i32).abs() <= tol)
}

/// 油漆桶：从 `seed`（画布坐标）按容差取一片，填 `color`。
///
/// * `contiguous` = false 时填全图所有同色像素（PS 的「连续」不勾）。
/// * 有选区时只填选区覆盖到的地方（按覆盖率做软边）。
/// * 返回 null 表示点在目标位图外。
pub fn flood_fill(
    img: &RgbaImage,
    origin: Origin,
    seed: (i32, i32),
    tolerance: u8,
    contiguous: bool,
    selection: Option<&SelectionMask>,
    color: [u8; 4],
    opacity: f32,
) -> Option<Patch> {
    let w = img.width() as i32;
    let h = img.height() as i32;
    if w <= 0 || h <= 0 {
        return None;
    }
    let sx = seed.0 - origin.0;
    let sy = seed.1 - origin.1;
    if sx < 0 || sy < 0 || sx >= w || sy >= h {
        return None;
    }
    let seed_px = img.get_pixel(sx as u32, sy as u32).0;
    let tol = tolerance as i32;
    let mut hit = vec![false; (w * h) as usize];
    let idx = |x: i32, y: i32| (y * w + x) as usize;

    if contiguous {
        // 和快速选择一样用队列：栈会把「邻居」走成蛇形，虽然这里没有深度
        // 上限，结果一样，但队列语义更明确（先近后远，命中包围盒更紧）。
        let mut queue: Vec<(i32, i32)> = Vec::with_capacity(4096);
        hit[idx(sx, sy)] = true;
        queue.push((sx, sy));
        let mut head = 0;
        while head < queue.len() {
            let (x, y) = queue[head];
            head += 1;
            for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                let i = idx(nx, ny);
                if hit[i] {
                    continue;
                }
                if near(img.get_pixel(nx as u32, ny as u32).0, seed_px, tol) {
                    hit[i] = true;
                    queue.push((nx, ny));
                }
            }
        }
    } else {
        for y in 0..h {
            for x in 0..w {
                if near(img.get_pixel(x as u32, y as u32).0, seed_px, tol) {
                    hit[idx(x, y)] = true;
                }
            }
        }
    }

    let mut x0 = w;
    let mut y0 = h;
    let mut x1 = -1;
    let mut y1 = -1;
    for y in 0..h {
        for x in 0..w {
            if !hit[idx(x, y)] {
                continue;
            }
            // 选区限制：只填选区里的部分，覆盖率当不透明度。
            if let Some(sel) = selection {
                if sel.coverage_at(x + origin.0, y + origin.1) == 0 {
                    continue;
                }
            }
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x1 < x0 {
        return None;
    }
    let rect = Rect::new(x0, y0, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
    let mut out = RgbaImage::new(rect.width, rect.height);
    for y in 0..rect.height as i32 {
        for x in 0..rect.width as i32 {
            let (ix, iy) = (x0 + x, y0 + y);
            let src = img.get_pixel(ix as u32, iy as u32).0;
            let cov = selection
                .map(|s| s.coverage_at(ix + origin.0, iy + origin.1))
                .unwrap_or(255);
            if !hit[idx(ix, iy)] || cov == 0 {
                out.put_pixel(x as u32, y as u32, image::Rgba(src));
                continue;
            }
            let a = opacity.clamp(0.0, 1.0) * (cov as f32 / 255.0);
            out.put_pixel(x as u32, y as u32, image::Rgba(over(src, color, a)));
        }
    }
    Some(Patch { rect, image: out })
}

/// 渐变：`from` → `to`（画布坐标）定义方向与长度，线性或径向。
///
/// 有选区时只作用于选区（按覆盖率）；否则铺满整张目标位图。`dither`
/// 给 t 加一点有序噪声，消除大面积渐变的色带。
#[allow(clippy::too_many_arguments)]
pub fn gradient_fill(
    img: &RgbaImage,
    origin: Origin,
    canvas: Size,
    selection: Option<&SelectionMask>,
    from: (f32, f32),
    to: (f32, f32),
    c0: [u8; 4],
    c1: [u8; 4],
    radial: bool,
    reverse: bool,
    opacity: f32,
    dither: bool,
) -> Option<Patch> {
    let w = img.width() as i32;
    let h = img.height() as i32;
    if w <= 0 || h <= 0 {
        return None;
    }
    // 作用范围：选区包围盒，或整张位图（画布裁剪后的区域）。
    let (cx0, cy0, cx1, cy1) = match selection {
        Some(sel) if !sel.is_empty() => {
            let b = sel.bounds;
            (b.x, b.y, b.right(), b.bottom())
        }
        _ => (0, 0, canvas.width as i32, canvas.height as i32),
    };
    if cx1 <= cx0 || cy1 <= cy0 {
        return None;
    }
    let x0 = (cx0 - origin.0).max(0);
    let y0 = (cy0 - origin.1).max(0);
    let x1 = (cx1 - origin.0).min(w);
    let y1 = (cy1 - origin.1).min(h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let rect = Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32);

    let (ax, ay) = from;
    let (bx, by) = to;
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-6 {
        return None;
    }
    let len = len2.sqrt();

    let mut out = RgbaImage::new(rect.width, rect.height);
    for y in 0..rect.height as i32 {
        for x in 0..rect.width as i32 {
            let (ix, iy) = (x0 + x, y0 + y);
            let src = img.get_pixel(ix as u32, iy as u32).0;
            let px = (ix + origin.0) as f32 + 0.5;
            let py = (iy + origin.1) as f32 + 0.5;
            let cov = selection
                .map(|s| s.coverage_at(ix + origin.0, iy + origin.1))
                .unwrap_or(255);
            if cov == 0 {
                out.put_pixel(x as u32, y as u32, image::Rgba(src));
                continue;
            }
            let mut t = if radial {
                let (vx, vy) = (px - ax, py - ay);
                (vx * vx + vy * vy).sqrt() / len
            } else {
                ((px - ax) * dx + (py - ay) * dy) / len2
            };
            if dither {
                // 4x4 有序抖动，幅度约 0.4%，肉眼不可见但足以打散色带。
                const BAYER: [[f32; 4]; 4] = [
                    [0.0, 8.0, 2.0, 10.0],
                    [12.0, 4.0, 14.0, 6.0],
                    [3.0, 11.0, 1.0, 9.0],
                    [15.0, 7.0, 13.0, 5.0],
                ];
                t += (BAYER[(iy & 3) as usize][(ix & 3) as usize] / 16.0 - 0.5) * 0.008;
            }
            if reverse {
                t = 1.0 - t;
            }
            let c = lerp_rgba(c0, c1, t);
            let a = opacity.clamp(0.0, 1.0) * (cov as f32 / 255.0);
            out.put_pixel(x as u32, y as u32, image::Rgba(over(src, c, a)));
        }
    }
    Some(Patch { rect, image: out })
}
