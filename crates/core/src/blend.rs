//! Layer blend modes.
//!
//! PSD files carry a blend mode per layer, and the popular ones change the
//! picture a lot: a `Color` layer over a white background is what turns the
//! backdrop of a UI file into its final tint. Flattening every non-Normal mode
//! to Normal was the main reason an opened PSD did not look like Photoshop.
//!
//! The math follows the PDF/Photoshop spec: separable modes work per channel,
//! the four non-separable ones (`Hue` / `Saturation` / `Color` / `Luminosity`)
//! work on whole RGB triples in HSL space.

/// How a layer's pixels combine with everything already composited below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    /// Plain alpha compositing — the historical behaviour.
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    LinearDodge,
    LinearBurn,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    /// `true` for the mode that needs no math at all (fast path).
    pub fn is_normal(self) -> bool {
        matches!(self, BlendMode::Normal)
    }

    /// Human-readable name, for the degradation list and diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "正常",
            BlendMode::Multiply => "正片叠底",
            BlendMode::Screen => "滤色",
            BlendMode::Overlay => "叠加",
            BlendMode::Darken => "变暗",
            BlendMode::Lighten => "变亮",
            BlendMode::ColorDodge => "颜色减淡",
            BlendMode::ColorBurn => "颜色加深",
            BlendMode::LinearDodge => "线性减淡(添加)",
            BlendMode::LinearBurn => "线性加深",
            BlendMode::SoftLight => "柔光",
            BlendMode::HardLight => "强光",
            BlendMode::Difference => "差值",
            BlendMode::Exclusion => "排除",
            BlendMode::Hue => "色相",
            BlendMode::Saturation => "饱和度",
            BlendMode::Color => "颜色",
            BlendMode::Luminosity => "明度",
        }
    }
}

/// Composite `src` onto `out`, honouring `mode`.
///
/// `out` is straight-alpha RGBA in 0..=255, same convention as the rest of the
/// compositor: colours are accumulated the way `blend_over` always did, and the
/// blend function is only consulted where the backdrop actually has coverage —
/// a `Multiply` layer over transparent pixels still pastes its own colour.
pub fn blend_pixel(out: &mut [u8; 4], src: [u8; 4], opacity: f32, mode: BlendMode) {
    let a = (src[3] as f32 / 255.0) * opacity;
    if a <= 0.0 {
        return;
    }
    // Fast path: identical to the pre-blend-mode implementation, bit for bit.
    if mode.is_normal() {
        blend_normal(out, src, a);
        return;
    }
    let ab = out[3] as f32 / 255.0;
    let cb = [
        out[0] as f32 / 255.0,
        out[1] as f32 / 255.0,
        out[2] as f32 / 255.0,
    ];
    let cs = [
        src[0] as f32 / 255.0,
        src[1] as f32 / 255.0,
        src[2] as f32 / 255.0,
    ];
    let mixed = blend_rgb(mode, cb, cs);
    let inv = 1.0 - a;
    for c in 0..3 {
        // Where the backdrop is transparent there is nothing to blend with, so
        // the source keeps its own colour.
        let eff = cs[c] * (1.0 - ab) + mixed[c] * ab;
        out[c] = ((eff * a + cb[c] * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out[3] = ((a + ab * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
}

pub(crate) fn blend_normal(out: &mut [u8; 4], src: [u8; 4], a: f32) {
    let inv = 1.0 - a;
    for c in 0..3 {
        out[c] = (src[c] as f32 * a + out[c] as f32 * inv).round() as u8;
    }
    out[3] = ((a + out[3] as f32 / 255.0 * inv) * 255.0).round() as u8;
}

/// `blend_pixel` 的浮点版本，用于整幅合成。
///
/// 输出缓冲是**直通 alpha 的 f32**（0..=1），颜色分量也是 0..=1。公式与
/// `blend_pixel` 逐项一致，唯一区别是中间结果不再每层 round 到 u8 —— 一张
/// 有几十层堆叠的 PSD 每层都舍入一次会累积 1~2/255 的系统性偏移，保真模式
/// 下这就是可见的色差。
pub(crate) fn blend_pixel_f32(out: &mut [f32; 4], src: [f32; 4], opacity: f32, mode: BlendMode) {
    let a = src[3] * opacity;
    if a <= 0.0 {
        return;
    }
    if mode.is_normal() {
        let inv = 1.0 - a;
        for c in 0..3 {
            out[c] = src[c] * a + out[c] * inv;
        }
        out[3] = a + out[3] * inv;
        return;
    }
    let ab = out[3];
    let cb = [out[0], out[1], out[2]];
    let cs = [src[0], src[1], src[2]];
    let mixed = blend_rgb(mode, cb, cs);
    let inv = 1.0 - a;
    for c in 0..3 {
        // 背景透明处没有可混合的颜色，源色自己保留（与 u8 版一致）。
        let eff = cs[c] * (1.0 - ab) + mixed[c] * ab;
        out[c] = eff * a + cb[c] * inv;
    }
    out[3] = a + ab * inv;
}

/// Blend two straight colours (0..=1 components), returning the blended colour.
fn blend_rgb(mode: BlendMode, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    match mode {
        BlendMode::Normal => s,
        BlendMode::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        BlendMode::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
        BlendMode::Color => set_lum(s, lum(b)),
        BlendMode::Luminosity => set_lum(b, lum(s)),
        _ => {
            let mut out = [0.0f32; 3];
            for c in 0..3 {
                out[c] = separable(mode, b[c], s[c]);
            }
            out
        }
    }
}

fn separable(mode: BlendMode, b: f32, s: f32) -> f32 {
    match mode {
        BlendMode::Normal => s,
        BlendMode::Multiply => b * s,
        BlendMode::Screen => b + s - b * s,
        // Overlay is HardLight with the operands swapped.
        BlendMode::Overlay => hard_light(s, b),
        BlendMode::Darken => b.min(s),
        BlendMode::Lighten => b.max(s),
        BlendMode::ColorDodge => {
            if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        BlendMode::ColorBurn => {
            if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        BlendMode::LinearDodge => (b + s).min(1.0),
        BlendMode::LinearBurn => (b + s - 1.0).max(0.0),
        BlendMode::HardLight => hard_light(b, s),
        BlendMode::SoftLight => soft_light(b, s),
        BlendMode::Difference => (b - s).abs(),
        BlendMode::Exclusion => b + s - 2.0 * b * s,
        // Handled by `blend_rgb`.
        BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => s,
    }
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        2.0 * s * b
    } else {
        1.0 - 2.0 * (1.0 - s) * (1.0 - b)
    }
}

/// W3C soft light: the "pegtop" curve Photoshop uses for its default 柔光.
fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b - (1.0 - 2.0 * s) * b * (1.0 - b)
    } else {
        let d = if b <= 0.25 {
            ((16.0 * b - 12.0) * b + 4.0) * b
        } else {
            b.sqrt()
        };
        b + (2.0 * s - 1.0) * (d - b)
    }
}

/// Perceptual luminance, the weight Photoshop uses for its非分离 modes.
fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn clip_color(mut c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    if n < 0.0 {
        for v in c.iter_mut() {
            *v = l + (*v - l) * l / (l - n).max(1e-6);
        }
    }
    if x > 1.0 {
        for v in c.iter_mut() {
            *v = l + (*v - l) * (1.0 - l) / (x - l).max(1e-6);
        }
    }
    c
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    // Order the three components, stretch the extremes to 0 / s, then put the
    // values back where they came from.
    let mut idx = [0usize, 1, 2];
    idx.sort_by(|a, b| c[*a].partial_cmp(&c[*b]).unwrap_or(std::cmp::Ordering::Equal));
    let (lo, mid, hi) = (idx[0], idx[1], idx[2]);
    let mut out = [0.0f32; 3];
    let (cmin, cmid, cmax) = (c[lo], c[mid], c[hi]);
    if cmax > cmin {
        out[mid] = (cmid - cmin) * s / (cmax - cmin);
        out[hi] = s;
    } else {
        out[mid] = 0.0;
        out[hi] = 0.0;
    }
    out[lo] = 0.0;
    let mut res = [0.0f32; 3];
    res[lo] = out[0];
    res[mid] = out[1];
    res[hi] = out[2];
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn over(b: [u8; 3], s: [u8; 3], mode: BlendMode) -> [u8; 3] {
        let mut out = [b[0], b[1], b[2], 255];
        blend_pixel(&mut out, [s[0], s[1], s[2], 255], 1.0, mode);
        [out[0], out[1], out[2]]
    }

    #[test]
    fn normal_mode_is_unchanged() {
        assert_eq!(over([10, 20, 30], [200, 100, 50], BlendMode::Normal), [200, 100, 50]);
    }

    #[test]
    fn multiply_darkens_and_screen_lightens() {
        assert_eq!(over([255, 128, 0], [128, 128, 128], BlendMode::Multiply), [128, 64, 0]);
        assert_eq!(over([0, 128, 255], [128, 128, 128], BlendMode::Screen), [128, 192, 255]);
    }

    #[test]
    fn linear_dodge_adds_and_clamps() {
        assert_eq!(over([100, 10, 0], [100, 10, 0], BlendMode::LinearDodge), [200, 20, 0]);
        assert_eq!(over([200, 0, 0], [200, 0, 0], BlendMode::LinearDodge), [255, 0, 0]);
    }

    #[test]
    fn color_mode_keeps_backdrop_luminance() {
        // A saturated blue over white must stay light, which is exactly what the
        // UI file's background layer relies on.
        let out = over([255, 255, 255], [99, 117, 255], BlendMode::Color);
        assert!(out[0] > 200 && out[1] > 200 && out[2] > 200, "got {out:?}");
        // ... and it keeps the source's hue ordering (blue strongest).
        assert!(out[2] >= out[0] && out[2] >= out[1]);
    }

    #[test]
    fn blend_over_transparent_backdrop_keeps_source() {
        let mut out = [0u8, 0, 0, 0];
        blend_pixel(&mut out, [10, 200, 30, 255], 1.0, BlendMode::Multiply);
        assert_eq!(out, [10, 200, 30, 255]);
    }

    #[test]
    fn opacity_scales_the_blend() {
        // Half-opacity Multiply lands halfway between the backdrop and the
        // fully blended result (which for a grey backdrop is (0, 128, 0)).
        let mut out = [128u8, 128, 128, 255];
        blend_pixel(&mut out, [0, 255, 0, 255], 0.5, BlendMode::Multiply);
        assert_eq!(&out[..3], &[64, 128, 64]);
    }
}
