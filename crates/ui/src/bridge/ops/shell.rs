// 窗口级外部打开事件 + 显示前的窗口几何（预置尺寸/位置）。
use super::super::*;

impl qobject::Bridge {
    pub fn start(mut self: Pin<&mut Self>) {
        // `--selftest` 自带退出；`--script` 默认留窗口（有些探针要截图/看效果），
        // 不想留下窗口时再加 `--auto-quit`。
        let selftest = std::env::args().any(|a| a == "--selftest");
        let auto_quit = selftest || std::env::args().any(|a| a == "--auto-quit");
        self.as_mut().set_auto_quit(auto_quit);

        let script = std::env::args().find_map(|a| {
            a.strip_prefix("--script=")
                .map(|value| value.to_string())
        });
        let file = std::env::args().skip(1).find(|a| !a.starts_with("--"));
        let Some(file) = file else {
            self.as_mut().set_status(QString::from("no-input"));
            println!("{{\"phase\":\"ui\",\"event\":\"no-input\"}}");
            return;
        };
        // Automation runs (any -- flag: selftest, scripts, probes) decode
        // synchronously because the script ops right after expect a loaded
        // document. Plain double-click runs decode in the background so a
        // huge image never blocks the window from showing up.
        let automation = std::env::args().any(|a| a.starts_with("--"));
        if automation {
            self.as_mut().load_path_sync(Path::new(&file));
        } else {
            self.as_mut().load_path(Path::new(&file));
        }

        if let Some(script) = script {
            self.as_mut().run_script(&script);
        }
    }

    pub fn take_external_open(self: Pin<&mut Self>) -> QString {
        let Some(path) = crate::instance::take_pending() else {
            return QString::default();
        };
        println!(
            "{{\"phase\":\"ui\",\"event\":\"external-open-consumed\",\"file\":\"{}\"}}",
            path.to_string_lossy().replace('\\', "/")
        );
        let url = file_url(&path);
        QString::from(url.as_str())
    }

    /// 只读文件头取像素尺寸（"宽x高"；读不出来返回空串）。**不解码像素**，
    /// 所以对大图也是毫秒级——窗口可以"先按最终尺寸摆好再显示"。
    ///
    /// 入参既接受 QML 的 `file:///…` URL，也接受 CLI 给的裸 Windows 路径。
    pub fn probe_image_size(self: Pin<&mut Self>, target: &QString) -> QString {
        let raw = target.to_string();
        if raw.is_empty() {
            return QString::default();
        }
        let path = if raw.starts_with("file:") {
            PathBuf::from(
                QUrl::from(raw.as_str())
                    .to_local_file_or_default()
                    .to_string(),
            )
        } else {
            PathBuf::from(&raw)
        };
        match decode::probe_path(&path) {
            Ok(info) => QString::from(format!("{}x{}", info.width, info.height).as_str()),
            Err(err) => {
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"probe-size-failed\",\"detail\":\"{}\"}}",
                    err.to_string().replace('"', "'")
                );
                QString::default()
            }
        }
    }

    /// 光标所在显示器的信息，供 QML 把**物理**坐标换算成 Qt 的**逻辑**坐标。
    ///
    /// 返回 `设备名\t偏移X\t偏移Y\t显示器物理宽\t显示器物理高`（失败=空串）。
    /// 偏移是光标相对该显示器左上角的**物理**像素。
    ///
    /// ★为什么不能直接返回"全局物理坐标 ÷ dpr"：多显示器时 Qt 的逻辑坐标系
    /// 是按每块屏各自的 dpr 分段的（屏幕 A 1920 宽@100%、屏幕 B 3440 宽@125%，
    /// 则 B 的逻辑原点是 1920 而不是 3440/1.25=2752），全局除一个 dpr 必然错位，
    /// 错位后又被夹回主屏工作区——就是"新窗口永远出现在第一块屏"。
    /// 正解：先按设备名找到**那块屏**（QScreen.name 在 Windows 上就是
    /// `\\.\DISPLAYn`），再用该屏自己的 dpr 把屏内偏移换算成逻辑坐标。
    pub fn cursor_monitor(self: Pin<&mut Self>) -> QString {
        let Some(line) = cursor_monitor_line() else {
            return QString::default();
        };
        QString::from(line.as_str())
    }

    /// 在资源管理器里定位当前文件（打开所在文件夹并选中它）。
    /// 路径可能带空格/中文，所以用 `raw_arg` 原样拼命令行：先 format 再
    /// `arg()` 会被 Rust 重新加引号，explorer 认不出 `/select,` 前缀。
    pub fn reveal_in_explorer(mut self: Pin<&mut Self>) {
        let path = self.source_path().to_string();
        if path.is_empty() {
            self.as_mut()
                .set_status(QString::from("这个窗口还没有文件（先保存一次）"));
            return;
        }
        if !Path::new(&path).exists() {
            self.as_mut()
                .set_status(QString::from("文件已被移动或删除，无法定位"));
            return;
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let line = format!("/select,\"{path}\"");
            match std::process::Command::new("explorer.exe")
                .raw_arg(line)
                .spawn()
            {
                Ok(_) => self
                    .as_mut()
                    .set_status(QString::from("已在资源管理器中定位")),
                Err(err) => self.as_mut().set_status(QString::from(
                    format!("打开资源管理器失败：{err}").as_str(),
                )),
            }
        }
        #[cfg(not(windows))]
        self.as_mut()
            .set_status(QString::from("当前平台不支持定位到文件夹"));
    }
}

/// 进程启动瞬间的光标位置（物理像素）。窗口要等 QML 起来（~0.9s）才摆位，
/// 这期间用户可能已经把鼠标挪开了 —— 那时用"当下"的光标就会摆在人手现在
/// 的位置，看起来"没有对准我双击的地方"。记住启动时的位置，**只给进程里的
/// 第一次摆位（也就是主窗口）用**，之后各窗口（新开文档窗、转发进来的图）
/// 一律用当下光标。
static CURSOR_AT_START: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

/// main() 一开始调用：把光标位置钉在"用户双击的时候"。
pub fn remember_cursor_start() {
    #[cfg(windows)]
    if let Some(pt) = current_cursor() {
        if let Ok(mut slot) = CURSOR_AT_START.lock() {
            *slot = Some(pt);
        }
    }
}

/// 现取光标物理坐标。
#[cfg(windows)]
fn current_cursor() -> Option<(i32, i32)> {
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetCursorPos(point: *mut Point) -> i32;
    }
    let mut pt = Point { x: 0, y: 0 };
    // SAFETY: POINT 是两个 i32 的 POD，指针指向本函数栈上的有效对象。
    unsafe {
        if GetCursorPos(&mut pt) == 0 {
            return None;
        }
    }
    Some((pt.x, pt.y))
}

#[cfg(not(windows))]
fn current_cursor() -> Option<(i32, i32)> {
    None
}

/// Win32：光标位置 + 所在显示器的设备名/物理尺寸。偏移以该显示器左上角为原点。
#[cfg(windows)]
struct MonitorHit {
    /// 显示器的友好名（EnumDisplayDevices 的 DeviceString，如 `S342-DP`）。
    /// **Qt 的 QScreen.name 在 Windows 上返回的就是这个**，不是 `\\.\DISPLAY1`。
    friendly: String,
    /// GDI 设备名（`\\.\DISPLAY1`），部分 Qt 版本/环境下 name 会用它。
    gdi: String,
    offset_x: i32,
    offset_y: i32,
    width: i32,
    height: i32,
    /// 显示器在虚拟桌面里的物理原点（用于同型号多屏时消歧）。
    origin_x: i32,
    origin_y: i32,
}

#[cfg(windows)]
impl MonitorHit {
    /// QML 侧解析格式：
    /// `友好名\tGDI名\t偏移X\t偏移Y\t物理宽\t物理高\t物理原点X\t物理原点Y`
    fn to_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.friendly,
            self.gdi,
            self.offset_x,
            self.offset_y,
            self.width,
            self.height,
            self.origin_x,
            self.origin_y
        )
    }
}

#[cfg(windows)]
fn cursor_monitor_info() -> Option<MonitorHit> {
    mod win {
        use std::ffi::c_void;
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct Point {
            pub x: i32,
            pub y: i32,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct Rect {
            pub left: i32,
            pub top: i32,
            pub right: i32,
            pub bottom: i32,
        }
        /// MONITORINFOEXW：在 MONITORINFO 后面多一个 WCHAR[32] 设备名。
        #[repr(C)]
        pub struct MonitorInfoEx {
            pub cb_size: u32,
            pub rc_monitor: Rect,
            /// 只为了结构体布局（GetMonitorInfoW 会写它），代码里不读。
            #[allow(dead_code)]
            pub rc_work: Rect,
            pub dw_flags: u32,
            pub sz_device: [u16; 32],
        }
        #[link(name = "user32")]
        extern "system" {
            pub fn MonitorFromPoint(point: Point, flags: u32) -> *mut c_void;
            pub fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfoEx) -> i32;
        }
        /// DISPLAY_DEVICEW：友好名在 DeviceString 里（Qt 的 QScreen.name 用它）。
        #[repr(C)]
        pub struct DisplayDeviceW {
            pub cb: u32,
            pub device_name: [u16; 32],
            pub device_string: [u16; 128],
            pub state_flags: u32,
            pub device_id: [u16; 128],
            pub device_key: [u16; 128],
        }
        #[link(name = "user32")]
        extern "system" {
            pub fn EnumDisplayDevicesW(
                device: *const u16,
                dev_num: u32,
                display_device: *mut DisplayDeviceW,
                flags: u32,
            ) -> i32;
        }
        pub const MONITOR_DEFAULTTONEAREST: u32 = 2;
    }

    // 启动快照只服务第一次摆位（主窗口），取走即失效。
    let snapped = CURSOR_AT_START.lock().ok().and_then(|mut s| s.take());
    let pt = match snapped {
        Some((x, y)) => win::Point { x, y },
        None => {
            let (x, y) = current_cursor()?;
            win::Point { x, y }
        }
    };
    let mut info = win::MonitorInfoEx {
        cb_size: std::mem::size_of::<win::MonitorInfoEx>() as u32,
        rc_monitor: win::Rect { left: 0, top: 0, right: 0, bottom: 0 },
        rc_work: win::Rect { left: 0, top: 0, right: 0, bottom: 0 },
        dw_flags: 0,
        sz_device: [0; 32],
    };
    // SAFETY: 结构体布局与 Win32 的 POINT / MONITORINFOEXW 一致（全 4 字节对齐
    // 的 POD，末尾是 32 个 u16），指针都指向本函数栈上的有效对象。
    unsafe {
        let monitor = win::MonitorFromPoint(pt, win::MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() {
            return None;
        }
        if win::GetMonitorInfoW(monitor, &mut info as *mut win::MonitorInfoEx) == 0 {
            return None;
        }
    }
    let r = info.rc_monitor;
    let gdi = String::from_utf16_lossy(&info.sz_device)
        .trim_end_matches('\0')
        .to_string();
    // 友好名：EnumDisplayDevices(显示器设备名) 的 DeviceString。
    // 拿不到就退回 GDI 名（QML 侧两种名字都认）。
    let mut dd = win::DisplayDeviceW {
        cb: std::mem::size_of::<win::DisplayDeviceW>() as u32,
        device_name: [0; 32],
        device_string: [0; 128],
        state_flags: 0,
        device_id: [0; 128],
        device_key: [0; 128],
    };
    // SAFETY: sz_device 是 32 个 u16 的以 NUL 结尾的缓冲区，dd 是栈上有效对象，
    // 结构与 Win32 的 DISPLAY_DEVICEW 一致。
    let friendly = unsafe {
        if win::EnumDisplayDevicesW(info.sz_device.as_ptr(), 0, &mut dd, 0) == 0 {
            gdi.clone()
        } else {
            let name = String::from_utf16_lossy(&dd.device_string)
                .trim_end_matches('\0')
                .to_string();
            if name.is_empty() { gdi.clone() } else { name }
        }
    };
    Some(MonitorHit {
        friendly,
        gdi,
        offset_x: pt.x - r.left,
        offset_y: pt.y - r.top,
        width: r.right - r.left,
        height: r.bottom - r.top,
        origin_x: r.left,
        origin_y: r.top,
    })
}

#[cfg(windows)]
fn cursor_monitor_line() -> Option<String> {
    cursor_monitor_info().map(|hit| hit.to_line())
}

#[cfg(not(windows))]
fn cursor_monitor_line() -> Option<String> {
    None
}
