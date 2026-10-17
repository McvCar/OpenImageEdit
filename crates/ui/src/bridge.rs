use core::pin::Pin;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use cxx_qt_lib::{QString, QUrl};
use decode::{decode_path, encode, format_from_path, DecodeOptions, Format};
use editcore::{Document, Layer, Rect, Selection, Size, Transform};
use image::RgbaImage;

pub static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// Session ids make preview temp filenames unique per document, which is
/// required once multiple windows (documents) live in the same process.
static SESSION_SEQ: AtomicU32 = AtomicU32::new(0);

pub fn mark_process_start() {
    let _ = PROCESS_START.set(Instant::now());
}

fn elapsed_ms() -> i32 {
    PROCESS_START
        .get()
        .map(|t| t.elapsed().as_millis().min(i32::MAX as u128) as i32)
        .unwrap_or(-1)
}

const MAX_PREVIEW_EDGE: u32 = 4096;
const MAX_DISPLAY_EDGE: u32 = 32767;
const MIN_ZOOM: f64 = 0.02;
const MAX_ZOOM: f64 = 16.0;
const ZOOM_STEP: f64 = 1.25;
const SAVE_QUALITY: u8 = 92;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, status)]
        #[qproperty(QString, preview_url)]
        #[qproperty(i32, image_width)]
        #[qproperty(i32, image_height)]
        #[qproperty(i32, display_width)]
        #[qproperty(i32, display_height)]
        #[qproperty(i32, zoom_percent)]
        #[qproperty(bool, fit_mode)]
        #[qproperty(bool, can_undo)]
        #[qproperty(bool, can_redo)]
        #[qproperty(i32, decode_ms)]
        #[qproperty(i32, shown_ms)]
        #[qproperty(bool, auto_quit)]
        #[qproperty(bool, load_failed, cxx_name = "loadFailed")]
        #[qproperty(QString, default_save_url, cxx_name = "defaultSaveUrl")]
        #[qproperty(i32, layer_count)]
        #[qproperty(i32, selected_layer)]
        #[qproperty(QString, layers_json)]
        #[qproperty(bool, dirty)]
        #[qproperty(QString, file_name, cxx_name = "fileName")]
        #[qproperty(QString, file_size_str, cxx_name = "fileSizeStr")]
        #[qproperty(QString, checker_light, cxx_name = "checkerLight")]
        #[qproperty(QString, checker_dark, cxx_name = "checkerDark")]
        #[qproperty(bool, open_in_new_window, cxx_name = "openInNewWindow")]
        #[namespace = "imageedit"]
        type Bridge = super::BridgeRust;

        #[qinvokable]
        fn start(self: Pin<&mut Self>);

        #[qinvokable]
        fn image_shown(self: Pin<&mut Self>);

        #[qinvokable]
        fn mark(self: Pin<&mut Self>, name: &QString);

        #[qinvokable]
        fn open_file(self: Pin<&mut Self>, url: &QUrl);

        // ---- 图片浏览 / 设置 ----

        /// Open the previous (-1) or next (+1) image in the same folder.
        #[qinvokable]
        fn navigate(self: Pin<&mut Self>, dir: i32);

        /// Register the app in the Windows "Open with" list and open the
        /// system default-apps settings page (the user picks the default).
        #[qinvokable]
        fn register_as_viewer(self: Pin<&mut Self>);

        #[qinvokable]
        fn update_checker_light(self: Pin<&mut Self>, value: &QString);

        #[qinvokable]
        fn update_checker_dark(self: Pin<&mut Self>, value: &QString);

        #[qinvokable]
        fn update_open_in_new_window(self: Pin<&mut Self>, value: bool);

        #[qinvokable]
        fn set_viewport(self: Pin<&mut Self>, width: i32, height: i32);

        #[qinvokable]
        fn zoom_in(self: Pin<&mut Self>);

        #[qinvokable]
        fn zoom_out(self: Pin<&mut Self>);

        #[qinvokable]
        fn zoom_fit(self: Pin<&mut Self>);

        #[qinvokable]
        fn zoom_actual(self: Pin<&mut Self>);

        #[qinvokable]
        fn rotate_cw(self: Pin<&mut Self>);

        #[qinvokable]
        fn rotate_ccw(self: Pin<&mut Self>);

        #[qinvokable]
        fn flip_horizontal(self: Pin<&mut Self>);

        #[qinvokable]
        fn flip_vertical(self: Pin<&mut Self>);

        #[qinvokable]
        fn undo(self: Pin<&mut Self>);

        #[qinvokable]
        fn redo(self: Pin<&mut Self>);

        #[qinvokable]
        fn save_edited(self: Pin<&mut Self>);

        #[qinvokable]
        fn save_as(self: Pin<&mut Self>, url: &QUrl);

        #[qinvokable]
        fn crop_to(self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32);

        #[qinvokable]
        fn resize_to(self: Pin<&mut Self>, width: i32, height: i32);

        /// Change only the canvas resolution: the image is NOT resampled;
        /// content keeps its pixel size and is re-anchored to the center of
        /// the new canvas (clipped when shrinking, padded when growing).
        #[qinvokable]
        fn resize_canvas(self: Pin<&mut Self>, width: i32, height: i32);

        #[qinvokable]
        fn reset_crop(self: Pin<&mut Self>);

        // ---- 选区图层 ----

        #[qinvokable]
        fn extract_selection(self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32);

        #[qinvokable]
        fn layer_name(self: Pin<&mut Self>, index: i32) -> QString;

        #[qinvokable]
        fn layer_visible(self: Pin<&mut Self>, index: i32) -> bool;

        #[qinvokable]
        fn select_layer(self: Pin<&mut Self>, index: i32);

        #[qinvokable]
        fn toggle_selected_visible(self: Pin<&mut Self>);

        #[qinvokable]
        fn set_layer_visible_at(self: Pin<&mut Self>, index: i32, visible: bool);

        #[qinvokable]
        fn delete_selected_layer(self: Pin<&mut Self>);

        #[qinvokable]
        fn move_selected_up(self: Pin<&mut Self>);

        #[qinvokable]
        fn move_selected_down(self: Pin<&mut Self>);

        #[qinvokable]
        fn reorder_layer(self: Pin<&mut Self>, from: i32, to: i32);

        #[qinvokable]
        fn set_layer_locked(self: Pin<&mut Self>, index: i32, locked: bool);

        #[qinvokable]
        fn set_base_locked(self: Pin<&mut Self>, locked: bool);

        #[qinvokable]
        #[qinvokable]
        fn delete_base_layer(self: Pin<&mut Self>);

        #[qinvokable]
        fn toggle_base_visible(self: Pin<&mut Self>);

        #[qinvokable]
        fn translate_selected(self: Pin<&mut Self>, dx: i32, dy: i32);

        // ---- 剪贴板（跨文档） ----

        #[qinvokable]
        fn copy_selection_pixels(self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32);

        #[qinvokable]
        fn cut_selection_pixels(self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32);

        #[qinvokable]
        fn erase_selection_pixels(self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32);

        #[qinvokable]
        fn copy_selected_layer(self: Pin<&mut Self>);

        #[qinvokable]
        fn cut_selected_layer(self: Pin<&mut Self>);

        #[qinvokable]
        fn paste_clipboard(self: Pin<&mut Self>);

        // ---- 图层属性 ----

        #[qinvokable]
        fn set_selected_opacity(self: Pin<&mut Self>, opacity: f32, commit: bool);

        #[qinvokable]
        fn rename_selected_layer(self: Pin<&mut Self>, name: &QString);

        #[qinvokable]
        fn scale_selected_layer(self: Pin<&mut Self>, width: i32, height: i32, keep_ratio: bool);

        // ---- 移动图层拖拽实时预览 ----

        #[qinvokable]
        fn drag_translate_begin(self: Pin<&mut Self>);

        #[qinvokable]
        fn drag_translate_update(self: Pin<&mut Self>, dx: i32, dy: i32);

        #[qinvokable]
        fn drag_translate_end(self: Pin<&mut Self>, commit: bool);

        // ---- 图层缩放拖拽实时预览 ----

        #[qinvokable]
        fn scale_layer_begin(self: Pin<&mut Self>);

        #[qinvokable]
        fn scale_layer_update(self: Pin<&mut Self>, w: i32, h: i32, ox: i32, oy: i32);

        #[qinvokable]
        fn scale_layer_end(
            self: Pin<&mut Self>,
            commit: bool,
            w: i32,
            h: i32,
            ox: i32,
            oy: i32,
        );
    }
}

/// User preferences persisted across runs (plain key=value file under
/// %APPDATA%/ImageEdit so no extra dependency is needed).
struct AppConfig {
    checker_light: String,
    checker_dark: String,
    open_in_new_window: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            checker_light: "#ffffff".to_string(),
            checker_dark: "#cccccc".to_string(),
            open_in_new_window: false,
        }
    }
}

impl AppConfig {
    fn path() -> Option<PathBuf> {
        std::env::var_os("APPDATA")
            .map(|root| PathBuf::from(root).join("ImageEdit").join("settings.ini"))
    }

    fn load() -> Self {
        let mut cfg = Self::default();
        if let Some(path) = Self::path() {
            if let Ok(text) = std::fs::read_to_string(path) {
                for line in text.lines() {
                    let (key, value) = match line.split_once('=') {
                        Some((k, v)) => (k.trim(), v.trim()),
                        None => continue,
                    };
                    match key {
                        "checker_light" if !value.is_empty() => {
                            cfg.checker_light = value.to_string()
                        }
                        "checker_dark" if !value.is_empty() => cfg.checker_dark = value.to_string(),
                        "open_in_new_window" => cfg.open_in_new_window = value == "true",
                        _ => {}
                    }
                }
            }
        }
        cfg
    }

    fn save(&self) {
        if let Some(path) = Self::path() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let text = format!(
                "checker_light={}\nchecker_dark={}\nopen_in_new_window={}\n",
                self.checker_light, self.checker_dark, self.open_in_new_window
            );
            let _ = std::fs::write(path, text);
        }
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub struct BridgeRust {
    status: QString,
    preview_url: QString,
    image_width: i32,
    image_height: i32,
    display_width: i32,
    display_height: i32,
    zoom_percent: i32,
    fit_mode: bool,
    can_undo: bool,
    can_redo: bool,
    decode_ms: i32,
    shown_ms: i32,
    auto_quit: bool,
    load_failed: bool,
    default_save_url: QString,
    layer_count: i32,
    selected_layer: i32,
    layers_json: QString,
    dirty: bool,
    /// Loaded file's display name and human-readable size for the status bar.
    file_name: QString,
    file_size_str: QString,
    /// User preferences (checkerboard colors, open-in-new-window behavior).
    checker_light: QString,
    checker_dark: QString,
    open_in_new_window: bool,
    paste_counter: RefCell<u32>,
    /// Opacity of the selected layer before the current live slider drag, so
    /// the committed history op can record the true pre-drag value.
    opacity_live: RefCell<Option<f32>>,
    /// Offset of the selected layer when a drag started; `None` while not
    /// dragging. `drag_translate_update` mutates the offset without touching
    /// history, `drag_translate_end` commits a single history op.
    drag_start_offset: RefCell<Option<(i32, i32)>>,
    /// Image + offset of the selected layer when a handle-driven scale
    /// started; live updates resample without history, end commits one op.
    scale_start: RefCell<Option<(RgbaImage, (i32, i32))>>,
    session: RefCell<Option<Session>>,
    viewport: RefCell<Size>,
}

impl Default for BridgeRust {
    fn default() -> Self {
        let cfg = AppConfig::load();
        Self {
            status: QString::default(),
            preview_url: QString::default(),
            image_width: 0,
            image_height: 0,
            display_width: 0,
            display_height: 0,
            zoom_percent: 100,
            fit_mode: true,
            can_undo: false,
            can_redo: false,
            decode_ms: 0,
            shown_ms: 0,
            auto_quit: false,
            load_failed: false,
            default_save_url: QString::default(),
            layer_count: 0,
            selected_layer: -1,
            layers_json: QString::default(),
            dirty: false,
            file_name: QString::default(),
            file_size_str: QString::default(),
            checker_light: QString::from(cfg.checker_light.as_str()),
            checker_dark: QString::from(cfg.checker_dark.as_str()),
            open_in_new_window: cfg.open_in_new_window,
            paste_counter: RefCell::new(0),
            opacity_live: RefCell::new(None),
            drag_start_offset: RefCell::new(None),
            scale_start: RefCell::new(None),
            session: RefCell::new(None),
            viewport: RefCell::new(Size::new(0, 0)),
        }
    }
}

/// Process-global clipboard (per-window `RefCell` fields cannot share state
/// across the multi-window Bridges). Holds a full `Layer`, so pixels survive
/// opening a different document (cross-image copy/paste).
fn clipboard() -> &'static std::sync::Mutex<Option<Layer>> {
    static CLIPBOARD: std::sync::OnceLock<std::sync::Mutex<Option<Layer>>> =
        std::sync::OnceLock::new();
    CLIPBOARD.get_or_init(|| std::sync::Mutex::new(None))
}

struct Session {
    id: u32,
    doc: Document,
    source: Option<PathBuf>,
    zoom: f64,
    fit: bool,
    render_count: u32,
    dirty: bool,
    /// UI lock for the base row (blocks delete). Plain flag, not undoable.
    base_locked: bool,
}

struct Rendered {
    url: String,
    display: Size,
    preview: Size,
}

enum Refresh {
    Ok {
        url: String,
        canvas: Size,
        display: Size,
        preview: Size,
        zoom_percent: i32,
        fit: bool,
        can_undo: bool,
        can_redo: bool,
        save_url: String,
        layer_count: i32,
        layers_json: String,
        dirty: bool,
    },
    Err(String),
}

fn fit_zoom(canvas: Size, viewport: Size) -> f64 {
    if canvas.pixels() == 0 || viewport.pixels() == 0 {
        return 1.0;
    }
    let sx = viewport.width as f64 / canvas.width as f64;
    let sy = viewport.height as f64 / canvas.height as f64;
    sx.min(sy).min(1.0)
}

fn display_size(canvas: Size, zoom: f64) -> Size {
    Size::new(
        ((canvas.width as f64 * zoom).round() as u64).clamp(1, MAX_DISPLAY_EDGE as u64) as u32,
        ((canvas.height as f64 * zoom).round() as u64).clamp(1, MAX_DISPLAY_EDGE as u64) as u32,
    )
}

fn preview_target(display: Size) -> Size {
    let longest = display.width.max(display.height);
    if longest <= MAX_PREVIEW_EDGE {
        return display;
    }
    let scale = MAX_PREVIEW_EDGE as f64 / longest as f64;
    Size::new(
        ((display.width as f64 * scale).round() as u32).max(1),
        ((display.height as f64 * scale).round() as u32).max(1),
    )
}

fn file_url(path: &Path) -> String {
    format!("file:///{}", path.to_string_lossy().replace('\\', "/"))
}

fn extension_for(format: Format) -> &'static str {
    match format {
        Format::Jpeg => "jpg",
        Format::Png => "png",
        Format::WebP => "webp",
        Format::Bmp => "bmp",
        Format::Tiff => "tiff",
        Format::Gif => "gif",
        _ => "png",
    }
}

impl Session {
    fn open(path: &Path, viewport: Size) -> Result<(Self, i32), String> {
        let t0 = Instant::now();
        let decoded = decode_path(path, DecodeOptions::default()).map_err(|e| e.to_string())?;
        let doc = Document::from_image(decoded.image);
        let zoom = fit_zoom(doc.canvas, viewport);
        let ms = t0.elapsed().as_millis().min(i32::MAX as u128) as i32;
        Ok((
            Self {
                id: SESSION_SEQ.fetch_add(1, Ordering::Relaxed),
                doc,
                source: Some(path.to_path_buf()),
                zoom,
                fit: true,
                render_count: 0,
                dirty: false,
                base_locked: true,
            },
            ms,
        ))
    }

    fn refit(&mut self, viewport: Size) {
        self.zoom = fit_zoom(self.doc.canvas, viewport);
        self.fit = true;
    }

    fn refit_if_needed(&mut self, viewport: Size) {
        if self.fit {
            self.refit(viewport);
        }
    }

    fn zoom_by(&mut self, factor: f64) {
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.fit = false;
    }

    fn render(&mut self) -> Result<Rendered, String> {
        let canvas = self.doc.canvas;
        let display = display_size(canvas, self.zoom);
        let target = preview_target(display);
        if target.pixels() == 0 {
            return Err("empty canvas".into());
        }
        let image = render::composite_for_preview(&self.doc, target);
        // BMP on purpose: uncompressed means both the encode here and Qt's
        // decode are near memcpy speed. PNG compression dominated each live
        // drag tick (tens of ms); BMP keeps the preview path real-time. The
        // URL keeps changing (forces Qt to re-read) while temp files stay
        // bounded; the session id keeps concurrent windows separate.
        let bytes = encode(&image, Format::Bmp, 90).map_err(|e| e.to_string())?;
        self.render_count = self.render_count.wrapping_add(1);
        let path = std::env::temp_dir().join(format!(
            "imageedit-preview-{}-{}.bmp",
            self.id,
            self.render_count % 2
        ));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        Ok(Rendered {
            url: file_url(&path),
            display,
            preview: Size::new(image.width(), image.height()),
        })
    }

    fn default_save_path(&self) -> (PathBuf, Format) {
        match &self.source {
            Some(src) => {
                let stem = src
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "image".to_string());
                let format = format_from_path(src).unwrap_or(Format::Png);
                (
                    src.with_file_name(format!("{stem}-edited.{}", extension_for(format))),
                    format,
                )
            }
            None => (
                std::env::temp_dir().join("imageedit-edited.png"),
                Format::Png,
            ),
        }
    }

    fn save(&self) -> Result<PathBuf, String> {
        let (path, format) = self.default_save_path();
        self.write_to(&path, format)?;
        Ok(path)
    }

    fn write_to(&self, path: &Path, format: Format) -> Result<(), String> {
        let bytes = encode(&self.doc.flatten(), format, SAVE_QUALITY).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }

    fn layers_json(&self) -> String {
        let (base_deleted, base_locked) = (self.doc.base_deleted, self.base_locked);
        let mut parts: Vec<String> = Vec::new();
        for (i, layer) in self.doc.layers.iter().enumerate().rev() {
            let name = layer.name.replace('\\', "\\\\").replace('"', "\\\"");
            let bounds = layer.bounds();
            // o: opacity in percent (0-100); x/y/w/h: bounds in image pixels
            // (the QML side uses them for the live layer outline while dragging);
            // l: locked (blocks move / scale / delete in the UI).
            parts.push(format!(
                "{{\"i\":{i},\"n\":\"{name}\",\"v\":{},\"o\":{},\"l\":{},\"x\":{},\"y\":{},\"w\":{},\"h\":{}}}",
                layer.visible,
                (layer.opacity * 100.0).round() as i32,
                layer.locked,
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height
            ));
        }
        // The base image is listed last (display order = top first) as a
        // locked-by-default row: not reorderable, not deletable while locked
        // (see §4.8 of the UX plan). v reports visibility (deleted/hidden),
        // bd reports base_deleted so the panel can distinguish "hidden" from
        // "deleted" (Delete key flow) for the base row.
        parts.push(format!(
            "{{\"i\":-1,\"n\":\"背景\",\"v\":{},\"bd\":{},\"o\":100,\"l\":{},\"x\":0,\"y\":0,\"w\":{},\"h\":{},\"base\":true}}",
            !base_deleted && !self.doc.base_hidden,
            base_deleted,
            base_locked,
            self.doc.canvas.width,
            self.doc.canvas.height
        ));
        format!("[{}]", parts.join(","))
    }

    fn refresh(&mut self) -> Refresh {
        match self.render() {
            Ok(rendered) => Refresh::Ok {
                url: rendered.url,
                canvas: self.doc.canvas,
                display: rendered.display,
                preview: rendered.preview,
                zoom_percent: (self.zoom * 100.0).round() as i32,
                fit: self.fit,
                can_undo: self.doc.can_undo(),
                can_redo: self.doc.can_redo(),
                save_url: file_url(&self.default_save_path().0),
                layer_count: self.doc.layers.len() as i32,
                layers_json: self.layers_json(),
                dirty: self.dirty,
            },
            Err(err) => Refresh::Err(err),
        }
    }
}

impl qobject::Bridge {
    pub fn start(mut self: Pin<&mut Self>) {
        let selftest = std::env::args().any(|a| a == "--selftest");
        self.as_mut().set_auto_quit(selftest);

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
        self.as_mut().load_path(Path::new(&file));

        if let Some(script) = script {
            self.as_mut().run_script(&script);
        }
    }

    fn run_script(mut self: Pin<&mut Self>, script: &str) {
        for op in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            match op {
                "rotate-cw" => self.as_mut().rotate_cw(),
                "rotate-ccw" => self.as_mut().rotate_ccw(),
                "flip-h" => self.as_mut().flip_horizontal(),
                "flip-v" => self.as_mut().flip_vertical(),
                "undo" => self.as_mut().undo(),
                "redo" => self.as_mut().redo(),
                "zoom-in" => self.as_mut().zoom_in(),
                "zoom-out" => self.as_mut().zoom_out(),
                "zoom-fit" => self.as_mut().zoom_fit(),
                "zoom-actual" => self.as_mut().zoom_actual(),
                "save" => self.as_mut().save_edited(),
                "reset-crop" => self.as_mut().reset_crop(),
                "toggle-visible-selected" => self.as_mut().toggle_selected_visible(),
                "delete-selected" => self.as_mut().delete_selected_layer(),
                "layer-up" => self.as_mut().move_selected_up(),
                "layer-down" => self.as_mut().move_selected_down(),
                "paste" => self.as_mut().paste_clipboard(),
                "next" => self.as_mut().navigate(1),
                "prev" => self.as_mut().navigate(-1),
                "copy" => self.as_mut().copy_selected_layer(),
                "cut" => self.as_mut().cut_selected_layer(),
                other if other.starts_with("opacity:") => {
                    match other
                        .strip_prefix("opacity:")
                        .and_then(|s| s.trim().parse::<f32>().ok())
                    {
                        Some(v) => self.as_mut().set_selected_opacity(v, true),
                        None => println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        ),
                    }
                }
                other if other.starts_with("rename:") => {
                    let name = other.strip_prefix("rename:").unwrap_or("").trim();
                    if !name.is_empty() {
                        self.as_mut()
                            .rename_selected_layer(&QString::from(name.to_string()))
                    } else {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}")
                    }
                }
                other if other.starts_with("scale-selected:") => {
                    let nums: Vec<i32> = other
                        .strip_prefix("scale-selected:")
                        .unwrap_or("")
                        .split(',')
                        .map(str::trim)
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    if nums.len() == 2 {
                        self.as_mut()
                            .scale_selected_layer(nums[0], nums[1], false)
                    } else {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}")
                    }
                }
                other => match other
                    .strip_prefix("copy:")
                    .or_else(|| other.strip_prefix("cut:"))
                    .or_else(|| other.strip_prefix("erase:"))
                {
                    Some(spec) => {
                        let nums: Vec<i32> = spec
                            .split(',')
                            .map(str::trim)
                            .filter_map(|s| s.parse().ok())
                            .collect();
                        if nums.len() != 4 {
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                            );
                        } else if other.starts_with("copy:") {
                            self.as_mut()
                                .copy_selection_pixels(nums[0], nums[1], nums[2], nums[3]);
                        } else if other.starts_with("cut:") {
                            self.as_mut()
                                .cut_selection_pixels(nums[0], nums[1], nums[2], nums[3]);
                        } else {
                            self.as_mut()
                                .erase_selection_pixels(nums[0], nums[1], nums[2], nums[3]);
                        }
                    }
                    None => match other.strip_prefix("extract:") {
                        Some(spec) => {
                            let nums: Vec<i32> = spec
                                .split(',')
                                .map(str::trim)
                                .filter_map(|s| s.parse().ok())
                                .collect();
                            if nums.len() == 4 {
                                self.as_mut()
                                    .extract_selection(nums[0], nums[1], nums[2], nums[3])
                            } else {
                                println!(
                                    "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                )
                            }
                        }
                        None => match other.strip_prefix("select-layer:") {
                            Some(spec) => match spec.trim().parse::<i32>() {
                                Ok(n) => self.as_mut().select_layer(n),
                                Err(_) => println!(
                                    "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                ),
                            },
                            None => match other.strip_prefix("translate-selected:") {
                                Some(spec) => {
                                    let nums: Vec<i32> = spec
                                        .split(',')
                                        .map(str::trim)
                                        .filter_map(|s| s.parse().ok())
                                        .collect();
                                    if nums.len() == 2 {
                                        self.as_mut().translate_selected(nums[0], nums[1])
                                    } else {
                                        println!(
                                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                        )
                                    }
                                }
                                None => match other.strip_prefix("crop:") {
                                    Some(spec) => {
                                        let nums: Vec<i32> = spec
                                            .split(',')
                                            .map(str::trim)
                                            .filter_map(|s| s.parse().ok())
                                            .collect();
                                        if nums.len() == 4 {
                                            self.as_mut()
                                                .crop_to(nums[0], nums[1], nums[2], nums[3])
                                        } else {
                                            println!(
                                                "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                            )
                                        }
                                    }
                                    _ => match other.strip_prefix("resize:") {
                                        Some(spec) => {
                                            let nums: Vec<i32> = spec
                                                .split(',')
                                                .map(str::trim)
                                                .filter_map(|s| s.parse().ok())
                                                .collect();
                                            if nums.len() == 2 {
                                                self.as_mut().resize_to(nums[0], nums[1])
                                            } else {
                                                println!(
                                                    "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                                )
                                            }
                                        }
                                        None => println!(
                                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                        ),
                                    },
                                },
                            },
                        },
                    },
                },
            }
        }
println!(
                "{{\"phase\":\"ui\",\"event\":\"script-done\",\"width\":{},\"height\":{},\"display\":\"{}x{}\",\"zoom\":{},\"undo\":{},\"redo\":{},\"layers\":{},\"selected_layer\":{},\"status\":\"{}\"}}",
                self.image_width(),
                self.image_height(),
                self.display_width(),
                self.display_height(),
                self.zoom_percent(),
                self.can_undo(),
                self.can_redo(),
                self.layer_count(),
                self.selected_layer(),
                self.status(),
            );
    }

    pub fn image_shown(mut self: Pin<&mut Self>) {
        let total = elapsed_ms();
        self.as_mut().set_shown_ms(total);
        println!(
            "{{\"phase\":\"ui\",\"decode_ms\":{},\"shown_ms\":{total},\"width\":{},\"height\":{},\"zoom\":{},\"status\":\"{}\"}}",
            self.decode_ms(),
            self.image_width(),
            self.image_height(),
            self.zoom_percent(),
            self.status(),
        );
    }

    pub fn mark(self: Pin<&mut Self>, name: &QString) {
        println!(
            "{{\"phase\":\"ui\",\"event\":\"mark\",\"name\":\"{}\",\"ms\":{}}}",
            name,
            elapsed_ms()
        );
    }

    pub fn open_file(mut self: Pin<&mut Self>, url: &QUrl) {
        let path = PathBuf::from(url.to_local_file_or_default().to_string());
        self.as_mut().load_path(&path);
    }

    /// Open the previous/next image file in the loaded file's folder. The
    /// dirty check happens in QML (confirm overlay) before calling this.
    pub fn navigate(mut self: Pin<&mut Self>, dir: i32) {
        let current = self
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.source.clone());
        let Some(current) = current else {
            self.as_mut()
                .set_status(QString::from("尚未打开文件，无法切换"));
            return;
        };
        let Some(parent) = current.parent().map(|p| p.to_path_buf()) else {
            return;
        };
        let is_image = |p: &Path| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| {
                    matches!(
                        e.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff"
                    )
                })
                .unwrap_or(false)
        };
        let mut siblings: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && is_image(&path) {
                    siblings.push(path);
                }
            }
        }
        siblings.sort_by_key(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        });
        if siblings.len() < 2 {
            self.as_mut()
                .set_status(QString::from("当前目录没有其他图片"));
            return;
        }
        let cur_key = current
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let idx = siblings
            .iter()
            .position(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default()
                    == cur_key
            })
            .unwrap_or(0);
        let len = siblings.len() as i32;
        let next = (idx as i32 + dir).rem_euclid(len) as usize;
        self.as_mut().load_path(&siblings[next]);
    }

    /// Register a ProgId + "Open with" entries under HKCU (no admin needed),
    /// then open the Windows default-apps settings page so the user can pick
    /// ImageEdit as the default viewer. Windows does not allow setting the
    /// default programmatically (UserChoice is hash-protected by design).
    pub fn register_as_viewer(mut self: Pin<&mut Self>) {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;

            let exe = match std::env::current_exe() {
                Ok(path) => path.to_string_lossy().to_string(),
                Err(_) => {
                    self.as_mut()
                        .set_status(QString::from("无法定位程序路径"));
                    return;
                }
            };
            let run_reg = |args: Vec<String>| -> bool {
                std::process::Command::new("reg")
                    .args(&args)
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
                    .map(|out| out.status.success())
                    .unwrap_or(false)
            };

            let mut all_ok = true;
            all_ok &= run_reg(vec![
                "add".into(),
                r"HKCU\Software\Classes\ImageEdit.Document".into(),
                "/ve".into(),
                "/d".into(),
                "ImageEdit 图片".into(),
                "/f".into(),
            ]);
            all_ok &= run_reg(vec![
                "add".into(),
                r"HKCU\Software\Classes\ImageEdit.Document\shell\open\command".into(),
                "/ve".into(),
                "/d".into(),
                format!("\"{exe}\" \"%1\""),
                "/f".into(),
            ]);
            for ext in [
                ".png", ".jpg", ".jpeg", ".bmp", ".gif", ".webp", ".tif", ".tiff",
            ] {
                all_ok &= run_reg(vec![
                    "add".into(),
                    format!(r"HKCU\Software\Classes\{ext}\OpenWithProgids"),
                    "/v".into(),
                    "ImageEdit.Document".into(),
                    "/t".into(),
                    "REG_SZ".into(),
                    "/d".into(),
                    "".into(),
                    "/f".into(),
                ]);
            }
            if !all_ok {
                self.as_mut()
                    .set_status(QString::from("注册失败：请尝试以管理员身份运行"));
                return;
            }
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", "ms-settings:defaultapps"])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            self.as_mut().set_status(QString::from(
                "已加入系统「打开方式」列表：请在打开的系统设置中将 ImageEdit 设为默认应用",
            ));
        }
        #[cfg(not(windows))]
        {
            self.as_mut()
                .set_status(QString::from("此功能仅支持 Windows"));
        }
    }

    fn persist_config(&self) {
        AppConfig {
            checker_light: self.checker_light().to_string(),
            checker_dark: self.checker_dark().to_string(),
            open_in_new_window: *self.open_in_new_window(),
        }
        .save();
    }

    pub fn update_checker_light(mut self: Pin<&mut Self>, value: &QString) {
        if *self.checker_light() == *value {
            return;
        }
        self.as_mut().set_checker_light(value.clone());
        self.persist_config();
    }

    pub fn update_checker_dark(mut self: Pin<&mut Self>, value: &QString) {
        if *self.checker_dark() == *value {
            return;
        }
        self.as_mut().set_checker_dark(value.clone());
        self.persist_config();
    }

    pub fn update_open_in_new_window(mut self: Pin<&mut Self>, value: bool) {
        if *self.open_in_new_window() == value {
            return;
        }
        self.as_mut().set_open_in_new_window(value);
        self.persist_config();
    }

    /// Refresh the status-bar file info from the session's source path.
    fn update_file_info(mut self: Pin<&mut Self>) {
        let info = self
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.source.clone())
            .map(|path| {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string());
                let size = std::fs::metadata(&path)
                    .map(|m| human_size(m.len()))
                    .unwrap_or_default();
                (name, size)
            });
        match info {
            Some((name, size)) => {
                self.as_mut().set_file_name(QString::from(name.as_str()));
                self.as_mut()
                    .set_file_size_str(QString::from(size.as_str()));
            }
            None => {
                self.as_mut().set_file_name(QString::from(""));
                self.as_mut().set_file_size_str(QString::from(""));
            }
        }
    }

    pub fn set_viewport(mut self: Pin<&mut Self>, width: i32, height: i32) {
        let size = Size::new(width.max(0) as u32, height.max(0) as u32);
        if *self.viewport.borrow() == size {
            return;
        }
        *self.viewport.borrow_mut() = size;
        let refresh = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                Some(session) if session.fit => {
                    session.refit(size);
                    true
                }
                _ => false,
            }
        };
        if refresh {
            self.as_mut().refresh_preview();
        }
    }

    pub fn zoom_in(mut self: Pin<&mut Self>) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.zoom_by(ZOOM_STEP);
        }
        self.as_mut().refresh_preview();
    }

    pub fn zoom_out(mut self: Pin<&mut Self>) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.zoom_by(1.0 / ZOOM_STEP);
        }
        self.as_mut().refresh_preview();
    }

    pub fn zoom_fit(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.refit(viewport);
        }
        self.as_mut().refresh_preview();
    }

    pub fn zoom_actual(mut self: Pin<&mut Self>) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.zoom = 1.0;
            session.fit = false;
        }
        self.as_mut().refresh_preview();
    }

    pub fn rotate_cw(mut self: Pin<&mut Self>) {
        self.as_mut().apply_transform(Transform::Rotate90Cw, "rotate-cw");
    }

    pub fn rotate_ccw(mut self: Pin<&mut Self>) {
        self.as_mut()
            .apply_transform(Transform::Rotate90Ccw, "rotate-ccw");
    }

    pub fn flip_horizontal(mut self: Pin<&mut Self>) {
        self.as_mut()
            .apply_transform(Transform::FlipHorizontal, "flip-horizontal");
    }

    pub fn flip_vertical(mut self: Pin<&mut Self>) {
        self.as_mut()
            .apply_transform(Transform::FlipVertical, "flip-vertical");
    }

    pub fn undo(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let outcome: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => match session.doc.undo() {
                    Ok(()) => {
                        session.refit_if_needed(viewport);
                        session.dirty = true;
                        Ok(true)
                    }
                    Err(err) => Err(err.to_string()),
                },
            }
        };
        match outcome {
            Ok(true) => self.as_mut().refresh_preview(),
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn redo(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let outcome: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => match session.doc.redo() {
                    Ok(()) => {
                        session.refit_if_needed(viewport);
                        session.dirty = true;
                        Ok(true)
                    }
                    Err(err) => Err(err.to_string()),
                },
            }
        };
        match outcome {
            Ok(true) => self.as_mut().refresh_preview(),
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn save_edited(mut self: Pin<&mut Self>) {
        let result = self
            .session
            .borrow()
            .as_ref()
            .map(Session::save)
            .unwrap_or_else(|| Err("nothing to save".to_string()));
        if result.is_ok() {
            if let Some(s) = self.session.borrow_mut().as_mut() {
                s.dirty = false;
            }
            self.as_mut().set_dirty(false);
            self.as_mut().update_file_info();
        }
        self.as_mut().report_save(result);
    }

    pub fn save_as(mut self: Pin<&mut Self>, url: &QUrl) {
        let path = PathBuf::from(url.to_local_file_or_default().to_string());
        let result = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                None => Err("nothing to save".to_string()),
                Some(session) => {
                    let format = format_from_path(&path).unwrap_or(Format::Png);
                    session.write_to(&path, format).map(|()| path.clone())
                }
            }
        };
        if result.is_ok() {
            if let Some(s) = self.session.borrow_mut().as_mut() {
                s.dirty = false;
                s.source = Some(path.clone());
            }
            self.as_mut().set_dirty(false);
            self.as_mut().update_file_info();
        }
        self.as_mut().report_save(result);
    }

    pub fn crop_to(mut self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32) {
        let viewport = *self.viewport.borrow();
        let canvas = self
            .session
            .borrow()
            .as_ref()
            .map(|s| s.doc.canvas);
        let canvas = match canvas {
            None => return,
            Some(c) => c,
        };
        let x = x.clamp(0, canvas.width as i32 - 1);
        let y = y.clamp(0, canvas.height as i32 - 1);
        let w = w.max(1).min(canvas.width as i32 - x) as u32;
        let h = h.max(1).min(canvas.height as i32 - y) as u32;
        let rect = Rect::new(x, y, w, h);
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    session
                        .doc
                        .crop(rect)
                        .map(|_| true)
                        .map_err(|e| e.to_string())
                }
            }
        };
        match result {
            Ok(true) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                    }
                }
                self.as_mut().refresh_preview();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"cropped\",\"rect\":\"{x},{y},{w},{h}\"}}"
                );
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn resize_to(mut self: Pin<&mut Self>, width: i32, height: i32) {
        if width < 1 || height < 1 {
            self.as_mut()
                .set_status(QString::from("resize-error: size must be >= 1"));
            return;
        }
        let viewport = *self.viewport.borrow();
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    let size = Size::new(width as u32, height as u32);
                    let old = session.doc.canvas;
session
                        .doc
                        .resize(size)
                        .map(move |_| {
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"resized\",\"from\":\"{}x{}\",\"to\":\"{}x{}\"}}",
                                old.width, old.height, size.width, size.height
                            );
                            true
                        })
                        .map_err(|e| e.to_string())
                }
            }
        };
        match result {
            Ok(true) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                    }
                }
                self.as_mut().refresh_preview();
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn resize_canvas(mut self: Pin<&mut Self>, width: i32, height: i32) {
        if width < 1 || height < 1 {
            self.as_mut()
                .set_status(QString::from("resize-error: size must be >= 1"));
            return;
        }
        let viewport = *self.viewport.borrow();
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    let size = Size::new(width as u32, height as u32);
                    let old = session.doc.canvas;
                    session
                        .doc
                        .resize_canvas(size)
                        .map(move |_| {
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"canvas-resized\",\"from\":\"{}x{}\",\"to\":\"{}x{}\"}}",
                                old.width, old.height, size.width, size.height
                            );
                            true
                        })
                        .map_err(|e| e.to_string())
                }
            }
        };
        match result {
            Ok(true) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                    }
                }
                self.as_mut().refresh_preview();
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn reset_crop(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    let base = session.doc.base.dimensions();
                    let is_full =
                        session.doc.view.x == 0
                            && session.doc.view.y == 0
                            && session.doc.view.width == base.0
                            && session.doc.view.height == base.1;
                    if is_full {
                        Ok(false)
                    } else {
                        match session.doc.reset_crop() {
                            Ok(()) => {
                                session.refit_if_needed(viewport);
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        match result {
            Ok(true) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                    }
                }
                self.as_mut().refresh_preview();
                self.as_mut()
                    .set_status(QString::from("crop reset"));
            }
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    fn clamp_selection(&self, x: i32, y: i32, w: i32, h: i32) -> Option<Rect> {
        let canvas = self
            .session
            .borrow()
            .as_ref()
            .map(|s| s.doc.canvas)?;
        if canvas.pixels() == 0 {
            return None;
        }
        let x = x.clamp(0, canvas.width as i32 - 1);
        let y = y.clamp(0, canvas.height as i32 - 1);
        let w = w.max(1).min(canvas.width as i32 - x) as u32;
        let h = h.max(1).min(canvas.height as i32 - y) as u32;
        Some(Rect::new(x, y, w, h))
    }

    fn selected_guard(&self, index: i32) -> Option<usize> {
        let ix = index.max(0) as usize;
        let guard = self.session.borrow();
        let session = guard.as_ref()?;
        (ix < session.doc.layers.len()).then_some(ix)
    }

    fn layer_locked(&self, ix: usize) -> bool {
        self.session
            .borrow()
            .as_ref()
            .and_then(|s| s.doc.layers.get(ix))
            .map(|l| l.locked)
            .unwrap_or(false)
    }

    pub fn extract_selection(mut self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32) {
        let viewport = *self.viewport.borrow();
        let Some(rect) = self.clamp_selection(x, y, w, h) else {
            return;
        };
        let result: Result<i32, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => session
                    .doc
                    .extract_selection(&Selection::Rect(rect))
                    .map(|ix| ix as i32)
                    .map_err(|e| e.to_string()),
            }
        };
        match result {
            Ok(ix) => {
                self.as_mut().set_selected_layer(ix);
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                    }
                }
                self.as_mut().refresh_preview();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"layer-extracted\",\"index\":{ix},\"rect\":\"{},{},{},{}\"}}",
                    rect.x, rect.y, rect.width, rect.height
                );
            }
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn layer_name(self: Pin<&mut Self>, index: i32) -> QString {
        let name = self
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.doc.layers.get(index.max(0) as usize))
            .map(|l| l.name.clone())
            .unwrap_or_default();
        QString::from(name.as_str())
    }

    pub fn layer_visible(self: Pin<&mut Self>, index: i32) -> bool {
        self.session
            .borrow()
            .as_ref()
            .and_then(|s| s.doc.layers.get(index.max(0) as usize))
            .map(|l| l.visible)
            .unwrap_or(false)
    }

    pub fn select_layer(mut self: Pin<&mut Self>, index: i32) {
        if index < 0 {
            // Negative index = deselect (Esc / clicking empty canvas).
            self.as_mut().set_selected_layer(-1);
            return;
        }
        if self.selected_guard(index).is_none() {
            return;
        }
        self.as_mut().set_selected_layer(index);
    }

    pub fn toggle_selected_visible(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let cur = session.doc.layers[ix].visible;
                    session
                        .doc
                        .set_layer_visible(ix, !cur)
                        .map(|()| true)
                        .map_err(|e| e.to_string())
                }
            }
        };
        match result {
            Ok(true) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                    }
                }
                self.as_mut().refresh_preview();
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    /// Eye icon in the layers panel: toggle one layer's visibility by index.
    pub fn set_layer_visible_at(mut self: Pin<&mut Self>, index: i32, visible: bool) {
        if index < 0 {
            return;
        }
        let viewport = *self.viewport.borrow();
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if index as usize >= session.doc.layers.len() {
                        Err("layer index out of range".into())
                    } else {
                        session
                            .doc
                            .set_layer_visible(index as usize, visible)
                            .map(|()| true)
                            .map_err(|e| e.to_string())
                    }
                }
            }
        };
        match result {
            Ok(true) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                    }
                }
                self.as_mut().refresh_preview();
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn delete_selected_layer(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            self.as_mut()
                .set_status(QString::from("图层已锁定，先解锁再删除"));
            return;
        }
        let result: Result<i32, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => match session.doc.remove_layer(ix) {
                        Ok(()) => {
                            let new_count = session.doc.layers.len();
                            Ok(if new_count == 0 {
                                -1
                            } else {
                                (ix - 1).min(new_count - 1) as i32
                            })
                        }
                        Err(e) => Err(e.to_string()),
                    },
            }
        };
        match result {
            Ok(sel) => {
                self.as_mut().set_selected_layer(sel);
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                    }
                }
                self.as_mut().refresh_preview();
                println!("{{\"phase\":\"ui\",\"event\":\"layer-deleted\",\"index\":{ix}}}");
            }
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn move_selected_up(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if ix + 1 >= session.doc.layers.len() {
                        Ok(false)
                    } else {
                        match session.doc.move_layer(ix, ix + 1) {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        if result.as_ref().ok().copied() == Some(true) {
            self.as_mut().set_selected_layer((ix + 1) as i32);
        }
        self.apply_layer_edit(viewport, result);
    }

    pub fn move_selected_down(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if ix == 0 {
                        Ok(false)
                    } else {
                        match session.doc.move_layer(ix, ix - 1) {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        if result.as_ref().ok().copied() == Some(true) {
            self.as_mut().set_selected_layer((ix - 1) as i32);
        }
        self.apply_layer_edit(viewport, result);
    }

    /// Drag-reorder in the layers panel: one undoable step via core
    /// `move_layer(from, to)`; the selection follows the moved layer.
    pub fn reorder_layer(mut self: Pin<&mut Self>, from: i32, to: i32) {
        if from < 0 || to < 0 || from == to {
            return;
        }
        let viewport = *self.viewport.borrow();
        if self.layer_locked(from as usize) || self.layer_locked(to as usize) {
            self.as_mut()
                .set_status(QString::from("已锁定的图层不能调整顺序"));
            return;
        }
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let len = session.doc.layers.len() as i32;
                    if from >= len || to >= len {
                        Err("layer index out of range".into())
                    } else {
                        match session.doc.move_layer(from as usize, to as usize) {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        if result.as_ref().ok().copied() == Some(true) {
            self.as_mut().set_selected_layer(to);
        }
        self.apply_layer_edit(viewport, result);
    }

    pub fn translate_selected(mut self: Pin<&mut Self>, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
            return;
        }
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            self.as_mut()
                .set_status(QString::from("图层已锁定，先解锁再移动"));
            return;
        }
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => match session.doc.translate_layer(ix, dx, dy) {
                    Ok(()) => {
                        session.dirty = true;
                        Ok(true)
                    }
                    Err(e) => Err(e.to_string()),
                },
            }
        };
        self.apply_layer_edit(viewport, result);
    }

    // ---- 剪贴板（跨文档复制粘贴） ----

    pub fn copy_selection_pixels(mut self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32) {
        let Some(rect) = self.clamp_selection(x, y, w, h) else {
            return;
        };
        let result: Result<Layer, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => session
                    .doc
                    .selection_layer(&Selection::Rect(rect))
                    .map_err(|e| e.to_string()),
            }
        };
        match result {
            Ok(layer) => {
                let text = format!(
                    "已复制 {}×{}（Ctrl+V 粘贴，可跨文档）",
                    layer.image.width(),
                    layer.image.height()
                );
                *clipboard().lock().unwrap() = Some(layer);
                *self.paste_counter.borrow_mut() = 0;
                self.as_mut().set_status(QString::from(text.as_str()));
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"copied\",\"rect\":\"{},{},{},{}\"}}",
                    rect.x, rect.y, rect.width, rect.height
                );
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn cut_selection_pixels(mut self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32) {
        let Some(rect) = self.clamp_selection(x, y, w, h) else {
            return;
        };
        let viewport = *self.viewport.borrow();
        let result: Result<Layer, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    match session.doc.selection_layer(&Selection::Rect(rect)) {
                        Err(e) => Err(e.to_string()),
                        Ok(layer) => {
                            match session.doc.erase_selection(&Selection::Rect(rect)) {
                                Err(e) => Err(e.to_string()),
                                Ok(()) => {
                                    session.refit_if_needed(viewport);
                                    session.dirty = true;
                                    Ok(layer)
                                }
                            }
                        }
                    }
                }
            }
        };
        match result {
            Ok(layer) => {
                let text = format!(
                    "已剪切 {}×{}（Ctrl+V 粘贴，可跨文档）",
                    layer.image.width(),
                    layer.image.height()
                );
                *clipboard().lock().unwrap() = Some(layer);
                *self.paste_counter.borrow_mut() = 0;
                self.as_mut().set_status(QString::from(text.as_str()));
                self.as_mut().refresh_preview();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"cut\",\"rect\":\"{},{},{},{}\"}}",
                    rect.x, rect.y, rect.width, rect.height
                );
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn erase_selection_pixels(mut self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32) {
        let Some(rect) = self.clamp_selection(x, y, w, h) else {
            return;
        };
        let viewport = *self.viewport.borrow();
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    match session.doc.erase_selection(&Selection::Rect(rect)) {
                        Ok(()) => {
                            session.refit_if_needed(viewport);
                            session.dirty = true;
                            Ok(true)
                        }
                        Err(e) => Err(e.to_string()),
                    }
                }
            }
        };
        match result {
            Ok(true) => {
                self.as_mut().refresh_preview();
                self.as_mut().set_status(QString::from("已删除选区像素（非破坏，可撤销）"));
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"erased\",\"rect\":\"{},{},{},{}\"}}",
                    rect.x, rect.y, rect.width, rect.height
                );
            }
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn copy_selected_layer(mut self: Pin<&mut Self>) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            self.as_mut().set_status(QString::from("没有选中的图层"));
            return;
        };
        let layer = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => session.doc.layers[ix].clone(),
                None => return,
            }
        };
        let text = format!("已复制图层「{}」（Ctrl+V 粘贴，可跨文档）", layer.name);
        *clipboard().lock().unwrap() = Some(layer);
        *self.paste_counter.borrow_mut() = 0;
        self.as_mut().set_status(QString::from(text.as_str()));
        println!("{{\"phase\":\"ui\",\"event\":\"copied-layer\",\"index\":{ix}}}");
    }

    pub fn cut_selected_layer(mut self: Pin<&mut Self>) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            self.as_mut().set_status(QString::from("没有选中的图层"));
            return;
        };
        let layer = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => session.doc.layers[ix].clone(),
                None => return,
            }
        };
        *clipboard().lock().unwrap() = Some(layer);
        *self.paste_counter.borrow_mut() = 0;
        println!("{{\"phase\":\"ui\",\"event\":\"cut-layer\",\"index\":{ix}}}");
        self.as_mut().delete_selected_layer();
    }

    pub fn paste_clipboard(mut self: Pin<&mut Self>) {
        let clipboard = clipboard().lock().unwrap().clone();
        let Some(clip) = clipboard else {
            self.as_mut().set_status(QString::from("剪贴板为空"));
            return;
        };
        let viewport = *self.viewport.borrow();
        const PASTE_SHIFT: i32 = 12;
        let result: Result<usize, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let canvas = session.doc.canvas;
                    let count = *self.paste_counter.borrow() as i32;
                    let mut layer = clip.clone();
                    // Auto-name by creation order ("1", "2", ...).
                    layer.name = session.doc.next_layer_name();
                    // Keep the pasted layer at its copied position, clamped so
                    // part of it stays on the canvas; repeated pastes fan out.
                    layer.offset.0 = (clip.offset.0 + PASTE_SHIFT * count).clamp(
                        -(layer.image.width() as i32 - 1),
                        canvas.width as i32 - 1,
                    );
                    layer.offset.1 = (clip.offset.1 + PASTE_SHIFT * count).clamp(
                        -(layer.image.height() as i32 - 1),
                        canvas.height as i32 - 1,
                    );
                    match session.doc.add_layer(layer) {
                        Ok(index) => {
                            session.dirty = true;
                            *self.paste_counter.borrow_mut() = count as u32 + 1;
                            Ok(index)
                        }
                        Err(e) => Err(e.to_string()),
                    }
                }
            }
        };
        match result {
            Ok(index) => {
                self.as_mut().set_selected_layer(index as i32);
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                self.as_mut().refresh_preview();
                println!("{{\"phase\":\"ui\",\"event\":\"pasted\",\"index\":{index}}}");
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    // ---- 图层属性 ----

    pub fn set_selected_opacity(self: Pin<&mut Self>, opacity: f32, commit: bool) {
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let op = opacity.clamp(0.0, 1.0);
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if commit {
                        // Rewind any live (history-free) change first so the
                        // history op records the true pre-drag opacity.
                        let start = self.opacity_live.borrow_mut().take();
                        if let Some(start) = start {
                            if let Some(layer) = session.doc.layers.get_mut(ix) {
                                layer.opacity = start;
                            }
                        }
                        match session.doc.set_layer_opacity(ix, op) {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    } else {
                        if self.opacity_live.borrow().is_none() {
                            let cur = session.doc.layers.get(ix).map(|l| l.opacity);
                            *self.opacity_live.borrow_mut() = cur;
                        }
                        if let Some(layer) = session.doc.layers.get_mut(ix) {
                            layer.opacity = op;
                        }
                        Ok(true)
                    }
                }
            }
        };
        self.apply_layer_edit(viewport, result);
    }

    pub fn rename_selected_layer(self: Pin<&mut Self>, name: &QString) {
        let new_name = name.to_string();
        if new_name.trim().is_empty() {
            return;
        }
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => match session.doc.rename_layer(ix, new_name.as_str()) {
                    Ok(()) => {
                        session.dirty = true;
                        Ok(true)
                    }
                    Err(e) => Err(e.to_string()),
                },
            }
        };
        self.apply_layer_edit(viewport, result);
    }

    /// Scale the selected layer to `width × height` (Lanczos3 via core
    /// `scale_layer`). keep_ratio: if either dimension is <= 0, derive it from
    /// the other using the layer's current aspect. The layer offset stays.
    pub fn scale_selected_layer(
        mut self: Pin<&mut Self>,
        width: i32,
        height: i32,
        keep_ratio: bool,
    ) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            self.as_mut()
                .set_status(QString::from("图层已锁定，先解锁再缩放"));
            return;
        }
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let layer = &session.doc.layers[ix];
                    let (ow, oh) = (layer.image.width() as i32, layer.image.height() as i32);
                    let offset = layer.offset;
                    let (mut tw, mut th) = (width, height);
                    if keep_ratio && (tw <= 0 || th <= 0) {
                        // Derive the missing side from the current aspect.
                        if tw <= 0 && th > 0 {
                            tw = ((th as f64) * (ow as f64 / oh as f64)).round() as i32;
                        } else if th <= 0 && tw > 0 {
                            th = ((tw as f64) * (oh as f64 / ow as f64)).round() as i32;
                        }
                    }
                    if tw < 1 || th < 1 {
                        Err("缩放尺寸至少为 1×1".into())
                    } else if tw == ow && th == oh {
                        Ok(false)
                    } else {
                        match session
                            .doc
                            .scale_layer(ix, Size::new(tw as u32, th as u32), offset)
                        {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        match result {
            Ok(true) => {
                self.as_mut().refresh_preview();
                self.as_mut().set_dirty(true);
                println!("{{\"phase\":\"ui\",\"event\":\"layer-scaled\",\"index\":{ix}}}");
            }
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    // ---- 图层缩放拖拽实时预览 ----

    pub fn scale_layer_begin(self: Pin<&mut Self>) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            return;
        }
        let guard = self.session.borrow();
        if let Some(session) = guard.as_ref() {
            if let Some(layer) = session.doc.layers.get(ix) {
                *self.scale_start.borrow_mut() = Some((layer.image.clone(), layer.offset));
            }
        }
    }

    pub fn scale_layer_update(self: Pin<&mut Self>, w: i32, h: i32, ox: i32, oy: i32) {
        let Some(_) = *self.scale_start.borrow() else {
            return;
        };
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if w < 1 || h < 1 || self.layer_locked(ix) {
            return;
        }
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                // Live preview: fast (Triangle) resample, no history entry.
                let _ = session.doc.scale_layer_live(ix, Size::new(w as u32, h as u32), (ox, oy));
            }
        }
        self.refresh_preview();
    }

    pub fn scale_layer_end(mut self: Pin<&mut Self>, commit: bool, w: i32, h: i32, ox: i32, oy: i32) {
        let Some((image, offset)) = self.scale_start.borrow_mut().take() else {
            return;
        };
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    // Roll the live preview back to the pre-drag state, then
                    // re-apply once so the history op records the true start.
                    if let Some(layer) = session.doc.layers.get_mut(ix) {
                        layer.image = image;
                        layer.offset = offset;
                    }
                    if !commit {
                        Ok(false)
                    } else {
                        match session
                            .doc
                            .scale_layer(ix, Size::new(w.max(1) as u32, h.max(1) as u32), (ox, oy))
                        {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        match result {
            Ok(true) => {
                self.as_mut().refresh_preview();
                self.as_mut().set_dirty(true);
                println!("{{\"phase\":\"ui\",\"event\":\"layer-scaled\",\"index\":{ix}}}");
            }
            Ok(false) => self.as_mut().refresh_preview(),
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    // ---- 图层锁定 ----

    pub fn set_layer_locked(mut self: Pin<&mut Self>, index: i32, locked: bool) {
        if index < 0 {
            return;
        }
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                if let Some(layer) = session.doc.layers.get_mut(index as usize) {
                    layer.locked = locked;
                }
            }
        }
        self.as_mut().refresh_layers_only();
    }

    pub fn set_base_locked(mut self: Pin<&mut Self>, locked: bool) {
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                session.base_locked = locked;
            }
        }
        self.as_mut().refresh_layers_only();
    }

    /// Eye toggle on the base row: hide/show the background (undoable).
    pub fn toggle_base_visible(mut self: Pin<&mut Self>) {
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let make_visible = session.doc.base_hidden;
                    match session.doc.set_base_visible(make_visible) {
                        Ok(()) => {
                            session.dirty = true;
                            Ok(make_visible)
                        }
                        Err(e) => Err(e.to_string()),
                    }
                }
            }
        };
        match result {
            Ok(_) => {
                // Visibility flips don't change the canvas size: no refit.
                self.as_mut().refresh_preview();
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn delete_base_layer(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if session.base_locked {
                        Err("背景已锁定，先点击锁图标解锁".into())
                    } else if session.doc.base_deleted {
                        Ok(false)
                    } else {
                        match session.doc.delete_base() {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        match result {
            Ok(true) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                self.as_mut().refresh_preview();
                println!("{{\"phase\":\"ui\",\"event\":\"base-deleted\"}}");
            }
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    /// Re-publish `layers_json` (lock toggles change panel state only).
    fn refresh_layers_only(mut self: Pin<&mut Self>) {
        let json = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                Some(session) => session.layers_json(),
                None => return,
            }
        };
        self.as_mut()
            .set_layers_json(QString::from(json.as_str()));
    }

    // ---- 移动图层拖拽实时预览 ----

    pub fn drag_translate_begin(self: Pin<&mut Self>) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            return;
        }
        let guard = self.session.borrow();
        if let Some(session) = guard.as_ref() {
            if let Some(layer) = session.doc.layers.get(ix) {
                *self.drag_start_offset.borrow_mut() = Some(layer.offset);
            }
        }
    }

    pub fn drag_translate_update(self: Pin<&mut Self>, dx: i32, dy: i32) {
        let Some(start) = *self.drag_start_offset.borrow() else {
            return;
        };
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            return;
        }
        {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => {
                    // Skip render when the offset did not actually change.
                    if session.doc.layers.get(ix).map(|l| l.offset)
                        == Some((start.0 + dx, start.1 + dy))
                    {
                        return;
                    }
                }
                None => return,
            }
        }
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                if let Some(layer) = session.doc.layers.get_mut(ix) {
                    // Live preview only: no history entry until commit.
                    layer.offset = (start.0 + dx, start.1 + dy);
                }
            }
        }
        self.refresh_preview();
    }

    pub fn drag_translate_end(self: Pin<&mut Self>, commit: bool) {
        let viewport = *self.viewport.borrow();
        let Some(start) = self.drag_start_offset.borrow_mut().take() else {
            return;
        };
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let cur = session
                        .doc
                        .layers
                        .get(ix)
                        .map(|l| l.offset)
                        .unwrap_or(start);
                    if !commit || cur == start {
                        // Roll back the live (history-free) offset.
                        if let Some(layer) = session.doc.layers.get_mut(ix) {
                            layer.offset = start;
                        }
                        Ok(true)
                    } else {
                        // Reset to the drag start, then translate once so the
                        // history op records from=start to=cur.
                        session.doc.layers[ix].offset = start;
                        match session
                            .doc
                            .translate_layer(ix, cur.0 - start.0, cur.1 - start.1)
                        {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        self.apply_layer_edit(viewport, result);
    }

    fn apply_layer_edit(mut self: Pin<&mut Self>, viewport: Size, result: Result<bool, String>) {
        match result {
            Ok(true) => {
                let mut guard = self.session.borrow_mut();
                if let Some(s) = guard.as_mut() {
                    s.refit_if_needed(viewport);
                }
                drop(guard);
                self.refresh_preview();
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    fn report_save(mut self: Pin<&mut Self>, result: Result<PathBuf, String>) {
        match result {
            Ok(path) => {
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"saved\",\"path\":\"{}\"}}",
                    path.display().to_string().replace('\\', "/")
                );
                let text = format!("saved: {}", path.display());
                self.as_mut().set_status(QString::from(text.as_str()));
            }
            Err(err) => {
                let text = format!("save-error: {err}");
                self.as_mut().set_status(QString::from(text.as_str()));
            }
        }
    }

    fn load_path(mut self: Pin<&mut Self>, path: &Path) {
        let viewport = *self.viewport.borrow();
        println!(
            "{{\"phase\":\"ui\",\"event\":\"load\",\"file\":\"{}\"}}",
            path.to_string_lossy().replace('\\', "/")
        );
        match Session::open(path, viewport) {
            Ok((session, ms)) => {
                self.as_mut().set_load_failed(false);
                self.as_mut().set_decode_ms(ms);
                *self.session.borrow_mut() = Some(session);
                self.as_mut().update_file_info();
                self.as_mut().refresh_preview();
            }
            Err(err) => {
                *self.session.borrow_mut() = None;
                eprintln!("{{\"phase\":\"ui\",\"error\":\"decode-failed\",\"detail\":\"{err}\"}}");
                let status = format!("decode-error: {err}");
                self.as_mut().set_status(QString::from(status.as_str()));
                self.as_mut().set_preview_url(QString::from(""));
                self.as_mut().set_load_failed(true);
                self.as_mut().update_file_info();
            }
        }
    }

    fn apply_transform(mut self: Pin<&mut Self>, transform: Transform, label: &str) {
        let viewport = *self.viewport.borrow();
        let outcome: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => match session.doc.apply_transform(transform) {
                    Ok(()) => {
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"edit\",\"op\":\"{label}\",\"width\":{},\"height\":{}}}",
                            session.doc.canvas.width, session.doc.canvas.height
                        );
                        session.refit_if_needed(viewport);
                        session.dirty = true;
                        Ok(true)
                    }
                    Err(err) => Err(err.to_string()),
                },
            }
        };
        match outcome {
            Ok(true) => self.as_mut().refresh_preview(),
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    fn refresh_preview(mut self: Pin<&mut Self>) {
        let outcome = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                Some(session) => session.refresh(),
                None => return,
            }
        };
        match outcome {
            Refresh::Ok {
                url,
                canvas,
                display,
                preview,
                zoom_percent,
                fit,
                can_undo,
                can_redo,
                save_url,
                layer_count,
                layers_json,
                dirty,
            } => {
                // The status bar's left side already shows the file name and
                // size; a second resolution readout here was redundant, so
                // only keep the preview-degraded hint when it matters.
                let status = if preview == display {
                    String::new()
                } else {
                    format!(
                        "{}  (预览 {}x{})",
                        if fit { "适应窗口" } else { "缩放" },
                        preview.width,
                        preview.height
                    )
                };
                self.as_mut().set_image_width(canvas.width as i32);
                self.as_mut().set_image_height(canvas.height as i32);
                self.as_mut().set_display_width(display.width as i32);
                self.as_mut().set_display_height(display.height as i32);
                self.as_mut().set_zoom_percent(zoom_percent);
                self.as_mut().set_fit_mode(fit);
                self.as_mut().set_can_undo(can_undo);
                self.as_mut().set_can_redo(can_redo);
                self.as_mut().set_layer_count(layer_count);
                self.as_mut().set_dirty(dirty);
                self.as_mut()
                    .set_layers_json(QString::from(layers_json.as_str()));
                if *self.selected_layer() < 0 || *self.selected_layer() >= layer_count {
                    self.as_mut().set_selected_layer(-1);
                }
                self.as_mut().set_default_save_url(QString::from(save_url.as_str()));
                self.as_mut().set_status(QString::from(status.as_str()));
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"preview-ready\",\"zoom\":{zoom_percent},\"display\":\"{}x{}\",\"preview\":\"{}x{}\",\"undo\":{can_undo},\"redo\":{can_redo}}}",
                    display.width, display.height, preview.width, preview.height
                );
                // Last: updating preview_url triggers Image loading, which may
                // complete synchronously and re-enter `image_shown`.
                self.as_mut().set_preview_url(QString::from(url.as_str()));
            }
            Refresh::Err(err) => {
                let status = format!("render-error: {err}");
                self.as_mut().set_status(QString::from(status.as_str()));
            }
        }
    }
}
