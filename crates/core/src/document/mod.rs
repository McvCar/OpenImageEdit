// crates/core/src/document/mod.rs
use super::*;

#[derive(Debug, Clone)]
pub struct Document {
    pub source: Option<std::path::PathBuf>,
    pub canvas: Size,
    pub base: RgbaImage,
    /// Visible region of `base`, in `base` coordinates. Cropping only shrinks
    /// this rect (no pixel copy), so crop is O(1) and reversible.
    pub view: Rect,
    pub layers: Vec<Layer>,
    /// True while the base layer has been "deleted" (fully transparent
    /// canvas). Tracked so the panel can grey out the base row.
    pub base_deleted: bool,
    /// True while the base layer is hidden by the user (eye toggle in the
    /// panel). Non-destructive and undoable.
    pub base_hidden: bool,
    /// True once the base image has been converted into a regular layer
    /// ("unlock" in the panel): it becomes movable / scalable / deletable
    /// like any other layer, and the synthetic base row disappears. The
    /// base field stays as a transparent canvas of the same size so crop /
    /// rotate / canvas-resize keep working unchanged.
    pub base_converted: bool,
    /// Vector markup annotations (rect / ellipse / arrow / brush / …) drawn
    /// on top of the composite at render time. Canvas-space coordinates.
    pub marks: Vec<Mark>,
    /// Active selection (PS-style alpha coverage map, canvas coordinates).
    /// Lives here instead of on the UI session so Ctrl+Z walks it back in the
    /// same history as pixel edits. `None` = nothing selected.
    pub selection: Option<SelectionState>,
    /// Monotonic counter bumped by every committed / structural change
    /// (execute, undo, redo). Preview caches key on it: any real edit
    /// invalidates them, while history-free live mutations (layer offset
    /// during a drag, live mark moves) deliberately do NOT bump it so each
    /// drag tick can reuse cached scaled layers.
    pub edit_epoch: u64,
    /// Auto-naming counter: new layers (extract/paste) are named by their
    /// creation order ("1", "2", ...). Not part of snapshots on purpose, so
    /// names keep increasing across undo/redo.
    pub(crate) next_layer_seq: u32,
    pub(crate) history: History,
}


/// Full document state taken before a pixel-modifying op, so undo can restore
/// it exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSnapshot {
    pub canvas: Size,
    pub base: RgbaImage,
    pub view: Rect,
    pub layers: Vec<Layer>,
    /// True while the base has been "deleted" (fully transparent canvas).
    pub base_deleted: bool,
    /// Base hidden by the user (eye toggle).
    pub base_hidden: bool,
    /// Base already converted into a regular layer (panel unlock).
    pub base_converted: bool,
    /// Vector markup annotations.
    pub marks: Vec<Mark>,
}

impl Document {
    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            canvas: self.canvas,
            base: self.base.clone(),
            view: self.view,
            layers: self.layers.clone(),
            base_deleted: self.base_deleted,
            base_hidden: self.base_hidden,
            base_converted: self.base_converted,
            marks: self.marks.clone(),
        }
    }

    /// Replace everything except history (used when undoing a pixel op).
    pub fn restore(&mut self, snapshot: DocumentSnapshot) {
        self.canvas = snapshot.canvas;
        self.base = snapshot.base;
        self.view = snapshot.view;
        self.layers = snapshot.layers;
        self.base_deleted = snapshot.base_deleted;
        self.base_hidden = snapshot.base_hidden;
        self.base_converted = snapshot.base_converted;
        self.marks = snapshot.marks;
    }
}



impl Document {
    pub fn from_image(image: RgbaImage) -> Self {
        let canvas = Size::new(image.width(), image.height());
        Self {
            source: None,
            canvas,
            base: image,
            view: Rect::new(0, 0, canvas.width, canvas.height),
            layers: Vec::new(),
            marks: Vec::new(),
            selection: None,
            base_deleted: false,
            base_hidden: false,
            base_converted: false,
            edit_epoch: 0,
            next_layer_seq: 0,
            history: History::new(256),
        }
    }

    pub fn execute(&mut self, op: EditOp) -> Result<(), EditError> {
        op.apply(self)?;
        self.history.push(op);
        self.edit_epoch = self.edit_epoch.wrapping_add(1);
        Ok(())
    }

    // ---- 选区（可撤销的文档状态，见 `EditOp::SetSelection`）-------------

    /// 换一个选区（新建 / 反选 / 取消 / 平移 / 缩放）。与当前完全相同则
    /// 不产生历史条目。
    pub fn set_selection(&mut self, next: Option<SelectionState>) -> Result<(), EditError> {
        if self.selection == next {
            return Ok(());
        }
        let from = self.selection.clone();
        self.execute_selection(EditOp::SetSelection { from, to: next })
    }

    /// 改边界 / 平滑 / 羽化参数（从 base 重算，不叠加）。
    pub fn set_selection_ops(&mut self, ops: SelOps) -> Result<(), EditError> {
        let Some(state) = self.selection.as_ref() else {
            return Ok(());
        };
        let from = state.ops;
        if from == ops {
            return Ok(());
        }
        self.execute_selection(EditOp::SelectionOptions { from, to: ops })
    }

    /// 选区不碰任何一个像素，所以**不 bump `edit_epoch`**：预览合成器的
    /// 分层缓存按 epoch 失效，不该因为拖了一下选区就整块重新重采样。
    fn execute_selection(&mut self, op: EditOp) -> Result<(), EditError> {
        op.apply(self)?;
        self.history.push(op);
        Ok(())
    }

    /// Crop to `rect`, given in canvas coordinates. The rect is clamped to the
    /// current canvas; a rect that falls outside it is rejected. Use
    /// [`Document::reset_crop`] to reveal the full base image again.
    pub fn crop(&mut self, rect: Rect) -> Result<(), EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let clamped = rect.intersect(canvas_rect).ok_or(EditError::EmptyCrop)?;
        if clamped.is_empty() {
            return Err(EditError::EmptyCrop);
        }
        self.execute(EditOp::CropDocument {
            from: self.view,
            to: Rect::new(
                self.view.x + clamped.x,
                self.view.y + clamped.y,
                clamped.width,
                clamped.height,
            ),
        })
    }

    /// Crop to the selection's **outline** (not just its bounding box): the
    /// canvas shrinks to the bounds AND every pixel outside the outline fades
    /// to transparent (scaled by the mask's coverage, so feathered edges stay
    /// soft). Ellipse / lasso / feathered selections therefore crop to their
    /// own shape instead of leaving a hard rectangular cut. Base and layers
    /// are both punched; undo restores the whole snapshot.
    pub fn crop_to_selection(&mut self, selection: &SelectionMask) -> Result<(), EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let area = selection
            .bounds
            .intersect(canvas_rect)
            .ok_or(EditError::EmptyCrop)?;
        if area.is_empty() || selection.area() == 0 {
            return Err(EditError::EmptyCrop);
        }
        let from = self.snapshot();
        self.execute(EditOp::CropToSelection {
            from,
            selection: selection.clone(),
        })
    }

    /// Undo every crop, showing the full base image again.
    pub fn reset_crop(&mut self) -> Result<(), EditError> {
        if self.view.x == 0
            && self.view.y == 0
            && self.view.width == self.base.width()
            && self.view.height == self.base.height()
        {
            return Ok(());
        }
        self.execute(EditOp::CropDocument {
            from: self.view,
            to: Rect::new(0, 0, self.base.width(), self.base.height()),
        })
    }

    pub fn apply_transform(&mut self, transform: Transform) -> Result<(), EditError> {
        self.execute(EditOp::TransformDocument { transform })
    }

    /// Resize the document to exactly `to`: the current canvas (what the user
    /// sees) is resampled (Lanczos3) into a new base, any crop is dropped, and
    /// layers are scaled proportionally. Full previous state is snapshotted for
    /// undo.
    pub fn resize(&mut self, to: Size) -> Result<(), EditError> {
        if to.width < 1 || to.height < 1 {
            return Err(EditError::InvalidCanvasSize);
        }
        let from = self.snapshot();
        self.execute(EditOp::ResizeDocument { from, to })
    }

    /// Change only the canvas resolution: no pixel is resampled. Content is
    /// re-anchored to the center of the new canvas (see `apply_canvas_resize`).
    pub fn resize_canvas(&mut self, to: Size) -> Result<(), EditError> {
        if to.width < 1 || to.height < 1 {
            return Err(EditError::InvalidCanvasSize);
        }
        let from = self.snapshot();
        self.execute(EditOp::CanvasResize { from, to })
    }

    pub fn add_layer(&mut self, layer: Layer) -> Result<usize, EditError> {
        let index = self.layers.len();
        self.execute(EditOp::AddLayer { layer, index })?;
        Ok(index)
    }

    /// Append a markup annotation (drawn on top of the composite). Returns
    /// its index (creation order, topmost last).
    pub fn add_mark(&mut self, mark: Mark) -> Result<usize, EditError> {
        let index = self.marks.len();
        self.execute(EditOp::AddMark { index, mark })?;
        Ok(index)
    }

    /// Replace a markup annotation (move / resize / restyle), undoable.
    pub fn update_mark(&mut self, index: usize, to: Mark) -> Result<(), EditError> {
        let from = self
            .marks
            .get(index)
            .cloned()
            .ok_or(EditError::MarkIndex(index))?;
        self.execute(EditOp::UpdateMark { index, from, to })
    }

    /// History-free live preview of [`Document::update_mark`] for handle
    /// drags; the committing call records the true pre-drag mark.
    pub fn update_mark_live(&mut self, index: usize, mark: Mark) -> Result<(), EditError> {
        let m = self
            .marks
            .get_mut(index)
            .ok_or(EditError::MarkIndex(index))?;
        *m = mark;
        Ok(())
    }

    /// Remove a markup annotation, undoable.
    pub fn delete_mark(&mut self, index: usize) -> Result<(), EditError> {
        let mark = self
            .marks
            .get(index)
            .cloned()
            .ok_or(EditError::MarkIndex(index))?;
        self.execute(EditOp::DeleteMark { index, mark })
    }

    pub fn remove_layer(&mut self, index: usize) -> Result<(), EditError> {
        let layer = self
            .layers
            .get(index)
            .cloned()
            .ok_or(EditError::LayerIndex(index))?;
        self.execute(EditOp::RemoveLayer { layer, index })
    }

    pub fn move_layer(&mut self, from: usize, to: usize) -> Result<(), EditError> {
        self.execute(EditOp::MoveLayer { from, to })
    }

    pub fn set_layer_visible(&mut self, index: usize, visible: bool) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .visible;
        self.execute(EditOp::SetVisible {
            index,
            from,
            to: visible,
        })
    }

    pub fn set_layer_opacity(&mut self, index: usize, opacity: f32) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .opacity;
        self.execute(EditOp::SetOpacity {
            index,
            from,
            to: opacity.clamp(0.0, 1.0),
        })
    }

    pub fn translate_layer(&mut self, index: usize, dx: i32, dy: i32) -> Result<(), EditError> {
        let layer = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?;
        let from = layer.offset;
        let to = (from.0 + dx, from.1 + dy);
        self.execute(EditOp::TranslateLayer { index, from, to })
    }

    pub fn rename_layer(&mut self, index: usize, name: impl Into<String>) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .name
            .clone();
        self.execute(EditOp::RenameLayer {
            index,
            from,
            to: name.into(),
        })
    }

    /// Scale a layer's image to exactly `to` (Lanczos3) and place its
    /// top-left corner at `offset`, so handle-driven resizes anchor at the
    /// dragged corner / edge. Undo restores the full previous state via
    /// snapshot.
    pub fn scale_layer(
        &mut self,
        index: usize,
        to: Size,
        offset: (i32, i32),
    ) -> Result<(), EditError> {
        self.scale_layer_flipped(index, to, offset, (false, false))
    }

    /// `scale_layer` plus mirroring: `flip` mirrors the resampled bitmap on
    /// that axis. The UI passes `true` when a handle was dragged past the
    /// opposite edge (negative width/height), which is how "resize to a
    /// negative size" is expressed here — `Size` stays unsigned.
    pub fn scale_layer_flipped(
        &mut self,
        index: usize,
        to: Size,
        offset: (i32, i32),
        flip: (bool, bool),
    ) -> Result<(), EditError> {
        if to.width < 1 || to.height < 1 {
            return Err(EditError::InvalidCanvasSize);
        }
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        let from = self.snapshot();
        self.execute(EditOp::ScaleLayer {
            index,
            from,
            to,
            offset,
            flip,
        })
    }

    /// Flip / rotate one layer in place (its box — offset and image size
    /// except for 90° rotations — stays where it is). This is the PS
    /// behaviour when a layer is selected: the transform hits that layer
    /// only, not the whole document.
    pub fn transform_layer(&mut self, index: usize, transform: Transform) -> Result<(), EditError> {
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        self.execute(EditOp::TransformLayer { index, transform })
    }

    /// "Delete" the base layer: the canvas becomes fully transparent while
    /// layers stay untouched. Requires at least one layer to remain.
    pub fn delete_base(&mut self) -> Result<(), EditError> {
        if self.layers.is_empty() {
            return Err(EditError::BaseRequired);
        }
        let from = self.snapshot();
        self.execute(EditOp::DeleteBase { from })
    }

    /// Hide/show the base layer. No-op (no history entry) when the state
    /// already matches.
    pub fn set_base_visible(&mut self, visible: bool) -> Result<(), EditError> {
        let from = self.base_hidden;
        let to = !visible;
        if from == to {
            return Ok(());
        }
        self.execute(EditOp::SetBaseVisible { from, to })
    }

    /// "Unlock" the base layer: convert it into a regular layer at the
    /// bottom of the stack so it can be moved / scaled / deleted / renamed
    /// like any other layer. Idempotent: converting twice is a no-op (the
    /// second call adds no history entry).
    pub fn convert_base_to_layer(&mut self) -> Result<(), EditError> {
        if self.base_converted {
            return Ok(());
        }
        let from = self.snapshot();
        self.execute(EditOp::BaseToLayer { from })
    }

    pub fn undo(&mut self) -> Result<(), EditError> {
        let op = self.history.undo.pop().ok_or(EditError::NothingToUndo)?;
        op.revert(self)?;
        self.history.redo.push(op);
        self.edit_epoch = self.edit_epoch.wrapping_add(1);
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), EditError> {
        let op = self.history.redo.pop().ok_or(EditError::NothingToRedo)?;
        op.apply(self)?;
        self.history.undo.push(op);
        self.edit_epoch = self.edit_epoch.wrapping_add(1);
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    pub fn selection_layer(&self, selection: &SelectionMask) -> Result<Layer, EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let bounds = selection
            .bounds
            .intersect(canvas_rect)
            .ok_or(EditError::EmptySelection)?;
        if bounds.is_empty() || selection.area() == 0 {
            return Err(EditError::EmptySelection);
        }
        let mut image = RgbaImage::new(bounds.width, bounds.height);
        let mut any = false;
        for y in 0..bounds.height {
            for x in 0..bounds.width {
                let sx = bounds.x + x as i32;
                let sy = bounds.y + y as i32;
                let cov = selection.coverage_at(sx, sy);
                if cov == 0 {
                    continue;
                }
                any = true;
                // The selection is an alpha coverage map: feathered edges
                // come out of the extracted layer already semi-transparent.
                let mut px = self.composite_pixel(sx, sy);
                px.0[3] = ((px.0[3] as u32 * cov as u32 + 127) / 255) as u8;
                image.put_pixel(x, y, px);
            }
        }
        if !any {
            return Err(EditError::EmptySelection);
        }
        let mut layer = Layer::new("Selection", image);
        layer.offset = (bounds.x, bounds.y);
        Ok(layer)
    }

    /// Next auto name for a created layer ("1", "2", ...), by creation order.
    pub fn next_layer_name(&mut self) -> String {
        self.next_layer_seq += 1;
        self.next_layer_seq.to_string()
    }

    pub fn extract_selection(&mut self, selection: &SelectionMask) -> Result<usize, EditError> {
        let mut layer = self.selection_layer(selection)?;
        layer.name = self.next_layer_name();
        let index = self.layers.len();
        self.execute(EditOp::AddLayer { layer, index })?;
        Ok(index)
    }

    /// Non-destructive "delete selection pixels": base pixels under the
    /// selection become transparent (scaled by the selection's coverage, so a
    /// feathered edge fades out instead of being cut); layers are untouched;
    /// undo restores the exact previous state via snapshot.
    pub fn erase_selection(&mut self, selection: &SelectionMask) -> Result<(), EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let clipped = selection
            .bounds
            .intersect(canvas_rect)
            .ok_or(EditError::EmptySelection)?;
        if clipped.is_empty() || selection.area() == 0 {
            return Err(EditError::EmptySelection);
        }
        let from = self.snapshot();
        self.execute(EditOp::EraseBasePixels {
            from,
            selection: selection.clone(),
        })
    }

    /// Pixels a color adjustment would touch (histogram source / live scope).
    pub fn scope_image(&self, target: AdjustTarget) -> Option<&RgbaImage> {
        match target {
            AdjustTarget::Base => Some(&self.base),
            AdjustTarget::Layer(index) => self.layers.get(index).map(|layer| &layer.image),
        }
    }

    /// Histogram of the adjustment scope ("调整颜色" panel graph + auto levels).
    pub fn scope_histogram(&self, target: AdjustTarget) -> Option<Histogram> {
        self.scope_image(target).map(Histogram::of)
    }

    /// Bake `params` into the scope's pixels as one undoable operation.
    /// A neutral parameter set is a no-op (no history entry, no epoch bump).
    pub fn adjust_pixels(
        &mut self,
        target: AdjustTarget,
        params: &Adjust,
    ) -> Result<(), EditError> {
        let from = match target {
            AdjustTarget::Base => self.base.clone(),
            AdjustTarget::Layer(index) => self
                .layers
                .get(index)
                .map(|layer| layer.image.clone())
                .ok_or(EditError::LayerIndex(index))?,
        };
        if params.is_identity() {
            return Ok(());
        }
        self.execute(EditOp::AdjustPixels {
            target,
            from,
            params: *params,
        })
    }

    pub fn composite_pixel(&self, x: i32, y: i32) -> image::Rgba<u8> {
        if x < 0 || y < 0 || x >= self.canvas.width as i32 || y >= self.canvas.height as i32 {
            return image::Rgba([0, 0, 0, 0]);
        }
        let bx = (self.view.x + x) as u32;
        let by = (self.view.y + y) as u32;
        let mut out = if !self.base_hidden
            && bx < self.base.width()
            && by < self.base.height()
        {
            self.base.get_pixel(bx, by).0
        } else {
            [0, 0, 0, 0]
        };
        for layer in &self.layers {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            let lx = x - layer.offset.0;
            let ly = y - layer.offset.1;
            if lx < 0
                || ly < 0
                || lx >= layer.image.width() as i32
                || ly >= layer.image.height() as i32
            {
                continue;
            }
            let src = layer.image.get_pixel(lx as u32, ly as u32).0;
            let a = (src[3] as f32 / 255.0) * layer.opacity;
            if a <= 0.0 {
                continue;
            }
            let inv = 1.0 - a;
            for c in 0..3 {
                out[c] = (src[c] as f32 * a + out[c] as f32 * inv).round() as u8;
            }
            out[3] = ((a + out[3] as f32 / 255.0 * inv) * 255.0).round() as u8;
        }
        image::Rgba(out)
    }

    pub fn flatten(&self) -> RgbaImage {
        let mut out = RgbaImage::new(self.canvas.width, self.canvas.height);
        for y in 0..self.canvas.height {
            for x in 0..self.canvas.width {
                out.put_pixel(x, y, self.composite_pixel(x as i32, y as i32));
            }
        }
        // Markup annotations always sit on top of every layer.
        for mark in &self.marks {
            if mark.visible {
                mark.draw(&mut out);
            }
        }
        out
    }
}

