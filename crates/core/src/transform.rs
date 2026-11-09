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
    let (cw, ch) = (canvas.width as i32, canvas.height as i32);
    // Marks live in canvas coordinates: same mapping as layer offsets.
    let (cwf, chf) = (canvas.width as f32, canvas.height as f32);
    let map_pt = |x: f32, y: f32| -> (f32, f32) {
        match transform {
            Transform::Rotate90Cw => (chf - y, x),
            Transform::Rotate90Ccw => (y, cwf - x),
            Transform::Rotate180 => (cwf - x, chf - y),
            Transform::FlipHorizontal => (cwf - x, y),
            Transform::FlipVertical => (x, chf - y),
        }
    };

    // 背景层：图像按 `base` 语义旋转 / 翻转，`view` 在 base 坐标里跟着变换，
    // offset 由新的 view 反推 —— 与统一模型之前的几何完全一致。
    let base_before = Size::new(
        doc.layers[0].image().map(|i| i.width()).unwrap_or(canvas.width),
        doc.layers[0].image().map(|i| i.height()).unwrap_or(canvas.height),
    );
    doc.layers[0].ensure_baked();
    if let Some(r) = doc.layers[0].raster_mut() {
        r.image = transform.apply_to_image(&r.image);
    }
    doc.view = transform_rect(doc.view, transform, base_before);
    if let Some(r) = doc.layers[0].raster_mut() {
        r.offset = (-doc.view.x, -doc.view.y);
    }

    // 其余图层：位图按画布尺寸做同样的刚体变换，标记走点位映射。
    for layer in doc.layers.iter_mut().skip(1) {
        match &mut layer.content {
            LayerContent::Raster(r) => {
                let bounds = Rect::new(
                    r.offset.0,
                    r.offset.1,
                    r.image.width(),
                    r.image.height(),
                );
                let (ox, oy) = (bounds.x, bounds.y);
                let (lw, lh) = (bounds.width as i32, bounds.height as i32);
                // 文档级旋转 / 翻转直接改每层位图，先烘焙掉非破坏性状态。
                r.transform = None;
                r.image = transform.apply_to_image(&r.image);
                r.offset = match transform {
                    Transform::Rotate90Cw => (ch - (oy + lh), ox),
                    Transform::Rotate90Ccw => (oy, cw - (ox + lw)),
                    Transform::Rotate180 => (cw - (ox + lw), ch - (oy + lh)),
                    Transform::FlipHorizontal => (cw - (ox + lw), oy),
                    Transform::FlipVertical => (ox, ch - (oy + lh)),
                };
            }
            LayerContent::Mark(m) => {
                let (x0, y0) = map_pt(m.rect.0, m.rect.1);
                let (x1, y1) = map_pt(m.rect.2, m.rect.3);
                m.rect = norm_rect((x0, y0, x1, y1));
                for p in &mut m.points {
                    *p = map_pt(p.0, p.1);
                }
            }
        }
    }
    if transform.swaps_axes() {
        doc.canvas = Size::new(canvas.height, canvas.width);
    }
}

/// 旋转后的包围盒尺寸（向上取整，不裁切内容）。
///
/// ★必须先吸掉浮点残差再 ceil：`sin(180°)` 是 1.22e-16 而不是 0，
/// `800*1 + 600*1.2e-16` 算出来是 800.0000000000001，直接 ceil 就是 **801**。
/// 于是每转 90°/180°/360°，图层就长 1px —— 用户把图层转一圈，四边都胖了
/// 好几像素（"旋转一圈后四边越来越大"）。减去 1‰ 像素只影响远小于一像素的
/// 残差，真正需要裁切的角度（如 37.0001°）仍然照样进一位。
fn expanded_size(w: f32, h: f32, sin: f32, cos: f32) -> (u32, u32) {
    // ★必须**向上取偶**（多 1px 透明边），奇偶性直接决定画面会不会抖：
    //
    // 摆放是 `round(center - W/2)`（图层 offset 只能是整数），所以内容中心
    // 实际落在 `round(center - W/2) + W/2`。W 为偶数时 W/2 是整数，这个值
    // 恒等于 `round(center)`，与旋转角度无关；W 为奇数时它变成 `round(... )
    // + 0.5`，落在半像素上。
    //
    // 而 AABB 宽高随角度逐度变化（奇偶交替），拖动帧每帧都用当前角度重算
    // 中心 → 中心在 `.0 / .5` 之间来回跳 → 整层每帧跳 1px（用户 11-08 报的
    // 「持续旋转时图层来回抖动，放大 200 多倍才看清」）。取偶之后中心是常量，
    // 拖动帧与落定帧也锚在同一个点，松手不再平移。
    let cell = |v: f32| {
        let n = ((v - 1e-3).ceil() as i64).max(1) as u32;
        n + (n & 1)
    };
    (
        cell(w * cos.abs() + h * sin.abs()),
        cell(w * sin.abs() + h * cos.abs()),
    )
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
        expanded_size(w, h, sin, cos)
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
        expanded_size(w, h, sin, cos)
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
    // 新左上角相对旧左上角的位移（调用方拿它平移图层 offset）。offset 只能
    // 取整，所以这里先 round 定死，再让采样中心跟着它走。
    let delta = (
        ((w - nw as f32) / 2.0).round() as i32,
        ((h - nh as f32) / 2.0).round() as i32,
    );
    let (scx, scy) = (w / 2.0, h / 2.0);
    // ★中心用 `scx - delta` 反推，**不能**用 `nw / 2`。两者只在 (w-nw) 为
    // 偶数时相等；奇数时差半个像素，原本该精确映射的 90°/180° 会整体偏
    // 0.5px 并触发插值——叠加到 offset 的取整误差上，就是用户看到的
    // "放手后位置有微小偏移"，以及转几次就发软。用 delta 反推后，源图中
    // 心恰好落在「旧位置 + delta」上，和图层的整数 offset 严格一致。
    let (dcx, dcy) = (scx - delta.0 as f32, scy - delta.1 as f32);
    (rotate_to_at(img, sin, cos, nw, nh, hq, dcx, dcy), delta)
}

/// 旋转到指定尺寸，并让**源图中心**落在输出图连续坐标 `center` 上。
///
/// 拖动帧专用：落定帧的内容中心是 `round(center - AABB/2) + AABB/2`（外接框
/// 宽高为奇数时带 0.5px），而拖动帧若死守「源图中心」，两者就会差 0.5~1px，
/// 松手瞬间整层肉眼看得出挪一下。这里把落定的那个亚像素中心原样喂进来，
/// 于是拖动帧与落定帧的采样网格锚在同一点，`sx = 1` 时两者逐像素等价。
pub fn rotate_into_centered(
    img: &RgbaImage,
    sin: f32,
    cos: f32,
    nw: u32,
    nh: u32,
    center: (f32, f32),
) -> RgbaImage {
    let nw = nw.max(1);
    let nh = nh.max(1);
    if img.width() < 1 || img.height() < 1 {
        return RgbaImage::new(nw, nh);
    }
    rotate_to_at(img, sin, cos, nw, nh, false, center.0, center.1)
}

/// 采样实现：`dcx/dcy` = 源图中心在输出图连续坐标里的位置（可为小数）。
fn rotate_to_at(
    img: &RgbaImage,
    sin: f32,
    cos: f32,
    nw: u32,
    nh: u32,
    hq: bool,
    dcx: f32,
    dcy: f32,
) -> RgbaImage {
    let nw = nw.max(1);
    let nh = nh.max(1);
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let (scx, scy) = (iw as f32 / 2.0, ih as f32 / 2.0);
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
    out
}

/// 内容中心的统一量化：四舍五入到 **+∞**（`floor(x + 0.5)`）。
///
/// ★**禁止**在这里用 `f64::round()`：它对 `.5` 是「远离零」，正负两侧方向
/// 相反。拖动帧的输出是「对角线正方形」，`center - nw/2` 经常是**负数**，
/// 而落定帧的紧致外接框里同一个表达式是正数 —— 于是同一个中心被量化成两个
/// 值：正侧 175.5→176（内容中心 176），负侧 -43.5→-44（内容中心 175），
/// 松手瞬间整层平移 1px（用户 11-08 报「松手后位置还是会移动」）。
/// 统一成朝 +∞ 之后，两边逐位一致。
pub fn round_center(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// 整段旋转手势里恒定不变的输出尺寸：取对角线长度的正方形——
/// 任何角度下的包围盒都装得下，而尺寸与角度无关。`expand=false`（不扩边界）
/// 时直接用原尺寸。
///
/// ★尺寸再**向上取偶并多留 2px**：偶数让内容中心恒等于 `round(center)`
/// （见 [`expanded_size`]），2px 余量吸收那半个像素的居中偏差，避免贴边的那
/// 一行/一列被裁掉。
pub fn rotated_box_size(w: u32, h: u32, expand: bool) -> (u32, u32) {
    if !expand {
        return (w.max(1), h.max(1));
    }
    let d = (((w as f32) * (w as f32) + (h as f32) * (h as f32)).sqrt() - 1e-3).ceil() as u32;
    let d = d + (d & 1) + 2;
    (d.max(4), d.max(4))
}

/// 落定时的**紧致**外接框尺寸（与 `rotate_image_hq(.., expand=true)` 的输出
/// 逐位一致，同一套 f32 运算）。
///
/// 拖动帧要提前知道这个尺寸：落定的内容中心是
/// `round(center - AABB/2) + AABB/2`，外接框宽高为奇数时会落在半像素上，
/// 拖动帧必须跟着落在同一个点，松手才不会跳。
pub fn rotated_aabb_size(w: u32, h: u32, degrees: f32) -> (u32, u32) {
    let rad = degrees.to_radians();
    expanded_size(w as f32, h as f32, rad.sin(), rad.cos())
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

// ---------------------------------------------------------------------------
// 共享仿射取样器（P0-1 / §2.3 / §11 Tier 0）
//
// core 的 flatten/烘焙与 render 的预览必须共用这一个函数，禁止再出现
// 「预览 Nearest / 提交 Lanczos3」两条实现（D14 / R11）。与旋转路径一样，
// 全程预乘 alpha 累加、最后 normalize 反预乘，透明边缘不会泛黑。
//
// `inverse` 是「画布连续坐标 → 源图连续坐标」的逆变换；输出矩形 `out` 用
// 画布坐标，可为负 / 越出画布（调用方负责取 `bounds ∩ canvas`）。
// 逆变换按行增量步进：每行只算一次矩阵乘法，x 方向加常量 `(a, b)`。
// ---------------------------------------------------------------------------

struct AffineCtx {
    iw: i32,
    ih: i32,
    stride: usize,
    nstride: usize,
    nw: u32,
    out_x: f32,
    out_y: f32,
    ia: f32,
    ib: f32,
    ic: f32,
    id: f32,
    ie: f32,
    i_f: f32,
    hq: bool,
}

/// 逆变换取样：`hq=false` 双线性（拖动临时帧），`true` Catmull-Rom（落定/导出）。
/// 输出尺寸恒为 `out.width × out.height`，`out` 之外的区域不采样。
pub fn sample_affine(src: &RgbaImage, inverse: &Affine2, out: Rect, hq: bool) -> RgbaImage {
    let mut dst = RgbaImage::new(out.width, out.height);
    let (iw, ih) = (src.width() as i32, src.height() as i32);
    if out.is_empty() || iw < 1 || ih < 1 {
        return dst;
    }
    let ctx = AffineCtx {
        iw,
        ih,
        stride: iw as usize * 4,
        nstride: out.width as usize * 4,
        nw: out.width,
        out_x: out.x as f32,
        out_y: out.y as f32,
        ia: inverse.a as f32,
        ib: inverse.b as f32,
        ic: inverse.c as f32,
        id: inverse.d as f32,
        ie: inverse.e as f32,
        i_f: inverse.f as f32,
        hq,
    };
    let threads = rot_threads(out.height);
    let source: &[u8] = src.as_raw();
    let dest: &mut [u8] = &mut dst;
    if threads > 1 {
        let rows = (out.height as usize).div_ceil(threads);
        let cref = &ctx;
        std::thread::scope(|scope| {
            for (ti, chunk) in dest.chunks_mut(rows * cref.nstride).enumerate() {
                let dy0 = ti * rows;
                scope.spawn(move || render_affine_rows(chunk, dy0, source, cref));
            }
        });
    } else {
        render_affine_rows(dest, 0, source, &ctx);
    }
    dst
}

fn render_affine_rows(dst: &mut [u8], dy0: usize, src: &[u8], c: &AffineCtx) {
    for (r, row) in dst.chunks_mut(c.nstride).enumerate() {
        let cy = c.out_y + (dy0 + r) as f32 + 0.5;
        let cx = c.out_x + 0.5;
        // 目标像素中心 → 源图像素中心；`-0.5` 把连续坐标换成取样网格坐标。
        let mut sx = c.ia * cx + c.ic * cy + c.ie - 0.5;
        let mut sy = c.ib * cx + c.id * cy + c.i_f - 0.5;
        for dx in 0..c.nw as usize {
            let pixel = if c.hq {
                sample_hq(src, c, sx, sy)
            } else {
                sample_bilinear(src, c, sx, sy)
            };
            row[dx * 4..dx * 4 + 4].copy_from_slice(&pixel);
            sx += c.ia;
            sy += c.ib;
        }
    }
}

#[inline]
fn sample_bilinear(src: &[u8], c: &AffineCtx, sx: f32, sy: f32) -> [u8; 4] {
    let (fx, fy) = (sx.floor(), sy.floor());
    let (x0, y0) = (fx as i32, fy as i32);
    let (tx, ty) = (sx - fx, sy - fy);
    let mut acc = [0f32; 3];
    let mut a_sum = 0f32;
    for &(ox, oy, w) in &[
        (0, 0, (1.0 - tx) * (1.0 - ty)),
        (1, 0, tx * (1.0 - ty)),
        (0, 1, (1.0 - tx) * ty),
        (1, 1, tx * ty),
    ] {
        if w <= 0.0 {
            continue;
        }
        let (x, y) = (x0 + ox, y0 + oy);
        if x < 0 || y < 0 || x >= c.iw || y >= c.ih {
            continue;
        }
        let i = y as usize * c.stride + x as usize * 4;
        let a = src[i + 3] as f32 * w;
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

#[inline]
fn sample_hq(src: &[u8], c: &AffineCtx, sx: f32, sy: f32) -> [u8; 4] {
    let (fx, fy) = (sx.floor(), sy.floor());
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
    normalize(acc, a_sum)
}

#[cfg(test)]
mod affine_sample_tests {
    use super::*;
    use crate::layer_transform::LayerTransform;

    fn checker() -> RgbaImage {
        RgbaImage::from_fn(64, 48, |x, y| {
            if (x / 8 + y / 8) % 2 == 0 {
                image::Rgba([230, 30, 40, 255])
            } else {
                image::Rgba([20, 40, 220, 255])
            }
        })
    }

    #[test]
    fn identity_is_pixel_exact_for_both_quality_modes() {
        let src = checker();
        let inv = Affine2::identity();
        for hq in [false, true] {
            let out = sample_affine(&src, &inv, Rect::new(0, 0, 64, 48), hq);
            assert_eq!(out.as_raw(), src.as_raw(), "hq={hq}");
        }
    }

    #[test]
    fn identity_is_pixel_exact_on_threaded_rows() {
        // 高度 >= 96 才会走 std::thread::scope 分行；逐行 dy0 错位时这里会红。
        let src = RgbaImage::from_fn(120, 130, |x, y| {
            image::Rgba([(x * 2) as u8, (y * 2) as u8, ((x + y) % 256) as u8, 255])
        });
        let out = sample_affine(&src, &Affine2::identity(), Rect::new(0, 0, 120, 130), false);
        assert_eq!(out.as_raw(), src.as_raw());
    }

    #[test]
    fn integer_translation_is_lossless() {
        let src = checker();
        let inv = Affine2::translation(-7.0, -11.0);
        let out = sample_affine(&src, &inv, Rect::new(7, 11, 64, 48), false);
        assert_eq!(out.as_raw(), src.as_raw());
    }

    #[test]
    fn out_rect_may_be_negative_and_is_clipped_to_requested_size() {
        let src = checker();
        let out = sample_affine(&src, &Affine2::identity(), Rect::new(-5, -5, 10, 10), false);
        assert_eq!((out.width(), out.height()), (10, 10));
        // 只有 (5,5)..(10,10) 与源图相交，其余全透明。
        assert_eq!(*out.get_pixel(5, 5), *src.get_pixel(0, 0));
        assert_eq!(*out.get_pixel(0, 0), image::Rgba([0, 0, 0, 0]));
    }

    #[test]
    fn transparent_pixels_never_get_black_halo() {
        let mut src = RgbaImage::new(8, 8);
        src.put_pixel(3, 3, image::Rgba([250, 10, 10, 255]));
        let inv = Affine2::translation(-0.5, -0.5);
        let out = sample_affine(&src, &inv, Rect::new(0, 0, 8, 8), false);
        // 完全透明的邻域必须仍是 (0,0,0,0)，而不是被 unpremultiply 放大成噪点。
        assert_eq!(*out.get_pixel(0, 0), image::Rgba([0, 0, 0, 0]));
        let p = out.get_pixel(3, 3);
        assert!(p[3] > 0 && p[0] > 180, "半透明插值不该把红色染黑: {p:?}");
    }

    #[test]
    fn layer_transform_render_matches_bounds_geometry() {
        let mut t = LayerTransform::from_offset(checker(), 10, 20);
        t.rotation = 30.0;
        let (img, origin) = t.render_bounds(true);
        let r = t.bounds().round_out();
        assert_eq!(origin, (r.x, r.y));
        assert_eq!((img.width(), img.height()), (r.width, r.height));
        // 中心像素不该是 transparent（源图全不透明）。
        let (cx, cy) = t.bounds().center();
        let px = ((cx - r.x as f64) as u32).min(img.width() - 1);
        let py = ((cy - r.y as f64) as u32).min(img.height() - 1);
        assert!(img.get_pixel(px, py)[3] > 0);
    }
}

