//! Text rasterization for text marks.
//!
//! Fonts are loaded lazily from the Windows font directory at draw time and
//! cached per family id. Rendering produces a coverage buffer (u8 alpha per
//! pixel) that `Mark::draw` blits scaled onto the canvas, so resizes just
//! scale the buffer — no re-layout needed.

use ab_glyph::{Font as _, FontArc, ScaleFont};
use std::collections::HashMap;
use std::sync::Mutex;

/// One selectable font family: id used by marks, display name for the UI,
/// and candidate file paths (first that exists wins).
pub struct FontDef {
    pub id: &'static str,
    pub name: &'static str,
    pub paths: &'static [&'static str],
}

pub const FONTS: &[FontDef] = &[
    FontDef {
        id: "msyh",
        name: "微软雅黑",
        paths: &["C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\msyh.ttf"],
    },
    FontDef {
        id: "simsun",
        name: "宋体",
        paths: &["C:\\Windows\\Fonts\\simsun.ttc", "C:\\Windows\\Fonts\\simsun.ttf"],
    },
    FontDef {
        id: "simhei",
        name: "黑体",
        paths: &["C:\\Windows\\Fonts\\simhei.ttf"],
    },
    FontDef {
        id: "simkai",
        name: "楷体",
        paths: &["C:\\Windows\\Fonts\\simkai.ttf"],
    },
    FontDef {
        id: "arial",
        name: "Arial",
        paths: &["C:\\Windows\\Fonts\\arial.ttf"],
    },
];

fn font_def(id: &str) -> Option<&'static FontDef> {
    FONTS.iter().find(|f| f.id == id)
}

static FONT_CACHE: Mutex<Option<HashMap<&'static str, Option<FontArc>>>> = Mutex::new(None);

fn load_font(id: &str) -> Option<FontArc> {
    let def = font_def(id)?;
    let mut guard = FONT_CACHE.lock().ok()?;
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(hit) = cache.get(def.id) {
        return hit.clone();
    }
    let loaded = def.paths.iter().find_map(|p| {
        let bytes = std::fs::read(p).ok()?;
        // .ttc collections: index 0 is the regular face.
        FontArc::try_from_vec(bytes).ok()
    });
    cache.insert(def.id, loaded.clone());
    loaded
}

/// A rasterized text run: coverage buffer plus its natural pixel size.
pub struct TextLayout {
    pub width: u32,
    pub height: u32,
    /// Row-major coverage (0..=255) of `width * height`.
    pub coverage: Vec<u8>,
}

/// Shape `text` into a coverage buffer at `font_size` px. Faux bold / italic
/// are applied at the buffer level (1 px emboldening / ~12° shear), so no
/// separate face files are needed.
pub fn layout_text(text: &str, font_id: &str, font_size: f32, bold: bool, italic: bool) -> TextLayout {
    let empty = TextLayout { width: 1, height: 1, coverage: vec![0] };
    let font = match load_font(font_id) {
        Some(f) => f,
        None => return empty,
    };
    let font_size = font_size.clamp(4.0, 512.0);
    let scaled = font.as_scaled(font_size);
    let line_h = scaled.ascent() - scaled.descent() + scaled.line_gap();

    // Pass 1: measure (per-line advance sum; ignores kerning, fine for marks).
    let lines: Vec<&str> = text.split('\n').collect();
    let mut max_w = 0.0f32;
    for line in &lines {
        let mut w = 0.0f32;
        for ch in line.chars() {
            w += scaled.h_advance(font.glyph_id(ch));
        }
        max_w = max_w.max(w);
    }
    // Padding for faux-bold extension and italic shear.
    let shear = if italic { 0.22 } else { 0.0 };
    let pad = (line_h * shear).ceil() + if bold { 1.0 } else { 0.0 };
    let width = (max_w + pad).ceil().max(1.0) as u32;
    let height = (line_h * lines.len() as f32).ceil().max(1.0) as u32;

    // Pass 2: rasterize into the buffer. Shear shifts x right for higher
    // rows (top leans right like real italics); bold ORs a 1 px neighbor.
    let mut cov = vec![0u8; (width * height) as usize];
    let put = |cov: &mut [u8], x: i32, y: i32, a: f32| {
        if a <= 0.0 || x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
            return;
        }
        let idx = (y as u32 * width + x as u32) as usize;
        cov[idx] = cov[idx].max((a * 255.0).round() as u8);
    };
    let mut pen_y = scaled.ascent();
    for line in &lines {
        let mut pen_x = 0.0f32;
        for ch in line.chars() {
            let gid = font.glyph_id(ch);
            let glyph = gid.with_scale_and_position(font_size, ab_glyph::point(pen_x, pen_y));
            if let Some(og) = font.outline_glyph(glyph) {
                let b = og.px_bounds();
                let ox = b.min.x.floor() as i32;
                let oy = b.min.y.floor() as i32;
                og.draw(|x, y, a| {
                    // x/y are relative to the glyph's integer bounds origin.
                    let gx = ox + x as i32;
                    let gy = oy + y as i32;
                    if italic {
                        let shift = ((line_h - gy as f32) * shear).round() as i32;
                        put(&mut cov, gx + shift, gy, a);
                        if bold {
                            put(&mut cov, gx + shift + 1, gy, a);
                        }
                    } else {
                        put(&mut cov, gx, gy, a);
                        if bold {
                            put(&mut cov, gx + 1, gy, a);
                        }
                    }
                });
            }
            pen_x += scaled.h_advance(gid);
        }
        pen_y += line_h;
    }
    TextLayout { width, height, coverage: cov }
}

/// Natural pixel size of a text run (what `layout_text` would produce).
/// Falls back to a rough estimate when the font cannot be loaded.
pub fn measure_text(text: &str, font_id: &str, font_size: f32, bold: bool, italic: bool) -> (f32, f32) {
    if load_font(font_id).is_some() {
        let l = layout_text(text, font_id, font_size, bold, italic);
        return (l.width as f32, l.height as f32);
    }
    let size = font_size.clamp(4.0, 512.0);
    let max_line = text.lines().map(str::chars).map(|c| c.count()).max().unwrap_or(1);
    ((max_line as f32) * size * 0.6, size * 1.4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_produces_nonempty_coverage() {
        let l = layout_text("测试 Test", "msyh", 32.0, false, false);
        assert!(l.width > 10 && l.height > 10);
        assert!(l.coverage.iter().any(|&a| a > 0), "coverage must have ink");
    }

    #[test]
    fn measure_matches_layout() {
        let (w, h) = measure_text("Ab", "msyh", 24.0, false, false);
        let l = layout_text("Ab", "msyh", 24.0, false, false);
        assert_eq!(w, l.width as f32);
        assert_eq!(h, l.height as f32);
    }

    #[test]
    fn unknown_font_falls_back() {
        let (w, h) = measure_text("hello", "nope", 24.0, false, false);
        assert!(w > 0.0 && h > 0.0);
        // Layout still returns a 1x1 buffer rather than panicking.
        let l = layout_text("hello", "nope", 24.0, false, false);
        assert_eq!((l.width, l.height), (1, 1));
    }

    #[test]
    fn bold_and_italic_widen() {
        let a = layout_text("W", "msyh", 40.0, false, false);
        let b = layout_text("W", "msyh", 40.0, true, false);
        let it = layout_text("W", "msyh", 40.0, false, true);
        assert!(b.width >= a.width);
        assert!(it.width > a.width, "shear must add width");
    }
}
