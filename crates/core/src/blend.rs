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
    /// 溶解：按 alpha 做二值抖动，像素要么全有要么全无，需要坐标。
    Dissolve,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    LinearDodge,
    LinearBurn,
    /// 深色（`dkCl`）：整组取比较暗的那个颜色，不逐通道挑。
    DarkerColor,
    /// 浅色（`lgCl`）：整组取比较亮的那个颜色，不逐通道挑。
    LighterColor,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
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

    /// 溶解没有逐像素公式，要靠坐标抖动，调用方须先换成 [`BlendMode::Normal`]。
    pub fn is_dissolve(self) -> bool {
        matches!(self, BlendMode::Dissolve)
    }

    /// Human-readable name, for the degradation list and diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "正常",
            BlendMode::Dissolve => "溶解",
            BlendMode::Multiply => "正片叠底",
            BlendMode::Screen => "滤色",
            BlendMode::Overlay => "叠加",
            BlendMode::Darken => "变暗",
            BlendMode::Lighten => "变亮",
            BlendMode::ColorDodge => "颜色减淡",
            BlendMode::ColorBurn => "颜色加深",
            BlendMode::LinearDodge => "线性减淡(添加)",
            BlendMode::LinearBurn => "线性加深",
            BlendMode::DarkerColor => "深色",
            BlendMode::LighterColor => "浅色",
            BlendMode::SoftLight => "柔光",
            BlendMode::HardLight => "强光",
            BlendMode::VividLight => "亮光",
            BlendMode::LinearLight => "线性光",
            BlendMode::PinLight => "点光",
            BlendMode::HardMix => "实色混合",
            BlendMode::Difference => "差值",
            BlendMode::Exclusion => "排除",
            BlendMode::Subtract => "减去",
            BlendMode::Divide => "划分",
            BlendMode::Hue => "色相",
            BlendMode::Saturation => "饱和度",
            BlendMode::Color => "颜色",
            BlendMode::Luminosity => "明度",
        }
    }
}

/// 溶解的二值阈值：`(x, y)` 决定这一格在 `alpha` 下是否显形。
///
/// Photoshop 用一张固定的噪声图决定哪些像素保留；这里用坐标哈希做确定性
/// 噪声（同一份文档每次渲染结果一致），阈值是图层 alpha × 不透明度。返回
/// `true` 表示该像素保留为完全不透明。
pub fn dissolve_hit(x: i32, y: i32, alpha: f32) -> bool {
    let a = alpha.clamp(0.0, 1.0);
    if a >= 1.0 {
        return true;
    }
    if a <= 0.0 {
        return false;
    }
    let mut h = (x as u32).wrapping_mul(0x9E37_79B9) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    (h & 0xFFFF) as f32 / 65536.0 < a
}

/// Composite `src` onto `out`, honouring `mode`.
///
/// `out` and `src` are **straight-alpha** RGBA (0..=255) — the convention the
/// whole compositor, the preview renderer and the PSD writer use.
///
/// The honest Porter-Duff form is used instead of a plain lerp:
///
/// ```text
/// ao = as + ab * (1 - as)
/// co = as * (1 - ab) * Cs + as * ab * B(Cb, Cs) + (1 - as) * ab * Cb
/// Co = co / ao
/// ```
///
/// Without the final division the buffer accumulates *premultiplied* colour on
/// a semi-transparent backdrop, so a 50 % white layer over transparency came out
/// mid-grey, and every anti-aliased edge in a PSD with a transparent canvas was
/// darker than Photoshop's. The blend function is only consulted where the
/// backdrop has coverage: a `Multiply` layer over transparent pixels still
/// pastes its own colour.
pub fn blend_pixel(out: &mut [u8; 4], src: [u8; 4], opacity: f32, mode: BlendMode) {
    let a = (src[3] as f32 / 255.0) * opacity;
    if a <= 0.0 {
        return;
    }
    let ab = out[3] as f32 / 255.0;
    if mode.is_normal() {
        blend_normal(out, src, a);
        return;
    }
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
    let ao = a + ab * inv;
    if ao <= 0.0 {
        *out = [0, 0, 0, 0];
        return;
    }
    for c in 0..3 {
        // Where the backdrop is transparent there is nothing to blend with, so
        // the source keeps its own colour.
        let eff = cs[c] * (1.0 - ab) + mixed[c] * ab;
        let co = eff * a + cb[c] * ab * inv;
        out[c] = ((co / ao) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out[3] = (ao * 255.0).round().clamp(0.0, 255.0) as u8;
}

pub(crate) fn blend_normal(out: &mut [u8; 4], src: [u8; 4], a: f32) {
    let ab = out[3] as f32 / 255.0;
    let inv = 1.0 - a;
    let ao = a + ab * inv;
    if ao <= 0.0 {
        *out = [0, 0, 0, 0];
        return;
    }
    // 直通 alpha 下的 over：先按预乘写，再除掉合成 alpha，否则半透明背景
    // 会把累积值当成直通色继续往下叠（越叠越暗）。
    for c in 0..3 {
        let co = src[c] as f32 * a + out[c] as f32 * ab * inv;
        out[c] = (co / ao).round().clamp(0.0, 255.0) as u8;
    }
    out[3] = (ao * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// `blend_pixel` 的浮点版本，用于整幅合成。
///
/// 输出缓冲是**预乘 alpha 的 f32**（0..=1）：不透明画布上预乘与直通等价，
/// 但半透明画布上只有预乘才能把「已叠上去的颜色」和「覆盖率」分开累积。
/// 量化成 8 位图时由调用方一次性除掉 alpha（`acc_to_image`），中途不 round ——
/// 一张有几十层堆叠的 PSD 每层都舍入一次会累积 1~2/255 的系统性偏移，保真
/// 模式下这就是可见的色差。
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
    // 混合函数要的是直通色，缓冲里存的是预乘色：ab<1 时先还原一次。
    let cb = if ab > 0.0 {
        [out[0] / ab, out[1] / ab, out[2] / ab]
    } else {
        [0.0; 3]
    };
    let cs = [src[0], src[1], src[2]];
    let mixed = blend_rgb(mode, cb, cs);
    let inv = 1.0 - a;
    for c in 0..3 {
        // 背景透明处没有可混合的颜色，源色自己保留（与 u8 版一致）。
        let eff = cs[c] * (1.0 - ab) + mixed[c] * ab;
        out[c] = eff * a + out[c] * inv;
    }
    out[3] = a + ab * inv;
}

/// Blend two straight colours (0..=1 components), returning the blended colour.
fn blend_rgb(mode: BlendMode, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    match mode {
        BlendMode::Normal => s,
        // 溶解由调用方先抖动成 Normal，走到这里说明没有坐标可用（效果烘焙），
        // 按正常处理。
        BlendMode::Dissolve => s,
        BlendMode::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        BlendMode::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
        BlendMode::Color => set_lum(s, lum(b)),
        BlendMode::Luminosity => set_lum(b, lum(s)),
        // 深色 / 浅色不逐通道挑：比较两个颜色的明度后**整组**取一个，
        // 所以逐通道算会把红蓝混出新色，Photoshop 里不会。
        BlendMode::DarkerColor => {
            if lum(s) < lum(b) {
                s
            } else {
                b
            }
        }
        BlendMode::LighterColor => {
            if lum(s) > lum(b) {
                s
            } else {
                b
            }
        }
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
        BlendMode::Dissolve => s,
        BlendMode::Multiply => b * s,
        BlendMode::Screen => b + s - b * s,
        // Overlay is HardLight with the operands swapped.
        BlendMode::Overlay => hard_light(s, b),
        BlendMode::Darken => b.min(s),
        BlendMode::Lighten => b.max(s),
        BlendMode::ColorDodge => color_dodge(b, s),
        BlendMode::ColorBurn => color_burn(b, s),
        BlendMode::LinearDodge => (b + s).min(1.0),
        BlendMode::LinearBurn => (b + s - 1.0).max(0.0),
        BlendMode::HardLight => hard_light(b, s),
        BlendMode::SoftLight => soft_light(b, s),
        // 亮光：源色暗于 50% 用颜色加深（源色翻倍），亮于 50% 用颜色减淡。
        BlendMode::VividLight => {
            if s <= 0.5 {
                color_burn(b, 2.0 * s)
            } else {
                color_dodge(b, 2.0 * (s - 0.5))
            }
        }
        // 线性光：源色每增加 1 相当于线性减淡 2。
        BlendMode::LinearLight => (b + 2.0 * s - 1.0).clamp(0.0, 1.0),
        // 点光：源色暗于 50% 取变暗，亮于 50% 取变亮。
        BlendMode::PinLight => {
            if s <= 0.5 {
                b.min(2.0 * s)
            } else {
                b.max(2.0 * s - 1.0)
            }
        }
        // 实色混合：亮光的结果过 50% 阈值，只剩 0 / 1（等价 b + s >= 1）。
        BlendMode::HardMix => {
            if b + s >= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        BlendMode::Difference => (b - s).abs(),
        BlendMode::Exclusion => b + s - 2.0 * b * s,
        BlendMode::Subtract => (b - s).max(0.0),
        // 划分：源色为 0 时按白色处理，避免除零。
        BlendMode::Divide => {
            if s <= 0.0 {
                1.0
            } else {
                (b / s).min(1.0)
            }
        }
        // Handled by `blend_rgb`.
        BlendMode::DarkerColor
        | BlendMode::LighterColor
        | BlendMode::Hue
        | BlendMode::Saturation
        | BlendMode::Color
        | BlendMode::Luminosity => s,
    }
}

fn color_dodge(b: f32, s: f32) -> f32 {
    if s >= 1.0 {
        1.0
    } else {
        (b / (1.0 - s)).min(1.0)
    }
}

fn color_burn(b: f32, s: f32) -> f32 {
    if s <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - b) / s).min(1.0)
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
    // values back where they came from. `res` is indexed by channel; writing
    // the rank-ordered values into it and remapping afterwards scrambles any
    // colour whose channels are not already ascending (pure green became blue).
    let mut idx = [0usize, 1, 2];
    idx.sort_by(|a, b| c[*a].partial_cmp(&c[*b]).unwrap_or(std::cmp::Ordering::Equal));
    let (lo, mid, hi) = (idx[0], idx[1], idx[2]);
    let (cmin, cmid, cmax) = (c[lo], c[mid], c[hi]);
    let mut res = [0.0f32; 3];
    if cmax > cmin {
        res[mid] = (cmid - cmin) * s / (cmax - cmin);
        res[hi] = s;
    }
    res
}

/// 混合颜色带（Blend If）里的一个范围。
///
/// PSD 每个范围占 4 字节：黑起、黑止、白起、白止（0..=255）。未拆分时起止
/// 相等，就是一条硬边；拆开后两值之间是线性斜坡。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlendRange {
    pub black: [u8; 2],
    pub white: [u8; 2],
}

impl Default for BlendRange {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl BlendRange {
    /// 全可见（黑 0、白 255），PSD 里的全默认值。
    pub const DEFAULT: Self = Self {
        black: [0, 0],
        white: [255, 255],
    };

    pub fn from_psd(bytes: [u8; 4]) -> Self {
        Self {
            black: [bytes[0], bytes[1]],
            white: [bytes[2], bytes[3]],
        }
    }

    pub fn to_psd(self) -> [u8; 4] {
        [self.black[0], self.black[1], self.white[0], self.white[1]]
    }

    pub fn is_default(&self) -> bool {
        *self == Self::DEFAULT
    }

    /// `value` 落在这个范围里时的可见权重（0..=1）。
    pub fn factor(&self, value: u8) -> f32 {
        if self.is_default() {
            return 1.0;
        }
        // 拆分时文件里按「低位、高位」存；个别写手会反过来，取大小序保证
        // 斜坡方向总是「黑端抬升、白端下降」。
        let b_lo = self.black[0].min(self.black[1]) as f32;
        let b_hi = self.black[0].max(self.black[1]) as f32;
        let w_lo = self.white[0].min(self.white[1]) as f32;
        let w_hi = self.white[0].max(self.white[1]) as f32;
        let v = value as f32;
        if v < b_lo {
            return 0.0;
        }
        if v < b_hi {
            return (v - b_lo) / (b_hi - b_lo);
        }
        if v <= w_lo {
            return 1.0;
        }
        if v < w_hi {
            return (w_hi - v) / (w_hi - w_lo);
        }
        0.0
    }
}

/// 混合颜色带：本图层（source，看图层自己的颜色）与下层（destination，
/// 看已经合成好的背景色）两组范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlendIf {
    pub source: BlendRange,
    pub destination: BlendRange,
    /// `true`：叠加每个通道的范围（PSD 里按 RGB 分通道设置的那一组）。
    /// 灰度模式下只用 `source` / `destination` 两组合成范围。
    pub rgb: bool,
    pub source_channels: [BlendRange; 3],
    pub destination_channels: [BlendRange; 3],
}

impl Default for BlendIf {
    fn default() -> Self {
        Self {
            source: BlendRange::DEFAULT,
            destination: BlendRange::DEFAULT,
            rgb: false,
            source_channels: [BlendRange::DEFAULT; 3],
            destination_channels: [BlendRange::DEFAULT; 3],
        }
    }
}

impl BlendIf {
    /// 所有范围都是默认值 = 不影响画面，解码时直接丢掉。
    pub fn is_noop(&self) -> bool {
        self.source.is_default()
            && self.destination.is_default()
            && (!self.rgb
                || (self.source_channels.iter().all(|r| r.is_default())
                    && self.destination_channels.iter().all(|r| r.is_default())))
    }

    /// 单个像素的可见权重；乘到图层 alpha 上就是 Blend If 的效果。
    ///
    /// 源色取图层自己（直通 RGB），目标色取下层已合成的直通 RGB。灰度按
    /// Photoshop 的常见权重 0.299/0.587/0.114。
    pub fn factor(&self, source: [u8; 3], destination: [u8; 3]) -> f32 {
        self.source_factor(source) * self.destination_factor(destination)
    }

    /// 只看「本图层」那一半。下层是全透明时没有颜色可比，合成路径只乘这部分。
    pub fn source_factor(&self, source: [u8; 3]) -> f32 {
        let mut f = self.source.factor(gray(source));
        if self.rgb {
            for i in 0..3 {
                f *= self.source_channels[i].factor(source[i]);
            }
        }
        f
    }

    /// 只看「下层」那一半。
    pub fn destination_factor(&self, destination: [u8; 3]) -> f32 {
        let mut f = self.destination.factor(gray(destination));
        if self.rgb {
            for i in 0..3 {
                f *= self.destination_channels[i].factor(destination[i]);
            }
        }
        f
    }
}

fn gray(rgb: [u8; 3]) -> u8 {
    let v = 0.299 * rgb[0] as f32 + 0.587 * rgb[1] as f32 + 0.114 * rgb[2] as f32;
    v.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_if_range_ramps_and_hard_edges() {
        let split = BlendRange {
            black: [40, 80],
            white: [200, 240],
        };
        assert_eq!(split.factor(0), 0.0);
        assert_eq!(split.factor(40), 0.0);
        assert!((split.factor(60) - 0.5).abs() < 1e-6);
        assert_eq!(split.factor(120), 1.0);
        assert_eq!(split.factor(200), 1.0);
        assert!((split.factor(220) - 0.5).abs() < 1e-6);
        assert_eq!(split.factor(250), 0.0);

        // 未拆分 = 硬边：黑端以下全无，白端以上全无。
        let cut = BlendRange {
            black: [128, 128],
            white: [255, 255],
        };
        assert_eq!(cut.factor(127), 0.0);
        assert_eq!(cut.factor(128), 1.0);
        assert_eq!(BlendRange::DEFAULT.factor(0), 1.0);
    }

    #[test]
    fn blend_if_uses_source_and_destination_gray() {
        let source_only = BlendIf {
            source: BlendRange {
                black: [0, 0],
                white: [128, 128],
            },
            ..Default::default()
        };
        assert_eq!(source_only.factor([255, 255, 255], [0, 0, 0]), 0.0);
        assert_eq!(source_only.factor([0, 0, 0], [255, 255, 255]), 1.0);

        let destination_only = BlendIf {
            destination: BlendRange {
                black: [0, 0],
                white: [64, 64],
            },
            ..Default::default()
        };
        assert_eq!(destination_only.factor([255, 255, 255], [32, 32, 32]), 1.0);
        assert_eq!(
            destination_only.factor([255, 255, 255], [200, 200, 200]),
            0.0
        );
        assert!(BlendIf::default().is_noop());
    }

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

    /// 溶解：半透明区域按坐标抖动成二值，覆盖率约等于 alpha。
    #[test]
    fn dissolve_thresholds_partial_alpha_by_coordinate() {
        assert!(dissolve_hit(0, 0, 1.0));
        assert!(!dissolve_hit(0, 0, 0.0));
        let kept = (0..64)
            .flat_map(|y| (0..64).map(move |x| (x, y)))
            .filter(|&(x, y)| dissolve_hit(x, y, 0.5))
            .count();
        assert!((1700..=2400).contains(&kept), "保留像素数异常: {kept}");
        // 同一坐标必须稳定，否则每帧渲染都会闪。
        assert_eq!(dissolve_hit(7, 9, 0.5), dissolve_hit(7, 9, 0.5));
    }

    /// 补齐的八个模式：期望值按 PDF/W3C 公式手算，和 Photoshop 的
    /// `vLit` / `lLit` / `pLit` / `hMix` / `fsub` / `fdiv` / `dkCl` / `lgCl` 一致。
    #[test]
    fn extended_layer_modes_match_the_spec_formulas() {
        // 亮光：源色 192(0.753) 走颜色减淡（2×(0.753-0.5)=0.506），
        // 底 128(0.502) → 0.502/0.494 超过 1 → 白；源色 64(0.251) 走颜色加深，
        // 底 200(0.784) → 1-min(1, 0.216/0.502) = 0.569 → 145。
        assert_eq!(over([128, 128, 128], [192, 192, 192], BlendMode::VividLight), [255, 255, 255]);
        assert_eq!(over([200, 200, 200], [64, 64, 64], BlendMode::VividLight), [145, 145, 145]);
        // 线性光：b + 2s - 1。
        assert_eq!(over([128, 128, 128], [192, 192, 192], BlendMode::LinearLight), [255, 255, 255]);
        assert_eq!(over([128, 128, 128], [64, 64, 64], BlendMode::LinearLight), [1, 1, 1]);
        // 点光：暗取 min(b, 2s)，亮取 max(b, 2s-1)。
        assert_eq!(over([128, 128, 128], [64, 64, 64], BlendMode::PinLight), [128, 128, 128]);
        assert_eq!(over([200, 200, 200], [192, 192, 192], BlendMode::PinLight), [200, 200, 200]);
        // 实色混合：b + s >= 1 出白，否则出黑。
        assert_eq!(over([100, 100, 100], [100, 100, 100], BlendMode::HardMix), [0, 0, 0]);
        assert_eq!(over([200, 200, 200], [200, 200, 200], BlendMode::HardMix), [255, 255, 255]);
        // 减去 / 划分。
        assert_eq!(over([200, 60, 0], [50, 20, 90], BlendMode::Subtract), [150, 40, 0]);
        assert_eq!(over([64, 128, 0], [128, 64, 0], BlendMode::Divide), [128, 255, 255]);
        // 深色 / 浅色整组取色：底 (50,200,50) 明度 138.5，源灰 (100,100,100) 明度 100。
        assert_eq!(
            over([50, 200, 50], [100, 100, 100], BlendMode::DarkerColor),
            [100, 100, 100]
        );
        assert_eq!(over([50, 200, 50], [100, 100, 100], BlendMode::LighterColor), [50, 200, 50]);
    }

    /// 其余分离 / 非分离模式：同样按 W3C 公式手算（Overlay 是 HardLight
    /// 交换操作数，柔光用 pegtop，色相 / 饱和度 / 颜色 / 明度走 SetLum/SetSat）。
    #[test]
    fn remaining_layer_modes_match_the_spec_formulas() {
        assert_eq!(over([64, 64, 64], [192, 192, 192], BlendMode::Overlay), [96, 96, 96]);
        assert_eq!(over([64, 64, 64], [64, 64, 64], BlendMode::SoftLight), [40, 40, 40]);
        assert_eq!(over([64, 64, 64], [64, 192, 192], BlendMode::HardLight), [32, 161, 161]);
        assert_eq!(over([200, 60, 30], [50, 20, 90], BlendMode::Difference), [150, 40, 60]);
        assert_eq!(over([64, 192, 128], [192, 64, 64], BlendMode::Exclusion), [160, 160, 128]);
        // 色相：取源色相 + 底色饱和度 / 明度；红底上放绿 → 绿相、红亮度。
        assert_eq!(over([255, 0, 0], [0, 255, 0], BlendMode::Hue), [0, 130, 0]);
        // 饱和度：红底取源的饱和度（灰 = 0）→ 灰；保持底色明度。
        assert_eq!(over([255, 0, 0], [128, 128, 128], BlendMode::Saturation), [77, 77, 77]);
        // 颜色：取源色 + 底色明度。绿色底放红色 → 红相、绿明度。
        assert_eq!(over([0, 255, 0], [255, 0, 0], BlendMode::Color), [255, 106, 106]);
        // 明度：取底色 + 源明度。绿色底放红色 → 绿相、红明度。
        assert_eq!(over([0, 255, 0], [255, 0, 0], BlendMode::Luminosity), [0, 130, 0]);
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

    #[test]
    fn half_opacity_layer_over_transparency_keeps_its_colour() {
        // 透明画布上 50% 的白色：Photoshop 存的是 (255,255,255,128)，不是
        // 预乘后的 (128,128,128,128)。旧实现把累积值当直通色，这里会得到灰色，
        // 于是所有「透明底 + 半透明贴图」的稿子整片偏暗。
        let mut out = [0u8, 0, 0, 0];
        blend_pixel(&mut out, [255, 255, 255, 255], 0.5, BlendMode::Normal);
        assert_eq!(out, [255, 255, 255, 128]);
    }

    #[test]
    fn second_half_opacity_layer_keeps_straight_alpha() {
        // 两层 50%：合成 alpha = 0.75，颜色仍是纯白（叠加不改变颜色本身）。
        let mut out = [0u8, 0, 0, 0];
        blend_pixel(&mut out, [255, 255, 255, 255], 0.5, BlendMode::Normal);
        blend_pixel(&mut out, [255, 255, 255, 255], 0.5, BlendMode::Normal);
        assert_eq!(out, [255, 255, 255, 192]);
    }

    #[test]
    fn blend_on_half_transparent_backdrop_uses_straight_colours() {
        // 半透明背景 (10,200,30,α128) 上叠 50% 的 (200,100,50)，按
        // Porter-Duff（ab≈0.502, as=0.5, Multiply）：
        //   ao = 0.751 → 192
        //   Co = (72, 126, 29)
        // 旧实现把背景当直通色、又不除 ao，会得到明显偏暗的 (52,60,6)。
        let mut out = [10u8, 200, 30, 128];
        blend_pixel(&mut out, [200, 100, 50, 255], 0.5, BlendMode::Multiply);
        assert_eq!(out[3], 192);
        assert!(
            (out[0] as i32 - 72).abs() <= 1
                && (out[1] as i32 - 126).abs() <= 1
                && (out[2] as i32 - 29).abs() <= 1,
            "got {out:?}"
        );
    }
}
