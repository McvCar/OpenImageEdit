//! Baking of Photoshop layer styles (`lfx2` / `lmfx` / `lfxs`) into a layer
//! bitmap.
//!
//! Photoshop CS6+ stores *several effects of the same kind* under `xxxMulti`
//! descriptor lists. The vendored parser flattens those into one entry per
//! stack (see `psd::effects::parse_layer_effects`), so this renderer only ever
//! sees one stroke / shadow / overlay per kind per layer.
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
//! drop shadow / outer glow           (behind the content)
//! layer content
//! inner shadow, inner glow           (inside the content)
//! colour / gradient overlay          (over the content)
//! stroke                             (over everything, overlays included)
//! ```
//!
//! The stroke really is last: an inside stroke on an icon that also carries a
//! colour overlay is the *stroke* you see in Photoshop, not the overlay colour.
//! Drawing it before the overlays (which is what this file used to do) leaves
//! the ring completely hidden under the overlay whenever the overlay is opaque —
//! the "red X" icons lost their dark red outline that way.
//!
//! Geometry follows Photoshop, not convenience, because a style baked *slightly*
//! wrong is worse than one baked coarsely — it moves hundreds of layers' pixels
//! in a consistent direction:
//!
//! * the shadow offset is `(-cos, +sin) × distance` (light at 90° comes from the
//!   top, so the shadow falls down), rounded to whole pixels;
//! * an inner shadow moves the *hole* along that same vector, so it lands on the
//!   lit side;
//! * a linear gradient spans the *chord* of the bounds along its angle, not the
//!   projection of the bounds;
//! * soft falloffs are two box passes of width `size` (a tent), which is what
//!   Photoshop's Size slider draws — a gaussian of `σ = size / 2` spreads the same
//!   mass much further out and dims the edge;
//! * spread / choke dilates by `size × spread` whole pixels and blurs with the
//!   remaining `size × (1 - spread)`, instead of brightening a band by hand;
//! * distances are a 5 × 5 chamfer (steps 1, √2, √5) seeded by the alpha, which
//!   is the metric Photoshop's stroke and glow edges follow.

use image::{Rgba, RgbaImage};
use psd::effects::{EffectKind, LayerEffect, StrokePosition};

#[cfg(test)]
mod debug;
mod gradient;
mod ops;
mod paint;
#[cfg(test)]
mod tests;

use ops::{blur_margin, dist_inside, dist_outside, distance_to};
use paint::{
    alpha_bounds, draw_bevel, draw_color_overlay, draw_drop_shadow, draw_gradient_overlay,
    draw_inner_glow, draw_inner_shadow, draw_outer_glow, draw_pattern_overlay, draw_stroke,
    offset_vector, trace_label, FX_BEHIND, FX_COVER, FX_FRONT, FX_IS_BEHIND,
};
pub use paint::{behind_blend, ring_coverage, set_trace_label};
pub(crate) use gradient::gradient_t;

/// Result of baking: a possibly larger bitmap plus where it now sits.
pub struct Baked {
    /// Pixels with the styles rendered in.
    pub image: RgbaImage,
    /// Canvas position of the new bitmap's top-left corner.
    pub offset: (i32, i32),
    /// Per-layer notes (unsupported style kinds, size caps).
    pub notes: Vec<String>,
    /// How much of each pixel the *styles* painted (0 = untouched content).
    ///
    /// Photoshop composites a layer's styles on top of its content, so an
    /// adjustment layer clipped to that layer changes the content but never the
    /// styles. Baking flattens both into one bitmap, which loses that
    /// distinction; this plane puts it back. It is what stops a hue shift
    /// clipped to a button from repainting the button's own outline.
    pub effect_coverage: Vec<u8>,
    /// 只记「背后」样式（投影 / 外发光）的覆盖率。
    ///
    /// 和 `effect_coverage` 的区别：内阴影 / 描边 / 叠加都在内容**之内**或
    /// 贴着内容，烘焙时背景就是图层内容，混合已经正确；只有这一份是画在
    /// 内容之外、需要合成阶段用样式自己的混合模式补的。
    pub behind_coverage: Vec<u8>,
}

/// Diagnostic ablation: `IMAGEEDIT_FX_SKIP=stroke,gradient` removes whole
/// effect kinds so a bad renderer can be isolated by measuring the composite.
fn skip_kind(kind: EffectKind) -> bool {
    static SKIP: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let list = SKIP.get_or_init(|| {
        std::env::var("IMAGEEDIT_FX_SKIP")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    });
    if list.is_empty() {
        return false;
    }
    let name = match kind {
        EffectKind::DropShadow => "dropshadow",
        EffectKind::InnerShadow => "innershadow",
        EffectKind::OuterGlow => "outerglow",
        EffectKind::InnerGlow => "innerglow",
        EffectKind::Stroke => "stroke",
        EffectKind::ColorOverlay => "color",
        EffectKind::GradientOverlay => "gradient",
        EffectKind::PatternOverlay => "pattern",
        EffectKind::BevelEmboss => "bevel",
    };
    list.iter().any(|s| s == name || s == "all")
}

/// Diagnostic ablation of the *geometry* rules, so each can be measured on its own
/// against the PSD's merged image: `IMAGEEDIT_FX_LEGACY=dir,grad,bounds,blur,dist`.
///
/// `dir` puts the shadows back on the old offset vector, `grad` spans the bounds'
/// projection instead of the chord, `bounds` lays gradients out over the ink
/// instead of the layer rect, `blur` uses the old gaussian, `dist` the old 3 × 3
/// chamfer. Off by default; a wrong value here is a bug, not an option.
fn legacy(axis: &str) -> bool {
    static LIST: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let list = LIST.get_or_init(|| {
        std::env::var("IMAGEEDIT_FX_LEGACY")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    });
    list.iter().any(|s| s == axis || s == "all")
}

/// Bake every enabled style into `image`.
///
/// Returns `None` when there is nothing to do, so callers can keep the
/// original allocation. `offset` is the canvas position of `image`'s top-left
/// corner; effects that paint outside the layer (shadow, outside stroke) grow
/// the bitmap and push the offset accordingly.
pub fn bake(image: &RgbaImage, offset: (i32, i32), effects: &[LayerEffect]) -> Option<Baked> {
    bake_with_alpha(image, offset, effects, None, None)
}

/// Like [`bake`], but the alpha that drives the styles can come from the shape's
/// vector path instead of the bitmap.
///
/// A shape layer can have `fillEnabled = false`: its pixels are empty, yet
/// Photoshop still rings it with a stroke and shades it with inner shadows. The
/// styles are computed from the path, not from the (absent) fill.
pub fn bake_with_alpha(
    image: &RgbaImage,
    offset: (i32, i32),
    effects: &[LayerEffect],
    alpha_override: Option<&[f32]>,
    patterns: Option<&crate::psd_pattern::PatternStore>,
) -> Option<Baked> {
    let active: Vec<&LayerEffect> = effects
        .iter()
        .filter(|e| e.enabled && !skip_kind(e.kind))
        .collect();
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
            EffectKind::DropShadow => {
                // The shadow is the shape moved by the offset and then blurred by
                // `size`, so its reach is both.
                let (dx, dy) = offset_vector(e.distance, e.angle);
                let reach = blur_margin(e.size);
                pad.left = pad.left.max(reach - dx.min(0));
                pad.right = pad.right.max(reach + dx.max(0));
                pad.top = pad.top.max(reach - dy.min(0));
                pad.bottom = pad.bottom.max(reach + dy.max(0));
            }
            EffectKind::OuterGlow => {
                let reach = blur_margin(e.size);
                pad.left = pad.left.max(reach);
                pad.right = pad.right.max(reach);
                pad.top = pad.top.max(reach);
                pad.bottom = pad.bottom.max(reach);
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
            // Inner effects read the *inverse* mask (the world outside the
            // shape), so they can only fall off correctly when that world is
            // part of the bitmap. A shape layer's bitmap ends exactly at its
            // bounding box, which left no transparent border to blur: the glow
            // and the shadow vanished along every straight edge and only
            // survived in the corners.
            EffectKind::InnerShadow => {
                let (dx, dy) = offset_vector(e.distance, e.angle);
                let reach = blur_margin(e.size) + dx.abs().max(dy.abs());
                pad.left = pad.left.max(reach);
                pad.right = pad.right.max(reach);
                pad.top = pad.top.max(reach);
                pad.bottom = pad.bottom.max(reach);
            }
            EffectKind::InnerGlow => {
                let reach = blur_margin(e.size);
                pad.left = pad.left.max(reach);
                pad.right = pad.right.max(reach);
                pad.top = pad.top.max(reach);
                pad.bottom = pad.bottom.max(reach);
            }
            // 斜面/浮雕按 alpha 的模糊梯度算光照；图层边缘外侧要有透明
            // 缓冲，否则直边上的坡度为 0，斜面会整条消失。
            EffectKind::BevelEmboss => {
                let reach = blur_margin(e.size + e.soften);
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
    // 从这里开始记「样式画上去的像素」。内容那一步直接写 `out`，不经过
    // `composite`，所以不会被记进来——正好是我们要的分界。
    FX_COVER.with(|c| *c.borrow_mut() = Some(vec![0.0f32; (w * h) as usize]));
    FX_BEHIND.with(|c| *c.borrow_mut() = Some(vec![0.0f32; (w * h) as usize]));
    FX_FRONT.with(|c| *c.borrow_mut() = Some(vec![0.0f32; (w * h) as usize]));

    // ---- layer alpha in the expanded coordinate system ------------------
    let mut alpha = vec![0.0f32; (w * h) as usize];
    for y in 0..ih {
        for x in 0..iw {
            let a = match alpha_override {
                Some(o) => o[(y * iw + x) as usize],
                None => image.get_pixel(x, y).0[3] as f32 / 255.0,
            };
            let (tx, ty) = ((x as i64 + ox) as u32, (y as i64 + oy) as u32);
            alpha[(ty * w + tx) as usize] = a;
        }
    }

    // ---- 1. behind the content: drop shadow, outer glow ------------------
    // 这一段单独打个标记：它们画在内容**之外**，合成时要用样式自己的混合
    // 模式与下层画面混合（见 `behind_blend`），不能和描边、叠加混为一谈。
    FX_IS_BEHIND.with(|f| f.set(true));
    for e in &active {
        match e.kind {
            EffectKind::DropShadow => draw_drop_shadow(&mut out, w, h, &alpha, e),
            EffectKind::OuterGlow => draw_outer_glow(&mut out, w, h, &alpha, e),
            _ => {}
        }
    }
    FX_IS_BEHIND.with(|f| f.set(false));
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

    // ---- 3. inside the content: inner shadow, inner glow -----------------
    // Overlays and the gradient stroke are laid out over the *layer's* bounds (a
    // style with `Algn` covers the layer rect, not the ink), so that is the frame
    // they run across.
    let bounds_layer = [ox as f32, oy as f32, (ox + iw as i64 - 1) as f32, (oy + ih as i64 - 1) as f32];
    let bounds = if legacy("bounds") {
        alpha_bounds(&alpha, w, h)
    } else {
        bounds_layer
    };
    // Distance fields, one per side, built only for what actually needs them.
    let strokes: Vec<&&LayerEffect> = active.iter().filter(|e| e.kind == EffectKind::Stroke).collect();
    let need_in = strokes.iter().any(|e| e.stroke_position != StrokePosition::Outside);
    let need_out = strokes.iter().any(|e| e.stroke_position != StrokePosition::Inside);
    let (dist_in, dist_out) = if legacy("dist") {
        let inside: Vec<bool> = alpha.iter().map(|a| *a >= 0.5).collect();
        let outside: Vec<bool> = inside.iter().map(|b| !b).collect();
        let din = if need_in { distance_to(&outside, w, h) } else { Vec::new() };
        let dout = if need_out { distance_to(&inside, w, h) } else { Vec::new() };
        (din, dout)
    } else {
        let din = if need_in { dist_inside(&alpha, w, h) } else { Vec::new() };
        let dout = if need_out { dist_outside(&alpha, w, h) } else { Vec::new() };
        (din, dout)
    };

    for e in &active {
        match e.kind {
            EffectKind::InnerShadow => {
                draw_inner_shadow(&mut out, w, h, &alpha, e, None);
            }
            EffectKind::InnerGlow => {
                draw_inner_glow(&mut out, w, h, &alpha, e, None);
            }
            _ => {}
        }
    }

    // ---- 4. overlays, on top of the content ------------------------------
    for e in &active {
        match e.kind {
            EffectKind::ColorOverlay => draw_color_overlay(&mut out, w, h, &alpha, e, None),
            EffectKind::GradientOverlay => {
                draw_gradient_overlay(&mut out, w, h, &alpha, bounds, e, None)
            }
            EffectKind::PatternOverlay => {
                if !draw_pattern_overlay(
                    &mut out,
                    w,
                    h,
                    &alpha,
                    offset,
                    ox,
                    oy,
                    e,
                    patterns,
                ) {
                    notes.push("图层样式「图案叠加」缺少图案像素".into());
                }
            }
            EffectKind::BevelEmboss => {
                draw_bevel(&mut out, w, h, &alpha, e);
            }
            _ => {}
        }
    }

    // ---- 5. the stroke, last --------------------------------------------
    // Photoshop stacks the stroke above the overlays: a 3 px inside stroke on an
    // icon that also has a colour overlay stays visible in the file, so it has to
    // stay visible here. Painting it in step 3 buried it under an opaque overlay.
    for e in &strokes {
        draw_stroke(&mut out, w, h, &alpha, &dist_in, &dist_out, bounds, e, None);
    }

    let new_offset = (offset.0 - pad.left, offset.1 - pad.top);
    if let Ok(v) = std::env::var("IMAGEEDIT_FX_TRACE") {
        if let Some((tx, ty)) = v.split_once(',').and_then(|(a, b)| {
            Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?))
        }) {
            if tx < w && ty < h {
                eprintln!(
                    "fx-result   [{}] ({tx},{ty}) = {:?}  offset {new_offset:?} pad {:?}",
                    trace_label(),
                    out.get_pixel(tx, ty).0,
                    (pad.left, pad.top),
                );
            }
        }
    }
    let effect_coverage: Vec<u8> = FX_COVER
        .with(|c| c.borrow_mut().take())
        .unwrap_or_default()
        .into_iter()
        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    let behind_raw = FX_BEHIND.with(|c| c.borrow_mut().take()).unwrap_or_default();
    let front = FX_FRONT.with(|c| c.borrow_mut().take()).unwrap_or_default();
    let behind_coverage: Vec<u8> = behind_raw
        .iter()
        .zip(front.iter())
        .map(|(b, f)| {
            // 被后续样式（描边 / 叠加 / 内阴影）盖过的像素不算：露出来的
            // 已经不是投影自己的颜色了。
            if *f > 0.0 {
                0
            } else {
                (b.clamp(0.0, 1.0) * 255.0).round() as u8
            }
        })
        .collect();
    Some(Baked {
        image: out,
        offset: new_offset,
        notes,
        effect_coverage,
        behind_coverage,
    })
}

#[derive(Default)]
struct Padding {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

/// Styles rendered for a **group** (the `lfx2` on a folder record), split into
/// the two rasters a group's effect stack produces:
///
/// * `behind` — drop shadow / outer glow, drawn *below* the group's content;
/// * `front` — inner shadow / inner glow / colour & gradient overlays / the
///   stroke ring, drawn *above* it (Photoshop's stacking order).
///
/// The group is a contiguous run of the layer stack, so inserting these as two
/// extra raster layers inside that run composites exactly like Photoshop's
/// group container would.
pub struct GroupBaked {
    pub behind: Option<RgbaImage>,
    pub front: Option<RgbaImage>,
    /// `behind` 那份光栅自己的混合模式（见 [`behind_blend`]）：组的投影要按
    /// 它和组**下方**的画面混合，不能当成 Normal 图层盖上去。
    pub behind_blend: editcore::BlendMode,
    /// Canvas position of both rasters' top-left corner.
    pub offset: (i32, i32),
    pub notes: Vec<String>,
}

/// Bake the styles of a **group** onto its members' joint alpha.
///
/// `alpha` is the group's composite coverage (every member blended over, which
/// for alpha is blend-mode independent), `size` its pixel size and `offset`
/// where it sits on the canvas. Only the style pixels are rendered — the
/// members themselves stay separate layers.
pub fn bake_group_effects(
    alpha: &[f32],
    size: (u32, u32),
    offset: (i32, i32),
    effects: &[LayerEffect],
) -> Option<GroupBaked> {
    let active: Vec<&LayerEffect> = effects
        .iter()
        .filter(|e| e.enabled && !skip_kind(e.kind))
        .collect();
    if active.is_empty() || alpha.len() as u64 != size.0 as u64 * size.1 as u64 {
        return None;
    }
    let (iw, ih) = size;
    if iw == 0 || ih == 0 {
        return None;
    }

    // ---- how far the styles reach outside the group's rectangle ----------
    let mut pad = Padding::default();
    for e in &active {
        match e.kind {
            EffectKind::DropShadow => {
                let (dx, dy) = offset_vector(e.distance, e.angle);
                let reach = blur_margin(e.size);
                pad.left = pad.left.max(reach - dx.min(0));
                pad.right = pad.right.max(reach + dx.max(0));
                pad.top = pad.top.max(reach - dy.min(0));
                pad.bottom = pad.bottom.max(reach + dy.max(0));
            }
            EffectKind::OuterGlow => {
                let reach = blur_margin(e.size);
                pad.left = pad.left.max(reach);
                pad.right = pad.right.max(reach);
                pad.top = pad.top.max(reach);
                pad.bottom = pad.bottom.max(reach);
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
            // Inner effects read the *inverse* mask (the world outside the
            // shape), so they can only fall off correctly when that world is
            // part of the bitmap. A shape layer's bitmap ends exactly at its
            // bounding box, which left no transparent border to blur: the glow
            // and the shadow vanished along every straight edge and only
            // survived in the corners.
            EffectKind::InnerShadow => {
                let (dx, dy) = offset_vector(e.distance, e.angle);
                let reach = blur_margin(e.size) + dx.abs().max(dy.abs());
                pad.left = pad.left.max(reach);
                pad.right = pad.right.max(reach);
                pad.top = pad.top.max(reach);
                pad.bottom = pad.bottom.max(reach);
            }
            EffectKind::InnerGlow => {
                let reach = blur_margin(e.size);
                pad.left = pad.left.max(reach);
                pad.right = pad.right.max(reach);
                pad.top = pad.top.max(reach);
                pad.bottom = pad.bottom.max(reach);
            }
            _ => {}
        }
    }
    // A group spans the whole panel; an unbounded margin would explode the
    // bitmap long before the per-layer cap does. 64 px of ring is far beyond
    // any style this file format carries.
    let cap = 64;
    pad.left = pad.left.min(cap);
    pad.right = pad.right.min(cap);
    pad.top = pad.top.min(cap);
    pad.bottom = pad.bottom.min(cap);

    let (w, h) = (
        (iw as i64 + pad.left as i64 + pad.right as i64).max(1) as u32,
        (ih as i64 + pad.top as i64 + pad.bottom as i64).max(1) as u32,
    );
    if w as u64 * h as u64 > 30_000_000 {
        return None;
    }

    // The joint alpha, expanded into the padded coordinate system.
    let mut big = vec![0.0f32; (w * h) as usize];
    for y in 0..ih {
        for x in 0..iw {
            let (tx, ty) = ((x as i64 + pad.left as i64) as u32, (y as i64 + pad.top as i64) as u32);
            big[(ty * w + tx) as usize] = alpha[(y * iw + x) as usize];
        }
    }

    let mut notes = Vec::new();
    let empty = || RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));

    // ---- behind: drop shadow / outer glow --------------------------------
    let mut behind = empty();
    for e in &active {
        match e.kind {
            EffectKind::DropShadow => draw_drop_shadow(&mut behind, w, h, &big, e),
            EffectKind::OuterGlow => draw_outer_glow(&mut behind, w, h, &big, e),
            _ => {}
        }
    }

    // ---- front: inner effects, overlays, then the stroke ------------------
    // Photoshop's stack for a group is the same as for a layer: inner shadow /
    // inner glow sit on the content, overlays above them, the stroke on top.
    // Everything is painted source-over at `coverage × group alpha` because a
    // fresh front raster has no dst alpha to "replace" (the same reason the
    // group stroke needed this treatment).
    let mut front = empty();
    for e in &active {
        match e.kind {
            EffectKind::InnerShadow => draw_inner_shadow(&mut front, w, h, &big, e, Some(&big)),
            EffectKind::InnerGlow => draw_inner_glow(&mut front, w, h, &big, e, Some(&big)),
            _ => {}
        }
    }
    // Gradient overlays run across the group's ink, like the group stroke.
    let bounds = alpha_bounds(&big, w, h);
    for e in &active {
        match e.kind {
            EffectKind::ColorOverlay => draw_color_overlay(&mut front, w, h, &big, e, Some(&big)),
            EffectKind::GradientOverlay => {
                draw_gradient_overlay(&mut front, w, h, &big, bounds, e, Some(&big))
            }
            _ => {}
        }
    }
    let strokes: Vec<&&LayerEffect> = active.iter().filter(|e| e.kind == EffectKind::Stroke).collect();
    if !strokes.is_empty() {
        let need_in = strokes.iter().any(|e| e.stroke_position != StrokePosition::Outside);
        let need_out = strokes.iter().any(|e| e.stroke_position != StrokePosition::Inside);
        let din = if need_in { dist_inside(&big, w, h) } else { Vec::new() };
        let dout = if need_out { dist_outside(&big, w, h) } else { Vec::new() };
        // A group stroke's gradient (if any) runs across the group's ink, not
        // its bounding rectangle of member layers.
        for e in &strokes {
            draw_stroke(&mut front, w, h, &big, &din, &dout, bounds, e, Some(&big));
        }
    }
    for e in &active {
        match e.kind {
            EffectKind::PatternOverlay | EffectKind::BevelEmboss => {
                notes.push("组样式「图案叠加/斜面和浮雕」暂不支持（组级）".into());
            }
            _ => {}
        }
    }

    let new_offset = (offset.0 - pad.left, offset.1 - pad.top);
    // A fully transparent raster carries nothing: skip it so the caller does
    // not insert an invisible layer.
    let has_px = |img: &RgbaImage| img.pixels().any(|p| p.0[3] > 0);
    let behind = if has_px(&behind) { Some(behind) } else { None };
    let front = if has_px(&front) { Some(front) } else { None };
    if behind.is_none() && front.is_none() {
        return None;
    }
    Some(GroupBaked {
        behind,
        front,
        behind_blend: behind_blend(active.iter().copied()).unwrap_or(editcore::BlendMode::Normal),
        offset: new_offset,
        notes,
    })
}
