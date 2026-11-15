use super::PsdLayerData;

/// Which already-pushed layers an adjustment layer applies to.
///
/// A clipped adjustment (Photoshop's "剪贴蒙版", the usual way a designer
/// recolours one button graphic) only touches the chain it clips to. Whether
/// that means "the whole clip chain plus its base" or "just the layer directly
/// below" is measurable, not obvious — Photoshop chains adjustments through each
/// other, which a flat baking pass cannot do exactly — so both are available
/// under `IMAGEEDIT_PSD_ADJUST_SCOPE` for the fidelity harness to decide.
///
/// The result is always clamped to the adjustment's own group: a group is a
/// separate composite in Photoshop, so layers outside it are never in scope.
pub(super) fn adjustment_scope(
    layers: &[PsdLayerData],
    clips: &[bool],
    group: Option<u32>,
    clipped: bool,
    mode: ScopeMode,
) -> usize {
    let len = layers.len();
    let mut start = match mode {
        ScopeMode::All => 0,
        ScopeMode::Last => len.saturating_sub(1),
        ScopeMode::Chain => {
            if !clipped {
                0
            } else {
                let mut s = len;
                while s > 0 && clips[s - 1] {
                    s -= 1;
                }
                // Include the base layer the chain clips to.
                s.saturating_sub(1)
            }
        }
    };
    if !cross_group() {
        // Back up to the first layer of this group's contiguous run, so the
        // adjustment never leaks into a sibling group's pixels.
        let mut g = len;
        while g > 0 && layers[g - 1].group == group {
            g -= 1;
        }
        start = start.max(g);
    }
    start.min(len)
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ScopeMode {
    /// Everything below (the pre-2026-11 fallback).
    All,
    /// Only the layer directly below (Photoshop's literal clip target).
    Last,
    /// The trailing clip run plus the base it clips to.
    Chain,
}

pub(super) fn adjust_scope_mode() -> ScopeMode {
    match std::env::var("IMAGEEDIT_PSD_ADJUST_SCOPE").ok().as_deref() {
        Some("all") => ScopeMode::All,
        Some("last") => ScopeMode::Last,
        _ => ScopeMode::Chain,
    }
}

pub(super) fn scope_mode_name(mode: ScopeMode) -> &'static str {
    match mode {
        ScopeMode::All => "all",
        ScopeMode::Last => "last",
        ScopeMode::Chain => "chain",
    }
}

fn cross_group() -> bool {
    static CROSS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CROSS.get_or_init(|| std::env::var("IMAGEEDIT_PSD_ADJUST_CROSS_GROUP").ok().as_deref() == Some("1"))
}

/// Bake one adjustment layer's effect into the pixels of `targets`.
///
/// Adjustment layers have no bitmap of their own, so the only way to show what
/// Photoshop shows is to apply the operation to the layers below — exactly what
/// "拼合图层" would do. Alpha is never touched: an adjustment changes colour,
/// not coverage.
pub(super) fn apply_adjustment(
    targets: &mut [PsdLayerData],
    bc: Option<psd::BrightnessContrast>,
    hs: Option<psd::HueSaturation>,
    mask: Option<&editcore::LayerMask>,
    clipped: bool,
) {
    // 剪贴调整层只吃基底的「内容」alpha；非剪贴的调整层（作用于整条链）没有
    // 这个限制。可用 `IMAGEEDIT_PSD_ADJUST_FULL_ALPHA=1` 关掉做消融。
    let content_alpha = clipped && !full_alpha_adjust();
    for layer in targets.iter_mut() {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        let pre = if content_alpha {
            layer.content_alpha.as_ref()
        } else {
            None
        };
        // Fast path: no mask and no content-alpha limit means every pixel takes
        // the full adjustment, which is the common case and must stay as cheap
        // as the original loop.
        if mask.is_none() && pre.is_none() {
            for px in layer.image.pixels_mut() {
                if px.0[3] == 0 {
                    continue;
                }
                let mut rgb = [px.0[0], px.0[1], px.0[2]];
                if let Some(hs) = hs {
                    rgb = hue_saturation(rgb, hs);
                }
                if let Some(bc) = bc {
                    rgb = brightness_contrast(rgb, bc);
                }
                px.0[0] = rgb[0];
                px.0[1] = rgb[1];
                px.0[2] = rgb[2];
            }
            continue;
        }
        let (ox, oy) = layer.offset;
        let (w, h) = (layer.image.width() as i32, layer.image.height() as i32);
        for y in 0..h {
            for x in 0..w {
                // How much of the adjustment reaches this pixel: all of it where
                // the adjustment's mask is white, none where it is black, times
                // the base layer's content alpha for a clipped adjustment.
                let mut k = match mask {
                    Some(mask) => mask.coverage(ox + x, oy + y),
                    None => 1.0,
                };
                if let Some(pre) = pre {
                    k *= pre.at(ox + x, oy + y);
                }
                let k = k.clamp(0.0, 1.0);
                if k <= 0.0 {
                    continue;
                }
                let px = layer.image.get_pixel_mut(x as u32, y as u32);
                if px.0[3] == 0 {
                    continue;
                }
                let src = px.0;
                let mut rgb = [src[0], src[1], src[2]];
                if let Some(hs) = hs {
                    rgb = hue_saturation(rgb, hs);
                }
                if let Some(bc) = bc {
                    rgb = brightness_contrast(rgb, bc);
                }
                if k < 1.0 {
                    rgb = [
                        (src[0] as f32 + (rgb[0] as f32 - src[0] as f32) * k).round() as u8,
                        (src[1] as f32 + (rgb[1] as f32 - src[1] as f32) * k).round() as u8,
                        (src[2] as f32 + (rgb[2] as f32 - src[2] as f32) * k).round() as u8,
                    ];
                }
                px.0[0] = rgb[0];
                px.0[1] = rgb[1];
                px.0[2] = rgb[2];
            }
        }
    }
}

/// 剪贴调整层按 Photoshop 的顺序作用在「图层内容」上：调整层夹在内容与
/// 图层样式之间，所以内阴影 / 描边不会被二次上色。
///
/// 与 [`apply_adjustment`] 的区别只在对象与限幅：这里直接吃烘焙样式之前的
/// 原始像素（alpha = 0 的像素没有颜色可调），覆盖面只由调整层自己的蒙版
/// 决定，不再乘基底的「内容 alpha」。
pub(super) fn apply_content_adjustment(
    image: &mut image::RgbaImage,
    offset: (i32, i32),
    bc: Option<psd::BrightnessContrast>,
    hs: Option<psd::HueSaturation>,
    mask: Option<&editcore::LayerMask>,
) {
    if bc.is_none() && hs.is_none() {
        return;
    }
    let (ox, oy) = offset;
    let (w, h) = (image.width() as i32, image.height() as i32);
    let Some(mask) = mask else {
        for px in image.pixels_mut() {
            if px.0[3] == 0 {
                continue;
            }
            let mut rgb = [px.0[0], px.0[1], px.0[2]];
            if let Some(hs) = hs {
                rgb = hue_saturation(rgb, hs);
            }
            if let Some(bc) = bc {
                rgb = brightness_contrast(rgb, bc);
            }
            px.0[0] = rgb[0];
            px.0[1] = rgb[1];
            px.0[2] = rgb[2];
        }
        return;
    };
    for y in 0..h {
        for x in 0..w {
            let k = mask.coverage(ox + x, oy + y).clamp(0.0, 1.0);
            if k <= 0.0 {
                continue;
            }
            let px = image.get_pixel_mut(x as u32, y as u32);
            if px.0[3] == 0 {
                continue;
            }
            let src = px.0;
            let mut rgb = [src[0], src[1], src[2]];
            if let Some(hs) = hs {
                rgb = hue_saturation(rgb, hs);
            }
            if let Some(bc) = bc {
                rgb = brightness_contrast(rgb, bc);
            }
            if k < 1.0 {
                rgb = [
                    (src[0] as f32 + (rgb[0] as f32 - src[0] as f32) * k).round() as u8,
                    (src[1] as f32 + (rgb[1] as f32 - src[1] as f32) * k).round() as u8,
                    (src[2] as f32 + (rgb[2] as f32 - src[2] as f32) * k).round() as u8,
                ];
            }
            px.0[0] = rgb[0];
            px.0[1] = rgb[1];
            px.0[2] = rgb[2];
        }
    }
}

fn full_alpha_adjust() -> bool {
    static FULL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FULL.get_or_init(|| std::env::var("IMAGEEDIT_PSD_ADJUST_FULL_ALPHA").ok().as_deref() == Some("1"))
}

/// Photoshop's brightness/contrast sliders (legacy range -100..=100).
fn brightness_contrast(rgb: [u8; 3], bc: psd::BrightnessContrast) -> [u8; 3] {
    if bc.brightness == 0 && bc.contrast == 0 {
        return rgb;
    }
    let bright = bc.brightness as f32 / 100.0;
    let contrast = 1.0 + bc.contrast as f32 / 100.0;
    let mut out = rgb;
    for c in 0..3 {
        let v = rgb[c] as f32 / 255.0;
        let v = (v - 0.5) * contrast + 0.5 + bright;
        out[c] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Photoshop's hue/saturation adjustment in HSL space.
fn hue_saturation(rgb: [u8; 3], hs: psd::HueSaturation) -> [u8; 3] {
    let (mut h, mut s, mut l) = rgb_to_hsl(rgb);
    if hs.colorize {
        // "着色": flatten the colour to a single hue first, then apply.
        s = 0.0;
        l = l * 0.5;
        h = 0.0;
        h = (h + hs.colorize_hue as f32 / 360.0).rem_euclid(1.0);
        s = apply_slider(s, hs.colorize_saturation as f32);
        l = apply_slider(l, hs.colorize_lightness as f32);
    } else {
        h = (h + hs.hue as f32 / 360.0).rem_euclid(1.0);
        s = apply_slider(s, hs.saturation as f32);
        l = apply_slider(l, hs.lightness as f32);
    }
    hsl_to_rgb(h, s, l)
}

/// Photoshop's -100..=100 sliders move a 0..1 value towards the corresponding
/// end of its range instead of scaling it linearly (so +100 really is "max").
fn apply_slider(v: f32, slider: f32) -> f32 {
    let t = (slider / 100.0).clamp(-1.0, 1.0);
    if t >= 0.0 {
        v + (1.0 - v) * t
    } else {
        v * (1.0 + t)
    }
}

fn rgb_to_hsl(rgb: [u8; 3]) -> (f32, f32, f32) {
    let r = rgb[0] as f32 / 255.0;
    let g = rgb[1] as f32 / 255.0;
    let b = rgb[2] as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    if s <= 0.0 {
        let v = (l * 255.0).round().clamp(0.0, 255.0) as u8;
        return [v, v, v];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |t: f32| -> f32 {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [
        (f(h + 1.0 / 3.0) * 255.0).round().clamp(0.0, 255.0) as u8,
        (f(h) * 255.0).round().clamp(0.0, 255.0) as u8,
        (f(h - 1.0 / 3.0) * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}
