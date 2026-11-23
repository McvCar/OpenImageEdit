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
                *clipboard().lock().unwrap() = Some(Clip {
                    layers: vec![layer],
                    group: None,
                });
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
                *clipboard().lock().unwrap() = Some(Clip {
                    layers: vec![layer],
                    group: None,
                });
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

    /// 复制图层 / 分组。
    ///
    /// * 当前是「组整体选中」(`selectedGroupId > 0`) → 复制**整组**：子树里
    ///   的非影子成员（自底向顶）+ 组记录（名字 / 混合 / 不透明度 / 穿透）。
    /// * 否则只复制主选中层，并且**清掉它的组归属**：以前副本会把 `group`
    ///   一起带过去，粘贴后新层直接落进原来的组，用户看到的就是「复制一个
    ///   图层，把上级分组也复制了」。
    pub fn copy_selected_layer(mut self: Pin<&mut Self>) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            self.as_mut().set_status(QString::from("没有选中的图层"));
            return;
        };
        let gid = *self.selected_group_id();
        let (clip, text): (Result<Clip, String>, String) = {
            let guard = self.session.borrow();
            let Some(session) = guard.as_ref() else {
                return;
            };
            let doc = &session.doc;
            if gid > 0 && doc.find_group(gid as u32).is_some() {
                let members: Vec<usize> = doc
                    .group_subtree_layers(gid as u32)
                    .into_iter()
                    .filter(|&i| doc.layers.get(i).map(|l| !l.synthetic).unwrap_or(false))
                    .collect();
                let layers: Vec<Layer> = members.iter().map(|&i| doc.layers[i].clone()).collect();
                let group = doc.find_group(gid as u32).cloned();
                let name = group
                    .as_ref()
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                (
                    Ok(Clip { layers, group }),
                    format!(
                        "已复制分组「{}」共 {} 层（Ctrl+V 粘贴）",
                        name,
                        members.len()
                    ),
                )
            } else {
                let mut layer = doc.layers[ix].clone();
                layer.group = None;
                (
                    Ok(Clip {
                        layers: vec![layer],
                        group: None,
                    }),
                    format!("已复制图层「{}」（Ctrl+V 粘贴，可跨文档）", doc.layers[ix].name),
                )
            }
        };
        match clip {
            Ok(clip) => {
                *clipboard().lock().unwrap() = Some(clip);
                consume_sys_clipboard();
                *self.paste_counter.borrow_mut() = 0;
                self.as_mut().set_status(QString::from(text.as_str()));
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"copied-layer\",\"index\":{ix},\
                     \"group\":{gid}}}"
                );
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn cut_selected_layer(mut self: Pin<&mut Self>) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            self.as_mut().set_status(QString::from("没有选中的图层"));
            return;
        };
        let gid = *self.selected_group_id();
        {
            let guard = self.session.borrow();
            let Some(session) = guard.as_ref() else {
                return;
            };
            let doc = &session.doc;
            let clip = if gid > 0 && doc.find_group(gid as u32).is_some() {
                let members: Vec<usize> = doc
                    .group_subtree_layers(gid as u32)
                    .into_iter()
                    .filter(|&i| doc.layers.get(i).map(|l| !l.synthetic).unwrap_or(false))
                    .collect();
                Clip {
                    layers: members.iter().map(|&i| doc.layers[i].clone()).collect(),
                    group: doc.find_group(gid as u32).cloned(),
                }
            } else {
                let mut layer = doc.layers[ix].clone();
                layer.group = None;
                Clip {
                    layers: vec![layer],
                    group: None,
                }
            };
            *clipboard().lock().unwrap() = Some(clip);
        }
        consume_sys_clipboard();
        *self.paste_counter.borrow_mut() = 0;
        println!(
            "{{\"phase\":\"ui\",\"event\":\"cut-layer\",\"index\":{ix},\"group\":{gid}}}"
        );
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
        // ★「复制不带走组」和「粘贴落到哪个组」是两件事：前者已经由
        //   `copy_selected_layer` 把副本的 `group` 清成 None 保证了；这里
        //   传 true 让单层粘贴沿用锚点图层的组归属 —— 用户复制组内图层粘贴，
        //   期望新层就在**同一个组**里，而不是掉到组外面变成顶层孤立行。
        let (anchor, anchor_group) = self.as_mut().insert_anchor(true);
        let is_group = clip.is_group();
        let result: Result<Vec<usize>, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("这个窗口还没有打开图片".into()),
                Some(session) => {
                    let canvas = session.doc.canvas;
                    let count = *self.paste_counter.borrow() as i32;
                    let mut layers = Vec::with_capacity(clip.layers.len());
                    for src in &clip.layers {
                        let mut layer = src.clone();
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
                        layers.push(layer);
                    }
                    // 复制整组 → 粘贴也建一个组；组 id 由目标文档重新分配。
                    let group = match (clip.group.clone(), anchor_group) {
                        (Some(g), _) => Some(g),
                        // 单层粘贴：沿用锚点所在组的归属（`insert_anchor(true)`
                        // 已给出），落到与原件同级的分组里。
                        (None, g) => {
                            if let Some(gid) = g {
                                for l in layers.iter_mut() {
                                    l.group = Some(gid);
                                }
                            }
                            None
                        }
                    };
                    session
                        .doc
                        .paste_layers_at(layers, anchor, group)
                        .map_err(|e| e.to_string())
                        .map(|placed| {
                            session.dirty = true;
                            *self.paste_counter.borrow_mut() = count as u32 + 1;
                            placed
                        })
                }
            }
        };
        match result {
            Ok(placed) => {
                let index = *placed.last().unwrap_or(&0);
                self.as_mut().set_selected_layer(index as i32);
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                // 多选集合跟着粘贴结果走：整组粘贴时整组入集合（可直接拖动）。
                self.as_mut().set_selection_indices(&placed);
                self.as_mut().refresh_preview();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"pasted\",\"index\":{index},\
                     \"count\":{},\"group\":{is_group}}}",
                    placed.len()
                );
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
        let (w, h) = image.dimensions();
        let on = self.as_mut().insert_image_layer(image);
        if on {
            self.as_mut().set_status(QString::from(
                format!("已从剪贴板粘贴 {w}×{h} 为新图层").as_str(),
            ));
            println!(
                "{{\"phase\":\"ui\",\"event\":\"pasted-image\",\"w\":{w},\"h\":{h}}}"
            );
        }
        on
    }

    /// 把一张已解码的图片作为新图层插入**当前画布**（居中），供剪贴板粘贴
    /// 与外部拖图共用。没有文档时返回 false。不碰 session 本身、不 bump
    /// load_serial（那是「打开新文档窗口贴合」专用）。
    pub(in crate::bridge) fn insert_image_layer(mut self: Pin<&mut Self>, image: RgbaImage) -> bool {
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
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"insert-image-layer\",\"index\":{index},\"w\":{w},\"h\":{h}}}"
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
