//! macOS Preview style "Adjust Color" pipeline.
//!
//! All parameters are normalized so the QML panel can expose ±100 sliders:
//!
//! * bipolar parameters (`exposure` … `tint`) run `-1.0 ..= 1.0`, `0` = neutral
//! * `sepia` / `sharpness` run `0.0 ..= 1.0`, `0` = neutral
//! * levels: `black < white` in `0.0 ..= 1.0` (`0` / `1` = neutral) and
//!   `gamma > 0` (`1.0` = neutral)
//!
//! The tone chain (levels → exposure → shadows/highlights → contrast) plus the
//! white-balance gains fold into three 256-entry lookup tables, so every
//! parameter except sepia / saturation / sharpness costs one table lookup per
//! channel. Sepia and saturation are cross-channel and run afterwards;
//! sharpness is an unsharp mask applied last.
//!
//! Alpha is never modified: fully transparent pixels are left byte-identical
//! so a transparent canvas stays exactly transparent.

use image::RgbaImage;

/// Number of bins per RGB channel in [`Histogram`].
pub const BINS: usize = 64;

/// Which pixels a color adjustment is baked into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdjustTarget {
    /// The (cropped) base photo. Adjusting covers the WHOLE base bitmap, not
    /// just the visible crop, so "reset crop" reveals consistently adjusted
    /// pixels.
    Base,
    /// A regular layer, by index.
    Layer(usize),
}

/// Normalized color-adjustment parameters (see the module docs for ranges).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjust {
    /// Exposure in EV: `0` neutral, `±1` = `±2 EV`.
    pub exposure: f32,
    /// Contrast around mid-grey: `-1` flattens, `+1` triples the slope.
    pub contrast: f32,
    /// Highlight lift / recover. `+1` brightens bright areas.
    pub highlights: f32,
    /// Shadow lift / deepen. `+1` brightens dark areas.
    pub shadows: f32,
    /// `-1` = fully desaturated, `0` neutral, `+1` = 2x chroma.
    pub saturation: f32,
    /// `+1` warmer (red up, blue down), `-1` cooler.
    pub temperature: f32,
    /// `+1` magenta, `-1` green.
    pub tint: f32,
    /// Sepia toning amount, `0 ..= 1`.
    pub sepia: f32,
    /// Unsharp-mask amount, `0 ..= 1`.
    pub sharpness: f32,
    /// Levels black point (input value mapped to 0), `0 ..= 0.95`.
    pub black: f32,
    /// Levels mid-tone gamma, `> 0`, `1.0` = neutral.
    pub gamma: f32,
    /// Levels white point (input value mapped to 1), `0 ..= 1`.
    pub white: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Self::identity()
    }
}

impl Adjust {
    pub const fn identity() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            saturation: 0.0,
            temperature: 0.0,
            tint: 0.0,
            sepia: 0.0,
            sharpness: 0.0,
            black: 0.0,
            gamma: 1.0,
            white: 1.0,
        }
    }

    /// True when no pixel would change (every parameter neutral).
    pub fn is_identity(&self) -> bool {
        self.exposure.abs() < 1e-4
            && self.contrast.abs() < 1e-4
            && self.highlights.abs() < 1e-4
            && self.shadows.abs() < 1e-4
            && self.saturation.abs() < 1e-4
            && self.temperature.abs() < 1e-4
            && self.tint.abs() < 1e-4
            && self.sepia.abs() < 1e-4
            && self.sharpness.abs() < 1e-4
            && self.black.abs() < 1e-4
            && (self.gamma - 1.0).abs() < 1e-3
            && (self.white - 1.0).abs() < 1e-4
    }

    /// Clamp every parameter into its valid range (levels order included).
    pub fn sanitize(&mut self) {
        self.exposure = self.exposure.clamp(-1.0, 1.0);
        self.contrast = self.contrast.clamp(-1.0, 1.0);
        self.highlights = self.highlights.clamp(-1.0, 1.0);
        self.shadows = self.shadows.clamp(-1.0, 1.0);
        self.saturation = self.saturation.clamp(-1.0, 1.0);
        self.temperature = self.temperature.clamp(-1.0, 1.0);
        self.tint = self.tint.clamp(-1.0, 1.0);
        self.sepia = self.sepia.clamp(0.0, 1.0);
        self.sharpness = self.sharpness.clamp(0.0, 1.0);
        self.black = self.black.clamp(0.0, 0.95);
        self.white = self.white.clamp(self.black + 0.02, 1.0);
        self.gamma = self.gamma.clamp(0.05, 20.0);
    }

    /// Adjust a copy of `src`.
    pub fn apply_to(&self, src: &RgbaImage) -> RgbaImage {
        let mut out = src.clone();
        self.apply_in_place(&mut out);
        out
    }

    /// Adjust `img` in place (alpha untouched).
    pub fn apply_in_place(&self, img: &mut RgbaImage) {
        if self.is_identity() || img.width() == 0 || img.height() == 0 {
            return;
        }
        let lut = self.tone_lut();
        let sepia = self.sepia.clamp(0.0, 1.0);
        let sat = 1.0 + self.saturation.clamp(-1.0, 1.0);
        let cross_channel = sepia > 0.0 || (sat - 1.0).abs() > 1e-4;
        let raw: &mut [u8] = &mut *img;
        if !cross_channel {
            // Fast path (levels / exposure / contrast / shadows / highlights /
            // white balance only): pure per-channel table lookups, no float
            // math — ~4x faster than the general path on multi-megapixel
            // commits and on every live preview tick.
            for px in raw.chunks_exact_mut(4) {
                if px[3] == 0 {
                    continue;
                }
                px[0] = lut[0][px[0] as usize];
                px[1] = lut[1][px[1] as usize];
                px[2] = lut[2][px[2] as usize];
            }
        } else {
            for px in raw.chunks_exact_mut(4) {
                if px[3] == 0 {
                    continue;
                }
                let mut r = lut[0][px[0] as usize] as f32;
                let mut g = lut[1][px[1] as usize] as f32;
                let mut b = lut[2][px[2] as usize] as f32;
                // Saturation first, sepia last: 棕褐色 is a look of its own, so
                // it must survive a "-100 saturation" setting (order reversed
                // would desaturate the sepia tone straight back to grey).
                if (sat - 1.0).abs() > 1e-4 {
                    let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    r = l + (r - l) * sat;
                    g = l + (g - l) * sat;
                    b = l + (b - l) * sat;
                }
                if sepia > 0.0 {
                    let sr = 0.393 * r + 0.769 * g + 0.189 * b;
                    let sg = 0.349 * r + 0.686 * g + 0.168 * b;
                    let sb = 0.272 * r + 0.534 * g + 0.131 * b;
                    r += (sr - r) * sepia;
                    g += (sg - g) * sepia;
                    b += (sb - b) * sepia;
                }
                px[0] = r.clamp(0.0, 255.0).round() as u8;
                px[1] = g.clamp(0.0, 255.0).round() as u8;
                px[2] = b.clamp(0.0, 255.0).round() as u8;
            }
        }
        if self.sharpness > 0.002 {
            unsharp_mask(img, self.sharpness.clamp(0.0, 1.0));
        }
    }

    /// Three per-channel tone lookup tables (levels, exposure, shadows /
    /// highlights, contrast, white balance).
    fn tone_lut(&self) -> [[u8; 256]; 3] {
        let (gr, gg, gb) = self.white_balance_gains();
        let mut lut = [[0u8; 256]; 3];
        for (i, tone) in (0..256).map(|i| (i, self.tone(i as f32 / 255.0))) {
            lut[0][i] = (tone * gr * 255.0).clamp(0.0, 255.0).round() as u8;
            lut[1][i] = (tone * gg * 255.0).clamp(0.0, 255.0).round() as u8;
            lut[2][i] = (tone * gb * 255.0).clamp(0.0, 255.0).round() as u8;
        }
        lut
    }

    /// Per-channel white-balance gains (temperature + tint).
    fn white_balance_gains(&self) -> (f32, f32, f32) {
        let temp = self.temperature.clamp(-1.0, 1.0);
        let tint = self.tint.clamp(-1.0, 1.0);
        let mut gr = 1.0 + 0.30 * temp;
        let mut gg = 1.0;
        let mut gb = 1.0 - 0.30 * temp;
        gr *= 1.0 + 0.15 * tint;
        gb *= 1.0 + 0.15 * tint;
        gg *= 1.0 - 0.22 * tint;
        (gr.max(0.0), gg.max(0.0), gb.max(0.0))
    }

    /// Single-channel tone curve for an input value in `0.0 ..= 1.0`.
    fn tone(&self, v: f32) -> f32 {
        let black = self.black.clamp(0.0, 0.95);
        let white = self.white.clamp(black + 0.02, 1.0);
        let mut t = ((v - black) / (white - black)).clamp(0.0, 1.0);
        let gamma = self.gamma.clamp(0.05, 20.0);
        if (gamma - 1.0).abs() > 1e-3 {
            t = t.powf(1.0 / gamma);
        }
        let exposure = self.exposure.clamp(-1.0, 1.0);
        if exposure != 0.0 {
            t = (t * 2.0f32.powf(2.0 * exposure)).clamp(0.0, 1.0);
        }
        let shadows = self.shadows.clamp(-1.0, 1.0);
        if shadows != 0.0 {
            let w = (1.0 - t) * (1.0 - t);
            t += shadows * w * 0.75;
        }
        let highlights = self.highlights.clamp(-1.0, 1.0);
        if highlights != 0.0 {
            let w = t * t;
            t += highlights * w * 0.75;
        }
        let contrast = self.contrast.clamp(-1.0, 1.0);
        if contrast != 0.0 {
            let k = if contrast >= 0.0 {
                1.0 + 2.0 * contrast
            } else {
                1.0 / (1.0 - 2.0 * contrast)
            };
            t = (t - 0.5) * k + 0.5;
        }
        t.clamp(0.0, 1.0)
    }

    /// Levels mid-handle position (0..1 between the black and white handles)
    /// from a gamma value. `0.5` = neutral, left = brighter mids.
    pub fn handle_from_gamma(gamma: f32) -> f32 {
        let g = gamma.clamp(0.05, 20.0);
        (0.5f32.powf(g)).clamp(0.02, 0.98)
    }

    /// Inverse of [`Adjust::handle_from_gamma`].
    pub fn gamma_from_handle(pos: f32) -> f32 {
        let p = pos.clamp(0.02, 0.98);
        (p.ln() / 0.5f32.ln()).clamp(0.05, 20.0)
    }
}

/// 64-bin RGB histogram of an image (used for the panel graph and for
/// "auto levels").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram {
    r: [u32; BINS],
    g: [u32; BINS],
    b: [u32; BINS],
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            r: [0; BINS],
            g: [0; BINS],
            b: [0; BINS],
        }
    }
}

impl Histogram {
    pub fn of(img: &RgbaImage) -> Self {
        let mut hist = Self::default();
        hist.add(img);
        hist
    }

    /// Accumulate `img` into this histogram (RGB channels only).
    pub fn add(&mut self, img: &RgbaImage) {
        let scale = BINS as f32 / 256.0;
        let raw: &[u8] = img;
        for px in raw.chunks_exact(4) {
            // Fully transparent pixels contribute nothing: the panel graph
            // would otherwise be dominated by padding.
            if px[3] == 0 {
                continue;
            }
            let ri = ((px[0] as f32 * scale) as usize).min(BINS - 1);
            let gi = ((px[1] as f32 * scale) as usize).min(BINS - 1);
            let bi = ((px[2] as f32 * scale) as usize).min(BINS - 1);
            self.r[ri] += 1;
            self.g[gi] += 1;
            self.b[bi] += 1;
        }
    }

    pub fn total(&self) -> u32 {
        self.r.iter().sum()
    }

    /// Raw bins of channel `0` = R, `1` = G, `2` = B.
    pub fn bin_counts(&self, channel: usize) -> &[u32; BINS] {
        match channel {
            0 => &self.r,
            1 => &self.g,
            _ => &self.b,
        }
    }

    /// Luma-weighted bin counts (the graph's grey backdrop / auto levels).
    pub fn luma_bins(&self) -> [u32; BINS] {
        let mut out = [0u32; BINS];
        for i in 0..BINS {
            out[i] = ((self.r[i] as f32 * 0.2126
                + self.g[i] as f32 * 0.7152
                + self.b[i] as f32 * 0.0722)
                .round()) as u32;
        }
        out
    }

    /// Normalized `(black, white)` levels that stretch the luma distribution
    /// to the full range, clipping `clip_lo ..= clip_hi` of the pixels.
    pub fn auto_bounds(&self, clip_lo: f32, clip_hi: f32) -> (f32, f32) {
        let luma = self.luma_bins();
        let total: u64 = luma.iter().map(|v| *v as u64).sum();
        if total == 0 {
            return (0.0, 1.0);
        }
        let lo_target = (total as f64 * clip_lo.clamp(0.0, 0.49) as f64) as u64;
        let hi_target = (total as f64 * clip_hi.clamp(0.51, 1.0) as f64) as u64;
        let mut acc = 0u64;
        let mut lo_bin = 0usize;
        let mut hi_bin = BINS - 1;
        for (i, v) in luma.iter().enumerate() {
            acc += *v as u64;
            if acc >= lo_target {
                lo_bin = i;
                break;
            }
        }
        acc = 0;
        for (i, v) in luma.iter().enumerate() {
            acc += *v as u64;
            if acc >= hi_target {
                hi_bin = i;
                break;
            }
        }
        if hi_bin <= lo_bin {
            return (0.0, 1.0);
        }
        let norm = |bin: usize| (bin as f32 + 0.5) / BINS as f32;
        let black = norm(lo_bin).clamp(0.0, 0.95);
        let white = norm(hi_bin).clamp(black + 0.02, 1.0);
        (black, white)
    }

    /// Auto-levels parameters: percentile-clipped black/white points, neutral
    /// gamma ratio.
    pub fn auto_levels(&self) -> (f32, f32) {
        // 0.4% / 99.6% tails keeps a few stray pixels from pinning the range.
        self.auto_bounds(0.004, 0.996)
    }

    /// JSON for the QML graph canvas: `{"n":64,"r":[…],"g":[…],"b":[…]}`.
    pub fn to_json(&self) -> String {
        fn bins(values: &[u32; BINS]) -> String {
            let parts: Vec<String> = values.iter().map(|v| v.to_string()).collect();
            format!("[{}]", parts.join(","))
        }
        format!(
            "{{\"n\":{BINS},\"r\":{},\"g\":{},\"b\":{}}}",
            bins(&self.r),
            bins(&self.g),
            bins(&self.b)
        )
    }
}

/// Unsharp mask: `out = px + amount * (px - blur)` with a separable 1-2-1
/// blur. Rolling row buffers keep the extra memory at three rows instead of a
/// second full-size image (a 50 MP base would otherwise cost another 200 MB).
fn unsharp_mask(img: &mut RgbaImage, amount: f32) {
    let w = img.width() as usize;
    let h = img.height() as usize;
    if w < 2 || h < 2 || amount <= 0.0 {
        return;
    }
    let strength = amount * 1.6;
    let stride = w * 4;
    let mut cur_row = vec![0u8; stride];
    let mut next_row = vec![0u8; stride];
    let mut hb_prev = vec![0f32; w * 3];
    let mut hb_cur = vec![0f32; w * 3];
    let mut hb_next = vec![0f32; w * 3];

    fn blur_row(target: &mut [f32], src: &[u8], w: usize) {
        for x in 0..w {
            let l = x.saturating_sub(1) * 4;
            let c = x * 4;
            let r = (x + 1).min(w - 1) * 4;
            for ch in 0..3 {
                target[x * 3 + ch] =
                    0.25 * src[l + ch] as f32 + 0.5 * src[c + ch] as f32 + 0.25 * src[r + ch] as f32;
            }
        }
    }

    // Prime: `cur_row` = original row 0, `hb_prev`/`hb_cur` = its blur.
    {
        let raw: &[u8] = img;
        cur_row.copy_from_slice(&raw[0..stride]);
    }
    blur_row(&mut hb_prev, &cur_row, w);
    blur_row(&mut hb_cur, &cur_row, w);

    for y in 0..h {
        let y0 = y * stride;
        // Row y+1 is still original (we only ever write row y below).
        if y + 1 < h {
            let raw: &[u8] = img;
            next_row.copy_from_slice(&raw[y0 + stride..y0 + 2 * stride]);
            blur_row(&mut hb_next, &next_row, w);
        } else {
            hb_next.copy_from_slice(&hb_cur);
        }
        {
            let raw_out: &mut [u8] = img;
            for x in 0..w {
                let i = x * 3;
                for ch in 0..3 {
                    let blur =
                        0.25 * hb_prev[i + ch] + 0.5 * hb_cur[i + ch] + 0.25 * hb_next[i + ch];
                    let o = y0 + x * 4 + ch;
                    let orig = raw_out[o] as f32;
                    raw_out[o] = (orig + strength * (orig - blur)).clamp(0.0, 255.0).round() as u8;
                }
            }
        }
        // Roll the row window: above ← current, current ← below.
        std::mem::swap(&mut hb_prev, &mut hb_cur);
        std::mem::swap(&mut hb_cur, &mut hb_next);
        std::mem::swap(&mut cur_row, &mut next_row);
    }
}

/// JSON snapshot of the parameters for the QML panel (slider positions and
/// level-handle positions). Keys are short to keep the per-tick string small:
/// `e` exposure, `c` contrast, `hi` highlights, `sh` shadows, `sa` saturation,
/// `te` temperature, `ti` tint, `se` sepia, `sp` sharpness, `bl` black,
/// `ga` gamma, `wh` white, `mh` mid handle position (0..1).
pub fn adjust_json(adjust: &Adjust) -> String {
    format!(
        "{{\"e\":{:.4},\"c\":{:.4},\"hi\":{:.4},\"sh\":{:.4},\"sa\":{:.4},\"te\":{:.4},\"ti\":{:.4},\"se\":{:.4},\"sp\":{:.4},\"bl\":{:.4},\"ga\":{:.4},\"wh\":{:.4},\"mh\":{:.4}}}",
        adjust.exposure,
        adjust.contrast,
        adjust.highlights,
        adjust.shadows,
        adjust.saturation,
        adjust.temperature,
        adjust.tint,
        adjust.sepia,
        adjust.sharpness,
        adjust.black,
        adjust.gamma,
        adjust.white,
        Adjust::handle_from_gamma(adjust.gamma)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, image::Rgba(color))
    }

    fn mean(img: &RgbaImage) -> f32 {
        let raw: &[u8] = img;
        let mut sum = 0f64;
        let mut n = 0f64;
        for px in raw.chunks_exact(4) {
            sum += px[0] as f64 + px[1] as f64 + px[2] as f64;
            n += 3.0;
        }
        (sum / n) as f32
    }

    #[test]
    fn identity_leaves_every_byte_untouched() {
        let mut img = RgbaImage::from_fn(16, 16, |x, y| {
            image::Rgba([x as u8 * 15, y as u8 * 15, (x * y) as u8, 200])
        });
        let before = img.clone();
        let adjust = Adjust::identity();
        assert!(adjust.is_identity());
        adjust.apply_in_place(&mut img);
        assert_eq!(img.as_raw(), before.as_raw());
    }

    #[test]
    fn exposure_brightens_and_darkens() {
        let base = solid(4, 4, [100, 100, 100, 255]);
        let brighter = Adjust {
            exposure: 0.5,
            ..Adjust::identity()
        }
        .apply_to(&base);
        assert!(brighter.get_pixel(0, 0).0[0] > 100);

        let darker = Adjust {
            exposure: -0.3,
            ..Adjust::identity()
        }
        .apply_to(&base);
        assert!(darker.get_pixel(0, 0).0[0] < 100);
    }

    #[test]
    fn saturation_minus_one_produces_grey() {
        let base = solid(4, 4, [200, 60, 30, 255]);
        let out = Adjust {
            saturation: -1.0,
            ..Adjust::identity()
        }
        .apply_to(&base);
        let px = out.get_pixel(0, 0).0;
        assert_eq!(px[0], px[1]);
        assert_eq!(px[1], px[2]);
    }

    #[test]
    fn sepia_warms_a_grey_pixel() {
        let base = solid(2, 2, [128, 128, 128, 255]);
        let out = Adjust {
            sepia: 1.0,
            ..Adjust::identity()
        }
        .apply_to(&base);
        let px = out.get_pixel(0, 0).0;
        assert!(px[0] > px[1], "{px:?}");
        assert!(px[1] > px[2], "{px:?}");
    }

    #[test]
    fn temperature_and_tint_move_the_expected_channels() {
        let base = solid(2, 2, [128, 128, 128, 255]);
        let warm = Adjust {
            temperature: 1.0,
            ..Adjust::identity()
        }
        .apply_to(&base)
        .get_pixel(0, 0)
        .0;
        assert!(warm[0] > 128 && warm[2] < 128, "{warm:?}");

        let magenta = Adjust {
            tint: 1.0,
            ..Adjust::identity()
        }
        .apply_to(&base)
        .get_pixel(0, 0)
        .0;
        assert!(magenta[1] < magenta[0] && magenta[1] < magenta[2], "{magenta:?}");
    }

    #[test]
    fn levels_black_point_clips_dark_pixels() {
        let base = solid(2, 2, [100, 100, 100, 255]);
        let out = Adjust {
            black: 0.5,
            ..Adjust::identity()
        }
        .apply_to(&base);
        assert_eq!(out.get_pixel(0, 0).0[0], 0);
    }

    #[test]
    fn levels_white_point_pushes_bright_pixels_to_full() {
        let base = solid(2, 2, [200, 200, 200, 255]);
        let out = Adjust {
            white: 0.6,
            ..Adjust::identity()
        }
        .apply_to(&base);
        assert_eq!(out.get_pixel(0, 0).0[0], 255);
    }

    #[test]
    fn sepia_survives_full_desaturation() {
        // 棕褐色 is a look: "-100 saturation" must not wash it back to grey
        // (saturation runs first, sepia second).
        let base = solid(2, 2, [90, 140, 200, 255]);
        let out = Adjust {
            saturation: -1.0,
            sepia: 1.0,
            ..Adjust::identity()
        }
        .apply_to(&base);
        let px = out.get_pixel(0, 0).0;
        assert!(px[0] > px[1] && px[1] > px[2], "{px:?}");
    }

    #[test]
    fn alpha_is_never_adjusted() {
        let base = solid(2, 2, [90, 120, 200, 77]);
        let out = Adjust {
            exposure: 0.8,
            saturation: 0.5,
            sharpness: 1.0,
            ..Adjust::identity()
        }
        .apply_to(&base);
        assert_eq!(out.get_pixel(0, 0).0[3], 77);
    }

    #[test]
    fn transparent_pixels_keep_their_bytes() {
        let mut img = solid(3, 3, [10, 20, 30, 0]);
        img.put_pixel(1, 1, image::Rgba([200, 100, 50, 255]));
        let out = Adjust {
            exposure: 1.0,
            ..Adjust::identity()
        }
        .apply_to(&img);
        assert_eq!(out.get_pixel(0, 0).0, [10, 20, 30, 0]);
        assert!(out.get_pixel(1, 1).0[0] > 200);
    }

    #[test]
    fn sharpness_increases_edge_contrast() {
        let img = RgbaImage::from_fn(16, 16, |x, _| {
            if x < 8 {
                image::Rgba([60, 60, 60, 255])
            } else {
                image::Rgba([190, 190, 190, 255])
            }
        });
        let out = Adjust {
            sharpness: 1.0,
            ..Adjust::identity()
        }
        .apply_to(&img);
        assert!(out.get_pixel(7, 8).0[0] < 60);
        assert!(out.get_pixel(8, 8).0[0] > 190);
        // Far away from the edge the mask is neutral.
        assert_eq!(out.get_pixel(0, 8).0[0], 60);
    }

    #[test]
    fn auto_levels_stretches_a_low_contrast_image() {
        let img = RgbaImage::from_fn(32, 32, |x, _| {
            let v = 100 + (x % 8) * 6;
            image::Rgba([v as u8, v as u8, v as u8, 255])
        });
        let hist = Histogram::of(&img);
        let (black, white) = hist.auto_levels();
        assert!(black > 0.3 && black < 0.45, "black={black}");
        assert!(white > 0.5 && white < 0.7, "white={white}");
        assert!(white > black);

        let out = Adjust {
            black,
            white,
            ..Adjust::identity()
        }
        .apply_to(&img);
        assert_eq!(out.get_pixel(0, 0).0[0], 0);
        assert_eq!(out.get_pixel(31, 0).0[0], 255);
    }

    #[test]
    fn auto_levels_on_empty_histogram_is_neutral() {
        let hist = Histogram::default();
        assert_eq!(hist.auto_levels(), (0.0, 1.0));
    }

    #[test]
    fn histogram_counts_per_channel_and_skips_transparent() {
        let mut img = solid(2, 2, [255, 0, 0, 255]);
        img.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        let hist = Histogram::of(&img);
        assert_eq!(hist.total(), 3);
        assert_eq!(hist.bin_counts(0)[BINS - 1], 3);
        assert_eq!(hist.bin_counts(2)[0], 3);
    }

    #[test]
    fn gamma_handle_roundtrip() {
        for gamma in [0.4f32, 0.8, 1.0, 1.6, 2.5] {
            let pos = Adjust::handle_from_gamma(gamma);
            let back = Adjust::gamma_from_handle(pos);
            assert!((back - gamma).abs() < 0.05, "{gamma} -> {pos} -> {back}");
        }
        assert!((Adjust::handle_from_gamma(1.0) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn sanitize_keeps_levels_in_order() {
        let mut adjust = Adjust {
            black: 0.9,
            white: 0.2,
            gamma: 0.0,
            ..Adjust::identity()
        };
        adjust.sanitize();
        assert!(adjust.white >= adjust.black + 0.02);
        assert!(adjust.gamma >= 0.05);
    }

    #[test]
    fn contrast_pivots_around_mid_grey() {
        let mid = solid(2, 2, [128, 128, 128, 255]);
        let out = Adjust {
            contrast: 0.5,
            ..Adjust::identity()
        }
        .apply_to(&mid);
        // 128 is 0.5 px above the 127.5 pivot, so the slope may round to ±1.
        let px = out.get_pixel(0, 0).0[0] as i32;
        assert!((px - 128).abs() <= 1, "{px}");

        let dark = solid(2, 2, [90, 90, 90, 255]);
        let out = Adjust {
            contrast: 0.5,
            ..Adjust::identity()
        }
        .apply_to(&dark);
        assert!(out.get_pixel(0, 0).0[0] < 90);
    }

    #[test]
    fn shadows_and_highlights_act_on_their_own_range() {
        let dark = solid(2, 2, [40, 40, 40, 255]);
        let bright = solid(2, 2, [220, 220, 220, 255]);
        let lift = Adjust {
            shadows: 1.0,
            ..Adjust::identity()
        };
        let hi = Adjust {
            highlights: 1.0,
            ..Adjust::identity()
        };
        assert!(lift.apply_to(&dark).get_pixel(0, 0).0[0] > 60);
        assert!(hi.apply_to(&bright).get_pixel(0, 0).0[0] > 220);
        // Highlights barely touch deep shadows.
        assert!(hi.apply_to(&dark).get_pixel(0, 0).0[0] < 60);
        let _ = mean(&lift.apply_to(&bright));
    }
}
