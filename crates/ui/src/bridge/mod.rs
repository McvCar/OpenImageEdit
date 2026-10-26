use core::pin::Pin;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use cxx_qt_lib::{QString, QUrl};
use decode::{decode_path, encode, format_from_path, DecodeOptions, Decoded, Format};
use editcore::{
    adjust_json, Adjust, AdjustTarget, Document, Histogram, Layer, Mark, MarkKind, PickedColor, Rect,
    Selection, Size, Transform,
};
use image::RgbaImage;

mod ops;
mod geom;
mod config;
mod paths;
mod session;
pub(in crate::bridge) use geom::*;
pub(in crate::bridge) use config::*;
pub(in crate::bridge) use paths::*;
pub(in crate::bridge) use session::*;

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

/// Longest-edge cap for the preview bitmap, in PHYSICAL pixels. The preview
/// is rendered at the display size × device-pixel-ratio, so the cap must be
/// sized for physical resolution (8192 ≈ a 4096-logical-px preview at 2× DPI).
const MAX_PREVIEW_EDGE: u32 = 8192;
const MAX_DISPLAY_EDGE: u32 = 32767;
const MIN_ZOOM: f64 = 0.02;
const MAX_ZOOM: f64 = 16.0;
const ZOOM_STEP: f64 = 1.25;
/// How many picked colors the 拾色器 panel keeps for re-use.
const PICKER_HISTORY: usize = 12;

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
        #[qproperty(i32, selected_mark)]
        #[qproperty(QString, marks_json)]
        #[qproperty(bool, dirty)]
        #[qproperty(QString, file_name, cxx_name = "fileName")]
        #[qproperty(QString, file_size_str, cxx_name = "fileSizeStr")]
        #[qproperty(QString, checker_light, cxx_name = "checkerLight")]
        #[qproperty(QString, checker_dark, cxx_name = "checkerDark")]
        /// file:/// URL of the generated 16×16 checker tile BMP; QML tiles it
        /// with `Image.fillMode Tile` (zero repaint on zoom/resize).
        #[qproperty(QString, checker_tile_url, cxx_name = "checkerTileUrl")]
        #[qproperty(bool, open_in_new_window, cxx_name = "openInNewWindow")]
        #[qproperty(i32, save_quality, cxx_name = "saveQuality")]
        #[qproperty(QString, source_path, cxx_name = "sourcePath")]
        // ---- 后台解码 ----
        /// True while an image is being decoded off the UI thread; the QML
        /// poll timer calls `poll_decode()` until it flips back to false.
        #[qproperty(bool, loading)]
        /// Downsampling-first open for very large images (longest edge
        /// capped at 8192 px). `false` keeps the original resolution.
        #[qproperty(bool, large_downsample, cxx_name = "largeDownsample")]
        // ---- 调整颜色 (color adjustment) panel ----
        #[qproperty(bool, adjust_open, cxx_name = "adjustOpen")]
        #[qproperty(QString, adjust_target_name, cxx_name = "adjustTargetName")]
        #[qproperty(QString, adjust_values, cxx_name = "adjustValues")]
        #[qproperty(QString, adjust_histogram, cxx_name = "adjustHistogram")]
        // ---- 尺子 (ruler) ----
        /// Show ruler Y coordinates from the BOTTOM edge (origin at the
        /// bottom-left, Y grows upward) instead of the top-left origin.
        #[qproperty(bool, ruler_bottom_up, cxx_name = "rulerBottomUp")]
        // ---- 拾色器 (color picker) ----
        #[qproperty(bool, picker_open, cxx_name = "pickerOpen")]
        /// JSON of the last picked color ("" until the first pick).
        #[qproperty(QString, picker_color, cxx_name = "pickerColor")]
        /// JSON array of recent picks, newest first.
        #[qproperty(QString, picker_history, cxx_name = "pickerHistory")]
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

        /// Save quality 1-100 (settings panel slider). Lossy formats encode
        /// with it directly; lossless PNG maps it to compression effort.
        #[qinvokable]
        fn update_save_quality(self: Pin<&mut Self>, value: i32);

        /// Drain the background decode job: applies the finished result on
        /// the UI thread (no-op while the decode thread is still running).
        #[qinvokable]
        fn poll_decode(self: Pin<&mut Self>);

        /// Toggle downsampling-first open for very large images.
        #[qinvokable]
        fn update_large_downsample(self: Pin<&mut Self>, value: bool);

        /// Export a single layer's pixels as a PNG next to the source file,
        /// named `layout_<编号>.png` (编号 = numeric layer name or doc
        /// position). Returns the written path for logging.
        #[qinvokable]
        fn export_layer_png(self: Pin<&mut Self>, index: i32);

        #[qinvokable]
        fn set_viewport(self: Pin<&mut Self>, width: i32, height: i32);

        /// Screen device-pixel-ratio changed (Windows scaling adjusted, or the
        /// window moved to a different-density monitor). The preview must be
        /// re-rendered at the new physical resolution.
        #[qinvokable]
        fn set_dpr(self: Pin<&mut Self>, ratio: f64);

        #[qinvokable]
        fn zoom_in(self: Pin<&mut Self>);

        #[qinvokable]
        fn zoom_out(self: Pin<&mut Self>);

        /// Interactive zoom: apply a cumulative factor. QML queues wheel
        /// ticks during a gesture (stretching the stale preview to the
        /// predicted display size) and calls this ONCE per gesture — one
        /// full recomposite instead of one per wheel notch.
        #[qinvokable]
        fn zoom_step(self: Pin<&mut Self>, factor: f64);

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
        fn delete_base_layer(self: Pin<&mut Self>);

        #[qinvokable]
        fn toggle_base_visible(self: Pin<&mut Self>);

        #[qinvokable]
        fn translate_selected(self: Pin<&mut Self>, dx: i32, dy: i32);

        // ---- 标记（markup annotations）----

        /// Select a mark (-1 = none); selection survives refreshes.
        #[qinvokable]
        fn select_mark(self: Pin<&mut Self>, index: i32);

        /// Add a Rect/Ellipse/Arrow mark from canvas-space geometry.
        /// kind: 0 rect, 1 ellipse, 2 arrow. Colors are "#rrggbb".
        #[qinvokable]
        fn add_shape_mark(
            self: Pin<&mut Self>,
            kind: i32,
            x0: f64,
            y0: f64,
            x1: f64,
            y1: f64,
            stroke: i32,
            color: &QString,
            fill: bool,
            fill_color: &QString,
        );

        /// Add a freehand Brush mark; `points` is "x,y x,y …" image coords.
        #[qinvokable]
        fn add_brush_mark(
            self: Pin<&mut Self>,
            points: &QString,
            stroke: i32,
            color: &QString,
        );

        /// Add a Mosaic brush mark; `cell` is the pixelation block size
        /// (the "strength"). The stroke path is "x,y x,y …" image coords.
        #[qinvokable]
        fn add_mosaic_mark(self: Pin<&mut Self>, points: &QString, stroke: i32, cell: i32);

        /// Add a Mosaic region mark: the whole rect is pixelated. (x, y) is
        /// one corner, (x1, y1) the opposite one, in image coords.
        #[qinvokable]
        fn add_mosaic_rect(
            self: Pin<&mut Self>,
            x0: f64,
            y0: f64,
            x1: f64,
            y1: f64,
            cell: i32,
        );

        /// Add a Text mark anchored at (x, y) top-left; the natural text
        /// size is measured with the named font and stored as the mark rect.
        #[qinvokable]
        fn add_text_mark(
            self: Pin<&mut Self>,
            x: f64,
            y: f64,
            text: &QString,
            font: &QString,
            font_size: i32,
            bold: bool,
            italic: bool,
            color: &QString,
        );

        /// Restyle the selected Text mark (family / size / faux styles /
        /// underline / alignment), undoable.
        #[qinvokable]
        fn update_text_style(
            self: Pin<&mut Self>,
            font: &QString,
            font_size: i32,
            bold: bool,
            italic: bool,
            underline: bool,
            align: i32,
        );

        /// Hide (or restore, -1) a mark in the preview while its text is
        /// edited inline in QML. Plain preview state, not undoable.
        #[qinvokable]
        fn set_editing_mark(self: Pin<&mut Self>, index: i32);

        /// Replace the selected Text mark's content, re-measuring the rect,
        /// undoable.
        #[qinvokable]
        fn update_text_content(self: Pin<&mut Self>, text: &QString);

        /// Restyle the selected Mosaic mark's block size ("strength"),
        /// undoable.
        #[qinvokable]
        fn update_mark_cell(self: Pin<&mut Self>, cell: i32);

        /// Begin a live move/resize of the selected mark (history-free).
        #[qinvokable]
        fn mark_drag_begin(self: Pin<&mut Self>);

        /// Live-update the dragged mark to new bounds (image coords).
        #[qinvokable]
        fn mark_drag_update(self: Pin<&mut Self>, x: f64, y: f64, w: f64, h: f64);

        /// Commit (or roll back) the live drag as a single history op.
        #[qinvokable]
        fn mark_drag_end(self: Pin<&mut Self>, commit: bool);

        /// Delete the selected mark (undoable).
        #[qinvokable]
        fn delete_selected_mark(self: Pin<&mut Self>);

        /// Show/hide a mark (undoable via UpdateMark).
        #[qinvokable]
        fn set_mark_visible(self: Pin<&mut Self>, index: i32, visible: bool);

        /// Restyle the selected mark (width / colors / fill), undoable.
        #[qinvokable]
        fn update_mark_style(
            self: Pin<&mut Self>,
            stroke: i32,
            color: &QString,
            fill: bool,
            fill_color: &QString,
        );

        // ---- 调整颜色（macOS 预览风格） ----
        /// Open the color-adjust panel and capture its scope: the selected
        /// layer, else the base photo. Parameters start neutral.
        #[qinvokable]
        fn adjust_begin(self: Pin<&mut Self>);

        /// Live parameter update (history-free; document pixels untouched).
        /// `name` is one of exposure/contrast/highlights/shadows/saturation/
        /// temperature/tint/sepia/sharpness/black/gamma/white.
        #[qinvokable]
        fn adjust_set(self: Pin<&mut Self>, name: &QString, value: f64);

        /// Levels: `black` / `gamma` / `white` (0..1, > 0, 0..1).
        #[qinvokable]
        fn adjust_set_levels(self: Pin<&mut Self>, black: f64, gamma: f64, white: f64);

        /// Auto levels from the scope's own histogram (percentile clip).
        #[qinvokable]
        fn adjust_auto_levels(self: Pin<&mut Self>);

        /// Back to neutral (pixels were never modified; keeps the panel open).
        #[qinvokable]
        fn adjust_reset(self: Pin<&mut Self>);

        /// Bake the parameters into the scope's pixels as ONE undoable op and
        /// close the panel (the macOS-friendly "close applies" flow).
        #[qinvokable]
        fn adjust_commit(self: Pin<&mut Self>);

        /// Close the panel discarding the parameters.
        #[qinvokable]
        fn adjust_cancel(self: Pin<&mut Self>);

        // ---- 尺子 / 拾色器 ----

        /// Ruler coordinate direction: `true` = origin at the bottom-left
        /// (Y grows upward), `false` = top-left origin (screen order). Pure
        /// display preference; persisted to settings.ini.
        #[qinvokable]
        fn update_ruler_bottom_up(self: Pin<&mut Self>, value: bool);

        /// Open the color-picker panel (clicks on the canvas then pick pixels).
        #[qinvokable]
        fn picker_begin(self: Pin<&mut Self>);

        /// Close the color-picker panel (the picked colors are kept).
        #[qinvokable]
        fn picker_close(self: Pin<&mut Self>);

        /// Sample the composed pixel at image coordinates `(x, y)` and publish
        /// it as the current color + push it onto the recent list.
        #[qinvokable]
        fn picker_pick(self: Pin<&mut Self>, x: i32, y: i32);

        /// Re-select one of the recent colors (does not add a new entry).
        #[qinvokable]
        fn picker_select(self: Pin<&mut Self>, index: i32);

        /// Clear the recent-color list.
        #[qinvokable]
        fn picker_clear(self: Pin<&mut Self>);

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

        /// Paste the process-wide layer clipboard as a new layer. Returns
        /// `false` when nothing was pasted (empty clipboard / no document), so
        /// the QML side only enters the "floating placement" state on success.
        #[qinvokable]
        fn paste_clipboard(self: Pin<&mut Self>) -> bool;

        /// Pop a file handed over by another process ("" when none). QML polls
        /// this from the primary window and opens it either in a new window or
        /// in place, following the "在单独的新窗口显示" setting. This is what
        /// makes an Explorer double-click reuse the running instance instead of
        /// starting a second process (and a second, empty layer clipboard).
        #[qinvokable]
        fn take_external_open(self: Pin<&mut Self>) -> QString;

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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            checker_light: "#ffffff".to_string(),
            checker_dark: "#cccccc".to_string(),
            open_in_new_window: true,
            save_quality: 92,
            ruler_bottom_up: false,
            max_dimension: 8192,
        }
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
    /// Selected markup annotation (-1 = none); survives refreshes.
    selected_mark: i32,
    /// JSON list of marks for the QML overlay: bounds, kind, style.
    marks_json: QString,
    dirty: bool,
    /// Loaded file's display name and human-readable size for the status bar.
    file_name: QString,
    file_size_str: QString,
    /// User preferences (checkerboard colors, open-in-new-window behavior).
    checker_light: QString,
    checker_dark: QString,
    /// file:/// URL of the generated 16×16 checker tile BMP ("" on failure).
    checker_tile_url: QString,
    /// Alternates 0/1 per regeneration: the QML Image only reloads when the
    /// source URL string changes, so the filename toggles a/b on color edits.
    /// `Cell` because interior mutability is required behind `Pin<&mut Self>`.
    checker_tile_slot: std::cell::Cell<u8>,
    open_in_new_window: bool,
    /// Save quality 1-100 (mirrors the settings.ini value for QML).
    save_quality: i32,
    /// Full path of the loaded file ("" when no file), for the clickable
    /// status-bar path button.
    source_path: QString,
    /// "调整颜色" panel state published to QML.
    adjust_open: bool,
    /// Scope label shown in the panel title ("背景" / layer name).
    adjust_target_name: QString,
    /// JSON of the live parameters (slider positions + level handles).
    adjust_values: QString,
    /// JSON of the 256-bin RGB histogram of the current preview.
    adjust_histogram: QString,
    /// Ruler Y direction (see [`AppConfig::ruler_bottom_up`]).
    ruler_bottom_up: bool,
    /// True while an image decodes off the UI thread (see `loading`).
    loading: bool,
    /// Downsampling-first open for very large images (see `large_downsample`).
    large_downsample: bool,
    /// In-flight background decode. The worker thread only sends into the
    /// channel; the UI thread drains it from `poll_decode()`. Replacing the
    /// option drops the old receiver, so a superseded job's result is simply
    /// discarded (the thread itself cannot touch any Qt state).
    decode_job: RefCell<Option<mpsc::Receiver<DecodeMsg>>>,
    /// "拾色器" panel: open flag + the last pick ("" before the first one).
    picker_open: bool,
    picker_color: QString,
    /// Recent picks, newest first, as a JSON array.
    picker_history: QString,
    /// Recent picks in decoded form (the JSON is rebuilt from this).
    picker_recent: RefCell<Vec<PickedColor>>,
    paste_counter: RefCell<u32>,
    /// Opacity of the selected layer before the current live slider drag, so
    /// the committed history op can record the true pre-drag value.
    opacity_live: RefCell<Option<f32>>,
    /// Offset of the selected layer when a drag started; `None` while not
    /// dragging. `drag_translate_update` mutates the offset without touching
    /// history, `drag_translate_end` commits a single history op.
    drag_start_offset: RefCell<Option<(i32, i32)>>,
    /// The selected mark and its pre-drag geometry; live updates map
    /// geometry without history, end commits one UpdateMark op.
    mark_drag: RefCell<Option<(usize, Mark)>>,
    session: RefCell<Option<Session>>,
    viewport: RefCell<Size>,
    /// Screen device-pixel-ratio pushed by QML (1.0 at 100% scaling). The
    /// preview bitmap is rendered at display size × dpr so it stays crisp on
    /// high-DPI displays. `Cell` because `set_dpr` mutates it behind
    /// `Pin<&mut Self>`; the render path reads it.
    dpr: std::cell::Cell<f64>,
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
            selected_mark: -1,
            marks_json: QString::default(),
            dirty: false,
            file_name: QString::default(),
            file_size_str: QString::default(),
            checker_light: QString::from(cfg.checker_light.as_str()),
            checker_dark: QString::from(cfg.checker_dark.as_str()),
            // Generate the tile eagerly so the first paint already has it;
            // a failed write just leaves the URL empty (checker hidden).
            checker_tile_url: QString::from(
                write_checker_tile_bmp(&cfg.checker_light, &cfg.checker_dark, 0)
                    .unwrap_or_default()
                    .as_str(),
            ),
            checker_tile_slot: std::cell::Cell::new(0),
            open_in_new_window: cfg.open_in_new_window,
            save_quality: cfg.save_quality as i32,
            source_path: QString::default(),
            adjust_open: false,
            adjust_target_name: QString::default(),
            adjust_values: QString::default(),
            adjust_histogram: QString::default(),
            ruler_bottom_up: cfg.ruler_bottom_up,
            loading: false,
            large_downsample: cfg.max_dimension != 0,
            decode_job: RefCell::new(None),
            picker_open: false,
            picker_color: QString::default(),
            picker_history: QString::from("[]"),
            picker_recent: RefCell::new(Vec::new()),
            paste_counter: RefCell::new(0),
            opacity_live: RefCell::new(None),
            drag_start_offset: RefCell::new(None),
            mark_drag: RefCell::new(None),
            session: RefCell::new(None),
            viewport: RefCell::new(Size::new(0, 0)),
            dpr: std::cell::Cell::new(1.0),
        }
    }
}


impl qobject::Bridge {
    /// Open the previous/next image file in the loaded file's folder. The
    /// dirty check happens in QML (confirm overlay) before calling this.
    /// Register a ProgId + "Open with" entries under HKCU (no admin needed),
    /// then open the Windows default-apps settings page so the user can pick
    /// ImageEdit as the default viewer. Windows does not allow setting the
    /// default programmatically (UserChoice is hash-protected by design).
    fn persist_config(&self) {
        AppConfig {
            checker_light: self.checker_light().to_string(),
            checker_dark: self.checker_dark().to_string(),
            open_in_new_window: *self.open_in_new_window(),
            save_quality: (*self.save_quality()).clamp(1, 100) as u8,
            ruler_bottom_up: *self.ruler_bottom_up(),
            max_dimension: if *self.large_downsample() { 8192 } else { 0 },
        }
        .save();
    }

    /// Re-render the checker tile after a color change and publish the new
    /// (filename-alternated) URL so the QML `Image` reloads it.
    fn regenerate_checker_tile(mut self: Pin<&mut Self>) {
        let slot = (self.checker_tile_slot.get() + 1) % 2;
        self.checker_tile_slot.set(slot);
        let light = self.checker_light().to_string();
        let dark = self.checker_dark().to_string();
        if let Some(url) = write_checker_tile_bmp(&light, &dark, slot) {
            self.as_mut().set_checker_tile_url(QString::from(url.as_str()));
        }
    }

    /// Export a single layer's pixels as `layout_<编号>.png` next to the
    /// source file. 编号 = the layer's numeric auto-name ("1", "2", ...) or
    /// its 1-based stack position when the name isn't numeric. Existing
    /// files are never overwritten — `_2`, `_3`... suffixes are appended.
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
                (name, size, path.to_string_lossy().to_string())
            });
        match info {
            Some((name, size, full)) => {
                self.as_mut().set_file_name(QString::from(name.as_str()));
                self.as_mut()
                    .set_file_size_str(QString::from(size.as_str()));
                self.as_mut().set_source_path(QString::from(full.as_str()));
            }
            None => {
                self.as_mut().set_file_name(QString::from(""));
                self.as_mut().set_file_size_str(QString::from(""));
                self.as_mut().set_source_path(QString::from(""));
            }
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

    /// Eye icon in the layers panel: toggle one layer's visibility by index.
    /// Drag-reorder in the layers panel: one undoable step via core
    /// `move_layer(from, to)`; the selection follows the moved layer.
    // ---- 调整颜色（macOS 预览风格） ----

    /// Open the panel and capture its scope. Parameters start neutral, so
    /// opening the panel never changes the document.
    /// Live parameter update: history-free, document pixels untouched.
    /// Auto levels from the scope's own histogram (percentile clip), like
    /// the "自动色阶" button in the screenshot.
    /// Bake the parameters into the scope's pixels as one undoable op and
    /// close the panel (macOS Preview's "closing applies" flow).
    // ---- 尺子 / 拾色器 ----

    /// Sample the COMPOSED pixel (layers + opacity + baked adjustments) at
    /// image coordinates. Marks are annotations, not pixels, so they are not
    /// sampled — the picker reports the artwork's color.
    /// Rebuild the recent-list JSON from `picker_recent`.
    fn publish_picker_history(mut self: Pin<&mut Self>) {
        let json = {
            let recent = self.picker_recent.borrow();
            let parts: Vec<String> = recent.iter().map(|c| c.to_json()).collect();
            format!("[{}]", parts.join(","))
        };
        if self.picker_history().to_string() != json {
            self.as_mut().set_picker_history(QString::from(json.as_str()));
        }
    }

    // ---- 剪贴板（跨文档复制粘贴） ----

    /// Pop a file handed to us by another process (single-instance hand-off).
    /// Returns a `file:///…` URL, or "" when nothing is waiting.
    // ---- 图层属性 ----

    /// Scale the selected layer to `width × height` (Lanczos3 via core
    /// `scale_layer`). keep_ratio: if either dimension is <= 0, derive it from
    /// the other using the layer's current aspect. The layer offset stays.
    // ---- 图层缩放拖拽实时预览 ----
    //
    // The gesture keeps the document bitmap untouched: `begin` snapshots the
    // layer, `update` only records the requested size/offset (the preview
    // stretches the snapshot via composite_live_scale), and `end` applies the
    // real, undoable resize exactly once. This keeps every drag tick at
    // O(the dragged layer's preview pixels) instead of a full-resolution
    // resample + full preview recompose.

    // ---- 图层锁定 ----

    /// Eye toggle on the base row: hide/show the background (undoable).
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

    /// Decode options derived from the current preferences.
    fn decode_opts(&self) -> DecodeOptions {
        DecodeOptions {
            max_pixels: Some(300_000_000),
            max_dimension: if *self.large_downsample() {
                Some(DOWNSAMPLE_EDGE)
            } else {
                None
            },
        }
    }

    /// Interactive load: hand the file to the worker thread and return
    /// immediately, so even a huge image never freezes the UI. The current
    /// document stays visible until the decode lands (or fails).
    fn load_path(mut self: Pin<&mut Self>, path: &Path) {
        println!(
            "{{\"phase\":\"ui\",\"event\":\"load\",\"file\":\"{}\"}}",
            path.to_string_lossy().replace('\\', "/")
        );
        let (tx, rx) = mpsc::channel::<DecodeMsg>();
        let thread_path = path.to_path_buf();
        let opts = self.decode_opts();
        std::thread::spawn(move || {
            let t0 = Instant::now();
            let result = decode_path(&thread_path, opts).map_err(|e| e.to_string());
            let ms = t0.elapsed().as_millis().min(i32::MAX as u128) as i32;
            let _ = tx.send(DecodeMsg {
                path: thread_path,
                result: result.map(|d| (d, ms)),
            });
        });
        // Dropping the previous receiver (if any) discards its result.
        *self.decode_job.borrow_mut() = Some(rx);
        self.as_mut().set_loading(true);
        self.as_mut()
            .set_status(QString::from("正在打开…"));
    }

    /// Drain the finished decode job (QML poll timer drives this). Applying
    /// happens here on the UI thread, so no Qt state is ever touched from
    /// the worker.
    /// Synchronous load for automation (the script ops that follow `start()`
    /// assume the document already exists).
    fn load_path_sync(mut self: Pin<&mut Self>, path: &Path) {
        println!(
            "{{\"phase\":\"ui\",\"event\":\"load\",\"file\":\"{}\"}}",
            path.to_string_lossy().replace('\\', "/")
        );
        let viewport = *self.viewport.borrow();
        let opts = self.decode_opts();
        match Session::open(path, viewport, (*self.save_quality()).clamp(1, 100) as u8, opts) {
            Ok((session, ms)) => {
                if let Some((w, h)) = session.downsampled_from {
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"downsampled\",\"from\":\"{}x{}\",\"to\":\"{}x{}\"}}",
                        w, h, session.doc.canvas.width, session.doc.canvas.height
                    );
                }
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

    // ---- 标记（markup annotations）----

    fn parse_color(s: &str) -> [u8; 4] {
        let t = s.trim().trim_start_matches('#');
        if t.len() < 6 {
            return [255, 0, 0, 255];
        }
        let byte = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).unwrap_or(255);
        [byte(0), byte(2), byte(4), 255]
    }

    fn mark_kind(kind: i32) -> MarkKind {
        match kind {
            1 => MarkKind::Ellipse,
            2 => MarkKind::Arrow,
            _ => MarkKind::Rect,
        }
    }

    /// Shared tail of the "add mark" invokables: select the new (last)
    /// mark and republish state.
    fn select_newest_mark(mut self: Pin<&mut Self>) {
        let count = {
            let guard = self.session.borrow();
            guard
                .as_ref()
                .map(|s| s.doc.marks.len())
                .unwrap_or(0)
        } as i32;
        self.as_mut().set_selected_mark(count - 1);
        self.as_mut().refresh_preview();
    }

    fn refresh_preview(self: Pin<&mut Self>) {
        self.refresh_preview_inner(false);
    }

    /// Zoom-entry refresh (wheel ticks, slider, presets, zoom buttons). Only
    /// these may reuse the previous frame when the preview target is unchanged
    /// (the >100% regime); every other refresh path renders for real.
    fn refresh_preview_zoom(self: Pin<&mut Self>) {
        self.refresh_preview_inner(true);
    }

    fn refresh_preview_inner(mut self: Pin<&mut Self>, zoom_only: bool) {
        let render_t0 = Instant::now();
        let outcome = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                Some(session) => session.refresh(zoom_only, self.dpr.get()),
                None => return,
            }
        };
        let render_ms = render_t0.elapsed().as_millis().min(i32::MAX as u128) as i32;
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
                marks_json,
                dirty,
                adjust_open,
                adjust_target_name,
                adjust_values,
                adjust_histogram,
            } => {
                // The status bar's left side already shows the file name and
                // size; a second resolution readout here was redundant, so
                // only keep the preview-degraded hint when it matters. The
                // preview is rendered in PHYSICAL pixels (display × dpr), so
                // "degraded" means it is smaller than the physical display
                // size (zoomed past the native resolution or past the memory
                // cap) — not merely different from the logical display size.
                let dpr = self.dpr.get();
                let phys_w = ((display.width as f64 * dpr).round() as u32).max(1);
                let phys_h = ((display.height as f64 * dpr).round() as u32).max(1);
                let degraded = preview.width < phys_w || preview.height < phys_h;
                let mut status = if degraded {
                    format!(
                        "{}  (预览 {}x{})",
                        if fit { "适应窗口" } else { "缩放" },
                        preview.width,
                        preview.height
                    )
                } else {
                    String::new()
                };
                // Persistent downsampling hint: refreshes rebuild the status
                // text, so the "save resolution differs" notice must live here
                // or the first viewport change would wipe it.
                if let Some((w, h)) = self
                    .session
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.downsampled_from)
                {
                    let hint = format!("已降采样 {}×{} → {}×{}", w, h, canvas.width, canvas.height);
                    if status.is_empty() {
                        status = hint;
                    } else {
                        status = format!("{status} · {hint}");
                    }
                }
                // Decoder hints (PSD flattened preview) survive refreshes the
                // same way the downsampling notice does.
                if let Some(note) = self.session.borrow().as_ref().and_then(|s| s.note.clone()) {
                    if status.is_empty() {
                        status = note;
                    } else {
                        status = format!("{status} · {note}");
                    }
                }
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
                // Publish JSON properties only when they actually changed:
                // QML re-parses them (and rebuilds the list panels) on every
                // change signal, which churned every live-drag tick.
                if self.layers_json().to_string() != layers_json {
                    self.as_mut()
                        .set_layers_json(QString::from(layers_json.as_str()));
                }
                if self.marks_json().to_string() != marks_json {
                    self.as_mut()
                        .set_marks_json(QString::from(marks_json.as_str()));
                }
                // "调整颜色" panel state: only republished when it actually
                // changed (the histogram JSON changes on every slider tick,
                // which is what redraws the graph).
                if *self.adjust_open() != adjust_open {
                    self.as_mut().set_adjust_open(adjust_open);
                }
                if self.adjust_target_name().to_string() != adjust_target_name {
                    self.as_mut()
                        .set_adjust_target_name(QString::from(adjust_target_name.as_str()));
                }
                if self.adjust_values().to_string() != adjust_values {
                    self.as_mut()
                        .set_adjust_values(QString::from(adjust_values.as_str()));
                }
                if self.adjust_histogram().to_string() != adjust_histogram {
                    self.as_mut()
                        .set_adjust_histogram(QString::from(adjust_histogram.as_str()));
                }
                if *self.selected_layer() < 0 || *self.selected_layer() >= layer_count {
                    self.as_mut().set_selected_layer(-1);
                }
                {
                    let count = {
                        let guard = self.session.borrow();
                        guard
                            .as_ref()
                            .map(|s| s.doc.marks.len())
                            .unwrap_or(0)
                    } as i32;
                    if *self.selected_mark() < -1 || *self.selected_mark() >= count {
                        self.as_mut().set_selected_mark(-1);
                    }
                }
                self.as_mut().set_default_save_url(QString::from(save_url.as_str()));
                self.as_mut().set_status(QString::from(status.as_str()));
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"preview-ready\",\"zoom\":{zoom_percent},\"display\":\"{}x{}\",\"preview\":\"{}x{}\",\"undo\":{can_undo},\"redo\":{can_redo},\"ms\":{render_ms}}}",
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
