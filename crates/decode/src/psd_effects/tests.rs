use super::*;
use super::ops::*;
use psd::effects::{GlowSource, Gradient, GradientKind, StrokeFill};
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
