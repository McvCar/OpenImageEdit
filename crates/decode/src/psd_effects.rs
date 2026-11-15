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

/// How far a tent of `size` reaches before it is visually zero: its half-width is
/// `size` (two box passes of that width), plus a pixel of slack.
fn blur_margin(size: f32) -> i32 {
    size.ceil() as i32 + 2
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
    static FX_COVER: std::cell::RefCell<Option<Vec<f32>>> = const { std::cell::RefCell::new(None) };
    /// 同一时刻只记「背后」样式（投影 / 外发光）的那一份覆盖率。
    static FX_BEHIND: std::cell::RefCell<Option<Vec<f32>>> = const { std::cell::RefCell::new(None) };
    /// 记「非背后」样式（内阴影、叠加、描边、斜面）。这些画在投影之上，
    /// 一个像素只要被它们碰过，露出来的颜色就不是投影自己的了。
    static FX_FRONT: std::cell::RefCell<Option<Vec<f32>>> = const { std::cell::RefCell::new(None) };
    /// 当前是否在画「背后」样式。
    static FX_IS_BEHIND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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

fn trace_label() -> String {
    TRACE_LABEL
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

fn draw_drop_shadow(out: &mut RgbaImage, w: u32, h: u32, alpha: &[f32], e: &LayerEffect) {
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

fn draw_outer_glow(out: &mut RgbaImage, w: u32, h: u32, alpha: &[f32], e: &LayerEffect) {
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

fn draw_stroke(
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

fn draw_inner_shadow(
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

fn draw_inner_glow(
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

fn draw_color_overlay(
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

fn draw_gradient_overlay(
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
fn draw_pattern_overlay(
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
fn draw_bevel(out: &mut RgbaImage, w: u32, h: u32, alpha: &[f32], e: &LayerEffect) {
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
fn offset_vector(distance: f32, angle_deg: f32) -> (i32, i32) {
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

/// Position of `(x, y)` along the ramp, 0..1.
///
/// A Linear or Reflected gradient spans the **chord** of the bounds along its
/// angle (`min(w / |cos|, h / |sin|)`), not the projection of the bounds: on a
/// wide, flat layer at 45° the two differ by several times, and Photoshop uses
/// the chord.
pub(crate) fn gradient_t(x: f32, y: f32, bounds: [f32; 4], g: &Gradient) -> f32 {
    if legacy("grad") {
        return gradient_t_projected(x, y, bounds, g);
    }
    let w = (bounds[2] - bounds[0] + 1.0).max(1.0);
    let h = (bounds[3] - bounds[1] + 1.0).max(1.0);
    // Pixel centres: (x0 + x1 + 1) / 2.
    let cx = bounds[0] + w * 0.5;
    let cy = bounds[1] + h * 0.5;
    let scale = (g.scale / 100.0).clamp(0.01, 100.0);
    let rad = g.angle.to_radians();
    let (s, c) = rad.sin_cos();
    let dx = x - cx;
    let dy = y - cy;
    // Distance along the gradient direction (image rows grow downwards).
    let along = dx * c - dy * s;
    let across = dx * s + dy * c;
    let chord = ((w / c.abs().max(1e-6)).min(h / s.abs().max(1e-6)).max(1.0)) * scale;
    let len = ((c * w).powi(2) + (s * h).powi(2)).sqrt().max(1.0) * scale;
    // Linear samples the pixel's top-left corner, half a pixel before its centre.
    let corner = 0.5 * (c - s);
    let t = match g.kind {
        GradientKind::Linear => (along - corner) / chord + 0.5,
        GradientKind::Reflected => ((along - corner) / (chord * 0.5)).abs(),
        GradientKind::Radial => (dx * dx + dy * dy).sqrt() / (len * 0.5),
        GradientKind::Angle => {
            // Clockwise sweep starting at the gradient angle.
            (rad - (-dy).atan2(dx)) / std::f32::consts::TAU
        }
        GradientKind::Diamond => (along.abs() + across.abs()) / (len * 0.5),
    };
    t.clamp(0.0, 1.0)
}

/// The pre-2026-11 geometry: the ramp spans the bounds' projection onto the angle.
fn gradient_t_projected(x: f32, y: f32, bbox: [f32; 4], g: &Gradient) -> f32 {
    let cx = (bbox[0] + bbox[2]) * 0.5;
    let cy = (bbox[1] + bbox[3]) * 0.5;
    let hw = ((bbox[2] - bbox[0]) * 0.5).max(0.5);
    let hh = ((bbox[3] - bbox[1]) * 0.5).max(0.5);
    let scale = (g.scale / 100.0).clamp(0.01, 100.0);
    let t = match g.kind {
        GradientKind::Linear | GradientKind::Reflected => {
            let rad = g.angle.to_radians();
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
///
/// `outside` fills what the shift pulls in from beyond the buffer: a shadow moves
/// a transparent shape (0), an inner shadow moves the *hole* (1).
fn shift_mask(mask: &[f32], w: u32, h: u32, dx: i32, dy: i32, outside: f32) -> Vec<f32> {
    let mut out = vec![outside; mask.len()];
    let (wi, hi) = (w as i32, h as i32);
    for y in 0..hi {
        let sy = y - dy;
        if sy < 0 || sy >= hi {
            continue;
        }
        for x in 0..wi {
            let sx = x - dx;
            if sx < 0 || sx >= wi {
                continue;
            }
            out[(y * wi + x) as usize] = mask[(sy * wi + sx) as usize];
        }
    }
    out
}

/// Photoshop's Size/Spread split: the spread grows the outline by `size × spread`
/// whole pixels, the blur keeps the remaining `size × (1 - spread)`.
fn spread_split(size: f32, spread_pct: f32) -> (f32, f32) {
    let spread = (spread_pct / 100.0).clamp(0.0, 1.0);
    ((size * spread).round(), size * (1.0 - spread))
}

/// Grows a soft map by `r` px (anti-aliased): a partly covered pixel grows from its
/// own coverage, so a small spread does not turn every edge pixel opaque.
fn dilate(map: &[f32], w: u32, h: u32, r: f32) -> Vec<f32> {
    if !(r > 0.0) {
        return map.to_vec();
    }
    let dist = dist_outside(map, w, h);
    map.iter()
        .zip(&dist)
        .map(|(&v, &d)| {
            if d < 0.0 {
                (v + r).min(1.0)
            } else {
                v.max((r + 0.5 - d).clamp(0.0, 1.0))
            }
        })
        .collect()
}

/// A style's soft falloff: two box passes of width `size` along each axis (a tent),
/// which is what Photoshop's Size slider draws.
///
/// Replaces a three-pass gaussian of `σ = size / 2`: that spreads the same mass
/// over a much longer tail and leaves the edge dimmer than Photoshop's.
fn tent_blur(map: &[f32], w: u32, h: u32, size: f32) -> Vec<f32> {
    if legacy("blur") {
        return gaussian_blur(map, w, h, size);
    }
    if !(size > 0.05) || w < 2 || h < 2 {
        return map.to_vec();
    }
    let (radius, taps) = box_taps(size);
    if radius == 0 {
        return map.to_vec();
    }
    let (wu, hu) = (w as usize, h as usize);
    let mut cur = box_pass(map, wu, hu, radius, &taps, true);
    cur = box_pass(&cur, wu, hu, radius, &taps, true);
    cur = box_pass(&cur, wu, hu, radius, &taps, false);
    box_pass(&cur, wu, hu, radius, &taps, false)
}

/// The taps of a centred box of (fractional) width `w`: `radius` whole taps each
/// side of the centre, and the two end taps at `f`.
fn box_taps(w: f32) -> (usize, Vec<f32>) {
    let w = if w.is_finite() { w.clamp(1.0, 512.0) } else { 1.0 };
    let half = w / 2.0;
    let radius = ((half - 0.5).ceil()).max(0.0) as usize;
    let f = (half - (radius as f32 - 0.5)).clamp(0.0, 1.0);
    let norm = 1.0 / w;
    let mut taps = vec![norm; radius * 2 + 1];
    if radius > 0 {
        taps[0] = f * norm;
        taps[radius * 2] = f * norm;
    }
    (radius, taps)
}

fn box_pass(src: &[f32], w: usize, h: usize, radius: usize, taps: &[f32], horizontal: bool) -> Vec<f32> {
    let mut out = vec![0.0f32; src.len()];
    let r = radius as i64;
    if horizontal {
        for y in 0..h {
            let row = y * w;
            for x in 0..w {
                let mut acc = 0.0;
                for (k, t) in taps.iter().enumerate() {
                    let xx = x as i64 + k as i64 - r;
                    if xx >= 0 && xx < w as i64 {
                        acc += src[row + xx as usize] * t;
                    }
                }
                out[row + x] = acc;
            }
        }
    } else {
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for (k, t) in taps.iter().enumerate() {
                    let yy = y as i64 + k as i64 - r;
                    if yy >= 0 && yy < h as i64 {
                        acc += src[yy as usize * w + x] * t;
                    }
                }
                out[y * w + x] = acc;
            }
        }
    }
    out
}

/// Alpha below this reads as empty: style maps only paint where the layer has ink.
const INSIDE_EPS: f32 = 0.004;

const CHAMFER_INF: f32 = 1.0e20;

/// Forward-pass neighbours of a 5 × 5 chamfer (the backward pass mirrors them).
///
/// Steps are exact Euclidean lengths (1, √2, √5). Photoshop's effect distances
/// follow this metric — its stroke corners measure 3.236 at offset (1, 3) and 3.650
/// at (2, 3), where a 3 × 3 chamfer would say 4 and 4.24.
const CHAMFER_FWD: [(i64, i64, f32); 8] = [
    (-1, 0, 1.0),
    (-1, -1, std::f32::consts::SQRT_2),
    (0, -1, 1.0),
    (1, -1, std::f32::consts::SQRT_2),
    (-1, -2, 2.236_068),
    (1, -2, 2.236_068),
    (-2, -1, 2.236_068),
    (2, -1, 2.236_068),
];

fn chamfer_from(starts: &[f32], w: u32, h: u32) -> Vec<f32> {
    let mut d = starts.to_vec();
    let (wi, hi) = (w as i64, h as i64);
    for y in 0..hi {
        for x in 0..wi {
            let i = (y * wi + x) as usize;
            let mut best = d[i];
            for &(ox, oy, wt) in &CHAMFER_FWD {
                let (nx, ny) = (x + ox, y + oy);
                if nx < 0 || ny < 0 || nx >= wi || ny >= hi {
                    continue;
                }
                let c = d[(ny * wi + nx) as usize] + wt;
                if c < best {
                    best = c;
                }
            }
            d[i] = best;
        }
    }
    for y in (0..hi).rev() {
        for x in (0..wi).rev() {
            let i = (y * wi + x) as usize;
            let mut best = d[i];
            for &(ox, oy, wt) in &CHAMFER_FWD {
                let (nx, ny) = (x - ox, y - oy);
                if nx < 0 || ny < 0 || nx >= wi || ny >= hi {
                    continue;
                }
                let c = d[(ny * wi + nx) as usize] + wt;
                if c < best {
                    best = c;
                }
            }
            d[i] = best;
        }
    }
    d
}

/// Distance from each pixel centre to the shape's edge: positive outside, −0.5
/// inside.
///
/// The edge is seeded from the alpha, so a 25 %-covered border pixel puts the
/// contour a quarter pixel in from its boundary instead of snapping to the pixel
/// grid: that is what makes a 1 px stroke follow a curve smoothly.
fn dist_outside(alpha: &[f32], w: u32, h: u32) -> Vec<f32> {
    let starts: Vec<f32> = alpha
        .iter()
        .map(|&a| {
            if a > INSIDE_EPS {
                1.0 - a.min(1.0)
            } else {
                CHAMFER_INF
            }
        })
        .collect();
    let d = chamfer_from(&starts, w, h);
    d.iter()
        .zip(alpha)
        .map(|(d, &a)| if a > INSIDE_EPS { -0.5 } else { d - 0.5 })
        .collect()
}

/// Distance from each pixel centre to the shape's edge: positive inside, −0.5
/// outside.
fn dist_inside(alpha: &[f32], w: u32, h: u32) -> Vec<f32> {
    let (wi, hi) = (w as i64, h as i64);
    let mut starts = vec![CHAMFER_INF; alpha.len()];
    for y in 0..hi {
        for x in 0..wi {
            let i = (y * wi + x) as usize;
            let a = alpha[i];
            if a <= INSIDE_EPS {
                starts[i] = 0.0;
                continue;
            }
            // A pixel dimmer than its neighbourhood sits on the edge, at its own
            // coverage from the contour.
            let mut mx = a;
            for dy in -1..=1i64 {
                for dx in -1..=1i64 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= wi || ny >= hi {
                        continue;
                    }
                    mx = mx.max(alpha[(ny * wi + nx) as usize]);
                }
            }
            let coverage = (a / mx).min(1.0);
            starts[i] = if coverage < 1.0 - INSIDE_EPS {
                coverage
            } else {
                CHAMFER_INF
            };
        }
    }
    let d = chamfer_from(&starts, w, h);
    d.iter()
        .zip(alpha)
        .map(|(d, &a)| if a <= INSIDE_EPS { -0.5 } else { d - 0.5 })
        .collect()
}

/// The pre-2026-11 falloff: three box passes ≈ a gaussian of `σ = size / 2`.
fn gaussian_blur(mask: &[f32], w: u32, h: u32, size: f32) -> Vec<f32> {
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

/// The pre-2026-11 distance: 3 × 3 chamfer over a boolean mask.
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
            pattern: None,
            pattern_phase: (0.0, 0.0),
            highlight_color: [255, 255, 255],
            highlight_opacity: 0.75,
            highlight_blend: psd::BlendMode::Screen,
            shadow_color: [0, 0, 0],
            shadow_opacity: 0.75,
            shadow_blend: psd::BlendMode::Multiply,
            bevel_style: psd::BevelStyle::Inner,
            bevel_direction: psd::BevelDirection::Up,
            altitude: 30.0,
            depth: 100.0,
            soften: 0.0,
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

    /// 颜色叠加是「换色」：半透明内容 + 白色叠加后 RGB 必须**是白**，alpha
    /// 保持内容自己的值。
    ///
    /// source-over 会把颜色按 alpha 预乘（白 100% 叠到 40% 内容上成 65% 灰），
    /// 还把 alpha 叠成 `a + a(1-a)`。PSD 里半透明白斜纹层（「增加时间」横幅的
    /// 图层 84）就是这样变成灰纱罩的。
    #[test]
    fn color_overlay_does_not_premultiply_semi_transparent_content() {
        let img = layer(8, 8, [12, 34, 56, 102], 2);
        let mut e = effect(EffectKind::ColorOverlay);
        e.color = [255, 255, 255];
        let baked = bake(&img, (0, 0), &[e]).expect("color overlay bakes");
        let p = baked.image.get_pixel(4, 4).0;
        assert_eq!([p[0], p[1], p[2]], [255, 255, 255], "叠加色原样写进 RGB");
        assert_eq!(p[3], 102, "alpha 由内容决定，叠加不往上加");
        // 没有内容的像素仍然什么都没有。
        assert_eq!(baked.image.get_pixel(0, 0).0[3], 0);
    }

    /// A group's stroke is one ring around the group's joint alpha, on a
    /// raster of its own — the members are not touched.
    #[test]
    fn group_stroke_rings_the_joint_alpha() {
        // Two squares apart: the ring must follow each outline, and nothing
        // is painted between them.
        let (w, h) = (20u32, 10u32);
        let mut alpha = vec![0.0f32; (w * h) as usize];
        for y in 2..5 {
            for x in 2..5 {
                alpha[(y * w + x) as usize] = 1.0;
            }
            for x in 10..13 {
                alpha[(y * w + x) as usize] = 1.0;
            }
        }
        let mut e = effect(EffectKind::Stroke);
        e.size = 1.0;
        e.color = [187, 123, 15];
        let baked = bake_group_effects(&alpha, (w, h), (5, 7), &[e]).expect("baked");
        let front = baked.front.expect("stroke raster");
        assert!(baked.behind.is_none(), "no shadow, no behind raster");
        // Padded by size+2 on each side; the offset moves accordingly.
        assert_eq!(baked.offset, (2, 4));
        // One pixel outside each square's edge carries the ring (the edge
        // pixel itself is half-covered by the band — same semantics the
        // per-layer stroke uses; on a transparent background the premultiplied
        // colour halves too). The rasters sit at canvas - offset, i.e. alpha
        // coords shifted by the pad (+3).
        let ring = front.get_pixel(5 + 3, 3 + 3).0;
        assert!(ring[3] > 0 && ring[0] > 0, "ring right of square 1");
        assert!(front.get_pixel(9 + 3, 3 + 3).0[3] > 0, "ring left of square 2");
        // Between the squares there is no ring.
        assert_eq!(front.get_pixel(8 + 3, 3 + 3).0[3], 0);
        // Inside a square the ring does not reach (outside stroke).
        assert_eq!(front.get_pixel(3 + 3, 3 + 3).0[3], 0);
    }

    /// A group's drop shadow lands below the joint alpha, offset by the light.
    #[test]
    fn group_shadow_is_one_blur_behind_every_member() {
        let (w, h) = (12u32, 12u32);
        let mut alpha = vec![0.0f32; (w * h) as usize];
        for y in 3..6 {
            for x in 3..6 {
                alpha[(y * w + x) as usize] = 1.0;
            }
        }
        let mut e = effect(EffectKind::DropShadow);
        e.color = [115, 115, 115];
        e.distance = 2.0;
        e.angle = 90.0;
        e.size = 0.0;
        let baked = bake_group_effects(&alpha, (w, h), (0, 0), &[e]).expect("baked");
        let behind = baked.behind.expect("shadow raster");
        assert!(baked.front.is_none());
        // Light from the top: the shadow falls straight down, so the pixel
        // just under the square is full shadow colour.
        let px = behind.get_pixel(4 + 3, 6 + 3).0;
        assert_eq!(px, [115, 115, 115, 255], "shadow below the shape");
        // Above the shape there is nothing.
        assert_eq!(behind.get_pixel(4 + 3, 1 + 3).0[3], 0);
    }

    /// 组级内阴影 / 内发光 / 颜色叠加 / 渐变叠加画在同一张前景光栅上。
    /// 回归点：前景光栅从全透明开始，`composite_inside` 遇到 dst alpha = 0
    /// 会把内阴影/内发光整块吃掉（组描边曾经就是这么消失的）。
    #[test]
    fn group_inner_effects_and_overlays_land_on_the_front_raster() {
        let (w, h) = (12u32, 12u32);
        let mut alpha = vec![0.0f32; (w * h) as usize];
        for y in 3..9 {
            for x in 3..9 {
                alpha[(y * w + x) as usize] = 1.0;
            }
        }

        let mut overlay = effect(EffectKind::ColorOverlay);
        overlay.color = [200, 10, 10];
        let baked = bake_group_effects(&alpha, (w, h), (0, 0), &[overlay]).expect("overlay");
        let front = baked.front.as_ref().expect("overlay raster");
        assert_eq!(front.get_pixel(4, 4).0, [200, 10, 10, 255]);
        assert_eq!(front.get_pixel(0, 0).0[3], 0, "组外不应有叠加色");

        let mut shadow = effect(EffectKind::InnerShadow);
        shadow.color = [0, 0, 0];
        shadow.distance = 2.0;
        shadow.angle = 90.0;
        let baked = bake_group_effects(&alpha, (w, h), (0, 0), &[shadow]).expect("inner shadow");
        let front = baked.front.as_ref().expect("inner shadow raster");
        assert!(
            front.pixels().any(|p| p.0[3] > 0),
            "内阴影不能整块消失"
        );

        let mut glow = effect(EffectKind::InnerGlow);
        glow.color = [255, 255, 255];
        glow.size = 2.0;
        let baked = bake_group_effects(&alpha, (w, h), (0, 0), &[glow]).expect("inner glow");
        let front = baked.front.as_ref().expect("inner glow raster");
        assert!(front.pixels().any(|p| p.0[3] > 0), "内发光不能整块消失");

        let mut grad = effect(EffectKind::GradientOverlay);
        grad.gradient = Some(Gradient {
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
            angle: 0.0,
            reverse: false,
            align: true,
            scale: 100.0,
        });
        let baked = bake_group_effects(&alpha, (w, h), (0, 0), &[grad]).expect("gradient");
        let front = baked.front.as_ref().expect("gradient raster");
        // 角度 0：渐变沿 x 从黑到白。
        let left = front.get_pixel(3, 5).0[0] as i32;
        let right = front.get_pixel(8, 5).0[0] as i32;
        assert!(right > left + 100, "left {left} right {right}");
        assert_eq!(front.get_pixel(0, 0).0[3], 0, "组外不应有渐变");
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

    /// A clipped adjustment layer must leave a layer's *styles* alone, so `bake`
    /// has to say which pixels the styles painted. An inside stroke covers the
    /// border of the content; the middle of the square stays untouched content.
    #[test]
    fn coverage_marks_the_pixels_the_styles_painted() {
        // 16×16 的透明画布，中心 8×8 是内容；内描边 2px 只吃外圈两像素。
        let img = layer(16, 16, [0, 0, 0, 255], 4);
        let mut e = effect(EffectKind::Stroke);
        e.size = 2.0;
        e.color = [255, 0, 0];
        e.stroke_position = StrokePosition::Inside;
        let baked = bake(&img, (0, 0), &[e]).expect("baked");
        assert_eq!(baked.image.dimensions(), (16, 16));
        assert_eq!(baked.effect_coverage.len(), 16 * 16);
        let cov = |x: u32, y: u32| baked.effect_coverage[(y * 16 + x) as usize];
        // 外圈两像素是样式画的，四角也一样。
        assert_eq!(cov(4, 4), 255, "stroke should own the corner of the ring");
        assert_eq!(cov(4, 7), 255, "and the edge between the corners");
        assert_eq!(cov(11, 11), 255);
        // 中间仍是内容本身，没被样式碰过。
        assert_eq!(cov(7, 7), 0, "the interior is content, not style");
        // 形状之外什么都没画。
        assert_eq!(cov(0, 0), 0);
        // 像素本身：外圈被描边覆盖，中心保留原色。
        assert_eq!(baked.image.get_pixel(4, 4).0, [255, 0, 0, 255]);
        assert_eq!(baked.image.get_pixel(7, 7).0, [0, 0, 0, 255]);
    }

    #[test]
    fn drop_shadow_paints_below_the_content() {        let img = layer(6, 6, [255, 255, 255, 255], 1);
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
    fn distance_fields_are_signed_and_start_at_the_edge() {
        let (w, h) = (5u32, 5u32);
        let mut alpha = vec![0.0f32; 25];
        alpha[12] = 1.0; // centre
        let out = dist_outside(&alpha, w, h);
        assert_eq!(out[12], -0.5, "inside is −0.5");
        assert!((out[13] - 0.5).abs() < 1e-3, "one pixel out is 0.5 px from the edge");
        assert!((out[8] - 0.914).abs() < 0.01, "diagonal uses √2, not a chamfer of 1.5");

        let din = dist_inside(&alpha, w, h);
        assert!(din[13] < 0.0, "outside is −0.5");
        assert!((din[12] - 0.5).abs() < 1e-3, "a 1 px blob is half a pixel from its edge");
    }

    #[test]
    fn a_five_by_five_chamfer_keeps_the_diagonal_exact() {
        let (w, h) = (9u32, 9u32);
        let mut alpha = vec![0.0f32; 81];
        alpha[4 * 9 + 4] = 1.0; // (4, 4)
        let d = dist_outside(&alpha, w, h);
        // (1, 2) is a √5 step, (2, 3) is √5 + √2: Photoshop measures 3.236 and 3.650,
        // where a 3 × 3 chamfer would say 4 and 4.24.
        let s5 = 2.236_068;
        assert!((d[6 * 9 + 5] - (s5 - 0.5)).abs() < 1e-3, "{}", d[6 * 9 + 5]);
        assert!(
            (d[7 * 9 + 6] - (s5 + std::f32::consts::SQRT_2 - 0.5)).abs() < 1e-3,
            "{}",
            d[7 * 9 + 6]
        );
    }

    #[test]
    fn tent_blur_keeps_the_mass_and_spreads_by_size() {
        let (w, h) = (17u32, 17u32);
        let mut src = vec![0.0f32; (w * h) as usize];
        src[(8 * w + 8) as usize] = 1.0;
        let blurred = tent_blur(&src, w, h, 4.0);
        let sum: f32 = blurred.iter().sum();
        assert!((sum - 1.0).abs() < 1e-2, "sum {sum}");
        // A tent of width 4 reaches four pixels out and is zero beyond.
        assert!(blurred[(8 * w + 12) as usize] > 1e-3, "reaches 4 px");
        assert!(blurred[(8 * w + 14) as usize] < 1e-6, "and stops there");
        // Monotonic falloff: a tent, not a gaussian's long tail.
        let at = |dx: u32| blurred[(8 * w + 8 + dx) as usize];
        assert!(at(0) > at(1) && at(1) > at(2) && at(2) > at(3), "{} {} {} {}", at(0), at(1), at(2), at(3));
    }

    #[test]
    fn spread_grows_the_shape_before_it_blurs() {
        // Size 4 at 50 % spread: two whole pixels of dilation, two of blur.
        let (w, h) = (21u32, 21u32);
        let mut alpha = vec![0.0f32; (w * h) as usize];
        for y in 8..13 {
            for x in 8..13 {
                alpha[(y * w + x) as usize] = 1.0;
            }
        }
        let (grow, soft) = spread_split(4.0, 50.0);
        assert_eq!((grow, soft), (2.0, 2.0));
        let grown = dilate(&alpha, w, h, grow);
        assert!(grown[(10 * w + 7) as usize] > 0.99, "grown 1 px out, solid");
        assert!(grown[(10 * w + 6) as usize] > 0.99, "2 px out, still solid");
        assert_eq!(grown[(10 * w + 5) as usize], 0.0, "3 px out is beyond the spread");
        // No spread: the outline is untouched.
        let same = dilate(&alpha, w, h, 0.0);
        assert_eq!(same[(10 * w + 7) as usize], 0.0);
    }

    #[test]
    fn shadow_offset_follows_photoshop_light_angles() {
        // 90° is light from the top: the shadow falls straight down.
        assert_eq!(offset_vector(5.0, 90.0), (0, 5));
        // 120° is light from the upper left: the default shadow lands bottom-right.
        let (dx, dy) = offset_vector(5.0, 120.0);
        assert!(dx > 0 && dy > 0, "({dx}, {dy})");
        // 0° is light from the right: the shadow goes left.
        assert_eq!(offset_vector(4.0, 0.0), (-4, 0));
    }

    #[test]
    fn inner_shadow_lands_on_the_lit_side() {
        // Light from the top (90°): the shadow is the band along the top inner edge.
        let img = layer(20, 20, [255, 255, 255, 255], 0);
        let mut e = effect(EffectKind::InnerShadow);
        e.size = 0.0;
        e.distance = 3.0;
        e.color = [0, 0, 0];
        e.opacity = 1.0;
        let baked = bake(&img, (0, 0), &[e]).expect("baked");
        let top = baked.image.get_pixel(10, 1).0[0];
        let bottom = baked.image.get_pixel(10, 18).0[0];
        assert!(top < 60, "top row should carry the shadow, got {top}");
        assert!(bottom > 200, "bottom row should be clear, got {bottom}");
    }

    #[test]
    fn gradient_spans_the_chord_not_the_projection() {
        // A 60 × 12 layer at 45°: the ramp runs 12 / sin45 ≈ 17 px along the chord,
        // not the 51 px the bounds project to, so it saturates well before the edge.
        let img = layer(60, 12, [0, 0, 0, 255], 0);
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
            angle: 45.0,
            reverse: false,
            align: true,
            scale: 100.0,
        });
        let baked = bake(&img, (0, 0), &[e]).expect("baked");
        // The ramp runs bottom-left (black) to top-right (white).
        let bright = baked.image.get_pixel(45, 0).0[0];
        let dark = baked.image.get_pixel(5, 11).0[0];
        assert!(bright > 240, "top right is already white: {bright}");
        assert!(dark < 15, "bottom left stays black: {dark}");
    }

    /// 图案叠加按图案像素平铺，而不是退回纯色。
    #[test]
    fn pattern_overlay_tiles_the_pattern() {
        let img = layer(8, 8, [128, 128, 128, 255], 0);
        let mut e = effect(EffectKind::PatternOverlay);
        e.pattern = Some(psd::VectorPattern {
            id: "p1".into(),
            name: "P".into(),
            scale: 100.0,
            angle: 0.0,
            align: true,
        });
        e.opacity = 1.0;
        let store = crate::psd_pattern::PatternStore {
            tiles: vec![crate::psd_pattern::PatternTile {
                id: "p1".into(),
                name: "P".into(),
                image: RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap(),
            }],
        };
        let baked = crate::psd_effects::bake_with_alpha(&img, (0, 0), &[e], None, Some(&store))
            .expect("baked");
        assert_eq!(baked.image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(baked.image.get_pixel(1, 0).0, [0, 255, 0, 255]);
        assert_eq!(baked.image.get_pixel(2, 0).0, [255, 0, 0, 255]);
    }

    /// 没有图案像素时保持原样，并记一条降级说明。
    #[test]
    fn pattern_overlay_without_pixels_is_a_no_op() {
        let img = layer(8, 8, [128, 128, 128, 255], 0);
        let mut e = effect(EffectKind::PatternOverlay);
        e.pattern = Some(psd::VectorPattern {
            id: "missing".into(),
            name: "missing".into(),
            scale: 100.0,
            angle: 0.0,
            align: true,
        });
        let baked = crate::psd_effects::bake_with_alpha(
            &img,
            (0, 0),
            &[e],
            None,
            Some(&crate::psd_pattern::PatternStore::default()),
        )
        .expect("baked");
        assert_eq!(baked.image.get_pixel(4, 4).0, [128, 128, 128, 255]);
        assert!(baked.notes.iter().any(|n| n.contains("图案叠加")));
    }

    /// 斜面：光从左上来时，左上边缘提亮、右下边缘压暗。
    #[test]
    fn bevel_lights_the_side_facing_the_light() {
        let img = layer(40, 40, [128, 128, 128, 255], 5);
        let mut e = effect(EffectKind::BevelEmboss);
        e.size = 4.0;
        e.depth = 100.0;
        e.angle = 135.0;
        e.highlight_color = [255, 255, 255];
        e.shadow_color = [0, 0, 0];
        e.highlight_opacity = 1.0;
        e.shadow_opacity = 1.0;
        let baked = bake(&img, (0, 0), &[e]).expect("baked");
        let at = |x: i32, y: i32| {
            let px = (x - baked.offset.0) as u32;
            let py = (y - baked.offset.1) as u32;
            baked.image.get_pixel(px, py).0[0] as i32
        };
        let lit = at(6, 6);
        let middle = at(20, 20);
        let dark = at(33, 33);
        assert!(lit > middle + 20, "左上应提亮: lit={lit} mid={middle}");
        assert!(dark < middle - 20, "右下应压暗: dark={dark} mid={middle}");
    }

    /// `bvlD = Down` 把凹凸翻过来：同一光向下左上变暗、右下变亮。
    #[test]
    fn bevel_direction_flips_the_shading() {
        let img = layer(40, 40, [128, 128, 128, 255], 5);
        let mut e = effect(EffectKind::BevelEmboss);
        e.size = 4.0;
        e.depth = 100.0;
        e.angle = 135.0;
        e.bevel_direction = psd::BevelDirection::Down;
        e.highlight_color = [255, 255, 255];
        e.shadow_color = [0, 0, 0];
        e.highlight_opacity = 1.0;
        e.shadow_opacity = 1.0;
        let baked = bake(&img, (0, 0), &[e]).expect("baked");
        let at = |x: i32, y: i32| {
            let px = (x - baked.offset.0) as u32;
            let py = (y - baked.offset.1) as u32;
            baked.image.get_pixel(px, py).0[0] as i32
        };
        assert!(at(6, 6) < at(33, 33), "凹凸翻转后右下反而更亮");
    }
}

/// Exposed for the diagnostic test below.
#[cfg(test)]
pub(crate) fn parse_layer_effects_for_debug(bytes: &[u8]) -> Option<Vec<psd::effects::LayerEffect>> {
    psd::effects::parse_layer_effects(bytes)
}

#[cfg(test)]
mod debug_probe {
    /// Diagnostic: parse every `vstk` block in a file and report what the
    /// shape-stroke reader accepts.
    ///
    /// ```text
    /// set IMAGEEDIT_INSPECT_PSD=<file.psd>
    /// cargo test -p imageedit-decode debug_vstk -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn debug_vstk() {
        let Ok(path) = std::env::var("IMAGEEDIT_INSPECT_PSD") else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let (mut blocks, mut parsed) = (0, 0);
        let mut i = 0usize;
        while let Some(off) = find(&bytes[i..], b"vstk") {
            let at = i + off;
            i = at + 4;
            let Some(len_bytes) = bytes.get(at + 4..at + 8) else { break };
            let len = u32::from_be_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
            let Some(block) = bytes.get(at + 8..at + 8 + len) else { continue };
            blocks += 1;
            match psd::vector::parse_vector_stroke(block) {
                Some(s) => {
                    parsed += 1;
                    if parsed <= 4 {
                        println!(
                            "ok fill_enabled={} stroke_enabled={} width={} align={:?} opacity={} solid={:?} grad={}",
                            s.fill_enabled, s.enabled, s.width, s.align, s.opacity, s.fill.solid, s.fill.gradient.is_some()
                        );
                    }
                }
                None => {
                    if blocks <= 4 {
                        println!("fail head={:?}", &block[..block.len().min(20)]);
                    }
                }
            }
        }
        println!("vstk blocks {blocks} parsed {parsed}");
    }

    /// Diagnostic: dump the contour (`Trns`) sub-descriptors of every `lfx2`
    /// block, so the renderer can be written against the shapes the target
    /// file actually carries.
    ///
    /// ```text
    /// set IMAGEEDIT_INSPECT_PSD=<file.psd>
    /// cargo test -p imageedit-decode debug_lfx2_contours -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn debug_lfx2_contours() {
        use psd::descriptor::{parse_block2, Descriptor, DescriptorValue};
        let Ok(path) = std::env::var("IMAGEEDIT_INSPECT_PSD") else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let (mut blocks, mut effects, mut with_contour, mut non_linear) = (0, 0, 0, 0);
        let mut shapes: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut printed = 0;
        let mut i = 0usize;
        while let Some(off) = find(&bytes[i..], b"lfx2") {
            let at = i + off;
            i = at + 4;
            let Some(len_bytes) = bytes.get(at + 4..at + 8) else { break };
            let len = u32::from_be_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
            let Some(block) = bytes.get(at + 8..at + 8 + len) else { continue };
            let Some(root) = parse_block2(block) else { continue };
            blocks += 1;
            for (key, value) in &root.items {
                let DescriptorValue::Descriptor(d) = value else { continue };
                effects += 1;
                let key_s = String::from_utf8_lossy(key).to_string();
                if seen_keys.insert(key_s.clone()) {
                    println!(
                        "keys on {}: {:?}",
                        key_s,
                        d.items
                            .iter()
                            .map(|(k, v)| {
                                let t = match v {
                                    DescriptorValue::Descriptor(_) => "Objc",
                                    DescriptorValue::List(_) => "VlLs",
                                    DescriptorValue::Unit { .. } => "UntF",
                                    DescriptorValue::Double(_) => "doub",
                                    DescriptorValue::Integer(_) => "long",
                                    DescriptorValue::Bool(_) => "bool",
                                    DescriptorValue::Text(_) => "TEXT",
                                    DescriptorValue::Enum { .. } => "enum",
                                    DescriptorValue::Opaque => "opaque",
                                };
                                format!("{}:{t}", String::from_utf8_lossy(k))
                            })
                            .collect::<Vec<_>>()
                    );
                }
                let contour = d
                    .get(b"TrnS")
                    .or_else(|| d.get(b"Trns"))
                    .and_then(|v| match v {
                        DescriptorValue::Descriptor(c) => Some(c),
                        _ => None,
                    });
                let Some(c) = contour else { continue };
                with_contour += 1;
                let pts = contour_points(c);
                if pts.len() > 2 || pts.iter().any(|(x, y)| (x - y).abs() > 0.02) {
                    non_linear += 1;
                }
                let key_s = format!(
                    "{}/{} pts={}",
                    String::from_utf8_lossy(key),
                    String::from_utf8_lossy(&c.class_id),
                    pts.len()
                );
                *shapes.entry(key_s).or_insert(0) += 1;
                if printed < 6 && (pts.len() > 2 || pts.iter().any(|(x, y)| (x - y).abs() > 0.02)) {
                    printed += 1;
                    println!(
                        "contour on {} children {:?} pts {:?}",
                        String::from_utf8_lossy(key),
                        c.items.iter().map(|(k, _)| String::from_utf8_lossy(k).to_string()).collect::<Vec<_>>(),
                        pts
                    );
                }
            }
        }
        println!("lfx2 blocks {blocks} effects {effects} with-contour {with_contour} non-linear {non_linear}");
        let mut list: Vec<_> = shapes.into_iter().collect();
        list.sort_by(|a, b| b.1.cmp(&a.1));
        for (k, v) in list {
            println!("{v:>5}  {k}");
        }
        let _ = (parse_block2 as fn(&[u8]) -> Option<Descriptor>, &mut printed);
    }

    /// Normalised (x, y) control points of a contour descriptor (`Crv ` list of
    /// `Hrzn` / `Vrtc` pairs; Photoshop's plane is 0..255 with y down).
    fn contour_points(c: &psd::descriptor::Descriptor) -> Vec<(f32, f32)> {
        use psd::descriptor::DescriptorValue;
        let mut out = Vec::new();
        let Some(list) = c.list(b"Crv ") else { return out };
        for item in list {
            let DescriptorValue::Descriptor(p) = item else { continue };
            let (Some(x), Some(y)) = (p.number(b"Hrzn"), p.number(b"Vrtc")) else { continue };
            out.push((x as f32, y as f32));
        }
        out
    }

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
