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

    /// 以光标为中心算窗口左上角，夹进光标所在显示器的工作区（扣掉任务栏）。
    /// 单位是**物理像素**：进程是 PerMonitorV2，GetCursorPos/GetMonitorInfo
    /// 返回的都是物理值，QML 侧再除以 dpr 换成 Qt 的逻辑坐标。
    pub fn place_window_at_cursor(self: Pin<&mut Self>, width: i32, height: i32) -> QString {
        let Some((x, y)) = cursor_place(width, height) else {
            return QString::default();
        };
        QString::from(format!("{x},{y}").as_str())
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

/// Win32：光标位置 + 所在显示器工作区。`width/height` 是窗口尺寸（物理像素）。
#[cfg(windows)]
fn cursor_place(width: i32, height: i32) -> Option<(i32, i32)> {
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
        #[repr(C)]
        pub struct MonitorInfo {
            pub cb_size: u32,
            pub rc_monitor: Rect,
            pub rc_work: Rect,
            pub dw_flags: u32,
        }
        #[link(name = "user32")]
        extern "system" {
            pub fn MonitorFromPoint(point: Point, flags: u32) -> *mut c_void;
            pub fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
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
    let mut info = win::MonitorInfo {
        cb_size: std::mem::size_of::<win::MonitorInfo>() as u32,
        rc_monitor: win::Rect { left: 0, top: 0, right: 0, bottom: 0 },
        rc_work: win::Rect { left: 0, top: 0, right: 0, bottom: 0 },
        dw_flags: 0,
    };
    // SAFETY: 结构体布局与 Win32 的 POINT / MONITORINFO 一致（全 4 字节对齐
    // 的 POD），指针都指向本函数栈上的有效对象。
    unsafe {
        let monitor = win::MonitorFromPoint(pt, win::MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() {
            return None;
        }
        if win::GetMonitorInfoW(monitor, &mut info as *mut win::MonitorInfo) == 0 {
            return None;
        }
    }
    let work = info.rc_work;
    let x = (pt.x - width / 2).clamp(work.left, (work.right - width).max(work.left));
    let y = (pt.y - height / 2).clamp(work.top, (work.bottom - height).max(work.top));
    Some((x, y))
}

#[cfg(not(windows))]
fn cursor_place(width: i32, height: i32) -> Option<(i32, i32)> {
    let _ = (width, height);
    None
}
