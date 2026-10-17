use core::pin::Pin;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use cxx_qt_lib::{QString, QUrl};
use decode::{decode_path, encode, format_from_path, DecodeOptions, Format};
use editcore::{Document, Rect, Selection, Size, Transform};

pub static PROCESS_START: OnceLock<Instant> = OnceLock::new();

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
        fn delete_selected_layer(self: Pin<&mut Self>);

        #[qinvokable]
        fn move_selected_up(self: Pin<&mut Self>);

        #[qinvokable]
        fn move_selected_down(self: Pin<&mut Self>);

        #[qinvokable]
        fn translate_selected(self: Pin<&mut Self>, dx: i32, dy: i32);
    }
}

#[derive(Default)]
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
    session: RefCell<Option<Session>>,
    viewport: RefCell<Size>,
}

struct Session {
    doc: Document,
    source: Option<PathBuf>,
    zoom: f64,
    fit: bool,
    render_count: u32,
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
                doc,
                source: Some(path.to_path_buf()),
                zoom,
                fit: true,
                render_count: 0,
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
        let png = encode(&image, Format::Png, 90).map_err(|e| e.to_string())?;
        self.render_count = self.render_count.wrapping_add(1);
        // Alternate between two files so the URL keeps changing (forces Qt to
        // re-read) while the number of temp files stays bounded.
        let path = std::env::temp_dir().join(format!(
            "imageedit-preview-{}.png",
            self.render_count % 2
        ));
        std::fs::write(&path, png).map_err(|e| e.to_string())?;
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
        let mut parts: Vec<String> = Vec::new();
        for (i, layer) in self.doc.layers.iter().enumerate().rev() {
            let name = layer.name.replace('\\', "\\\\").replace('"', "\\\"");
            parts.push(format!(
                "{{\"i\":{},\"n\":\"{name}\",\"v\":{}}}",
                i, layer.visible
            ));
        }
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
                other if other.starts_with("extract:") => {
                    let nums: Vec<i32> = other
                        .strip_prefix("extract:")
                        .unwrap_or("")
                        .split(',')
                        .map(str::trim)
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    if nums.len() == 4 {
                        self.as_mut()
                            .extract_selection(nums[0], nums[1], nums[2], nums[3])
                    } else {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}")
                    }
                }
                other if other.starts_with("select-layer:") => {
                    let n: Option<i32> = other
                        .strip_prefix("select-layer:")
                        .and_then(|s| s.trim().parse().ok());
                    if let Some(n) = n {
                        self.as_mut().select_layer(n)
                    } else {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}")
                    }
                }
                other if other.starts_with("translate-selected:") => {
                    let nums: Vec<i32> = other
                        .strip_prefix("translate-selected:")
                        .unwrap_or("")
                        .split(',')
                        .map(str::trim)
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    if nums.len() == 2 {
                        self.as_mut().translate_selected(nums[0], nums[1])
                    } else {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}")
                    }
                }
                other => match other.strip_prefix("crop:") {
                    Some(spec) => {
                        let nums: Vec<i32> = spec
                            .split(',')
                            .map(str::trim)
                            .filter_map(|s| s.parse().ok())
                            .collect();
                        if nums.len() == 4 {
                            self.as_mut().crop_to(nums[0], nums[1], nums[2], nums[3])
                        } else {
                            println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}")
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
                        session
                            .doc
                            .move_layer(ix, ix + 1)
                            .map(|()| true)
                            .map_err(|e| e.to_string())
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
                        session
                            .doc
                            .move_layer(ix, ix - 1)
                            .map(|()| true)
                            .map_err(|e| e.to_string())
                    }
                }
            }
        };
        if result.as_ref().ok().copied() == Some(true) {
            self.as_mut().set_selected_layer((ix - 1) as i32);
        }
        self.apply_layer_edit(viewport, result);
    }

    pub fn translate_selected(self: Pin<&mut Self>, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
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
                Some(session) => session
                    .doc
                    .translate_layer(ix, dx, dy)
                    .map(|()| true)
                    .map_err(|e| e.to_string()),
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
                self.as_mut().refresh_preview();
            }
            Err(err) => {
                *self.session.borrow_mut() = None;
                eprintln!("{{\"phase\":\"ui\",\"error\":\"decode-failed\",\"detail\":\"{err}\"}}");
                let status = format!("decode-error: {err}");
                self.as_mut().set_status(QString::from(status.as_str()));
                self.as_mut().set_preview_url(QString::from(""));
                self.as_mut().set_load_failed(true);
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
            } => {
                let status = if preview == display {
                    format!("{}x{}", canvas.width, canvas.height)
                } else {
                    format!(
                        "{}x{}  {}  (预览 {}x{})",
                        canvas.width,
                        canvas.height,
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
