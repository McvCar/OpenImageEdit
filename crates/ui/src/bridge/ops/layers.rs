// 全部图层操作。
use super::super::*;

impl qobject::Bridge {
    pub fn layer_name(self: Pin<&mut Self>, index: i32) -> QString {
        let name = self
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.doc.layers.get(index.max(0) as usize))
            .map(|l| l.name.clone())
            .unwrap_or_default();
        QString::from(name.as_str())
    }

    pub fn layer_visible(self: Pin<&mut Self>, index: i32) -> bool {
        self.session
            .borrow()
            .as_ref()
            .and_then(|s| s.doc.layers.get(index.max(0) as usize))
            .map(|l| l.visible)
            .unwrap_or(false)
    }

    pub fn select_layer(mut self: Pin<&mut Self>, index: i32) {
        if index < 0 {
            // Negative index = deselect (Esc / clicking empty canvas).
            self.as_mut().set_selected_layer(-1);
            return;
        }
        if self.selected_guard(index).is_none() {
            return;
        }
        self.as_mut().set_selected_layer(index);
    }

    pub fn toggle_selected_visible(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let cur = session.doc.layers[ix].visible;
                    session
                        .doc
                        .set_layer_visible(ix, !cur)
                        .map(|()| true)
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
                    }
                }
                self.as_mut().refresh_preview();
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn set_layer_visible_at(mut self: Pin<&mut Self>, index: i32, visible: bool) {
        if index < 0 {
            return;
        }
        let viewport = *self.viewport.borrow();
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if index as usize >= session.doc.layers.len() {
                        Err("layer index out of range".into())
                    } else {
                        session
                            .doc
                            .set_layer_visible(index as usize, visible)
                            .map(|()| true)
                            .map_err(|e| e.to_string())
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
                    }
                }
                self.as_mut().refresh_preview();
            }
            Ok(false) => {}
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn delete_selected_layer(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            self.as_mut()
                .set_status(QString::from("图层已锁定，先解锁再删除"));
            return;
        }
        let result: Result<i32, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => match session.doc.remove_layer(ix) {
                        Ok(()) => {
                            let new_count = session.doc.layers.len();
                            Ok(selection_after_removal(ix, new_count))
                        }
                        Err(e) => Err(e.to_string()),
                    },
            }
        };
        match result {
            Ok(sel) => {
                self.as_mut().set_selected_layer(sel);
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                    }
                }
                self.as_mut().refresh_preview();
                println!("{{\"phase\":\"ui\",\"event\":\"layer-deleted\",\"index\":{ix}}}");
            }
            Err(message) => self
                .as_mut()
                .set_status(QString::from(message.as_str())),
        }
    }

    pub fn move_selected_up(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if ix + 1 >= session.doc.layers.len() {
                        Ok(false)
                    } else {
                        match session.doc.move_layer(ix, ix + 1) {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        if result.as_ref().ok().copied() == Some(true) {
            self.as_mut().set_selected_layer((ix + 1) as i32);
        }
        self.apply_layer_edit(viewport, result);
    }

    pub fn move_selected_down(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if ix == 0 {
                        Ok(false)
                    } else {
                        match session.doc.move_layer(ix, ix - 1) {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        if result.as_ref().ok().copied() == Some(true) {
            self.as_mut().set_selected_layer((ix - 1) as i32);
        }
        self.apply_layer_edit(viewport, result);
    }

    pub fn reorder_layer(mut self: Pin<&mut Self>, from: i32, to: i32) {
        if from < 0 || to < 0 || from == to {
            return;
        }
        let viewport = *self.viewport.borrow();
        if self.layer_locked(from as usize) || self.layer_locked(to as usize) {
            self.as_mut()
                .set_status(QString::from("已锁定的图层不能调整顺序"));
            return;
        }
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let len = session.doc.layers.len() as i32;
                    if from >= len || to >= len {
                        Err("layer index out of range".into())
                    } else {
                        match session.doc.move_layer(from as usize, to as usize) {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        if result.as_ref().ok().copied() == Some(true) {
            self.as_mut().set_selected_layer(to);
        }
        self.apply_layer_edit(viewport, result);
    }

    pub fn translate_selected(mut self: Pin<&mut Self>, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
            return;
        }
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            self.as_mut()
                .set_status(QString::from("图层已锁定，先解锁再移动"));
            return;
        }
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => match session.doc.translate_layer(ix, dx, dy) {
                    Ok(()) => {
                        session.dirty = true;
                        Ok(true)
                    }
                    Err(e) => Err(e.to_string()),
                },
            }
        };
        self.apply_layer_edit(viewport, result);
    }

    pub fn set_selected_opacity(self: Pin<&mut Self>, opacity: f32, commit: bool) {
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let op = opacity.clamp(0.0, 1.0);
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if commit {
                        // Rewind any live (history-free) change first so the
                        // history op records the true pre-drag opacity.
                        let start = self.opacity_live.borrow_mut().take();
                        if let Some(start) = start {
                            if let Some(layer) = session.doc.layers.get_mut(ix) {
                                layer.opacity = start;
                            }
                        }
                        match session.doc.set_layer_opacity(ix, op) {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    } else {
                        if self.opacity_live.borrow().is_none() {
                            let cur = session.doc.layers.get(ix).map(|l| l.opacity);
                            *self.opacity_live.borrow_mut() = cur;
                        }
                        if let Some(layer) = session.doc.layers.get_mut(ix) {
                            layer.opacity = op;
                        }
                        Ok(true)
                    }
                }
            }
        };
        self.apply_layer_edit(viewport, result);
    }

    pub fn rename_selected_layer(self: Pin<&mut Self>, name: &QString) {
        let new_name = name.to_string();
        if new_name.trim().is_empty() {
            return;
        }
        let viewport = *self.viewport.borrow();
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => match session.doc.rename_layer(ix, new_name.as_str()) {
                    Ok(()) => {
                        session.dirty = true;
                        Ok(true)
                    }
                    Err(e) => Err(e.to_string()),
                },
            }
        };
        self.apply_layer_edit(viewport, result);
    }

    pub fn scale_selected_layer(
        mut self: Pin<&mut Self>,
        width: i32,
        height: i32,
        keep_ratio: bool,
    ) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            self.as_mut()
                .set_status(QString::from("图层已锁定，先解锁再缩放"));
            return;
        }
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let layer = &session.doc.layers[ix];
                    let (ow, oh) = (layer.image.width() as i32, layer.image.height() as i32);
                    let offset = layer.offset;
                    let (mut tw, mut th) = (width, height);
                    if keep_ratio && (tw <= 0 || th <= 0) {
                        // Derive the missing side from the current aspect.
                        if tw <= 0 && th > 0 {
                            tw = ((th as f64) * (ow as f64 / oh as f64)).round() as i32;
                        } else if th <= 0 && tw > 0 {
                            th = ((tw as f64) * (oh as f64 / ow as f64)).round() as i32;
                        }
                    }
                    if tw < 1 || th < 1 {
                        Err("缩放尺寸至少为 1×1".into())
                    } else if tw == ow && th == oh {
                        Ok(false)
                    } else {
                        match session
                            .doc
                            .scale_layer(ix, Size::new(tw as u32, th as u32), offset)
                        {
                            Ok(()) => {
                                session.dirty = true;
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
                self.as_mut().refresh_preview();
                self.as_mut().set_dirty(true);
                println!("{{\"phase\":\"ui\",\"event\":\"layer-scaled\",\"index\":{ix}}}");
            }
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn scale_layer_begin(self: Pin<&mut Self>) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            return;
        }
        let mut guard = self.session.borrow_mut();
        if let Some(session) = guard.as_mut() {
            if let Some(layer) = session.doc.layers.get(ix) {
                session.live_scale = Some(LiveScaleState {
                    index: ix,
                    snapshot: layer.image.clone(),
                    to: Size::new(layer.image.width(), layer.image.height()),
                    offset: layer.offset,
                    flip: (false, false),
                });
            }
        }
    }

    pub fn scale_layer_update(
        self: Pin<&mut Self>,
        w: i32,
        h: i32,
        ox: i32,
        oy: i32,
        flip_x: bool,
        flip_y: bool,
    ) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if w < 1 || h < 1 || self.layer_locked(ix) {
            return;
        }
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            match &mut session.live_scale {
                Some(ls) if ls.index == ix => {
                    ls.to = Size::new(w as u32, h as u32);
                    ls.offset = (ox, oy);
                    ls.flip = (flip_x, flip_y);
                }
                _ => return,
            }
        }
        self.refresh_preview();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn scale_layer_end(
        mut self: Pin<&mut Self>,
        commit: bool,
        w: i32,
        h: i32,
        ox: i32,
        oy: i32,
        flip_x: bool,
        flip_y: bool,
    ) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let mut outcome: Result<bool, String> = Ok(false);
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                // Drop the live state first: the document bitmap was never
                // modified during the gesture, so a cancel needs no rollback.
                match session.live_scale.take() {
                    Some(ls) if ls.index == ix && commit => {
                        match session.doc.scale_layer_flipped(
                            ix,
                            Size::new(w.max(1) as u32, h.max(1) as u32),
                            (ox, oy),
                            (flip_x, flip_y),
                        ) {
                            Ok(()) => {
                                session.dirty = true;
                                outcome = Ok(true);
                            }
                            Err(e) => outcome = Err(e.to_string()),
                        }
                    }
                    _ => {}
                }
            }
        }
        match outcome {
            Ok(true) => {
                self.as_mut().refresh_preview();
                self.as_mut().set_dirty(true);
                println!("{{\"phase\":\"ui\",\"event\":\"layer-scaled\",\"index\":{ix}}}");
            }
            Ok(false) => self.as_mut().refresh_preview(),
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn set_layer_locked(mut self: Pin<&mut Self>, index: i32, locked: bool) {
        if index < 0 {
            return;
        }
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                if let Some(layer) = session.doc.layers.get_mut(index as usize) {
                    layer.locked = locked;
                }
            }
        }
        self.as_mut().refresh_layers_only();
    }

    pub fn set_base_locked(mut self: Pin<&mut Self>, locked: bool) {
        let converted = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                Some(session) => {
                    session.base_locked = locked;
                    // Unlocking converts the base into a regular bottom
                    // layer once: from then on it moves / scales / deletes
                    // / renames exactly like any other layer, and the
                    // synthetic base row disappears from the panel.
                    if !locked {
                        matches!(session.doc.convert_base_to_layer(), Ok(()))
                    } else {
                        false
                    }
                }
                None => false,
            }
        };
        if converted {
            // Inserting at index 0 shifts every other layer index, so a
            // stale selection must go; a layer was added too, so
            // layer_count / undo state changed and a full refresh (not
            // just layers_json) is required.
            self.as_mut().set_selected_layer(-1);
            self.as_mut().refresh_preview();
        } else {
            self.as_mut().refresh_layers_only();
        }
    }

    pub fn toggle_base_visible(mut self: Pin<&mut Self>) {
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let make_visible = session.doc.base_hidden;
                    match session.doc.set_base_visible(make_visible) {
                        Ok(()) => {
                            session.dirty = true;
                            Ok(make_visible)
                        }
                        Err(e) => Err(e.to_string()),
                    }
                }
            }
        };
        match result {
            Ok(_) => {
                // Visibility flips don't change the canvas size: no refit.
                self.as_mut().refresh_preview();
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn delete_base_layer(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if session.base_locked {
                        Err("背景已锁定，先点击锁图标解锁".into())
                    } else if session.doc.base_converted {
                        // The base row no longer exists as such; deletion
                        // goes through the normal layer path.
                        Ok(false)
                    } else if session.doc.base_deleted {
                        Ok(false)
                    } else {
                        match session.doc.delete_base() {
                            Ok(()) => {
                                session.dirty = true;
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
                    }
                }
                self.as_mut().refresh_preview();
                println!("{{\"phase\":\"ui\",\"event\":\"base-deleted\"}}");
            }
            Ok(false) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    pub fn drag_translate_begin(self: Pin<&mut Self>) {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            return;
        }
        let guard = self.session.borrow();
        if let Some(session) = guard.as_ref() {
            if let Some(layer) = session.doc.layers.get(ix) {
                *self.drag_start_offset.borrow_mut() = Some(layer.offset);
            }
        }
    }

    pub fn drag_translate_update(self: Pin<&mut Self>, dx: i32, dy: i32) {
        let Some(start) = *self.drag_start_offset.borrow() else {
            return;
        };
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        if self.layer_locked(ix) {
            return;
        }
        {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => {
                    // Skip render when the offset did not actually change.
                    if session.doc.layers.get(ix).map(|l| l.offset)
                        == Some((start.0 + dx, start.1 + dy))
                    {
                        return;
                    }
                }
                None => return,
            }
        }
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                if let Some(layer) = session.doc.layers.get_mut(ix) {
                    // Live preview only: no history entry until commit.
                    layer.offset = (start.0 + dx, start.1 + dy);
                }
            }
        }
        self.refresh_preview();
    }

    pub fn drag_translate_end(self: Pin<&mut Self>, commit: bool) {
        let viewport = *self.viewport.borrow();
        let Some(start) = self.drag_start_offset.borrow_mut().take() else {
            return;
        };
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return;
        };
        let result: Result<bool, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let cur = session
                        .doc
                        .layers
                        .get(ix)
                        .map(|l| l.offset)
                        .unwrap_or(start);
                    if !commit || cur == start {
                        // Roll back the live (history-free) offset.
                        if let Some(layer) = session.doc.layers.get_mut(ix) {
                            layer.offset = start;
                        }
                        Ok(true)
                    } else {
                        // Reset to the drag start, then translate once so the
                        // history op records from=start to=cur.
                        session.doc.layers[ix].offset = start;
                        match session
                            .doc
                            .translate_layer(ix, cur.0 - start.0, cur.1 - start.1)
                        {
                            Ok(()) => {
                                session.dirty = true;
                                Ok(true)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        self.apply_layer_edit(viewport, result);
    }
}
