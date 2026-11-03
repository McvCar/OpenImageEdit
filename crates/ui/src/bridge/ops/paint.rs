// 修复 / 油漆 / 图层旋转。所有「拖动中」的工具共用一个套路：
//   begin  → 记下目标位图的原始副本 + 手势参数
//   move   → 先按原始副本还原上一次写过的矩形，再整段重算 + 写回
//   end    → 把矩形内的前后两份位图补成一条撤销记录
// 这样一次涂抹只会留下一条 Ctrl+Z，而不是每帧一条。
use super::super::*;

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

/// 选中图层优先，否则改背景（PS 语义）。
fn pick_target(doc: &Document, selected: i32) -> Option<AdjustTarget> {
    if selected >= 0 && (selected as usize) < doc.layers.len() {
        Some(AdjustTarget::Layer(selected as usize))
    } else if doc.canvas.width > 0 && doc.canvas.height > 0 {
        Some(AdjustTarget::Base)
    } else {
        None
    }
}

fn layer_locked(doc: &Document, target: AdjustTarget) -> bool {
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

    pub fn rotate_layer_begin(self: Pin<&mut Self>, index: i32) {
        let mut guard = self.session.borrow_mut();
        let Some(session) = guard.as_mut() else { return };
        let ix = index.max(0) as usize;
        let Some(layer) = session.doc.layers.get(ix) else {
            return;
        };
        if layer.locked {
            return;
        }
        let before = layer.clone();
        session.rotate_live = Some(RotateLive {
            index: ix,
            before: before.clone(),
            current: before,
            degrees: 0.0,
            expand: true,
        });
    }

    pub fn rotate_layer_update(mut self: Pin<&mut Self>, degrees: f64, expand: bool) {
        let update;
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            let Some(st) = session.rotate_live.as_mut() else {
                return;
            };
            let mut after = st.before.clone();
            let (image, delta) = editcore::rotate_image(&st.before.image, degrees as f32, expand);
            after.image = image;
            after.offset = (
                st.before.offset.0 + delta.0,
                st.before.offset.1 + delta.1,
            );
            st.current = after.clone();
            st.degrees = degrees as f32;
            st.expand = expand;
            update = (st.index, after);
        }
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                let _ = session.doc.preview_layer(update.0, update.1);
            }
        }
        self.as_mut().refresh_preview();
    }

    pub fn rotate_layer_finish(mut self: Pin<&mut Self>, apply: bool) {
        let changed;
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else { return };
            let Some(st) = session.rotate_live.take() else {
                return;
            };
            if apply {
                changed = session
                    .doc
                    .commit_layer(st.index, st.before.clone(), st.current.clone())
                    .is_ok();
            } else {
                let _ = session.doc.preview_layer(st.index, st.before);
                changed = true;
            }
        }
        if changed {
            self.as_mut().refresh_preview();
        }
    }

    pub fn rotate_layer_angle(mut self: Pin<&mut Self>, index: i32, degrees: f64, expand: bool) {
        let mut changed = false;
        if let Some(session) = self.session.borrow_mut().as_mut() {
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
            .and_then(|s| s.rotate_live.as_ref())
            .map(|s| format!("{:.2},{}", s.degrees, s.expand as i32))
            .unwrap_or_default();
        QString::from(text.as_str())
    }
}
