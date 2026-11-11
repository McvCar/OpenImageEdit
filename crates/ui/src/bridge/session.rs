// Session 渲染管线核心。所有项标 pub(in crate::bridge)，mod.rs 再导出。
use super::*;

pub(in crate::bridge) struct Session {
    pub(in crate::bridge) id: u32,
    pub(in crate::bridge) doc: Document,
    pub(in crate::bridge) source: Option<PathBuf>,
    pub(in crate::bridge) zoom: f64,
    pub(in crate::bridge) fit: bool,
    pub(in crate::bridge) render_count: u32,
    pub(in crate::bridge) dirty: bool,
    /// Save quality snapshot from AppConfig at session creation (updated
    /// live by `update_save_quality`).
    pub(in crate::bridge) save_quality: u8,
    /// Mark index hidden from the PREVIEW only while its text is being
    /// edited inline in QML (the TextInput overlays the real mark). -1 = none.
    pub(in crate::bridge) preview_hidden_mark: i32,
    /// ★ P0-2b1：移动 / 八向缩放 / 旋转三种手势的**唯一**进行中会话。
    ///
    /// 旧实现把 live 预览态拆成 4 个独立 Option（外加 BridgeRust 里的
    /// `drag_start_offset`），每个 begin/update/end 各清各的；只要有一处
    /// 漏了收尾，合成器就一直画「旧快照 + 旧 offset」，而 `layers_json`
    /// 已经报出新框 —— 用户看到的「框被拖走、内容钉在原地」「放手角度
    /// 复位」都出在这里。改成枚举变体后三种手势在类型层面互斥，会话
    /// 结束就是整体消失，清理点只剩一个。
    pub(in crate::bridge) transform: Option<TransformSession>,
    /// 选区缩放手柄的「当前是否已越过对边」。负宽高拖拽一帧一帧地进来，
    /// 只在符号翻转的那一帧真正镜像 mask（每帧都翻会让拖动中的内容来回抖）。
    /// `sel_resize_begin` 在按下手柄时清零。
    pub(in crate::bridge) sel_flip_sign: (bool, bool),
    /// Cached preview-scaled base/layers; invalidated by doc.edit_epoch so
    /// live drag ticks skip the per-layer resample.
    pub(in crate::bridge) composer: render::PreviewComposer,
    /// Display size of the last rendered preview. A window resize almost
    /// always shifts the fit zoom by a fraction of a pixel, which would
    /// otherwise re-render (and re-encode the BMP) for a preview identical to
    /// the one already on screen.
    pub(in crate::bridge) last_display: Size,
    /// Result of the last non-special render (see [`CachedRender`]). Pure
    /// zoom ticks above 100% clamp to the same preview target, so the whole
    /// composite + BMP encode + disk write collapses into a URL reuse.
    pub(in crate::bridge) last_render: Option<CachedRender>,
    /// Open "调整颜色" panel: the scope captured when it opened plus the live
    /// parameters. Document pixels are NOT touched until `adjust_commit`.
    pub(in crate::bridge) adjust: Option<AdjustLive>,
    /// Original (width, height) when downsampling-first open shrank this
    /// document. The status bar keeps a persistent hint (refreshes rebuild
    /// the status text, so a one-shot message would be wiped instantly).
    pub(in crate::bridge) downsampled_from: Option<(u32, u32)>,
    /// True only when the document was assembled from real PSD layer seeds.
    /// A composite-only PSD (stage 2 failed / skipped / flat / downsampled)
    /// must NOT be written back as PSD — that would destroy the original
    /// layer stack (§8). Plain image documents leave this false but can still
    /// be "Saved As" a new PSD (a fresh layered file from their flat pixels).
    pub(in crate::bridge) psd_layered: bool,
    /// Decoder hint shown in the status bar (e.g. PSD flattened preview).
    pub(in crate::bridge) note: Option<String>,
    /// Screen device-pixel-ratio (OS scaling, e.g. 1.25 at 125%). Kept on the
    /// session so `refit`/`from_decoded` can compute a correct fit zoom even
    /// before the first `render`. Mirrors the bridge's `dpr` (synced by
    /// `set_dpr`); used by `fit_zoom`/`display_size` to make 100% mean
    /// 1 image px = 1 *physical* px (Photos/PS convention).
    pub(in crate::bridge) dpr: f64,
    /// Quick mask (PS's Q): dim everything OUTSIDE the selection in the
    /// PREVIEW only, so the feathered soft edge becomes visible.
    pub(in crate::bridge) quick_mask: bool,
    /// Bumped by every selection mutation; QML re-pulls the outline when the
    /// published copy of it changes. Kept on the session so a plain refresh
    /// never looks like a selection change.
    pub(in crate::bridge) sel_rev: u32,
    /// 修复 / 油漆类工具的进行中手势。拖动期间像素直接写进文档（不记
    /// 历史），松手时补成一条撤销记录 —— 与选区的手势模型一致。
    pub(in crate::bridge) brush_live: Option<BrushLive>,
    pub(in crate::bridge) gradient_live: Option<GradientLive>,
    pub(in crate::bridge) patch_live: Option<PatchLive>,
    /// 修补工具圈出来的区域（画布坐标）。第二次拖动把这块区域拖到取样点。
    pub(in crate::bridge) patch_region: Option<SelectionMask>,
    /// 修复画笔的取样锚点（画布坐标），Alt + 点击设置。
    pub(in crate::bridge) heal_source: Option<(f32, f32)>,
    /// 连续旋转会话的原图锚点（见 `RotateOrigin`）。**P0-2b2 起已退场**：
    /// 累计角度和未插值原图改由 `Layer::transform` 参数层持有，本字段只是
    /// 旧合成缓存的兼容位，落定后恒为 None，不再参与任何读取。
    pub(in crate::bridge) rotate_origin: Option<RotateOrigin>,
    /// 旋转会话自增号（见 `LiveRotate::token`）。同样是跨会话的缓存版本号。
    pub(in crate::bridge) rotate_token: u64,
    // ---- P0-4 智能参考线 / P0-2b2 mods 透传 ----
    /// 起手时预生成并排序的静态候选（画布 / 其它图层 / 选区）。
    pub(in crate::bridge) snap_statics: Vec<editcore::snap::SnapCandidate>,
    /// 最近一次求解结果；随每帧 refresh 下发成 `guides_json`。
    pub(in crate::bridge) snap: editcore::snap::SnapResult,
    /// QML 透传的 `Qt::KeyboardModifiers` 位掩码，修饰键语义一律在 Rust 解释。
    pub(in crate::bridge) mods: i32,
    /// 显示像素 / 图像像素；`8 / disp_scale` 就是 8 显示像素阈值。
    pub(in crate::bridge) disp_scale: f64,
    /// 统一三件套记录的起点（图像坐标）、起手外接框与手势类型。
    pub(in crate::bridge) gesture: i32,
    pub(in crate::bridge) gesture_start: (f64, f64),
    pub(in crate::bridge) gesture_rect: (f64, f64, f64, f64),
    // ---- 拼图咬合（内容占位吸附） ----
    /// 移动图层的**内容框**（alpha 包围盒，画布坐标）。`None` = 内容全透明
    /// 或图层已旋转（投影失真，退回纯外框吸附）。
    pub(in crate::bridge) fit_content: Option<editcore::layer_transform::RectF>,
    /// 起手时记录的手势目标图层，供每帧重建占位栅格时排除自己。
    pub(in crate::bridge) gesture_index: i32,
    /// 复用的占位栅格（每帧按当前内容框重建，窗口很小）。
    pub(in crate::bridge) fit_occ: editcore::alpha_fit::Occupancy,
    /// Tier 1 拖动帧**跳过**的 CPU 全幅合成次数（计数器断言：启用快路径后
    /// 每次拖动都应递增，而实际合成次数为 0）。
    pub(in crate::bridge) full_composites: u32,
    // ---- P0-2b2 Tier 1 GPU 快路径 ----
    /// true 时 render() 直接复用上一帧 URL，QML 改用「静态层 + 活动层」两纹理。
    pub(in crate::bridge) gpu_live: bool,
    pub(in crate::bridge) gpu_static_url: String,
    pub(in crate::bridge) gpu_active_url: String,
    pub(in crate::bridge) gpu_index: usize,
    /// 活动层显示矩形 + 不透明度（QML 逻辑像素）：x, y, w, h, opacity。
    pub(in crate::bridge) gpu_rect: (f64, f64, f64, f64, f64),
}

impl Session {
    /// 丢弃所有「手势进行中」的临时预览态。
    ///
    /// 旋转 / 缩放拖动期间合成器走 special 分支：用拖动起手的快照 + 固定
    /// 的 offset / 角度临时画一帧，文档像素不动。正常由 `*_finish` 收尾清掉；
    /// 一旦 QML 丢了 release / canceled（浮层抢焦点、切窗口、手柄在拖动中
    /// 被隐藏），这些状态会**残留**。之后拖图层时 `layers_json` 照常报新
    /// offset（框跟着鼠标走），预览却还在用旧快照和旧 offset（内容钉在原
    /// 地）—— 就是「旋转 45° 后拖选框，框被拖走、图层不动」。旋转后特别
    /// 容易撞上，因为只有旋转会设置 `live_rotate`。
    ///
    /// 不动 `rotate_origin`：它只决定下一次旋转从哪张原图起手，不参与合成。
    /// 返回清理前的状态给调用方打诊断日志。
    pub(in crate::bridge) fn clear_live_gestures(&mut self) -> (bool, bool, bool, bool) {
        // ★统一会话后只剩一个 take：不可能再出现「清了缩放却漏了旋转帧」
        // 这种半截状态。返回值仍按旧四元组报，live-stale 日志不变。
        match self.transform.take() {
            Some(ts) => ts.stale_flags(),
            None => (false, false, false, false),
        }
    }

    /// 取走移动会话的起点并结束会话。不是移动会话就原样放回，避免误删。
    pub(in crate::bridge) fn take_move_start(
        &mut self,
    ) -> Option<(usize, (i32, i32), Option<(f64, f64)>)> {
        match self.transform.take() {
            Some(TransformSession::Move { index, start, start_center, .. }) => {
                Some((index, start, start_center))
            }
            other => {
                self.transform = other;
                None
            }
        }
    }

    /// 取走缩放会话（同时结束它），返回起手快照供落定使用。
    pub(in crate::bridge) fn take_scale_state(&mut self) -> Option<LiveScaleState> {
        match self.transform.take() {
            Some(TransformSession::Scale(ls)) => Some(ls),
            other => {
                self.transform = other;
                None
            }
        }
    }

    /// 取走旋转会话（连同拖动帧与预览上限一起结束）。
    pub(in crate::bridge) fn take_rotate_live(&mut self) -> Option<RotateLive> {
        match self.transform.take() {
            Some(TransformSession::Rotate { live, .. }) => Some(live),
            other => {
                self.transform = other;
                None
            }
        }
    }

    /// 旋转对话框用的「当前累计角度 + 是否扩张」。
    pub(in crate::bridge) fn rotate_state(&self) -> Option<(f32, bool)> {
        match self.transform.as_ref() {
            Some(TransformSession::Rotate { live, .. }) => {
                Some((live.base_degrees + live.last_delta, live.expand))
            }
            _ => None,
        }
    }

    /// 统一会话的诊断快照（给 `transform_state` 探针 / `--probe-session`）。
    pub(in crate::bridge) fn transform_state_json(&self) -> String {
        let diag = format!(
            r#""gpu":{},"skipped":{},"guides":{},"gesture":{},"mods":{}"#,
            self.gpu_live,
            self.full_composites,
            self.snap.guides.len(),
            self.gesture,
            self.mods
        );
        match self.transform.as_ref() {
            None => format!(r#"{{"active":false,{diag}}}"#),
            Some(ts) => {
                let extra = match ts {
                    TransformSession::Move { start, last, .. } => format!(
                        r#""start":[{},{}],"last":[{},{}]"#,
                        start.0, start.1, last.0, last.1
                    ),
                    TransformSession::Scale(ls) => format!(
                        r#""to":[{},{}],"offset":[{},{}],"flip":[{},{}]"#,
                        ls.to.width, ls.to.height, ls.offset.0, ls.offset.1,
                        ls.flip.0, ls.flip.1
                    ),
                    TransformSession::Rotate { live, frame, .. } => format!(
                        r#""base":{:.2},"delta":{:.2},"frame":{},"token":{}"#,
                        live.base_degrees, live.last_delta,
                        frame.is_some(), live.token
                    ),
                };
                format!(
                    "{{\"active\":true,\"kind\":\"{}\",\"index\":{},\"special\":{},\"cap\":{},{},{diag}}}",
                    ts.kind(), ts.index(), ts.special(), ts.cap().unwrap_or(0), extra
                )
            }
        }
    }

    /// 拉伸图层之后，把「连续旋转」的原图锚点同步到新尺寸。
    ///
    /// 旋转提交会把位图烘焙成**最小外接矩形**（四角是透明像素），锚点里
    /// 留着未插值的原图 + 累计角度，这样接着转不会一遍遍重采样。但 `rotate_
    /// origin` 的复用判据是「当前图层 image == 上次提交那一版」：用户一拖
    /// 八向手柄，位图变了，锚点立刻作废。下一次旋转只能拿这张**已经烘焙过
    /// 一次的外接矩形**当原图再转 —— 100 → 142(45°) → 拉伸 180 → 再转
    /// 45° 变成 255，框越转越大（用户反馈的「描边变得很大」）。
    ///
    /// 这里按本次缩放的同一比例把锚点里的原图也缩放一份，并把 `committed`
    /// 更新成缩放后的图层，让判据继续命中。外接矩形尺寸对原图尺寸是一次
    /// 齐次的（`bbox_w = w|cos| + h|sin|`），所以同比例缩放原图后按同角度
    /// 转回来，外接矩形仍是用户拉出来的那个尺寸；下一次旋转则从这个原图
    /// 按**累计角度**重算，不再叠加一层透明角。
    ///
    /// 不重新烘焙当前位图：拉伸的所见即所得，也不会多一条历史 / 一次插值。
    /// 镜像翻转与旋转的顺序不可交换，遇到翻转就放弃锚点（宁可退化一次）。
    pub(in crate::bridge) fn sync_rotate_origin_after_scale(
        &mut self,
        ix: usize,
        before_image: &RgbaImage,
        to: Size,
        flip: (bool, bool),
    ) {
        let (degrees, mut origin_layer) = {
            let Some(o) = self.rotate_origin.as_ref() else {
                return;
            };
            if o.index != ix
                || o.degrees.abs() < 0.001
                || o.committed.img() != before_image
            {
                return;
            }
            (o.degrees, (*o.layer).clone())
        };
        if flip.0 || flip.1 {
            self.rotate_origin = None;
            return;
        }
        let sx = to.width as f64 / before_image.width().max(1) as f64;
        let sy = to.height as f64 / before_image.height().max(1) as f64;
        let ow = (origin_layer.img().width() as f64 * sx).round().max(1.0) as u32;
        let oh = (origin_layer.img().height() as f64 * sy).round().max(1.0) as u32;
        let old = (origin_layer.img().width(), origin_layer.img().height());
        // 锚点是「已缩放的原图」克隆，不再是任何 transform 的有效缓存。
        origin_layer.ensure_baked();
        *origin_layer.image_mut().unwrap() = render::resize_to(origin_layer.img(), Size::new(ow, oh));
        let committed = match self.doc.layers.get(ix) {
            Some(layer) => std::rc::Rc::new(layer.clone()),
            None => return,
        };
        println!(
            "{{\"phase\":\"ui\",\"event\":\"rotate-origin-scaled\",\"origin\":[{}x{}],\
             \"scaled\":[{}x{}],\"degrees\":{:.2}}}",
            old.0, old.1, ow, oh, degrees
        );
        self.rotate_origin = Some(RotateOrigin {
            index: ix,
            layer: std::rc::Rc::new(origin_layer),
            degrees,
            committed,
        });
    }

    pub(in crate::bridge) fn clear_selection(&mut self) {
        // 走文档的历史（选区是可撤销的文档状态）：Ctrl+Z 能把选区拿回来。
        let _ = self.doc.set_selection(None);
        // 快速蒙版是"看选区软边"的视图态：选区没了就自动关，否则下次
        // 新建选区时会莫名其妙处于蒙版态。
        self.quick_mask = false;
    }

    /// The mask every pixel-consuming command should use (feathered).
    pub(in crate::bridge) fn effective_selection(&self) -> Option<SelectionMask> {
        self.selection().map(|s| s.effective())
    }

    pub(in crate::bridge) fn selection(&self) -> Option<&SelectionState> {
        self.doc.selection.as_ref()
    }
}

/// Live "调整颜色" state (history-free until the panel commits).
#[derive(Debug, Clone, Copy)]
pub(in crate::bridge) struct AdjustLive {
    pub(in crate::bridge) target: AdjustTarget,
    pub(in crate::bridge) params: Adjust,
}

/// Stage-2 (PSD layer parsing) progress / result messages. The worker sends
/// one `Progress` per layer so the UI can show "已加载 k/N 层" while the
/// editing lock is held (C2-a).
pub(in crate::bridge) enum PsdLayerMsg {
    Progress { done: usize, total: usize },
    Done {
        result: Result<decode::PsdLayerSet, String>,
        elapsed_ms: u64,
        /// 40px thumbnails already rendered by the worker, in the same
        /// bottom-to-top order as `PsdLayerSet::layers` (C6). Each is only
        /// 6.4 KB, and dropping a discarded job leaves no temp files behind.
        thumbs: Vec<RgbaImage>,
    },
}

/// C6: the stage-2 worker owns every full-resolution layer bitmap, so it
/// renders the panel thumbnails there. The UI thread never calls
/// `imageops::thumbnail` on a 12MP layer while the window is being laid out.
fn psd_thumb_images(set: &decode::PsdLayerSet) -> Vec<RgbaImage> {
    set.layers
        .iter()
        .map(|layer| editcore::thumbnail_image(&layer.image, 40))
        .collect()
}

/// Message from the background decode worker (see `BridgeRust::decode_job`).
/// `layer_job` is `Some` only for interactive PSD opens; the stage-2 thread is
/// already running when this message is sent, so the first frame never waits
/// for it (C2).
pub(in crate::bridge) struct DecodeMsg {
    pub(in crate::bridge) path: PathBuf,
    pub(in crate::bridge) result: Result<(Decoded, i32), String>,
    pub(in crate::bridge) layer_job: Option<mpsc::Receiver<PsdLayerMsg>>,
}

/// Start the stage-2 PSD layer parse on its own thread. Returns `None` for
/// non-PSD files (or any file that should stay composite-only). Dropping the
/// receiver discards a superseded job's result without touching Qt state.
pub(in crate::bridge) fn spawn_psd_layer_job(
    path: &Path,
    opts: decode::DecodeOptions,
) -> Option<mpsc::Receiver<PsdLayerMsg>> {
    if !is_psd_path(path) {
        return None;
    }
    let (tx, rx) = mpsc::channel::<PsdLayerMsg>();
    let owned = path.to_path_buf();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        // Automation-only hooks. They are inert unless the matching env vars
        // are set, and let probes exercise slow / failed stage-2 paths
        // without shipping a multi-hundred-MB PSD.
        //
        // The delay is injected PER LAYER, i.e. while the UI is already
        // polling, not at thread start: stage-2 is launched by prewarm before
        // the first frame lands, so a start-of-thread sleep would be spent
        // before the editing lock is even raised and would not exercise the
        // lock at all.
        let per_layer_ms = std::env::var("IMAGEEDIT_PSD_STAGE2_DELAY_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let nap = std::time::Duration::from_millis(per_layer_ms);
        let force_fail = std::env::var("IMAGEEDIT_PSD_STAGE2_FAIL").is_ok();
        let result = if force_fail {
            Err("模拟分层解析失败（自动化测试钩子）".into())
        } else {
            (|| -> Result<decode::PsdLayerSet, String> {
                let bytes = std::fs::read(&owned).map_err(|e| e.to_string())?;
                decode::decode_psd_layers_with_progress(&bytes, opts, |done, total| {
                    let _ = tx.send(PsdLayerMsg::Progress { done, total });
                    if !nap.is_zero() {
                        std::thread::sleep(nap);
                    }
                })
                .map_err(|e| e.to_string())
            })()
        };
        let thumbs = match &result {
            Ok(set) => psd_thumb_images(set),
            Err(_) => Vec::new(),
        };
        let _ = tx.send(PsdLayerMsg::Done {
            result,
            elapsed_ms: t0.elapsed().as_millis().min(u64::MAX as u128) as u64,
            thumbs,
        });
    });
    Some(rx)
}

/// Result of one background encode job. Encoding (which can take 1~2 s for a
/// large layered PSD) never runs on the UI thread (C5); only the atomic
/// rename and the dirty/source update happen there.
pub(in crate::bridge) struct SaveMsg {
    pub(in crate::bridge) bytes: Result<Vec<u8>, String>,
    pub(in crate::bridge) encode_ms: u64,
    pub(in crate::bridge) is_psd: bool,
}

/// Everything the UI thread needs to finish a save once the worker is done.
/// `epoch` is the document's `edit_epoch` at snapshot time: if the user kept
/// editing while the encode ran, the completion must NOT clear `dirty`.
pub(in crate::bridge) struct SaveJob {
    pub(in crate::bridge) path: PathBuf,
    pub(in crate::bridge) format: SaveFormat,
    pub(in crate::bridge) epoch: u64,
    pub(in crate::bridge) session_id: u32,
    pub(in crate::bridge) update_source: bool,
    pub(in crate::bridge) rx: mpsc::Receiver<SaveMsg>,
}

/// Encode a cloned Document snapshot on a worker thread. `catch_unwind`
/// keeps a malformed layer from taking down the app (panic=unwind is a
/// release hard constraint). Dropping the receiver cancels the job.
pub(in crate::bridge) fn spawn_save_job(
    doc: &Document,
    format: SaveFormat,
    quality: u8,
) -> mpsc::Receiver<SaveMsg> {
    let snapshot = doc.clone();
    let (tx, rx) = mpsc::channel::<SaveMsg>();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let bytes: Result<Vec<u8>, String> =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                encode_document(&snapshot, format, quality)
            })) {
                Ok(Ok(v)) => Ok(v),
                Ok(Err(e)) => Err(e.to_string()),
                Err(_) => Err("编码时发生内部错误，已取消保存（原文件未被修改）".into()),
            };
        let _ = tx.send(SaveMsg {
            bytes,
            encode_ms: t0.elapsed().as_millis().min(u64::MAX as u128) as u64,
            is_psd: matches!(format, SaveFormat::Psd),
        });
    });
    rx
}

/// Longest-edge cap applied when the user enables downsampling-first open.
pub(in crate::bridge) const DOWNSAMPLE_EDGE: u32 = 8192;

/// P0-2b1 统一变换会话：三种手势互斥，一个字段管住全部 live 预览态。
///
/// 本阶段是**行为等价的结构收口**：变体里仍携带旧预览链路需要的数据，
/// `composite_for_preview` 仍复用三个 `composite_live_*`。P0-2b2 接上
/// `LayerTransform` params 与 GPU 快路径后，这些临时数据会整体删除，
/// 落定也统一成一条 `SetLayerTransform`。
pub(in crate::bridge) enum TransformSession {
    /// 拖动图层内容：文档 offset 实时改（不记历史），松手补一条历史。
    ///
    /// ★`start_center` 是起手时 `Layer::transform.center`（没有参数层时为
    /// `None`）。拖动期间 offset 每帧都变，参数层如果不同步，渲染仍然按
    /// 旧的 center 取样 —— 框（来自 offset）跟着鼠标走、内容钉在原地。
    /// 用户报的「第二次拖图层框和内容分家、调一下大小又好了」就是这个：
    /// 第一次拖动把图层变成参数层（center 已提交），第二次只改 offset。
    Move {
        index: usize,
        start: (i32, i32),
        start_center: Option<(f64, f64)>,
        last: (i32, i32),
    },
    /// 八向手柄缩放：文档像素不动，合成器拉伸 snapshot。
    Scale(LiveScaleState),
    /// 旋柄 / 角度对话框：文档像素不动，合成器按预览尺寸临时转。
    Rotate {
        live: RotateLive,
        frame: Option<LiveRotate>,
        cap: Option<u32>,
    },
}

impl TransformSession {
    pub(in crate::bridge) fn kind(&self) -> &'static str {
        match self {
            Self::Move { .. } => "move",
            Self::Scale(_) => "scale",
            Self::Rotate { .. } => "rotate",
        }
    }

    pub(in crate::bridge) fn index(&self) -> usize {
        match self {
            Self::Move { index, .. } => *index,
            Self::Scale(ls) => ls.index,
            Self::Rotate { live, .. } => live.index,
        }
    }

    pub(in crate::bridge) fn cap(&self) -> Option<u32> {
        match self {
            Self::Rotate { cap, .. } => *cap,
            _ => None,
        }
    }

    /// 该会话是否需要走「special 合成分支」（不能用上一帧的 BMP 复用）。
    pub(in crate::bridge) fn special(&self) -> bool {
        match self {
            Self::Scale(_) => true,
            Self::Rotate { frame, .. } => frame.is_some(),
            Self::Move { .. } => false,
        }
    }

    /// 兼容旧的 `live-stale` 四元组日志：(rotate_live, live_rotate, live_scale, cap)。
    pub(in crate::bridge) fn stale_flags(&self) -> (bool, bool, bool, bool) {
        match self {
            Self::Move { .. } => (false, false, false, false),
            Self::Scale(_) => (false, false, true, false),
            Self::Rotate { frame, cap, .. } => (true, frame.is_some(), false, cap.is_some()),
        }
    }
}

/// Session-side state for a live layer resize gesture.
pub(in crate::bridge) struct LiveScaleState {
    pub(in crate::bridge) index: usize,
    pub(in crate::bridge) snapshot: RgbaImage,
    pub(in crate::bridge) to: Size,
    pub(in crate::bridge) offset: (i32, i32),
    /// 手柄被拖过对边：预览也要镜像（负宽高的可视化）。
    pub(in crate::bridge) flip: (bool, bool),
    /// P1-A：Ctrl+拖边 = 斜切（source 空间剪切量），非斜切手势恒为 0。
    pub(in crate::bridge) skew: (f64, f64),
    /// 斜切的对侧锚定补偿（W/N 缘剪切绕中心会把对侧带走，平移抵消）。
    pub(in crate::bridge) center_delta: (f64, f64),
}

/// 修复画笔 / 污点修复的进行中手势。
pub(in crate::bridge) struct BrushLive {
    /// true = 从 Alt 取样点复制（修复画笔），false = 周边插值（污点修复）。
    pub(in crate::bridge) clone_from_source: bool,
    pub(in crate::bridge) target: AdjustTarget,
    /// 目标位图左上角的画布坐标（位图 = 画布 - origin）。
    pub(in crate::bridge) origin: (i32, i32),
    /// 手势开始时的整张目标位图：每次预览都从它重算，避免反复涂抹叠加。
    pub(in crate::bridge) from: RgbaImage,
    pub(in crate::bridge) points: Vec<(f32, f32)>,
    pub(in crate::bridge) source: Option<(f32, f32)>,
    pub(in crate::bridge) brush: editcore::BrushSpec,
    pub(in crate::bridge) strength: f32,
    /// 已经写进文档的区域（下一次更新前先按 `from` 还原）。
    pub(in crate::bridge) dirty: Option<Rect>,
}

/// 渐变的进行中手势。
pub(in crate::bridge) struct GradientLive {
    pub(in crate::bridge) target: AdjustTarget,
    pub(in crate::bridge) origin: (i32, i32),
    pub(in crate::bridge) from: RgbaImage,
    pub(in crate::bridge) start: (f32, f32),
    pub(in crate::bridge) end: (f32, f32),
    pub(in crate::bridge) radial: bool,
    pub(in crate::bridge) reverse: bool,
    pub(in crate::bridge) opacity: f32,
    pub(in crate::bridge) c0: [u8; 4],
    pub(in crate::bridge) c1: [u8; 4],
    pub(in crate::bridge) dirty: Option<Rect>,
}

/// 修补工具「拖到取样点」的进行中手势。
pub(in crate::bridge) struct PatchLive {
    pub(in crate::bridge) target: AdjustTarget,
    pub(in crate::bridge) origin: (i32, i32),
    pub(in crate::bridge) from: RgbaImage,
    pub(in crate::bridge) offset: (i32, i32),
    pub(in crate::bridge) feather: u32,
    pub(in crate::bridge) dirty: Option<Rect>,
}

/// 图层旋转的进行中手势。每次预览都从 `before` 重算，松手才写历史。
pub(in crate::bridge) struct RotateLive {
    pub(in crate::bridge) index: usize,
    /// 本次会话的起点位图。跨多次拖拽时会话会复用「未被重采样过的原图」，
    /// 所以这里不一定是图层当前的样子。
    pub(in crate::bridge) before: std::rc::Rc<Layer>,
    /// 会话编号（见 `LiveRotate::token`）。
    pub(in crate::bridge) token: u64,
    /// 本次**拖拽**开始时图层的样子（取消时还原到它，而不是还原到 `before`）。
    pub(in crate::bridge) start: std::rc::Rc<Layer>,
    /// ★本次拖拽开始时图层的**精确几何中心**（连续坐标，可以为小数）。
    ///
    /// 不能再用 `offset + image.size/2` 现场推：位图摆放是
    /// `round(center - w/2)`，反推回来已经被量化到 0.5px 栅格；而旋转会改
    /// 变外接框的宽高（奇/偶交替），量化误差的方向跟着翻转，于是每转一轮
    /// 中心就漂 0.5px，转两三次就肉眼可见地偏移 1px（用户 11-08 报的 bug）。
    /// `LayerTransform::center` 才是真值，这里在起手时抓一次，拖动帧与落定
    /// 都用它，反复旋转多少次都不会漂。
    pub(in crate::bridge) start_center: (f64, f64),
    /// 会话开始前已经转过的角度（`before` 就是那个状态）。
    pub(in crate::bridge) base_degrees: f32,
    /// 最后一次 update 传进来的增量角（`rotate_layer_state` 报告用）。
    pub(in crate::bridge) last_delta: f32,
    pub(in crate::bridge) expand: bool,
    /// 是否允许把原图留给下一次拖拽复用（画布手柄 = true，角度对话框 =
    /// false：对话框每次都是一个独立的一次性操作）。
    pub(in crate::bridge) keep: bool,
}

/// 旋转拖动期间喂给合成器的临时状态（文档像素不动，松手才落地）。
pub(in crate::bridge) struct LiveRotate {
    pub(in crate::bridge) index: usize,
    pub(in crate::bridge) snapshot: std::rc::Rc<Layer>,
    /// ★落定后图层内容的**中心**（画布坐标，允许 .5）。
    ///
    /// 拖动帧必须摆在这个点上，否则松手会跳：
    /// * 落定的摆放是 `round(center - AABB/2)`，内容中心 = `摆放 + AABB/2`；
    ///   外接框宽高为**奇数**时它就落在 `.5` 上（偶数时正好是 center）。
    /// * 预览若死守「源图中心」（`round(center - 源图宽/2) + 源图宽/2`），两次
    ///   旋转之后源图宽与 AABB 的奇偶不同，两边就差 0.5~1px —— 用户报的
    ///   「旋转中的画面和放手的画面不一致」正是这个。
    ///
    /// 存中心而不是 offset：offset 是整数，表达不了这半个像素。
    pub(in crate::bridge) center: (f64, f64),
    pub(in crate::bridge) degrees: f32,
    pub(in crate::bridge) expand: bool,
    /// 会话编号：合成器用它判定「缩到预览尺寸的那张快照」还能不能复用。
    /// 必须每次 begin 换一个新值，否则下一次拖拽会拿上一次的旧快照出图。
    pub(in crate::bridge) token: u64,
}

/// 连续旋转会话的「锚」：原图 + 已累计角度 + 提交当时那一版的图层内容。
///
/// 手柄拖完一次就会提交一条 `RotateLayer`，图层里的像素已经被重采样过一
/// 遍；如果下一次拖拽还从这份像素出发，来回拖几下画面就糊了。所以提交时
/// 顺手把「没被插值过的原图」留下来，下一次 begin 直接拿它出发（角度累加），
/// 一次会话来多少次单次拖拽都只插值一次。
pub(in crate::bridge) struct RotateOrigin {
    pub(in crate::bridge) index: usize,
    /// `Rc` 是为了每次起手都不必再复制一份十几 MB 的位图。
    pub(in crate::bridge) layer: std::rc::Rc<Layer>,
    pub(in crate::bridge) degrees: f32,
    /// 提交后那一版图层内容。锚点是否还能用，看的是**这个图层自己有没有被
    /// 改过**，而不是文档历史条数——以前用 history_len，结果在别的图层上
    /// 画一笔、或者随手撤销一下，锚点就废了：base 归零，框退回轴对齐的扩边
    /// 矩形，看着就像"放手角度复位"。
    pub(in crate::bridge) committed: std::rc::Rc<Layer>,
}

pub(in crate::bridge) struct Rendered {
    pub(in crate::bridge) url: String,
    pub(in crate::bridge) display: Size,
    pub(in crate::bridge) preview: Size,
    /// JSON of the preview histogram (only while the adjust panel is open).
    pub(in crate::bridge) histogram: Option<String>,
}

/// What the preview file on disk currently shows, for the zoom-only skip in
/// [`Session::render`]. `special` marks frames produced under a transient
/// visual state (live color adjust, hidden editing mark, live layer resize):
/// reusing their file would freeze or corrupt the next normal frame, so a
/// special render only becomes reusable after a fresh normal one replaced it.
pub(in crate::bridge) struct CachedRender {
    pub(in crate::bridge) target: Size,
    pub(in crate::bridge) epoch: u64,
    pub(in crate::bridge) url: String,
    pub(in crate::bridge) special: bool,
}

pub(in crate::bridge) enum Refresh {
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
        marks_json: String,
        dirty: bool,
        adjust_open: bool,
        adjust_target_name: String,
        adjust_values: String,
        adjust_histogram: String,
        /// 这一帧是「拖动中的降质实时预览」（见 `live_cap`）：状态栏不该
        /// 因此报「预览降质」。
        live: bool,
    },
    Err(String),
}

impl Session {
    /// Build a session from already-decoded pixels (the decode itself runs
    /// either inline for automation or on the worker thread for the UI).
    pub(in crate::bridge) fn from_decoded(
        path: &Path,
        decoded: Decoded,
        viewport: Size,
        save_quality: u8,
        dpr: f64,
    ) -> (Self, i32) {
        Self::from_decoded_with_layers(path, decoded, viewport, save_quality, dpr, None)
    }

    /// Same as `from_decoded`, but installs the real PSD layer stack when the
    /// stage-2 parse is already available (synchronous/automation path).
    pub(in crate::bridge) fn from_decoded_with_layers(
        path: &Path,
        decoded: Decoded,
        viewport: Size,
        save_quality: u8,
        dpr: f64,
        layers: Option<decode::PsdLayerSet>,
    ) -> (Self, i32) {
        let t0 = Instant::now();
        let layered = matches!(&layers, Some(set) if !set.layers.is_empty());
        let (doc, note) = match layers {
            Some(set) if !set.layers.is_empty() => {
                let mut text = format!("PSD 分层已就绪：{} 个可编辑图层", set.layer_count());
                if !set.degraded.is_empty() {
                    text.push_str(&format!(
                        "（已降级 {} 项：{}）",
                        set.degraded.len(),
                        set.degraded.join("、")
                    ));
                }
                (
                    Document::from_psd_layers(set.canvas, set.into_seeds()),
                    Some(text),
                )
            }
            _ => (Document::from_image(decoded.image), decoded.note.clone()),
        };
        let dpr = if dpr.is_finite() && dpr > 0.0 { dpr } else { 1.0 };
        let zoom = fit_zoom(doc.canvas, viewport, dpr);
        let ms = t0.elapsed().as_millis().min(i32::MAX as u128) as i32;
        // A relative CLI path ("photo.jpg") must not leak into the session: the
        // status-bar path, the "save a copy" target and the "open the same file
        // in a new window" flow all need an absolute one. `absolute` keeps the
        // normal form (no \\?\ prefix, unlike canonicalize).
        let source = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        (
            Self {
                id: SESSION_SEQ.fetch_add(1, Ordering::Relaxed),
                doc,
                source: Some(source),
                zoom,
                fit: true,
                render_count: 0,
                dirty: false,
                save_quality,
                preview_hidden_mark: -1,
                transform: None,
                sel_flip_sign: (false, false),
                composer: render::PreviewComposer::new(),
                last_display: Size::new(0, 0),
                last_render: None,
                adjust: None,
                downsampled_from: decoded.downsampled_from,
                psd_layered: layered,
                note,
                dpr,
                quick_mask: false,
                sel_rev: 0,
                brush_live: None,
                gradient_live: None,
                patch_live: None,
                patch_region: None,
                heal_source: None,
                rotate_origin: None,
                rotate_token: 0,
                snap_statics: Vec::new(),
                snap: editcore::snap::SnapResult::default(),
                mods: 0,
                disp_scale: 1.0,
                gesture: -1,
                gesture_start: (0.0, 0.0),
                gesture_rect: (0.0, 0.0, 0.0, 0.0),
                fit_content: None,
                gesture_index: -1,
                fit_occ: editcore::alpha_fit::Occupancy::default(),
                full_composites: 0,
                gpu_live: false,
                gpu_static_url: String::new(),
                gpu_active_url: String::new(),
                gpu_index: 0,
                gpu_rect: (0.0, 0.0, 0.0, 0.0, 1.0),
            },
            ms,
        )
    }

    /// Synchronous open (automation only: --selftest / --script / probes run
    /// script ops immediately after the load, so the document must exist).
    pub(in crate::bridge) fn open(
        path: &Path,
        viewport: Size,
        save_quality: u8,
        opts: DecodeOptions,
        dpr: f64,
    ) -> Result<(Self, i32), String> {
        let t0 = Instant::now();
        let decoded = decode_path(path, opts).map_err(|e| e.to_string())?;
        let decode_ms = t0.elapsed().as_millis().min(i32::MAX as u128) as i32;
        // Automation (--script / --selftest / probes) must see the FINAL
        // layered document inline. It never goes through the editing lock.
        // IMAGEEDIT_PSD_SYNC_COMPOSITE is a probe-only hook that keeps the
        // composite placeholder, so the write-back guard can be exercised
        // without an asynchronous stage-2 run.
        let force_composite = std::env::var("IMAGEEDIT_PSD_SYNC_COMPOSITE").is_ok();
        let layers = if is_psd_path(path)
            && decoded.downsampled_from.is_none()
            && !force_composite
        {
            std::fs::read(path)
                .ok()
                .and_then(|bytes| decode::decode_psd_layers(&bytes, opts).ok())
                .filter(|set| !set.layers.is_empty())
        } else {
            None
        };
        let (session, _) =
            Self::from_decoded_with_layers(path, decoded, viewport, save_quality, dpr, layers);
        Ok((session, decode_ms))
    }

    pub(in crate::bridge) fn refit(&mut self, viewport: Size) {
        self.zoom = fit_zoom(self.doc.canvas, viewport, self.dpr);
        self.fit = true;
    }

    pub(in crate::bridge) fn refit_if_needed(&mut self, viewport: Size) {
        if self.fit {
            self.refit(viewport);
        }
    }

    pub(in crate::bridge) fn zoom_by(&mut self, factor: f64) {
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.fit = false;
    }

    /// Rebuild the preview bitmap. `zoom_only` marks a refresh triggered by a
    /// pure zoom change (wheel / slider / presets): when that zoom lands in
    /// the >100% regime the preview target is pinned to the canvas resolution,
    /// so if the previous frame already used that target and the document has
    /// not changed, the existing BMP is still exactly right — reuse its URL
    /// and let the QML Image stretch (sub-millisecond instead of seconds).
    /// Non-zoom refreshes (mark drags, layer ops, live gestures) never skip:
    /// some of them mutate the document without bumping `edit_epoch`.
    pub(in crate::bridge) fn render(&mut self, zoom_only: bool, dpr: f64) -> Result<Rendered, String> {
        let prof_t0 = Instant::now();
        let canvas = self.doc.canvas;
        let display = display_size(canvas, self.zoom, dpr);
        let target = preview_target(display, canvas, dpr);
        // 拖动中的实时预览可以先用小图顶上：QML 会把它拉到 display 尺寸，
        // 松手那一帧再按原分辨率重画。合成/编码/写盘的开销都随像素数线性
        // 下降，这是让旋转跟手最划算的一刀。
        let target = match self.transform.as_ref().and_then(|t| t.cap()) {
            Some(cap) => capped(target, cap),
            None => target,
        };
        if target.pixels() == 0 {
            return Err("empty canvas".into());
        }
        self.last_display = display;
        // ★Tier 1：GPU 快路径期间主预览不产生新帧。三层里只有活动层在动，
        // QML 直接改它的位置/尺寸/不透明度即可，CPU 全幅合成、BMP 编码、
        // 临时文件写入、Qt 解码全部跳过（拖动帧合成次数 = 0）。
        if self.gpu_live {
            if let Some(cached) = self.last_render.as_ref() {
                self.full_composites = self.full_composites.saturating_add(1);
                let url = cached.url.clone();
                let preview = cached.target;
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"gpu-skip\",\"skipped\":{}}}",
                    self.full_composites
                );
                return Ok(Rendered { url, display, preview, histogram: None });
            }
        }
        let special = self.adjust.is_some()
            || self.preview_hidden_mark >= 0
            || self.transform.as_ref().is_some_and(|t| t.special());
        if zoom_only && !special {
            let reusable = self.last_render.as_ref().is_some_and(|c| {
                !c.special && size_close(c.target, target) && c.epoch == self.doc.edit_epoch
            });
            if reusable {
                let url = self.last_render.as_ref().expect("checked").url.clone();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"render-skip\",\"target\":\"{}x{}\"}}",
                    target.width, target.height
                );
                return Ok(Rendered {
                    url,
                    display,
                    preview: target,
                    histogram: None,
                });
            }
        }
        // Inline text editing: the mark being edited is hidden from the
        // preview only (restored right after the composite; never undoable).
        // An in-flight layer resize routes the composite through the
        // live-scale path (snapshot stretch, document untouched). The open
        // "调整颜色" panel applies its live parameters to the scope's
        // preview-resolution bitmap only.
        let adjust = self
            .adjust
            .map(|state| (state.target, state.params));
        let hidden_idx = self.preview_hidden_mark;
        let image = if hidden_idx >= 0 {
            // 预览时临时隐藏某个标记：可见性在图层上，绕开历史直接改。
            match self.doc.mark_layer_index(hidden_idx as usize) {
                Some(li) => {
                    let saved = self.doc.layers[li].visible;
                    self.doc.layers[li].visible = false;
                    let img = self.composite_for_preview(target, adjust);
                    self.doc.layers[li].visible = saved;
                    img
                }
                None => self.composite_for_preview(target, adjust),
            }
        } else {
            self.composite_for_preview(target, adjust)
        };
        let composite_ms = prof_t0.elapsed().as_millis();
        // The panel graph reflects the ADJUSTED preview (like Preview), so it
        // is computed from the composited pixels and only while it is open.
        // It must be sampled BEFORE the quick-mask overlay, which would skew
        // the histogram with the dimming.
        let histogram = if self.adjust.is_some() {
            Some(Histogram::of(&image).to_json())
        } else {
            None
        };
        // Quick mask (Q): dim everything outside the selection, preview only.
        let mut image = image;
        if self.quick_mask {
            apply_quick_mask(&mut image, self.doc.selection.as_ref(), canvas);
        }
        // BMP on purpose: uncompressed means both the encode here and Qt's
        // decode are near memcpy speed. PNG compression dominated each live
        // drag tick (tens of ms); BMP keeps the preview path real-time. The
        // URL keeps changing (forces Qt to re-read) while temp files stay
        // bounded; the session id keeps concurrent windows separate.
        //
        // ★环形文件数是 4 而不是 2：QML 侧用两张 Image 交替显示（一张解码
        // 时另一张继续显示上一帧，避免换源瞬间画面空白）。槽位按帧的奇偶
        // 轮换，若只有 2 个文件名，某个槽位下次拿到的又会是它当前持有的那个
        // URL——`Image.source` 赋同一个值时 Qt 不会重新加载，画面就卡住。
        //
        // ★11-07 晚修正：改用**每次渲染唯一**的文件名。原来 `% 4` 轮换在
        //   GPU 拖动下必然死锁：一次 GPU 拖动 = gpu_begin(+1) + 提交渲染(+1)，
        //   相邻两次提交渲染的计数步长恰为 2，mod 4 下文件名只在两个值之间
        //   振荡 —— 第三次提交的名字必然等于双缓冲「后台槽」当前持有的 URL，
        //   pumpPreview 的防重派发守卫把它拒掉：拖动提交后**画面停在旧帧**，
        //   而选框（layers_json）实时跟进 → 「图层和选框位置不一致」，直到
        //   改尺寸等连续两次非 GPU 渲染打破循环。步长随 GPU 次数可取任意
        //   值，只有唯一文件名对一切步长稳健；旧文件延迟多帧删除（见下）。
        let bytes = encode(&image, Format::Bmp, 90).map_err(|e| e.to_string())?;
        let encode_ms = prof_t0.elapsed().as_millis();
        self.render_count = self.render_count.wrapping_add(1);
        let path = std::env::temp_dir().join(format!(
            "imageedit-preview-{}-{}.bmp",
            self.id, self.render_count
        ));
        let frame_bytes = bytes.len();
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        // 环形保留 N 帧再删 —— 双缓冲最多引用最近两帧，但 Qt 的异步解码是
        // 「先拿 URL、稍后才打开文件」：拖动帧密集（60ms/帧）时删到 3 帧前
        // 只剩 ~180ms 余量，赶上一次慢解码就会 decode-failed，那一侧槽位
        // 从此卡死（画面停在旧帧）。
        //
        // ★11-08：原来固定 12 帧，大预览（4K 全幅 BMP 每张 30MB+）与拖动
        //   帧率一起会把「解码比删除慢」的窗口放大。改成按体积取帧数：
        //   总占用压在 ~96MB 以内，小帧（360p ≈ 450KB）能留 32 帧 ≈ 2s，
        //   大帧至少留 3 帧（够双缓冲 + 一次在途解码）。同步解码兜底
        //   （CanvasView.pulse）可能比预取更晚才去开文件，这个余量要留够。
        let keep = if frame_bytes == 0 {
            3
        } else {
            (96 * 1024 * 1024 / frame_bytes).clamp(3, 32)
        };
        if let Some(old) = self.render_count.checked_sub(keep as u32) {
            let stale = std::env::temp_dir().join(format!(
                "imageedit-preview-{}-{}.bmp",
                self.id, old
            ));
            let _ = std::fs::remove_file(stale);
        }
        println!(
            "{{\"phase\":\"ui\",\"event\":\"render-profile\",\"composite\":{},\"encode\":{},\"write\":{},\"target\":\"{}x{}\"}}",
            composite_ms,
            encode_ms - composite_ms,
            prof_t0.elapsed().as_millis(),
            target.width,
            target.height
        );
        // The panel graph reflects the adjusted preview (like Preview), so it
        // is computed from the composited pixels and only while it is open.
        let url = file_url(&path);
        self.last_render = Some(CachedRender {
            target,
            epoch: self.doc.edit_epoch,
            url: url.clone(),
            special,
        });
        Ok(Rendered {
            url,
            display,
            preview: Size::new(image.width(), image.height()),
            histogram,
        })
    }

    /// Composite one preview frame. ★P0-2b1：三种手势共用一个分派点。
    ///
    /// 旧实现是「先看 live_rotate、再看 live_scale」的两段式优先级；注释
    /// 写「缩放 > 旋转」、代码却是旋转先返回，谁残留谁就吞掉另一边的预览。
    /// 统一会话后变体互斥，不再存在优先级歧义。移动手势直接改文档
    /// offset，走普通合成即可。
    pub(in crate::bridge) fn composite_for_preview(
        &mut self,
        target: Size,
        adjust: Option<(AdjustTarget, Adjust)>,
    ) -> RgbaImage {
        let Some(ts) = self.transform.as_ref() else {
            return self.composer.composite_adjusted(&self.doc, target, adjust);
        };
        match ts {
            TransformSession::Rotate { frame: Some(lr), .. } => {
                self.composer.composite_live_rotate_adjusted(
                    &self.doc,
                    target,
                    lr.index,
                    lr.snapshot.img(),
                    lr.center,
                    lr.degrees,
                    lr.expand,
                    lr.token,
                    adjust,
                )
            }
            TransformSession::Scale(ls) if ls.skew != (0.0, 0.0) => {
                // P1-A：Ctrl+拖边的斜切帧走参数化采样（与落定同一个
                // sample_affine 采样器，双线性档），不再按 AABB 拉伸。
                self.composer.composite_live_skew_adjusted(
                    &self.doc,
                    target,
                    ls.index,
                    &ls.snapshot,
                    ls.skew,
                    ls.center_delta,
                    adjust,
                )
            }
            TransformSession::Scale(ls) => self.composer.composite_live_scale_adjusted(
                &self.doc,
                target,
                ls.index,
                &ls.snapshot,
                ls.to,
                ls.offset,
                adjust,
                ls.flip,
            ),
            // Move，或 Rotate 还没出第一帧：文档已是最新状态。
            _ => self.composer.composite_adjusted(&self.doc, target, adjust),
        }
    }

    /// Scope label for the panel title.
    pub(in crate::bridge) fn adjust_target_name(&self, target: AdjustTarget) -> String {
        match target {
            AdjustTarget::Base => "背景".to_string(),
            AdjustTarget::Layer(index) => self
                .doc
                .layers
                .get(index)
                .map(|layer| layer.name.clone())
                .unwrap_or_else(|| format!("图层 {}", index + 1)),
        }
    }

    /// Scope captured when the panel opens: the selected layer, else the base
    /// photo. Once the base has been "unlocked" into a regular layer the base
    /// bitmap is only transparent padding, so fall back to the bottom layer.
    pub(in crate::bridge) fn adjust_scope(&self, selected_layer: i32) -> AdjustTarget {
        if selected_layer >= 0 && (selected_layer as usize) < self.doc.layers.len() {
            return AdjustTarget::Layer(selected_layer as usize);
        }
        // 背景就是 layers[0]，`AdjustTarget::Base` 恒指向它，不需要迁移特判。
        AdjustTarget::Base
    }

    /// Default "Save a copy" target. A PSD source now defaults to `.psd` —
    /// the encoder exists, so `<stem>-edited.png` is no longer necessary.
    pub(in crate::bridge) fn default_save_path(&self) -> (PathBuf, SaveFormat) {
        match &self.source {
            Some(src) => {
                let stem = src
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "image".to_string());
                let format = save_format_for_source(src);
                (
                    src.with_file_name(format!(
                        "{stem}-edited.{}",
                        extension_for_save(format)
                    )),
                    format,
                )
            }
            None => (
                std::env::temp_dir().join("imageedit-edited.png"),
                SaveFormat::Image(Format::Png),
            ),
        }
    }

    /// Validate a save target and return its EXPLICIT format. This is the
    /// single gate behind both Ctrl+S and Save-As: `.psd` can never fall
    /// through to `unwrap_or(Png)` and write image bytes into a PSD file.
    ///
    /// §8 / C8: a composite-only PSD (layers skipped / failed / flat /
    /// downsampled) or any downsampled source overwrite is rejected with an
    /// actionable hint; exporting to a DIFFERENT path stays allowed.
    pub(in crate::bridge) fn save_target(&self, target: &Path) -> Result<SaveFormat, String> {
        validate_save_target(
            target,
            self.source.as_deref(),
            self.downsampled_from.is_some(),
            self.psd_layered,
        )
    }

    /// 面板用的图层列表（显示顺序 = 顶 → 底）。
    ///
    /// 统一模型后**每一行都是真图层**：背景是 `layers[0]`，标记是
    /// `kind:"mark"` 的图层。`i` 是文档索引（可直接回传给 `select_layer`
    /// 等入口），`id` 是稳定 id（多选集合 / 缩略图缓存用）。
    /// 兼容字段：背景行仍带 `base:true` / `bd`，脚本探针不至于全废。
    pub(in crate::bridge) fn layers_json(&self) -> String {
        let base_deleted = self.doc.base_deleted();
        let mut parts: Vec<String> = Vec::new();
        for (i, layer) in self.doc.layers.iter().enumerate().rev() {
            let name = layer.name.replace('\\', "\\\\").replace('"', "\\\"");
            let bounds = layer.bounds();
            // a = 累计旋转角；sw/sh = 变换前原图尺寸（QML 画倾斜框用）。
            // 标记没有位图（`img()` 只是给旧代码兜底的 1×1 透明图），
            // 这里按它的画布外接框报，别让面板以为原图是 1×1。
            let angle = layer.xform().map(|t| t.rotation).unwrap_or(0.0);
            let swsh = layer.xform().map(|t| (t.source.width(), t.source.height()));
            let (sw, sh) = swsh.unwrap_or_else(|| match layer.raster() {
                Some(r) => (r.image.width(), r.image.height()),
                None => (
                    bounds.width.max(0) as u32,
                    bounds.height.max(0) as u32,
                ),
            });
            let kind = if layer.is_mark() { "mark" } else { "raster" };
            // 标记行要用种类图标；接口给的是 MarkKind 的数字（与 marks_json 一致）。
            let mk = match &layer.content {
                editcore::LayerContent::Mark(m) => format!(",\"mk\":{}", m.kind as i32),
                _ => String::new(),
            };
            let extra = if layer.background {
                format!(
                    ",\"base\":true,\"bg\":true,\"bd\":{base_deleted}"
                )
            } else {
                mk
            };
            parts.push(format!(
                "{{\"i\":{i},\"id\":{},\"n\":\"{name}\",\"v\":{},\"o\":{},\"l\":{},\"x\":{},\"y\":{},\"w\":{},\"h\":{},\"a\":{:.3},\"sw\":{},\"sh\":{},\"k\":\"{kind}\"{extra}}}",
                layer.id,
                layer.visible,
                (layer.opacity * 100.0).round() as i32,
                layer.locked,
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                angle,
                sw,
                sh
            ));
        }
        format!("[{}]", parts.join(","))
    }

    // ---- P0-2b2 Tier 1：静态层 + 活动层两纹理 ----------------------
    /// 起手一次生成两张纹理：
    ///  * static = 文档里「活动层隐藏」后的完整预览（base + 其它层）；
    ///  * active = 活动层自身位图（未插值原图，QML 拉伸到显示尺寸）。
    /// 之后每个拖动帧只改 QML 的 x/y/w/h/opacity 属性，render() 完全跳过
    /// 全幅合成 + BMP 编码 + 写盘 + 解码 —— 这正是「拖动帧 CPU 全幅合成 = 0」。
    ///
    /// 自动回退（§11.4）：调色面板、快速蒙版、文本内联编辑、可见标记 ——
    /// 这些状态要么会改 static 像素，要么绘制顺序在活动层之上，命中即
    /// 返回 false 走 CPU 路径，优先保证正确性。
    pub(in crate::bridge) fn gpu_begin(
        &mut self,
        index: usize,
        disp_scale: f64,
        dpr: f64,
    ) -> bool {
        self.gpu_end();
        if self.adjust.is_some()
            || self.quick_mask
            || self.preview_hidden_mark >= 0
            || self.doc.layers.iter().any(|l| l.is_mark() && l.visible)
            || !(disp_scale > 0.0)
        {
            return false;
        }
        let Some(layer) = self.doc.layers.get(index) else {
            return false;
        };
        if !layer.visible || layer.opacity <= 0.0 {
            return false;
        }
        let canvas = self.doc.canvas;
        let display = display_size(canvas, self.zoom, dpr);
        let target = preview_target(display, canvas, dpr);
        if target.pixels() == 0 {
            return false;
        }
        let active_img = layer.img().clone();
        let opacity = layer.opacity;
        let saved = self.doc.layers[index].visible;
        self.doc.layers[index].visible = false;
        let static_img = self.composer.composite_adjusted(&self.doc, target, None);
        self.doc.layers[index].visible = saved;

        let (Ok(bytes_s), Ok(bytes_a)) = (
            encode(&static_img, Format::Bmp, 90),
            encode(&active_img, Format::Bmp, 90),
        ) else {
            return false;
        };
        self.render_count = self.render_count.wrapping_add(1);
        let tag = self.render_count;
        let ps = std::env::temp_dir().join(format!("imageedit-gpu-{}-s-{tag}.bmp", self.id));
        let pa = std::env::temp_dir().join(format!("imageedit-gpu-{}-a-{tag}.bmp", self.id));
        if std::fs::write(&ps, bytes_s).is_err() || std::fs::write(&pa, bytes_a).is_err() {
            return false;
        }
        self.gpu_static_url = file_url(&ps);
        self.gpu_active_url = file_url(&pa);
        self.gpu_index = index;
        self.gpu_rect = (
            self.doc.layers[index].offset().0 as f64 * disp_scale,
            self.doc.layers[index].offset().1 as f64 * disp_scale,
            active_img.width() as f64 * disp_scale,
            active_img.height() as f64 * disp_scale,
            opacity as f64,
        );
        self.gpu_live = true;
        println!(
            "{{\"phase\":\"ui\",\"event\":\"gpu-begin\",\"index\":{index},\"target\":\"{}x{}\"}}",
            target.width, target.height
        );
        true
    }

    /// 每帧只重算活动层的显示矩形（纯浮点，零分配）。
    pub(in crate::bridge) fn gpu_update_rect(&mut self, disp_scale: f64) {
        if !self.gpu_live || !(disp_scale > 0.0) {
            return;
        }
        if let Some(layer) = self.doc.layers.get(self.gpu_index) {
            self.gpu_rect.0 = layer.offset().0 as f64 * disp_scale;
            self.gpu_rect.1 = layer.offset().1 as f64 * disp_scale;
            self.gpu_rect.4 = layer.opacity as f64;
        }
    }

    pub(in crate::bridge) fn gpu_end(&mut self) {
        self.gpu_live = false;
    }

    pub(in crate::bridge) fn gpu_rect_json(&self) -> String {
        let (x, y, w, h, o) = self.gpu_rect;
        format!(
            r#"{{"x":{x:.2},"y":{y:.2},"w":{w:.2},"h":{h:.2},"o":{o:.3}}}"#
        )
    }

    /// P0-5：选中图层的外接框参数（数值面板只读回显）。
    /// 普通位图图层用 identity 参数表达，避免面板显示两套语义。
    pub(in crate::bridge) fn transform_panel_json(&self, index: i32) -> String {
        let Some(layer) = self.doc.layers.get(index.max(0) as usize) else {
            return "{\"has\":false}".to_string();
        };
        // 不要为「读一次数值」clone 整张位图：identity 层直接由 offset/尺寸报。
        let (x, y, w, h, rot, sx, sy, fx, fy, param) = match layer.xform().as_ref() {
            Some(t) => {
                let p = t.panel_values();
                (
                    p.x, p.y, p.width, p.height, p.rotation,
                    t.scale.0, t.scale.1, t.flip.0, t.flip.1, true,
                )
            }
            None => (
                layer.offset().0 as f64,
                layer.offset().1 as f64,
                layer.img().width() as f64,
                layer.img().height() as f64,
                0.0,
                1.0,
                1.0,
                false,
                false,
                false,
            ),
        };
        format!(
            r#"{{"has":true,"x":{x:.2},"y":{y:.2},"w":{w:.2},"h":{h:.2},"rot":{rot:.2},"sx":{sx:.4},"sy":{sy:.4},"fx":{fx},"fy":{fy},"param":{param}}}"#
        )
    }

    /// JSON list of markup annotations for the QML overlay (selection box,
    /// handles, hit-testing): index, kind, bounds, stroke, colors, fill.
    pub(in crate::bridge) fn marks_json(&self) -> String {
        let hex = |c: [u8; 4]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
        let escape = |s: &str| {
            let mut out = String::with_capacity(s.len());
            for ch in s.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c => out.push(c),
                }
            }
            out
        };
        let kind = |k: MarkKind| match k {
            MarkKind::Rect => 0,
            MarkKind::Ellipse => 1,
            MarkKind::Arrow => 2,
            MarkKind::Brush => 3,
            MarkKind::Mosaic => 4,
            MarkKind::Text => 5,
        };
        let parts: Vec<String> = self
            .doc
            .marks()
            .enumerate()
            .map(|(i, m)| {
                let (x0, y0, x1, y1) = m.rect;
                let extra = match m.kind {
                    MarkKind::Mosaic => format!(",\"cell\":{}", m.cell),
                    MarkKind::Text => format!(
                        ",\"t\":\"{}\",\"fn\":\"{}\",\"fs\":{},\"b\":{},\"it\":{},\"u\":{},\"al\":{}",
                        escape(&m.text),
                        escape(&m.font),
                        m.font_size,
                        m.bold as u8,
                        m.italic as u8,
                        m.underline as u8,
                        m.align
                    ),
                    _ => String::new(),
                };
                format!(
                    "{{\"i\":{i},\"k\":{},\"x\":{x0:.1},\"y\":{y0:.1},\"w\":{:.1},\"h\":{:.1},\"s\":{},\"c\":\"{}\",\"f\":{},\"fc\":\"{}\",\"v\":{}}}",
                    kind(m.kind),
                    x1 - x0,
                    y1 - y0,
                    m.stroke,
                    hex(m.color),
                    m.fill,
                    hex(m.fill_color),
                    self.doc
                        .mark_layer_index(i)
                        .map(|li| self.doc.layers[li].visible)
                        .unwrap_or(false)
                )
                .replace("}", &format!("{}}}", extra))
            })
            .collect();
        format!("[{}]", parts.join(","))
    }

    pub(in crate::bridge) fn refresh(&mut self, zoom_only: bool, dpr: f64) -> Refresh {
        // GPU 快路径：每次 refresh 只重算活动层的显示矩形（纯浮点）。
        if self.gpu_live {
            self.gpu_update_rect(self.disp_scale);
        }
        match self.render(zoom_only, dpr) {
            Ok(rendered) => {
                let adjust = self.adjust;
                let adjust_target_name = match adjust {
                    Some(state) => self.adjust_target_name(state.target),
                    None => String::new(),
                };
                Refresh::Ok {
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
                    marks_json: self.marks_json(),
                    dirty: self.dirty,
                    adjust_open: adjust.is_some(),
                    adjust_target_name,
                    adjust_values: adjust_json(
                        &adjust.map(|state| state.params).unwrap_or_default(),
                    ),
                    adjust_histogram: rendered.histogram.unwrap_or_default(),
                    live: self.transform.as_ref().and_then(|t| t.cap()).is_some(),
                }
            }
            Err(err) => Refresh::Err(err),
        }
    }
}

impl Session {
    /// Outline of the current selection as QML reads it: `[[[x,y],...],...]`
    /// in IMAGE coordinates. Downsampled by `step` for big canvases — the
    /// ants are one pixel wide, so tracing 60 MP at full resolution would
    /// cost hundreds of ms for no visible gain.
    pub(in crate::bridge) fn selection_outline_json(&self) -> String {
        let Some(state) = self.doc.selection.as_ref() else {
            return "[]".to_string();
        };
        let m = state.mask();
        if m.is_empty() {
            return "[]".to_string();
        }
        let c = self.doc.canvas;
        let step = ((c.width.max(c.height) as f64 / 1400.0).ceil() as u32).max(1);
        let parts: Vec<String> = m
            .outline(step)
            .iter()
            .map(|ring| {
                let pts: Vec<String> = ring
                    .iter()
                    .map(|(x, y)| format!("[{},{}]", x.round() as i32, y.round() as i32))
                    .collect();
                format!("[{}]", pts.join(","))
            })
            .collect();
        format!("[{}]", parts.join(","))
    }
}

/// Quick-mask overlay (PS's Q): multiply the pixels OUTSIDE the selection
/// toward black so the feathered edge becomes visible. Preview only — the
/// document pixels and the saved file are untouched.
fn apply_quick_mask(image: &mut RgbaImage, sel: Option<&SelectionState>, canvas: Size) {
    let Some(state) = sel else {
        return;
    };
    // ★必须用 effective()（= mask + 羽化），不是 mask()：羽化只在 effective()
    // 里算，用 mask() 会让快速蒙版在羽化半径内仍是硬边，完全看不出渐变。
    let mask = state.effective();
    if mask.is_empty() || canvas.width == 0 || canvas.height == 0 {
        return;
    }
    let sw = image.width() as f64 / canvas.width as f64;
    let sh = image.height() as f64 / canvas.height as f64;
    if sw <= 0.0 || sh <= 0.0 {
        return;
    }
    for y in 0..image.height() {
        let cy = (y as f64 / sh) as i32;
        for x in 0..image.width() {
            let cx = (x as f64 / sw) as i32;
            let cov = mask.coverage_at(cx, cy);
            if cov >= 255 {
                continue;
            }
            let dim = 1.0 - 0.55 * (1.0 - cov as f64 / 255.0);
            let p = image.get_pixel_mut(x, y);
            for i in 0..3 {
                p.0[i] = (p.0[i] as f64 * dim).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}
