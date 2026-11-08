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
        // ★P0-2b2：累计角度与原图的唯一真值是 `Layer::transform`（params），
        // 不再是跨会话的 `rotate_origin` 锚点缓存。撤销 / 重做 / 缩放之后
        // 它自动同步，不会再出现「锚点判据失效 → 从烘焙过的外接矩形再转
        // 一遍 → 包围盒一路长大」。
        let current = &session.doc.layers[ix];
        let start = std::rc::Rc::new(current.clone());
        let base = current
            .transform
            .as_ref()
            .map(|t| t.rotation as f32)
            .unwrap_or(0.0);
        // ★精确中心：有 params 时它就是唯一真值；普通位图图层才用
        // `offset + 尺寸/2`（此时两者本来就相等，且是整/半像素，不会被量化）。
        let start_center = match current.transform.as_ref() {
            Some(t) => t.center,
            None => (
                current.offset.0 as f64 + current.image.width() as f64 / 2.0,
                current.offset.1 as f64 + current.image.height() as f64 / 2.0,
            ),
        };
        let before = match current.transform.as_ref() {
            Some(t) => {
                let mut pristine = (*start).clone();
                // ★预览的快照必须是「**当前尺寸**的未插值图」，不是裸原图：
                //   拉伸过（scale ≠ 1）的图层如果拿 source 起手，拖动期间
                //   合成器画出来的是原尺寸，框却还是拉伸后的外接框 —— 又是
                //   「框和内容不一致」。这里先把 scale/flip 烘焙到预览快照
                //   （一次 Lanczos），旋转本身仍然交给合成器实时转。
                let no_scale = (t.scale.0 - 1.0).abs() < 1e-9
                    && (t.scale.1 - 1.0).abs() < 1e-9;
                if no_scale {
                    pristine.image = (*t.source).clone();
                } else {
                    let mut flat = t.clone();
                    flat.rotation = 0.0;
                    let (image, _) = flat.render_compatible();
                    pristine.image = image;
                }
                std::rc::Rc::new(pristine)
            }
            None => start.clone(),
        };
        println!(
            "{{\"phase\":\"ui\",\"event\":\"rotate-anchor\",\"reuse\":{},\"base\":{:.2}}}",
            current.transform.is_some(),
            base
        );
        let keep = keep_session;
        session.transform = Some(TransformSession::Rotate {
            live: RotateLive {
                index: ix,
                before,
                token,
                start,
                start_center,
                base_degrees: base,
                last_delta: 0.0,
                expand: true,
                keep,
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
    /// P1-A：拖动帧角度可带修饰键 —— Shift 把**累计角度**吸附到 15° 整数倍
    /// （PS 语义；增量角按 `snap(base+delta)-base` 反推）。QML 只透传掩码。
    pub fn rotate_layer_update(
        mut self: Pin<&mut Self>,
        degrees: f64,
        expand: bool,
        mods: i32,
    ) {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            let Some(TransformSession::Rotate { live: st, frame, cap }) =
                session.transform.as_mut()
            else {
                return;
            };
            let degrees = editcore::snap_angle(
                st.base_degrees as f64 + degrees,
                mods & super::transform::MOD_SHIFT != 0,
                15.0,
                if mods & super::transform::MOD_MAGNET != 0 {
                    super::transform::MAGNET_TOL
                } else {
                    0.0
                },
            ) - st.base_degrees as f64;
            let total = st.base_degrees + degrees as f32;
            st.last_delta = degrees as f32;
            st.expand = expand;
            // 超大画布上顺手把预览压到 1600 长边（QML 会拉回窗口尺寸）。
            *cap = Some(LIVE_ROTATE_EDGE);
            let index = st.index;
            let snapshot = st.before.clone();
            // ★拖动帧的内容中心 = **落定量化之后**的那个中心，一个常量。
            //
            // 落定的摆放是 `round_center(center - W/2)`（图层 offset 只能整数）：
            //  · `W` 为偶数（扩展包围盒，现在的常态）→ `W/2` 整数，内容中心恒为
            //    `round_center(center)`，**与角度无关**；
            //  · `W` 为奇数（勾了「不扩展」，W 就是源尺寸）→ 恒为
            //    `round_center(center - W/2) + W/2`，同样与角度无关。
            // 两种都是常量，所以拖动帧直接算一次即可，绝不每帧依赖当前 AABB。
            //
            // ★不能直接用 `start_center` 原值：真值可能是半像素（用户日志
            //   center.y=175.5，由奇数尺寸拉伸产生），而落定只能落在整像素上
            //   （1px 栅格），拖动帧若保留 .5 就恒定差半个像素 —— 屏幕上看就是
            //   「松手后图层挪了一下」（zoom 195% 时约 1px）。
            let (scx, scy) = st.start_center;
            let center = if expand {
                (
                    editcore::round_center(scx),
                    editcore::round_center(scy),
                )
            } else {
                let (sw, sh) = snapshot.image.dimensions();
                (
                    editcore::round_center(scx - sw as f64 / 2.0) + sw as f64 / 2.0,
                    editcore::round_center(scy - sh as f64 / 2.0) + sh as f64 / 2.0,
                )
            };
            let token = st.token;
            *frame = Some(LiveRotate {
                index,
                snapshot,
                center,
                degrees: total,
                expand,
                token,
            });
        }
        self.as_mut().refresh_preview();
    }

    /// 结束会话：把拖动期间只存在于合成器里的角度真正落到位图上（一次重采样），
    /// 或者直接丢掉（取消）。
    /// P1-A：`mods` 带 Shift 时，落定角度同样吸附 15°（与拖动帧一致）。
    pub fn rotate_layer_finish(
        mut self: Pin<&mut Self>,
        apply: bool,
        degrees: f64,
        expand: bool,
        mods: i32,
    ) {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            // 落定吸角要按累计角算，但 base 在 take 之后才拿得到：先偷看。
            let base = match session.transform.as_ref() {
                Some(TransformSession::Rotate { live, .. }) => live.base_degrees as f64,
                _ => 0.0,
            };
            let degrees = editcore::snap_angle(
                base + degrees,
                mods & super::transform::MOD_SHIFT != 0,
                15.0,
                if mods & super::transform::MOD_MAGNET != 0 {
                    super::transform::MAGNET_TOL
                } else {
                    0.0
                },
            ) - base;
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
                // ★摆放按「起手时的精确中心」算：转轴就是中心，尺寸怎么变中心
                // 都不动（旧的 `start.offset + 位图尺寸/2` 在每轮旋转后都会被
                // 重新量化一次，转两三次就漂 1px）。
                let (cx, cy) = st.start_center;
                let (start_offset, start_size) = (
                    st.start.offset,
                    (st.start.image.width(), st.start.image.height()),
                );
                // ★P0-2b2：旋转落定统一成一条 `SetLayerTransform`。
                // `source` 恒为未插值原图、rotation 是累计角度，所以：
                // ① 连续旋转不再累积模糊；② undo 精确回退到上一个角度；
                // ③ 与缩放 / 移动共用同一条参数级历史，锚点缓存彻底退场。
                //
                // 「不扩展」例外：裁剪到原尺寸这件事仿射表达不了，只能把本次
                // 旋转烘焙进位图（该选项的固有代价，用户显式勾选才走）。
                if expand {
                    let layer = &session.doc.layers[st.index];
                    let mut t = layer.transform.clone().unwrap_or_else(|| {
                        LayerTransform::from_offset(
                            layer.image.clone(),
                            layer.offset.0,
                            layer.offset.1,
                        )
                    });
                    t.rotation = total as f64;
                    t.center = (cx, cy);
                    if session.doc.set_layer_transform(st.index, Some(t)).is_ok() {
                        let placed = session.doc.layers[st.index].offset;
                        let new_size = session.doc.layers[st.index].image.dimensions();
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"rotate-place\",\"start\":[{},{}],\"start_size\":[{}x{}],\
                             \"new_size\":[{}x{}],\"placed\":[{},{}],\"center\":[{:.3},{:.3}]}}",
                            start_offset.0, start_offset.1, start_size.0, start_size.1,
                            new_size.0, new_size.1, placed.0, placed.1, cx, cy
                        );
                    }
                } else {
                    let (image, _delta) = editcore::rotate_image_hq(&st.before.image, total, false);
                    // 「不扩展」也要用同一个中心量化（见 core::round_center）。
                    let offset = (
                        editcore::round_center(cx - image.width() as f64 / 2.0) as i32,
                        editcore::round_center(cy - image.height() as f64 / 2.0) as i32,
                    );
                    let after = Layer {
                        name: st.start.name.clone(),
                        image,
                        offset,
                        opacity: st.start.opacity,
                        visible: st.start.visible,
                        locked: st.start.locked,
                        transform: None,
                    };
                    if *st.start != after {
                        let _ = session
                            .doc
                            .commit_layer(st.index, (*st.start).clone(), after);
                    }
                }
                // 旧 rotate_origin 锚点不再参与任何读取（begin 只读 params），
                // 显式清空避免它继续占着一份整层 Rc<Layer>。
                session.rotate_origin = None;
                let _ = (st.keep, &st.before);
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
            // 锚点缓存已退场（begin 只读 params），这里清一下纯属保险。
            session.rotate_origin = None;
            let ix = index.max(0) as usize;
            // ★P0-2b2：默认（扩展包围盒）走参数化旋转 —— `source` 恒为未插值
            // 原图，连续改角度不再累积重采样。只有显式「不扩展」还需要把
            // 裁剪烘焙进位图（仿射表达不了裁剪），走旧的破坏性路径。
            let params_ok = if expand {
                let next = session.doc.layers.get(ix).map(|l| {
                    let mut t = l.transform.clone().unwrap_or_else(|| {
                        LayerTransform::from_offset(l.image.clone(), l.offset.0, l.offset.1)
                    });
                    t.rotation += degrees;
                    // ★中心**不动**：转轴就是中心，角度变化不改变中心。
                    // 以前这里按 `offset + 位图尺寸/2` 重算一遍，等于把 params
                    // 里的亚像素中心量化成整像素 —— 反复改角度就一路往一个
                    // 方向漂（用户 11-08 报的「转两三次偏 1px」）。
                    t
                });
                match next {
                    Some(t) => session.doc.set_layer_transform(ix, Some(t)).is_ok(),
                    None => false,
                }
            } else {
                false
            };
            changed = params_ok
                || session
                    .doc
                    .rotate_layer_angle(ix, degrees as f32, expand)
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
