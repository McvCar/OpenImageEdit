// geom 相关项。所有项标 pub(in crate::bridge)，mod.rs 再导出。
use super::*;

/// Zoom-to-fit: the largest zoom that fits the (logical) viewport.
///
/// `zoom` is defined as *physical* pixels per image pixel (the convention
/// used by Photos and Photoshop for "actual size"): at zoom 1.0 one image
/// pixel occupies one physical pixel on screen, whatever the OS scaling. The
/// on-screen item is sized in logical pixels, so the physical viewport is
/// `viewport × dpr` and the fitting zoom is `viewport × dpr / canvas`, capped
/// at 1.0 (fit never zooms past 100%). At dpr = 1 this is exactly the old
/// `viewport / canvas`, so 100%-scaling displays are unchanged.
pub(in crate::bridge) fn fit_zoom(canvas: Size, viewport: Size, dpr: f64) -> f64 {
    let dpr = if dpr.is_finite() && dpr > 0.0 { dpr } else { 1.0 };
    if canvas.pixels() == 0 || viewport.pixels() == 0 {
        return 1.0;
    }
    let sx = (viewport.width as f64 * dpr) / canvas.width as f64;
    let sy = (viewport.height as f64 * dpr) / canvas.height as f64;
    sx.min(sy).min(1.0)
}

/// On-screen (logical) size of the image at a given zoom.
///
/// `zoom` is physical pixels per image pixel, and the QML `Image` item is
/// sized in logical pixels, so the logical size is `canvas × zoom / dpr`. At
/// zoom 1.0 (100%) the image is 1 image px = 1 physical px independent of OS
/// scaling — matching Photos/PS "actual size". Dividing by dpr (rather than
/// the old bare `canvas × zoom`) is what keeps 100% from being 25% larger than
/// Photos on a 125%-scaled display. At dpr = 1 this is unchanged.
pub(in crate::bridge) fn display_size(canvas: Size, zoom: f64, dpr: f64) -> Size {
    let dpr = if dpr.is_finite() && dpr > 0.0 { dpr } else { 1.0 };
    Size::new(
        ((canvas.width as f64 * zoom / dpr).round() as u64).clamp(1, MAX_DISPLAY_EDGE as u64) as u32,
        ((canvas.height as f64 * zoom / dpr).round() as u64).clamp(1, MAX_DISPLAY_EDGE as u64) as u32,
    )
}

/// Two display sizes that differ by at most 2px on each axis are treated as
/// "the preview already on screen": re-rendering for them costs a full preview
/// composite (hundreds of ms in debug) and changes nothing visible.
pub(in crate::bridge) fn size_close(a: Size, b: Size) -> bool {
    const TOL: u32 = 2;
    let dw = a.width.abs_diff(b.width);
    let dh = a.height.abs_diff(b.height);
    dw <= TOL && dh <= TOL
}

/// Preview bitmap size for a display size.
///
/// The bitmap is rendered in PHYSICAL pixels: the QML `Image` item is sized in
/// logical pixels (`display`) but the GPU draws it at `logical × devicePixelRatio`
/// physical pixels, so the source texture must match that to stay crisp. Since
/// `display` is already `canvas × zoom / dpr`, the physical size is `canvas ×
/// zoom` — independent of OS scaling. At dpr = 1 this is exactly the display
/// size (no behaviour change); at dpr > 1 the preview is upscaled accordingly
/// so a fit-mode image is no longer blurred by the GPU.
///
/// Two caps still apply, in order:
///  1. The document's own resolution — zooming past 100% cannot create detail,
///     so the composite never exceeds the canvas pixel count (every tick above
///     100% shares one target, letting the zoom-only render skip reuse a frame).
///  2. [`MAX_PREVIEW_EDGE`] (physical) bounds memory/composite cost for giant
///     canvases and high-DPI displays.
pub(in crate::bridge) fn preview_target(display: Size, canvas: Size, dpr: f64) -> Size {
    let dpr = if dpr.is_finite() && dpr > 0.0 { dpr } else { 1.0 };
    // Physical pixels the Image will be drawn at.
    let physical = Size::new(
        ((display.width as f64 * dpr).round() as u64).max(1) as u32,
        ((display.height as f64 * dpr).round() as u64).max(1) as u32,
    );
    // Cap at the document's native resolution (the real detail limit).
    let mut w = physical.width.min(canvas.width).max(1);
    let mut h = physical.height.min(canvas.height).max(1);
    // Cap at the physical memory limit for giant canvases / high DPI.
    let longest = w.max(h);
    if longest > MAX_PREVIEW_EDGE {
        let scale = MAX_PREVIEW_EDGE as f64 / longest as f64;
        w = ((w as f64 * scale).round() as u32).max(1);
        h = ((h as f64 * scale).round() as u32).max(1);
    }
    Size::new(w, h)
}

/// Selection that replaces a just-deleted layer: prefer the neighbour above
/// (`ix-1`), clamped to the remaining list. Deleting layer 0 selects the new
/// layer 0 (usize underflow guard — this crashed debug builds via the cxx FFI
/// abort); no layers left = -1.
pub(in crate::bridge) fn selection_after_removal(ix: usize, new_count: usize) -> i32 {
    if new_count == 0 {
        return -1;
    }
    ix.saturating_sub(1).min(new_count - 1) as i32
}

#[cfg(test)]
mod preview_target_tests {
    use crate::bridge::*;

    #[test]
    fn below_100pct_uses_display_as_is() {
        let canvas = Size::new(1926, 1109);
        let display = Size::new(880, 507);
        assert_eq!(preview_target(display, canvas, 1.0), display);
    }

    #[test]
    fn above_100pct_caps_at_native_canvas() {
        // Zooming in cannot create detail: the composite never grows past the
        // document resolution, and every tick above 100% shares one target so
        // the zoom-only render skip can reuse the frame.
        let canvas = Size::new(1926, 1109);
        let display = Size::new(4196, 2416); // zoom 218%
        assert_eq!(preview_target(display, canvas, 1.0), canvas);
    }

    #[test]
    fn giant_canvas_still_respects_max_preview_edge() {
        // Longest physical edge 12000 > 8192 cap → scaled down to 8192.
        let canvas = Size::new(12000, 8000);
        let display = Size::new(15000, 10000); // zoom 125%
        let target = preview_target(display, canvas, 1.0);
        assert_eq!(target, Size::new(8192, 5461));
    }

    #[test]
    fn never_returns_zero() {
        let canvas = Size::new(1, 1);
        let display = Size::new(4000, 4000); // zoom 4000x
        assert_eq!(preview_target(display, canvas, 1.0), canvas);
    }

    #[test]
    fn high_dpi_renders_at_physical_pixels() {
        // At 150% scaling a 880-logical-px display occupies 1320 physical px;
        // the bitmap must match that or the GPU upscales (blurs) it.
        let canvas = Size::new(1926, 1109);
        let display = Size::new(880, 507);
        assert_eq!(preview_target(display, canvas, 1.5), Size::new(1320, 761));
    }

    #[test]
    fn high_dpi_still_capped_by_canvas() {
        // Zoomed past the document resolution at high DPI: still capped at the
        // native canvas, never grows past the real detail.
        let canvas = Size::new(1926, 1109);
        let display = Size::new(2000, 1158); // zoom 104%
        assert_eq!(preview_target(display, canvas, 2.0), canvas);
    }
}

#[cfg(test)]
mod fit_zoom_tests {
    use crate::bridge::*;

    #[test]
    fn dpr_one_matches_legacy_fit() {
        // Regression: at 100% OS scaling the fit zoom is exactly the old
        // viewport/canvas value — nothing about 100%-scaling displays changes.
        let canvas = Size::new(1926, 1109);
        let viewport = Size::new(880, 507);
        let z = fit_zoom(canvas, viewport, 1.0);
        assert!((z - (880f64 / 1926.0)).abs() < 1e-9);
    }

    #[test]
    fn fit_never_exceeds_100pct() {
        // A viewport larger than the canvas still caps at 1.0 (fit is
        // never a zoom-in), at any dpr.
        let canvas = Size::new(100, 100);
        let viewport = Size::new(4000, 4000);
        assert_eq!(fit_zoom(canvas, viewport, 1.25), 1.0);
    }

    #[test]
    fn fit_scales_with_dpr() {
        // At 125% the same window holds 1.25× more physical pixels, so the
        // fitting (physical-px-per-image-px) zoom is 1.25× the dpr-1 value.
        let canvas = Size::new(1926, 1109);
        let viewport = Size::new(880, 507);
        let z1 = fit_zoom(canvas, viewport, 1.0);
        let z125 = fit_zoom(canvas, viewport, 1.25);
        assert!((z125 - z1 * 1.25).abs() < 1e-9);
    }

    #[test]
    fn degenerate_canvas_falls_back_to_100pct() {
        assert_eq!(fit_zoom(Size::new(0, 100), Size::new(500, 500), 1.25), 1.0);
    }
}

#[cfg(test)]
mod display_size_tests {
    use crate::bridge::*;

    #[test]
    fn hundred_pct_is_one_physical_px_at_any_dpr() {
        // The core fix: at zoom 1.0 the image is 1 image px = 1 physical px,
        // so the *logical* size is canvas/dpr. On a 125%-scaled display a
        // 1000-px image shows at 800 logical px (×1.25 = 1000 physical) —
        // the same physical size as Photos, not 25% larger.
        let canvas = Size::new(1000, 800);
        assert_eq!(display_size(canvas, 1.0, 1.0), Size::new(1000, 800));
        assert_eq!(display_size(canvas, 1.0, 1.25), Size::new(800, 640));
        assert_eq!(display_size(canvas, 1.0, 1.5), Size::new(667, 533));
    }

    #[test]
    fn dpr_one_is_unchanged() {
        // Regression: 100%-scaling displays see the identical logical size as
        // before the fix (canvas × zoom).
        let canvas = Size::new(1926, 1109);
        assert_eq!(display_size(canvas, 0.5, 1.0), Size::new(963, 555));
    }

    #[test]
    fn clamps_to_one_min_pixel() {
        // A sub-pixel display size never rounds to zero.
        assert_eq!(display_size(Size::new(1, 1), 0.01, 16.0), Size::new(1, 1));
    }
}

#[cfg(test)]
mod selection_after_removal_tests {
    use crate::bridge::selection_after_removal;

    #[test]
    fn deleting_layer_zero_selects_new_first_layer() {
        // Regression: `ix - 1` underflowed usize for ix == 0 and aborted the
        // process through the cxx FFI boundary (0xc0000409).
        assert_eq!(selection_after_removal(0, 2), 0);
        assert_eq!(selection_after_removal(0, 1), 0);
    }

    #[test]
    fn mid_and_last_layer_removal() {
        assert_eq!(selection_after_removal(3, 4), 2);
        assert_eq!(selection_after_removal(2, 2), 1); // deleted the last of 3
    }

    #[test]
    fn no_layers_left() {
        assert_eq!(selection_after_removal(0, 0), -1);
        assert_eq!(selection_after_removal(5, 0), -1);
    }

    #[test]
    fn file_url_round_trips_fragment_and_special_chars() {
        use std::path::Path;
        use crate::bridge::file_url;
        // The regression: a `#` in a directory/file name must not terminate
        // the URL path (QML url treats `#` as the fragment separator).
        assert_eq!(
            file_url(Path::new(r"D:\Users\Downloads\按键#236652\a#b.png")),
            "file:///D:/Users/Downloads/%E6%8C%89%E9%94%AE%23236652/a%23b.png"
        );
        // Percent signs in names must not look like pre-existing escapes.
        assert_eq!(
            file_url(Path::new("D:/art/100%.png")),
            "file:///D:/art/100%25.png"
        );
        // Spaces come back through QUrl::toLocalFile as literal spaces.
        assert_eq!(
            file_url(Path::new("D:/my art/ui 1.png")),
            "file:///D:/my%20art/ui%201.png"
        );
        // Plain ASCII stays readable.
        assert_eq!(file_url(Path::new("D:/art/ui.png")), "file:///D:/art/ui.png");
    }
}
