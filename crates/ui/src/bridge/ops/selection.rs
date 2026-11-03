// 选区（PS 式）命令。几何真值在 core::SelectionState，这里只做
// 「QML 坐标 → 画布坐标 → 更新状态 → 发布」的搬运，外加视图刷新。
//
// 选区存在 `Document` 里（不是 session），所以每次改动都进文档历史 ——
// Ctrl+Z 撤掉选区、Ctrl+Y 再拿回来，和像素编辑走同一条时间线。
use super::super::*;

/// Two opposite corners (possibly dragged right-to-left / bottom-to-top) →
/// a normalized integer rect in image pixels.
fn rect_from_corners(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
    let xa = x0.min(x1).floor() as i32;
    let ya = y0.min(y1).floor() as i32;
    let xb = x0.max(x1).ceil() as i32;
    let yb = y0.max(y1).ceil() as i32;
    Rect::new(xa, ya, (xb - xa).max(0) as u32, (yb - ya).max(0) as u32)
}

impl qobject::Bridge {
    /// Publish the selection's public state (bounds / ops / active flag) and
    /// mirror the session's revision into `sel_version` so QML re-pulls the
    /// outline when — and only when — the selection really changed.
    ///
    /// Returns true when the quick mask was switched on/off: the mask is drawn
    /// INTO the preview bitmap, so both directions need a fresh frame — and by
    /// the time the caller asks, `quick_mask` already holds the NEW value, so
    /// "is it on now" cannot answer for "just turned off".
    pub(crate) fn publish_selection(mut self: Pin<&mut Self>) -> bool {
        let (active, rect, ops, rev, quick, can_undo, can_redo) = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => {
                    let (active, rect, ops) = match session.doc.selection.as_ref() {
                        Some(st) if !st.is_empty() => (
                            true,
                            st.mask().bounds,
                            (
                                st.ops.border as i32,
                                st.ops.smooth as i32,
                                st.ops.feather as i32,
                            ),
                        ),
                        _ => (false, Rect::default(), (0, 0, 0)),
                    };
                    (
                        active,
                        rect,
                        ops,
                        session.sel_rev as i32,
                        session.quick_mask,
                        session.doc.can_undo(),
                        session.doc.can_redo(),
                    )
                }
                None => (false, Rect::default(), (0, 0, 0), 0, false, false, false),
            }
        };
        let mut mask_changed = false;
        // 选区现在是文档历史的一部分：只动选区也会让"撤销"变成可用。
        // 之前这几步不刷新 preview，工具栏的撤销图标就会一直是灰的。
        if *self.can_undo() != can_undo {
            self.as_mut().set_can_undo(can_undo);
        }
        if *self.can_redo() != can_redo {
            self.as_mut().set_can_redo(can_redo);
        }
        if *self.sel_active() != active {
            self.as_mut().set_sel_active(active);
        }
        if *self.sel_x() != rect.x {
            self.as_mut().set_sel_x(rect.x);
        }
        if *self.sel_y() != rect.y {
            self.as_mut().set_sel_y(rect.y);
        }
        if *self.sel_w() != rect.width as i32 {
            self.as_mut().set_sel_w(rect.width as i32);
        }
        if *self.sel_h() != rect.height as i32 {
            self.as_mut().set_sel_h(rect.height as i32);
        }
        if *self.sel_border() != ops.0 {
            self.as_mut().set_sel_border(ops.0);
        }
        if *self.sel_smooth() != ops.1 {
            self.as_mut().set_sel_smooth(ops.1);
        }
        if *self.sel_feather() != ops.2 {
            self.as_mut().set_sel_feather(ops.2);
        }
        if *self.quick_mask() != quick {
            self.as_mut().set_quick_mask(quick);
            mask_changed = true;
        }
        if *self.sel_version() != rev {
            self.as_mut().set_sel_version(rev);
        }
        mask_changed
    }

    /// Bump the session's selection revision (call after every mutation).
    pub(crate) fn touch_selection(&self) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.sel_rev = session.sel_rev.wrapping_add(1);
        }
    }

    /// 撤销 / 重做之后必须把选区重新发布一遍：`SetSelection` 会把文档里的
    /// 选区整体换掉（或者换回来），QML 的蚂蚁线、尺寸徽标、手柄全靠这次
    /// 发布刷新。放在这里是因为 undo/redo 在 canvas.rs。
    pub(crate) fn sync_selection_after_history(mut self: Pin<&mut Self>) {
        self.touch_selection();
        self.as_mut().commit_selection();
    }

    /// 选区改完之后的统一出口：发布公开状态 + 必要时重出预览帧。
    pub(crate) fn commit_selection(mut self: Pin<&mut Self>) {
        let mask_changed = self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection(mask_changed);
    }

    /// The quick mask is drawn INTO the preview bitmap, so every selection
    /// change needs a re-render while it is on (and none while it is off).
    /// `mask_changed` covers the on→off case: 取消选区会自动关掉快速蒙版，
    /// 这时 `quick_mask` 已经是 false，只看它就会漏掉那一次重绘，蒙版的
    /// 暗化层会一直留在屏幕上（必须重出一帧未蒙版的位图才能擦掉）。
    fn refresh_for_selection(mut self: Pin<&mut Self>, mask_changed: bool) {
        if (mask_changed || *self.quick_mask()) && self.session.borrow().is_some() {
            self.as_mut().refresh_preview();
        }
    }

    /// 改选区参数（边界 / 平滑 / 羽化）。`SelectionOptions` 会被文档历史
    /// 自动合并，所以数字框连点十几下也只算一步。
    fn mutate_selection_ops(self: Pin<&mut Self>, f: impl FnOnce(&mut SelOps)) -> bool {
        let mut guard = self.session.borrow_mut();
        let Some(session) = guard.as_mut() else {
            return false;
        };
        let Some(mut ops) = session.doc.selection.as_ref().map(|st| st.ops) else {
            return false;
        };
        f(&mut ops);
        session.doc.set_selection_ops(ops).is_ok()
    }

    /// 把当前选区的参数记成「下次新建选区的默认值」并写盘。用户改一次
    /// 平滑填 20，之后每画一个新选区都直接是 20，不用反复重填。
    fn remember_selection_ops(self: Pin<&mut Self>) {
        let ops = self
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.doc.selection.as_ref().map(|st| st.ops));
        let Some(ops) = ops else {
            return;
        };
        if self.sel_ops_default.get() == ops {
            return;
        }
        self.sel_ops_default.set(ops);
        self.persist_config();
    }

    /// Replace / add / subtract / intersect the current selection with `mask`.
    /// mode: 0 replace, 1 add, 2 subtract, 3 intersect.
    fn combine_selection(mut self: Pin<&mut Self>, mask: SelectionMask, mode: i32) {
        let canvas = match self.session.borrow().as_ref().map(|s| s.doc.canvas) {
            Some(c) => c,
            None => return,
        };
        let mask = mask.clipped(canvas);
        // 单击（没拖出面积）不该把已有选区清掉。
        if mask.is_empty() && mode == 0 {
            return;
        }
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let defaults = self.sel_ops_default.get();
            let current = session.doc.selection.as_ref().map(|s| s.mask().clone());
            let current_ops = session
                .doc
                .selection
                .as_ref()
                .map(|s| s.ops)
                .unwrap_or(defaults);
            let combined = match current {
                _ if mode == 0 => mask,
                Some(c) if mode == 1 => c.union(&mask),
                Some(c) if mode == 2 => c.subtract(&mask),
                Some(c) if mode == 3 => c.intersect(&mask),
                // 加/减/交但当前没有选区：行为同替换（与 PS 一致）。
                _ => mask,
            };
            let next = if combined.is_empty() {
                None
            } else {
                // mode 0 = 用户画了一个新形状 → 套用记住的边界/平滑/羽化；
                // 加/减/交 是从**当前可见选区**派生的（边界/平滑已烘焙进那
                // 张 mask），再套一遍会在结果边缘长出多余环带，所以只继承
                // 羽化（见 SelectionState::derived）。
                let state = if mode == 0 {
                    SelectionState::new_with_ops(combined, defaults)
                } else {
                    SelectionState::derived(combined, &current_ops)
                };
                Some(state)
            };
            let _ = session.doc.set_selection(next);
        }
        self.touch_selection();
        self.as_mut().commit_selection();
    }

    pub fn sel_set_rect(
        mut self: Pin<&mut Self>,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        mode: i32,
    ) {
        let rect = rect_from_corners(x0, y0, x1, y1);
        let mask = SelectionMask::rect(rect);
        self.as_mut().combine_selection(mask, mode);
        println!(
            "{{\"phase\":\"ui\",\"event\":\"sel-rect\",\"rect\":\"{},{},{},{}\",\"mode\":{mode}}}",
            rect.x, rect.y, rect.width, rect.height
        );
    }

    pub fn sel_set_ellipse(
        mut self: Pin<&mut Self>,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        mode: i32,
    ) {
        let rect = rect_from_corners(x0, y0, x1, y1);
        let mask = SelectionMask::ellipse(rect);
        self.as_mut().combine_selection(mask, mode);
        println!(
            "{{\"phase\":\"ui\",\"event\":\"sel-ellipse\",\"rect\":\"{},{},{},{}\",\"mode\":{mode}}}",
            rect.x, rect.y, rect.width, rect.height
        );
    }

    pub fn sel_set_polygon(mut self: Pin<&mut Self>, points: &QString, mode: i32) {
        let text = points.to_string();
        let pts: Vec<(f32, f32)> = text
            .split_whitespace()
            .filter_map(|pair| {
                let mut it = pair.split(',');
                let x: f32 = it.next()?.trim().parse().ok()?;
                let y: f32 = it.next()?.trim().parse().ok()?;
                Some((x, y))
            })
            .collect();
        if pts.len() < 3 {
            return;
        }
        let mask = SelectionMask::polygon(&pts);
        println!(
            "{{\"phase\":\"ui\",\"event\":\"sel-polygon\",\"points\":{},\"mode\":{mode}}}",
            pts.len()
        );
        self.as_mut().combine_selection(mask, mode);
    }

    pub fn sel_select_all(mut self: Pin<&mut Self>) {
        let canvas = match self.session.borrow().as_ref().map(|s| s.doc.canvas) {
            Some(c) => c,
            None => return,
        };
        let mask = SelectionMask::rect(Rect::new(0, 0, canvas.width, canvas.height));
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            // 全选就是整块画布，**不**套用「记住的边界/平滑」（那会把全选
            // 变成画布边缘的一圈环带，随后的反选/删除就全错了）。
            let _ = session.doc.set_selection(Some(SelectionState::new(mask)));
        }
        self.touch_selection();
        self.as_mut().commit_selection();
    }

    pub fn sel_clear(mut self: Pin<&mut Self>) {
        let changed = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                Some(session) => {
                    let had = session.doc.selection.is_some();
                    session.clear_selection();
                    had
                }
                None => false,
            }
        };
        if !changed {
            return;
        }
        self.touch_selection();
        self.as_mut().commit_selection();
        println!("{{\"phase\":\"ui\",\"event\":\"sel-clear\"}}");
    }

    pub fn sel_invert(mut self: Pin<&mut Self>) {
        let canvas = match self.session.borrow().as_ref().map(|s| s.doc.canvas) {
            Some(c) => c,
            None => return,
        };
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let inverted = match session.doc.selection.as_ref() {
                // 反选针对"看得见的选区"（含已烘焙的边界/平滑），见
                // SelectionState::inverted 的注释。
                Some(st) => st.inverted(canvas),
                None => SelectionState::new(SelectionMask::rect(Rect::new(
                    0,
                    0,
                    canvas.width,
                    canvas.height,
                ))),
            };
            let _ = session.doc.set_selection(Some(inverted));
        }
        self.touch_selection();
        self.as_mut().commit_selection();
        println!("{{\"phase\":\"ui\",\"event\":\"sel-invert\"}}");
    }

    pub fn sel_set_border(mut self: Pin<&mut Self>, k: i32) {
        let k = k.clamp(0, 400) as u32;
        if !self.as_mut().mutate_selection_ops(|ops| ops.border = k) {
            return;
        }
        self.as_mut().remember_selection_ops();
        self.touch_selection();
        self.as_mut().commit_selection();
    }

    pub fn sel_set_smooth(mut self: Pin<&mut Self>, k: i32) {
        let k = k.clamp(0, 400) as u32;
        if !self.as_mut().mutate_selection_ops(|ops| ops.smooth = k) {
            return;
        }
        self.as_mut().remember_selection_ops();
        self.touch_selection();
        self.as_mut().commit_selection();
    }

    pub fn sel_set_feather(mut self: Pin<&mut Self>, r: i32) {
        let r = r.clamp(0, 250) as u32;
        if !self.as_mut().mutate_selection_ops(|ops| ops.feather = r) {
            return;
        }
        self.as_mut().remember_selection_ops();
        self.touch_selection();
        self.as_mut().commit_selection();
    }

    /// 记住用户选的选区工具（下次启动继续用同一个）。
    pub fn save_sel_tool(mut self: Pin<&mut Self>, tool: &QString) {
        let name = tool.to_string();
        let valid = matches!(
            name.as_str(),
            "rect" | "ellipse" | "lasso" | "polygon" | "wand" | "quick"
        );
        if !valid || self.sel_tool_saved().to_string() == name {
            return;
        }
        self.as_mut().set_sel_tool_saved(tool.clone());
        self.persist_config();
    }

    /// 记住魔术棒参数（容差 / 连续 / 取样全部图层）。
    pub fn save_sel_wand(mut self: Pin<&mut Self>, spec: &QString) {
        let Some(normalized) = super::super::config::normalize_wand(&spec.to_string()) else {
            return;
        };
        if self.sel_wand_saved().to_string() == normalized {
            return;
        }
        self.as_mut().set_sel_wand_saved(QString::from(normalized.as_str()));
        self.persist_config();
    }

    /// 魔术棒：按点击处的颜色选出一片区域。
    ///
    /// 采样源：`sample_all` = 可见合成（PS 的「取样全部图层」）；否则取当前
    /// 选中的图层，没有选中图层时取背景。
    pub fn sel_magic_wand(
        mut self: Pin<&mut Self>,
        x: f64,
        y: f64,
        tolerance: i32,
        contiguous: bool,
        sample_all: bool,
        mode: i32,
    ) {
        let target = if sample_all {
            None
        } else {
            match *self.selected_layer() {
                i if i >= 0 => Some(AdjustTarget::Layer(i as usize)),
                _ => Some(AdjustTarget::Base),
            }
        };
        let opts = WandOptions {
            tolerance: tolerance.clamp(0, 255) as u8,
            contiguous,
        };
        let seed = (x.floor() as i32, y.floor() as i32);
        let mask = {
            let guard = self.session.borrow();
            let Some(session) = guard.as_ref() else {
                return;
            };
            session.doc.magic_wand_selection(seed.0, seed.1, opts, target)
        };
        let area = mask.area();
        self.as_mut().combine_selection(mask, mode);
        println!(
            "{{\"phase\":\"ui\",\"event\":\"sel-wand\",\"seed\":\"{},{}\",\"tolerance\":{},\"contiguous\":{},\"sample_all\":{},\"area\":{},\"mode\":{}}}",
            seed.0,
            seed.1,
            opts.tolerance,
            contiguous,
            sample_all,
            area,
            mode
        );
    }

    /// 快速选择：把「上一笔迹点 → (x, y)」这一段当种子扩散一次，并并入本次
    /// 涂抹的累积选区。
    ///
    /// 拖动期间只走 `preview_selection`（不进历史），每帧的工作量只有
    /// 「一个胶囊体 + grow 窗口」，大图上也是毫秒级。松手时由
    /// `quick_select_commit` 把整段涂抹补成一条撤销记录。
    pub fn quick_select_brush(
        mut self: Pin<&mut Self>,
        x: f64,
        y: f64,
        radius: f64,
        tolerance: i32,
        grow: i32,
        mode: i32,
    ) {
        let p = (x as f32, y as f32);
        let r = radius.clamp(1.0, 400.0) as u32;

        // 第一次调用：快照按下前的选区（撤销目标 + 加/减基准）。
        if self.quick_drag.borrow().is_none() {
            let (from, base) = {
                let guard = self.session.borrow();
                let Some(session) = guard.as_ref() else {
                    return;
                };
                let from = session.doc.selection.clone();
                let base = from.as_ref().map(|st| st.mask().clone());
                (from, base)
            };
            *self.quick_drag.borrow_mut() = Some(QuickDrag {
                from,
                base,
                accum: None,
                last: p,
                mode,
            });
        }

        // 本段 = 上次的点 → 当前点。
        let (seg, mode, base) = {
            let mut drag = self.quick_drag.borrow_mut();
            let Some(d) = drag.as_mut() else {
                return;
            };
            let seg = [d.last, p];
            d.last = p;
            (seg, d.mode, d.base.clone())
        };

        let opts = QuickSelectOptions {
            tolerance: tolerance.clamp(0, 255) as u8,
            // 边缘保护跟着容差走：容差开得大，允许的局部跳变也要跟着放宽，
            // 否则大容差会被一个细噪点边缘挡住，反而选不动。
            edge: (tolerance * 2).clamp(20, 120) as u8,
            grow: grow.clamp(0, 2000) as u32,
        };
        let spread = {
            let guard = self.session.borrow();
            let Some(session) = guard.as_ref() else {
                return;
            };
            // None = 取样可见合成（PS 快速选择默认吃「所有图层」）。
            session.doc.quick_select_selection(&seg, r, opts, None)
        };

        // 累积扩散结果，再按 mode 与按下前的选区合成。
        let (result, from_ops) = {
            let mut drag = self.quick_drag.borrow_mut();
            let Some(d) = drag.as_mut() else {
                return;
            };
            let accum = match d.accum.take() {
                Some(a) => a.union(&spread),
                None => spread,
            };
            d.accum = Some(accum.clone());
            let from_ops = d.from.as_ref().map(|st| st.ops);
            let result = match mode {
                1 => match &base {
                    Some(b) => b.union(&accum),
                    None => accum.clone(),
                },
                2 => match &base {
                    Some(b) => b.subtract(&accum),
                    None => SelectionMask::empty(),
                },
                _ => accum,
            };
            (result, from_ops)
        };

        let defaults = self.sel_ops_default.get();
        let state = if result.is_empty() {
            None
        } else if mode == 0 {
            Some(SelectionState::new_with_ops(result, defaults))
        } else {
            // 加/减是从当前可见选区派生的：边界/平滑已经烘焙进那张 mask，
            // 再套一遍会在结果边缘长出多余环带（同 combine_selection）。
            Some(SelectionState::derived(
                result,
                &from_ops.unwrap_or(defaults),
            ))
        };
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            session.doc.preview_selection(state);
        }
        self.as_mut().commit_selection();
    }

    /// 松开鼠标：把整段涂抹提交为一条撤销记录。
    pub fn quick_select_commit(mut self: Pin<&mut Self>) {
        let Some(drag) = self.quick_drag.borrow_mut().take() else {
            return;
        };
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            session.doc.commit_selection_change(drag.from);
        }
        self.as_mut().commit_selection();
    }

    /// 放弃本次涂抹（Esc）：回到按下前的选区，不留历史。
    pub fn quick_select_cancel(mut self: Pin<&mut Self>) {
        let Some(drag) = self.quick_drag.borrow_mut().take() else {
            return;
        };
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            session.doc.preview_selection(drag.from);
        }
        self.as_mut().commit_selection();
    }

    pub fn sel_outline(self: Pin<&mut Self>) -> QString {
        let json = self
            .session
            .borrow()
            .as_ref()
            .map(|s| s.selection_outline_json())
            .unwrap_or_else(|| "[]".to_string());
        QString::from(json.as_str())
    }

    pub fn sel_contains(self: Pin<&mut Self>, x: i32, y: i32) -> bool {
        self.session
            .borrow()
            .as_ref()
            .and_then(|s| s.doc.selection.as_ref())
            .map(|st| st.mask().contains(x as f32 + 0.5, y as f32 + 0.5))
            .unwrap_or(false)
    }

    pub fn sel_translate(mut self: Pin<&mut Self>, dx: i32, dy: i32) {
        let canvas = match self.session.borrow().as_ref().map(|s| s.doc.canvas) {
            Some(c) => c,
            None => return,
        };
        if dx == 0 && dy == 0 {
            return;
        }
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(mut st) = session.doc.selection.clone() else {
                return;
            };
            st.translate(dx, dy, canvas);
            let next = if st.is_empty() { None } else { Some(st) };
            let _ = session.doc.set_selection(next);
        }
        self.touch_selection();
        self.as_mut().commit_selection();
    }

    /// 拖手柄改选区大小（图像坐标，浮点）。Rust 侧按新包围盒重采样
    /// mask，QML 拖动期间只画本地预览框 —— 每帧重采样大选区会卡。
    /// 按手柄拖拽的新框重采样选区。`w` / `h` 允许为负：表示手柄拖过了
    /// 对边，此时选区在该轴做镜像（PS 的行为），`x` / `y` 仍是被拖动那条
    /// 边带动后的锚点，负值代表框向左/向上生长。
    pub fn sel_resize(mut self: Pin<&mut Self>, x: f64, y: f64, w: f64, h: f64) {
        let canvas = match self.session.borrow().as_ref().map(|s| s.doc.canvas) {
            Some(c) => c,
            None => return,
        };
        let flip_x = w < 0.0;
        let flip_y = h < 0.0;
        let w_abs = w.abs();
        let h_abs = h.abs();
        // 拖到厚度为 0 时先忽略（避免产生 0 宽选区）；再往外拖就开始镜像。
        if w_abs < 1.0 || h_abs < 1.0 {
            return;
        }
        let bounds = Rect::new(
            (if flip_x { x + w } else { x }).round() as i32,
            (if flip_y { y + h } else { y }).round() as i32,
            w_abs.round().max(1.0) as u32,
            h_abs.round().max(1.0) as u32,
        );
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(mut st) = session.doc.selection.clone() else {
                return;
            };
            // 只在跨过对边的那一帧镜像，之后继续往外拖就只是放大。
            let sign = (flip_x, flip_y);
            if session.sel_flip_sign.0 != sign.0 {
                st.flip_horizontal();
            }
            if session.sel_flip_sign.1 != sign.1 {
                st.flip_vertical();
            }
            session.sel_flip_sign = sign;
            st.resize_to(bounds, canvas);
            // 拖到没有面积了 = 取消选区（PS 里手柄拖过对边也是这个结果）。
            let next = if st.is_empty() { None } else { Some(st) };
            let _ = session.doc.set_selection(next);
        }
        self.touch_selection();
        self.as_mut().commit_selection();
    }

    /// 按下选区手柄：把「已镜像」状态清零，让下一次越过对边重新触发镜像。
    pub fn sel_resize_begin(self: Pin<&mut Self>) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.sel_flip_sign = (false, false);
        }
    }

    pub fn sel_toggle_quick_mask(mut self: Pin<&mut Self>) {
        let next = !*self.quick_mask();
        {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                // 没有选区时快速蒙版没有意义（PS 里也是灰的）。
                Some(session) if session.doc.selection.is_some() => {
                    session.quick_mask = next;
                }
                _ => return,
            }
        }
        self.as_mut().set_quick_mask(next);
        self.as_mut().refresh_preview();
        println!("{{\"phase\":\"ui\",\"event\":\"quick-mask\",\"on\":{next}}}");
    }

    pub fn crop_to_selection(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        // 用 effective()（含羽化）：软边要按覆盖率淡出，不能裁成硬边。
        let mask = {
            let guard = self.session.borrow();
            guard
                .as_ref()
                .and_then(|s| s.doc.selection.as_ref())
                .map(|st| st.effective())
        };
        let Some(mask) = mask else {
            return;
        };
        let area = mask.bounds;
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => session
                    .doc
                    .crop_to_selection(&mask)
                    .map(|_| true)
                    .map_err(|e| e.to_string()),
            }
        };
        match result {
            Ok(true) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                        // canvas 坐标变了：选区（画布坐标）失效。
                        s.clear_selection();
                    }
                }
                self.as_mut().refresh_preview();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"cropped\",\"rect\":\"{},{},{},{}\",\"shape\":\"selection\"}}",
                    area.x, area.y, area.width, area.height
                );
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
        // Cropping moves the canvas origin: canvas coordinates (and therefore
        // the selection) are no longer meaningful. 裁剪 op 自己已经把选区清
        // 掉了，这里只是顺手关掉快速蒙版并把状态同步回 QML。
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                session.clear_selection();
            }
        }
        self.touch_selection();
        self.as_mut().publish_selection();
    }
}
