use core::pin::Pin;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use cxx_qt_lib::{QString, QUrl};
use decode::{
    decode_path, encode, encode_document, format_from_path, DecodeOptions, Decoded, Format,
    SaveFormat,
};
use editcore::{
    adjust_json, Adjust, AdjustTarget, BoundsSolve, Document, Field, Histogram, Layer,
    LayerTransform, Mark, MarkKind, PickedColor, QuickSelectOptions, Rect, ReferencePoint, SelOps,
    SelectionMask, SelectionState, Size, Transform, WandOptions,
};
use image::RgbaImage;

mod ops;
mod geom;
mod config;
mod paths;
mod prewarm;
mod session;
pub(in crate::bridge) use geom::*;
pub(in crate::bridge) use config::*;
pub(in crate::bridge) use paths::*;
pub(in crate::bridge) use session::*;

/// Kick off the decode of the first file before the window exists, so the
/// decode overlaps Qt/QML start-up instead of following it. See the module
/// docs for the measurements behind this.
pub fn prewarm_open(path: &Path) {
    prewarm::spawn(path);
}

pub static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// Session ids make preview temp filenames unique per document, which is
/// required once multiple windows (documents) live in the same process.
static SESSION_SEQ: AtomicU32 = AtomicU32::new(0);

pub fn mark_process_start() {
    let _ = PROCESS_START.set(Instant::now());
}

/// 记住"用户双击那一刻"的光标位置（窗口稍后才摆位，见 shell.rs 的注释）。
pub fn remember_cursor_start() {
    ops::shell::remember_cursor_start();
}

fn elapsed_ms() -> i32 {
    PROCESS_START
        .get()
        .map(|t| t.elapsed().as_millis().min(i32::MAX as u128) as i32)
        .unwrap_or(-1)
}

/// Decode options shared by the bridge and the prewarm path — both must agree,
/// otherwise a prewarmed bitmap could differ from a normally opened one (for
/// example by skipping the downsampling-first cap).
pub(in crate::bridge) fn decode_options(large_downsample: bool) -> DecodeOptions {
    DecodeOptions {
        max_pixels: Some(300_000_000),
        max_dimension: if large_downsample {
            Some(DOWNSAMPLE_EDGE)
        } else {
            None
        },
    }
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
        /// 多选：选中图层的稳定 id 列表（JSON 数组），面板据此高亮。
        #[qproperty(QString, layer_selection_ids, cxx_name = "layerSelectionIds")]
        /// 多选：选中图层数量（面板标题 / 批量操作条用）。
        #[qproperty(i32, selected_layer_count, cxx_name = "selectedLayerCount")]
        /// 文档像素版本（`Document::edit_epoch`）。面板把它混进列表签名，
        /// 像素一改就重建列表 → 缩略图跟着刷新。
        #[qproperty(i32, layer_epoch, cxx_name = "layerEpoch")]
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
        /// Bumped every time a NEW document is installed (open / navigate).
        /// QML uses it to resize the window to the image once per load —
        /// `image_width` alone cannot be used because crop/rotate/canvas
        /// resize also change it.
        #[qproperty(i32, load_serial, cxx_name = "loadSerial")]
        // ---- 后台解码 ----
        /// True while an image is being decoded off the UI thread; the QML
        /// poll timer calls `poll_decode()` until it flips back to false.
        #[qproperty(bool, loading)]
        /// PSD stage-2 lock: ALL editing tools are disabled until the layer
        /// stack is installed (or the user skips / the parse fails). Viewing,
        /// zoom, pan and opening files stay available.
        #[qproperty(bool, editing_locked, cxx_name = "editingLocked")]
        /// True while a stage-2 layer job is still being polled; keeps the
        /// existing 50 ms decode timer alive after `loading` turns false.
        #[qproperty(bool, psd_layers_pending, cxx_name = "psdLayersPending")]
        /// Status text for the layer panel / status bar ("已加载 k/N 层…").
        #[qproperty(QString, psd_progress, cxx_name = "psdProgress")]
        /// One-shot PSD notice for the info overlay (degraded layers /
        /// composite-only / downsampled read-only). Never blocks saving.
        #[qproperty(QString, psd_notice, cxx_name = "psdNotice")]
        /// True while a save is being encoded on the worker thread; keeps the
        /// existing 50 ms timer alive and gates the save entry points.
        #[qproperty(bool, saving)]
        /// C6: panel thumbnails are enqueued by the stage-2 worker and written
        /// in batches of <=6 rows per 50 ms tick (single UI occupancy <=16ms).
        #[qproperty(bool, thumbs_pending, cxx_name = "thumbsPending")]
        /// Bumped after every thumbnail batch so QML re-queries the URLs.
        #[qproperty(i32, thumb_epoch, cxx_name = "thumbEpoch")]
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
        // ---- 选区（PS 式：矩形 / 椭圆 / 套索 / 多边形套索）----
        /// A selection exists and covers at least one pixel.
        #[qproperty(bool, sel_active, cxx_name = "selActive")]
        /// Bumped on every selection change; QML re-pulls the outline on it.
        #[qproperty(i32, sel_version, cxx_name = "selVersion")]
        /// Bounds of the selection in IMAGE coordinates (drives the handles).
        #[qproperty(i32, sel_x, cxx_name = "selX")]
        #[qproperty(i32, sel_y, cxx_name = "selY")]
        #[qproperty(i32, sel_w, cxx_name = "selW")]
        #[qproperty(i32, sel_h, cxx_name = "selH")]
        /// Current 边界 / 平滑 / 羽化 parameters (shown in the right-click menu).
        #[qproperty(i32, sel_border, cxx_name = "selBorder")]
        #[qproperty(i32, sel_smooth, cxx_name = "selSmooth")]
        #[qproperty(i32, sel_feather, cxx_name = "selFeather")]
        /// Quick mask (Q): dim everything outside the selection in the preview.
        #[qproperty(bool, quick_mask, cxx_name = "quickMask")]
        /// Last used selection tool (settings.ini). QML seeds its `selTool`
        /// from this so a restart keeps the marquee type the user was using.
        #[qproperty(QString, sel_tool_saved, cxx_name = "selToolSaved")]
        /// 魔术棒参数（settings.ini）：`tol,contiguous,sample_all`。
        #[qproperty(QString, sel_wand_saved, cxx_name = "selWandSaved")]
        /// 修复 / 油漆工具与参数（settings.ini，`|` 分隔）。
        #[qproperty(QString, paint_spec_saved, cxx_name = "paintSpecSaved")]
        // ---- P0-4 智能参考线 / P0-5 数值面板 ----
        /// D13：数值面板显示状态（settings.ini 记忆）。
        #[qproperty(bool, transform_panel_visible, cxx_name = "transformPanelVisible")]
        /// D16：只控制提示线绘制，不短路吸附求解。
        #[qproperty(bool, show_guides, cxx_name = "showGuides")]
        /// D16：关闭后吸附求解直接返回空结果。
        #[qproperty(bool, snap_enabled, cxx_name = "snapEnabled")]
        /// 每帧随 refresh 下发的参考线 / 距离标签 JSON。
        #[qproperty(QString, guides_json, cxx_name = "guidesJson")]
        /// 选中图层的外接框参数（数值面板只读回显）。
        #[qproperty(QString, transform_values_json, cxx_name = "transformValuesJson")]
        // ---- P0-2b2 Tier 1 GPU 快路径 ----
        #[qproperty(bool, gpu_live, cxx_name = "gpuLive")]
        #[qproperty(QString, gpu_static_url, cxx_name = "gpuStaticUrl")]
        #[qproperty(QString, gpu_active_url, cxx_name = "gpuActiveUrl")]
        /// 活动层显示矩形 + 不透明度 JSON（每帧只改这几个数）。
        #[qproperty(QString, gpu_rect_json, cxx_name = "gpuRectJson")]
        #[namespace = "imageedit"]
        type Bridge = super::BridgeRust;

        #[qinvokable]
        fn start(self: Pin<&mut Self>);

        #[qinvokable]
        fn image_shown(self: Pin<&mut Self>);

        /// 新的一帧预览已经写好盘（`preview_url` 刚更新）。
        ///
        /// QML 靠它把「后台那张 Image 去加载新帧」这件事从声明式绑定里拆
        /// 出来：绑定会在换台时把刚显示的帧再加载一遍，两次解码互相取消，
        /// 画面就一直停在旧帧。信号名由我们自己定，不依赖属性名的推导规则。
        #[qsignal]
        fn preview_advanced(self: Pin<&mut Self>);

        #[qinvokable]
        fn mark(self: Pin<&mut Self>, name: &QString);

        #[qinvokable]
        fn open_file(self: Pin<&mut Self>, url: &QUrl);

        // ---- 图片浏览 / 设置 ----

        /// Open the previous (-1) or next (+1) image in the same folder.
        #[qinvokable]
        fn navigate(self: Pin<&mut Self>, dir: i32);

        /// 只读文件头取像素尺寸（"宽x高"，失败返回空串）。窗口在**显示之前**
        /// 用它算最终尺寸，避免"先弹一个空窗口、解码完再跳成图片大小"。
        #[qinvokable]
        fn probe_image_size(self: Pin<&mut Self>, target: &QString) -> QString;

        /// 光标所在显示器的信息，格式 `设备名\t光标屏内偏移X\t偏移Y\t物理宽\t物理高`
        /// （偏移是**物理**像素，相对该显示器左上角）。失败返回空串。
        ///
        /// ★这里只报事实、不做换算：Qt 的逻辑坐标系是按每块屏各自的 dpr 分段的，
        /// "全局物理坐标 ÷ dpr" 在多屏（尤其混合缩放）下会算到别的屏上去。QML
        /// 侧必须先用设备名（Windows 上是 `\\.\DISPLAYn`）匹配到那块 QScreen，
        /// 再用**那块屏**的 dpr 和 virtualX/virtualY 换算。
        #[qinvokable]
        fn cursor_monitor(self: Pin<&mut Self>) -> QString;

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

        /// Drain PSD stage-2 progress / result messages on the UI thread.
        /// Called by the same 50 ms timer as `poll_decode`.
        #[qinvokable]
        fn poll_psd_layers(self: Pin<&mut Self>);

        /// Escape hatch: drop the stage-2 job and edit the composite preview
        /// immediately (C2-b). Big/slow files must never lock the user out.
        #[qinvokable]
        fn skip_psd_layers(self: Pin<&mut Self>);

        /// One-shot status hint when a locked-UI edit is attempted.
        #[qinvokable]
        fn psd_locked_hint(self: Pin<&mut Self>);

        /// Drain the background save job: encoding runs off the UI thread
        /// (C5), only the atomic rename happens here.
        #[qinvokable]
        fn poll_save(self: Pin<&mut Self>);

        /// Write at most 6 pre-rendered layer thumbnails per 50 ms tick (C6).
        #[qinvokable]
        fn poll_thumbs(self: Pin<&mut Self>);

        /// Dismiss the PSD info overlay (degraded / loading / read-only).
        #[qinvokable]
        fn clear_psd_notice(self: Pin<&mut Self>);

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

        // ---- 选区（PS 式，画布坐标）----

        /// Replace / combine the selection with a rectangle given by two
        /// opposite corners (image coords). `mode`: 0 replace, 1 add,
        /// 2 subtract, 3 intersect.
        #[qinvokable]
        fn sel_set_rect(
            self: Pin<&mut Self>,
            x0: f64,
            y0: f64,
            x1: f64,
            y1: f64,
            mode: i32,
        );

        /// Same as `sel_set_rect` for an ellipse inscribed in the drag box.
        #[qinvokable]
        fn sel_set_ellipse(
            self: Pin<&mut Self>,
            x0: f64,
            y0: f64,
            x1: f64,
            y1: f64,
            mode: i32,
        );

        /// Freehand lasso / polygonal lasso: `"x,y x,y …"` image coords.
        #[qinvokable]
        fn sel_set_polygon(self: Pin<&mut Self>, points: &QString, mode: i32);

        #[qinvokable]
        fn sel_select_all(self: Pin<&mut Self>);

        #[qinvokable]
        fn sel_clear(self: Pin<&mut Self>);

        /// 选择反选 (Ctrl+Shift+I).
        #[qinvokable]
        fn sel_invert(self: Pin<&mut Self>);

        /// 边界: ring of total width `k` centered on the current edge.
        #[qinvokable]
        fn sel_set_border(self: Pin<&mut Self>, k: i32);

        /// 平滑: round off the staircase edge, radius `k`.
        #[qinvokable]
        fn sel_set_smooth(self: Pin<&mut Self>, k: i32);

        /// 羽化: soft edge of radius `r` for subsequent edits.
        #[qinvokable]
        fn sel_set_feather(self: Pin<&mut Self>, r: i32);

        /// Closed rings of the selection in image coords, `[[[x,y],…],…]`.
        #[qinvokable]
        fn sel_outline(self: Pin<&mut Self>) -> QString;

        /// Whether the image-space point is inside the selection (right-click
        /// hit test for the context menu).
        #[qinvokable]
        fn sel_contains(self: Pin<&mut Self>, x: i32, y: i32) -> bool;

        /// Move the selection itself (dragging inside it, PS default).
        #[qinvokable]
        fn sel_translate(self: Pin<&mut Self>, dx: i32, dy: i32);

        /// Resize the selection to a new bounding box (dragging a handle).
        /// Negative `w` / `h` = the handle crossed the opposite edge, so the
        /// selection mirrors on that axis instead of collapsing.
        #[qinvokable]
        fn sel_resize(self: Pin<&mut Self>, x: f64, y: f64, w: f64, h: f64);

        /// 按下选区手柄时调用：清零「已镜像」标记，下一次越过对边重新镜像。
        #[qinvokable]
        fn sel_resize_begin(self: Pin<&mut Self>);

        /// Quick mask (Q): dim outside the selection in the preview.
        #[qinvokable]
        fn sel_toggle_quick_mask(self: Pin<&mut Self>);

        /// 魔术棒：按 (x, y) 处的颜色自动选出一片区域。
        /// `tolerance` 0..=255，`contiguous` = PS 的「连续」，
        /// `sample_all` = 取样全部图层（否则只取当前图层/背景）。
        /// `mode` 同其它选区命令：0 替换 / 1 加选 / 2 减选 / 3 相交。
        #[qinvokable]
        fn sel_magic_wand(
            self: Pin<&mut Self>,
            x: f64,
            y: f64,
            tolerance: i32,
            contiguous: bool,
            sample_all: bool,
            mode: i32,
        );

        /// 记住魔术棒参数并写盘：`tol,contiguous,sample_all`（如 "32,1,0"）。
        #[qinvokable]
        fn save_sel_wand(self: Pin<&mut Self>, spec: &QString);

        /// 记住修复 / 油漆工具与参数（`|` 分隔的 12 段，见 AppConfig）。
        #[qinvokable]
        fn save_paint_spec(self: Pin<&mut Self>, spec: &QString);

        /// Remember the selection tool the user picked ("rect" / "ellipse" /
        /// "lasso" / "polygon") and write it to settings.ini, so the next
        /// launch starts with the same marquee.
        #[qinvokable]
        fn save_sel_tool(self: Pin<&mut Self>, tool: &QString);

        /// 快速选择：把「上一个笔迹点 → (x, y)」这一段当成一粒种子，按颜色
        /// 相近 + 边缘保护向外扩散，再并进本次拖动的累积选区。
        /// 拖动期间**不进历史**（每帧一条会把撤销栈撑爆），松开时由
        /// `quick_select_commit` 补成一条。
        /// `mode`：0 替换 / 1 加选 / 2 减选。
        #[qinvokable]
        fn quick_select_brush(
            self: Pin<&mut Self>,
            x: f64,
            y: f64,
            radius: f64,
            tolerance: i32,
            grow: i32,
            mode: i32,
        );

        /// 松开鼠标：把整段涂抹提交为一次撤销记录。
        #[qinvokable]
        fn quick_select_commit(self: Pin<&mut Self>);

        /// 放弃本次涂抹（Esc），恢复按下前的选区。
        #[qinvokable]
        fn quick_select_cancel(self: Pin<&mut Self>);

        // ---- 修复 / 油漆 / 图层旋转 ------------------------------------

        /// 开始一笔修复。`kind` = "spot"（周边插值）/ "heal"（Alt 取样复制）。
        /// 目标：有选中图层改图层，否则改背景。拖动期间不进历史，松开时
        /// `retouch_end` 补成一条记录。
        #[qinvokable]
        fn retouch_begin(
            self: Pin<&mut Self>,
            kind: &QString,
            x: f64,
            y: f64,
            size: f64,
            hardness: f64,
            spacing: f64,
            strength: f64,
        );

        #[qinvokable]
        fn retouch_move(self: Pin<&mut Self>, x: f64, y: f64);

        #[qinvokable]
        fn retouch_end(self: Pin<&mut Self>);

        #[qinvokable]
        fn retouch_cancel(self: Pin<&mut Self>);

        /// 修复画笔的取样锚点（Alt + 点击，画布坐标）。
        #[qinvokable]
        fn retouch_set_source(self: Pin<&mut Self>, x: f64, y: f64);

        /// 修补工具：第一次拖动圈出区域（画布坐标的折线，空格分隔）。
        #[qinvokable]
        fn patch_set_region(self: Pin<&mut Self>, points: &QString);

        #[qinvokable]
        fn patch_contains(self: Pin<&mut Self>, x: f64, y: f64) -> bool;

        /// 修补工具：在区域里按下，拖到取样位置。`feather` = 扩散（像素）。
        #[qinvokable]
        fn patch_drag_begin(self: Pin<&mut Self>, x: f64, y: f64, feather: i32);

        #[qinvokable]
        fn patch_drag_move(self: Pin<&mut Self>, dx: f64, dy: f64);

        /// 松手：提交改动并返回是否真的改了（QML 据此清掉区域轮廓）。
        #[qinvokable]
        fn patch_drag_end(self: Pin<&mut Self>) -> bool;

        #[qinvokable]
        fn patch_drag_cancel(self: Pin<&mut Self>);

        #[qinvokable]
        fn patch_clear_region(self: Pin<&mut Self>);

        /// 油漆桶：按容差填充。有选区时只填选区内的部分。
        #[qinvokable]
        fn bucket_fill(
            self: Pin<&mut Self>,
            x: f64,
            y: f64,
            tolerance: i32,
            contiguous: bool,
            opacity: f64,
            color: &QString,
        );

        /// 渐变：按下定起点，拖动定终点（每帧预览），松手提交。
        #[qinvokable]
        fn gradient_begin(
            self: Pin<&mut Self>,
            x: f64,
            y: f64,
            radial: bool,
            reverse: bool,
            opacity: f64,
            c0: &QString,
            c1: &QString,
        );

        #[qinvokable]
        fn gradient_move(self: Pin<&mut Self>, x: f64, y: f64);

        #[qinvokable]
        fn gradient_end(self: Pin<&mut Self>);

        #[qinvokable]
        fn gradient_cancel(self: Pin<&mut Self>);

        /// 图层旋转：begin 记下旋转前状态，update 实时预览，finish 提交/放弃。
        /// `keep_session` = true 时沿用上一次旋转留下的原图锚点（手柄连拖不糊）；
        /// 返回会话起始角。
        #[qinvokable]
        fn rotate_layer_begin(self: Pin<&mut Self>, index: i32, keep_session: bool) -> f64;

        #[qinvokable]
        fn rotate_layer_update(self: Pin<&mut Self>, degrees: f64, expand: bool, mods: i32);

        /// `degrees` = 最终增量角（不必先调 update）；`mods` 带 Shift 时
        /// 落定角吸附 15°（P1-A）。
        #[qinvokable]
        fn rotate_layer_finish(
            self: Pin<&mut Self>,
            apply: bool,
            degrees: f64,
            expand: bool,
            mods: i32,
        );

        /// 一次性旋转（角度对话框确认 / 脚本探针）。
        #[qinvokable]
        fn rotate_layer_angle(self: Pin<&mut Self>, index: i32, degrees: f64, expand: bool);

        /// 进行中旋转的状态 "角度,是否扩展"，无手势时为空串。
        #[qinvokable]
        fn rotate_layer_state(self: Pin<&mut Self>) -> QString;

        /// ★P0-2b1：统一变换会话的诊断快照（JSON）；无手势时 active=false。
        #[qinvokable]
        fn transform_state(self: Pin<&mut Self>) -> QString;

        /// 统一取消进行中的变换手势（切工具 / 失焦兜底），返回是否真有会话。
        #[qinvokable]
        fn transform_abort(self: Pin<&mut Self>) -> bool;

        /// Crop to the selection's bounding box, then drop the selection
        /// (the canvas coordinate frame changed).
        #[qinvokable]
        fn crop_to_selection(self: Pin<&mut Self>);

        #[qinvokable]
        fn extract_selection(self: Pin<&mut Self>);

        #[qinvokable]
        fn layer_name(self: Pin<&mut Self>, index: i32) -> QString;

        #[qinvokable]
        fn layer_visible(self: Pin<&mut Self>, index: i32) -> bool;

        #[qinvokable]
        fn select_layer(self: Pin<&mut Self>, index: i32);

        /// 多选：mode 0 = 单选替换，1 = Ctrl 切换，2 = Shift 连选，
        /// 3 = 并进集合并设为主选中层（画布 Ctrl+点，只加不减）。
        #[qinvokable]
        fn select_layer_ex(self: Pin<&mut Self>, index: i32, mode: i32);

        // ---- 图层命令（P1，只做按钮 / 菜单入口，不绑快捷键）----

        #[qinvokable]
        fn new_layer(self: Pin<&mut Self>);

        #[qinvokable]
        fn duplicate_selected_layer(self: Pin<&mut Self>);

        #[qinvokable]
        fn merge_selected_down(self: Pin<&mut Self>);

        /// 合并**选中的多个**图层（≥2 层、背景层不参与）。
        /// ★漏声明这条，面板 ⤓ 与右键菜单就会静默失效（QML 里 TypeError，
        ///   而脚本 op 直接调 Rust，探针照样 PASS）——所以必须有。
        #[qinvokable]
        fn merge_selected_layers(self: Pin<&mut Self>);

        #[qinvokable]
        fn merge_visible_layers(self: Pin<&mut Self>);

        #[qinvokable]
        fn flatten_layers(self: Pin<&mut Self>);

        #[qinvokable]
        fn rasterize_selected_mark(self: Pin<&mut Self>);

        /// edge: left / hcenter / right / top / vcenter / bottom
        #[qinvokable]
        fn align_selected_layers(self: Pin<&mut Self>, edge: &QString);

        /// axis: h / v
        #[qinvokable]
        fn distribute_selected_layers(self: Pin<&mut Self>, axis: &QString);

        #[qinvokable]
        fn set_selected_layers_visible(self: Pin<&mut Self>, visible: bool);

        #[qinvokable]
        fn delete_selected_layers(self: Pin<&mut Self>);

        /// 图层缩略图 URL（面板每行一张；按需生成 + 缓存）。
        #[qinvokable]
        fn layer_thumb_url(self: Pin<&mut Self>, index: i32) -> QString;

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

        // ---- P0-4 / P0-5 开关与数值面板 ----
        #[qinvokable]
        fn update_transform_panel_visible(self: Pin<&mut Self>, value: bool);
        #[qinvokable]
        fn update_show_guides(self: Pin<&mut Self>, value: bool);
        #[qinvokable]
        fn update_snap_enabled(self: Pin<&mut Self>, value: bool);
        /// 数值面板写回：x/y/w/h 是变换后外接框（图像坐标），rotation 单位度。
        /// 一次调用 = 一条 SetLayerTransform 历史。
        #[qinvokable]
        fn transform_set_params(self: Pin<&mut Self>, x: f64, y: f64, w: f64, h: f64,
                                rotation: f64, flip_x: bool, flip_y: bool,
                                lock_ratio: bool, ref_point: i32) -> bool;
        /// QML 手势透传修饰键位掩码与显示比例（Rust 侧统一解释，D17）。
        #[qinvokable]
        fn transform_mods(self: Pin<&mut Self>, mods: i32, disp_scale: f64);

        // ---- P0-2b2 统一变换三件套（旧 *_begin/update/end 保留别名）----
        /// gesture: 0=move 1=scale 2=rotate；px/py 为图像坐标。
        #[qinvokable]
        fn transform_begin(self: Pin<&mut Self>, gesture: i32, index: i32,
                           px: f64, py: f64, mods: i32, disp_scale: f64) -> bool;
        #[qinvokable]
        fn transform_update(self: Pin<&mut Self>, gesture: i32, px: f64, py: f64,
                            mods: i32, disp_scale: f64) -> bool;
        #[qinvokable]
        fn transform_end(self: Pin<&mut Self>, commit: bool, mods: i32) -> bool;
        /// ★P1-A：缩放手势语义求解（锁比例 / 从中心 / 斜切 / 吸附都在
        /// Rust 解出），返回 `{"x","y","w","h","fx","fy","kx","ky","cx","cy"}`。
        #[qinvokable]
        fn transform_solve_scale(self: Pin<&mut Self>, sx: f64, sy: f64, sw: f64, sh: f64,
                                 role: QString, dx: f64, dy: f64, mods: i32,
                                 disp_scale: f64) -> QString;
        /// 八向缩放吸附：返回 `{"dx":..,"dy":..}`，只让 `edges` 指定的边参与。
        #[qinvokable]
        fn transform_snap_rect(self: Pin<&mut Self>, x: f64, y: f64, w: f64, h: f64,
                               edges: i32, mods: i32, disp_scale: f64) -> QString;
        #[qinvokable]
        fn guides_state(self: Pin<&mut Self>) -> QString;

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

        /// Hex color at image coordinates `(x, y)` for the Alt-sampling of the
        /// brush / healing brush. Empty string when no document is open.
        #[qinvokable]
        fn canvas_color_at(self: Pin<&mut Self>, x: i32, y: i32) -> QString;

        // ---- 剪贴板（跨文档） ----

        #[qinvokable]
        fn copy_selection_pixels(self: Pin<&mut Self>);

        #[qinvokable]
        fn cut_selection_pixels(self: Pin<&mut Self>);

        #[qinvokable]
        fn erase_selection_pixels(self: Pin<&mut Self>);

        #[qinvokable]
        fn copy_selected_layer(self: Pin<&mut Self>);

        #[qinvokable]
        fn cut_selected_layer(self: Pin<&mut Self>);

        /// Paste the process-wide layer clipboard as a new layer. Returns
        /// `false` when nothing was pasted (empty clipboard / no document), so
        /// the QML side only enters the "floating placement" state on success.
        #[qinvokable]
        fn paste_clipboard(self: Pin<&mut Self>) -> bool;

        /// Ctrl+拖动选区：把选区里的像素**剪**成一个独立图层（原位置挖空），
        /// 选中新图层并返回 true —— QML 接着进入「拖动这个图层」的手势，
        /// 就是 PS 里按住 Ctrl 出剪刀、拖走选区内容的行为。
        #[qinvokable]
        fn cut_selection_to_layer(self: Pin<&mut Self>) -> bool;

        /// 在资源管理器里定位当前文件（打开所在文件夹并选中它）。
        #[qinvokable]
        fn reveal_in_explorer(self: Pin<&mut Self>);

        /// 画布光标种类：0 = 交回 Qt，1 = 加选(+)，2 = 减选(−)，
        /// 3 = 剪刀，4 = 图层旋转。
        #[qinvokable]
        fn cursor_set_kind(self: Pin<&mut Self>, kind: i32);

        /// 把当前线程的顶层窗口挂上自定义光标（幂等；新窗口出现后再调）。
        #[qinvokable]
        fn cursor_install(self: Pin<&mut Self>);

        /// 再 SetCursor 一次当前种类的光标。Qt 在 cursorShape 变化时会自己
        /// 刷光标，把我们的盖掉；QML 在悬停进入的下一帧调这个钉回去。
        #[qinvokable]
        fn cursor_refresh(self: Pin<&mut Self>);

        /// 修饰键状态位：bit0 Shift / bit1 Ctrl / bit2 Alt（QML 轮询用）。
        #[qinvokable]
        fn modifier_bits(self: Pin<&mut Self>) -> i32;

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

        /// `w` / `h` are always positive; `flip_x` / `flip_y` say the handle
        /// was dragged past the opposite edge, so the bitmap mirrors.
        /// `skew_x` / `skew_y`：Ctrl+拖边的斜切量（P1-A）；非斜切手势传 0。
        /// `center_dx` / `center_dy`：斜切的对侧锚定补偿（W/N 缘）。
        #[qinvokable]
        fn scale_layer_update(
            self: Pin<&mut Self>,
            w: i32,
            h: i32,
            ox: i32,
            oy: i32,
            flip_x: bool,
            flip_y: bool,
            skew_x: f64,
            skew_y: f64,
            center_dx: f64,
            center_dy: f64,
        );

        /// `w` / `h` are always positive; `flip_x` / `flip_y` say the handle
        /// was dragged past the opposite edge, so the bitmap mirrors.
        /// `edges` 是本次被拖动的边位掩码（1 左 / 4 右 / 8 上 / 32 下），
        /// 未列出的边在对侧锚定 —— 旋转过的图层靠它反解 scale（不再烘焙）。
        /// `skew_x` / `skew_y` 非零 = 斜切手势落定（P1-A），走 skew 分支。
        #[qinvokable]
        fn scale_layer_end(
            self: Pin<&mut Self>,
            commit: bool,
            w: i32,
            h: i32,
            ox: i32,
            oy: i32,
            flip_x: bool,
            flip_y: bool,
            edges: i32,
            skew_x: f64,
            skew_y: f64,
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
            sel_border: 0,
            sel_smooth: 0,
            sel_feather: 0,
            sel_tool: "rect".to_string(),
            sel_wand: "32,1,1".to_string(),
            paint_spec: "none|40|70|20|100|2|32|1|100|0|0|100".to_string(),
            transform_panel_visible: false,
            show_guides: true,
            snap_enabled: true,
        }
    }
}

/// 快速选择一次涂抹的临时状态。
///
/// 拖动期间每一帧都会重算选区，但**只有松手时**才写一条历史（见
/// `Document::preview_selection`）。`from` 既是撤销目标，也是加选/减选的
/// 基准；`base` 是那个基准的硬轮廓（`mask()`，不含羽化）。
pub(in crate::bridge) struct QuickDrag {
    /// 按下鼠标前的选区。
    pub from: Option<SelectionState>,
    /// 按下鼠标前选区的硬轮廓（加/减的基准）。
    pub base: Option<SelectionMask>,
    /// 本次涂抹已累积的扩散结果。
    pub accum: Option<SelectionMask>,
    /// 上一个笔迹点：下一段从它连到新位置。
    pub last: (f32, f32),
    /// 0 替换 / 1 加选 / 2 减选。
    pub mode: i32,
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
    /// 多选：选中图层的稳定 id（顺序 = 点击顺序，最后一个是主选中层）。
    /// 下标会随重排 / 删除变化，所以缓存必须用 id。
    selection_ids: RefCell<Vec<u32>>,
    /// 上面那份 id 的 JSON 形式（QML 直接读）。
    layer_selection_ids: QString,
    selected_layer_count: i32,
    layer_epoch: i32,
    /// 缩略图缓存：(会话 id, 图层 id) → (epoch, file:// URL, 临时文件路径)。
    thumb_cache: RefCell<std::collections::HashMap<(u32, u32), (u64, String, PathBuf)>>,
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
    /// Counts installed documents (see [`qobject::Bridge::load_serial`]).
    load_serial: i32,
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
    /// ---- PSD stage-2 (layered editing) ----
    /// All editing tools are disabled while this is true. Always cleared by
    /// `finish_psd_layers` (success / failure / skip / switch / handoff).
    editing_locked: bool,
    /// Keeps the poll timer running after `loading` becomes false.
    psd_layers_pending: bool,
    /// "已加载 k/N 层…" text shown in the layer panel and status bar.
    psd_progress: QString,
    /// Receiver for the stage-2 thread. Dropping it discards a superseded job.
    psd_job: RefCell<Option<mpsc::Receiver<PsdLayerMsg>>>,
    /// One-shot PSD notice (degraded / composite-only / downsampled read-only).
    psd_notice: QString,
    /// In-flight background encode. The worker owns a Document snapshot; the
    /// UI thread only performs the atomic rename and the dirty/source update.
    saving: bool,
    save_job: RefCell<Option<SaveJob>>,
    /// C6: (session id, layer id, doc epoch, already-downscaled 40px bitmap).
    thumbs_pending: bool,
    thumb_epoch: i32,
    thumb_queue: RefCell<std::collections::VecDeque<(u32, u32, u64, RgbaImage)>>,
    /// "拾色器" panel: open flag + the last pick ("" before the first one).
    picker_open: bool,
    picker_color: QString,
    /// Recent picks, newest first, as a JSON array.
    picker_history: QString,
    /// Recent picks in decoded form (the JSON is rebuilt from this).
    picker_recent: RefCell<Vec<PickedColor>>,
    // ---- 选区（PS 式）----
    sel_active: bool,
    /// Bumped by every selection mutation so QML can re-pull the outline.
    sel_version: i32,
    /// Selection bounds in image coords (published for the handle overlay).
    sel_x: i32,
    sel_y: i32,
    sel_w: i32,
    sel_h: i32,
    /// Live 边界 / 平滑 / 羽化 values, mirrored into the right-click menu.
    sel_border: i32,
    sel_smooth: i32,
    sel_feather: i32,
    quick_mask: bool,
    /// Last used selection tool, mirrored for QML (`selTool` seeds from it).
    sel_tool_saved: QString,
    /// 魔术棒参数 `tol,contiguous,sample_all`（settings.ini 里的原样字符串）。
    sel_wand_saved: QString,
    /// 修复 / 油漆工具参数（settings.ini，`|` 分隔）。
    paint_spec_saved: QString,
    /// 快速选择涂抹中的临时状态；`None` = 当前没在涂。
    quick_drag: RefCell<Option<QuickDrag>>,
    /// 边界 / 平滑 / 羽化 that a NEWLY drawn selection starts with. Kept on
    /// the bridge (not on the document) so it survives opening another file
    /// and is written to settings.ini on every change. `Cell` because
    /// `Pin<&mut Self>` cannot assign a plain field (E0594).
    sel_ops_default: std::cell::Cell<SelOps>,
    paste_counter: RefCell<u32>,
    /// Opacity of the selected layer before the current live slider drag, so
    /// the committed history op can record the true pre-drag value.
    opacity_live: RefCell<Option<f32>>,
    /// ★P0-2b1：拖动起点已并入 `Session::transform`（TransformSession::Move），
    /// 不再在 Bridge 上单独留一份容易和手势状态不同步的副本。
    /// The selected mark and its pre-drag geometry; live updates map
    /// geometry without history, end commits one UpdateMark op.
    mark_drag: RefCell<Option<(usize, Mark)>>,
    session: RefCell<Option<Session>>,
    viewport: RefCell<Size>,
    /// Decode result waiting for a known viewport (prewarm can finish before
    /// QML has reported one — see `poll_decode`).
    pending_decode: RefCell<Option<(DecodeMsg, u32)>>,
    /// Screen device-pixel-ratio pushed by QML (1.0 at 100% scaling). The
    /// preview bitmap is rendered at display size × dpr so it stays crisp on
    /// high-DPI displays. `Cell` because `set_dpr` mutates it behind
    /// `Pin<&mut Self>`; the render path reads it.
    dpr: std::cell::Cell<f64>,
    // ---- P0-4 / P0-5 ----
    /// D13：数值面板显示状态（settings.ini 记忆）。
    transform_panel_visible: bool,
    /// D16：提示线绘制开关（不影响求解）。
    show_guides: bool,
    /// D16：吸附求解开关。
    snap_enabled: bool,
    /// 智能参考线：QML 叠加层只读的 JSON（轴线 + 距离标签）。
    guides_json: QString,
    /// 数值面板：选中图层当前外接框参数 JSON。
    transform_values_json: QString,
    gpu_live: bool,
    gpu_static_url: QString,
    gpu_active_url: QString,
    gpu_rect_json: QString,
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
            selection_ids: RefCell::new(Vec::new()),
            layer_selection_ids: QString::from("[]"),
            selected_layer_count: 0,
            layer_epoch: 0,
            thumb_cache: RefCell::new(std::collections::HashMap::new()),
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
            load_serial: 0,
            adjust_open: false,
            adjust_target_name: QString::default(),
            adjust_values: QString::default(),
            adjust_histogram: QString::default(),
            ruler_bottom_up: cfg.ruler_bottom_up,
            loading: false,
            large_downsample: cfg.max_dimension != 0,
            decode_job: RefCell::new(None),
            editing_locked: false,
            psd_layers_pending: false,
            psd_progress: QString::default(),
            psd_job: RefCell::new(None),
            psd_notice: QString::default(),
            saving: false,
            save_job: RefCell::new(None),
            thumbs_pending: false,
            thumb_epoch: 0,
            thumb_queue: RefCell::new(std::collections::VecDeque::new()),
            picker_open: false,
            picker_color: QString::default(),
            picker_history: QString::from("[]"),
            picker_recent: RefCell::new(Vec::new()),
            sel_active: false,
            sel_version: 0,
            sel_x: 0,
            sel_y: 0,
            sel_w: 0,
            sel_h: 0,
            sel_border: 0,
            sel_smooth: 0,
            sel_feather: 0,
            quick_mask: false,
            sel_tool_saved: QString::from(cfg.sel_tool.as_str()),
            sel_wand_saved: QString::from(cfg.sel_wand.as_str()),
            paint_spec_saved: QString::from(cfg.paint_spec.as_str()),
            quick_drag: RefCell::new(None),
            sel_ops_default: std::cell::Cell::new(SelOps {
                border: cfg.sel_border,
                smooth: cfg.sel_smooth,
                feather: cfg.sel_feather,
            }),
            paste_counter: RefCell::new(0),
            opacity_live: RefCell::new(None),
            mark_drag: RefCell::new(None),
            session: RefCell::new(None),
            viewport: RefCell::new(Size::new(0, 0)),
            pending_decode: RefCell::new(None),
            dpr: std::cell::Cell::new(1.0),
            transform_panel_visible: cfg.transform_panel_visible,
            show_guides: cfg.show_guides,
            snap_enabled: cfg.snap_enabled,
            guides_json: QString::from("[]"),
            transform_values_json: QString::from("{}"),
            gpu_live: false,
            gpu_static_url: QString::default(),
            gpu_active_url: QString::default(),
            gpu_rect_json: QString::from("{}"),
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
            sel_border: self.sel_ops_default.get().border,
            sel_smooth: self.sel_ops_default.get().smooth,
            sel_feather: self.sel_ops_default.get().feather,
            sel_tool: self.sel_tool_saved().to_string(),
            sel_wand: self.sel_wand_saved().to_string(),
            paint_spec: self.paint_spec_saved().to_string(),
            transform_panel_visible: *self.transform_panel_visible(),
            show_guides: *self.show_guides(),
            snap_enabled: *self.snap_enabled(),
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

    /// Announce that a new document was installed (open / navigate / the
    /// single-instance hand-off). QML watches the counter to size the window
    /// to the image it just opened, so it must NOT fire for canvas edits
    /// (crop / rotate / resize), which change `image_width` too.
    fn bump_load_serial(mut self: Pin<&mut Self>) {
        let next = self.load_serial().wrapping_add(1).max(1);
        self.as_mut().set_load_serial(next);
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
        // 图层增删 / 重排之后多选集合可能指向已经不存在的 id，顺手校准。
        self.as_mut().sync_layer_selection();
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
                // Structured event for the round-trip / write-back probes:
                // a rejected save must never look like a silent success.
                let safe = err.replace('\\', "/").replace('"', "'");
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"save-error\",\"detail\":\"{}\"}}",
                    safe
                );
            }
        }
    }

    /// Decode options derived from the current preferences.
    fn decode_opts(&self) -> DecodeOptions {
        decode_options(*self.large_downsample())
    }

    /// Interactive load: hand the file to the worker thread and return
    /// immediately, so even a huge image never freezes the UI. The current
    /// document stays visible until the decode lands (or fails).
    fn load_path(mut self: Pin<&mut Self>, path: &Path) {
        // finally-style cleanup: opening another file must never leave the
        // previous stage-2 lock, an in-flight save, or a stale PSD notice.
        self.as_mut().finish_psd_layers("new-load");
        self.as_mut().cancel_save("new-load");
        self.as_mut().cancel_thumbs("new-load");
        self.as_mut().clear_psd_notice();
        println!(
            "{{\"phase\":\"ui\",\"event\":\"load\",\"file\":\"{}\"}}",
            path.to_string_lossy().replace('\\', "/")
        );
        // Prewarmed in `main()` before the window existed: the pixels may
        // already be decoded by now, so just claim the receiver instead of
        // starting a second decode (see the `prewarm` module).
        if let Some(rx) = prewarm::take(path) {
            *self.decode_job.borrow_mut() = Some(rx);
            self.as_mut().set_loading(true);
            self.as_mut()
                .set_status(QString::from("正在打开…"));
            return;
        }
        let (tx, rx) = mpsc::channel::<DecodeMsg>();
        let thread_path = path.to_path_buf();
        let opts = self.decode_opts();
        // Stage 2 parses PSD layers in parallel; stage 1 keeps the existing
        // composite-only path so the first frame is unchanged (C1/C2).
        let layer_job = spawn_psd_layer_job(&thread_path, opts);
        std::thread::spawn(move || {
            let t0 = Instant::now();
            let result = decode_path(&thread_path, opts).map_err(|e| e.to_string());
            let ms = t0.elapsed().as_millis().min(i32::MAX as u128) as i32;
            let _ = tx.send(DecodeMsg {
                path: thread_path,
                result: result.map(|d| (d, ms)),
                layer_job,
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
        // Synchronous automation path is NEVER locked; make sure a leftover
        // interactive lock / save / notice cannot leak into --script probes.
        self.as_mut().finish_psd_layers("sync-load");
        self.as_mut().cancel_save("sync-load");
        self.as_mut().cancel_thumbs("sync-load");
        self.as_mut().clear_psd_notice();
        println!(
            "{{\"phase\":\"ui\",\"event\":\"load\",\"file\":\"{}\"}}",
            path.to_string_lossy().replace('\\', "/")
        );
        let viewport = *self.viewport.borrow();
        let opts = self.decode_opts();
        match Session::open(path, viewport, (*self.save_quality()).clamp(1, 100) as u8, opts, self.dpr.get()) {
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
                // LAST: the QML window-fit handler reads `image_width` /
                // `display_*`, which refresh_preview just published.
                self.as_mut().bump_load_serial();
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
        // PS 语义：选中一个普通图层时，翻转变换只作用于该图层（背景和其它
        // 图层不动）。没有选中图层、或选中的是背景行时，仍然翻整幅。
        // 旋转不在此列：转 90° 会换掉画布坐标系，图层单独转语义完全不同。
        let layer_only = match transform {
            Transform::FlipHorizontal | Transform::FlipVertical => {
                let ix = *self.selected_layer();
                self.selected_guard(ix)
                    .filter(|ix| !self.layer_locked(*ix))
            }
            _ => None,
        };
        let outcome: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    let result = match layer_only {
                        Some(ix) => session.doc.transform_layer(ix, transform),
                        None => session.doc.apply_transform(transform),
                    };
                    match result {
                        Ok(()) => {
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"edit\",\"op\":\"{label}\",\"scope\":\"{}\",\"width\":{},\"height\":{}}}",
                                if layer_only.is_some() { "layer" } else { "document" },
                                session.doc.canvas.width,
                                session.doc.canvas.height
                            );
                            session.refit_if_needed(viewport);
                            session.dirty = true;
                            // 整幅旋转/翻转换掉了画布坐标系：选区随之失效。
                            // 只翻一个图层时坐标没变，选区必须留着（用户很可能
                            // 就是要"选中某图层 → 翻它 → 继续在那个区域操作"）。
                            if layer_only.is_none() {
                                session.clear_selection();
                            }
                            Ok(true)
                        }
                        Err(err) => Err(err.to_string()),
                    }
                }
            }
        };
        match outcome {
            Ok(true) => {
                if layer_only.is_some() {
                    // 选区还在：刷新它的发布态（尺寸/轮廓没变，但预览换了）。
                    self.as_mut().touch_selection();
                    self.as_mut().publish_selection();
                }
                self.as_mut().refresh_preview();
            }
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
                .map(|s| s.doc.mark_count())
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
                live,
            } => {
                // The status bar's left side already shows the file name and
                // size; a second resolution readout here was redundant, so
                // only keep the preview-degraded hint when it matters. The
                // preview is rendered in PHYSICAL pixels (display × dpr), so
                // "degraded" means it is smaller than the physical display
                // size (zoomed past the native resolution or past the memory
                // cap) — not merely different from the logical display size.
                // 拖动中的降质实时帧不算（否则旋转时状态栏一直在闪提示）。
                let dpr = self.dpr.get();
                let phys_w = ((display.width as f64 * dpr).round() as u32).max(1);
                let phys_h = ((display.height as f64 * dpr).round() as u32).max(1);
                let degraded =
                    !live && (preview.width < phys_w || preview.height < phys_h);
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
                // 面板列表签名里混了这个值：像素一改（含撤销）就重建列表，
                // 缩略图随之刷新；拖动帧不改 epoch，所以拖动期间列表不动。
                let epoch = self
                    .session
                    .borrow()
                    .as_ref()
                    .map(|s| s.doc.edit_epoch as i32)
                    .unwrap_or(0);
                self.as_mut().set_layer_epoch(epoch);
                self.as_mut().set_dirty(dirty);
                // Publish JSON properties only when they actually changed:
                // QML re-parses them (and rebuilds the list panels) on every
                // change signal, which churned every live-drag tick.
                if self.layers_json().to_string() != layers_json {
                    self.as_mut()
                        .set_layers_json(QString::from(layers_json.as_str()));
                }
                // P0-4 / P0-5：随帧下发参考线与数值面板参数；内容不变时不发信号，
                // 避免拖动帧触发 QML 重新解析 / 重绘。
                let guides = if *self.show_guides() {
                    self.session
                        .borrow()
                        .as_ref()
                        .map(|s| editcore::snap::guides_json(&s.snap, s.disp_scale))
                        .unwrap_or_else(|| "[]".to_string())
                } else {
                    "[]".to_string()
                };
                if self.guides_json().to_string() != guides {
                    self.as_mut().set_guides_json(QString::from(guides.as_str()));
                }
                let values = self
                    .session
                    .borrow()
                    .as_ref()
                    .map(|s| s.transform_panel_json(*self.selected_layer()))
                    .unwrap_or_else(|| "{\"has\":false}".to_string());
                if self.transform_values_json().to_string() != values {
                    self.as_mut()
                        .set_transform_values_json(QString::from(values.as_str()));
                }
                // Tier 1：随帧下发活动层矩形；URL 只在起手时换一次。
                let (gpu_on, gpu_rect) = self
                    .session
                    .borrow()
                    .as_ref()
                    .map(|s| (s.gpu_live, s.gpu_rect_json()))
                    .unwrap_or((false, "{}".to_string()));
                if *self.gpu_live() != gpu_on {
                    self.as_mut().set_gpu_live(gpu_on);
                }
                if self.gpu_rect_json().to_string() != gpu_rect {
                    self.as_mut()
                        .set_gpu_rect_json(QString::from(gpu_rect.as_str()));
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
                            .map(|s| s.doc.mark_count())
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
                // Selection state (bounds / ops / active) is republished on
                // every refresh: canvas-changing ops clear it, and the QML
                // overlay must learn about that without a second round trip.
                self.as_mut().publish_selection();
                // Last: updating preview_url triggers Image loading, which may
                // complete synchronously and re-enter `image_shown`.
                self.as_mut().set_preview_url(QString::from(url.as_str()));
                // 告诉 QML「有新帧了」，由它派给后台那张 Image 去解码。
                self.as_mut().preview_advanced();
            }
            Refresh::Err(err) => {
                let status = format!("render-error: {err}");
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"render-error\",\"detail\":\"{}\"}}",
                    err.replace('"', "'")
                );
                self.as_mut().set_status(QString::from(status.as_str()));
            }
        }
    }
}
