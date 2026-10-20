//! ImageEdit Explorer thumbnail provider — an in-proc COM DLL that renders
//! PSD previews (flattened composite) for Windows Explorer thumbnails.
//!
//! Design notes:
//! - This DLL is loaded into `explorer.exe`, so it must never crash the
//!   host. PSD decoding goes through `imageedit-decode`'s guarded pipeline
//!   (`validate_psd_sections` pre-bounds every offset the panicky `psd`
//!   crate touches), and everything user-facing is additionally wrapped in
//!   `catch_unwind`. Build the release DLL with the `thumb` profile
//!   (`panic = "unwind"`) so `catch_unwind` can actually work:
//!   `cargo build -p imageedit-thumbnail --profile thumb`
//! - Registration is per-user (`HKCU\Software\Classes`) so `regsvr32`
//!   needs no elevation; NSIS writes the very same keys itself.
//! - Thumbnails are premultiplied 32bpp top-down DIBs (`WTSAT_ARGB`).

use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use decode::{decode_psd_bytes, DecodeOptions, Decoded};

use windows::core::{implement, Error, GUID, HRESULT, IUnknown, Interface, PCWSTR, Result, BOOL};
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, S_OK};
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IInitializeWithFile, IInitializeWithFile_Impl,
};
use windows::Win32::UI::Shell::{
    IThumbnailProvider, IThumbnailProvider_Impl, WTS_ALPHATYPE, WTSAT_ARGB,
};
/// CLSID of `ImageEdit.PsdThumbnailProvider`. Keep in sync with
/// `installer/imageedit.nsi` (THUMB_CLSID).
pub const CLSID_THUMB_PROVIDER_STR: &str = "{43B34785-3F16-49F5-BAB6-069B487ED407}";
pub const CLSID_THUMB_PROVIDER: GUID =
    GUID::from_u128(0x43b34785_3f16_49f5_bab6_069b487ed407);

/// Shell extension category for `IThumbnailProvider` (fixed by the system).
const THUMB_SHELLEX_CATEGORY: &str = "{e357fccd-a995-4577-b039-fa759f30b17e}";

/// Decode ceiling: PSDs whose canvas exceeds this many pixels are rejected
/// instead of allocating. `explorer.exe` must stay responsive.
const MAX_PIXELS: u64 = 120_000_000;
/// Files larger than this are almost certainly not worth thumbnailing.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
/// Upper bound for the decode resolution (Lanczos3 target); the box filter
/// below then shrinks to the exact requested size.
const MAX_DECODE_EDGE: u32 = 1024;

// ---------------------------------------------------------------------------
// COM object
// ---------------------------------------------------------------------------

#[implement(IInitializeWithFile, IThumbnailProvider)]
struct Provider {
    path: RefCell<Option<PathBuf>>,
}

impl Provider {
    fn new() -> Self {
        Self {
            path: RefCell::new(None),
        }
    }
}

impl IInitializeWithFile_Impl for Provider_Impl {
    fn Initialize(&self, pszfilepath: &PCWSTR, _grfmode: u32) -> Result<()> {
        if pszfilepath.is_null() {
            return Err(Error::from_hresult(HRESULT(0x8000_4005u32 as i32))); // E_POINTER
        }
        let s = unsafe { pszfilepath.to_string() }.map_err(|_| Error::from_hresult(E_FAIL))?;
        *self.path.borrow_mut() = Some(PathBuf::from(s));
        Ok(())
    }
}

impl IThumbnailProvider_Impl for Provider_Impl {
    // Note: the interface has a single `cx` — per thumbcache.h it is the
    // maximum size for BOTH x and y (aspect preserved).
    fn GetThumbnail(
        &self,
        cx: u32,
        phbmp: *mut HBITMAP,
        pdwalpha: *mut WTS_ALPHATYPE,
    ) -> Result<()> {
        let path = self
            .path
            .borrow()
            .clone()
            .ok_or_else(|| Error::from_hresult(E_FAIL))?;

        // Decode + GDI work inside catch_unwind: any residual panic in the
        // dependency chain must not abort explorer.exe (requires the `thumb`
        // profile; `validate_psd_sections` is the first line of defence).
        let result = catch_unwind(AssertUnwindSafe(|| render_thumbnail(&path, cx)));
        match result {
            Ok(Ok(hbmp)) => unsafe {
                if phbmp.is_null() || pdwalpha.is_null() {
                    eprintln!("[thumb] out ptrs null");
                    return Err(Error::from_hresult(E_FAIL));
                }
                *phbmp = hbmp;
                *pdwalpha = WTSAT_ARGB;
                Ok(())
            },
            Ok(Err(e)) => {
                eprintln!("[thumb] render error: {e}");
                Err(Error::from_hresult(E_FAIL))
            }
            Err(_) => {
                eprintln!("[thumb] render panicked");
                Err(Error::from_hresult(E_FAIL))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Class factory
// ---------------------------------------------------------------------------

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        punkouter: windows::core::Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        // Ref::ok() fails with E_POINTER for a null in-param, so success
        // means an aggregating outer was supplied — we don't support it.
        if punkouter.ok().is_ok() {
            return Err(Error::from_hresult(CLASS_E_NOAGGREGATION));
        }
        let unknown: IUnknown = Provider::new().into();
        unsafe { unknown.query(riid, ppvobject) }.ok()
    }

    fn LockServer(&self, _flock: BOOL) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

#[no_mangle]
extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    unsafe {
        if rclsid.is_null() || riid.is_null() || ppv.is_null() {
            return HRESULT(0x8000_4003u32 as i32); // E_INVALIDARG
        }
        if (*rclsid) != CLSID_THUMB_PROVIDER {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(riid, ppv)
    }
}

#[no_mangle]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    // No global state, no server lock: always safe to unload.
    S_OK
}

/// Self-registration under HKCU\Software\Classes (no admin required).
/// NSIS writes the identical keys; this exists for `regsvr32` users and CI.
#[no_mangle]
extern "system" fn DllRegisterServer() -> HRESULT {
    catch_unwind(register).unwrap_or_else(|_| E_FAIL)
}

#[no_mangle]
extern "system" fn DllUnregisterServer() -> HRESULT {
    catch_unwind(unregister).unwrap_or_else(|_| E_FAIL)
}

// ---------------------------------------------------------------------------
// Registration helpers (winreg)
// ---------------------------------------------------------------------------

fn register() -> HRESULT {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let dll_path = match current_dll_path() {
        Some(p) => p,
        None => return E_FAIL,
    };
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let clsid_path = format!(r"Software\Classes\CLSID\{CLSID_THUMB_PROVIDER_STR}\InprocServer32");
    let (key, _) = match hkcu.create_subkey(&clsid_path) {
        Ok(k) => k,
        Err(_) => return E_FAIL,
    };
    if key.set_value("", &dll_path).is_err() {
        return E_FAIL;
    }
    if key.set_value("ThreadingModel", &"Apartment").is_err() {
        return E_FAIL;
    }
    let shellex_path = format!(r"Software\Classes\.psd\ShellEx\{THUMB_SHELLEX_CATEGORY}");
    let (key, _) = match hkcu.create_subkey(&shellex_path) {
        Ok(k) => k,
        Err(_) => return E_FAIL,
    };
    if key.set_value("", &CLSID_THUMB_PROVIDER_STR).is_err() {
        return E_FAIL;
    }
    S_OK
}

fn unregister() -> HRESULT {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    // Only remove what we own; ignore "not found". The CLSID key has a
    // single InprocServer32 subkey, so two plain deletes replace delete_tree.
    let _ = hkcu.delete_subkey(format!(
        r"Software\Classes\CLSID\{CLSID_THUMB_PROVIDER_STR}\InprocServer32"
    ));
    let _ = hkcu.delete_subkey(format!(r"Software\Classes\CLSID\{CLSID_THUMB_PROVIDER_STR}"));
    let _ = hkcu.delete_subkey(format!(r"Software\Classes\.psd\ShellEx\{THUMB_SHELLEX_CATEGORY}"));
    S_OK
}

fn current_dll_path() -> Option<String> {
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    unsafe {
        let mut hmodule = HMODULE::default();
        // Anchor on this DLL's own code so we get its module, not the host's.
        let anchor = PCWSTR(DllGetClassObject as *const () as *const u16);
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            anchor,
            &mut hmodule,
        )
        .is_err()
        {
            return None;
        }
        let mut buf = [0u16; 512];
        let len = GetModuleFileNameW(Some(hmodule), &mut buf);
        if len == 0 || len as usize >= buf.len() {
            return None;
        }
        String::from_utf16(&buf[..len as usize]).ok()
    }
}

// ---------------------------------------------------------------------------
// Thumbnail rendering
// ---------------------------------------------------------------------------

fn render_thumbnail(path: &PathBuf, cx: u32) -> std::result::Result<HBITMAP, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_FILE_BYTES {
        return Err("file too large".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;

    let options = DecodeOptions {
        max_pixels: Some(MAX_PIXELS),
        max_dimension: Some(MAX_DECODE_EDGE),
    };
    let decoded: Decoded = decode_psd_bytes(&bytes, options).map_err(|e| e.to_string())?;
    let img = decoded.image;

    // Fit inside the requested square (cx = max edge); never upscale.
    let (w, h) = (img.width(), img.height());
    let cx = cx.max(1);
    let scale = (cx as f64 / w.max(h) as f64).min(1.0);
    let (tw, th) = (
        ((w as f64 * scale).round() as u32).clamp(1, cx),
        ((h as f64 * scale).round() as u32).clamp(1, cx),
    );
    let thumb = if (tw, th) == (w, h) { img } else { resize_box(&img, tw, th) };

    create_argb_dib(&thumb)
}

/// Simple box (area-average) downscale for straight-alpha RGBA8.
fn resize_box(src: &image::RgbaImage, tw: u32, th: u32) -> image::RgbaImage {
    let (sw, sh) = (src.width() as u64, src.height() as u64);
    let mut out = image::RgbaImage::new(tw, th);
    for ty in 0..th {
        let y0 = (ty as u64 * sh) / th as u64;
        let y1 = (((ty + 1) as u64 * sh) / th as u64).max(y0 + 1);
        for tx in 0..tw {
            let x0 = (tx as u64 * sw) / tw as u64;
            let x1 = (((tx + 1) as u64 * sw) / tw as u64).max(x0 + 1);
            let area = ((y1 - y0) * (x1 - x0)) as u32;
            let mut acc = [0u32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = src.get_pixel(x as u32, y as u32).0;
                    acc[0] += p[0] as u32;
                    acc[1] += p[1] as u32;
                    acc[2] += p[2] as u32;
                    acc[3] += p[3] as u32;
                }
            }
            let px = [
                (acc[0] / area) as u8,
                (acc[1] / area) as u8,
                (acc[2] / area) as u8,
                (acc[3] / area) as u8,
            ];
            out.put_pixel(tx, ty, image::Rgba(px));
        }
    }
    out
}

/// Copy straight-alpha RGBA into a 32bpp premultiplied top-down DIB section.
fn create_argb_dib(img: &image::RgbaImage) -> std::result::Result<HBITMAP, String> {
    let (w, h) = (img.width(), img.height());
    let size = w as usize * h as usize * 4;
    if size == 0 {
        return Err("empty image".into());
    }

    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w as i32;
    bmi.bmiHeader.biHeight = -(h as i32); // negative = top-down rows
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = 0; // BI_RGB
    bmi.bmiHeader.biSizeImage = size as u32;

    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let hbmp = unsafe {
        CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).map_err(|e| e.to_string())?
    };
    if bits.is_null() {
        return Err("CreateDIBSection returned null bits".into());
    }

    let dst = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, size) };
    for (i, px) in img.pixels().enumerate() {
        let [r, g, b, a] = px.0;
        let (r, g, b) = premultiply(r, g, b, a);
        let o = i * 4;
        // Little-endian 32bpp: BGRA in memory.
        dst[o] = b;
        dst[o + 1] = g;
        dst[o + 2] = r;
        dst[o + 3] = a;
    }
    Ok(hbmp)
}

fn premultiply(r: u8, g: u8, b: u8, a: u8) -> (u8, u8, u8) {
    fn mul(c: u8, a: u8) -> u8 {
        ((c as u32 * a as u32 + 127) / 255) as u8
    }
    (mul(r, a), mul(g, a), mul(b, a))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> PathBuf {
        // crates/thumbnail -> repo root
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    #[test]
    fn render_sample_psd() {
        let p = repo_root().join("tmp/sample-design-raw.psd");
        if !p.exists() {
            return; // sample asset not present in CI checkouts
        }
        render_thumbnail(&p, 256).unwrap_or_else(|e| panic!("render failed: {e}"));
    }

    #[test]
    fn corrupt_psd_is_rejected_not_panicking() {
        let dir = std::env::temp_dir();
        let p = dir.join("ie_thumb_corrupt.psd");
        let mut data = vec![0x38u8, 0x42, 0x50, 0x53]; // 8BPS magic
        data.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // rest of header
        data.extend_from_slice(&[0, 1]); // channels
        data.extend_from_slice(&[0, 0, 0, 64]); // huge height
        data.extend_from_slice(&[0, 0, 0, 64]); // huge width
        data.extend_from_slice(&[0, 8, 0, 3]); // depth, mode
        data.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]); // absurd section length
        std::fs::write(&p, &data).unwrap();
        assert!(render_thumbnail(&p, 256).is_err());
        let _ = std::fs::remove_file(&p);
    }
}
