use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    // cxx-qt-build's own rerun-if-changed directives do not cover the QML
    // source, so a QML-only edit rebuilt nothing and the qrc silently kept
    // embedding the stale main.qml (the "界面与最新源码不一致" trap). Watch
    // the QML tree explicitly.
    println!("cargo:rerun-if-changed=qml");
    CxxQtBuilder::new_qml_module(
        QmlModule::new("com.imageedit")
            .qml_file("qml/main.qml")
            // Extracted components (type name = file name); main.qml sees
            // them through its `import com.imageedit`.
            .qml_files([
                "qml/editor/DocWindow.qml",
                "qml/editor/EditorContent.qml",
                "qml/panels/AdjustPanel.qml",
                "qml/panels/ConfirmOverlay.qml",
                "qml/panels/LayerPanel.qml",
                "qml/panels/OverflowMenu.qml",
                "qml/panels/PickerPanel.qml",
                "qml/panels/PsdHintOverlay.qml",
                "qml/panels/ResizeOverlay.qml",
                "qml/panels/SettingsOverlay.qml",
                "qml/panels/StatusBar.qml",
                "qml/panels/ZoomMenu.qml",
                "qml/widgets/AdjustRow.qml",
                "qml/widgets/CheckRow.qml",
                "qml/widgets/DashedRect.qml",
                "qml/widgets/EdgeGrab.qml",
                "qml/widgets/EdgeHandles.qml",
                "qml/widgets/IconButton.qml",
                "qml/widgets/MenuRow.qml",
                "qml/widgets/ToolButton.qml",
                "qml/widgets/WindowEdges.qml",
            ]),
    )
        .qt_module("Quick")
        .qt_module("Qml")
        .files(["src/bridge.rs"])
        .build();

    // Embed the application icon (and version info) into the Windows exe so
    // Explorer / taskbar / shortcuts display it. The .ico is a checked-in
    // multi-size conversion of the source artwork.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.set("FileDescription", "ImageEdit");
        res.set("ProductName", "ImageEdit");
        // Pin the toolkit path so winresource never probes the registry
        // (reg.exe is blocked in sandboxed builds). The MSVC target defaults
        // to the MSVC toolkit (rc.exe) already.
        res.set_toolkit_path(&find_windows_sdk_bin().to_string_lossy());
        res.compile().expect("failed to embed Windows resources");
    }
}

/// Newest `...\Windows Kits\10\bin\<ver>\x64` directory (home of rc.exe).
fn find_windows_sdk_bin() -> std::path::PathBuf {
    let root = std::path::Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut candidates: Vec<(Vec<u64>, std::path::PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let rc = path.join("x64").join("rc.exe");
            if rc.is_file() {
                let ver = entry
                    .file_name()
                    .to_string_lossy()
                    .split('.')
                    .map(|s| s.parse::<u64>().unwrap_or(0))
                    .collect();
                candidates.push((ver, path.join("x64")));
            }
        }
    }
    candidates.sort();
    candidates
        .pop()
        .map(|(_, path)| path)
        // Fall back to winresource's own (registry-based) lookup.
        .unwrap_or_default()
}
