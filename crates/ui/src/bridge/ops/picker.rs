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

    pub fn picker_select(mut self: Pin<&mut Self>, index: i32) {
        let picked = {
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
