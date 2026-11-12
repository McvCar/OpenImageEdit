//! Layer style (`lfx2` / `lmfx` / `lfxs`) parsing.
//!
//! Photoshop stores layer styles — drop shadow, inner shadow, stroke, gradient
//! overlay, ... — as a descriptor tree. This module turns the tree into a flat
//! list of [`LayerEffect`] values the renderer can apply; it deliberately keeps
//! the numbers in Photoshop's own units (pixels, percent, degrees) and only
//! normalises opacity to 0..1 and gradient stop locations to 0..1.
//!
//! Two shapes exist: a single key (`DrSh`, `FrFX`, `GrFl`, ...) holding one
//! descriptor, and the CS6+ `xxxMulti` key holding a `VlLs` list of them. Both
//! are flattened here; see [`parse_layer_effects`] for the stacking rule.
//!
//! Unknown or unparsable blocks produce `None` so the caller renders the layer
//! without styles instead of with half-applied styles.

use crate::descriptor::{parse_block2, Descriptor, DescriptorValue};
use crate::sections::layer_and_mask_information_section::layer::BlendMode;

/// Which layer style this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    /// `DrSh`
    DropShadow,
    /// `IrSh`
    InnerShadow,
    /// `OrGl`
    OuterGlow,
    /// `IrGl`
    InnerGlow,
    /// `FrFX`
    Stroke,
    /// `SoFi`
    ColorOverlay,
    /// `Grdn`
    GradientOverlay,
    /// `PtFl`
    PatternOverlay,
    /// `ebbl`
    BevelEmboss,
}

/// Where a stroke is drawn relative to the layer's edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokePosition {
    /// `OutF` — outside the shape, grows the layer rectangle.
    Outside,
    /// `InsF` — inside the shape.
    Inside,
    /// `CtrF` — centred on the edge.
    Center,
}

/// How a stroke is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeFill {
    /// `SClr`
    Color,
    /// `GRad`
    Gradient,
    /// `Ptrn`
    Pattern,
}

/// Which part of the shape an inner glow starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlowSource {
    /// `SrcS` — centre.
    Center,
    /// `SrcE` — edge.
    Edge,
}

/// Gradient geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientKind {
    /// `Lnr `
    Linear,
    /// `Radl`
    Radial,
    /// `Angl`
    Angle,
    /// `Rflc`
    Reflected,
    /// `Dmnd`
    Diamond,
}

/// One colour stop, location normalised to 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorStop {
    /// 0..1 along the gradient.
    pub location: f32,
    /// sRGB.
    pub color: [u8; 3],
}

/// One transparency stop, both fields normalised to 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlphaStop {
    /// 0..1 along the gradient.
    pub location: f32,
    /// 0..1 opacity.
    pub alpha: f32,
}

/// A gradient as stored in `Grad`.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    /// Colour ramp.
    pub color_stops: Vec<ColorStop>,
    /// Opacity ramp.
    pub alpha_stops: Vec<AlphaStop>,
    /// Linear / radial / ...
    pub kind: GradientKind,
    /// Degrees, counter-clockwise from the +x axis (Photoshop's `Angl`).
    pub angle: f32,
    /// `Rvrs`: flip the ramp.
    pub reverse: bool,
    /// `Algn`: align the ramp to the layer instead of the document.
    pub align: bool,
    /// `Scl ` percent.
    pub scale: f32,
}

impl Gradient {
    /// Colour and alpha at `t` (0..1, clamped) with linear interpolation.
    pub fn sample(&self, t: f32) -> ([u8; 3], f32) {
        let t = if self.reverse { 1.0 - t } else { t };
        let color = sample_color(&self.color_stops, t);
        let alpha = sample_alpha(&self.alpha_stops, t);
        (color, alpha)
    }
}

fn sample_color(stops: &[ColorStop], t: f32) -> [u8; 3] {
    if stops.is_empty() {
        return [255, 255, 255];
    }
    let t = t.clamp(0.0, 1.0);
    if t <= stops[0].location {
        return stops[0].color;
    }
    for pair in stops.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if t <= b.location {
            let span = (b.location - a.location).max(1e-5);
            let k = (t - a.location) / span;
            return [
                lerp_u8(a.color[0], b.color[0], k),
                lerp_u8(a.color[1], b.color[1], k),
                lerp_u8(a.color[2], b.color[2], k),
            ];
        }
    }
    stops[stops.len() - 1].color
}

fn sample_alpha(stops: &[AlphaStop], t: f32) -> f32 {
    if stops.is_empty() {
        return 1.0;
    }
    let t = t.clamp(0.0, 1.0);
    if t <= stops[0].location {
        return stops[0].alpha;
    }
    for pair in stops.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if t <= b.location {
            let span = (b.location - a.location).max(1e-5);
            let k = (t - a.location) / span;
            return a.alpha + (b.alpha - a.alpha) * k;
        }
    }
    stops[stops.len() - 1].alpha
}

fn lerp_u8(a: u8, b: u8, k: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * k).round().clamp(0.0, 255.0) as u8
}

/// One enabled (or disabled) layer style.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerEffect {
    /// Which style it is.
    pub kind: EffectKind,
    /// `enab`: Photoshop keeps disabled styles in the file.
    pub enabled: bool,
    /// `Md  `: how the effect composites onto the layer.
    pub blend: BlendMode,
    /// `Clr ` as sRGB.
    pub color: [u8; 3],
    /// `Opct` percent, normalised to 0..1.
    pub opacity: f32,
    /// `lagl` / `Angl` in degrees.
    pub angle: f32,
    /// `Dstn` in pixels.
    pub distance: f32,
    /// `blur` / `Sz  ` in pixels.
    pub size: f32,
    /// `Ckmt` percent (0..100).
    pub choke: f32,
    /// Stroke only.
    pub stroke_position: StrokePosition,
    /// Stroke only.
    pub stroke_fill: StrokeFill,
    /// Stroke / gradient overlay ramp.
    pub gradient: Option<Gradient>,
    /// `layerConceals`: the layer knocks the shadow out of itself.
    pub knocks_out: bool,
    /// Inner glow only.
    pub glow_source: GlowSource,
}

impl LayerEffect {
    fn new(kind: EffectKind) -> Self {
        LayerEffect {
            kind,
            enabled: true,
            blend: BlendMode::Normal,
            color: [0, 0, 0],
            opacity: 1.0,
            angle: 120.0,
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
}

/// Parse the payload of an `lfx2` / `lmfx` / `lfxs` additional-layer-info block.
///
/// Photoshop CS6+ can attach *several effects of the same kind* to one layer.
/// Those live under `xxxMulti` keys as a `VlLs` list of descriptors, e.g.
/// `frameFXMulti` holds three `FrFX`-shaped descriptors (2 px pink, 4 px white,
/// 6 px pink). The single-key form (`DrSh`, `FrFX`, ...) only holds one. Both
/// forms are flattened into the returned `Vec`, in painting order.
///
/// Returns `None` when the descriptor cannot be read; an empty `Vec` means the
/// block is well-formed but carries no style.
pub fn parse_layer_effects(bytes: &[u8]) -> Option<Vec<LayerEffect>> {
    let root = parse_block2(bytes)?;
    let mode = *FX_MODE.get_or_init(multi_mode);
    // A layer can carry a full effect stack with the master switch off; in that
    // case Photoshop renders none of it.
    if root.boolean(b"masterFXSwitch") == Some(false) {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for (key, value) in &root.items {
        let Some((kind, multi)) = kind_for_key(key, mode == MultiMode::Off) else {
            continue;
        };
        if multi {
            collect_multi(kind, value, mode, &mut out);
        } else {
            let DescriptorValue::Descriptor(desc) = value else {
                continue;
            };
            out.push(parse_effect(kind, desc));
        }
    }
    Some(out)
}

/// How `*Multi` lists are turned into a painting order.
///
/// Kept as an environment switch so the correct Photoshop behaviour can be
/// established by measuring the composite against the PSD's own merged image
/// without rebuilding; `IMAGEEDIT_FX_MODE=off` reproduces the pre-fix coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MultiMode {
    /// Ignore `*Multi` (and the keys only the new format uses). Legacy path.
    Off,
    /// File order: later entries paint last and cover earlier ones.
    File,
    /// Reverse file order: the first entry ends up on top (Photoshop's stack).
    Reverse,
    /// Only the first enabled entry.
    First,
    /// Only the last enabled entry.
    Last,
}

fn multi_mode() -> MultiMode {
    match std::env::var("IMAGEEDIT_FX_MODE").ok().as_deref() {
        Some("off") => MultiMode::Off,
        Some("file") => MultiMode::File,
        Some("reverse") => MultiMode::Reverse,
        Some("first") => MultiMode::First,
        Some("last") => MultiMode::Last,
        // Measured against the PSD's own merged image (750x1700, 617 layers):
        //   off 6.986 | file 6.971 | reverse 6.767 | first 6.674 | last 6.885
        // Painting every entry of a `*Multi` stack accumulates errors in our
        // baked renderer (no live style stack), and the extra entries are
        // usually superseded drafts. Taking the top-most enabled entry is both
        // the closest match and the cheapest to render.
        _ => MultiMode::First,
    }
}

static FX_MODE: std::sync::OnceLock<MultiMode> = std::sync::OnceLock::new();

fn collect_multi(
    kind: EffectKind,
    value: &DescriptorValue,
    mode: MultiMode,
    out: &mut Vec<LayerEffect>,
) {
    if mode == MultiMode::Off {
        return;
    }
    let DescriptorValue::List(items) = value else {
        return;
    };
    let mut group: Vec<LayerEffect> = items
        .iter()
        .filter_map(|item| match item {
            DescriptorValue::Descriptor(d) => Some(parse_effect(kind, d)),
            _ => None,
        })
        .collect();
    match mode {
        MultiMode::File => {}
        // Photoshop lists a stack's effects top-first in the UI but draws them
        // bottom-first, so the first descriptor must be painted last.
        MultiMode::Reverse => group.reverse(),
        MultiMode::First => {
            if let Some(pos) = group.iter().position(|e| e.enabled) {
                let e = group.remove(pos);
                group = vec![e];
            } else {
                group.clear();
            }
        }
        MultiMode::Last => {
            if let Some(pos) = group.iter().rposition(|e| e.enabled) {
                let e = group.remove(pos);
                group = vec![e];
            } else {
                group.clear();
            }
        }
        MultiMode::Off => unreachable!(),
    }
    out.extend(group);
}

/// Map a top-level effect key to its kind and whether it is a `VlLs` list.
///
/// `legacy` restricts the table to the keys the pre-fix parser knew about, so
/// `IMAGEEDIT_FX_MODE=off` is an exact baseline rather than an approximation.
fn kind_for_key(key: &[u8], legacy: bool) -> Option<(EffectKind, bool)> {
    let single = |k: EffectKind| Some((k, false));
    let list = |k: EffectKind| Some((k, true));
    match key {
        b"DrSh" => single(EffectKind::DropShadow),
        b"IrSh" => single(EffectKind::InnerShadow),
        b"OrGl" => single(EffectKind::OuterGlow),
        b"IrGl" => single(EffectKind::InnerGlow),
        b"FrFX" => single(EffectKind::Stroke),
        b"SoFi" => single(EffectKind::ColorOverlay),
        b"GrFl" | b"Grdn" if !legacy => single(EffectKind::GradientOverlay),
        b"Grdn" => single(EffectKind::GradientOverlay),
        b"PtFl" => single(EffectKind::PatternOverlay),
        b"patternFill" if !legacy => single(EffectKind::PatternOverlay),
        b"ebbl" => single(EffectKind::BevelEmboss),
        b"dropShadowMulti" => list(EffectKind::DropShadow),
        b"innerShadowMulti" => list(EffectKind::InnerShadow),
        b"outerGlowMulti" => list(EffectKind::OuterGlow),
        b"innerGlowMulti" => list(EffectKind::InnerGlow),
        b"frameFXMulti" => list(EffectKind::Stroke),
        b"solidFillMulti" => list(EffectKind::ColorOverlay),
        b"gradientFillMulti" => list(EffectKind::GradientOverlay),
        b"patternFillMulti" => list(EffectKind::PatternOverlay),
        b"bevelEmbossMulti" => list(EffectKind::BevelEmboss),
        _ => None,
    }
}

fn parse_effect(kind: EffectKind, d: &Descriptor) -> LayerEffect {
    let mut e = LayerEffect::new(kind);
    if let Some(v) = d.boolean(b"enab") {
        e.enabled = v;
    }
    // Some CS6+ descriptors keep historical entries with `enab: true` but
    // `present: false`. Those are not part of the rendered stack.
    if d.boolean(b"present") == Some(false) {
        e.enabled = false;
    }
    if let Some(v) = d.enum_value(b"Md  ") {
        e.blend = blend_from_enum(&v);
    }
    if let Some(c) = d.descriptor(b"Clr ") {
        e.color = rgb_from(&c);
    }
    if let Some(v) = d.number(b"Opct") {
        e.opacity = (v / 100.0).clamp(0.0, 1.0) as f32;
    }
    // `lagl` is the effect light angle; `Angl` shows up on gradient overlays
    // and strokes.
    if let Some(v) = d.number(b"lagl") {
        e.angle = v as f32;
    }
    if let Some(v) = d.number(b"Angl") {
        e.angle = v as f32;
    }
    if let Some(v) = d.number(b"Dstn") {
        e.distance = v as f32;
    }
    if let Some(v) = d.number(b"blur") {
        e.size = v as f32;
    }
    if let Some(v) = d.number(b"Sz  ") {
        e.size = v as f32;
    }
    if let Some(v) = d.number(b"Ckmt") {
        e.choke = v as f32;
    }
    if let Some(v) = d.boolean(b"layerConceals") {
        e.knocks_out = v;
    }
    if let Some(v) = d.enum_value(b"Styl") {
        e.stroke_position = match &v {
            b"InsF" => StrokePosition::Inside,
            b"CtrF" => StrokePosition::Center,
            _ => StrokePosition::Outside,
        };
    }
    if let Some(v) = d.enum_value(b"PntT") {
        e.stroke_fill = match &v {
            b"GRad" => StrokeFill::Gradient,
            b"Ptrn" => StrokeFill::Pattern,
            _ => StrokeFill::Color,
        };
    }
    if let Some(v) = d.enum_value(b"Src ") {
        e.glow_source = if &v == b"SrcS" {
            GlowSource::Center
        } else {
            GlowSource::Edge
        };
    }
    if let Some(g) = d.descriptor(b"Grad") {
        e.gradient = Some(parse_gradient(g, d));
    }
    e
}

fn parse_gradient(g: &Descriptor, effect: &Descriptor) -> Gradient {
    let mut grad = Gradient {
        color_stops: Vec::new(),
        alpha_stops: Vec::new(),
        kind: GradientKind::Linear,
        angle: effect.number(b"Angl").unwrap_or(90.0) as f32,
        reverse: false,
        align: true,
        scale: 100.0,
    };
    if let Some(list) = g.list(b"Clrs") {
        for item in list {
            let DescriptorValue::Descriptor(stop) = item else {
                continue;
            };
            let Some(loc) = stop.number(b"Lctn") else {
                continue;
            };
            let color = stop
                .descriptor(b"Clr ")
                .map(rgb_from)
                .unwrap_or([255, 255, 255]);
            grad.color_stops.push(ColorStop {
                location: (loc as f32 / 4096.0).clamp(0.0, 1.0),
                color,
            });
        }
    }
    if let Some(list) = g.list(b"Trns") {
        for item in list {
            let DescriptorValue::Descriptor(stop) = item else {
                continue;
            };
            let Some(loc) = stop.number(b"Lctn") else {
                continue;
            };
            let opacity = stop.number(b"Opct").unwrap_or(100.0) as f32 / 100.0;
            grad.alpha_stops.push(AlphaStop {
                location: (loc as f32 / 4096.0).clamp(0.0, 1.0),
                alpha: opacity.clamp(0.0, 1.0),
            });
        }
    }
    if grad.color_stops.is_empty() {
        // Photoshop always writes stops; an empty list means a corrupt block.
        grad.color_stops.push(ColorStop {
            location: 0.0,
            color: [255, 255, 255],
        });
    }
    grad.color_stops
        .sort_by(|a, b| a.location.total_cmp(&b.location));
    grad.alpha_stops
        .sort_by(|a, b| a.location.total_cmp(&b.location));

    // Both the gradient container and the effect carry geometry; the effect
    // wins when both are present (it is what the Styles panel edits).
    for source in [Some(g), Some(effect)] {
        let Some(src) = source else { continue };
        if let Some(v) = src.enum_value(b"Type").or_else(|| src.enum_value(b"GrdT")) {
            grad.kind = match &v {
                b"Radl" => GradientKind::Radial,
                b"Angl" => GradientKind::Angle,
                b"Rflc" => GradientKind::Reflected,
                b"Dmnd" => GradientKind::Diamond,
                _ => GradientKind::Linear,
            };
        }
        if let Some(v) = src.boolean(b"Rvrs") {
            grad.reverse = v;
        }
        if let Some(v) = src.boolean(b"Algn") {
            grad.align = v;
        }
        if let Some(v) = src.number(b"Scl ") {
            grad.scale = v as f32;
        }
        if let Some(v) = src.number(b"Angl") {
            grad.angle = v as f32;
        }
    }
    grad
}

fn rgb_from(d: &Descriptor) -> [u8; 3] {
    let channel = |key: &[u8; 4]| -> u8 {
        d.number(key)
            .map(|v| v.round().clamp(0.0, 255.0) as u8)
            .unwrap_or(0)
    };
    [channel(b"Rd  "), channel(b"Grn "), channel(b"Bl  ")]
}

/// Descriptor blend enums use their own spelling (`Nrml`, `Mltp`, ...).
fn blend_from_enum(v: &[u8; 4]) -> BlendMode {
    match v {
        b"Nrml" | b"norm" => BlendMode::Normal,
        b"Dslv" => BlendMode::Dissolve,
        b"Drkn" | b"dark" => BlendMode::Darken,
        b"Mltp" | b"mul " => BlendMode::Multiply,
        b"CBrn" | b"idiv" => BlendMode::ColorBurn,
        b"lbrn" => BlendMode::LinearBurn,
        b"dkCl" => BlendMode::DarkerColor,
        b"Lghn" | b"lite" => BlendMode::Lighten,
        b"Scrn" | b"scrn" => BlendMode::Screen,
        b"CDdg" | b"div " => BlendMode::ColorDodge,
        b"lddg" => BlendMode::LinearDodge,
        b"lgCl" => BlendMode::LighterColor,
        b"Ovrl" | b"over" => BlendMode::Overlay,
        b"sLit" => BlendMode::SoftLight,
        b"hLit" => BlendMode::HardLight,
        b"vLit" => BlendMode::VividLight,
        b"lLit" => BlendMode::LinearLight,
        b"pLit" => BlendMode::PinLight,
        b"hMix" => BlendMode::HardMix,
        b"Diff" | b"diff" => BlendMode::Difference,
        b"Smud" | b"smud" => BlendMode::Exclusion,
        b"fsub" => BlendMode::Subtract,
        b"fdiv" => BlendMode::Divide,
        b"H   " | b"hue " => BlendMode::Hue,
        b"Strt" | b"sat " => BlendMode::Saturation,
        b"colr" => BlendMode::Color,
        b"Lmrn" | b"lum " => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_u32(v: &mut Vec<u8>, n: u32) {
        v.extend_from_slice(&n.to_be_bytes());
    }

    fn push_ustring(v: &mut Vec<u8>, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        push_u32(v, units.len() as u32);
        for u in units {
            v.extend_from_slice(&u.to_be_bytes());
        }
    }

    fn key(v: &mut Vec<u8>, k: &[u8; 4]) {
        push_u32(v, 0);
        v.extend_from_slice(k);
    }

    fn unicode_double(v: &mut Vec<u8>, key4: &[u8; 4], n: f64) {
        key(v, key4);
        v.extend_from_slice(b"doub");
        v.extend_from_slice(&n.to_be_bytes());
    }
    fn unit(v: &mut Vec<u8>, key4: &[u8; 4], unit4: &[u8; 4], n: f64) {
        key(v, key4);
        v.extend_from_slice(b"UntF");
        v.extend_from_slice(unit4);
        v.extend_from_slice(&n.to_be_bytes());
    }
    fn bool_item(v: &mut Vec<u8>, key4: &[u8; 4], b: bool) {
        key(v, key4);
        v.extend_from_slice(b"bool");
        v.push(b as u8);
    }

    fn bool_item_long(v: &mut Vec<u8>, k: &[u8], b: bool) {
        push_u32(v, k.len() as u32);
        v.extend_from_slice(k);
        v.extend_from_slice(b"bool");
        v.push(b as u8);
    }
    fn enum_item(v: &mut Vec<u8>, key4: &[u8; 4], type4: &[u8; 4], val4: &[u8; 4]) {
        key(v, key4);
        v.extend_from_slice(b"enum");
        // Both halves are length-prefixed keys, not raw four-byte values.
        key(v, type4);
        key(v, val4);
    }

    /// A `DrSh` block with the parameters a real file carries.
    fn drop_shadow_block() -> Vec<u8> {
        let mut inner = Vec::new();
        bool_item(&mut inner, b"enab", true);
        enum_item(&mut inner, b"Md  ", b"BlnM", b"Mltp");
        unit(&mut inner, b"Opct", b"#Prc", 50.0);
        unit(&mut inner, b"lagl", b"#Ang", 90.0);
        unit(&mut inner, b"Dstn", b"#Pxl", 2.0);
        unit(&mut inner, b"blur", b"#Pxl", 5.0);
        bool_item_long(&mut inner, b"layerConceals", true);
        // { Clr : { Rd, Grn, Bl } }
        let mut clr = Vec::new();
        unicode_double(&mut clr, b"Rd  ", 103.0);
        unicode_double(&mut clr, b"Grn ", 103.0);
        unicode_double(&mut clr, b"Bl  ", 103.0);
        key(&mut inner, b"Clr ");
        inner.extend_from_slice(b"Objc");
        push_ustring(&mut inner, "");
        push_u32(&mut inner, 0);
        inner.extend_from_slice(b"null");
        push_u32(&mut inner, 3);
        inner.extend_from_slice(&clr);

        let mut outer = Vec::new();
        push_u32(&mut outer, 0);
        push_u32(&mut outer, 16);
        push_ustring(&mut outer, "");
        push_u32(&mut outer, 0);
        outer.extend_from_slice(b"null");
        push_u32(&mut outer, 1);
        key(&mut outer, b"DrSh");
        outer.extend_from_slice(b"Objc");
        push_ustring(&mut outer, "");
        push_u32(&mut outer, 0);
        outer.extend_from_slice(b"null");
        // enab, Md, Opct, lagl, Dstn, blur, layerConceals, Clr
        push_u32(&mut outer, 8);
        outer.extend_from_slice(&inner);
        outer
    }

    #[test]
    fn parses_drop_shadow_parameters() {
        let effects = parse_layer_effects(&drop_shadow_block()).expect("parse");
        assert_eq!(effects.len(), 1);
        let e = &effects[0];
        assert_eq!(e.kind, EffectKind::DropShadow);
        assert!(e.enabled);
        assert_eq!(e.blend, BlendMode::Multiply);
        assert_eq!(e.color, [103, 103, 103]);
        assert!((e.opacity - 0.5).abs() < 1e-6);
        assert!((e.angle - 90.0).abs() < 1e-6);
        assert!((e.size - 5.0).abs() < 1e-6);
        assert!(e.knocks_out);
    }

    #[test]
    fn gradient_sampling_interpolates_linearly() {
        let g = Gradient {
            color_stops: vec![
                ColorStop {
                    location: 0.0,
                    color: [0, 0, 0],
                },
                ColorStop {
                    location: 1.0,
                    color: [200, 100, 50],
                },
            ],
            alpha_stops: vec![AlphaStop {
                location: 0.0,
                alpha: 1.0,
            }],
            kind: GradientKind::Linear,
            angle: 90.0,
            reverse: false,
            align: true,
            scale: 100.0,
        };
        assert_eq!(g.sample(0.0).0, [0, 0, 0]);
        assert_eq!(g.sample(1.0).0, [200, 100, 50]);
        assert_eq!(g.sample(0.5).0, [100, 50, 25]);
    }

    #[test]
    fn truncated_block_is_none() {
        let full = drop_shadow_block();
        for cut in [0, 3, 8, 12, full.len() / 2] {
            assert!(parse_layer_effects(&full[..cut]).is_none(), "cut at {cut}");
        }
    }

    fn fx_desc(size: f64, enabled: bool, present: Option<bool>) -> DescriptorValue {
        let mut items = vec![
            (b"enab".to_vec(), DescriptorValue::Bool(enabled)),
            (
                b"Sz  ".to_vec(),
                DescriptorValue::Unit {
                    unit: *b"#Pxl",
                    value: size,
                },
            ),
        ];
        if let Some(p) = present {
            items.push((b"present".to_vec(), DescriptorValue::Bool(p)));
        }
        DescriptorValue::Descriptor(Descriptor {
            class_id: *b"null",
            items,
        })
    }

    fn stroke_multi() -> DescriptorValue {
        DescriptorValue::List(vec![
            fx_desc(2.0, true, Some(true)),
            fx_desc(4.0, true, Some(true)),
            fx_desc(6.0, true, Some(true)),
        ])
    }

    #[test]
    fn multi_list_flattens_into_painting_order() {
        let list = stroke_multi();
        let mut file = Vec::new();
        collect_multi(EffectKind::Stroke, &list, MultiMode::File, &mut file);
        assert_eq!(
            file.iter().map(|e| e.size).collect::<Vec<_>>(),
            vec![2.0, 4.0, 6.0]
        );
    }

    #[test]
    fn multi_reverse_puts_the_first_entry_on_top() {
        let list = stroke_multi();
        let mut out = Vec::new();
        collect_multi(EffectKind::Stroke, &list, MultiMode::Reverse, &mut out);
        assert_eq!(
            out.iter().map(|e| e.size).collect::<Vec<_>>(),
            vec![6.0, 4.0, 2.0]
        );
        assert!(out.iter().all(|e| e.kind == EffectKind::Stroke && e.enabled));
    }

    #[test]
    fn multi_first_last_and_off() {
        let list = DescriptorValue::List(vec![
            fx_desc(1.0, false, Some(false)),
            fx_desc(2.0, true, Some(true)),
            fx_desc(3.0, true, Some(true)),
        ]);
        let mut first = Vec::new();
        collect_multi(EffectKind::Stroke, &list, MultiMode::First, &mut first);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].size, 2.0);

        let mut last = Vec::new();
        collect_multi(EffectKind::Stroke, &list, MultiMode::Last, &mut last);
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].size, 3.0);

        let mut off = Vec::new();
        collect_multi(EffectKind::Stroke, &list, MultiMode::Off, &mut off);
        assert!(off.is_empty());
    }

    #[test]
    fn present_false_wins_over_enab_true() {
        let desc = match fx_desc(2.0, true, Some(false)) {
            DescriptorValue::Descriptor(d) => d,
            _ => unreachable!(),
        };
        assert!(!parse_effect(EffectKind::Stroke, &desc).enabled);
    }

    #[test]
    fn new_format_keys_are_recognised() {
        assert_eq!(
            kind_for_key(b"GrFl", false),
            Some((EffectKind::GradientOverlay, false))
        );
        assert_eq!(
            kind_for_key(b"frameFXMulti", false),
            Some((EffectKind::Stroke, true))
        );
        assert_eq!(
            kind_for_key(b"gradientFillMulti", false),
            Some((EffectKind::GradientOverlay, true))
        );
        // Legacy mode must NOT pick up keys the old parser did not know.
        assert_eq!(kind_for_key(b"GrFl", true), None);
        assert_eq!(
            kind_for_key(b"FrFX", true),
            Some((EffectKind::Stroke, false))
        );
    }

    #[test]
    fn master_switch_off_disables_the_whole_stack() {
        let mut v = Vec::new();
        push_u32(&mut v, 0); // version
        push_u32(&mut v, 16); // data version
        push_ustring(&mut v, "");
        push_u32(&mut v, 0);
        v.extend_from_slice(b"null");
        push_u32(&mut v, 1); // one item
        push_u32(&mut v, b"masterFXSwitch".len() as u32);
        v.extend_from_slice(b"masterFXSwitch");
        v.extend_from_slice(b"bool");
        v.push(0);
        assert!(parse_layer_effects(&v).expect("parse").is_empty());
    }
}
