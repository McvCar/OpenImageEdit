// geom 相关项。所有项标 pub(in crate::bridge)，mod.rs 再导出。
use super::*;

pub(in crate::bridge) fn fit_zoom(canvas: Size, viewport: Size) -> f64 {
    if canvas.pixels() == 0 || viewport.pixels() == 0 {
        return 1.0;
    }
    let sx = viewport.width as f64 / canvas.width as f64;
    let sy = viewport.height as f64 / canvas.height as f64;
    sx.min(sy).min(1.0)
}

pub(in crate::bridge) fn display_size(canvas: Size, zoom: f64) -> Size {
    Size::new(
        ((canvas.width as f64 * zoom).round() as u64).clamp(1, MAX_DISPLAY_EDGE as u64) as u32,
        ((canvas.height as f64 * zoom).round() as u64).clamp(1, MAX_DISPLAY_EDGE as u64) as u32,
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
/// Zooming past 100% cannot create detail — the pixels just get bigger — so
/// the composite is capped at the document's own resolution first (then at
/// [`MAX_PREVIEW_EDGE`] for giant canvases). Two wins: every tick above 100%
/// shares ONE target (the zoom-only render skip can reuse the frame), and the
/// blur stays constant instead of degrading past the 4096 cap. The QML Image
/// does the upscale on the GPU for free.
pub(in crate::bridge) fn preview_target(display: Size, canvas: Size) -> Size {
    let display = Size::new(
        display.width.min(canvas.width).max(1),
        display.height.min(canvas.height).max(1),
    );
    let longest = display.width.max(display.height);
    if longest <= MAX_PREVIEW_EDGE {
        return display;
    }
    let scale = MAX_PREVIEW_EDGE as f64 / longest as f64;
    Size::new(
        ((display.width as f64 * scale).round() as u32).max(1),
        ((display.height as f64 * scale).round() as u32).max(1),
    )
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
