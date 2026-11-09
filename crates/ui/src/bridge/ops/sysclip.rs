//! 系统剪贴板里的**图片**（Windows）。
//!
//! 用途：用户在微信截图 / 浏览器 / 资源管理器里 Ctrl+C 之后，直接到画布上
//! Ctrl+V 就能粘成一个新图层。程序内部的复制粘贴走 `paths::clipboard()`
//! （存的是整个 `Layer`），这里只负责「外面复制进来的像素」。
//!
//! 支持的剪贴板格式，按优先级：
//!   1. `CF_DIBV5` (17) / `CF_DIB` (8) —— 绝大多数截图工具 / 浏览器 / Office
//!      给的都是设备无关位图；V5 优先（它带 alpha）。
//!   2. 注册格式 `PNG` —— 少数程序（部分 Chromium 版本、macOS 剪贴板转存）
//!      只放 PNG 字节流。
//!   3. `CF_HDROP` (15) —— 资源管理器里「复制文件」，取第一个路径当图片读。
//!
//! 只在 Windows 上有实现；其它平台一律返回 `None`（调用方回退到内部剪贴板）。

use image::RgbaImage;

// ---------------------------------------------------------------- 公开接口 --

/// 读系统剪贴板里的图片。没有图片（或读失败）返回 `None`。
pub(in crate::bridge) fn read_image() -> Option<RgbaImage> {
    #[cfg(windows)]
    {
        unsafe { read_image_win() }
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// 系统剪贴板的「版本号」：每次有人 SetClipboardData 都会自增。
///
/// 用它判断「内部剪贴板之后，剪贴板有没有被别的程序改写」——改写过就以
/// 系统里的图为准，否则继续用程序内部的像素（跨窗口复制粘贴靠它）。
pub(in crate::bridge) fn sequence_number() -> i64 {
    #[cfg(windows)]
    {
        unsafe { win::GetClipboardSequenceNumber() as i64 }
    }
    #[cfg(not(windows))]
    {
        0
    }
}

// ------------------------------------------------------------- Windows 实现 --

#[cfg(windows)]
unsafe fn read_image_win() -> Option<RgbaImage> {
    use win::*;
    const CF_DIB: u32 = 8;
    const CF_HDROP: u32 = 15;
    const CF_DIBV5: u32 = 17;

    // 别的程序可能正开着剪贴板（复制大图时窗口会短暂锁住），重试几次。
    let mut opened = false;
    for _ in 0..5 {
        if OpenClipboard(0) != 0 {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if !opened {
        return None;
    }
    let result = (|| {
        // 1) 位图。V5 先试：它带明确的 alpha 通道。
        for format in [CF_DIBV5, CF_DIB] {
            if IsClipboardFormatAvailable(format) == 0 {
                continue;
            }
            let Some(bytes) = clip_bytes(format) else {
                continue;
            };
            if let Some(img) = dib_to_rgba(&bytes) {
                return Some(img);
            }
        }
        // 2) PNG 字节流（注册格式名）。
        let name = wide("PNG");
        let png_format = RegisterClipboardFormatW(name.as_ptr());
        if png_format != 0 && IsClipboardFormatAvailable(png_format) != 0 {
            if let Some(bytes) = clip_bytes(png_format) {
                if let Ok(img) = image::load_from_memory(&bytes) {
                    return Some(img.to_rgba8());
                }
            }
        }
        // 3) 复制的文件：只认第一个，且必须是能解码的图片。
        if IsClipboardFormatAvailable(CF_HDROP) != 0 {
            if let Some(path) = hdrop_first_path() {
                if let Ok(img) = image::open(&path) {
                    return Some(img.to_rgba8());
                }
            }
        }
        None
    })();
    CloseClipboard();
    result
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 把某个剪贴板格式的 HGLOBAL 内容整个拷出来（拿到所有权后再解锁，
/// 后续解析都在我们自己的缓冲上做，不长时间占着剪贴板）。
#[cfg(windows)]
unsafe fn clip_bytes(format: u32) -> Option<Vec<u8>> {
    use win::*;
    let handle = GetClipboardData(format);
    if handle == 0 {
        return None;
    }
    let ptr = GlobalLock(handle);
    if ptr.is_null() {
        return None;
    }
    let len = GlobalSize(handle);
    let bytes = if len == 0 {
        None
    } else {
        Some(std::slice::from_raw_parts(ptr as *const u8, len).to_vec())
    };
    GlobalUnlock(handle);
    bytes
}

#[cfg(windows)]
unsafe fn hdrop_first_path() -> Option<std::path::PathBuf> {
    use win::*;
    const CF_HDROP: u32 = 15;
    let handle = GetClipboardData(CF_HDROP);
    if handle == 0 {
        return None;
    }
    // DragQueryFileW 直接吃 HDROP 句柄，不需要 GlobalLock。
    let len = DragQueryFileW(handle, 0, std::ptr::null_mut(), 0);
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = DragQueryFileW(handle, 0, buf.as_mut_ptr(), buf.len() as u32);
    if n == 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(std::path::PathBuf::from(String::from_utf16_lossy(&buf)))
}

// ---- DIB 解析 ----

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    let s = b.get(off..off + 2)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn i32_at(b: &[u8], off: usize) -> Option<i32> {
    u32_at(b, off).map(|v| v as i32)
}

/// `CF_DIB` 的内容 = 一个 `BITMAPINFO`（头 + 调色板）紧跟像素数组。
///
/// 支持 32 / 24 / 8 位、BI_RGB 与 BI_BITFIELDS；其余（RLE、16 位、1/4 位）
/// 返回 `None` 让调用方换下一个格式 —— 反正截图和 Office 复制出来的都是
/// 32 位 BI_RGB。
fn dib_to_rgba(bytes: &[u8]) -> Option<RgbaImage> {
    let header_size = u32_at(bytes, 0)? as usize;
    if header_size < 40 || bytes.len() < header_size {
        return None;
    }
    let width = i32_at(bytes, 4)?;
    let height_raw = i32_at(bytes, 8)?;
    let bit_count = u16_at(bytes, 14)?;
    let compression = u32_at(bytes, 16)?;
    let clr_used = u32_at(bytes, 32).unwrap_or(0) as usize;
    if width <= 0 || height_raw == 0 {
        return None;
    }
    if compression != 0 && compression != 3 {
        return None;
    }
    let top_down = height_raw < 0;
    let (w, h) = (width as u32, height_raw.unsigned_abs());

    let mut palette_off = header_size;
    // 40 字节头 + BI_BITFIELDS：三个颜色掩码紧跟在头后面，占 12 字节。
    if compression == 3 && header_size == 40 {
        palette_off += 12;
    }
    let palette_entries = if bit_count <= 8 {
        if clr_used > 0 {
            clr_used.min(256)
        } else {
            1usize << bit_count
        }
    } else {
        0
    };
    let pixels_off = palette_off + palette_entries * 4;
    let stride = ((width as usize * bit_count as usize + 31) / 32) * 4;
    let need = pixels_off.checked_add(stride.checked_mul(h as usize)?)?;
    if bytes.len() < need {
        return None;
    }

    let mut img = RgbaImage::new(w, h);
    let row_of = |y: u32| -> usize {
        let src_row = if top_down { y } else { h - 1 - y };
        pixels_off + src_row as usize * stride
    };

    match bit_count {
        32 => {
            let mut any_alpha = false;
            for y in 0..h {
                let base = row_of(y);
                for x in 0..w {
                    let p = base + x as usize * 4;
                    let b = *bytes.get(p)?;
                    let g = *bytes.get(p + 1)?;
                    let r = *bytes.get(p + 2)?;
                    let a = *bytes.get(p + 3)?;
                    if a != 0 {
                        any_alpha = true;
                    }
                    img.put_pixel(x, y, image::Rgba([r, g, b, a]));
                }
            }
            // 截图工具普遍把 alpha 全写成 0（「本来是 RGB 图」）——当成不透明，
            // 否则粘出来是一张全透明的图层，用户会以为粘贴失败。
            if !any_alpha {
                for px in img.pixels_mut() {
                    px.0[3] = 255;
                }
            }
        }
        24 => {
            for y in 0..h {
                let base = row_of(y);
                for x in 0..w {
                    let p = base + x as usize * 3;
                    let b = *bytes.get(p)?;
                    let g = *bytes.get(p + 1)?;
                    let r = *bytes.get(p + 2)?;
                    img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
                }
            }
        }
        8 => {
            for y in 0..h {
                let base = row_of(y);
                for x in 0..w {
                    let idx = *bytes.get(base + x as usize)? as usize;
                    if idx >= palette_entries {
                        return None;
                    }
                    let o = palette_off + idx * 4;
                    let b = *bytes.get(o)?;
                    let g = *bytes.get(o + 1)?;
                    let r = *bytes.get(o + 2)?;
                    img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
                }
            }
        }
        _ => return None,
    }
    Some(img)
}

// ------------------------------------------------------------- Win32 声明 --

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    pub type Handle = isize;

    #[link(name = "user32")]
    extern "system" {
        pub fn OpenClipboard(hwnd: Handle) -> i32;
        pub fn CloseClipboard() -> i32;
        pub fn GetClipboardData(format: u32) -> Handle;
        pub fn IsClipboardFormatAvailable(format: u32) -> i32;
        pub fn RegisterClipboardFormatW(name: *const u16) -> u32;
        pub fn GetClipboardSequenceNumber() -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GlobalLock(handle: Handle) -> *mut c_void;
        pub fn GlobalUnlock(handle: Handle) -> i32;
        pub fn GlobalSize(handle: Handle) -> usize;
    }

    #[link(name = "shell32")]
    extern "system" {
        pub fn DragQueryFileW(hdrop: Handle, index: u32, buf: *mut u16, size: u32) -> u32;
    }
}

#[cfg(test)]
mod tests {
    use super::dib_to_rgba;

    /// 造一个 2×2 的 32 位 bottom-up DIB（带 40 字节头）。
    fn dib32(pixels_top_down: [[u8; 4]; 4], any_alpha: bool) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&40u32.to_le_bytes()); // biSize
        b.extend_from_slice(&2i32.to_le_bytes()); // biWidth
        b.extend_from_slice(&2i32.to_le_bytes()); // biHeight（正 = bottom-up）
        b.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        b.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
        b.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
        b.extend_from_slice(&16u32.to_le_bytes()); // biSizeImage
        b.extend_from_slice(&0i32.to_le_bytes()); // xppm
        b.extend_from_slice(&0i32.to_le_bytes()); // yppm
        b.extend_from_slice(&0u32.to_le_bytes()); // clrUsed
        b.extend_from_slice(&0u32.to_le_bytes()); // clrImportant
        // 像素按 bottom-up 写：先最后一行。
        for row in (0..2).rev() {
            for col in 0..2 {
                let p = pixels_top_down[row * 2 + col];
                let mut px = p;
                if !any_alpha {
                    px[3] = 0;
                }
                b.extend_from_slice(&[px[2], px[1], px[0], px[3]]); // BGRA
            }
        }
        b
    }

    #[test]
    fn dib32_is_decoded_bottom_up_bgra() {
        let px = [
            [10, 20, 30, 255],
            [40, 50, 60, 255],
            [70, 80, 90, 255],
            [100, 110, 120, 255],
        ];
        let img = dib_to_rgba(&dib32(px, true)).unwrap();
        assert_eq!(img.dimensions(), (2, 2));
        assert_eq!(img.get_pixel(0, 0).0, [10, 20, 30, 255]);
        assert_eq!(img.get_pixel(1, 1).0, [100, 110, 120, 255]);
    }

    #[test]
    fn dib32_with_zero_alpha_becomes_opaque() {
        let px = [[1, 2, 3, 0]; 4];
        let img = dib_to_rgba(&dib32(px, false)).unwrap();
        assert!(img.pixels().all(|p| p.0[3] == 255));
        assert_eq!(img.get_pixel(0, 0).0, [1, 2, 3, 255]);
    }

    #[test]
    fn truncated_dib_is_rejected() {
        let mut bytes = dib32([[1, 2, 3, 255]; 4], true);
        bytes.truncate(bytes.len() - 3);
        assert!(dib_to_rgba(&bytes).is_none());
    }
}
