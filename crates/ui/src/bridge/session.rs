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
    /// UI lock for the base row (blocks delete). Plain flag, not undoable.
    pub(in crate::bridge) base_locked: bool,
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
    /// 连续旋转会话的原图锚点（见 `RotateOrigin`）。**跨手势**存在：一次
    /// 拖拽结束后下一次还要从没插值过的原图接着转，所以不能放进
    /// `TransformSession`（会话一结束就被 take 掉）。
    pub(in crate::bridge) rotate_origin: Option<RotateOrigin>,
    /// 旋转会话自增号（见 `LiveRotate::token`）。同样是跨会话的缓存版本号。
    pub(in crate::bridge) rotate_token: u64,
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
    pub(in crate::bridge) fn take_move_start(&mut self) -> Option<(usize, (i32, i32))> {
        match self.transform.take() {
            Some(TransformSession::Move { index, start, .. }) => Some((index, start)),
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
        match self.transform.as_ref() {
            None => r#"{"active":false}"#.to_string(),
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
                    "{{\"active\":true,\"kind\":\"{}\",\"index\":{},\"special\":{},\"cap\":{},{}}}",
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
                || o.committed.image != *before_image
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
        let ow = (origin_layer.image.width() as f64 * sx).round().max(1.0) as u32;
        let oh = (origin_layer.image.height() as f64 * sy).round().max(1.0) as u32;
        let old = (origin_layer.image.width(), origin_layer.image.height());
        // 锚点是「已缩放的原图」克隆，不再是任何 transform 的有效缓存。
        origin_layer.ensure_baked();
        origin_layer.image = render::resize_to(&origin_layer.image, Size::new(ow, oh));
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

/// Message from the background decode worker (see `BridgeRust::decode_job`).
pub(in crate::bridge) struct DecodeMsg {
    pub(in crate::bridge) path: PathBuf,
    pub(in crate::bridge) result: Result<(Decoded, i32), String>,
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
    Move {
        index: usize,
        start: (i32, i32),
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
    /// 预览用的图层原点，必须取**当前**图层的 offset。
    ///
    /// ★不能用 `snapshot.offset`：`snapshot` 在连续旋转时会复用「没被插值过
    ///   的原图」，那是第一次旋转前的状态，它的 offset 也比当前的小一圈（扩张
    ///   过的外接矩形会往左上顶）。拿它出图，拖动时内容就画在旧位置上，松手
    ///   提交（用当前 offset + delta）才"跳"进框里 —— 用户报的正是这个。
    pub(in crate::bridge) offset: (i32, i32),
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
        let t0 = Instant::now();
        let doc = Document::from_image(decoded.image);
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
                base_locked: true,
                save_quality,
                preview_hidden_mark: -1,
                transform: None,
                sel_flip_sign: (false, false),
                composer: render::PreviewComposer::new(),
                last_display: Size::new(0, 0),
                last_render: None,
                adjust: None,
                downsampled_from: decoded.downsampled_from,
                note: decoded.note,
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
        let (session, _) = Self::from_decoded(path, decoded, viewport, save_quality, dpr);
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
        let image = if hidden_idx >= 0
            && (hidden_idx as usize) < self.doc.marks.len()
        {
            let idx = hidden_idx as usize;
            let saved = self.doc.marks[idx].visible;
            self.doc.marks[idx].visible = false;
            let img = self.composite_for_preview(target, adjust);
            self.doc.marks[idx].visible = saved;
            img
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
        // 4 个名字保证每个槽位相邻两次拿到的 URL 必然不同。
        let bytes = encode(&image, Format::Bmp, 90).map_err(|e| e.to_string())?;
        let encode_ms = prof_t0.elapsed().as_millis();
        self.render_count = self.render_count.wrapping_add(1);
        let path = std::env::temp_dir().join(format!(
            "imageedit-preview-{}-{}.bmp",
            self.id,
            self.render_count % 4
        ));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
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
                    &lr.snapshot.image,
                    lr.offset,
                    lr.degrees,
                    lr.expand,
                    lr.token,
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
        if self.doc.base_converted && !self.doc.layers.is_empty() {
            return AdjustTarget::Layer(0);
        }
        AdjustTarget::Base
    }

    pub(in crate::bridge) fn default_save_path(&self) -> (PathBuf, Format) {
        match &self.source {
            Some(src) => {
                let stem = src
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "image".to_string());
                // A PSD source can't be re-encoded (no PSD writer); default
                // the target to PNG instead of a corrupt "<stem>-edited.psd".
                let format = save_format_for_source(src);
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

    /// Ctrl+S semantics: overwrite the file the document was loaded from,
    /// keeping its original format. Without a source (e.g. a pasted image)
    /// there is nothing to overwrite — QML routes those to the Save-As
    /// dialog instead.
    ///
    /// PSD is preview-only (the decoder flattens layers and image 0.25 has
    /// no PSD encoder): saving back would `unwrap_or(Png)` into PNG bytes
    /// written to the .psd path. Block it with an actionable hint.
    pub(in crate::bridge) fn save(&self) -> Result<PathBuf, String> {
        let src = self.source.clone().ok_or_else(|| "没有可保存的文件".to_string())?;
        if is_psd_path(&src) {
            return Err(PSD_SAVE_HINT.to_string());
        }
        let format = save_format_for_source(&src);
        self.write_to(&src, format)?;
        Ok(src)
    }

    pub(in crate::bridge) fn write_to(&self, path: &Path, format: Format) -> Result<(), String> {
        let bytes = encode(&self.doc.flatten(), format, self.save_quality)
            .map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }

    pub(in crate::bridge) fn layers_json(&self) -> String {
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
        // "deleted" (Delete key flow) for the base row. Once the base has
        // been converted into a regular layer (panel unlock) the synthetic
        // row disappears entirely — the layer row speaks for itself.
        if !self.doc.base_converted {
            parts.push(format!(
                "{{\"i\":-1,\"n\":\"背景\",\"v\":{},\"bd\":{},\"o\":100,\"l\":{},\"x\":0,\"y\":0,\"w\":{},\"h\":{},\"base\":true}}",
                !base_deleted && !self.doc.base_hidden,
                base_deleted,
                base_locked,
                self.doc.canvas.width,
                self.doc.canvas.height
            ));
        }
        format!("[{}]", parts.join(","))
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
            .marks
            .iter()
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
                    m.visible
                )
                .replace("}", &format!("{}}}", extra))
            })
            .collect();
        format!("[{}]", parts.join(","))
    }

    pub(in crate::bridge) fn refresh(&mut self, zoom_only: bool, dpr: f64) -> Refresh {
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
