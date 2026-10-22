// 视口 / 缩放 / 预览刷新。
use super::super::*;

impl qobject::Bridge {
    pub fn set_viewport(mut self: Pin<&mut Self>, width: i32, height: i32) {
        let size = Size::new(width.max(0) as u32, height.max(0) as u32);
        if *self.viewport.borrow() == size {
            return;
        }
        *self.viewport.borrow_mut() = size;
        let (refresh, unchanged) = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                Some(session) if session.fit => {
                    session.refit(size);
                    let display = display_size(session.doc.canvas, session.zoom);
                    (true, size_close(display, session.last_display))
                }
                _ => (false, false),
            }
        };
        // Same display size after the refit (the common case while dragging a
        // window edge 1–2px at a time): the rendered preview on screen is
        // already what this resize asks for, so skip the render entirely. A 2px
        // tolerance keeps micro-resizes free; the stretched 0.2% is invisible.
        if refresh && !unchanged {
            self.as_mut().refresh_preview();
        }
    }

    pub fn zoom_in(mut self: Pin<&mut Self>) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.zoom_by(ZOOM_STEP);
        }
        self.as_mut().refresh_preview_zoom();
    }

    pub fn zoom_out(mut self: Pin<&mut Self>) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.zoom_by(1.0 / ZOOM_STEP);
        }
        self.as_mut().refresh_preview_zoom();
    }

    pub fn zoom_step(mut self: Pin<&mut Self>, factor: f64) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.zoom_by(factor);
        }
        self.as_mut().refresh_preview_zoom();
    }

    pub fn zoom_fit(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.refit(viewport);
        }
        self.as_mut().refresh_preview_zoom();
    }

    pub fn zoom_actual(mut self: Pin<&mut Self>) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.zoom = 1.0;
            session.fit = false;
        }
        self.as_mut().refresh_preview_zoom();
    }
}
