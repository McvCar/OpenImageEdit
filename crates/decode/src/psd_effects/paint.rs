use image::{Rgba, RgbaImage};
use psd::effects::{EffectKind, GlowSource, LayerEffect, StrokeFill, StrokePosition};

use super::gradient_t;
use super::legacy;
use super::ops::{
    box_blur, dilate, dist_inside, dist_outside, shift_mask, spread_split, tent_blur, INSIDE_EPS,
};
use super::skip_kind;

fn composite(
    out: &mut RgbaImage,
    x: u32,
    y: u32,
    color: [u8; 3],
    coverage: f32,
    opacity: f32,
    mode: editcore::BlendMode,
) {
    let a = (coverage.clamp(0.0, 1.0) * opacity.clamp(0.0, 1.0) * 255.0).round();
    if a <= 0.0 {
        return;
    }
    if trace_at(x, y) {
        eprintln!(
            "fx-composite [{}] ({x},{y}) color {color:?} cov {coverage:.3} op {opacity:.3} mode {mode:?}",
            trace_label()
        );
    }
    let w = out.width();
    record_effect_coverage(x, y, w, a);
    let idx = ((y * w + x) * 4) as usize;
    let data = out.as_mut();
    let mut px = [data[idx], data[idx + 1], data[idx + 2], data[idx + 3]];
    editcore::blend_pixel(
        &mut px,
        [color[0], color[1], color[2], a as u8],
        1.0,
        mode,
    );
    data[idx..idx + 4].copy_from_slice(&px);
}

thread_local! {
    /// 当前 `bake` 正在记录「哪些像素是样式画上去的」。用一个线程内的暂存区，
    /// 是为了不必给 8 个 `draw_*` 都加一个 out 参数——它们唯一共同的出口就是
    /// [`composite`]。`bake` 之外始终是 `None`，[`composite`] 于是零成本。
    pub(super) static FX_COVER: std::cell::RefCell<Option<Vec<f32>>> = const { std::cell::RefCell::new(None) };
    /// 同一时刻只记「背后」样式（投影 / 外发光）的那一份覆盖率。
    pub(super) static FX_BEHIND: std::cell::RefCell<Option<Vec<f32>>> = const { std::cell::RefCell::new(None) };
    /// 记「非背后」样式（内阴影、叠加、描边、斜面）。这些画在投影之上，
    /// 一个像素只要被它们碰过，露出来的颜色就不是投影自己的了。
    pub(super) static FX_FRONT: std::cell::RefCell<Option<Vec<f32>>> = const { std::cell::RefCell::new(None) };
    /// 当前是否在画「背后」样式。
    pub(super) static FX_IS_BEHIND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Composite an effect that **only recolours the pixels already there**.
///
/// 内阴影/内发光在 Photoshop 里是被内容 alpha 剪辑的：它们换掉已有像素的颜色，
/// 不新增不透明度。用 [`composite`]（source-over）画会把 alpha 叠上去 ——
/// 17.6% 的阴影贴图配 3px 内描边被画成 32%，描边色与**页面背景**混合，
/// 底部多压出一条暗带（实测该带 MAE 25.5，改后 21.0）。
///
/// 所以这里只按 `coverage × opacity` 的权重换色，alpha 原样保留。
///
/// 注意适用范围：渐变叠加**实测**是 source-over 的（改成换色后 `(128,864)`
/// 一带误差 +11，整体 3.329 -> 3.384），内描边同样（+0.03），不要顺手统一
/// 过来。颜色叠加另有自己的 [`composite_fill`]：那次 A/B 里换色用的是
/// `coverage × opacity` 当权重，软边像素只换了一半颜色，所以测出来“不如
/// source-over”，见那里的说明。
fn composite_inside(
    out: &mut RgbaImage,
    x: u32,
    y: u32,
    color: [u8; 3],
    coverage: f32,
    opacity: f32,
    mode: editcore::BlendMode,
) {
    let t = (coverage.clamp(0.0, 1.0) * opacity.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    if t <= 0.002 {
        return;
    }
    let w = out.width();
    record_effect_coverage(x, y, w, t * 255.0);
    let idx = ((y * w + x) * 4) as usize;
    let data = out.as_mut();
    if data[idx + 3] == 0 {
        return;
    }
    let mut px = [data[idx], data[idx + 1], data[idx + 2], 255];
    editcore::blend_pixel(&mut px, [color[0], color[1], color[2], 255], t, mode);
    data[idx] = px[0];
    data[idx + 1] = px[1];
    data[idx + 2] = px[2];
}

/// 颜色叠加：Photoshop 是把叠加色**填满图层的形状**，不透明度由图层自己的
/// alpha 决定，叠加本身不往上加 alpha。
///
/// [`composite`]（source-over）在这里是错的：它的不透明度权重取了
/// `coverage × opacity`，而颜色叠加的 coverage 就是图层 alpha。于是
///   * 半透明内容 + 白色叠加 → 先按预乘算出 `255 × a` 的灰，而不是白；
///   * alpha 还被叠成 `a + a(1-a)`，比内容本身更实。
/// 「增加时间」横幅里那条半透明白斜纹层（图层 84，内容 alpha≈0.4）就是这样
/// 被烘成 65% 的灰纱罩：黄色斜纹发橄榄、粉色描边变灰、整块横幅发暗，全是
/// 它盖出来的。逐像素对照见
/// `color_overlay_does_not_premultiply_semi_transparent_content`。
///
/// 正确语义：按 `opacity`（叠加自身的不透明度，渐变再乘色标 alpha）与已有
/// 内容混合，alpha 原样保留。完全没有内容的像素（组烘焙的 behind/front
/// 缓冲）没有可换的色，直接写叠加色、alpha = `coverage × opacity`。
fn composite_fill(
    out: &mut RgbaImage,
    x: u32,
    y: u32,
    color: [u8; 3],
    coverage: f32,
    opacity: f32,
    mode: editcore::BlendMode,
) {
    let cov = coverage.clamp(0.0, 1.0);
    let t = opacity.clamp(0.0, 1.0);
    if cov <= 0.002 || t <= 0.002 {
        return;
    }
    let w = out.width();
    record_effect_coverage(x, y, w, cov * t * 255.0);
    let idx = ((y * w + x) * 4) as usize;
    let data = out.as_mut();
    if data[idx + 3] == 0 {
        data[idx] = color[0];
        data[idx + 1] = color[1];
        data[idx + 2] = color[2];
        data[idx + 3] = (cov * t * 255.0).round().clamp(0.0, 255.0) as u8;
        return;
    }
    // 已有内容：只换色，alpha 不动。
    let mut px = [data[idx], data[idx + 1], data[idx + 2], 255];
    editcore::blend_pixel(&mut px, [color[0], color[1], color[2], 255], t, mode);
    data[idx..idx + 3].copy_from_slice(&px[..3]);
}

fn record_effect_coverage(x: u32, y: u32, w: u32, alpha: f32) {
    let i = (y * w + x) as usize;
    let a = (alpha / 255.0).clamp(0.0, 1.0);
    FX_COVER.with(|c| {
        if let Some(cov) = c.borrow_mut().as_mut() {
            if let Some(v) = cov.get_mut(i) {
                // 多次叠加取并集：1-(1-a)(1-b)，描边 + 内阴影叠在一起仍然是 1。
                *v = 1.0 - (1.0 - *v) * (1.0 - a);
            }
        }
    });
    let add = |cov: Option<&mut Vec<f32>>| {
        if let Some(v) = cov.and_then(|c| c.get_mut(i)) {
            *v = 1.0 - (1.0 - *v) * (1.0 - a);
        }
    };
    if FX_IS_BEHIND.with(|f| f.get()) {
        FX_BEHIND.with(|c| add(c.borrow_mut().as_mut()));
    } else {
        FX_FRONT.with(|c| add(c.borrow_mut().as_mut()));
    }
}

/// Diagnostic: `IMAGEEDIT_FX_TRACE=x,y` prints every style composite at that
/// pixel (buffer coordinates), so a wrong colour can be traced to the effect and
/// the value that produced it.
fn trace_at(x: u32, y: u32) -> bool {
    static TRACE: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    let at = TRACE.get_or_init(|| {
        let v = std::env::var("IMAGEEDIT_FX_TRACE").ok()?;
        let mut it = v.split(',');
        let x = it.next()?.trim().parse().ok()?;
        let y = it.next()?.trim().parse().ok()?;
        Some((x, y))
    });
    *at == Some((x, y))
}

/// The layer whose styles are being baked, for [`trace_at`]'s output: tracing a
/// pixel is useless without knowing which layer's buffer it is.
static TRACE_LABEL: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Name the layer that [`bake`] is about to render (diagnostics only).
pub fn set_trace_label(name: &str) {
    if let Ok(mut slot) = TRACE_LABEL.lock() {
        slot.clear();
        slot.push_str(name);
    }
}

pub(super) fn trace_label() -> String {
    TRACE_LABEL
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

pub(super) fn draw_drop_shadow(out: &mut RgbaImage, w: u32, h: u32, alpha: &[f32], e: &LayerEffect) {
    let (dx, dy) = offset_vector(e.distance, e.angle);
    let shifted = shift_mask(alpha, w, h, dx, dy, 0.0);
    // Photoshop splits Size into spread and blur: the spread grows the outline by
    // whole pixels first, the blur keeps what is left.
    let (grow, soft) = spread_split(e.size, e.choke);
    let grown = dilate(&shifted, w, h, grow);
    let blurred = tent_blur(&grown, w, h, soft);
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let mut cov = blurred[i];
            if e.knocks_out {
                cov *= 1.0 - alpha[i];
            }
            if cov <= 0.002 {
                continue;
            }
            composite(out, x, y, e.color, cov, e.opacity, mode);
        }
    }
}

pub(super) fn draw_outer_glow(out: &mut RgbaImage, w: u32, h: u32, alpha: &[f32], e: &LayerEffect) {
    // The default "Softer" technique: the shape's outline grown by the spread and
    // blurred, read through the default 50 % Range (which doubles the blurred
    // edge's 0.5 to full strength).
    let (grow, soft) = spread_split(e.size, e.choke);
    let grown = dilate(alpha, w, h, grow);
    let blurred = tent_blur(&grown, w, h, soft);
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let cov = (blurred[(y * w + x) as usize] * 2.0).min(1.0);
            if cov <= 0.002 {
                continue;
            }
            composite(out, x, y, e.color, cov, e.opacity, mode);
        }
    }
}

/// Ring coverage of a shape, given its per-pixel coverage.
///
/// `out_half` / `in_half` are the band widths outside / inside the contour, so a
/// 2 px outside stroke is `(2.0, 0.0)` and a 1 px centre stroke `(0.5, 0.5)`.
///
/// `dist_*` measure from the **pixel centre** to the contour, which sits half a
/// pixel outside a fully opaque border pixel's centre. A band of `size` px is
/// therefore `size + 0.5 - d` wide as seen from the centre; measuring from the
/// centre without that half pixel made every 1 px ring half transparent at the
/// edge and shaved the last row off thicker rings.
pub fn ring_coverage(alpha: &[f32], w: u32, h: u32, out_half: f32, in_half: f32) -> Vec<f32> {
    let dist_in = if in_half > 0.0 {
        dist_inside(alpha, w, h)
    } else {
        Vec::new()
    };
    let dist_out = if out_half > 0.0 {
        dist_outside(alpha, w, h)
    } else {
        Vec::new()
    };
    ring_coverage_from_fields(alpha, w, h, &dist_in, &dist_out, out_half, in_half)
}

fn ring_coverage_from_fields(
    alpha: &[f32],
    w: u32,
    h: u32,
    dist_in: &[f32],
    dist_out: &[f32],
    out_half: f32,
    in_half: f32,
) -> Vec<f32> {
    let (out_band, in_band) = stroke_bands(alpha, w, h, dist_in, dist_out, out_half, in_half);
    out_band
        .iter()
        .zip(in_band.iter())
        .zip(alpha.iter())
        .map(|((o, i), a)| o.max(i * a))
        .collect()
}

/// 描边环的「外半边」与「内半边」覆盖率，两个半边分开返回。
///
/// 外半边是**新增**像素（画到内容之外），内半边只**换掉**内容颜色：内阴影
/// 已经踩过这个坑 —— 17.6% 的阴影贴图配 3px 内描边，若把内半边也按 source-over
/// 叠加，alpha 会从 0.176 涨到 0.32，描边色与页面背景混成一条暗带。
/// 所以内半边返回的是「带宽本身」（不含内容 alpha的），由调用方用
/// [`composite_inside`] 换色。
fn stroke_bands(
    alpha: &[f32],
    w: u32,
    h: u32,
    dist_in: &[f32],
    dist_out: &[f32],
    out_half: f32,
    in_half: f32,
) -> (Vec<f32>, Vec<f32>) {
    const CONTOUR_HALF: f32 = 0.5;
    let mut out_band = vec![0.0f32; alpha.len()];
    let mut in_band = vec![0.0f32; alpha.len()];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let a = alpha[i];
            // The inside half of the band lies over the content.
            if in_half > 0.0 && a > INSIDE_EPS {
                let d = dist_in.get(i).copied().unwrap_or(0.0).max(0.0);
                in_band[i] = (in_half + CONTOUR_HALF - d).clamp(0.0, 1.0);
            }
            // The outside half follows the contour, which runs *through* an
            // antialiased border pixel: only its uncovered share is outside.
            if out_half > 0.0 && a < 1.0 - INSIDE_EPS {
                let d = dist_out.get(i).copied().unwrap_or(0.0).max(0.0);
                let band = (out_half + CONTOUR_HALF - d).clamp(0.0, 1.0);
                let share = if a <= INSIDE_EPS { 1.0 } else { 1.0 - a };
                out_band[i] = band * share;
            }
        }
    }
    (out_band, in_band)
}

pub(super) fn draw_stroke(
    out: &mut RgbaImage,
    w: u32,
    h: u32,
    alpha: &[f32],
    dist_in: &[f32],
    dist_out: &[f32],
    bounds: [f32; 4],
    e: &LayerEffect,
    clip: Option<&[f32]>,
) {
    let size = e.size.max(0.5);
    let mode = blend_of(e.blend);
    let gradient = e.gradient.as_ref().filter(|_| e.stroke_fill == StrokeFill::Gradient);
    // The band is measured from the shape's edge outwards / inwards, so an
    // outside stroke is exactly `size` px wide whatever the edge's angle.
    let (out_half, in_half) = match e.stroke_position {
        StrokePosition::Outside => (size, 0.0),
        StrokePosition::Inside => (0.0, size),
        StrokePosition::Center => (size / 2.0, size / 2.0),
    };
    let (cov_out, cov_in) =
        stroke_bands(alpha, w, h, dist_in, dist_out, out_half, in_half);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let (color, mut a) = match gradient {
                Some(g) => {
                    let t = gradient_t(x as f32, y as f32, bounds, g);
                    let (c, ga) = g.sample(t);
                    (c, ga)
                }
                None => (e.color, 1.0),
            };
            a *= e.opacity;
            if a <= 0.002 {
                continue;
            }
            if cov_out[i] > 0.002 {
                composite(out, x, y, color, cov_out[i], a, mode);
            }
            if cov_in[i] > 0.002 {
                match clip {
                    // 组样式：`front` 是一张全新的透明光栅，插在组内容**上方**，
                    // 里面没有内容可换色。`composite_inside` 撞到 dst alpha = 0
                    // 就直接返回，内描边于是整条消失（绿勾 3px 内描边就是这么丢的）。
                    // 这里按组覆盖率把描边画成新像素：source-over 之后，若组内容
                    // 原本不透明，结果正好是 PS 的「描边盖在内容上」。
                    Some(c) => {
                        let cov = (cov_in[i] * c[i]).clamp(0.0, 1.0);
                        if cov > 0.002 {
                            composite(out, x, y, color, cov, a, mode);
                        }
                    }
                    None => composite_inside(out, x, y, color, cov_in[i], a, mode),
                }
            }
        }
    }
}

pub(super) fn draw_inner_shadow(
    out: &mut RgbaImage,
    w: u32,
    h: u32,
    alpha: &[f32],
    e: &LayerEffect,
    clip: Option<&[f32]>,
) {
    let (dx, dy) = offset_vector(e.distance, e.angle);
    // The shadow is the *hole* (everything outside the shape) moved by the same
    // offset the light would push it, blurred, then clipped to the shape: that
    // puts the shadow on the lit side, as Photoshop's inner shadow does.
    let inverse: Vec<f32> = alpha.iter().map(|a| 1.0 - a).collect();
    let (sx, sy) = if legacy("dir") { (-dx, -dy) } else { (dx, dy) };
    let shifted = shift_mask(&inverse, w, h, sx, sy, 1.0);
    let (grow, soft) = spread_split(e.size, e.choke);
    let grown = dilate(&shifted, w, h, grow);
    let blurred = tent_blur(&grown, w, h, soft);
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            // 内阴影只画在内容里：`out` 在这里可能已经被投影/外描边扩过，
            // 不能拿 out 的 alpha 当门槛（否则投影会被再染一遍）。
            if alpha[i] <= INSIDE_EPS {
                continue;
            }
            let cov = blurred[i];
            if cov <= 0.002 {
                continue;
            }
            // 组级前景光栅从全透明开始，`composite_inside` 会因为 dst alpha=0
            // 整块丢掉；组模式按组覆盖率画成新像素（与组描边同一策略）。
            match clip {
                Some(c) => {
                    let cov = (cov * c[i]).clamp(0.0, 1.0);
                    if cov > 0.002 {
                        composite(out, x, y, e.color, cov, e.opacity, mode);
                    }
                }
                None => composite_inside(out, x, y, e.color, cov, e.opacity, mode),
            }
        }
    }
}

pub(super) fn draw_inner_glow(
    out: &mut RgbaImage,
    w: u32,
    h: u32,
    alpha: &[f32],
    e: &LayerEffect,
    clip: Option<&[f32]>,
) {
    let inverse: Vec<f32> = alpha.iter().map(|a| 1.0 - a).collect();
    let (grow, soft) = spread_split(e.size, e.choke);
    let grown = dilate(&inverse, w, h, grow);
    let blurred = tent_blur(&grown, w, h, soft);
    let mode = blend_of(e.blend);
    let from_center = e.glow_source == GlowSource::Center;
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if alpha[i] <= 0.002 {
                continue;
            }
            // Default 50 % Range: what the contour leaves at half strength is full.
            let mut t = (blurred[i] * 2.0).min(1.0);
            if from_center {
                // A centre-source glow is the edge glow inverted, measured after
                // the range (Photoshop 27.11).
                t = 1.0 - t;
            }
            let cov = t;
            if cov <= 0.002 {
                continue;
            }
            match clip {
                Some(c) => {
                    let cov = (cov * c[i]).clamp(0.0, 1.0);
                    if cov > 0.002 {
                        composite(out, x, y, e.color, cov, e.opacity, mode);
                    }
                }
                None => composite_inside(out, x, y, e.color, cov, e.opacity, mode),
            }
        }
    }
}

pub(super) fn draw_color_overlay(
    out: &mut RgbaImage,
    w: u32,
    h: u32,
    alpha: &[f32],
    e: &LayerEffect,
    clip: Option<&[f32]>,
) {
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let mut cov = alpha[i];
            if let Some(c) = clip {
                cov *= c[i];
            }
            if cov <= 0.002 {
                continue;
            }
            composite_fill(out, x, y, e.color, cov, e.opacity, mode);
        }
    }
}

pub(super) fn draw_gradient_overlay(
    out: &mut RgbaImage,
    w: u32,
    h: u32,
    alpha: &[f32],
    bbox: [f32; 4],
    e: &LayerEffect,
    clip: Option<&[f32]>,
) {
    let Some(g) = e.gradient.as_ref() else {
        return;
    };
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let mut cov = alpha[i];
            if let Some(c) = clip {
                cov *= c[i];
            }
            if cov <= 0.002 {
                continue;
            }
            let t = gradient_t(x as f32, y as f32, bbox, g);
            let (color, ga) = g.sample(t);
            if ga <= 0.002 {
                continue;
            }
            composite(out, x, y, color, cov, e.opacity * ga, mode);
        }
    }
}

/// 图案叠加：把图案平铺进图层内容。
///
/// 返回 `false` 表示有样式但文件里没有对应图案像素（或缺少图案引用），
/// 调用方据此记一条降级说明；画面保持原样，不拿白色/纯色冒充图案。
pub(super) fn draw_pattern_overlay(
    out: &mut RgbaImage,
    w: u32,
    h: u32,
    alpha: &[f32],
    offset: (i32, i32),
    ox: i64,
    oy: i64,
    e: &LayerEffect,
    patterns: Option<&crate::psd_pattern::PatternStore>,
) -> bool {
    let Some(r) = e.pattern.as_ref() else {
        return false;
    };
    let Some(store) = patterns else {
        return false;
    };
    let Some(mut paint) = store.paint(r) else {
        return false;
    };
    paint.phase = e.pattern_phase;
    // `Algn`：图案原点跟随图层左上角；否则用文档原点。
    let anchor = if r.align {
        (offset.0 as f32, offset.1 as f32)
    } else {
        (0.0, 0.0)
    };
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let cov = alpha[i];
            if cov <= 0.002 {
                continue;
            }
            let cx = x as f32 - ox as f32 + offset.0 as f32;
            let cy = y as f32 - oy as f32 + offset.1 as f32;
            let px = paint.sample(cx, cy, anchor);
            let pa = px[3] as f32 / 255.0;
            if pa <= 0.002 {
                continue;
            }
            composite(
                out,
                x,
                y,
                [px[0], px[1], px[2]],
                cov,
                e.opacity * pa,
                mode,
            );
        }
    }
    true
}

/// 斜面 / 浮雕：按内容 alpha 的模糊梯度模拟一个受光表面。
///
/// Photoshop 的斜面本质是「边缘坡度 × 光照方向」：先按 Size 模糊 alpha
/// 得到高度场，边缘处的梯度就是朝外的法线；法线与光源方向的点积为正的
/// 一侧打高光（`hglC` / `hglM` / `hglO`），为负的一侧压暗影（`sdwC` /
/// `sdwM` / `sdwO`）。`depth`（`srgR`）放大坡度，`bvlD` 翻转凹凸。
pub(super) fn draw_bevel(out: &mut RgbaImage, w: u32, h: u32, alpha: &[f32], e: &LayerEffect) {
    // 高度场：Size 决定斜面宽度，Sftn 再柔化一层。
    let radius = (((e.size + e.soften) * 0.5).ceil() as i32).max(1);
    let height = box_blur(alpha, w, h, radius);
    let rad = e.angle.to_radians();
    // 指向光源的方向（图像坐标，y 向下）。PS 角度逆时针、y 向上，
    // 所以同一角度下「指向光源」= (cos, -sin)。
    let (lx, ly) = (rad.cos(), -rad.sin());
    let sign = match e.bevel_direction {
        psd::BevelDirection::Up => 1.0,
        psd::BevelDirection::Down => -1.0,
    };
    let depth = (e.depth / 100.0).clamp(0.0, 4.0);
    let hi = blend_of(e.highlight_blend);
    let sh = blend_of(e.shadow_blend);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let gx = sample(&height, w, h, x as i32 + 1, y as i32)
                - sample(&height, w, h, x as i32 - 1, y as i32);
            let gy = sample(&height, w, h, x as i32, y as i32 + 1)
                - sample(&height, w, h, x as i32, y as i32 - 1);
            let len = (gx * gx + gy * gy).sqrt();
            if len <= 1e-4 {
                continue;
            }
            // 朝外法线 = -梯度 / 长度。
            let shade = (-gx * lx - gy * ly) / len * sign * depth;
            if shade.abs() <= 0.002 {
                continue;
            }
            // 坡度带：边缘处 b*(1-b) 最大，深处和远处都衰减到 0。
            let edge = (height[i] * (1.0 - height[i]) * 4.0).clamp(0.0, 1.0);
            let side = match e.bevel_style {
                psd::BevelStyle::Inner => alpha[i],
                psd::BevelStyle::Outer => 1.0 - alpha[i],
                _ => 1.0,
            };
            let cov = edge * side * shade.abs().min(1.0);
            if cov <= 0.002 {
                continue;
            }
            if shade > 0.0 {
                composite(
                    out,
                    x,
                    y,
                    e.highlight_color,
                    cov,
                    e.highlight_opacity,
                    hi,
                );
            } else {
                composite(out, x, y, e.shadow_color, cov, e.shadow_opacity, sh);
            }
        }
    }
}

/// 越界取 0（高度场在图层外没有内容）。
fn sample(src: &[f32], w: u32, h: u32, x: i32, y: i32) -> f32 {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return 0.0;
    }
    src[(y as u32 * w + x as u32) as usize]
}

fn blend_of(mode: psd::BlendMode) -> editcore::BlendMode {
    crate::psd_layers::blend_of(mode)
}

/// 「背后」样式（投影 / 外发光）自己的混合模式（PSD 描述符 `Md  `）。
///
/// Photoshop 拿效果自己的混合模式把效果与**图层下方的画面**混合；烘焙把效果
/// 画进图层位图后，那层位图里已经没有「下方」（背景是透明的），`Multiply`
/// 于是退化成 `Normal` —— 标题栏的奶油色投影会把黄色面板盖成一条不透明的
/// 奶油带，而不是把它压暗。合成阶段用这个模式把语义补回来。
///
/// 多个背后样式时取最后一个非 `Normal` 的（PS 的堆叠里后画的在上，且
/// `Normal` 作者很少，非 Normal 才是要修的那类）。没有背后样式返回 `None`。
pub fn behind_blend<'a>(
    effects: impl IntoIterator<Item = &'a LayerEffect>,
) -> Option<editcore::BlendMode> {
    let mut mode = None;
    for e in effects
        .into_iter()
        .filter(|e| e.enabled && !skip_kind(e.kind))
    {
        if !matches!(e.kind, EffectKind::DropShadow | EffectKind::OuterGlow) {
            continue;
        }
        let m = blend_of(e.blend);
        // Normal 只作为兜底：先记下来，后面遇到非 Normal 就覆盖它。
        mode = Some(if m == editcore::BlendMode::Normal {
            mode.unwrap_or(m)
        } else {
            m
        });
    }
    mode
}

/// Photoshop's angle/distance pair as an image-space offset.
///
/// The light angle is measured counter-clockwise with +y pointing **up**, so 90°
/// is "light from the top"; the shadow falls *away* from the light, i.e. down the
/// screen. Image rows grow downwards, so the vertical component keeps its sign
/// while the horizontal one flips — Photoshop's default 120°/5 px shadow lands
/// bottom-right, which is what this produces. Photoshop snaps the offset to whole
/// pixels.
pub(super) fn offset_vector(distance: f32, angle_deg: f32) -> (i32, i32) {
    let rad = angle_deg.to_radians();
    if legacy("dir") {
        return (
            (distance * rad.cos()).round() as i32,
            (distance * rad.sin()).round() as i32,
        );
    }
    (
        (-distance * rad.cos()).round() as i32,
        (distance * rad.sin()).round() as i32,
    )
}

/// Axis-aligned bounds of everything with alpha above noise, in pixels.
pub(super) fn alpha_bounds(alpha: &[f32], w: u32, h: u32) -> [f32; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for y in 0..h {
        for x in 0..w {
            if alpha[(y * w + x) as usize] > 0.02 {
                x0 = x0.min(x as f32);
                y0 = y0.min(y as f32);
                x1 = x1.max(x as f32);
                y1 = y1.max(y as f32);
            }
        }
    }
    if x0 > x1 {
        return [0.0, 0.0, w as f32 - 1.0, h as f32 - 1.0];
    }
    [x0, y0, x1, y1]
}
