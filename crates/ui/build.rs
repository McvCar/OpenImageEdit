use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    // cxx-qt-build's own rerun-if-changed directives do not cover the QML
    // source, so a QML-only edit rebuilt nothing and the qrc silently kept
    // embedding the stale main.qml (the "界面与最新源码不一致" trap). Watch
    // the QML tree explicitly — per FILE, not just the directory: Cargo only
    // tracks the directory's own mtime, so editing a file inside it would not
    // rerun this script.
    watch_dir_recursive("qml");
    // Rebuild when the embedded DPI manifest changes.
    println!("cargo:rerun-if-changed=assets/app.manifest");
    CxxQtBuilder::new_qml_module(
        QmlModule::new("com.imageedit")
            .qml_file("qml/main.qml")
            // Extracted components (type name = file name); main.qml sees
            // them through its `import com.imageedit`.
            .qml_files([
                "qml/editor/DocWindow.qml",
                "qml/editor/EditorContent.qml",
                "qml/panels/AdjustPanel.qml",
                "qml/panels/CanvasSurface.qml",
                "qml/panels/CanvasView.qml",
                "qml/panels/ColorPopover.qml",
                "qml/panels/ConfirmOverlay.qml",
                "qml/panels/FontPopover.qml",
                "qml/panels/LayerPanel.qml",
                "qml/panels/MarkBar.qml",
                "qml/panels/MosaicPopover.qml",
                "qml/panels/OverflowMenu.qml",
                "qml/panels/PickerPanel.qml",
                "qml/panels/PsdHintOverlay.qml",
                "qml/panels/ResizeOverlay.qml",
                "qml/panels/RulerOverlay.qml",
                "qml/panels/SelectionBar.qml",
                "qml/panels/SelectionMenu.qml",
                "qml/panels/SelectionViz.qml",
                "qml/panels/SettingsOverlay.qml",
                "qml/panels/StatusBar.qml",
                "qml/panels/TitleBar.qml",
                "qml/panels/WidthPopover.qml",
                "qml/panels/ZoomMenu.qml",
                "qml/widgets/AdjustRow.qml",
                "qml/widgets/CheckRow.qml",
                "qml/widgets/DashedRect.qml",
                "qml/widgets/EdgeGrab.qml",
                "qml/widgets/EdgeHandles.qml",
                "qml/widgets/IconButton.qml",
                "qml/widgets/MenuRow.qml",
                "qml/widgets/SpinBox.qml",
                "qml/widgets/ToolButton.qml",
                "qml/widgets/WindowEdges.qml",
            ]),
    )
        .qt_module("Quick")
        .qt_module("Qml")
        // 纯 JS 模块（阶段 4）：不是 QML 组件，走 qrc 资源。EditorContent 用
        // 相对路径 import "../js/*.js"，自动前缀 /qt/qml/com/imageedit 后解析到
        // qml/js/ 下。
        .qrc_resources([
            "qml/js/geometry.js",
            "qml/js/layers.js",
            "qml/js/marks.js",
            "qml/js/adjust.js",
            "qml/js/picker.js",
        ])
        .files(["src/bridge/mod.rs"])
        .build();

    // Embed the application icon (and version info) into the Windows exe so
    // Explorer / taskbar / shortcuts display it. The .ico is a checked-in
    // multi-size conversion of the source artwork.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.set("FileDescription", "ImageEdit");
        res.set("ProductName", "ImageEdit");
        // Embed the PerMonitorV2 DPI-awareness manifest. Without it the process
        // is DPI-unaware: at >100% Windows scaling the OS bitmap-stretches the
        // whole window (UI + image) and Qt reports devicePixelRatio == 1.0, so
        // the preview stays blurred and 100% zoom looks larger than other apps.
        // With it, Qt sees the real per-monitor DPI and renders crisp.
        res.set_manifest_file("assets/app.manifest");
        // Pin the toolkit path so winresource never probes the registry
        // (reg.exe is blocked in sandboxed builds). The MSVC target defaults
        // to the MSVC toolkit (rc.exe) already.
        res.set_toolkit_path(&find_windows_sdk_bin().to_string_lossy());
        res.compile().expect("failed to embed Windows resources");
    }
}

/// Emit `rerun-if-changed` for every file under `dir` (recursively). Cargo
/// only watches a directory's own mtime, so a directory-level directive misses
/// edits to files inside it — which silently kept stale QML in the qrc.
fn watch_dir_recursive(dir: &str) {
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                println!("cargo:rerun-if-changed={}", p.display());
            }
        }
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
