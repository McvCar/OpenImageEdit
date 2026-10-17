pub mod bridge;

use std::time::Duration;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

fn elapsed_ms() -> i64 {
    bridge::PROCESS_START
        .get()
        .map(|t| t.elapsed().as_millis() as i64)
        .unwrap_or(-1)
}

fn main() {
    bridge::mark_process_start();

    let selftest = std::env::args().any(|a| a == "--selftest");
    println!(
        "{{\"phase\":\"ui\",\"event\":\"main-enter\",\"ms\":{}}}",
        elapsed_ms()
    );

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
            .on_object_creation_failed(|_, _| {
                eprintln!("{{\"phase\":\"ui\",\"error\":\"qml-load-failed\"}}");
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
