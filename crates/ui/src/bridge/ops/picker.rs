// 拾色器。
use super::super::*;

impl qobject::Bridge {
    pub fn picker_begin(mut self: Pin<&mut Self>) {
        if *self.picker_open() {
            return;
        }
        self.as_mut().set_picker_open(true);
        println!("{{\"phase\":\"ui\",\"event\":\"picker-open\"}}");
    }

    pub fn picker_close(mut self: Pin<&mut Self>) {
        if !*self.picker_open() {
            return;
        }
        self.as_mut().set_picker_open(false);
        println!("{{\"phase\":\"ui\",\"event\":\"picker-close\"}}");
    }

    pub fn picker_pick(mut self: Pin<&mut Self>, x: i32, y: i32) {
        let sample = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                None => None,
                Some(session) => {
                    let w = session.doc.canvas.width as i32;
                    let h = session.doc.canvas.height as i32;
                    if w <= 0 || h <= 0 {
                        None
                    } else {
                        // Clicks land in the 64px work-area pad around the
                        // canvas; clamp so an edge click still reports the
                        // nearest real pixel instead of dropping the pick.
                        let cx = x.clamp(0, w - 1);
                        let cy = y.clamp(0, h - 1);
                        let px = session.doc.composite_pixel(cx, cy);
                        Some(PickedColor::new(cx, cy, px.0))
                    }
                }
            }
        };
        let Some(sample) = sample else {
            return;
        };
        self.as_mut().set_picker_color(QString::from(sample.to_json().as_str()));
        {
            // Newest first, consecutive duplicates collapse into one entry.
            let mut recent = self.picker_recent.borrow_mut();
            if recent.first() != Some(&sample) {
                recent.insert(0, sample);
                recent.truncate(PICKER_HISTORY);
            }
        }
        self.as_mut().publish_picker_history();
        println!(
            "{{\"phase\":\"ui\",\"event\":\"picker-pick\",\"x\":{},\"y\":{},\"hex\":\"{}\"}}",
            sample.x,
            sample.y,
            sample.hex()
        );
    }

    /// 画布上某点（图像坐标）的十六进制颜色，供画笔 / 修复画笔按住 Alt
    /// 临时取样用。取样源跟随当前编辑目标（选中图层优先，否则背景），和
    /// 修复、油漆桶这些"改像素"的工具保持同一套语义。
    /// 越界钳制到最近像素；没有文档时返回空串。
    pub fn canvas_color_at(self: Pin<&mut Self>, x: i32, y: i32) -> QString {
        let selected = *self.selected_layer();
        let guard = self.session.borrow();
        let Some(session) = guard.as_ref() else {
            return QString::from("");
        };
        let w = session.doc.canvas.width as i32;
        let h = session.doc.canvas.height as i32;
        if w <= 0 || h <= 0 {
            return QString::from("");
        }
        let cx = x.clamp(0, w - 1);
        let cy = y.clamp(0, h - 1);
        let px = if selected >= 0 && (selected as usize) < session.doc.layers.len() {
            session.doc.layer_pixel(selected as usize, cx, cy)
        } else {
            session.doc.base_pixel_at(cx, cy)
        };
        let hex = format!("#{:02x}{:02x}{:02x}", px[0], px[1], px[2]);
        println!(
            "{{\"phase\":\"ui\",\"event\":\"sample-color\",\"x\":{cx},\"y\":{cy},\"hex\":\"{hex}\"}}"
        );
        QString::from(hex.as_str())
    }

    pub fn picker_select(mut self: Pin<&mut Self>, index: i32) {        let picked = {
            let recent = self.picker_recent.borrow();
            if index < 0 {
                None
            } else {
                recent.get(index as usize).copied()
            }
        };
        let Some(sample) = picked else {
            return;
        };
        self.as_mut().set_picker_color(QString::from(sample.to_json().as_str()));
        println!(
            "{{\"phase\":\"ui\",\"event\":\"picker-select\",\"index\":{index},\"hex\":\"{}\"}}",
            sample.hex()
        );
    }

    pub fn picker_clear(mut self: Pin<&mut Self>) {
        self.picker_recent.borrow_mut().clear();
        self.as_mut().publish_picker_history();
        self.as_mut().set_picker_color(QString::default());
        println!("{{\"phase\":\"ui\",\"event\":\"picker-clear\"}}");
    }
}
