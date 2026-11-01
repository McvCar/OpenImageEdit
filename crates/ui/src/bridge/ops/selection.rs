// 选区（PS 式）命令。几何真值在 core::SelectionState，这里只做
// 「QML 坐标 → 画布坐标 → 更新状态 → 发布」的搬运，外加视图刷新。
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
        let (active, rect, ops, rev, quick) = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => {
                    let (active, rect, ops) = match session.selection.as_ref() {
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
                    (active, rect, ops, session.sel_rev as i32, session.quick_mask)
                }
                None => (false, Rect::default(), (0, 0, 0), 0, false),
            }
        };
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
    fn touch_selection(&self) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.sel_rev = session.sel_rev.wrapping_add(1);
        }
    }

    /// The quick mask is drawn INTO the preview bitmap, so every selection
    /// change needs a re-render while it is on (and none while it is off).
    fn refresh_for_selection(mut self: Pin<&mut Self>) {
        if *self.quick_mask() && self.session.borrow().is_some() {
            self.as_mut().refresh_preview();
        }
    }

    /// Replace / add / subtract / intersect the current selection with `mask`.
    /// mode: 0 replace, 1 add, 2 subtract, 3 intersect.
    fn combine_selection(mut self: Pin<&mut Self>, mask: SelectionMask, mode: i32) {
        let canvas = match self.session.borrow().as_ref().map(|s| s.doc.canvas) {
            Some(c) => c,
            None => return,
        };
        let mask = mask.clipped(canvas);
        if !mask.is_empty() || mode != 0 {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let current = session.selection.as_ref().map(|s| s.mask().clone());
            let combined = match current {
                _ if mode == 0 => mask,
                Some(c) if mode == 1 => c.union(&mask),
                Some(c) if mode == 2 => c.subtract(&mask),
                Some(c) if mode == 3 => c.intersect(&mask),
                // 加/减/交但当前没有选区：行为同替换（与 PS 一致）。
                _ => mask,
            };
            session.selection = if combined.is_empty() {
                None
            } else {
                Some(SelectionState::new(combined))
            };
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
        self.as_mut().combine_selection(mask, 0);
    }

    pub fn sel_clear(mut self: Pin<&mut Self>) {
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                session.clear_selection();
            }
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
        let inverted = {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            match session.selection.as_ref() {
                Some(st) => st.mask().invert(canvas),
                None => SelectionMask::rect(Rect::new(0, 0, canvas.width, canvas.height)),
            }
        };
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.selection = Some(SelectionState::new(inverted));
        }
        self.touch_selection();
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
        println!("{{\"phase\":\"ui\",\"event\":\"sel-invert\"}}");
    }

    pub fn sel_set_border(mut self: Pin<&mut Self>, k: i32) {
        let k = k.clamp(0, 400) as u32;
        {
            let mut guard = self.session.borrow_mut();
            if let Some(st) = guard.as_mut().and_then(|s| s.selection.as_mut()) {
                st.set_border(k);
            }
        }
        self.touch_selection();
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
    }

    pub fn sel_set_smooth(mut self: Pin<&mut Self>, k: i32) {
        let k = k.clamp(0, 400) as u32;
        {
            let mut guard = self.session.borrow_mut();
            if let Some(st) = guard.as_mut().and_then(|s| s.selection.as_mut()) {
                st.set_smooth(k);
            }
        }
        self.touch_selection();
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
    }

    pub fn sel_set_feather(mut self: Pin<&mut Self>, r: i32) {
        let r = r.clamp(0, 250) as u32;
        {
            let mut guard = self.session.borrow_mut();
            if let Some(st) = guard.as_mut().and_then(|s| s.selection.as_mut()) {
                st.set_feather(r);
            }
        }
        self.touch_selection();
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
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
            .and_then(|s| s.selection.as_ref())
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
            if let Some(st) = guard.as_mut().and_then(|s| s.selection.as_mut()) {
                st.translate(dx, dy, canvas);
            }
        }
        self.touch_selection();
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
    }

    /// 拖手柄改选区大小（图像坐标，浮点）。Rust 侧按新包围盒重采样
    /// mask，QML 拖动期间只画本地预览框 —— 每帧重采样大选区会卡。
    pub fn sel_resize(mut self: Pin<&mut Self>, x: f64, y: f64, w: f64, h: f64) {
        let canvas = match self.session.borrow().as_ref().map(|s| s.doc.canvas) {
            Some(c) => c,
            None => return,
        };
        if w < 1.0 || h < 1.0 {
            return;
        }
        let bounds = Rect::new(
            x.round() as i32,
            y.round() as i32,
            w.round().max(1.0) as u32,
            h.round().max(1.0) as u32,
        );
        let empty = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut().and_then(|s| s.selection.as_mut()) {
                Some(st) => {
                    st.resize_to(bounds, canvas);
                    st.is_empty()
                }
                None => return,
            }
        };
        if empty {
            // 拖到没有面积了 = 取消选区（PS 里手柄拖过对边也是这个结果）。
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                session.clear_selection();
            }
        }
        self.touch_selection();
        self.as_mut().publish_selection();
        self.as_mut().refresh_for_selection();
    }

    pub fn sel_toggle_quick_mask(mut self: Pin<&mut Self>) {
        let next = !*self.quick_mask();
        {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                // 没有选区时快速蒙版没有意义（PS 里也是灰的）。
                Some(session) if session.selection.is_some() => {
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
        let rect = {
            let guard = self.session.borrow();
            guard
                .as_ref()
                .and_then(|s| s.selection.as_ref())
                .map(|st| st.mask().bounds)
        };
        let Some(rect) = rect else {
            return;
        };
        self.as_mut().crop_to(rect.x, rect.y, rect.width as i32, rect.height as i32);
        // Cropping moves the canvas origin: canvas coordinates (and therefore
        // the selection) are no longer meaningful.
        self.as_mut().sel_clear();
    }
}
