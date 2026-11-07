// 剪贴板。
use super::super::*;
// 「选中图层优先，否则背景」的唯一判定，和修复 / 油漆工具共用。
use super::paint::{layer_locked, pick_target};

impl qobject::Bridge {
    pub fn copy_selection_pixels(mut self: Pin<&mut Self>) {
        let selected = *self.selected_layer();
        let result: Result<Layer, String> = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                None => Err("no document".into()),
                Some(session) => match session.effective_selection() {
                    None => Err("没有选区".into()),
                    // 选中图层 → 只复制这一层；没选中 → 复制背景（PS 语义）。
                    Some(mask) => match pick_target(&session.doc, selected) {
                        None => Err("没有可复制的图层".into()),
                        Some(target) => session
                            .doc
                            .selection_layer_on(target, &mask)
                            .map_err(|e| e.to_string()),
                    },
                },
            }
        };
        match result {
            Ok(layer) => {
                let text = format!(
                    "已复制 {}×{}（Ctrl+V 粘贴，可跨文档）",
                    layer.image.width(),
                    layer.image.height()
                );
                *clipboard().lock().unwrap() = Some(layer);
                *self.paste_counter.borrow_mut() = 0;
                self.as_mut().set_status(QString::from(text.as_str()));
                println!("{{\"phase\":\"ui\",\"event\":\"copied\"}}");
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn cut_selection_pixels(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let selected = *self.selected_layer();
        let result: Result<Layer, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => match session.effective_selection() {
                    None => Err("没有选区".into()),
                    Some(mask) => match pick_target(&session.doc, selected) {
                        None => Err("没有可编辑的图层".into()),
                        Some(target) if layer_locked(&session.doc, target) => {
                            Err("图层已锁定".into())
                        }
                        Some(target) => match session.doc.selection_layer_on(target, &mask) {
                            Err(e) => Err(e.to_string()),
                            Ok(layer) => match session.doc.erase_selection_on(target, &mask) {
                                Err(e) => Err(e.to_string()),
                                Ok(()) => {
                                    session.refit_if_needed(viewport);
                                    session.dirty = true;
                                    Ok(layer)
                                }
                            },
                        },
                    },
                },
            }
        };
        match result {
            Ok(layer) => {
                let text = format!(
                    "已剪切 {}×{}（Ctrl+V 粘贴，可跨文档）",
                    layer.image.width(),
                    layer.image.height()
                );
                *clipboard().lock().unwrap() = Some(layer);
                *self.paste_counter.borrow_mut() = 0;
                self.as_mut().set_status(QString::from(text.as_str()));
                // 剪切 = 像素已经搬进剪贴板并从画布挖走。再把选区留在原地，
                // 用户会以为「剪完还能接着拖这块」—— 直接取消选择（PS 的
                // 剪切也是剪完即取消）。复制**不**动选区。
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.clear_selection();
                    }
                }
                self.touch_selection();
                self.as_mut().publish_selection();
                self.as_mut().refresh_preview();
                println!("{{\"phase\":\"ui\",\"event\":\"cut\"}}");
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn erase_selection_pixels(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let selected = *self.selected_layer();
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    let Some(mask) = session.effective_selection() else {
                        return;
                    };
                    let Some(target) = pick_target(&session.doc, selected) else {
                        return;
                    };
                    if layer_locked(&session.doc, target) {
                        return;
                    }
                    match session.doc.erase_selection_on(target, &mask) {
                        Ok(()) => {
                            session.refit_if_needed(viewport);
                            session.dirty = true;
                            Ok(true)
                        }
                        Err(e) => Err(e.to_string()),
                    }
                }
            }
        };
        match result {
            Ok(true) => {
                self.as_mut().refresh_preview();
                self.as_mut()
                    .set_status(QString::from("已删除选区像素（非破坏，可撤销）"));
                println!("{{\"phase\":\"ui\",\"event\":\"erased\"}}");
            }
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn copy_selected_layer(mut self: Pin<&mut Self>) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            self.as_mut().set_status(QString::from("没有选中的图层"));
            return;
        };
        let layer = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => session.doc.layers[ix].clone(),
                None => return,
            }
        };
        let text = format!("已复制图层「{}」（Ctrl+V 粘贴，可跨文档）", layer.name);
        *clipboard().lock().unwrap() = Some(layer);
        *self.paste_counter.borrow_mut() = 0;
        self.as_mut().set_status(QString::from(text.as_str()));
        println!("{{\"phase\":\"ui\",\"event\":\"copied-layer\",\"index\":{ix}}}");
    }

    pub fn cut_selected_layer(mut self: Pin<&mut Self>) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            self.as_mut().set_status(QString::from("没有选中的图层"));
            return;
        };
        let layer = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => session.doc.layers[ix].clone(),
                None => return,
            }
        };
        *clipboard().lock().unwrap() = Some(layer);
        *self.paste_counter.borrow_mut() = 0;
        println!("{{\"phase\":\"ui\",\"event\":\"cut-layer\",\"index\":{ix}}}");
        self.as_mut().delete_selected_layer();
    }

    /// Ctrl+拖动选区 = 剪切并搬走（PS 的剪刀光标）：选区像素抠成新图层、
    /// 原位置按选区覆盖率挖空（羽化边自然淡化），随后选中新图层让 QML
    /// 接着走「拖动图层」的手势。
    pub fn cut_selection_to_layer(mut self: Pin<&mut Self>) -> bool {
        let viewport = *self.viewport.borrow();
        let selected = *self.selected_layer();
        let result: Result<usize, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => match session.effective_selection() {
                    None => Err("没有选区".into()),
                    Some(mask) => match pick_target(&session.doc, selected) {
                        None => Err("没有可编辑的图层".into()),
                        Some(target) if layer_locked(&session.doc, target) => {
                            Err("图层已锁定".into())
                        }
                        Some(target) => match session.doc.extract_selection_on(target, &mask) {
                            Err(e) => Err(e.to_string()),
                            Ok(index) => match session.doc.erase_selection_on(target, &mask) {
                                Err(e) => Err(e.to_string()),
                                Ok(()) => {
                                    session.refit_if_needed(viewport);
                                    session.dirty = true;
                                    Ok(index)
                                }
                            },
                        },
                    },
                },
            }
        };
        match result {
            Ok(index) => {
                self.as_mut().set_selected_layer(index as i32);
                // 像素已经搬到新图层上了，选区（尤其是羽化边）留在原地只会
                // 让人以为还能继续剪同一块地方。PS 的剪刀手势也是剪完即取消。
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.clear_selection();
                    }
                }
                self.touch_selection();
                self.as_mut().publish_selection();
                self.as_mut().refresh_preview();
                self.as_mut()
                    .set_status(QString::from("已剪出为独立图层：拖动定位，可撤销"));
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"selection-cut-to-layer\",\"index\":{index}}}"
                );
                true
            }
            Err(message) => {
                self.as_mut().set_status(QString::from(message.as_str()));
                false
            }
        }
    }

    pub fn paste_clipboard(mut self: Pin<&mut Self>) -> bool {
        let clipboard = clipboard().lock().unwrap().clone();
        let Some(clip) = clipboard else {
            self.as_mut().set_status(QString::from(
                "剪贴板为空（跨窗口复制请用同一程序的「新窗口」，另起的程序不共享）",
            ));
            println!("{{\"phase\":\"ui\",\"event\":\"paste-empty\"}}");
            return false;
        };
        let viewport = *self.viewport.borrow();
        const PASTE_SHIFT: i32 = 12;
        let result: Result<usize, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("这个窗口还没有打开图片".into()),
                Some(session) => {
                    let canvas = session.doc.canvas;
                    let count = *self.paste_counter.borrow() as i32;
                    let mut layer = clip.clone();
                    // Auto-name by creation order ("1", "2", ...).
                    layer.name = session.doc.next_layer_name();
                    // Keep the pasted layer at its copied position, clamped so
                    // part of it stays on the canvas; repeated pastes fan out.
                    layer.offset.0 = (clip.offset.0 + PASTE_SHIFT * count).clamp(
                        -(layer.image.width() as i32 - 1),
                        canvas.width as i32 - 1,
                    );
                    layer.offset.1 = (clip.offset.1 + PASTE_SHIFT * count).clamp(
                        -(layer.image.height() as i32 - 1),
                        canvas.height as i32 - 1,
                    );
                    match session.doc.add_layer(layer) {
                        Ok(index) => {
                            session.dirty = true;
                            *self.paste_counter.borrow_mut() = count as u32 + 1;
                            Ok(index)
                        }
                        Err(e) => Err(e.to_string()),
                    }
                }
            }
        };
        match result {
            Ok(index) => {
                self.as_mut().set_selected_layer(index as i32);
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                self.as_mut().refresh_preview();
                println!("{{\"phase\":\"ui\",\"event\":\"pasted\",\"index\":{index}}}");
                true
            }
            Err(message) => {
                self.as_mut().set_status(QString::from(message.as_str()));
                println!("{{\"phase\":\"ui\",\"event\":\"paste-failed\",\"reason\":\"{}\"}}", message);
                false
            }
        }
    }
}
