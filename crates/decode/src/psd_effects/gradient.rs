use psd::effects::{Gradient, GradientKind};

use super::legacy;

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
