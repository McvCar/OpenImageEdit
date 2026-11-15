use super::legacy;

/// How far a tent of `size` reaches before it is visually zero: its half-width is
/// `size` (two box passes of that width), plus a pixel of slack.
pub(super) fn blur_margin(size: f32) -> i32 {
    size.ceil() as i32 + 2
}

/// `out[p] = mask[p - (dx, dy)]`, i.e. the mask's content moved by (+dx, +dy).
///
/// `outside` fills what the shift pulls in from beyond the buffer: a shadow moves
/// a transparent shape (0), an inner shadow moves the *hole* (1).
pub(super) fn shift_mask(mask: &[f32], w: u32, h: u32, dx: i32, dy: i32, outside: f32) -> Vec<f32> {
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
pub(super) fn spread_split(size: f32, spread_pct: f32) -> (f32, f32) {
    let spread = (spread_pct / 100.0).clamp(0.0, 1.0);
    ((size * spread).round(), size * (1.0 - spread))
}

/// Grows a soft map by `r` px (anti-aliased): a partly covered pixel grows from its
/// own coverage, so a small spread does not turn every edge pixel opaque.
pub(super) fn dilate(map: &[f32], w: u32, h: u32, r: f32) -> Vec<f32> {
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
pub(super) fn tent_blur(map: &[f32], w: u32, h: u32, size: f32) -> Vec<f32> {
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
pub(super) const INSIDE_EPS: f32 = 0.004;

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
pub(super) fn dist_outside(alpha: &[f32], w: u32, h: u32) -> Vec<f32> {
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
pub(super) fn dist_inside(alpha: &[f32], w: u32, h: u32) -> Vec<f32> {
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

pub(super) fn box_blur(src: &[f32], w: u32, h: u32, radius: i32) -> Vec<f32> {
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
pub(super) fn distance_to(mask: &[bool], w: u32, h: u32) -> Vec<f32> {
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
