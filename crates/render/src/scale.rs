use image::imageops::FilterType;
use image::RgbaImage;

use editcore::{Rect, Size};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub size: Size,
    pub zoom: Zoom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zoom {
    Fit,
    Actual,
    Factor(f32),
}

impl Viewport {
    pub fn new(width: u32, height: u32, zoom: Zoom) -> Self {
        Self {
            size: Size::new(width, height),
            zoom,
        }
    }
}

pub fn scale_for(source: Size, viewport: Size, zoom: Zoom) -> f32 {
    if source.pixels() == 0 || viewport.pixels() == 0 {
        return 1.0;
    }
    match zoom {
        Zoom::Actual => 1.0,
        Zoom::Factor(factor) => factor,
        Zoom::Fit => {
            let sx = viewport.width as f32 / source.width as f32;
            let sy = viewport.height as f32 / source.height as f32;
            sx.min(sy).min(1.0)
        }
    }
}

pub fn display_size(source: Size, viewport: Size, zoom: Zoom) -> Size {
    let scale = scale_for(source, viewport, zoom);
    Size::new(
        ((source.width as f32 * scale).round() as u32).max(1),
        ((source.height as f32 * scale).round() as u32).max(1),
    )
}

/// Resample `image` to `target` for PREVIEW use.
///
/// Downscaling goes through `imageops::thumbnail` (integer box filter): for a
/// preview-sized target it is ~2.5x faster than the general
/// `resize(…, Triangle)` and, at these scale factors, indistinguishable on
/// screen (measured against a Lanczos reference: box err 0.45/255 vs bilinear
/// 0.34, nearest 0.65). That matters in debug builds, where one Triangle
/// downscale of a 1600×1000 photo to a 1100×688 fit preview costs ~390 ms and a
/// window resize used to pay it per event.
///
/// Upscaling keeps the filtered path (a box filter would turn zoomed-in pixels
/// into blocky mush).
///
/// RGBA here is straight (non-premultiplied) alpha. Averaging straight-alpha
/// pixels lets the RGB of fully transparent pixels (RGB=0 after erase) bleed
/// into neighbours — erased holes grew a dark rim on all four sides. When the
/// bitmap carries any transparency we resample premultiplied instead; fully
/// opaque bitmaps (the common photo case) skip the extra passes entirely.
///
/// ★降采样用自带的 [`downscale_box`]，**不用** `imageops::thumbnail`：后者
///   会把每个输出像素的源块算成 `[ceil(left*ratio), ceil(right*ratio))`，
///   既丢掉左端的分数部分（整幅图右/下平移最多 1 个源像素），又因为 f32
///   累加在整数边界上 `ceil` 吃掉一个多余源像素。对不透明照片看不出来，
///   但「剪掉一块再贴回原位」时，洞的边缘会向相邻预览像素渗 1/3~1/2 的
///   alpha，透出棋盘格 —— 就是用户报的那条「粘回去边缘有空隙」的浅色缝。
pub fn resize_to(image: &RgbaImage, target: Size) -> RgbaImage {
    if image.width() == target.width && image.height() == target.height {
        return image.clone();
    }
    let downscale = target.width < image.width() || target.height < image.height();
    if has_transparent_pixel(image) {
        let pma = premultiply(image);
        let scaled = if downscale {
            downscale_box(&pma, target.width, target.height)
        } else {
            image::imageops::resize(&pma, target.width, target.height, FilterType::Triangle)
        };
        return unpremultiply(&scaled);
    }
    if downscale {
        downscale_box(image, target.width, target.height)
    } else {
        image::imageops::resize(image, target.width, target.height, FilterType::Triangle)
    }
}

/// 每个输出像素覆盖哪些源像素、各占多少（权重单位 1/256，每行合计 256）。
///
/// 两端带分数权重，所以输出像素的边缘与源图的边缘严格对齐 —— 这一点是
/// 「洞的边缘不能渗到相邻像素」的前提。
fn axis_weights(src: u32, dst: u32) -> Vec<Vec<(u32, u32)>> {
    let ratio = src as f64 / dst as f64;
    let last = src.saturating_sub(1) as i64;
    let mut out = Vec::with_capacity(dst as usize);
    for i in 0..dst {
        let a = i as f64 * ratio;
        let b = (i as f64 + 1.0) * ratio;
        let mut w: Vec<(u32, u32)> = Vec::with_capacity(4);
        let mut covers: Vec<(u32, f64)> = Vec::with_capacity(4);
        let mut x = a.floor() as i64;
        let mut total = 0.0f64;
        while (x as f64) < b {
            let cov = (b.min((x + 1) as f64) - a.max(x as f64)).max(0.0);
            if cov > 0.0 {
                covers.push((x.clamp(0, last) as u32, cov));
                total += cov;
            }
            x += 1;
        }
        if covers.is_empty() {
            out.push(vec![(i.min(src - 1), 256)]);
            continue;
        }
        let scale = 256.0 / total;
        let mut sum = 0u32;
        for &(ix, cov) in &covers {
            let weight = (cov * scale).round() as u32;
            sum += weight;
            w.push((ix, weight));
        }
        // 浮点四舍五入的余差补到权重最大的一项上，保证每行合计正好 256。
        if sum != 256 {
            if let Some(idx) = (0..w.len()).max_by_key(|&k| w[k].1) {
                w[idx].1 = (w[idx].1 as i64 + 256 - sum as i64).max(1) as u32;
            }
        }
        out.push(w);
    }
    out
}

/// 精确面积平均（box）降采样。
///
/// 竖直方向先把一趟源行按权重累加到一行缓冲，再做水平加权，所以每个源
/// 像素只读一次、且按行顺序读（缓存友好）。权重合计 256×256，据此回到
/// 0..255。行为等价于「每个输出像素是其源区间的面积平均」。
pub(super) fn downscale_box(image: &RgbaImage, nw: u32, nh: u32) -> RgbaImage {
    let (w, h) = image.dimensions();
    if nw == 0 || nh == 0 {
        return RgbaImage::new(nw, nh);
    }
    let xw = axis_weights(w, nw);
    let yw = axis_weights(h, nh);
    let raw = image.as_raw();
    let stride = w as usize * 4;
    let mut acc = vec![0u32; stride];
    let mut buf = vec![0u8; nw as usize * nh as usize * 4];
    for (oy, yws) in yw.iter().enumerate() {
        acc.fill(0);
        for &(sy, wy) in yws {
            let row = &raw[sy as usize * stride..][..stride];
            for (a, s) in acc.iter_mut().zip(row.iter()) {
                *a += *s as u32 * wy;
            }
        }
        let orow = &mut buf[oy * nw as usize * 4..][..nw as usize * 4];
        for (ox, xws) in xw.iter().enumerate() {
            let mut sum = [0u32; 4];
            for &(sx, wx) in xws {
                let base = sx as usize * 4;
                for c in 0..4 {
                    sum[c] += acc[base + c] * wx;
                }
            }
            for c in 0..4 {
                orow[ox * 4 + c] = ((sum[c] + 32768) / 65536).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(nw, nh, buf).expect("downscale_box buffer size")
}

/// True when at least one pixel is not fully opaque (cheapest possible scan;
/// a fully opaque bitmap never needs the premultiplied round trip).
fn has_transparent_pixel(image: &RgbaImage) -> bool {
    image.as_raw().chunks_exact(4).any(|px| px[3] != 255)
}

/// Straight alpha -> premultiplied alpha (in a fresh buffer).
fn premultiply(image: &RgbaImage) -> RgbaImage {
    let mut out = image.clone();
    for px in out.pixels_mut() {
        let [r, g, b, a] = px.0;
        if a != 255 && a != 0 {
            *px = image::Rgba([
                ((r as u32 * a as u32 + 127) / 255) as u8,
                ((g as u32 * a as u32 + 127) / 255) as u8,
                ((b as u32 * a as u32 + 127) / 255) as u8,
                a,
            ]);
        } else if a == 0 {
            *px = image::Rgba([0, 0, 0, 0]);
        }
    }
    out
}

/// 把 GPU 三明治纹理还原成 straight alpha 再交给 QML。
///
/// `PreviewComposer::blend` 的 Normal 快路径把 RGB 写成
/// `src.rgb*sa + dst.rgb*inv`，alpha 写成 `sa + da*inv` —— 也就是**预乘**
/// 表示。CPU 预览的最终底图不透明（alpha=255），预乘与直通一致，所以看
/// 不出问题；但 GPU 拖动把 static/active/above 分别合成到**透明底**，这些
/// 纹理的半透明像素 RGB 已经被 alpha 乘过一遍，BMP 交给 QML `Image` 后按
/// straight alpha 解释 —— 半透明白色描边（rgb≈255、a≈80）显示成 rgb≈80
/// 的深灰，就是用户看到的「拖动时描边变黑 / 文字像缩了一圈」。淡出帧里
/// 这张偏暗的 GPU 图再和正确的 CPU 帧交叉，半透明处叠加变深。
///
/// 只在 GPU 纹理出口做这一步，不改合成器本身：CPU 路径的输出口径不能动。
pub fn straighten_texture(image: &RgbaImage) -> RgbaImage {
    unpremultiply(image)
}

/// Premultiplied alpha -> straight alpha.
fn unpremultiply(image: &RgbaImage) -> RgbaImage {
    let mut out = image.clone();
    for px in out.pixels_mut() {
        let [r, g, b, a] = px.0;
        if a == 255 || a == 0 {
            if a == 0 {
                *px = image::Rgba([0, 0, 0, 0]);
            }
            continue;
        }
        let f = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
        *px = image::Rgba([f(r), f(g), f(b), a]);
    }
    out
}

pub fn crop_region(image: &RgbaImage, region: Rect) -> RgbaImage {
    let bounds = Rect::new(0, 0, image.width(), image.height());
    let Some(visible) = region.intersect(bounds) else {
        return RgbaImage::new(0, 0);
    };
    image::imageops::crop_imm(
        image,
        visible.x as u32,
        visible.y as u32,
        visible.width,
        visible.height,
    )
    .to_image()
}
