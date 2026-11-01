// End-user release builds (packaged with --no-default-features) are GUI
// subsystem: no console window flashes next to the app. Debug/CI builds keep
// the "console" feature so the JSON event stream stays visible.
#![cfg_attr(all(windows, not(feature = "console")), windows_subsystem = "windows")]

pub mod bridge;
pub mod instance;

use std::time::Duration;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

fn elapsed_ms() -> i64 {
    bridge::PROCESS_START
        .get()
        .map(|t| t.elapsed().as_millis() as i64)
        .unwrap_or(-1)
}


/// Best-effort Per-Monitor V2 DPI awareness, called before any window exists.
///
/// The embedded manifest (assets/app.manifest) is what normally pins this, and
/// it does work — an external probe reports PER_MONITOR_AWARE for the running
/// process. This call is only a fallback for the rare case where the manifest
/// is ignored; when the manifest already applied, Windows refuses the change
/// and it returns 0, which is expected and harmless.
///
/// Per-Monitor V2 = -4, in user32.dll (Win10 1703+). Only the setter is called,
/// never a getter: a previous shcore getter diagnostic raised an access
/// violation at startup.
#[cfg(windows)]
fn force_per_monitor_v2() {
    #[link(name = "user32")]
    extern "system" {
        fn SetProcessDpiAwarenessContext(value: *const std::ffi::c_void) -> i32;
    }
    const PER_MONITOR_AWARE_V2: i32 = -4;
    // `false` here means "already aware via the manifest", not "DPI broken".
    let forced = unsafe {
        SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2 as *const std::ffi::c_void)
    } != 0;
    println!(
        "{{\"phase\":\"ui\",\"event\":\"dpi-awareness\",\"forced-by-api\":{}}}",
        forced
    );
}

#[cfg(not(windows))]
fn force_per_monitor_v2() {}

fn main() {
    force_per_monitor_v2();
    bridge::mark_process_start();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let selftest = args.iter().any(|a| a == "--selftest");
    println!(
        "{{\"phase\":\"ui\",\"event\":\"main-enter\",\"ms\":{}}}",
        elapsed_ms()
    );

    // Single instance: a plain `imageedit.exe <image>` run (what Explorer's
    // double-click produces) hands the path to the running instance and exits,
    // so every document window shares one process — and therefore one layer
    // clipboard. Runs with any `--` flag (probes, scripts, selftest) opt out
    // completely, so automation can run beside the user's session.
    let automation = args.iter().any(|a| a.starts_with("--"));
    if !automation {
        // Explorer multi-select passes every file to ONE process launch; hand
        // each of them off independently (a partial handoff still lets the
        // leftovers open here). Files that survive beyond the first are
        // queued for the primary window's external-open timer.
        let mut leftovers: Vec<std::path::PathBuf> = Vec::new();
        for file in args.iter().filter(|a| !a.starts_with("--")) {
            if instance::try_handoff(std::path::Path::new(file)) {
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"handoff-ok\",\"file\":\"{}\"}}",
                    file.replace('\\', "/")
                );
            } else {
                leftovers.push(std::path::PathBuf::from(file));
            }
        }
        // At least one non-`--` arg is guaranteed here, so an empty leftover
        // list means every file was handed off — exit. A plain no-arg launch
        // (empty window) still becomes the primary.
        if leftovers.is_empty() && !args.is_empty() {
            return;
        }
        for extra in leftovers.iter().skip(1) {
            instance::queue_pending(extra.clone());
        }
        instance::acquire();
        // Decode the first document NOW, before Qt/QML start-up (~0.9 s to the
        // point where `start()` gets called): the two costs overlap instead of
        // stacking, which is most of the difference on a 50 MP image.
        if let Some(first) = leftovers.first() {
            bridge::prewarm_open(first);
        }
    }

    if selftest {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(20));
            eprintln!("{{\"phase\":\"ui\",\"error\":\"watchdog-timeout\"}}");
            std::process::exit(3);
        });
    }

    let mut app = QGuiApplication::new();
    println!(
        "{{\"phase\":\"ui\",\"event\":\"qapp-ready\",\"ms\":{}}}",
        elapsed_ms()
    );
    let mut engine = QQmlApplicationEngine::new();
    println!(
        "{{\"phase\":\"ui\",\"event\":\"engine-ready\",\"ms\":{}}}",
        elapsed_ms()
    );

    if let Some(engine) = engine.as_mut() {
        engine
            .on_object_creation_failed(|_, error_message| {
                let mut s = String::new();
                for c in error_message.to_string().chars() {
                    match c {
                        '"' => s.push_str("\\\""),
                        '\\' => s.push_str("\\\\"),
                        '\n' | '\r' => s.push(' '),
                        _ => s.push(c),
                    }
                }
                eprintln!("{{\"phase\":\"ui\",\"error\":\"qml-load-failed\",\"detail\":\"{}\"}}", s);
                std::process::exit(2);
            })
            .release();
    }

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/com/imageedit/qml/main.qml"));
    }
    println!(
        "{{\"phase\":\"ui\",\"event\":\"qml-loaded\",\"ms\":{}}}",
        elapsed_ms()
    );

    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
