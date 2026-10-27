// 视口 / 缩放 / 预览刷新。
use super::super::*;

/// Append a line to a small DPI diagnostic log so the *real* devicePixelRatio
/// the running app sees can be confirmed on a specific machine without a
/// console. Pure std file I/O (no Windows FFI — a previous FFI-based probe
/// raised an access violation on first call, so this stays in pure Rust).
/// The file is tiny and overwritten on each launch.
fn dpr_log(line: &str) {
    let Some(root) = std::env::var_os("APPDATA") else {
        return;
    };
    let dir = std::path::PathBuf::from(root).join("ImageEdit");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("dpr.log");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::write(&file, format!("{}\n", line).as_bytes());
    let _ = stamp; // (kept for clarity; timestamp is implicit from launch)
}

impl qobject::Bridge {
    pub fn set_dpr(mut self: Pin<&mut Self>, ratio: f64) {
        let ratio = if ratio.is_finite() && ratio > 0.0 { ratio } else { 1.0 };
        dpr_log(&format!(
            "set_dpr ratio={} dpr-before={:.3} pid={}",
            ratio,
            self.dpr.get(),
            std::process::id()
        ));
        if (self.dpr.get() - ratio).abs() < 0.001 {
            return;
        }
        self.dpr.set(ratio);
        // The on-screen preview was rendered at the old physical resolution;
        // re-render so it matches the new DPI. Only when a document is open.
        let viewport = *self.viewport.borrow();
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.dpr = ratio;
            // A fit-mode session was fitted at the old scaling; recompute the
            // fit zoom so "fit" stays correct and 100% keeps meaning 1 image
            // px = 1 physical px under the new dpr. Manual zooms are left
            // alone (zoom is physical px per image px, so it is dpr-independent).
            if session.fit {
                session.refit(viewport);
            }
        }
        if self.session.borrow().is_some() {
            self.as_mut().refresh_preview();
        }
    }

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
                    let display = display_size(session.doc.canvas, session.zoom, session.dpr);
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
