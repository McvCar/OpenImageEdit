// 剪贴板。
use super::super::*;

impl qobject::Bridge {
    pub fn copy_selection_pixels(mut self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32) {
        let Some(rect) = self.clamp_selection(x, y, w, h) else {
            return;
        };
        let result: Result<Layer, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => session
                    .doc
                    .selection_layer(&Selection::Rect(rect))
                    .map_err(|e| e.to_string()),
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
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"copied\",\"rect\":\"{},{},{},{}\"}}",
                    rect.x, rect.y, rect.width, rect.height
                );
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn cut_selection_pixels(mut self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32) {
        let Some(rect) = self.clamp_selection(x, y, w, h) else {
            return;
        };
        let viewport = *self.viewport.borrow();
        let result: Result<Layer, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    match session.doc.selection_layer(&Selection::Rect(rect)) {
                        Err(e) => Err(e.to_string()),
                        Ok(layer) => {
                            match session.doc.erase_selection(&Selection::Rect(rect)) {
                                Err(e) => Err(e.to_string()),
                                Ok(()) => {
                                    session.refit_if_needed(viewport);
                                    session.dirty = true;
                                    Ok(layer)
                                }
                            }
                        }
                    }
                }
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
                self.as_mut().refresh_preview();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"cut\",\"rect\":\"{},{},{},{}\"}}",
                    rect.x, rect.y, rect.width, rect.height
                );
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn erase_selection_pixels(mut self: Pin<&mut Self>, x: i32, y: i32, w: i32, h: i32) {
        let Some(rect) = self.clamp_selection(x, y, w, h) else {
            return;
        };
        let viewport = *self.viewport.borrow();
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Ok(false),
                Some(session) => {
                    match session.doc.erase_selection(&Selection::Rect(rect)) {
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
                self.as_mut().set_status(QString::from("已删除选区像素（非破坏，可撤销）"));
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"erased\",\"rect\":\"{},{},{},{}\"}}",
                    rect.x, rect.y, rect.width, rect.height
                );
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
