// 色彩调整。
use super::super::*;

impl qobject::Bridge {
    pub fn adjust_begin(mut self: Pin<&mut Self>) {
        if *self.adjust_open() {
            return;
        }
        if self
            .session
            .borrow()
            .as_ref()
            .map(|session| session.doc.canvas.pixels() == 0)
            .unwrap_or(true)
        {
            return;
        }
        let selected = *self.selected_layer();
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            // Scope: the selected layer, else the base photo. Locked at open
            // time so changing the selection mid-adjust cannot retarget it.
            let target = session.adjust_scope(selected);
            session.adjust = Some(AdjustLive {
                target,
                params: Adjust::identity(),
            });
        }
        println!("{{\"phase\":\"ui\",\"event\":\"adjust-open\"}}");
        self.as_mut().refresh_preview();
    }

    pub fn adjust_set(mut self: Pin<&mut Self>, name: &QString, value: f64) {
        let name = name.to_string();
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(state) = session.adjust.as_mut() else {
                return;
            };
            let v = value as f32;
            let bipolar = v.clamp(-1.0, 1.0);
            let unipolar = v.clamp(0.0, 1.0);
            match name.as_str() {
                "exposure" | "e" => state.params.exposure = bipolar,
                "contrast" | "c" => state.params.contrast = bipolar,
                "highlights" | "hi" => state.params.highlights = bipolar,
                "shadows" | "sh" => state.params.shadows = bipolar,
                "saturation" | "sa" => state.params.saturation = bipolar,
                "temperature" | "te" => state.params.temperature = bipolar,
                "tint" | "ti" => state.params.tint = bipolar,
                "sepia" | "se" => state.params.sepia = unipolar,
                "sharpness" | "sp" => state.params.sharpness = unipolar,
                "black" | "bl" => state.params.black = unipolar,
                "white" | "wh" => state.params.white = unipolar,
                "gamma" | "ga" => state.params.gamma = (v as f32).clamp(0.05, 20.0),
                _ => {
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"adjust-unknown\",\"name\":\"{name}\"}}"
                    );
                    return;
                }
            }
            state.params.sanitize();
        }
        self.as_mut().refresh_preview();
    }

    pub fn adjust_set_levels(mut self: Pin<&mut Self>, black: f64, gamma: f64, white: f64) {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(state) = session.adjust.as_mut() else {
                return;
            };
            state.params.black = (black as f32).clamp(0.0, 0.95);
            state.params.white = (white as f32).clamp(0.0, 1.0);
            state.params.gamma = (gamma as f32).clamp(0.05, 20.0);
            state.params.sanitize();
        }
        self.as_mut().refresh_preview();
    }

    pub fn adjust_auto_levels(mut self: Pin<&mut Self>) {
        let mut levels = None;
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(state) = session.adjust.as_mut() else {
                return;
            };
            if let Some(hist) = session.doc.scope_histogram(state.target) {
                let (black, white) = hist.auto_levels();
                state.params.black = black;
                state.params.white = white;
                state.params.gamma = 1.0;
                state.params.sanitize();
                levels = Some((black, white));
            }
        }
        if let Some((black, white)) = levels {
            println!(
                "{{\"phase\":\"ui\",\"event\":\"adjust-auto\",\"black\":{black:.3},\"white\":{white:.3}}}"
            );
        }
        self.as_mut().refresh_preview();
    }

    pub fn adjust_reset(mut self: Pin<&mut Self>) {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(state) = session.adjust.as_mut() else {
                return;
            };
            state.params = Adjust::identity();
        }
        self.as_mut().refresh_preview();
    }

    pub fn adjust_commit(mut self: Pin<&mut Self>) {
        let (scope, committed, error) = {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(state) = session.adjust else {
                return;
            };
            let scope = match state.target {
                AdjustTarget::Base => "base".to_string(),
                AdjustTarget::Layer(index) => format!("layer:{index}"),
            };
            let mut committed = false;
            let mut error = String::new();
            if !state.params.is_identity() {
                match session.doc.adjust_pixels(state.target, &state.params) {
                    Ok(()) => {
                        session.dirty = true;
                        committed = true;
                    }
                    Err(err) => error = err.to_string(),
                }
            }
            session.adjust = None;
            (scope, committed, error)
        };
        if error.is_empty() {
            println!(
                "{{\"phase\":\"ui\",\"event\":\"adjust-commit\",\"applied\":{committed},\"scope\":\"{scope}\"}}"
            );
        } else {
            println!(
                "{{\"phase\":\"ui\",\"event\":\"adjust-error\",\"detail\":\"{error}\"}}"
            );
        }
        if committed {
            self.as_mut()
                .set_status(QString::from("已应用调色（Ctrl+Z 可撤销）"));
        }
        self.as_mut().refresh_preview();
    }

    pub fn adjust_cancel(mut self: Pin<&mut Self>) {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            session.adjust = None;
        }
        println!("{{\"phase\":\"ui\",\"event\":\"adjust-cancel\"}}");
        self.as_mut().refresh_preview();
    }
}
