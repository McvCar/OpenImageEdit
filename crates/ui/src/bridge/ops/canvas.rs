// 裁剪 / 画布缩放 / 旋转翻转 / 撤销重做 / 选区提取。
use super::super::*;

impl qobject::Bridge {
    pub fn rotate_cw(mut self: Pin<&mut Self>) {
        self.as_mut().apply_transform(Transform::Rotate90Cw, "rotate-cw");
    }

    pub fn rotate_ccw(mut self: Pin<&mut Self>) {
        self.as_mut()
            .apply_transform(Transform::Rotate90Ccw, "rotate-ccw");
    }

    pub fn flip_horizontal(mut self: Pin<&mut Self>) {
        self.as_mut()
            .apply_transform(Transform::FlipHorizontal, "flip-horizontal");
    }

    pub fn flip_vertical(mut self: Pin<&mut Self>) {
        self.as_mut()
            .apply_transform(Transform::FlipVertical, "flip-vertical");
    }

    pub fn undo(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let outcome: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => match session.doc.undo() {
                    Ok(()) => {
                        session.refit_if_needed(viewport);
                        session.dirty = true;
                        Ok(true)
                    }
                    Err(err) => Err(err.to_string()),
                },
            }
        };
        match outcome {
            Ok(true) => {
                // 撤销可能把选区整个换掉（SetSelection 也在历史里）：蚂蚁线、
                // 尺寸徽标、八个蓝点全靠这次发布刷新。
                self.as_mut().sync_selection_after_history();
                self.as_mut().refresh_preview();
            }
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn redo(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let outcome: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => match session.doc.redo() {
                    Ok(()) => {
                        session.refit_if_needed(viewport);
                        session.dirty = true;
                        Ok(true)
                    }
                    Err(err) => Err(err.to_string()),
                },
            }
        };
        match outcome {
            Ok(true) => {
                self.as_mut().sync_selection_after_history();
                self.as_mut().refresh_preview();
            }
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn crop_to(mut self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32) {
        let viewport = *self.viewport.borrow();
        let canvas = self
            .session
            .borrow()
            .as_ref()
            .map(|s| s.doc.canvas);
        let canvas = match canvas {
            None => return,
            Some(c) => c,
        };
        let x = x.clamp(0, canvas.width as i32 - 1);
        let y = y.clamp(0, canvas.height as i32 - 1);
        let w = w.max(1).min(canvas.width as i32 - x) as u32;
        let h = h.max(1).min(canvas.height as i32 - y) as u32;
        let rect = Rect::new(x, y, w, h);
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    session
                        .doc
                        .crop(rect)
                        .map(|_| true)
                        .map_err(|e| e.to_string())
                }
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
                // ★只改文档不发布，QML 的 selExists 就还停在 true：蚂蚁线、
                //   手柄、尺寸徽标全留在原地 —— 用户看到的「裁剪成功但选框
                //   没消失」。选区状态必须显式 publish 一次。
                self.touch_selection();
                self.as_mut().publish_selection();
                self.as_mut().refresh_preview();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"cropped\",\"rect\":\"{x},{y},{w},{h}\"}}"
                );
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn resize_to(mut self: Pin<&mut Self>, width: i32, height: i32) {
        if width < 1 || height < 1 {
            self.as_mut()
                .set_status(QString::from("resize-error: size must be >= 1"));
            return;
        }
        let viewport = *self.viewport.borrow();
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    let size = Size::new(width as u32, height as u32);
                    let old = session.doc.canvas;
session
                        .doc
                        .resize(size)
                        .map(move |_| {
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"resized\",\"from\":\"{}x{}\",\"to\":\"{}x{}\"}}",
                                old.width, old.height, size.width, size.height
                            );
                            true
                        })
                        .map_err(|e| e.to_string())
                }
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
                self.touch_selection();
                self.as_mut().publish_selection();
                self.as_mut().refresh_preview();
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn resize_canvas(mut self: Pin<&mut Self>, width: i32, height: i32) {
        if width < 1 || height < 1 {
            self.as_mut()
                .set_status(QString::from("resize-error: size must be >= 1"));
            return;
        }
        let viewport = *self.viewport.borrow();
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    let size = Size::new(width as u32, height as u32);
                    let old = session.doc.canvas;
                    session
                        .doc
                        .resize_canvas(size)
                        .map(move |_| {
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"canvas-resized\",\"from\":\"{}x{}\",\"to\":\"{}x{}\"}}",
                                old.width, old.height, size.width, size.height
                            );
                            true
                        })
                        .map_err(|e| e.to_string())
                }
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
                self.touch_selection();
                self.as_mut().publish_selection();
                self.as_mut().refresh_preview();
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn reset_crop(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    let base = session.doc.base.dimensions();
                    let is_full =
                        session.doc.view.x == 0
                            && session.doc.view.y == 0
                            && session.doc.view.width == base.0
                            && session.doc.view.height == base.1;
                    if is_full {
                        Ok(false)
                    } else {
                        match session.doc.reset_crop() {
                            Ok(()) => {
                                session.refit_if_needed(viewport);
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
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
                self.touch_selection();
                self.as_mut().publish_selection();
                self.as_mut().refresh_preview();
                self.as_mut()
                    .set_status(QString::from("crop reset"));
            }
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn extract_selection(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let selected = *self.selected_layer();
        let result: Result<i32, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => match session.effective_selection() {
                    None => Err("没有选区".into()),
                    // 选中图层 → 抠这一层；没选中 → 抠背景（PS 语义）。
                    Some(mask) => match super::paint::pick_target(&session.doc, selected) {
                        None => Err("没有可编辑的图层".into()),
                        Some(target) => session
                            .doc
                            .extract_selection_on(target, &mask)
                            .map(|ix| ix as i32)
                            .map_err(|e| e.to_string()),
                    },
                },
            }
        };
        match result {
            Ok(ix) => {
                self.as_mut().set_selected_layer(ix);
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                        // ★抠出来的像素已经是独立图层了，画布上再留一圈蚂蚁线
                        //   只会让人以为「还能拖这块选区」—— 用户报的「提取 /
                        //   剪切成功后选区应该消失」。用户显式要选区的话再框一次。
                        s.clear_selection();
                    }
                }
                self.touch_selection();
                self.as_mut().publish_selection();
                self.as_mut().refresh_preview();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"layer-extracted\",\"index\":{ix}}}"
                );
            }
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }
}
