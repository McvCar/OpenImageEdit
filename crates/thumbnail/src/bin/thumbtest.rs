//! Test harness for the thumbnail COM DLL: loads the built
//! `imageedit_thumb.dll` via LoadLibrary and drives the exact chain
//! explorer.exe uses (DllGetClassObject -> IClassFactory ->
//! IInitializeWithFile -> IThumbnailProvider), then dumps the HBITMAP to a
//! PNG so the result can be verified without a shell restart.
//!
//! Also exposes `--register` / `--unregister` which invoke the DLL's own
//! DllRegisterServer / DllUnregisterServer exports (per-user HKCU keys,
//! same as regsvr32 but without needing regsvr32 itself).
//!
//! Usage: thumbtest.exe <file.psd> [out.png] [--dll <path>]
//!        thumbtest.exe --register   [--dll <path>]
//!        thumbtest.exe --unregister [--dll <path>]

use std::path::Path;

use imageedit_thumb::CLSID_THUMB_PROVIDER;
use windows::core::{GUID, Interface, PCWSTR};
use windows::Win32::Graphics::Gdi::{GetObjectW, BITMAP, HBITMAP, HGDIOBJ};
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Shell::PropertiesSystem::IInitializeWithFile;
use windows::Win32::UI::Shell::{IThumbnailProvider, WTS_ALPHATYPE, WTSAT_ARGB};

type DllGetClassObjectFn = unsafe extern "system" fn(
    *const GUID,
    *const GUID,
    *mut *mut core::ffi::c_void,
) -> windows::core::HRESULT;

type DllRegisterFn = unsafe extern "system" fn() -> windows::core::HRESULT;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!(
            "usage: thumbtest.exe <file.psd> [out.png] [--dll <path>]\n       thumbtest.exe --register|--unregister [--dll <path>]"
        );
        std::process::exit(2);
    }
    let mut dll = String::from("imageedit_thumb.dll");
    let mut out = String::from("thumb_out.png");
    let mut dll_flag = false;
    let mut mode = Mode::Render;
    args.retain(|a| {
        if a == "--dll" {
            dll_flag = true;
            return false;
        }
        if dll_flag {
            dll = a.clone();
            dll_flag = false;
            return false;
        }
        if a == "--register" {
            mode = Mode::Register;
            return false;
        }
        if a == "--unregister" {
            mode = Mode::Unregister;
            return false;
        }
        true
    });

    let dll_path = Path::new(&dll);
    match mode {
        Mode::Register => exit_with(call_register(dll_path, true), "register"),
        Mode::Unregister => exit_with(call_register(dll_path, false), "unregister"),
        Mode::Render => {
            let input = match args.first() {
                Some(p) => p.clone(),
                None => {
                    eprintln!("missing input file");
                    std::process::exit(2);
                }
            };
            if let Some(o) = args.get(1) {
                out = o.clone();
            }
            match run(dll_path, Path::new(&input), Path::new(&out)) {
                Ok((w, h)) => println!("OK: {w}x{h} -> {out}"),
                Err(e) => {
                    eprintln!("FAILED: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}

enum Mode {
    Render,
    Register,
    Unregister,
}

fn exit_with(hr: windows::core::HRESULT, what: &str) {
    if hr.is_ok() {
        println!("{what}: OK");
    } else {
        eprintln!("{what}: FAILED {hr:?}");
        std::process::exit(1);
    }
}

fn call_register(dll_path: &Path, register: bool) -> windows::core::HRESULT {
    unsafe {
        let lib = match load_dll(dll_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{e}");
                return windows::core::HRESULT(-1);
            }
        };
        let name = if register {
            windows::core::s!("DllRegisterServer")
        } else {
            windows::core::s!("DllUnregisterServer")
        };
        let Some(addr) = GetProcAddress(lib, name) else {
            eprintln!("export not found");
            return windows::core::HRESULT(-1);
        };
        let f: DllRegisterFn = std::mem::transmute(addr);
        f()
    }
}

unsafe fn load_dll(dll_path: &Path) -> Result<windows::Win32::Foundation::HMODULE, String> {
    let dll_wide: Vec<u16> = dll_path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    LoadLibraryW(PCWSTR(dll_wide.as_ptr())).map_err(|e| format!("LoadLibraryW: {e}"))
}

fn run(dll_path: &Path, input: &Path, out_path: &Path) -> Result<(u32, u32), String> {
    unsafe {
        let dll_wide: Vec<u16> = dll_path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let lib = LoadLibraryW(PCWSTR(dll_wide.as_ptr())).map_err(|e| format!("LoadLibraryW: {e}"))?;

        let addr = GetProcAddress(lib, windows::core::s!("DllGetClassObject"))
            .ok_or("DllGetClassObject not found")?;
        let get_class_object: DllGetClassObjectFn = std::mem::transmute(addr);

        let factory_iid = IClassFactory::IID;
        let mut factory_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let hr = get_class_object(&CLSID_THUMB_PROVIDER, &factory_iid, &mut factory_ptr);
        if hr.is_err() {
            return Err(format!("DllGetClassObject: {hr:?}"));
        }
        let factory: IClassFactory = std::mem::transmute(factory_ptr);

        let init: IInitializeWithFile = factory
            .CreateInstance(None)
            .map_err(|e| format!("CreateInstance: {e}"))?;
        let file_wide: Vec<u16> = input
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        init.Initialize(PCWSTR(file_wide.as_ptr()), 0)
            .map_err(|e| format!("Initialize: {e}"))?;

        let provider: IThumbnailProvider =
            init.cast().map_err(|e| format!("QI IThumbnailProvider: {e}"))?;
        let mut hbmp = HBITMAP::default();
        let mut fmt = WTS_ALPHATYPE::default();
        provider
            .GetThumbnail(256, &mut hbmp, &mut fmt)
            .map_err(|e| format!("GetThumbnail: {e}"))?;
        if fmt != WTSAT_ARGB {
            return Err(format!("unexpected alpha type: {fmt:?}"));
        }

        // Read the DIB back through GetObjectW and save it as PNG.
        let mut bmp = BITMAP::default();
        let got = GetObjectW(
            HGDIOBJ(hbmp.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bmp as *mut BITMAP as *mut core::ffi::c_void),
        );
        if got == 0 {
            return Err("GetObjectW failed".into());
        }
        let w = bmp.bmWidth as u32;
        let h = bmp.bmHeight.unsigned_abs();
        if w == 0 || h == 0 || bmp.bmBits.is_null() {
            return Err("invalid DIB info".into());
        }
        let row_bytes = bmp.bmWidthBytes.max(0) as usize;
        let mut img = image::RgbaImage::new(w, h);
        let bits = bmp.bmBits as *const u8;
        // Top-down DIB (negative height at creation). If the height came back
        // positive the rows are bottom-up; handle both.
        let top_down = bmp.bmHeight < 0;
        for y in 0..h as usize {
            let src_y = if top_down { y } else { h as usize - 1 - y };
            let row = std::slice::from_raw_parts(bits.add(src_y * row_bytes), w as usize * 4);
            for x in 0..w as usize {
                let (b, g, r, a) = (row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]);
                // Unpremultiply for PNG display.
                let (r, g, b) = if a > 0 {
                    (
                        ((r as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
                        ((g as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
                        ((b as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
                    )
                } else {
                    (0, 0, 0)
                };
                img.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, a]));
            }
        }
        img.save(out_path).map_err(|e| format!("save: {e}"))?;
        Ok((w, h))
    }
}
