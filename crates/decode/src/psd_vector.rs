//! 形状图层的矢量填充：把 `vmsk` 的路径栅格化进图层位图。
//!
//! 为什么需要它：形状图层（圆角矩形卡片、图标底板）的**通道数据是全透明的**，
//! 像素由 Photoshop 按矢量路径实时生成。我们只读通道，于是这类层是一片空白，
//! 它自己的描边、内阴影、渐变叠加也全都无从谈起 —— 实测目标稿里 5 张卡片的
//! 渐变描边因此整圈丢失，正是全图最差的几个误差块。
//!
//! 坐标尺度：路径点是 8.24 定点数，但**不同导出器给的单位不一样**。PS 原生
//! 文件写的是画布像素（数值几十到几千），而本仓库的素材写的是 0..1 的比例
//! （实测 5 个形状的包围盒是 0.73..0.94）。用一个常数缩放就会把整张卡片画成
//! 一个像素的亮点，所以按数值范围判断：全部点都在 ±4 以内就当作比例。

use image::RgbaImage;
use psd::{VectorFill, VectorMask};

/// 每条贝塞尔曲线切成多少段。形状层通常只有 4~16 个节点，24 段足以让
/// 圆角在 4 倍超采样下看不出折线。
const CURVE_STEPS: usize = 24;
/// 每像素纵向超采样数（横向走解析覆盖率，所以 4 就够）。
const SUBSAMPLES: usize = 4;

/// 栅格化早退时给 `IMAGEEDIT_PSD_VECTOR_TRACE` 一条可定位的日志。
fn trace_fail(why: &str) {
    if std::env::var("IMAGEEDIT_PSD_VECTOR_TRACE").ok().as_deref() == Some("1") {
        println!(
            "{{\"phase\":\"decode\",\"event\":\"vector-fail\",\"why\":\"{}\"}}",
            why
        );
    }
}

/// 把 `mask` 的闭式子路径栅格化到**位图局部坐标**的覆盖率平面。
///
/// 返回 `(cov, 路径包围盒@局部)`；`None` 表示路径不可用（空、坐标不是
/// 有限数、完全落在位图外），调用方据此保持原样。
fn shape_coverage(
    mask: &VectorMask,
    w: u32,
    h: u32,
    offset: (i32, i32),
    canvas: (u32, u32),
) -> Option<(Vec<f32>, [f32; 4])> {
    if w == 0 || h == 0 {
        trace_fail("empty-image");
        return None;
    }
    let Some((bx0, by0, bx1, by1)) = mask.bounds() else {
        trace_fail("no-bounds");
        return None;
    };
    // 尺度判断：PS 原生写像素，本仓库的素材写 0..1 的比例。
    let normalized = [bx0, by0, bx1, by1].iter().all(|v| v.abs() <= 4.0);
    let (sx, sy) = if normalized {
        (canvas.0 as f32, canvas.1 as f32)
    } else {
        (1.0, 1.0)
    };

    let mut polys: Vec<Vec<(f32, f32)>> = Vec::new();
    for sp in &mask.subpaths {
        if !sp.closed || sp.knots.len() < 2 {
            continue;
        }
        let mut poly = Vec::with_capacity(sp.knots.len() * 8);
        let n = sp.knots.len();
        for i in 0..n {
            let a = sp.knots[i];
            let b = sp.knots[(i + 1) % n];
            let p0 = (a.anchor.1 * sx - offset.0 as f32, a.anchor.0 * sy - offset.1 as f32);
            let c1 = (a.leaving.1 * sx - offset.0 as f32, a.leaving.0 * sy - offset.1 as f32);
            let c2 = (b.preceding.1 * sx - offset.0 as f32, b.preceding.0 * sy - offset.1 as f32);
            let p3 = (b.anchor.1 * sx - offset.0 as f32, b.anchor.0 * sy - offset.1 as f32);
            let straight = (c1.0 - p0.0).abs() < 1e-4
                && (c1.1 - p0.1).abs() < 1e-4
                && (c2.0 - p3.0).abs() < 1e-4
                && (c2.1 - p3.1).abs() < 1e-4;
            // 直的段不用插值：矩形卡片 8 个节点里 4 条直边、4 条圆角，
            // 插值只会在直边上多做无用的工作。
            let steps = if straight { 1 } else { CURVE_STEPS };
            for s in 0..steps {
                let t = s as f32 / steps as f32;
                poly.push(bezier(p0, c1, c2, p3, t));
            }
        }
        if poly.len() >= 3 {
            polys.push(poly);
        }
    }
    if polys.is_empty() {
        trace_fail("no-closed-subpath");
        return None;
    }

    // 路径包围盒在位图内的部分（局部坐标，浮点）。
    let px0 = ((bx0.min(bx1) * sx) - offset.0 as f32).max(0.0);
    let py0 = ((by0.min(by1) * sy) - offset.1 as f32).max(0.0);
    let px1 = ((bx0.max(bx1) * sx) - offset.0 as f32).min(w as f32);
    let py1 = ((by0.max(by1) * sy) - offset.1 as f32).min(h as f32);
    if px1 <= px0 || py1 <= py0 {
        trace_fail("bbox-outside");
        return None;
    }

    let mut cov = vec![0f32; (w * h) as usize];
    rasterize(&polys, &mut cov, w, h, py0, py1);
    if mask.invert {
        for v in cov.iter_mut() {
            *v = 1.0 - *v;
        }
    }
    Some((cov, [px0, py0, px1, py1]))
}

/// 把形状画到 `image` 上（`offset` 是这张位图左上角的画布坐标）。
///
/// 返回 `false` 表示路径不可用（空、坐标不是有限数、完全落在位图外），
/// 调用方据此保持原样。
pub fn fill_shape(
    image: &mut RgbaImage,
    offset: (i32, i32),
    canvas: (u32, u32),
    mask: &VectorMask,
    fill: Option<&VectorFill>,
) -> bool {
    let (w, h) = image.dimensions();
    let Some((cov, gb)) = shape_coverage(mask, w, h, offset, canvas) else {
        return false;
    };

    // 填充色：纯色优先，其次渐变（按路径包围盒做基准，与 PS 一致），
    // 都没有就当作纯白 —— 无填充的形状在 PS 里也只显示描边，这里由
    // 后续描边样式覆盖，不会漏出白色。
    let solid = fill.and_then(|f| f.solid);
    let gradient = fill.and_then(|f| f.gradient.as_ref());
    let mut painted = false;
    for y in 0..h {
        for x in 0..w {
            let c = cov[(y * w + x) as usize];
            if c <= 0.002 {
                continue;
            }
            let (color, ga) = match (solid, gradient) {
                (Some(c), _) => (c, 1.0),
                (None, Some(g)) => {
                    let t = crate::psd_effects::gradient_t(x as f32, y as f32, gb, g);
                    g.sample(t)
                }
                (None, None) => ([255, 255, 255], 1.0),
            };
            let a = (c.clamp(0.0, 1.0) * ga.clamp(0.0, 1.0) * 255.0).round() as u8;
            if a == 0 {
                continue;
            }
            let px = image.get_pixel_mut(x, y);
            // 洞填充：位图已有 alpha 的像素保持不动。PS 的形状内容 = 矢量
            // 填充实时渲染，但通道里缓存的内填与 PS 输出逐像素一致（实测
            // 面板内部 MAE 0），用我们的近似渐变覆盖它反而会倒退。缓存缺失
            // 的洞必须补：矢量路径可能比缓存位图大一圈（实测 `矩形 556 拷贝`
            // 的路径边界在位图外 1px，边缘整条渐变丢失）。全空位图的层所有
            // 像素都是洞，行为与旧的「全空才整张填」完全一致。
            if px.0[3] > 0 {
                continue;
            }
            let src = [color[0], color[1], color[2], a];
            // 源覆盖：形状自己的位图通常全透明，但外描边扩边后可能已有
            // 内容，直接覆盖会吃掉描边。
            let inv = 1.0 - a as f32 / 255.0;
            for k in 0..3 {
                px.0[k] = (src[k] as f32 * (a as f32 / 255.0) + px.0[k] as f32 * inv).round() as u8;
            }
            px.0[3] = ((a as f32 / 255.0 + px.0[3] as f32 / 255.0 * inv) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
            painted = true;
        }
    }
    painted
}

/// 把位图裁到矢量路径内：`alpha *= cov`（PS 显示形状层 = 内容 ∩ 路径）。
///
/// PS 保存的缓存像素一般已按路径烘焙，裁剪是恒等变换；但文件里存在
/// 「缓存过期」的形状层 —— 路径改过之后 PS 只按路径实时渲染，通道里的
/// 缓存还是旧形状（实测底部按钮行 `矩形 670 拷贝` 的缓存是直角、路径
/// 是圆角，圆角外多出一圈红）。psd-tools 同样对位图应用 vmsk。
pub fn clip_shape(
    image: &mut RgbaImage,
    offset: (i32, i32),
    canvas: (u32, u32),
    mask: &VectorMask,
) -> bool {
    let (w, h) = image.dimensions();
    let Some((cov, _gb)) = shape_coverage(mask, w, h, offset, canvas) else {
        return false;
    };
    let mut clipped = false;
    for (px, c) in image.pixels_mut().zip(cov.iter()) {
        if *c >= 0.999 || px.0[3] == 0 {
            continue;
        }
        let keep = (c.clamp(0.0, 1.0) * 255.0).round();
        let a = ((px.0[3] as f32) * keep / 255.0).round() as u8;
        if a != px.0[3] {
            px.0[3] = a;
            clipped = true;
        }
    }
    clipped
}

/// 三次贝塞尔在 `t` 处的点。
fn bezier(p0: (f32, f32), c1: (f32, f32), c2: (f32, f32), p3: (f32, f32), t: f32) -> (f32, f32) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p0.0 + b * c1.0 + c * c2.0 + d * p3.0,
        a * p0.1 + b * c1.1 + c * c2.1 + d * p3.1,
    )
}

/// 扫描线填充（非零环绕），每像素 4 条子扫描线，横向解析覆盖率。
fn rasterize(polys: &[Vec<(f32, f32)>], cov: &mut [f32], w: u32, h: u32, py0: f32, py1: f32) {
    let y_start = py0.floor().max(0.0) as u32;
    let y_end = (py1.ceil() as u32).min(h);
    let weight = 1.0 / SUBSAMPLES as f32;
    let mut xs: Vec<(f32, i32)> = Vec::new();
    for y in y_start..y_end {
        for s in 0..SUBSAMPLES {
            let sy = y as f32 + (s as f32 + 0.5) * weight;
            xs.clear();
            for poly in polys {
                let n = poly.len();
                for i in 0..n {
                    let (x0, y0) = poly[i];
                    let (x1, y1) = poly[(i + 1) % n];
                    if (y0 <= sy && y1 > sy) || (y1 <= sy && y0 > sy) {
                        let t = (sy - y0) / (y1 - y0);
                        xs.push((x0 + (x1 - x0) * t, if y1 > y0 { 1 } else { -1 }));
                    }
                }
            }
            if xs.len() < 2 {
                continue;
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0i32;
            let row = (y * w) as usize;
            for i in 0..xs.len() - 1 {
                winding += xs[i].1;
                if winding == 0 {
                    continue;
                }
                let (xa, xb) = (xs[i].0, xs[i + 1].0);
                add_span(&mut cov[row..row + w as usize], xa, xb, weight);
            }
        }
    }
}

/// 把 `[xa, xb)` 这段横向覆盖率加到一行里：整格加 1，两端按小数部分。
fn add_span(row: &mut [f32], xa: f32, xb: f32, weight: f32) {
    let w = row.len() as f32;
    let xa = xa.max(0.0);
    let xb = xb.min(w);
    if xb <= xa {
        return;
    }
    let i0 = xa.floor() as usize;
    let i1 = xb.ceil() as usize;
    if i0 == i1.saturating_sub(1) && xb - xa < 1.0 {
        // 同一格内。
        if i0 < row.len() {
            row[i0] += (xb - xa) * weight;
        }
        return;
    }
    if i0 < row.len() {
        row[i0] += (i0 as f32 + 1.0 - xa) * weight;
    }
    for i in (i0 + 1)..i1.min(row.len()) {
        row[i] += weight;
    }
    if i1 <= row.len() && i1 > 0 {
        let last = i1 - 1;
        if last > i0 && xb > last as f32 {
            row[last] += (xb - last as f32) * weight;
        }
    }
    // 修正：完整格已经加过 weight，这里只在末尾补足小数部分。
    if i1 <= row.len() && i1 > 0 && xb < i1 as f32 {
        let last = i1 - 1;
        if last > i0 {
            row[last] -= weight;
            row[last] += (xb - last as f32) * weight;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use psd::VectorKnot;

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> VectorMask {
        let k = |y: f32, x: f32| VectorKnot {
            preceding: (y, x),
            anchor: (y, x),
            leaving: (y, x),
        };
        VectorMask {
            invert: false,
            subpaths: vec![psd::VectorSubpath {
                closed: true,
                knots: vec![
                    k(y0, x0),
                    k(y0, x1),
                    k(y1, x1),
                    k(y1, x0),
                ],
            }],
        }
    }

    /// 归一化坐标（0..1）要按画布缩放，而不是画成一个像素的点。
    #[test]
    fn normalized_paths_scale_to_the_canvas() {
        let mut img = RgbaImage::new(20, 20);
        let mask = square(0.25, 0.25, 0.75, 0.75);
        assert!(fill_shape(&mut img, (0, 0), (20, 20), &mask, None));
        assert_eq!(img.get_pixel(10, 10).0[3], 255, "中间必须被填满");
        assert_eq!(img.get_pixel(1, 1).0[3], 0, "角落在形状外");
        assert_eq!(img.get_pixel(5, 10).0[3], 255, "左边界已进入形状");
    }

    /// 像素坐标的路径不缩放（PS 原生文件）。
    #[test]
    fn pixel_paths_are_used_as_is() {
        let mut img = RgbaImage::new(40, 40);
        let mask = square(10.0, 10.0, 30.0, 30.0);
        assert!(fill_shape(&mut img, (0, 0), (40, 40), &mask, None));
        assert_eq!(img.get_pixel(20, 20).0[3], 255);
        assert_eq!(img.get_pixel(2, 2).0[3], 0);
        assert_eq!(img.get_pixel(35, 35).0[3], 0);
    }

    /// 位图偏移必须跟着走：画布 10..20 的形状落进 (10,10) 起的小位图，
    /// 局部坐标是 0..10，而不是 10..20。
    #[test]
    fn offset_shifts_the_shape_into_local_coordinates() {
        let mut img = RgbaImage::new(20, 20);
        let mask = square(0.5, 0.5, 1.0, 1.0);
        assert!(fill_shape(&mut img, (10, 10), (20, 20), &mask, None));
        assert_eq!(img.get_pixel(5, 5).0[3], 255, "画布 (15,15) 在形状内");
        assert_eq!(img.get_pixel(9, 9).0[3], 255, "画布 (19,19) 仍在形状内");
        assert_eq!(img.get_pixel(15, 15).0[3], 0, "画布 (25,25) 在形状外");
    }

    /// 纯色填充直接落到像素上。
    #[test]
    fn solid_fill_colours_the_shape() {
        let mut img = RgbaImage::new(8, 8);
        let mask = square(0.0, 0.0, 1.0, 1.0);
        let fill = VectorFill {
            solid: Some([12, 200, 90]),
            gradient: None,
        };
        assert!(fill_shape(&mut img, (0, 0), (8, 8), &mask, Some(&fill)));
        assert_eq!(img.get_pixel(4, 4).0, [12, 200, 90, 255]);
    }
}
