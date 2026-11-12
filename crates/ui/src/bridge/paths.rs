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

/// File extension for a save target. `SaveFormat` is explicit: image 0.25's
/// extension map returns `None` for `.psd`, and blindly falling back to PNG
/// once wrote PNG bytes into a `.psd` file.
pub(in crate::bridge) fn extension_for_save(format: SaveFormat) -> &'static str {
    match format {
        SaveFormat::Psd => "psd",
        SaveFormat::Image(Format::Jpeg) => "jpg",
        SaveFormat::Image(Format::Png) => "png",
        SaveFormat::Image(Format::WebP) => "webp",
        SaveFormat::Image(Format::Bmp) => "bmp",
        SaveFormat::Image(Format::Tiff) => "tiff",
        SaveFormat::Image(Format::Gif) => "gif",
        SaveFormat::Image(_) => "png",
    }
}

/// True when the path points at a Photoshop document.
pub(in crate::bridge) fn is_psd_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("psd"))
        .unwrap_or(false)
}

/// C8: a downsampled (>8192px) document must never overwrite its source —
/// that would silently discard the original resolution.
pub(in crate::bridge) const DOWNSAMPLED_HINT: &str = "此文件以降采样预览打开，写回会丢失分辨率；请关闭「大图优先降采样」后重新打开，或另存为其它文件";

/// A PSD opened as a composite (stage 2 failed / skipped / flat file) has no
/// layer stack. Writing it back as PSD would destroy the original layers.
pub(in crate::bridge) const PSD_COMPOSITE_HINT: &str = "此 PSD 以合并预览打开（图层未加载或已跳过），直接写回会丢失原图层；请重新打开并等待图层加载完成，或另存为 PNG / JPEG 等格式";

/// Explicit format for ANY save target (source path or Save-As hand-typed
/// path). Never `unwrap_or(Png)` on a `.psd` — that was the historical bug.
pub(in crate::bridge) fn save_format_for_target(path: &Path) -> SaveFormat {
    if is_psd_path(path) {
        SaveFormat::Psd
    } else {
        SaveFormat::Image(format_from_path(path).unwrap_or(Format::Png))
    }
}

/// Save format for Ctrl+S (the document's own source path).
pub(in crate::bridge) fn save_format_for_source(path: &Path) -> SaveFormat {
    save_format_for_target(path)
}

/// Pure validation of a save target (unit-testable without a full Session).
/// `Session::save_target` is a thin wrapper over this.
pub(in crate::bridge) fn validate_save_target(
    target: &Path,
    source: Option<&Path>,
    downsampled: bool,
    psd_layered: bool,
) -> Result<SaveFormat, String> {
    let format = save_format_for_target(target);
    let overwriting_source = source == Some(target);
    // C8: overwriting a downsampled source silently discards resolution.
    if downsampled && overwriting_source {
        return Err(DOWNSAMPLED_HINT.to_string());
    }
    if matches!(format, SaveFormat::Psd) {
        if downsampled {
            return Err(DOWNSAMPLED_HINT.to_string());
        }
        // A composite-only PSD has no layer stack. Overwriting ITS OWN source
        // would burn the original layers, so that stays forbidden — but
        // exporting the current image to a DIFFERENT `.psd` is a legitimate
        // "flatten and export" (one flat layer), and it is also how a plain
        // PNG gets turned into a PSD. Only the overwrite is a data-loss risk.
        if overwriting_source && source.map(is_psd_path).unwrap_or(false) && !psd_layered {
            return Err(PSD_COMPOSITE_HINT.to_string());
        }
    }
    Ok(format)
}

/// Atomic write: `<target>.imageedit-<pid>.tmp` → flush+sync → rename over
/// the target. A failure at any point leaves the original file untouched.
/// On Windows `std::fs::rename` uses MOVEFILE_REPLACE_EXISTING, so overwriting
/// an existing document works without a remove-then-write gap.
pub(in crate::bridge) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| format!("无法创建目录：{e}"))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let tmp = path.with_extension(format!(
        "{ext}.imageedit-{}.tmp",
        std::process::id()
    ));
    let write = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = write {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("保存失败（原文件未被修改）：{e}"));
    }
    Ok(())
}

#[cfg(test)]
mod psd_save_guard_tests {
    use crate::bridge::*;

    #[test]
    fn psd_paths_detected_case_insensitive() {
        assert!(is_psd_path(Path::new("D:/art/ui.psd")));
        assert!(is_psd_path(Path::new("D:/art/UI.PSD")));
        assert!(!is_psd_path(Path::new("D:/art/ui.png")));
        assert!(!is_psd_path(Path::new("D:/art/psd"))); // no extension
        assert!(!is_psd_path(Path::new("")));
    }

    #[test]
    fn save_formats_are_explicit_for_every_target() {
        // The corruption bug: .psd + unwrap_or(Png) wrote PNG bytes into the
        // .psd file. Now .psd maps to SaveFormat::Psd, never to an image format.
        assert_eq!(save_format_for_source(Path::new("D:/art/ui.psd")), SaveFormat::Psd);
        assert_eq!(save_format_for_target(Path::new("D:/art/hand.PSD")), SaveFormat::Psd);
        assert_eq!(
            save_format_for_source(Path::new("D:/art/ui.PNG")),
            SaveFormat::Image(Format::Png)
        );
        assert_eq!(
            save_format_for_source(Path::new("D:/art/a.jpg")),
            SaveFormat::Image(Format::Jpeg)
        );
        assert_eq!(
            save_format_for_target(Path::new("D:/art/unknown.xyz")),
            SaveFormat::Image(Format::Png)
        );
        assert_eq!(extension_for_save(SaveFormat::Psd), "psd");
        assert_eq!(extension_for_save(SaveFormat::Image(Format::Jpeg)), "jpg");
    }

    #[test]
    fn writeback_guards_cover_composite_and_downsampled_cases() {
        let src = Path::new("D:/art/ui.psd");
        let png = Path::new("D:/art/ui.png");
        // Fully layered PSD: Ctrl+S and Save-As PSD both allowed.
        assert_eq!(validate_save_target(src, Some(src), false, true), Ok(SaveFormat::Psd));
        assert_eq!(
            validate_save_target(Path::new("D:/art/copy.psd"), Some(src), false, true),
            Ok(SaveFormat::Psd)
        );
        // Composite-only PSD source: writing back as PSD is forbidden, but
        // exporting to PNG (including a different path) stays available.
        assert_eq!(
            validate_save_target(src, Some(src), false, false),
            Err(PSD_COMPOSITE_HINT.to_string())
        );
        assert_eq!(
            validate_save_target(Path::new("D:/art/out.png"), Some(src), false, false),
            Ok(SaveFormat::Image(Format::Png))
        );
        // A plain image may be Saved As a NEW PSD (one flat layer).
        assert_eq!(
            validate_save_target(Path::new("D:/art/new.psd"), Some(png), false, false),
            Ok(SaveFormat::Psd)
        );
        // A composite-only PSD may still be EXPORTED to another .psd: only
        // overwriting its own source would destroy the original layers.
        assert_eq!(
            validate_save_target(Path::new("D:/art/flat.psd"), Some(src), false, false),
            Ok(SaveFormat::Psd)
        );
        // A document without a source path (new / untitled) is not an
        // overwrite either, so `另存为 PSD` must work from a blank document.
        assert_eq!(
            validate_save_target(Path::new("D:/art/untitled.psd"), None, false, false),
            Ok(SaveFormat::Psd)
        );
        // C8: downsampled source overwrite (any format) + any PSD write.
        assert_eq!(
            validate_save_target(src, Some(src), true, true),
            Err(DOWNSAMPLED_HINT.to_string())
        );
        assert_eq!(
            validate_save_target(Path::new("D:/art/copy.psd"), Some(src), true, true),
            Err(DOWNSAMPLED_HINT.to_string())
        );
        assert_eq!(
            validate_save_target(Path::new("D:/art/export.png"), Some(src), true, true),
            Ok(SaveFormat::Image(Format::Png))
        );
        // Historical bug guard: a hand-typed .psd must never become PNG bytes.
        assert_eq!(
            validate_save_target(Path::new("D:/art/hand.PSD"), None, false, false),
            Ok(SaveFormat::Psd)
        );
    }

    #[test]
    fn atomic_write_replaces_existing_file_and_keeps_it_on_failure() {
        let dir = std::env::temp_dir().join(format!(
            "imageedit-atomic-{}",
            std::process::id()
        ));
        let path = dir.join("doc.psd");
        let _ = std::fs::remove_file(&path);
        atomic_write(&path, b"first").expect("create");
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        atomic_write(&path, b"second").expect("overwrite");
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        // A second overwrite must not leave .tmp files behind.
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().to_string_lossy().contains(".imageedit-"))
                .count(),
            0
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
