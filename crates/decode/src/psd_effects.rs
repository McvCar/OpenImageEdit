//! Baking of Photoshop layer styles (`lfx2`) into a layer bitmap.
//!
//! Photoshop keeps styles as parameters, not pixels: a button's drop shadow,
//! its gradient fill and its 1 px stroke never appear in the layer's channel
//! data. Without applying them the file looks flat — text with a `stroke` style
//! loses its outline, a `gradient overlay` title loses its fill.
//!
//! The document model has no live style stack, so the honest thing to do is
//! what "拼合图层" does: render the styles once, into the layer's pixels. The
//! layer keeps its identity, position and opacity; the style becomes part of
//! its bitmap. The user is told, once, that styles were baked.
//!
//! Rendering order follows Photoshop's own stack (bottom to top):
//!
//! ```text
//! drop shadow / outer glow   (behind the content)
//! layer content
//! stroke, inner shadow, inner glow   (clipped to the content)
//! colour / gradient overlay          (over the content)
//! ```

use image::{Rgba, RgbaImage};
use psd::effects::{
    EffectKind, GlowSource, Gradient, GradientKind, LayerEffect, StrokeFill, StrokePosition,
};

/// Result of baking: a possibly larger bitmap plus where it now sits.
pub struct Baked {
    /// Pixels with the styles rendered in.
    pub image: RgbaImage,
    /// Canvas position of the new bitmap's top-left corner.
    pub offset: (i32, i32),
    /// Per-layer notes (unsupported style kinds, size caps).
    pub notes: Vec<String>,
}

/// Bake every enabled style into `image`.
///
/// Returns `None` when there is nothing to do, so callers can keep the
/// original allocation. `offset` is the canvas position of `image`'s top-left
/// corner; effects that paint outside the layer (shadow, outside stroke) grow
/// the bitmap and push the offset accordingly.
pub fn bake(image: &RgbaImage, offset: (i32, i32), effects: &[LayerEffect]) -> Option<Baked> {
    let active: Vec<&LayerEffect> = effects.iter().filter(|e| e.enabled).collect();
    if active.is_empty() {
        return None;
    }
    let mut notes = Vec::new();
    let (iw, ih) = (image.width(), image.height());
    if iw == 0 || ih == 0 {
        return None;
    }

    // ---- how far the styles reach outside the layer rectangle -----------
    let mut pad = Padding::default();
    for e in &active {
        match e.kind {
            EffectKind::DropShadow | EffectKind::OuterGlow => {
                let margin = blur_margin(e.size);
                let (dx, dy) = (e.distance * e.angle.to_radians().cos(), e.distance * e.angle.to_radians().sin());
                let (dx, dy) = if e.kind == EffectKind::DropShadow {
                    (dx.round() as i32, dy.round() as i32)
                } else {
                    (0, 0)
                };
                pad.left = pad.left.max(margin + (-dx).max(0));
                pad.right = pad.right.max(margin + dx.max(0));
                pad.top = pad.top.max(margin + (-dy).max(0));
                pad.bottom = pad.bottom.max(margin + dy.max(0));
            }
            EffectKind::Stroke => {
                let reach = match e.stroke_position {
                    StrokePosition::Outside => e.size.ceil() as i32 + 2,
                    StrokePosition::Center => (e.size / 2.0).ceil() as i32 + 2,
                    StrokePosition::Inside => 0,
                };
                pad.left = pad.left.max(reach);
                pad.right = pad.right.max(reach);
                pad.top = pad.top.max(reach);
                pad.bottom = pad.bottom.max(reach);
            }
            _ => {}
        }
    }
    // A pathological style must not turn a 4K layer into a 40K one.
    let cap = 512;
    pad.left = pad.left.min(cap);
    pad.right = pad.right.min(cap);
    pad.top = pad.top.min(cap);
    pad.bottom = pad.bottom.min(cap);

    let (w, h) = (
        (iw as i64 + pad.left as i64 + pad.right as i64).max(1) as u32,
        (ih as i64 + pad.top as i64 + pad.bottom as i64).max(1) as u32,
    );
    if w as u64 * h as u64 > 60_000_000 {
        notes.push("图层样式需要过大的画布，已跳过".into());
        return None;
    }

    let mut out = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
    let ox = pad.left as i64;
    let oy = pad.top as i64;

    // ---- layer alpha in the expanded coordinate system ------------------
    let mut alpha = vec![0.0f32; (w * h) as usize];
    for y in 0..ih {
        for x in 0..iw {
            let a = image.get_pixel(x, y).0[3] as f32 / 255.0;
            let (tx, ty) = ((x as i64 + ox) as u32, (y as i64 + oy) as u32);
            alpha[(ty * w + tx) as usize] = a;
        }
    }

    // ---- 1. behind the content: drop shadow, outer glow ------------------
    for e in &active {
        match e.kind {
            EffectKind::DropShadow => draw_drop_shadow(&mut out, w, h, &alpha, e),
            EffectKind::OuterGlow => draw_outer_glow(&mut out, w, h, &alpha, e),
            _ => {}
        }
    }

    // ---- 2. the content itself ------------------------------------------
    {
        let data = out.as_mut();
        for y in 0..ih {
            for x in 0..iw {
                let p = *image.get_pixel(x, y);
                if p.0[3] == 0 {
                    continue;
                }
                let (tx, ty) = ((x as i64 + ox) as usize, (y as i64 + oy) as usize);
                let idx = (ty * w as usize + tx) * 4;
                let dst = &mut data[idx..idx + 4];
                let mut px = [dst[0], dst[1], dst[2], dst[3]];
                editcore::blend_pixel(&mut px, p.0, 1.0, editcore::BlendMode::Normal);
                dst.copy_from_slice(&px);
            }
        }
    }

    // ---- 3. inside the content: stroke, inner shadow, inner glow --------
    let bbox = alpha_bounds(&alpha, w, h);
    // One distance field per direction, shared by every style that needs it.
    let need_in = active.iter().any(|e| {
        e.kind == EffectKind::InnerGlow
            || e.kind == EffectKind::InnerShadow
            || (e.kind == EffectKind::Stroke && e.stroke_position != StrokePosition::Outside)
    });
    let need_out = active.iter().any(|e| {
        e.kind == EffectKind::OuterGlow
            || (e.kind == EffectKind::Stroke && e.stroke_position != StrokePosition::Inside)
    });
    let inside: Vec<bool> = alpha.iter().map(|a| *a >= 0.5).collect();
    let outside: Vec<bool> = inside.iter().map(|b| !b).collect();
    let dist_in = if need_in {
        distance_to(&outside, w, h)
    } else {
        Vec::new()
    };
    let dist_out = if need_out {
        distance_to(&inside, w, h)
    } else {
        Vec::new()
    };

    for e in &active {
        match e.kind {
            EffectKind::Stroke => {
                draw_stroke(&mut out, w, h, &alpha, &dist_in, &dist_out, bbox, e);
            }
            EffectKind::InnerShadow => {
                draw_inner_shadow(&mut out, w, h, &alpha, &dist_in, e);
            }
            EffectKind::InnerGlow => {
                draw_inner_glow(&mut out, w, h, &alpha, &dist_in, e);
            }
            _ => {}
        }
    }

    // ---- 4. overlays, on top ---------------------------------------------
    for e in &active {
        match e.kind {
            EffectKind::ColorOverlay => draw_color_overlay(&mut out, w, h, &alpha, e),
            EffectKind::GradientOverlay => draw_gradient_overlay(&mut out, w, h, &alpha, bbox, e),
            EffectKind::PatternOverlay => {
                notes.push("图层样式「图案叠加」暂不支持".into());
            }
            EffectKind::BevelEmboss => {
                notes.push("图层样式「斜面和浮雕」暂不支持".into());
            }
            _ => {}
        }
    }

    let new_offset = (offset.0 - pad.left, offset.1 - pad.top);
    Some(Baked {
        image: out,
        offset: new_offset,
        notes,
    })
}

#[derive(Default)]
struct Padding {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

/// How far a gaussian of `size` reaches before it is visually zero.
fn blur_margin(size: f32) -> i32 {
    (size * 1.5).ceil() as i32 + 2
}

/// Axis-aligned bounds of everything with alpha above noise, in pixels.
fn alpha_bounds(alpha: &[f32], w: u32, h: u32) -> [f32; 4] {
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
    let w = out.width();
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

fn draw_drop_shadow(out: &mut RgbaImage, w: u32, h: u32, alpha: &[f32], e: &LayerEffect) {
    let (dx, dy) = offset_vector(e.distance, e.angle);
    let shifted = shift_mask(alpha, w, h, dx, dy);
    let soft = blur_mask(&shifted, w, h, e.size);
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let mut cov = soft[i];
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

fn draw_outer_glow(out: &mut RgbaImage, w: u32, h: u32, alpha: &[f32], e: &LayerEffect) {
    let inside: Vec<bool> = alpha.iter().map(|a| *a >= 0.5).collect();
    let dist = distance_to(&inside, w, h);
    let radius = e.size.max(0.5);
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let d = dist[(y * w + x) as usize];
            if d <= 0.0 {
                continue;
            }
            let cov = (1.0 - d / radius).clamp(0.0, 1.0);
            if cov <= 0.002 {
                continue;
            }
            composite(out, x, y, e.color, cov, e.opacity, mode);
        }
    }
}

fn draw_stroke(
    out: &mut RgbaImage,
    w: u32,
    h: u32,
    alpha: &[f32],
    dist_in: &[f32],
    dist_out: &[f32],
    bbox: [f32; 4],
    e: &LayerEffect,
) {
    let size = e.size.max(0.5);
    let mode = blend_of(e.blend);
    let gradient = e.gradient.as_ref().filter(|_| e.stroke_fill == StrokeFill::Gradient);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let cov = match e.stroke_position {
                StrokePosition::Outside => {
                    let d = dist_out.get(i).copied().unwrap_or(0.0);
                    if d < 0.5 {
                        continue;
                    }
                    (size + 0.5 - d).clamp(0.0, 1.0)
                }
                StrokePosition::Inside => {
                    let d = dist_in.get(i).copied().unwrap_or(0.0);
                    (size + 0.5 - d).clamp(0.0, 1.0) * alpha[i]
                }
                StrokePosition::Center => {
                    let half = size / 2.0;
                    let d = if dist_out.get(i).copied().unwrap_or(0.0) > 0.0 {
                        dist_out[i]
                    } else {
                        dist_in.get(i).copied().unwrap_or(0.0)
                    };
                    (half + 0.5 - d).clamp(0.0, 1.0)
                }
            };
            if cov <= 0.002 {
                continue;
            }
            let (color, mut a) = match gradient {
                Some(g) => {
                    let t = gradient_t(x as f32, y as f32, bbox, g);
                    let (c, ga) = g.sample(t);
                    (c, ga)
                }
                None => (e.color, 1.0),
            };
            a *= e.opacity;
            if a <= 0.002 {
                continue;
            }
            composite(out, x, y, color, cov, a, mode);
        }
    }
}

fn draw_inner_shadow(
    out: &mut RgbaImage,
    w: u32,
    h: u32,
    alpha: &[f32],
    dist_in: &[f32],
    e: &LayerEffect,
) {
    let (dx, dy) = offset_vector(e.distance, e.angle);
    // The shadow is the shape's silhouette moved the other way, blurred, then
    // clipped to the shape: exactly Photoshop's "inner shadow".
    let inverse: Vec<f32> = alpha.iter().map(|a| 1.0 - a).collect();
    let shifted = shift_mask(&inverse, w, h, -dx, -dy);
    let mut soft = blur_mask(&shifted, w, h, e.size);
    if e.choke > 0.0 {
        // Choke pulls the shadow towards the edge; approximating it by
        // dropping the blurred value where the pixel is "far enough" in.
        let reach = e.choke / 100.0 * e.size.max(1.0);
        for i in 0..soft.len() {
            let d = dist_in.get(i).copied().unwrap_or(0.0);
            if d > 0.0 && d < reach {
                soft[i] = soft[i].max(0.5);
            }
        }
    }
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let cov = alpha[i] * soft[i];
            if cov <= 0.002 {
                continue;
            }
            composite(out, x, y, e.color, cov, e.opacity, mode);
        }
    }
}

fn draw_inner_glow(
    out: &mut RgbaImage,
    w: u32,
    h: u32,
    alpha: &[f32],
    dist_in: &[f32],
    e: &LayerEffect,
) {
    let radius = e.size.max(0.5);
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if alpha[i] <= 0.002 {
                continue;
            }
            let d = dist_in.get(i).copied().unwrap_or(0.0);
            let t = match e.glow_source {
                GlowSource::Edge => 1.0 - d / radius,
                GlowSource::Center => d / radius,
            }
            .clamp(0.0, 1.0);
            let cov = alpha[i] * t;
            if cov <= 0.002 {
                continue;
            }
            composite(out, x, y, e.color, cov, e.opacity, mode);
        }
    }
}

fn draw_color_overlay(out: &mut RgbaImage, w: u32, h: u32, alpha: &[f32], e: &LayerEffect) {
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let cov = alpha[(y * w + x) as usize];
            if cov <= 0.002 {
                continue;
            }
            composite(out, x, y, e.color, cov, e.opacity, mode);
        }
    }
}

fn draw_gradient_overlay(
    out: &mut RgbaImage,
    w: u32,
    h: u32,
    alpha: &[f32],
    bbox: [f32; 4],
    e: &LayerEffect,
) {
    let Some(g) = e.gradient.as_ref() else {
        return;
    };
    let mode = blend_of(e.blend);
    for y in 0..h {
        for x in 0..w {
            let cov = alpha[(y * w + x) as usize];
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

fn blend_of(mode: psd::BlendMode) -> editcore::BlendMode {
    crate::psd_layers::blend_of(mode)
}

/// Photoshop's angle/distance pair as an image-space offset.
///
/// Photoshop measures the light angle counter-clockwise from the +x axis with
/// +y pointing **up**, so 90° is "light from the top" and pushes a drop shadow
/// down the screen. Image rows grow downwards, hence the unchanged sign on y.
fn offset_vector(distance: f32, angle_deg: f32) -> (i32, i32) {
    let rad = angle_deg.to_radians();
    (
        (distance * rad.cos()).round() as i32,
        (distance * rad.sin()).round() as i32,
    )
}

/// Position of `(x, y)` along the ramp, 0..1.
fn gradient_t(x: f32, y: f32, bbox: [f32; 4], g: &Gradient) -> f32 {
    let cx = (bbox[0] + bbox[2]) * 0.5;
    let cy = (bbox[1] + bbox[3]) * 0.5;
    let hw = ((bbox[2] - bbox[0]) * 0.5).max(0.5);
    let hh = ((bbox[3] - bbox[1]) * 0.5).max(0.5);
    let scale = (g.scale / 100.0).clamp(0.01, 100.0);
    let t = match g.kind {
        GradientKind::Linear | GradientKind::Reflected => {
            let rad = g.angle.to_radians();
            // +y is up for Photoshop angles, so negate the row component.
            let dir = (rad.cos(), -rad.sin());
            let proj = (x - cx) * dir.0 + (y - cy) * dir.1;
            let half = (hw * dir.0.abs() + hh * dir.1.abs()).max(0.5) * scale;
            let t = proj / (2.0 * half) + 0.5;
            if g.kind == GradientKind::Reflected {
                1.0 - (t - 0.5).abs() * 2.0
            } else {
                t
            }
        }
        GradientKind::Radial => {
            let r = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            let max_r = (hw.max(hh) * scale).max(0.5);
            r / max_r
        }
        GradientKind::Angle => {
            let a = (y - cy).atan2(x - cx) + std::f32::consts::FRAC_PI_2;
            (a / std::f32::consts::TAU).rem_euclid(1.0)
        }
        GradientKind::Diamond => {
            let dx = (x - cx).abs() / hw.max(0.5);
            let dy = (y - cy).abs() / hh.max(0.5);
            ((dx + dy) / scale).clamp(0.0, 1.0)
        }
    };
    t.clamp(0.0, 1.0)
}

/// `out[p] = mask[p - (dx, dy)]`, i.e. the mask's content moved by (+dx, +dy).
fn shift_mask(mask: &[f32], w: u32, h: u32, dx: i32, dy: i32) -> Vec<f32> {
    let mut out = vec![0.0f32; mask.len()];
    if dx == 0 && dy == 0 {
        out.copy_from_slice(mask);
        return out;
    }
    for y in 0..h as i32 {
        let sy = y - dy;
        if sy < 0 || sy >= h as i32 {
            continue;
        }
        for x in 0..w as i32 {
            let sx = x - dx;
            if sx < 0 || sx >= w as i32 {
                continue;
            }
            out[(y as u32 * w + x as u32) as usize] = mask[(sy as u32 * w + sx as u32) as usize];
        }
    }
    out
}

/// Three box passes ≈ a gaussian of `sigma = size / 2`, which is how Photoshop
/// scales its "size" slider.
fn blur_mask(mask: &[f32], w: u32, h: u32, size: f32) -> Vec<f32> {
    if size <= 0.05 || w < 2 || h < 2 {
        return mask.to_vec();
    }
    let sigma = (size * 0.5).max(0.5);
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let radius = (((ideal - 1.0) / 2.0).floor() as i32).max(1);
    let mut cur = mask.to_vec();
    for _ in 0..3 {
        cur = box_blur(&cur, w, h, radius);
    }
    cur
}

fn box_blur(src: &[f32], w: u32, h: u32, radius: i32) -> Vec<f32> {
    let (wu, hu) = (w as usize, h as usize);
    let mut tmp = vec![0.0f32; src.len()];
    let mut out = vec![0.0f32; src.len()];
    let window = (radius * 2 + 1) as f32;
    // Horizontal.
    for y in 0..hu {
        let row = y * wu;
        let mut sum = 0.0f32;
        for x in 0..=radius as usize {
            if x < wu {
                sum += src[row + x];
            }
        }
        for x in 0..wu {
            tmp[row + x] = sum / window;
            let add = x as i32 + radius + 1;
            if add < wu as i32 {
                sum += src[row + add as usize];
            }
            let sub = x as i32 - radius;
            if sub >= 0 {
                sum -= src[row + sub as usize];
            }
        }
    }
    // Vertical.
    for x in 0..wu {
        let mut sum = 0.0f32;
        for y in 0..=radius as usize {
            if y < hu {
                sum += tmp[y * wu + x];
            }
        }
        for y in 0..hu {
            out[y * wu + x] = sum / window;
            let add = y as i32 + radius + 1;
            if add < hu as i32 {
                sum += tmp[add as usize * wu + x];
            }
            let sub = y as i32 - radius;
            if sub >= 0 {
                sum -= tmp[sub as usize * wu + x];
            }
        }
    }
    out
}

/// Distance from every pixel to the nearest pixel where `mask` is true.
///
/// Two-pass chamfer with 1 / √2 steps. Good enough for stroke bands and glows,
/// and O(pixels) instead of the O(pixels × radius) of a dilation loop.
fn distance_to(mask: &[bool], w: u32, h: u32) -> Vec<f32> {
    let (wu, hu) = (w as usize, h as usize);
    const INF: f32 = 1.0e9;
    let mut d = vec![INF; mask.len()];
    for i in 0..mask.len() {
        if mask[i] {
            d[i] = 0.0;
        }
    }
    let diag = std::f32::consts::SQRT_2;
    for y in 0..hu {
        for x in 0..wu {
            let i = y * wu + x;
            let mut best = d[i];
            if x > 0 {
                best = best.min(d[i - 1] + 1.0);
            }
            if y > 0 {
                best = best.min(d[i - wu] + 1.0);
                if x > 0 {
                    best = best.min(d[i - wu - 1] + diag);
                }
                if x + 1 < wu {
                    best = best.min(d[i - wu + 1] + diag);
                }
            }
            d[i] = best;
        }
    }
    for y in (0..hu).rev() {
        for x in (0..wu).rev() {
            let i = y * wu + x;
            let mut best = d[i];
            if x + 1 < wu {
                best = best.min(d[i + 1] + 1.0);
            }
            if y + 1 < hu {
                best = best.min(d[i + wu] + 1.0);
                if x + 1 < wu {
                    best = best.min(d[i + wu + 1] + diag);
                }
                if x > 0 {
                    best = best.min(d[i + wu - 1] + diag);
                }
            }
            d[i] = best;
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use psd::BlendMode;

    fn layer(w: u32, h: u32, color: [u8; 4], inset: u32) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
        for y in inset..h - inset {
            for x in inset..w - inset {
                img.put_pixel(x, y, Rgba(color));
            }
        }
        img
    }

    fn effect(kind: EffectKind) -> LayerEffect {
        LayerEffect {
            kind,
            enabled: true,
            blend: BlendMode::Normal,
            color: [0, 0, 0],
            opacity: 1.0,
            angle: 90.0,
            distance: 0.0,
            size: 0.0,
            choke: 0.0,
            stroke_position: StrokePosition::Outside,
            stroke_fill: StrokeFill::Color,
            gradient: None,
            knocks_out: true,
            glow_source: GlowSource::Edge,
        }
    }

    #[test]
    fn no_enabled_effect_means_no_work() {
        let img = layer(8, 8, [10, 20, 30, 255], 2);
        assert!(bake(&img, (0, 0), &[]).is_none());
        let mut disabled = effect(EffectKind::ColorOverlay);
        disabled.enabled = false;
        let img = layer(8, 8, [10, 20, 30, 255], 2);
        assert!(bake(&img, (0, 0), &[disabled]).is_none());
    }

    #[test]
    fn color_overlay_replaces_the_fill_and_keeps_alpha() {
        let img = layer(8, 8, [10, 20, 30, 255], 2);
        let mut e = effect(EffectKind::ColorOverlay);
        e.color = [200, 10, 10];
        let baked = bake(&img, (0, 0), &[e]).expect("baked");
        assert_eq!(baked.image.get_pixel(4, 4).0, [200, 10, 10, 255]);
        // Outside the shape stays transparent.
        assert_eq!(baked.image.get_pixel(0, 0).0[3], 0);
        assert_eq!(baked.offset, (0, 0));
    }

    #[test]
    fn outside_stroke_grows_the_bitmap_and_paints_the_border() {
        let img = layer(8, 8, [0, 0, 0, 255], 2);
        let mut e = effect(EffectKind::Stroke);
        e.size = 2.0;
        e.color = [255, 0, 0];
        let baked = bake(&img, (10, 10), &[e]).expect("baked");
        // Four pixels of room on each side.
        assert_eq!(baked.image.dimensions(), (16, 16));
        assert_eq!(baked.offset, (6, 6));
        // The square sits at (6..9): one pixel outside its edge is the ring.
        assert_eq!(baked.image.get_pixel(5, 6).0, [255, 0, 0, 255]);
        assert_eq!(baked.image.get_pixel(6, 5).0, [255, 0, 0, 255]);
        // The shape itself keeps the layer colour.
        assert_eq!(baked.image.get_pixel(7, 7).0, [0, 0, 0, 255]);
        // Far outside the stroke the bitmap is still transparent.
        assert_eq!(baked.image.get_pixel(0, 0).0[3], 0);
    }

    #[test]
    fn drop_shadow_paints_below_the_content() {
        let img = layer(6, 6, [255, 255, 255, 255], 1);
        let mut e = effect(EffectKind::DropShadow);
        e.size = 1.0;
        e.distance = 2.0;
        e.color = [0, 0, 0];
        let baked = bake(&img, (0, 0), &[e]).expect("baked");
        // Shadow moved down by two rows: the pixel under the square is dark,
        // the pixel above it is untouched.
        let below = baked.image.get_pixel(3, 6).0;
        assert!(below[0] < 200, "expected a shadow below, got {below:?}");
        let above = baked.image.get_pixel(3, 0).0;
        assert_eq!(above[3], 0, "no shadow above, got {above:?}");
    }

    #[test]
    fn gradient_overlay_follows_the_angle() {
        let img = layer(10, 10, [0, 0, 0, 255], 0);
        let mut e = effect(EffectKind::GradientOverlay);
        e.gradient = Some(Gradient {
            color_stops: vec![
                psd::effects::ColorStop {
                    location: 0.0,
                    color: [0, 0, 0],
                },
                psd::effects::ColorStop {
                    location: 1.0,
                    color: [255, 255, 255],
                },
            ],
            alpha_stops: vec![psd::effects::AlphaStop {
                location: 0.0,
                alpha: 1.0,
            }],
            kind: GradientKind::Linear,
            angle: 90.0,
            reverse: false,
            align: true,
            scale: 100.0,
        });
        let baked = bake(&img, (0, 0), &[e]).expect("baked");
        // Angle 90 means the ramp runs bottom -> top.
        let top = baked.image.get_pixel(5, 0).0[0] as i32;
        let bottom = baked.image.get_pixel(5, 9).0[0] as i32;
        assert!(top > bottom + 100, "top {top} bottom {bottom}");
    }

    #[test]
    fn distance_transform_is_zero_inside_and_counts_outwards() {
        let (w, h) = (5u32, 5u32);
        let mut mask = vec![false; 25];
        mask[12] = true; // centre
        let d = distance_to(&mask, w, h);
        assert_eq!(d[12], 0.0);
        assert!((d[13] - 1.0).abs() < 1e-3);
        assert!((d[8] - std::f32::consts::SQRT_2).abs() < 1e-3);
    }

    #[test]
    fn box_blur_keeps_the_mass() {
        let mut src = vec![0.0f32; 49];
        src[24] = 1.0;
        let blurred = box_blur(&src, 7, 7, 1);
        let sum: f32 = blurred.iter().sum();
        assert!((sum - 1.0).abs() < 1e-3, "sum {sum}");
    }
}

/// Exposed for the diagnostic test below.
#[cfg(test)]
pub(crate) fn parse_layer_effects_for_debug(bytes: &[u8]) -> Option<Vec<psd::effects::LayerEffect>> {
    psd::effects::parse_layer_effects(bytes)
}

#[cfg(test)]
mod debug_probe {
    /// Diagnostic: parse every `lfx2` block in a file and report how many the
    /// descriptor reader accepts.
    ///
    /// ```text
    /// set IMAGEEDIT_INSPECT_PSD=<file.psd>
    /// cargo test -p imageedit-decode debug_lfx2 -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn debug_lfx2() {
        let Ok(path) = std::env::var("IMAGEEDIT_INSPECT_PSD") else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let (mut seen, mut ok, mut total_effects) = (0usize, 0usize, 0usize);
        let mut i = 0usize;
        while let Some(off) = find(&bytes[i..], b"lfx2") {
            let at = i + off;
            i = at + 4;
            let Some(len_bytes) = bytes.get(at + 4..at + 8) else {
                break;
            };
            let len = u32::from_be_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
            let Some(block) = bytes.get(at + 8..at + 8 + len) else {
                continue;
            };
            seen += 1;
            match super::parse_layer_effects_for_debug(block) {
                Some(effects) => {
                    ok += 1;
                    total_effects += effects.len();
                }
                None => {
                    if seen <= 3 {
                        println!("block at {at} len {len} head {:02x?}", &block[..block.len().min(24)]);
                    }
                }
            }
        }
        println!("lfx2 blocks {seen} parsed {ok} effects {total_effects}");
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }
}
