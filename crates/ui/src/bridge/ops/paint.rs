// 修复 / 油漆 / 图层旋转。所有「拖动中」的工具共用一个套路：
//   begin  → 记下目标位图的原始副本 + 手势参数
//   move   → 先按原始副本还原上一次写过的矩形，再整段重算 + 写回
//   end    → 把矩形内的前后两份位图补成一条撤销记录
// 这样一次涂抹只会留下一条 Ctrl+Z，而不是每帧一条。
use super::super::*;

/// 旋转拖动中实时预览的长边上限（像素）。只在**远超屏幕分辨率**时兜底，
/// 平时等于不压：压到 1600 再让 QML 拉回 1917 宽的窗口，整幅被放大 1.2 倍，
/// 拖动时看着就是"转起来发糊"（曾经的实际问题）。
const LIVE_ROTATE_EDGE: u32 = 4096;

/// 目标位图左上角的画布坐标：位图坐标 = 画布坐标 - origin。
fn target_origin(doc: &Document, target: AdjustTarget) -> (i32, i32) {
    match target {
        // base 的可见区域从 view 开始：base 坐标 = 画布 + view。
        AdjustTarget::Base => (-doc.view.x, -doc.view.y),
        AdjustTarget::Layer(i) => doc.layers.get(i).map(|l| l.offset).unwrap_or((0, 0)),
    }
}

fn target_image<'a>(doc: &'a Document, target: AdjustTarget) -> Option<&'a RgbaImage> {
    match target {
        AdjustTarget::Base => Some(&doc.base),
        AdjustTarget::Layer(i) => doc.layers.get(i).map(|l| &l.image),
    }
}

/// 选中图层优先，否则改背景（PS 语义）。选区 / 剪贴板这些"改像素"的
/// 操作共用同一个判定，保证工具之间的作用对象一致。
pub(super) fn pick_target(doc: &Document, selected: i32) -> Option<AdjustTarget> {
    if selected >= 0 && (selected as usize) < doc.layers.len() {
        Some(AdjustTarget::Layer(selected as usize))
    } else if doc.canvas.width > 0 && doc.canvas.height > 0 {
        Some(AdjustTarget::Base)
    } else {
        None
    }
}

pub(super) fn layer_locked(doc: &Document, target: AdjustTarget) -> bool {
    match target {
        AdjustTarget::Base => false,
        AdjustTarget::Layer(i) => doc.layers.get(i).map(|l| l.locked).unwrap_or(true),
    }
}

/// 把上一次写进文档的矩形按 `from` 还原（下一次重算前调用）。
fn restore_dirty(
    session: &mut Session,
    target: AdjustTarget,
    from: &RgbaImage,
    dirty: &mut Option<Rect>,
) {
    if let Some(rect) = dirty.take() {
        let (r, img) = editcore::crop_region(from, rect);
        if !r.is_empty() {
            session.doc.write_patch(
                target,
                &editcore::Patch {
                    rect: r,
                    image: img,
                },
            );
        }
    }
}

/// 笔迹 → 选区外的覆盖度 → 修复补丁 → 写回文档（不记历史）。
fn brush_live_apply(session: &mut Session) {
    let Some(mut st) = session.brush_live.take() else {
        return;
    };
    restore_dirty(session, st.target, &st.from, &mut st.dirty);
    let start = st.points.first().copied().unwrap_or((0.0, 0.0));
    let mask = editcore::stroke_mask(&st.points, st.origin, session.doc.canvas, st.brush);
    if let Some(mask) = mask {
        if let Some(img) = target_image(&session.doc, st.target) {
            let patch = if st.clone_from_source {
                st.source
                    .and_then(|src| editcore::heal_clone(img, &mask, start, src, st.strength))
            } else {
                editcore::spot_heal(img, &mask, st.strength)
            };
            if let Some(p) = patch {
                st.dirty = Some(p.rect);
                session.doc.write_patch(st.target, &p);
            }
        }
    }
    session.brush_live = Some(st);
}

/// 渐变预览：同样先还原再重算。
fn gradient_live_apply(session: &mut Session) {
    let Some(mut st) = session.gradient_live.take() else {
        return;
    };
    restore_dirty(session, st.target, &st.from, &mut st.dirty);
    let selection = session.doc.selection.as_ref().map(|s| s.effective());
    if let Some(img) = target_image(&session.doc, st.target) {
        let patch = editcore::gradient_fill(
            img,
            st.origin,
            session.doc.canvas,
            selection.as_ref(),
            st.start,
            st.end,
            st.c0,
            st.c1,
            st.radial,
            st.reverse,
            st.opacity,
            true,
        );
        if let Some(p) = patch {
            st.dirty = Some(p.rect);
            session.doc.write_patch(st.target, &p);
        }
    }
    session.gradient_live = Some(st);
}

fn patch_live_apply(session: &mut Session) {
    let Some(mut st) = session.patch_live.take() else {
        return;
    };
    restore_dirty(session, st.target, &st.from, &mut st.dirty);
    if let Some(region) = session.patch_region.clone() {
        if let Some(img) = target_image(&session.doc, st.target) {
            let patch = editcore::patch_fill(
                img,
                st.origin,
                &region,
                st.offset,
                st.feather,
                1.0,
            );
            if let Some(p) = patch {
                st.dirty = Some(p.rect);
                session.doc.write_patch(st.target, &p);
            }
        }
    }
    session.patch_live = Some(st);
}

/// 把「矩形内的前后两份」提交成一条历史。
fn commit_live(
    session: &mut Session,
    target: AdjustTarget,
    from: &RgbaImage,
    dirty: Option<Rect>,
) -> bool {
    let Some(rect) = dirty else { return false };
    let (r, before) = editcore::crop_region(from, rect);
    if r.is_empty() {
        return false;
    }
    let Some(after_img) = target_image(&session.doc, target) else {
        return false;
    };
    let (r2, after) = editcore::crop_region(after_img, r);
    if r2 != r {
        return false;
    }
    session.doc.commit_patch(target, r, before, after).is_ok()
}

impl qobject::Bridge {
    /// 记住修复 / 油漆工具与参数（`|` 分隔，QML 负责拼装与解析）。
    /// 只校验第一段的工具名和段数，数值范围由 QML 的 SpinBox 保证。
    pub fn save_paint_spec(mut self: Pin<&mut Self>, spec: &QString) {
        let text = spec.to_string();
        let parts: Vec<&str> = text.split('|').collect();
        if parts.len() != 12 {
            return;
        }
        if !matches!(
            parts[0],
            "none" | "spot" | "heal" | "patch" | "bucket" | "gradient"
        ) {
            return;
        }
        if self.paint_spec_saved().to_string() == text {
            return;
        }
        self.as_mut().set_paint_spec_saved(spec.clone());
        self.persist_config();
    }

    // ---- 修复画笔 / 污点修复 ----------------------------------------

    pub fn retouch_begin(
        mut self: Pin<&mut Self>,
        kind: &QString,
        x: f64,
        y: f64,
        size: f64,
        hardness: f64,
        spacing: f64,
        strength: f64,
    ) {
        let clone_from_source = kind.to_string() == "heal";
        let selected = *self.selected_layer();
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            let Some(target) = pick_target(&session.doc, selected) else {
                return;
            };
            if layer_locked(&session.doc, target) {
                return;
            }
            let Some(from) = target_image(&session.doc, target).cloned() else {
                return;
            };
            let origin = target_origin(&session.doc, target);
            let source = session.heal_source;
            session.brush_live = Some(BrushLive {
                clone_from_source,
                target,
                origin,
                from,
                points: vec![(x as f32, y as f32)],
                source,
                brush: editcore::BrushSpec {
                    size: size as f32,
                    hardness: hardness as f32,
                    spacing: spacing as f32,
                },
                strength: strength as f32,
                dirty: None,
            });
            brush_live_apply(session);
        }
        self.as_mut().refresh_preview();
    }

    pub fn retouch_move(mut self: Pin<&mut Self>, x: f64, y: f64) {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            let Some(st) = session.brush_live.as_mut() else {
                return;
            };
            let p = (x as f32, y as f32);
            if let Some(last) = st.points.last() {
                if (last.0 - p.0).abs() + (last.1 - p.1).abs() < 0.5 {
                    return;
                }
            }
            st.points.push(p);
            brush_live_apply(session);
        }
        self.as_mut().refresh_preview();
    }

    pub fn retouch_end(mut self: Pin<&mut Self>) {
        let changed;
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            if session.brush_live.is_none() {
                return;
            }
            brush_live_apply(session);
            let Some(st) = session.brush_live.take() else {
                return;
            };
            changed = commit_live(session, st.target, &st.from, st.dirty);
        }
        if changed {
            self.as_mut().refresh_preview();
        }
    }

    pub fn retouch_cancel(mut self: Pin<&mut Self>) {
        let mut changed = false;
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            if let Some(mut st) = session.brush_live.take() {
                if st.dirty.is_some() {
                    let mut dirty = st.dirty.take();
                    restore_dirty(session, st.target, &st.from, &mut dirty);
                    changed = true;
                }
            }
        }
        if changed {
            self.as_mut().refresh_preview();
        }
    }

    pub fn retouch_set_source(self: Pin<&mut Self>, x: f64, y: f64) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.heal_source = Some((x as f32, y as f32));
        }
        println!(
            "{{\"phase\":\"ui\",\"event\":\"heal-source\",\"x\":{},\"y\":{}}}",
            x.round() as i64,
            y.round() as i64
        );
    }

    // ---- 修补工具 ----------------------------------------------------

    pub fn patch_set_region(self: Pin<&mut Self>, points: &QString) {
        let pts: Vec<(f32, f32)> = points
            .to_string()
            .split_whitespace()
            .filter_map(|pair| {
                let (x, y) = pair.split_once(',')?;
                Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
            })
            .collect();
        let mask = editcore::SelectionMask::polygon(&pts);
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.patch_region = if mask.is_empty() { None } else { Some(mask) };
        }
        println!(
            "{{\"phase\":\"ui\",\"event\":\"patch-region\",\"points\":{}}}",
            pts.len()
        );
    }

    pub fn patch_contains(self: Pin<&mut Self>, x: f64, y: f64) -> bool {
        self.session
            .borrow()
            .as_ref()
            .and_then(|s| s.patch_region.as_ref())
            .map(|m| m.contains(x as f32, y as f32))
            .unwrap_or(false)
    }

    pub fn patch_drag_begin(self: Pin<&mut Self>, x: f64, y: f64, feather: i32) {
        let selected = *self.selected_layer();
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            if session.patch_region.is_none() {
                return;
            }
            let Some(target) = pick_target(&session.doc, selected) else {
                return;
            };
            if layer_locked(&session.doc, target) {
                return;
            }
            let Some(from) = target_image(&session.doc, target).cloned() else {
                return;
            };
            let origin = target_origin(&session.doc, target);
            session.patch_live = Some(PatchLive {
                target,
                origin,
                from,
                offset: (0, 0),
                feather: feather.clamp(0, 40) as u32,
                dirty: None,
            });
            let _ = (x, y);
        }
    }

    pub fn patch_drag_move(mut self: Pin<&mut Self>, dx: f64, dy: f64) {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            let Some(st) = session.patch_live.as_mut() else {
                return;
            };
            st.offset = (dx.round() as i32, dy.round() as i32);
            patch_live_apply(session);
        }
        self.as_mut().refresh_preview();
    }

    pub fn patch_drag_end(mut self: Pin<&mut Self>) -> bool {
        let changed;
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return false;
            };
            if session.patch_live.is_none() {
                return false;
            }
            let Some(st) = session.patch_live.take() else {
                return false;
            };
            changed = commit_live(session, st.target, &st.from, st.dirty);
            if changed {
                session.patch_region = None;
            }
        }
        if changed {
            self.as_mut().refresh_preview();
        }
        changed
    }

    pub fn patch_drag_cancel(mut self: Pin<&mut Self>) {
        let mut changed = false;
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            if let Some(mut st) = session.patch_live.take() {
                if st.dirty.is_some() {
                    let mut dirty = st.dirty.take();
                    restore_dirty(session, st.target, &st.from, &mut dirty);
                    changed = true;
                }
            }
        }
        if changed {
            self.as_mut().refresh_preview();
        }
    }

    pub fn patch_clear_region(self: Pin<&mut Self>) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.patch_region = None;
        }
    }

    // ---- 油漆桶 ------------------------------------------------------

    pub fn bucket_fill(
        mut self: Pin<&mut Self>,
        x: f64,
        y: f64,
        tolerance: i32,
        contiguous: bool,
        opacity: f64,
        color: &QString,
    ) {
        let selected = *self.selected_layer();
        let rgba = Self::parse_color(&color.to_string());
        let mut changed = false;
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            let Some(target) = pick_target(&session.doc, selected) else {
                return;
            };
            if layer_locked(&session.doc, target) {
                return;
            }
            let Some(img) = target_image(&session.doc, target).cloned() else {
                return;
            };
            let origin = target_origin(&session.doc, target);
            let selection = session.doc.selection.as_ref().map(|s| s.effective());
            let patch = editcore::flood_fill(
                &img,
                origin,
                (x as i32, y as i32),
                tolerance.clamp(0, 255) as u8,
                contiguous,
                selection.as_ref(),
                rgba,
                opacity as f32,
            );
            if let Some(p) = patch {
                if !p.is_empty() {
                    let (r, before) = editcore::crop_region(&img, p.rect);
                    if !r.is_empty() {
                        let after = p.image.clone();
                        session.doc.write_patch(target, &p);
                        changed = session.doc.commit_patch(target, r, before, after).is_ok();
                    }
                }
            }
        }
        if changed {
            self.as_mut().refresh_preview();
            println!("{{\"phase\":\"ui\",\"event\":\"bucket-fill\"}}");
        }
    }

    // ---- 渐变 --------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn gradient_begin(
        self: Pin<&mut Self>,
        x: f64,
        y: f64,
        radial: bool,
        reverse: bool,
        opacity: f64,
        c0: &QString,
        c1: &QString,
    ) {
        let selected = *self.selected_layer();
        let start_color = Self::parse_color(&c0.to_string());
        let end_color = Self::parse_color(&c1.to_string());
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            let Some(target) = pick_target(&session.doc, selected) else {
                return;
            };
            if layer_locked(&session.doc, target) {
                return;
            }
            let Some(from) = target_image(&session.doc, target).cloned() else {
                return;
            };
            let origin = target_origin(&session.doc, target);
            session.gradient_live = Some(GradientLive {
                target,
                origin,
                from,
                start: (x as f32, y as f32),
                end: (x as f32 + 1.0, y as f32),
                radial,
                reverse,
                opacity: opacity as f32,
                c0: start_color,
                c1: end_color,
                dirty: None,
            });
        }
    }

    pub fn gradient_move(mut self: Pin<&mut Self>, x: f64, y: f64) {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            let Some(st) = session.gradient_live.as_mut() else {
                return;
            };
            st.end = (x as f32, y as f32);
            gradient_live_apply(session);
        }
        self.as_mut().refresh_preview();
    }

    pub fn gradient_end(mut self: Pin<&mut Self>) {
        let changed;
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            if session.gradient_live.is_none() {
                return;
            }
            let Some(st) = session.gradient_live.take() else {
                return;
            };
            changed = commit_live(session, st.target, &st.from, st.dirty);
        }
        if changed {
            self.as_mut().refresh_preview();
        }
    }

    pub fn gradient_cancel(mut self: Pin<&mut Self>) {
        let mut changed = false;
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            if let Some(mut st) = session.gradient_live.take() {
                if st.dirty.is_some() {
                    let mut dirty = st.dirty.take();
                    restore_dirty(session, st.target, &st.from, &mut dirty);
                    changed = true;
                }
            }
        }
        if changed {
            self.as_mut().refresh_preview();
        }
    }

    // ---- 图层旋转 ----------------------------------------------------

    /// 开始一次旋转会话，返回会话的起始角（= 之前已经累计转过的角度）。
    ///
    /// `keep_session` = true（画布手柄）时，如果上一次旋转之后图层没被别的
    /// 操作改过，就从**没被重采样过的原图**继续转：来回拖多少次手柄，位图
    /// 都只被插值一次，画质不会越拖越糊。角度对话框用 false —— 它每次都是
    /// 从「图层当前的样子」出发的一次性操作。
    pub fn rotate_layer_begin(self: Pin<&mut Self>, index: i32, keep_session: bool) -> f64 {
        let mut guard = self.session.borrow_mut();
        let Some(session) = guard.as_mut() else {
            return 0.0;
        };
        let ix = index.max(0) as usize;
        let Some(layer) = session.doc.layers.get(ix) else {
            return 0.0;
        };
        if layer.locked {
            return 0.0;
        }
        session.rotate_token = session.rotate_token.wrapping_add(1);
        let token = session.rotate_token;
        // 统一会话：起手先整体清掉上一场手势（缩放 / 移动 / 没收尾的旋转），
        // 不再可能出现「旋转帧和缩放帧同时挂在 session 上抢合成器」。
        session.clear_live_gestures();
        if !keep_session {
            session.rotate_origin = None;
            let one = std::rc::Rc::new(session.doc.layers[ix].clone());
            session.transform = Some(TransformSession::Rotate {
                live: RotateLive {
                    index: ix,
                    before: one.clone(),
                    token,
                    start: one,
                    base_degrees: 0.0,
                    last_delta: 0.0,
                    expand: true,
                    keep: false,
                },
                frame: None,
                cap: None,
            });
            return 0.0;
        }
        // 锚点还新鲜才复用：同一个图层 + 真的转出过角度 + **像素内容**还是
        // 提交后的那一版。
        //
        // ★这里只比 image，不比整个 Layer：以前比整个结构，于是图层只要被
        //   挪动过 1px（offset 变了）锚点就废，base 归零，下一次旋转从已经
        //   放大过的位图再转一遍 —— 转几次 45°，包围盒就 150→213→301 一路
        //   长大。位移/透明度/名字跟"原图能不能接着转"无关，提交时都从
        //   `st.start`（当前状态）取，所以挪过也能接着转。
        let reusable = session
            .rotate_origin
            .as_ref()
            .map(|o| {
                o.index == ix
                    && o.degrees.abs() > 0.001
                    && session.doc.layers[ix].image == o.committed.image
            })
            .unwrap_or(false);
        let (before, start, base) = if reusable {
            let origin = session.rotate_origin.as_ref().expect("checked above");
            (
                origin.layer.clone(),
                std::rc::Rc::new(session.doc.layers[ix].clone()),
                origin.degrees,
            )
        } else {
            session.rotate_origin = None;
            let one = std::rc::Rc::new(session.doc.layers[ix].clone());
            (one.clone(), one, 0.0)
        };
        // 诊断：锚点有没有接上。base=0 且图层其实已经转过时，说明这次又从
        // 烘焙过的位图重算 —— 包围盒就会一路长大。
        println!(
            "{{\"phase\":\"ui\",\"event\":\"rotate-anchor\",\"reuse\":{},\"base\":{:.2}}}",
            reusable, base
        );
        session.transform = Some(TransformSession::Rotate {
            live: RotateLive {
                index: ix,
                before,
                token,
                start,
                base_degrees: base,
                last_delta: 0.0,
                expand: true,
                keep: true,
            },
            frame: None,
            cap: None,
        });
        base as f64
    }

    /// `degrees` 是相对本次会话起点的增量角（拖动手柄的位移量）。
    ///
    /// 拖动期间**不碰文档像素**：只把角度交给合成器，由它按预览尺寸临时旋
    /// 转（见 `composite_live_rotate_adjusted`）。整层重采样留到松手那一帧，
    /// 所以来回拖多少次，位图都只被插值一次 —— 不会越拖越糊。
    pub fn rotate_layer_update(mut self: Pin<&mut Self>, degrees: f64, expand: bool) {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            let Some(TransformSession::Rotate { live: st, frame, cap }) =
                session.transform.as_mut()
            else {
                return;
            };
            let total = st.base_degrees + degrees as f32;
            st.last_delta = degrees as f32;
            st.expand = expand;
            // 超大画布上顺手把预览压到 1600 长边（QML 会拉回窗口尺寸）。
            *cap = Some(LIVE_ROTATE_EDGE);
            let index = st.index;
            let snapshot = st.before.clone();
            // ★预览的摆放基准 = **当前图层的中心**，和 finish 里的提交算法一致。
            //
            // 合成器画的是「snapshot 按 angle 转完、落在 offset+delta」的位置，
            // 也就是以 snapshot.center 为转轴。但锚点 snapshot 不一定是图层现在
            // 的样子（连续旋转时它是没被插值过的原图，尺寸更小、offset 也更早），
            // 直接传 st.start.offset 会让「小图 + 大 offset」凑出一个错中心。
            // 所以这里把 offset 折算成「让 snapshot 中心落在当前图层中心」的值。
            let snap_w = snapshot.image.width() as f64;
            let snap_h = snapshot.image.height() as f64;
            let offset = (
                (st.start.offset.0 as f64 + st.start.image.width() as f64 / 2.0
                    - snap_w / 2.0)
                    .round() as i32,
                (st.start.offset.1 as f64 + st.start.image.height() as f64 / 2.0
                    - snap_h / 2.0)
                    .round() as i32,
            );
            let token = st.token;
            *frame = Some(LiveRotate {
                index,
                snapshot,
                offset,
                degrees: total,
                expand,
                token,
            });
        }
        self.as_mut().refresh_preview();
    }

    /// 结束会话：把拖动期间只存在于合成器里的角度真正落到位图上（一次重采样），
    /// 或者直接丢掉（取消）。
    pub fn rotate_layer_finish(mut self: Pin<&mut Self>, apply: bool, degrees: f64, expand: bool) {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            let Some(st) = session.take_rotate_live() else {
                // 迟到 / 重复的收尾（onReleased 之后又来一次 onCanceled，或
                // QML 补的清理调用）：会话已经结束了，但临时预览态可能因为
                // 更早漏发过 finish 而残留。清掉并按真实文档重画一帧。
                let stale = session.clear_live_gestures();
                if stale.0 || stale.1 || stale.2 || stale.3 {
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"live-stale\",\"where\":\"rotate-finish\",\
                         \"rotate_live\":{},\"live_rotate\":{},\"live_scale\":{},\"cap\":{}}}",
                        stale.0, stale.1, stale.2, stale.3
                    );
                }
                drop(guard);
                self.as_mut().refresh_preview();
                return;
            };
            // 会话已由 take_rotate_live() 整体结束：预览上限与临时旋转帧
            // 都是同一个枚举变体的字段，不会残留（文档像素本来就没动过）。
            let total = st.base_degrees + degrees as f32;
            // 这次拖拽的净效果为零（含「压根没动」）：文档没被改过，什么都不用做。
            let neutral = !apply || (total - st.base_degrees).abs() < 0.001;
            // 本次没产生净变化（含「压根没动」）：文档没被改过，什么都不用做。
            // ★取消也不要丢连续会话的锚点：文档像素和提交时完全一致，锚点
            //   依然有效，下一次 begin 的判据会照常命中。以前一取消就清掉，
            //   下一轮只能拿「已经烘焙成外接矩形的位图」当原图起手 —— 用户
            //   取消过一次之后再转，框就会平白大一圈。
            //   （对话框 / Esc 的取消同样落在这里。）
            if !neutral {
                let commit_t0 = std::time::Instant::now();
                // 双三次：整段拖拽只在这里真正重采样一次，用双线性转完会整层
                // 发软（用户反馈的"转一下就变模糊"）。
                let (image, _delta) = editcore::rotate_image_hq(&st.before.image, total, expand);
                // ★摆放必须按「当前图层的中心」算，不能用 `start.offset + delta`。
                //
                // `delta` 是相对 `before`（锚点原图）那张位图的位移，而 offset
                // 取的是 `start`（当前图层，可能是上一次旋转烘焙出的更大位图）。
                // 两者基准不同：80×80 第一次转 45° 变成 114×114，offset 减 17，
                // 中心不变；再转 90° 时 delta=0，`start.offset + 0` 就少了这 17
                // —— 松手瞬间图层往左上"跳"，而拖动中的预览（按中心转）是对的。
                // 以中心摆放则是几何上唯一自洽的：转轴就是中心，尺寸怎么变，
                // 中心都不动。
                let cx = st.start.offset.0 as f64 + st.start.image.width() as f64 / 2.0;
                let cy = st.start.offset.1 as f64 + st.start.image.height() as f64 / 2.0;
                let offset = (
                    (cx - image.width() as f64 / 2.0).round() as i32,
                    (cy - image.height() as f64 / 2.0).round() as i32,
                );
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"rotate-place\",\"start\":[{},{}],\"start_size\":[{}x{}],\
                     \"new_size\":[{}x{}],\"placed\":[{},{}]}}",
                    st.start.offset.0, st.start.offset.1,
                    st.start.image.width(), st.start.image.height(),
                    image.width(), image.height(), offset.0, offset.1
                );
                // 除像素之外的一切都取**当前**状态（st.start），而不是锚点那份：
                // 两次旋转之间用户可能挪过图层、改过透明度/名字，这些都得留着。
                let after = Layer {
                    name: st.start.name.clone(),
                    image,
                    offset,
                    opacity: st.start.opacity,
                    visible: st.start.visible,
                    locked: st.start.locked,
                    // 旋转结果是一次性重采样出来的，不保留旧的变换源图。
                    transform: None,
                };
                // 像素其实没变（拖一圈又回原位）：别再记一条空历史。
                if *st.start != after {
                    let _ = session
                        .doc
                        .commit_layer(st.index, (*st.start).clone(), after);
                }
                if st.keep {
                    // 锚点推进到最新角度：手柄下一次拖动还是从没被插值过的
                    // 原图出发，角度累加。committed 记下"这一版内容"，下一
                    // 次 begin 用它判断图层有没有被别人改过。
                    let committed = std::rc::Rc::new(session.doc.layers[st.index].clone());
                    session.rotate_origin = Some(RotateOrigin {
                        index: st.index,
                        layer: st.before,
                        degrees: total,
                        committed,
                    });
                } else {
                    session.rotate_origin = None;
                }
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"rotate-commit\",\"ms\":{},\"degrees\":{:.2}}}",
                    commit_t0.elapsed().as_millis(),
                    total
                );
            }
        }
        // 收尾保险：不管走哪条分支，临时预览态都不许留到下一次手势。
        // 上面只显式清了 live_cap / live_rotate，rotate_live 靠 take()，
        // live_scale 则可能来自上一次没收到 released 的缩放手势。
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                session.clear_live_gestures();
            }
        }
        // 不管提交还是取消，都要重画一帧：拖动中看到的那张是临时合成出来的。
        self.as_mut().refresh_preview();
    }

    pub fn rotate_layer_angle(mut self: Pin<&mut Self>, index: i32, degrees: f64, expand: bool) {
        let mut changed = false;
        if let Some(session) = self.session.borrow_mut().as_mut() {
            // 一次性旋转是直接改当前位图（会累加重采样），锚点必须作废。
            session.rotate_origin = None;
            changed = session
                .doc
                .rotate_layer_angle(index.max(0) as usize, degrees as f32, expand)
                .is_ok();
        }
        if changed {
            self.as_mut().refresh_preview();
            println!(
                "{{\"phase\":\"ui\",\"event\":\"rotate-layer\",\"degrees\":{:.2}}}",
                degrees
            );
        }
    }

    pub fn rotate_layer_state(self: Pin<&mut Self>) -> QString {
        let text = self
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.rotate_state())
            .map(|(degrees, expand)| format!("{:.2},{}", degrees, expand as i32))
            .unwrap_or_default();
        QString::from(text.as_str())
    }
}
