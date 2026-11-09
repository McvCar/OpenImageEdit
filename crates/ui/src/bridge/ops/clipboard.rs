// 剪贴板。
use super::super::*;
// 「选中图层优先，否则背景」的唯一判定，和修复 / 油漆工具共用。
use super::paint::{layer_locked, pick_target};

/// 上次「消费掉」的系统剪贴板序列号。
///
/// 用途：区分「剪贴板里的东西是本程序复制的」还是「用户从微信 / 浏览器 /
/// 资源管理器复制进来的」。内部复制不会动系统剪贴板，所以这里记下当时的值，
/// 之后序列号变了才知道是外面改的。
fn last_sys_seq() -> &'static std::sync::Mutex<i64> {
    static SEQ: std::sync::OnceLock<std::sync::Mutex<i64>> = std::sync::OnceLock::new();
    SEQ.get_or_init(|| std::sync::Mutex::new(0))
}

/// 内部复制 / 剪切之后调一次：把系统序列号记成「已消费」，下次粘贴不会
/// 误以为剪贴板被外部改写过。
fn consume_sys_clipboard() {
    *last_sys_seq().lock().unwrap() = super::sysclip::sequence_number();
}

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
                    layer.img().width(),
                    layer.img().height()
                );
                *clipboard().lock().unwrap() = Some(layer);
                consume_sys_clipboard();
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
                    layer.img().width(),
                    layer.img().height()
                );
                *clipboard().lock().unwrap() = Some(layer);
                consume_sys_clipboard();
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
        consume_sys_clipboard();
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
        consume_sys_clipboard();
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
        // 系统剪贴板序列号变了 = 刚有别的程序写过它（微信截图、浏览器右键
        // 复制图片、资源管理器复制文件）。这时候以系统里的图片为准，粘成
        // 一个新图层；序列号没变才继续用程序内部存的 Layer（跨窗口复制粘贴
        // 靠的是那个，它不会动系统剪贴板）。
        let seq = super::sysclip::sequence_number();
        if seq != *last_sys_seq().lock().unwrap() {
            *last_sys_seq().lock().unwrap() = seq;
            if let Some(image) = super::sysclip::read_image() {
                return self.as_mut().paste_foreign_image(image);
            }
        }
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
                    let (bx, by) = layer.offset();
                    let ox = (bx + PASTE_SHIFT * count).clamp(
                        -(layer.img().width() as i32 - 1),
                        canvas.width as i32 - 1,
                    );
                    let oy = (by + PASTE_SHIFT * count).clamp(
                        -(layer.img().height() as i32 - 1),
                        canvas.height as i32 - 1,
                    );
                    layer.set_offset((ox, oy));
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

    /// 把**系统剪贴板里的图片**粘成新图层，落在画布中心。
    ///
    /// 和内部粘贴的区别：内部剪贴板记着原来的位置（剪了再贴能贴回原处），
    /// 外部图片没有「原位置」可言，居中放最自然；比画布大的图同样居中，
    /// 四周均匀溢出（之后可以拖 / 缩放手柄调整）。
    fn paste_foreign_image(mut self: Pin<&mut Self>, image: RgbaImage) -> bool {
        let viewport = *self.viewport.borrow();
        let (w, h) = image.dimensions();
        let result: Result<usize, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("这个窗口还没有打开图片".into()),
                Some(session) => {
                    let canvas = session.doc.canvas;
                    let mut layer = Layer::new(session.doc.next_layer_name(), image);
                    layer.set_offset((
                        (canvas.width as i32 - w as i32) / 2,
                        (canvas.height as i32 - h as i32) / 2,
                    ));
                    match session.doc.add_layer(layer) {
                        Ok(index) => {
                            session.dirty = true;
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
                self.as_mut().set_status(QString::from(
                    format!("已从剪贴板粘贴 {w}×{h} 为新图层").as_str(),
                ));
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"pasted-image\",\"index\":{index},\"w\":{w},\"h\":{h}}}"
                );
                true
            }
            Err(message) => {
                self.as_mut().set_status(QString::from(message.as_str()));
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"paste-image-failed\",\"reason\":\"{}\"}}",
                    message
                );
                false
            }
        }
    }
}
