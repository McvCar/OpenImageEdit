// 标记 / 批注。
use super::super::*;

impl qobject::Bridge {
    pub fn select_mark(mut self: Pin<&mut Self>, index: i32) {
        self.as_mut().set_selected_mark(index);
    }

    pub fn add_shape_mark(
        mut self: Pin<&mut Self>,
        kind: i32,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        stroke: i32,
        color: &QString,
        fill: bool,
        fill_color: &QString,
    ) {
        let (x0, y0, x1, y1) = (x0 as f32, y0 as f32, x1 as f32, y1 as f32);
        let (rx0, rx1) = (x0.min(x1), x0.max(x1));
        let (ry0, ry1) = (y0.min(y1), y0.max(y1));
        let color = Self::parse_color(&color.to_string());
        let fill_color = Self::parse_color(&fill_color.to_string());
        let k = Self::mark_kind(kind);
        let points = match k {
            MarkKind::Arrow => vec![(x0, y0), (x1, y1)],
            _ => Vec::new(),
        };
        let mark = {
            let mut m = Mark::new(k);
            m.rect = (rx0, ry0, rx1, ry1);
            m.points = points;
            m.stroke = stroke.clamp(1, 255) as u8;
            m.color = color;
            m.fill = fill && matches!(k, MarkKind::Rect | MarkKind::Ellipse);
            m.fill_color = fill_color;
            m
        };
        {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => return,
                Some(session) => {
                    if session.doc.add_mark(mark).is_err() {
                        return;
                    }
                    session.dirty = true;
                }
            }
        }
        // Select the newly added (last) mark; a full refresh re-renders and
        // publishes marks_json.
        let count = {
            let guard = self.session.borrow();
            guard
                .as_ref()
                .map(|s| s.doc.marks.len())
                .unwrap_or(0)
        } as i32;
        self.as_mut().set_selected_mark(count - 1);
        self.as_mut().refresh_preview();
    }

    pub fn add_brush_mark(
        mut self: Pin<&mut Self>,
        points: &QString,
        stroke: i32,
        color: &QString,
    ) {
        let pts: Vec<(f32, f32)> = points
            .to_string()
            .split_whitespace()
            .filter_map(|pair| {
                let mut it = pair.split(',');
                let x = it.next()?.parse::<f32>().ok()?;
                let y = it.next()?.parse::<f32>().ok()?;
                Some((x, y))
            })
            .collect();
        if pts.is_empty() {
            return;
        }
        let (min_x, max_x) = pts.iter().fold((f32::MAX, f32::MIN), |acc, p| {
            (acc.0.min(p.0), acc.1.max(p.0))
        });
        let (min_y, max_y) = pts.iter().fold((f32::MAX, f32::MIN), |acc, p| {
            (acc.0.min(p.1), acc.1.max(p.1))
        });
        let mark = {
            let mut m = Mark::new(MarkKind::Brush);
            m.rect = (min_x, min_y, max_x, max_y);
            m.points = pts;
            m.stroke = stroke.clamp(1, 255) as u8;
            m.color = Self::parse_color(&color.to_string());
            m
        };
        {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => return,
                Some(session) => {
                    if session.doc.add_mark(mark).is_err() {
                        return;
                    }
                    session.dirty = true;
                }
            }
        }
        let count = {
            let guard = self.session.borrow();
            guard
                .as_ref()
                .map(|s| s.doc.marks.len())
                .unwrap_or(0)
        } as i32;
        self.as_mut().set_selected_mark(count - 1);
        self.as_mut().refresh_preview();
    }

    pub fn add_mosaic_mark(self: Pin<&mut Self>, points: &QString, stroke: i32, cell: i32) {
        let pts: Vec<(f32, f32)> = points
            .to_string()
            .split_whitespace()
            .filter_map(|pair| {
                let mut it = pair.split(',');
                let x = it.next()?.parse::<f32>().ok()?;
                let y = it.next()?.parse::<f32>().ok()?;
                Some((x, y))
            })
            .collect();
        if pts.is_empty() {
            return;
        }
        let (min_x, max_x) = pts.iter().fold((f32::MAX, f32::MIN), |acc, p| {
            (acc.0.min(p.0), acc.1.max(p.0))
        });
        let (min_y, max_y) = pts.iter().fold((f32::MAX, f32::MIN), |acc, p| {
            (acc.0.min(p.1), acc.1.max(p.1))
        });
        let mark = {
            let mut m = Mark::new(MarkKind::Mosaic);
            m.rect = (min_x, min_y, max_x, max_y);
            m.points = pts;
            m.stroke = stroke.clamp(1, 255) as u8;
            m.cell = cell.clamp(2, 255) as u8;
            m
        };
        {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => return,
                Some(session) => {
                    if session.doc.add_mark(mark).is_err() {
                        return;
                    }
                    session.dirty = true;
                }
            }
        }
        self.select_newest_mark();
    }

    pub fn add_mosaic_rect(
        self: Pin<&mut Self>,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        cell: i32,
    ) {
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let (y0, y1) = (y0.min(y1), y0.max(y1));
        if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
            return;
        }
        let mark = {
            let mut m = Mark::new(MarkKind::Mosaic);
            m.rect = (x0 as f32, y0 as f32, x1 as f32, y1 as f32);
            m.points = Vec::new(); // empty points = region mode in core
            m.cell = cell.clamp(2, 255) as u8;
            m
        };
        {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => return,
                Some(session) => {
                    if session.doc.add_mark(mark).is_err() {
                        return;
                    }
                    session.dirty = true;
                }
            }
        }
        self.select_newest_mark();
    }

    pub fn add_text_mark(
        self: Pin<&mut Self>,
        x: f64,
        y: f64,
        text: &QString,
        font: &QString,
        font_size: i32,
        bold: bool,
        italic: bool,
        color: &QString,
    ) {
        let text = text.to_string();
        if text.trim().is_empty() {
            return;
        }
        let font = font.to_string();
        let size = (font_size as f32).clamp(4.0, 512.0);
        let (tw, th) = editcore::measure_text(&text, &font, size, bold, italic);
        let mark = {
            let mut m = Mark::new(MarkKind::Text);
            m.rect = (x as f32, y as f32, x as f32 + tw, y as f32 + th);
            m.text = text;
            m.font = font;
            m.font_size = size;
            m.bold = bold;
            m.italic = italic;
            m.color = Self::parse_color(&color.to_string());
            m
        };
        {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => return,
                Some(session) => {
                    if session.doc.add_mark(mark).is_err() {
                        return;
                    }
                    session.dirty = true;
                }
            }
        }
        self.select_newest_mark();
    }

    pub fn update_mark_cell(mut self: Pin<&mut Self>, cell: i32) {
        let index = *self.selected_mark();
        if index < 0 {
            return;
        }
        let mut guard = self.session.borrow_mut();
        let Some(session) = guard.as_mut() else {
            return;
        };
        let Some(mark) = session.doc.marks.get(index as usize).cloned() else {
            return;
        };
        if mark.kind != MarkKind::Mosaic || mark.cell == cell.clamp(2, 255) as u8 {
            return;
        }
        let mut to = mark.clone();
        to.cell = cell.clamp(2, 255) as u8;
        session.dirty = true;
        let _ = session.doc.execute(editcore::EditOp::UpdateMark {
            index: index as usize,
            from: mark,
            to,
        });
        drop(guard);
        self.as_mut().refresh_preview();
    }

    pub fn update_text_style(
        mut self: Pin<&mut Self>,
        font: &QString,
        font_size: i32,
        bold: bool,
        italic: bool,
        underline: bool,
        align: i32,
    ) {
        let index = *self.selected_mark();
        if index < 0 {
            return;
        }
        let grabbed = {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(mark) = session.doc.marks.get(index as usize).cloned() else {
                return;
            };
            if mark.kind != MarkKind::Text {
                return;
            }
            let mut to = mark.clone();
            to.font = font.to_string();
            to.font_size = (font_size as f32).clamp(4.0, 512.0);
            to.bold = bold;
            to.italic = italic;
            to.underline = underline;
            to.align = align.clamp(0, 2) as u8;
            // Re-measure the rect so handles keep wrapping the new text.
            let (tw, th) =
                editcore::measure_text(&to.text, &to.font, to.font_size, to.bold, to.italic);
            to.rect = (mark.rect.0, mark.rect.1, mark.rect.0 + tw, mark.rect.1 + th);
            session.dirty = true;
            let _ = session.doc.execute(editcore::EditOp::UpdateMark {
                index: index as usize,
                from: mark,
                to,
            });
            ()
        };
        let _ = grabbed;
        self.as_mut().refresh_preview();
    }

    pub fn set_editing_mark(mut self: Pin<&mut Self>, index: i32) {
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                let idx = if index >= 0
                    && (index as usize) < session.doc.marks.len()
                {
                    index
                } else {
                    -1
                };
                if session.preview_hidden_mark != idx {
                    session.preview_hidden_mark = idx;
                }
            }
        }
        self.as_mut().refresh_preview();
    }

    pub fn update_text_content(mut self: Pin<&mut Self>, text: &QString) {
        let text = text.to_string();
        if text.trim().is_empty() {
            return;
        }
        let index = *self.selected_mark();
        if index < 0 {
            return;
        }
        let mut guard = self.session.borrow_mut();
        let Some(session) = guard.as_mut() else {
            return;
        };
        let Some(mark) = session.doc.marks.get(index as usize).cloned() else {
            return;
        };
        if mark.kind != MarkKind::Text || mark.text == text {
            return;
        }
        let mut to = mark.clone();
        to.text = text;
        // Re-measure the rect so handles keep wrapping the new text.
        let (tw, th) =
            editcore::measure_text(&to.text, &to.font, to.font_size, to.bold, to.italic);
        to.rect = (mark.rect.0, mark.rect.1, mark.rect.0 + tw, mark.rect.1 + th);
        session.dirty = true;
        let _ = session.doc.execute(editcore::EditOp::UpdateMark {
            index: index as usize,
            from: mark,
            to,
        });
        drop(guard);
        self.as_mut().refresh_preview();
    }

    pub fn mark_drag_begin(mut self: Pin<&mut Self>) {
        let index = *self.selected_mark();
        if index < 0 {
            return;
        }
        let grabbed = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                Some(session) => session.doc.marks.get(index as usize).cloned(),
                None => None,
            }
        };
        match grabbed {
            Some(mark) => {
                if std::env::var("IMAGEEDIT_MARK_TRACE").is_ok() {
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"mark-drag-begin\",\"index\":{index},\"rect\":[{},{},{},{}]}}",
                        mark.rect.0, mark.rect.1, mark.rect.2, mark.rect.3
                    );
                }
                *self.mark_drag.borrow_mut() = Some((index as usize, mark))
            }
            None => {
                if std::env::var("IMAGEEDIT_MARK_TRACE").is_ok() {
                    println!("{{\"phase\":\"ui\",\"event\":\"mark-drag-begin-skip\",\"index\":{index}}}");
                }
                self.as_mut().set_selected_mark(-1)
            }
        }
    }

    pub fn mark_drag_update(mut self: Pin<&mut Self>, x: f64, y: f64, w: f64, h: f64) {
        let Some((index, from)) = self.mark_drag.borrow().clone() else {
            if std::env::var("IMAGEEDIT_MARK_TRACE").is_ok() {
                println!("{{\"phase\":\"ui\",\"event\":\"mark-drag-update-skip\",\"why\":\"no-session\"}}");
            }
            return;
        };
        let to = from.mapped(
            from.rect,
            (
                x as f32,
                y as f32,
                x as f32 + w.max(1.0) as f32,
                y as f32 + h.max(1.0) as f32,
            ),
        );
        if std::env::var("IMAGEEDIT_MARK_TRACE").is_ok() {
            println!(
                "{{\"phase\":\"ui\",\"event\":\"mark-drag-update\",\"index\":{index},\"req\":[{x},{y},{w},{h}],\"from\":[{},{},{},{}],\"to\":[{},{},{},{}]}}",
                from.rect.0, from.rect.1, from.rect.2, from.rect.3,
                to.rect.0, to.rect.1, to.rect.2, to.rect.3
            );
        }
        let mut guard = self.session.borrow_mut();
        if let Some(session) = guard.as_mut() {
            if session.doc.update_mark_live(index, to).is_ok() {
                drop(guard);
                self.as_mut().refresh_preview();
            }
        }
    }

    pub fn mark_drag_end(mut self: Pin<&mut Self>, commit: bool) {
        let Some((index, from)) = self.mark_drag.borrow_mut().take() else {
            if std::env::var("IMAGEEDIT_MARK_TRACE").is_ok() {
                println!("{{\"phase\":\"ui\",\"event\":\"mark-drag-end-skip\",\"why\":\"no-session\"}}");
            }
            return;
        };
        let mut guard = self.session.borrow_mut();
        let Some(session) = guard.as_mut() else {
            return;
        };
        if !commit {
            let _ = session.doc.update_mark_live(index, from);
            drop(guard);
            self.as_mut().refresh_preview();
            return;
        }
        // `from` is the pre-drag snapshot, so undo restores it verbatim.
        let to = match session.doc.marks.get(index) {
            Some(m) => m.clone(),
            None => return,
        };
        session.dirty = true;
        let trace = std::env::var("IMAGEEDIT_MARK_TRACE").is_ok();
        let from_rect = from.rect;
        let _ = session.doc.execute(editcore::EditOp::UpdateMark { index, from, to });
        if trace {
            println!(
                "{{\"phase\":\"ui\",\"event\":\"mark-drag-end\",\"index\":{index},\"from\":[{},{},{},{}],\"now\":{:?}}}",
                from_rect.0, from_rect.1, from_rect.2, from_rect.3,
                session.doc.marks.get(index).map(|m| m.rect)
            );
        }
        drop(guard);
        self.as_mut().refresh_preview();
    }

    pub fn delete_selected_mark(mut self: Pin<&mut Self>) {
        let index = *self.selected_mark();
        if index < 0 {
            return;
        }
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            if session.doc.delete_mark(index as usize).is_err() {
                return;
            }
            session.dirty = true;
        }
        self.as_mut().set_selected_mark(-1);
        self.as_mut().refresh_preview();
    }

    pub fn set_mark_visible(mut self: Pin<&mut Self>, index: i32, visible: bool) {
        if index < 0 {
            return;
        }
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(mark) = session.doc.marks.get(index as usize).cloned() else {
                return;
            };
            if mark.visible == visible {
                return;
            }
            let mut to = mark.clone();
            to.visible = visible;
            session.dirty = true;
            let _ = session.doc.execute(editcore::EditOp::UpdateMark {
                index: index as usize,
                from: mark,
                to,
            });
        }
        self.as_mut().refresh_preview();
    }

    pub fn update_mark_style(
        mut self: Pin<&mut Self>,
        stroke: i32,
        color: &QString,
        fill: bool,
        fill_color: &QString,
    ) {
        let index = *self.selected_mark();
        if index < 0 {
            return;
        }
        let new_color = Self::parse_color(&color.to_string());
        let new_fill_color = Self::parse_color(&fill_color.to_string());
        let _from = {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(mark) = session.doc.marks.get(index as usize).cloned() else {
                return;
            };
            let mut to = mark.clone();
            to.stroke = stroke.clamp(1, 255) as u8;
            to.color = new_color;
            to.fill = fill && matches!(to.kind, MarkKind::Rect | MarkKind::Ellipse);
            to.fill_color = new_fill_color;
            session.dirty = true;
            let _ = session.doc.execute(editcore::EditOp::UpdateMark {
                index: index as usize,
                from: mark,
                to,
            });
        };
        self.as_mut().refresh_preview();
    }
}
