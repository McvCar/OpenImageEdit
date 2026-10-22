// paths 相关项。所有项标 pub(in crate::bridge)，mod.rs 再导出。
use super::*;

/// Process-global clipboard (per-window `RefCell` fields cannot share state
/// across the multi-window Bridges). Holds a full `Layer`, so pixels survive
/// opening a different document (cross-image copy/paste).
pub(in crate::bridge) fn clipboard() -> &'static std::sync::Mutex<Option<Layer>> {
    static CLIPBOARD: std::sync::OnceLock<std::sync::Mutex<Option<Layer>>> =
        std::sync::OnceLock::new();
    CLIPBOARD.get_or_init(|| std::sync::Mutex::new(None))
}

/// Render the 16×16 checkerboard tile (8 px cells) as a 32bpp bottom-up BMP
/// and return a `file:///` URL, or `None` on any failure (QML then just hides
/// the checkerboard).
///
/// Written next to settings.ini. `slot` alternates the filename (a/b) so the
/// QML `Image` sees a changed URL on color edits — same-path rewrites would
/// not reload the texture. Generated here instead of painted in QML because
/// the Canvas + `toDataURL` route re-enters the synchronous Immediate paint
/// path from `onPainted` and crashes (even with a JS re-entrancy guard).
pub(in crate::bridge) fn write_checker_tile_bmp(light: &str, dark: &str, slot: u8) -> Option<String> {
    fn parse_hex(s: &str) -> Option<[u8; 3]> {
        let s = s.strip_prefix('#')?;
        if s.len() != 6 {
            return None;
        }
        let mut out = [0u8; 3];
        for (i, c) in out.iter_mut().enumerate() {
            *c = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(out)
    }
    let (light, dark) = (parse_hex(light)?, parse_hex(dark)?);
    let (w, h) = (16u32, 16u32);
    let row = (w * 4) as usize;
    let pixels = row * h as usize;
    let mut buf = Vec::with_capacity(54 + pixels);
    // BITMAPFILEHEADER
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&((54 + pixels) as u32).to_le_bytes());
    buf.extend_from_slice(&[0; 4]);
    buf.extend_from_slice(&54u32.to_le_bytes());
    // BITMAPINFOHEADER (BI_RGB, bottom-up)
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(w as i32).to_le_bytes());
    buf.extend_from_slice(&(h as i32).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&32u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&(pixels as u32).to_le_bytes());
    buf.extend_from_slice(&2835u32.to_le_bytes());
    buf.extend_from_slice(&2835u32.to_le_bytes());
    buf.extend_from_slice(&[0; 8]);
    for gy in (0..h).rev() {
        for gx in 0..w {
            let c = if (gx / 8) % 2 == (gy / 8) % 2 { light } else { dark };
            buf.extend_from_slice(&[c[2], c[1], c[0], 255]);
        }
    }
    let dir = AppConfig::path()?.parent()?.to_path_buf();
    std::fs::create_dir_all(&dir).ok()?;
    let name = format!("checker-tile-{}.bmp", if slot == 0 { "a" } else { "b" });
    std::fs::write(dir.join(&name), &buf).ok()?;
    Some(format!(
        "file:///{}/{}",
        dir.to_string_lossy().replace('\\', "/"),
        name
    ))
}

/// `file:///…` URL for a local path, percent-encoded per RFC 3986.
///
/// This used to be a raw concat, so a `#` in a filename (game assets love
/// `颜色#236652.png`) leaked into the URL: QML's `url` type treats `#` as the
/// fragment separator, the path was silently truncated at it, and the file
/// "failed to open" while decode itself was never even reached. Encoding every
/// byte outside the unreserved set (plus `/` and `:`) round-trips through
/// QUrl → toLocalFile unchanged.
pub(in crate::bridge) fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str("file:///");
    for b in text.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => out.push(*b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub(in crate::bridge) fn extension_for(format: Format) -> &'static str {
    match format {
        Format::Jpeg => "jpg",
        Format::Png => "png",
        Format::WebP => "webp",
        Format::Bmp => "bmp",
        Format::Tiff => "tiff",
        Format::Gif => "gif",
        _ => "png",
    }
}

/// True when the path points at a Photoshop document. PSD is preview-only
/// (the decoder flattens layers) and image 0.25 has no PSD encoder — its
/// extension map returns None for `.psd`, so every `unwrap_or(Format::Png)`
/// save path would silently write PNG bytes into the .psd file.
pub(in crate::bridge) fn is_psd_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("psd"))
        .unwrap_or(false)
}

pub(in crate::bridge) const PSD_SAVE_HINT: &str = "PSD 是预览专用格式（打开时图层已合并），不能直接保存原文件，请用「另存为」导出 PNG / JPEG 等格式";

/// Encode format for saving a document loaded from `path`. PSD sources fall
/// back to PNG: image 0.25 has no PSD encoder and its extension map returns
/// None for `.psd`, so the historical `unwrap_or(Format::Png)` here is what
/// silently wrote PNG bytes into .psd files.
pub(in crate::bridge) fn save_format_for_source(path: &Path) -> Format {
    if is_psd_path(path) {
        Format::Png
    } else {
        format_from_path(path).unwrap_or(Format::Png)
    }
}
