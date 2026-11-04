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
///
/// 性能：直接读写 `[u8]` 原始缓冲（`get_pixel`/`put_pixel` 每像素都要过一遍
/// 泛型 `Pixel` 转换，拖动时每 60ms 就要重算整层，那点开销会翻几倍）。四邻域
/// 全在图内时走无边界检查的快路径，只有贴边的像素才逐点判越界。
pub fn rotate_image(img: &RgbaImage, degrees: f32, expand: bool) -> (RgbaImage, (i32, i32)) {
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    if iw < 1 || ih < 1 || degrees.abs() < 0.001 {
        return (img.clone(), (0, 0));
    }
    let (w, h) = (iw as f32, ih as f32);
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
    rotate_into(img, sin, cos, nw, nh)
}

/// 高质量旋转：Catmull-Rom（双三次）重采样，仍然预乘 alpha。
///
/// 只在「角度落定」时用（松手提交、角度对话框、一次性旋转图层）。双线性
/// 只用 2×2 个 tap，等效于一次低通，转完照片/线条会明显发软——用户看到的
/// "转一下就变模糊"就是它。双三次用 4×4 个 tap 并带负瓣，能把中高频补回来，
/// 代价是 4 倍采样，但只算一次，摊不进拖动帧。
pub fn rotate_image_hq(img: &RgbaImage, degrees: f32, expand: bool) -> (RgbaImage, (i32, i32)) {
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    if iw < 1 || ih < 1 || degrees.abs() < 0.001 {
        return (img.clone(), (0, 0));
    }
    let (w, h) = (iw as f32, ih as f32);
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
    rotate_to(img, sin, cos, nw, nh, true)
}

/// 旋转到**指定尺寸**的输出画布（源图中心对齐画布中心）。
///
/// 拖动预览专用：整段拖拽里把输出尺寸钉死成一个常量（见 [`rotated_box_size`]），
/// 采样网格 `dcx/dcy` 和返回的偏移就都是常量——每帧只有角度在变。
/// 用 `rotate_image` 的话包围盒会随角度每帧 ±1px 变化，格子跟着跳半像素、
/// 偏移也跟着跳 1px，30fps 下整层看起来就在抖。
pub fn rotate_into(
    img: &RgbaImage,
    sin: f32,
    cos: f32,
    nw: u32,
    nh: u32,
) -> (RgbaImage, (i32, i32)) {
    rotate_to(img, sin, cos, nw, nh, false)
}

/// 旋转的核心：`hq=false` 走双线性（拖动用），`true` 走双三次（落定用）。
fn rotate_to(
    img: &RgbaImage,
    sin: f32,
    cos: f32,
    nw: u32,
    nh: u32,
    hq: bool,
) -> (RgbaImage, (i32, i32)) {
    let nw = nw.max(1);
    let nh = nh.max(1);
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    if iw < 1 || ih < 1 {
        return (RgbaImage::new(nw, nh), (0, 0));
    }
    let (w, h) = (iw as f32, ih as f32);
    let (scx, scy) = (w / 2.0, h / 2.0);
    let (dcx, dcy) = (nw as f32 / 2.0, nh as f32 / 2.0);
    let mut out = RgbaImage::new(nw, nh);
    let ctx = RotCtx {
        iw,
        ih,
        stride: iw as usize * 4,
        nstride: nw as usize * 4,
        cos,
        sin,
        scx,
        scy,
        dcx,
        dcy,
        nw,
    };
    {
        let src: &[u8] = img.as_raw();
        let dst: &mut [u8] = &mut out;
        // 按输出行切给几个线程：拖手柄时每帧都要重算整层，单线程在 debug
        // 下 2752×1152 要 79ms，超出节流周期，手感就成了「一下一下跳」。
        let threads = rot_threads(nh);
        if threads > 1 {
            let rows = (nh as usize).div_ceil(threads);
            let cref = &ctx;
            std::thread::scope(|scope| {
                for (ti, chunk) in dst.chunks_mut(rows * cref.nstride).enumerate() {
                    let dy0 = ti * rows;
                    scope.spawn(move || {
                        if hq {
                            render_rows_hq(chunk, dy0, src, cref);
                        } else {
                            render_rows(chunk, dy0, src, cref);
                        }
                    });
                }
            });
        } else if hq {
            render_rows_hq(dst, 0, src, &ctx);
        } else {
            render_rows(dst, 0, src, &ctx);
        }
    }
    let delta = (
        ((w - nw as f32) / 2.0).round() as i32,
        ((h - nh as f32) / 2.0).round() as i32,
    );
    (out, delta)
}

/// 整段旋转手势里恒定不变的输出尺寸：取对角线长度的正方形——
/// 任何角度下的包围盒都装得下，而尺寸与角度无关。`expand=false`（不扩边界）
/// 时直接用原尺寸。
pub fn rotated_box_size(w: u32, h: u32, expand: bool) -> (u32, u32) {
    if !expand {
        return (w.max(1), h.max(1));
    }
    let d = ((w as f32) * (w as f32) + (h as f32) * (h as f32))
        .sqrt()
        .ceil() as u32;
    (d.max(1), d.max(1))
}

/// 一次旋转运算的常量参数（各线程共享只读）。
struct RotCtx {
    iw: i32,
    ih: i32,
    stride: usize,
    nstride: usize,
    cos: f32,
    sin: f32,
    scx: f32,
    scy: f32,
    dcx: f32,
    dcy: f32,
    nw: u32,
}

fn rot_threads(nh: u32) -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let n = *N.get_or_init(|| {
        std::thread::available_parallelism()
            .map(|v| v.get())
            .unwrap_or(1)
            .min(8)
    });
    // 太矮的图分线程得不偿失（线程开销比计算还大）。
    if nh < 96 {
        1
    } else {
        n
    }
}

/// Catmull-Rom 一维权重（a = -0.5）。带负瓣，正是它把双线性糊掉的
/// 中高频补回来；四个权重之和恒为 1。
#[inline]
fn catmull(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.5 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    ]
}

/// 双三次（Catmull-Rom）行渲染：4×4 个 tap，预乘 alpha 累加。
///
/// 负权会让累加出的 alpha 略微越界（负瓣），所以末了统一 clamp；alpha 接近
/// 0 的像素直接判透明，避免 unpremultiply 放大噪点。
fn render_rows_hq(dst: &mut [u8], dy0: usize, src: &[u8], c: &RotCtx) {
    for (r, row) in dst.chunks_mut(c.nstride).enumerate() {
        let ry = (dy0 + r) as f32 + 0.5 - c.dcy;
        for dx in 0..c.nw as usize {
            let rx = dx as f32 + 0.5 - c.dcx;
            let sx = c.cos * rx + c.sin * ry + c.scx - 0.5;
            let sy = -c.sin * rx + c.cos * ry + c.scy - 0.5;
            let fx = sx.floor();
            let fy = sy.floor();
            let (x0, y0) = (fx as i32, fy as i32);
            let wx = catmull(sx - fx);
            let wy = catmull(sy - fy);
            let mut acc = [0f32; 3];
            let mut a_sum = 0f32;
            for (j, wyv) in wy.iter().enumerate() {
                let y = y0 - 1 + j as i32;
                if y < 0 || y >= c.ih || *wyv == 0.0 {
                    continue;
                }
                let base = y as usize * c.stride;
                for (i, wxv) in wx.iter().enumerate() {
                    let x = x0 - 1 + i as i32;
                    if x < 0 || x >= c.iw {
                        continue;
                    }
                    let w = wyv * wxv;
                    if w == 0.0 {
                        continue;
                    }
                    let p = base + x as usize * 4;
                    let a = src[p + 3] as f32 * w;
                    if a == 0.0 {
                        continue;
                    }
                    acc[0] += src[p] as f32 * a;
                    acc[1] += src[p + 1] as f32 * a;
                    acc[2] += src[p + 2] as f32 * a;
                    a_sum += a;
                }
            }
            let pixel = if a_sum > 0.00255 {
                let inv = 1.0 / a_sum;
                [
                    (acc[0] * inv).round().clamp(0.0, 255.0) as u8,
                    (acc[1] * inv).round().clamp(0.0, 255.0) as u8,
                    (acc[2] * inv).round().clamp(0.0, 255.0) as u8,
                    a_sum.round().clamp(0.0, 255.0) as u8,
                ]
            } else {
                [0, 0, 0, 0]
            };
            row[dx * 4..dx * 4 + 4].copy_from_slice(&pixel);
        }
    }
}

/// 把输出缓冲按行切块后逐块渲染。`dy0` 是本次负责的首行在整幅图上的行号。
fn render_rows(dst: &mut [u8], dy0: usize, src: &[u8], c: &RotCtx) {
    for (r, row) in dst.chunks_mut(c.nstride).enumerate() {
        let ry = (dy0 + r) as f32 + 0.5 - c.dcy;
        for dx in 0..c.nw as usize {
            // 目标像素中心 → 相对新中心 → 反向旋转 → 源图坐标（像素中心）。
            let rx = dx as f32 + 0.5 - c.dcx;
            let sx = c.cos * rx + c.sin * ry + c.scx - 0.5;
            let sy = -c.sin * rx + c.cos * ry + c.scy - 0.5;
            let fx = sx.floor();
            let fy = sy.floor();
            let x0 = fx as i32;
            let y0 = fy as i32;
            let tx = sx - fx;
            let ty = sy - fy;
            let pixel = if x0 >= 0 && y0 >= 0 && x0 + 1 < c.iw && y0 + 1 < c.ih {
                let i00 = y0 as usize * c.stride + x0 as usize * 4;
                let (w00, w10) = ((1.0 - tx) * (1.0 - ty), tx * (1.0 - ty));
                let (w01, w11) = ((1.0 - tx) * ty, tx * ty);
                let a00 = src[i00 + 3] as f32 * w00;
                let a10 = src[i00 + 7] as f32 * w10;
                let a01 = src[i00 + c.stride + 3] as f32 * w01;
                let a11 = src[i00 + c.stride + 7] as f32 * w11;
                blend4(src, i00, c.stride, (a00, a10, a01, a11))
            } else {
                edge_pixel(src, c, x0, y0, tx, ty)
            };
            row[dx * 4..dx * 4 + 4].copy_from_slice(&pixel);
        }
    }
}

/// 贴边的像素：四个 tap 逐个判越界（数量少，慢一点无所谓）。
fn edge_pixel(src: &[u8], c: &RotCtx, x0: i32, y0: i32, tx: f32, ty: f32) -> [u8; 4] {
    let mut acc = [0f32; 3];
    let mut a_sum = 0f32;
    for &(ox, oy, wt) in &[
        (0, 0, (1.0 - tx) * (1.0 - ty)),
        (1, 0, tx * (1.0 - ty)),
        (0, 1, (1.0 - tx) * ty),
        (1, 1, tx * ty),
    ] {
        if wt <= 0.0 {
            continue;
        }
        let (x, y) = (x0 + ox, y0 + oy);
        if x < 0 || y < 0 || x >= c.iw || y >= c.ih {
            continue;
        }
        let i = y as usize * c.stride + x as usize * 4;
        let a = src[i + 3] as f32 * wt;
        if a <= 0.0 {
            continue;
        }
        acc[0] += src[i] as f32 * a;
        acc[1] += src[i + 1] as f32 * a;
        acc[2] += src[i + 2] as f32 * a;
        a_sum += a;
    }
    normalize(acc, a_sum)
}

/// 四个 tap 都在图内时的快路径（调用方已保证不越界）。
#[inline]
fn blend4(src: &[u8], i00: usize, stride: usize, a: (f32, f32, f32, f32)) -> [u8; 4] {
    let (a00, a10, a01, a11) = a;
    let offs = [i00, i00 + 4, i00 + stride, i00 + stride + 4];
    let ws = [a00, a10, a01, a11];
    let mut acc = [0f32; 3];
    for n in 0..4 {
        let wt = ws[n];
        if wt <= 0.0 {
            continue;
        }
        let i = offs[n];
        acc[0] += src[i] as f32 * wt;
        acc[1] += src[i + 1] as f32 * wt;
        acc[2] += src[i + 2] as f32 * wt;
    }
    normalize(acc, a00 + a10 + a01 + a11)
}

/// 把加权和还原成 straight-alpha 的像素。
#[inline]
fn normalize(acc: [f32; 3], a_sum: f32) -> [u8; 4] {
    if a_sum <= 0.00255 {
        return [0, 0, 0, 0];
    }
    let inv = 1.0 / a_sum;
    [
        (acc[0] * inv).round().clamp(0.0, 255.0) as u8,
        (acc[1] * inv).round().clamp(0.0, 255.0) as u8,
        (acc[2] * inv).round().clamp(0.0, 255.0) as u8,
        a_sum.round().clamp(0.0, 255.0) as u8,
    ]
}
