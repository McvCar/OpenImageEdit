// Session 渲染管线核心。所有项标 pub(in crate::bridge)，mod.rs 再导出。
use super::*;

pub(in crate::bridge) struct Session {
    pub(in crate::bridge) id: u32,
    pub(in crate::bridge) doc: Document,
    pub(in crate::bridge) source: Option<PathBuf>,
    pub(in crate::bridge) zoom: f64,
    pub(in crate::bridge) fit: bool,
    pub(in crate::bridge) render_count: u32,
    pub(in crate::bridge) dirty: bool,
    /// UI lock for the base row (blocks delete). Plain flag, not undoable.
    pub(in crate::bridge) base_locked: bool,
    /// Save quality snapshot from AppConfig at session creation (updated
    /// live by `update_save_quality`).
    pub(in crate::bridge) save_quality: u8,
    /// Mark index hidden from the PREVIEW only while its text is being
    /// edited inline in QML (the TextInput overlays the real mark). -1 = none.
    pub(in crate::bridge) preview_hidden_mark: i32,
    /// In-flight handle-driven layer resize. The document bitmap is NOT
    /// touched during the gesture: the preview stretches `snapshot` to
    /// `to`/`offset` via `PreviewComposer::composite_live_scale`, and
    /// `scale_layer_end` applies the real (undoable) resize exactly once.
    pub(in crate::bridge) live_scale: Option<LiveScaleState>,
    /// Cached preview-scaled base/layers; invalidated by doc.edit_epoch so
    /// live drag ticks skip the per-layer resample.
    pub(in crate::bridge) composer: render::PreviewComposer,
    /// Display size of the last rendered preview. A window resize almost
    /// always shifts the fit zoom by a fraction of a pixel, which would
    /// otherwise re-render (and re-encode the BMP) for a preview identical to
    /// the one already on screen.
    pub(in crate::bridge) last_display: Size,
    /// Result of the last non-special render (see [`CachedRender`]). Pure
    /// zoom ticks above 100% clamp to the same preview target, so the whole
    /// composite + BMP encode + disk write collapses into a URL reuse.
    pub(in crate::bridge) last_render: Option<CachedRender>,
    /// Open "调整颜色" panel: the scope captured when it opened plus the live
    /// parameters. Document pixels are NOT touched until `adjust_commit`.
    pub(in crate::bridge) adjust: Option<AdjustLive>,
    /// Original (width, height) when downsampling-first open shrank this
    /// document. The status bar keeps a persistent hint (refreshes rebuild
    /// the status text, so a one-shot message would be wiped instantly).
    pub(in crate::bridge) downsampled_from: Option<(u32, u32)>,
    /// Decoder hint shown in the status bar (e.g. PSD flattened preview).
    pub(in crate::bridge) note: Option<String>,
    /// Screen device-pixel-ratio (OS scaling, e.g. 1.25 at 125%). Kept on the
    /// session so `refit`/`from_decoded` can compute a correct fit zoom even
    /// before the first `render`. Mirrors the bridge's `dpr` (synced by
    /// `set_dpr`); used by `fit_zoom`/`display_size` to make 100% mean
    /// 1 image px = 1 *physical* px (Photos/PS convention).
    pub(in crate::bridge) dpr: f64,
    /// Active selection (PS-style, canvas coordinates). `None` = nothing
    /// selected. Everything that touches "selected pixels" goes through it.
    pub(in crate::bridge) selection: Option<SelectionState>,
    /// Quick mask (PS's Q): dim everything OUTSIDE the selection in the
    /// PREVIEW only, so the feathered soft edge becomes visible.
    pub(in crate::bridge) quick_mask: bool,
    /// Bumped by every selection mutation; QML re-pulls the outline when the
    /// published copy of it changes. Kept on the session so a plain refresh
    /// never looks like a selection change.
    pub(in crate::bridge) sel_rev: u32,
}

impl Session {
    pub(in crate::bridge) fn clear_selection(&mut self) {
        self.selection = None;
        // 快速蒙版是"看选区软边"的视图态：选区没了就自动关，否则下次
        // 新建选区时会莫名其妙处于蒙版态。
        self.quick_mask = false;
    }

    /// The mask every pixel-consuming command should use (feathered).
    pub(in crate::bridge) fn effective_selection(&self) -> Option<SelectionMask> {
        self.selection.as_ref().map(|s| s.effective())
    }
}

/// Live "调整颜色" state (history-free until the panel commits).
#[derive(Debug, Clone, Copy)]
pub(in crate::bridge) struct AdjustLive {
    pub(in crate::bridge) target: AdjustTarget,
    pub(in crate::bridge) params: Adjust,
}

/// Message from the background decode worker (see `BridgeRust::decode_job`).
pub(in crate::bridge) struct DecodeMsg {
    pub(in crate::bridge) path: PathBuf,
    pub(in crate::bridge) result: Result<(Decoded, i32), String>,
}

/// Longest-edge cap applied when the user enables downsampling-first open.
pub(in crate::bridge) const DOWNSAMPLE_EDGE: u32 = 8192;

/// Session-side state for a live layer resize gesture.
pub(in crate::bridge) struct LiveScaleState {
    pub(in crate::bridge) index: usize,
    pub(in crate::bridge) snapshot: RgbaImage,
    pub(in crate::bridge) to: Size,
    pub(in crate::bridge) offset: (i32, i32),
}

pub(in crate::bridge) struct Rendered {
    pub(in crate::bridge) url: String,
    pub(in crate::bridge) display: Size,
    pub(in crate::bridge) preview: Size,
    /// JSON of the preview histogram (only while the adjust panel is open).
    pub(in crate::bridge) histogram: Option<String>,
}

/// What the preview file on disk currently shows, for the zoom-only skip in
/// [`Session::render`]. `special` marks frames produced under a transient
/// visual state (live color adjust, hidden editing mark, live layer resize):
/// reusing their file would freeze or corrupt the next normal frame, so a
/// special render only becomes reusable after a fresh normal one replaced it.
pub(in crate::bridge) struct CachedRender {
    pub(in crate::bridge) target: Size,
    pub(in crate::bridge) epoch: u64,
    pub(in crate::bridge) url: String,
    pub(in crate::bridge) special: bool,
}

pub(in crate::bridge) enum Refresh {
    Ok {
        url: String,
        canvas: Size,
        display: Size,
        preview: Size,
        zoom_percent: i32,
        fit: bool,
        can_undo: bool,
        can_redo: bool,
        save_url: String,
        layer_count: i32,
        layers_json: String,
        marks_json: String,
        dirty: bool,
        adjust_open: bool,
        adjust_target_name: String,
        adjust_values: String,
        adjust_histogram: String,
    },
    Err(String),
}

impl Session {
    /// Build a session from already-decoded pixels (the decode itself runs
    /// either inline for automation or on the worker thread for the UI).
    pub(in crate::bridge) fn from_decoded(
        path: &Path,
        decoded: Decoded,
        viewport: Size,
        save_quality: u8,
        dpr: f64,
    ) -> (Self, i32) {
        let t0 = Instant::now();
        let doc = Document::from_image(decoded.image);
        let dpr = if dpr.is_finite() && dpr > 0.0 { dpr } else { 1.0 };
        let zoom = fit_zoom(doc.canvas, viewport, dpr);
        let ms = t0.elapsed().as_millis().min(i32::MAX as u128) as i32;
        // A relative CLI path ("photo.jpg") must not leak into the session: the
        // status-bar path, the "save a copy" target and the "open the same file
        // in a new window" flow all need an absolute one. `absolute` keeps the
        // normal form (no \\?\ prefix, unlike canonicalize).
        let source = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        (
            Self {
                id: SESSION_SEQ.fetch_add(1, Ordering::Relaxed),
                doc,
                source: Some(source),
                zoom,
                fit: true,
                render_count: 0,
                dirty: false,
                base_locked: true,
                save_quality,
                preview_hidden_mark: -1,
                live_scale: None,
                composer: render::PreviewComposer::new(),
                last_display: Size::new(0, 0),
                last_render: None,
                adjust: None,
                downsampled_from: decoded.downsampled_from,
                note: decoded.note,
                dpr,
                selection: None,
                quick_mask: false,
                sel_rev: 0,
            },
            ms,
        )
    }

    /// Synchronous open (automation only: --selftest / --script / probes run
    /// script ops immediately after the load, so the document must exist).
    pub(in crate::bridge) fn open(
        path: &Path,
        viewport: Size,
        save_quality: u8,
        opts: DecodeOptions,
        dpr: f64,
    ) -> Result<(Self, i32), String> {
        let t0 = Instant::now();
        let decoded = decode_path(path, opts).map_err(|e| e.to_string())?;
        let decode_ms = t0.elapsed().as_millis().min(i32::MAX as u128) as i32;
        let (session, _) = Self::from_decoded(path, decoded, viewport, save_quality, dpr);
        Ok((session, decode_ms))
    }

    pub(in crate::bridge) fn refit(&mut self, viewport: Size) {
        self.zoom = fit_zoom(self.doc.canvas, viewport, self.dpr);
        self.fit = true;
    }

    pub(in crate::bridge) fn refit_if_needed(&mut self, viewport: Size) {
        if self.fit {
            self.refit(viewport);
        }
    }

    pub(in crate::bridge) fn zoom_by(&mut self, factor: f64) {
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.fit = false;
    }

    /// Rebuild the preview bitmap. `zoom_only` marks a refresh triggered by a
    /// pure zoom change (wheel / slider / presets): when that zoom lands in
    /// the >100% regime the preview target is pinned to the canvas resolution,
    /// so if the previous frame already used that target and the document has
    /// not changed, the existing BMP is still exactly right — reuse its URL
    /// and let the QML Image stretch (sub-millisecond instead of seconds).
    /// Non-zoom refreshes (mark drags, layer ops, live gestures) never skip:
    /// some of them mutate the document without bumping `edit_epoch`.
    pub(in crate::bridge) fn render(&mut self, zoom_only: bool, dpr: f64) -> Result<Rendered, String> {
        let prof_t0 = Instant::now();
        let canvas = self.doc.canvas;
        let display = display_size(canvas, self.zoom, dpr);
        let target = preview_target(display, canvas, dpr);
        if target.pixels() == 0 {
            return Err("empty canvas".into());
        }
        self.last_display = display;
        let special = self.adjust.is_some()
            || self.preview_hidden_mark >= 0
            || self.live_scale.is_some();
        if zoom_only && !special {
            let reusable = self.last_render.as_ref().is_some_and(|c| {
                !c.special && size_close(c.target, target) && c.epoch == self.doc.edit_epoch
            });
            if reusable {
                let url = self.last_render.as_ref().expect("checked").url.clone();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"render-skip\",\"target\":\"{}x{}\"}}",
                    target.width, target.height
                );
                return Ok(Rendered {
                    url,
                    display,
                    preview: target,
                    histogram: None,
                });
            }
        }
        // Inline text editing: the mark being edited is hidden from the
        // preview only (restored right after the composite; never undoable).
        // An in-flight layer resize routes the composite through the
        // live-scale path (snapshot stretch, document untouched). The open
        // "调整颜色" panel applies its live parameters to the scope's
        // preview-resolution bitmap only.
        let adjust = self
            .adjust
            .map(|state| (state.target, state.params));
        let hidden_idx = self.preview_hidden_mark;
        let image = if hidden_idx >= 0
            && (hidden_idx as usize) < self.doc.marks.len()
        {
            let idx = hidden_idx as usize;
            let saved = self.doc.marks[idx].visible;
            self.doc.marks[idx].visible = false;
            let img = self.composite_for_preview(target, adjust);
            self.doc.marks[idx].visible = saved;
            img
        } else {
            self.composite_for_preview(target, adjust)
        };
        let composite_ms = prof_t0.elapsed().as_millis();
        // The panel graph reflects the ADJUSTED preview (like Preview), so it
        // is computed from the composited pixels and only while it is open.
        // It must be sampled BEFORE the quick-mask overlay, which would skew
        // the histogram with the dimming.
        let histogram = if self.adjust.is_some() {
            Some(Histogram::of(&image).to_json())
        } else {
            None
        };
        // Quick mask (Q): dim everything outside the selection, preview only.
        let mut image = image;
        if self.quick_mask {
            apply_quick_mask(&mut image, self.selection.as_ref(), canvas);
        }
        // BMP on purpose: uncompressed means both the encode here and Qt's
        // decode are near memcpy speed. PNG compression dominated each live
        // drag tick (tens of ms); BMP keeps the preview path real-time. The
        // URL keeps changing (forces Qt to re-read) while temp files stay
        // bounded; the session id keeps concurrent windows separate.
        let bytes = encode(&image, Format::Bmp, 90).map_err(|e| e.to_string())?;
        let encode_ms = prof_t0.elapsed().as_millis();
        self.render_count = self.render_count.wrapping_add(1);
        let path = std::env::temp_dir().join(format!(
            "imageedit-preview-{}-{}.bmp",
            self.id,
            self.render_count % 2
        ));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        println!(
            "{{\"phase\":\"ui\",\"event\":\"render-profile\",\"composite\":{},\"encode\":{},\"write\":{},\"target\":\"{}x{}\"}}",
            composite_ms,
            encode_ms - composite_ms,
            prof_t0.elapsed().as_millis(),
            target.width,
            target.height
        );
        // The panel graph reflects the adjusted preview (like Preview), so it
        // is computed from the composited pixels and only while it is open.
        let url = file_url(&path);
        self.last_render = Some(CachedRender {
            target,
            epoch: self.doc.edit_epoch,
            url: url.clone(),
            special,
        });
        Ok(Rendered {
            url,
            display,
            preview: Size::new(image.width(), image.height()),
            histogram,
        })
    }

    /// Composite one preview frame, honoring an in-flight live resize and the
    /// optional color adjustment.
    pub(in crate::bridge) fn composite_for_preview(
        &mut self,
        target: Size,
        adjust: Option<(AdjustTarget, Adjust)>,
    ) -> RgbaImage {
        match self.live_scale.as_ref() {
            Some(ls) => self.composer.composite_live_scale_adjusted(
                &self.doc,
                target,
                ls.index,
                &ls.snapshot,
                ls.to,
                ls.offset,
                adjust,
            ),
            None => self.composer.composite_adjusted(&self.doc, target, adjust),
        }
    }

    /// Scope label for the panel title.
    pub(in crate::bridge) fn adjust_target_name(&self, target: AdjustTarget) -> String {
        match target {
            AdjustTarget::Base => "背景".to_string(),
            AdjustTarget::Layer(index) => self
                .doc
                .layers
                .get(index)
                .map(|layer| layer.name.clone())
                .unwrap_or_else(|| format!("图层 {}", index + 1)),
        }
    }

    /// Scope captured when the panel opens: the selected layer, else the base
    /// photo. Once the base has been "unlocked" into a regular layer the base
    /// bitmap is only transparent padding, so fall back to the bottom layer.
    pub(in crate::bridge) fn adjust_scope(&self, selected_layer: i32) -> AdjustTarget {
        if selected_layer >= 0 && (selected_layer as usize) < self.doc.layers.len() {
            return AdjustTarget::Layer(selected_layer as usize);
        }
        if self.doc.base_converted && !self.doc.layers.is_empty() {
            return AdjustTarget::Layer(0);
        }
        AdjustTarget::Base
    }

    pub(in crate::bridge) fn default_save_path(&self) -> (PathBuf, Format) {
        match &self.source {
            Some(src) => {
                let stem = src
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "image".to_string());
                // A PSD source can't be re-encoded (no PSD writer); default
                // the target to PNG instead of a corrupt "<stem>-edited.psd".
                let format = save_format_for_source(src);
                (
                    src.with_file_name(format!("{stem}-edited.{}", extension_for(format))),
                    format,
                )
            }
            None => (
                std::env::temp_dir().join("imageedit-edited.png"),
                Format::Png,
            ),
        }
    }

    /// Ctrl+S semantics: overwrite the file the document was loaded from,
    /// keeping its original format. Without a source (e.g. a pasted image)
    /// there is nothing to overwrite — QML routes those to the Save-As
    /// dialog instead.
    ///
    /// PSD is preview-only (the decoder flattens layers and image 0.25 has
    /// no PSD encoder): saving back would `unwrap_or(Png)` into PNG bytes
    /// written to the .psd path. Block it with an actionable hint.
    pub(in crate::bridge) fn save(&self) -> Result<PathBuf, String> {
        let src = self.source.clone().ok_or_else(|| "没有可保存的文件".to_string())?;
        if is_psd_path(&src) {
            return Err(PSD_SAVE_HINT.to_string());
        }
        let format = save_format_for_source(&src);
        self.write_to(&src, format)?;
        Ok(src)
    }

    pub(in crate::bridge) fn write_to(&self, path: &Path, format: Format) -> Result<(), String> {
        let bytes = encode(&self.doc.flatten(), format, self.save_quality)
            .map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }

    pub(in crate::bridge) fn layers_json(&self) -> String {
        let (base_deleted, base_locked) = (self.doc.base_deleted, self.base_locked);
        let mut parts: Vec<String> = Vec::new();
        for (i, layer) in self.doc.layers.iter().enumerate().rev() {
            let name = layer.name.replace('\\', "\\\\").replace('"', "\\\"");
            let bounds = layer.bounds();
            // o: opacity in percent (0-100); x/y/w/h: bounds in image pixels
            // (the QML side uses them for the live layer outline while dragging);
            // l: locked (blocks move / scale / delete in the UI).
            parts.push(format!(
                "{{\"i\":{i},\"n\":\"{name}\",\"v\":{},\"o\":{},\"l\":{},\"x\":{},\"y\":{},\"w\":{},\"h\":{}}}",
                layer.visible,
                (layer.opacity * 100.0).round() as i32,
                layer.locked,
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height
            ));
        }
        // The base image is listed last (display order = top first) as a
        // locked-by-default row: not reorderable, not deletable while locked
        // (see §4.8 of the UX plan). v reports visibility (deleted/hidden),
        // bd reports base_deleted so the panel can distinguish "hidden" from
        // "deleted" (Delete key flow) for the base row. Once the base has
        // been converted into a regular layer (panel unlock) the synthetic
        // row disappears entirely — the layer row speaks for itself.
        if !self.doc.base_converted {
            parts.push(format!(
                "{{\"i\":-1,\"n\":\"背景\",\"v\":{},\"bd\":{},\"o\":100,\"l\":{},\"x\":0,\"y\":0,\"w\":{},\"h\":{},\"base\":true}}",
                !base_deleted && !self.doc.base_hidden,
                base_deleted,
                base_locked,
                self.doc.canvas.width,
                self.doc.canvas.height
            ));
        }
        format!("[{}]", parts.join(","))
    }

    /// JSON list of markup annotations for the QML overlay (selection box,
    /// handles, hit-testing): index, kind, bounds, stroke, colors, fill.
    pub(in crate::bridge) fn marks_json(&self) -> String {
        let hex = |c: [u8; 4]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
        let escape = |s: &str| {
            let mut out = String::with_capacity(s.len());
            for ch in s.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c => out.push(c),
                }
            }
            out
        };
        let kind = |k: MarkKind| match k {
            MarkKind::Rect => 0,
            MarkKind::Ellipse => 1,
            MarkKind::Arrow => 2,
            MarkKind::Brush => 3,
            MarkKind::Mosaic => 4,
            MarkKind::Text => 5,
        };
        let parts: Vec<String> = self
            .doc
            .marks
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let (x0, y0, x1, y1) = m.rect;
                let extra = match m.kind {
                    MarkKind::Mosaic => format!(",\"cell\":{}", m.cell),
                    MarkKind::Text => format!(
                        ",\"t\":\"{}\",\"fn\":\"{}\",\"fs\":{},\"b\":{},\"it\":{},\"u\":{},\"al\":{}",
                        escape(&m.text),
                        escape(&m.font),
                        m.font_size,
                        m.bold as u8,
                        m.italic as u8,
                        m.underline as u8,
                        m.align
                    ),
                    _ => String::new(),
                };
                format!(
                    "{{\"i\":{i},\"k\":{},\"x\":{x0:.1},\"y\":{y0:.1},\"w\":{:.1},\"h\":{:.1},\"s\":{},\"c\":\"{}\",\"f\":{},\"fc\":\"{}\",\"v\":{}}}",
                    kind(m.kind),
                    x1 - x0,
                    y1 - y0,
                    m.stroke,
                    hex(m.color),
                    m.fill,
                    hex(m.fill_color),
                    m.visible
                )
                .replace("}", &format!("{}}}", extra))
            })
            .collect();
        format!("[{}]", parts.join(","))
    }

    pub(in crate::bridge) fn refresh(&mut self, zoom_only: bool, dpr: f64) -> Refresh {
        match self.render(zoom_only, dpr) {
            Ok(rendered) => {
                let adjust = self.adjust;
                let adjust_target_name = match adjust {
                    Some(state) => self.adjust_target_name(state.target),
                    None => String::new(),
                };
                Refresh::Ok {
                    url: rendered.url,
                    canvas: self.doc.canvas,
                    display: rendered.display,
                    preview: rendered.preview,
                    zoom_percent: (self.zoom * 100.0).round() as i32,
                    fit: self.fit,
                    can_undo: self.doc.can_undo(),
                    can_redo: self.doc.can_redo(),
                    save_url: file_url(&self.default_save_path().0),
                    layer_count: self.doc.layers.len() as i32,
                    layers_json: self.layers_json(),
                    marks_json: self.marks_json(),
                    dirty: self.dirty,
                    adjust_open: adjust.is_some(),
                    adjust_target_name,
                    adjust_values: adjust_json(
                        &adjust.map(|state| state.params).unwrap_or_default(),
                    ),
                    adjust_histogram: rendered.histogram.unwrap_or_default(),
                }
            }
            Err(err) => Refresh::Err(err),
        }
    }
}

impl Session {
    /// Outline of the current selection as QML reads it: `[[[x,y],...],...]`
    /// in IMAGE coordinates. Downsampled by `step` for big canvases — the
    /// ants are one pixel wide, so tracing 60 MP at full resolution would
    /// cost hundreds of ms for no visible gain.
    pub(in crate::bridge) fn selection_outline_json(&self) -> String {
        let Some(state) = self.selection.as_ref() else {
            return "[]".to_string();
        };
        let m = state.mask();
        if m.is_empty() {
            return "[]".to_string();
        }
        let c = self.doc.canvas;
        let step = ((c.width.max(c.height) as f64 / 1400.0).ceil() as u32).max(1);
        let parts: Vec<String> = m
            .outline(step)
            .iter()
            .map(|ring| {
                let pts: Vec<String> = ring
                    .iter()
                    .map(|(x, y)| format!("[{},{}]", x.round() as i32, y.round() as i32))
                    .collect();
                format!("[{}]", pts.join(","))
            })
            .collect();
        format!("[{}]", parts.join(","))
    }
}

/// Quick-mask overlay (PS's Q): multiply the pixels OUTSIDE the selection
/// toward black so the feathered edge becomes visible. Preview only — the
/// document pixels and the saved file are untouched.
fn apply_quick_mask(image: &mut RgbaImage, sel: Option<&SelectionState>, canvas: Size) {
    let Some(state) = sel else {
        return;
    };
    let mask = state.mask();
    if mask.is_empty() || canvas.width == 0 || canvas.height == 0 {
        return;
    }
    let sw = image.width() as f64 / canvas.width as f64;
    let sh = image.height() as f64 / canvas.height as f64;
    if sw <= 0.0 || sh <= 0.0 {
        return;
    }
    for y in 0..image.height() {
        let cy = (y as f64 / sh) as i32;
        for x in 0..image.width() {
            let cx = (x as f64 / sw) as i32;
            let cov = mask.coverage_at(cx, cy);
            if cov >= 255 {
                continue;
            }
            let dim = 1.0 - 0.55 * (1.0 - cov as f64 / 255.0);
            let p = image.get_pixel_mut(x, y);
            for i in 0..3 {
                p.0[i] = (p.0[i] as f64 * dim).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}
