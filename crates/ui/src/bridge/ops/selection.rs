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
    pub(crate) fn publish_selection(mut self: Pin<&mut Self>) {
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
        }
        if *self.sel_version() != rev {
            self.as_mut().set_sel_version(rev);
        }
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
        self.as_mut().publish_selection();
        // 快速蒙版是画进预览位图的，选区变了就得重出一帧。
        self.as_mut().refresh_for_selection();
    }

    /// The quick mask is drawn INTO the preview bitmap, so every selection
    /// change needs a re-render while it is on (and none while it is off).
    fn refresh_for_selection(mut self: Pin<&mut Self>) {
        if *self.quick_mask() && self.session.borrow().is_some() {
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
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
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
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
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
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
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
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
        println!("{{\"phase\":\"ui\",\"event\":\"sel-invert\"}}");
    }

    pub fn sel_set_border(mut self: Pin<&mut Self>, k: i32) {
        let k = k.clamp(0, 400) as u32;
        if !self.as_mut().mutate_selection_ops(|ops| ops.border = k) {
            return;
        }
        self.as_mut().remember_selection_ops();
        self.touch_selection();
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
    }

    pub fn sel_set_smooth(mut self: Pin<&mut Self>, k: i32) {
        let k = k.clamp(0, 400) as u32;
        if !self.as_mut().mutate_selection_ops(|ops| ops.smooth = k) {
            return;
        }
        self.as_mut().remember_selection_ops();
        self.touch_selection();
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
    }

    pub fn sel_set_feather(mut self: Pin<&mut Self>, r: i32) {
        let r = r.clamp(0, 250) as u32;
        if !self.as_mut().mutate_selection_ops(|ops| ops.feather = r) {
            return;
        }
        self.as_mut().remember_selection_ops();
        self.touch_selection();
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
    }

    /// 记住用户选的选区工具（下次启动继续用同一个）。
    pub fn save_sel_tool(mut self: Pin<&mut Self>, tool: &QString) {
        let name = tool.to_string();
        let valid = matches!(name.as_str(), "rect" | "ellipse" | "lasso" | "polygon");
        if !valid || self.sel_tool_saved().to_string() == name {
            return;
        }
        self.as_mut().set_sel_tool_saved(tool.clone());
        self.persist_config();
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
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
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
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
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
