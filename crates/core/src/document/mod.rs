// crates/core/src/document/mod.rs
use super::*;

#[derive(Debug, Clone)]
pub struct Document {
    pub source: Option<std::path::PathBuf>,
    pub canvas: Size,
    /// 裁剪窗口：`view` 是背景图里当前可见的那一块（`layers[0]` 保存完整
    /// 像素，offset 恒为 `(-view.x, -view.y)`）。只用于「重置裁剪」与
    /// 坐标换算，不参与合成。
    pub view: Rect,
    /// 图层栈：索引 0 = 最底（背景），`len-1` = 最顶。
    /// 背景层 `background == true`、默认锁定；矢量标记作为 `LayerContent::Mark`
    /// 与位图图层同栈、同排序、同可见性 / 锁定 / 不透明度。
    pub layers: Vec<Layer>,
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
    pub view: Rect,
    pub layers: Vec<Layer>,
}

impl Document {
    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            canvas: self.canvas,
            view: self.view,
            layers: self.layers.clone(),
        }
    }

    /// Replace everything except history (used when undoing a pixel op).
    pub fn restore(&mut self, snapshot: DocumentSnapshot) {
        self.canvas = snapshot.canvas;
        self.view = snapshot.view;
        self.layers = snapshot.layers;
    }

    // ---- 背景层访问（`layers[0]`，恒存在）----------------------------

    pub fn background(&self) -> &Layer {
        &self.layers[0]
    }

    pub fn background_mut(&mut self) -> &mut Layer {
        &mut self.layers[0]
    }

    pub fn base_image(&self) -> &RgbaImage {
        self.layers[0].image().expect("background layer is raster")
    }

    pub fn base_image_mut(&mut self) -> &mut RgbaImage {
        self.layers[0]
            .image_mut()
            .expect("background layer is raster")
    }

    /// 覆盖背景像素并保持它对齐 `view`（不做额外位移）。
    pub fn set_base_image(&mut self, image: RgbaImage) {
        if let Some(r) = self.layers[0].raster_mut() {
            r.image = image;
            r.transform = None;
            r.offset = (-self.view.x, -self.view.y);
        }
    }

    pub fn base_visible(&self) -> bool {
        self.layers[0].visible
    }

    pub fn base_hidden(&self) -> bool {
        !self.layers[0].visible
    }

    /// 背景像素是否已全部透明（面板显示"已删除"用）。
    ///
    /// 这是**内容**判断而不是结构状态：没有单独标志位，避免出现「标志说
    /// 删了、像素还在」的不一致。只用于面板展示，按行抽样（每 16 行一次）
    /// 以避开整幅扫描。
    pub fn base_deleted(&self) -> bool {
        let Some(img) = self.layers[0].image() else {
            return true;
        };
        let step = (img.height() / 64).max(1);
        let mut y = 0;
        while y < img.height() {
            for x in (0..img.width()).step_by(16) {
                if img.get_pixel(x, y).0[3] != 0 {
                    return false;
                }
            }
            y += step;
        }
        true
    }

    pub fn base_converted(&self) -> bool {
        !self.layers[0].locked
    }
}




impl Document {
    pub fn from_image(image: RgbaImage) -> Self {
        let canvas = Size::new(image.width(), image.height());
        Self {
            source: None,
            canvas,
            view: Rect::new(0, 0, canvas.width, canvas.height),
            layers: vec![Layer::background(image)],
            selection: None,
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

    /// 拖动类工具（快速选择 / 笔刷选区）的临时选区：只改状态，**不进
    /// 历史、不 bump epoch**。
    ///
    /// 这类工具拖动期间每帧都要重算一次选区，若每帧都走 `set_selection`，
    /// 一次涂抹就会塞进几十条撤销记录（Ctrl+Z 要按到手酸）。松开时用
    /// [`Document::commit_selection_change`] 把整段操作补成一条。
    pub fn preview_selection(&mut self, next: Option<SelectionState>) {
        self.selection = next;
    }

    /// 把一次已经生效的选区变更记进历史，与 `preview_selection` 配对。
    pub fn commit_selection_change(&mut self, from: Option<SelectionState>) {
        let to = self.selection.clone();
        if from == to {
            return;
        }
        self.history.push(EditOp::SetSelection { from, to });
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
        let (bw, bh) = self
            .layers[0]
            .image()
            .map(|img| (img.width(), img.height()))
            .unwrap_or((self.canvas.width, self.canvas.height));
        if self.view.x == 0 && self.view.y == 0 && self.view.width == bw && self.view.height == bh {
            return Ok(());
        }
        self.execute(EditOp::CropDocument {
            from: self.view,
            to: Rect::new(0, 0, bw, bh),
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

    /// Append a markup annotation as a new topmost layer. Returns the mark's
    /// ordinal among marks (0-based, z order) — the same numbering the panel
    /// and the `mark-*` script ops use.
    pub fn add_mark(&mut self, mark: Mark) -> Result<usize, EditError> {
        let ordinal = self.mark_count();
        let layer = Layer::from_mark(mark);
        self.execute(EditOp::AddLayer {
            layer,
            index: self.layers.len(),
        })?;
        Ok(ordinal)
    }

    /// Replace a markup annotation (move / resize / restyle), undoable.
    /// `index` is the **mark ordinal** (see [`Document::mark_layer_index`]).
    pub fn update_mark(&mut self, index: usize, to: Mark) -> Result<(), EditError> {
        let li = self
            .mark_layer_index(index)
            .ok_or(EditError::MarkIndex(index))?;
        let from = self.layers[li]
            .mark()
            .cloned()
            .ok_or(EditError::MarkIndex(index))?;
        self.execute(EditOp::UpdateMark {
            index: li,
            from,
            to,
        })
    }

    /// History-free live preview of [`Document::update_mark`] for handle
    /// drags; the committing call records the true pre-drag mark.
    pub fn update_mark_live(&mut self, index: usize, mark: Mark) -> Result<(), EditError> {
        let li = self
            .mark_layer_index(index)
            .ok_or(EditError::MarkIndex(index))?;
        match &mut self.layers[li].content {
            LayerContent::Mark(m) => {
                *m = mark;
                Ok(())
            }
            LayerContent::Raster(_) => Err(EditError::MarkIndex(index)),
        }
    }

    /// Remove a markup annotation, undoable.
    pub fn delete_mark(&mut self, index: usize) -> Result<(), EditError> {
        let li = self
            .mark_layer_index(index)
            .ok_or(EditError::MarkIndex(index))?;
        let layer = self.layers[li].clone();
        self.execute(EditOp::RemoveLayer {
            layer,
            index: li,
        })
    }

    // ---- 标记视图（标记现在是图层，这里按 z 序暴露序号 ↔ 图层下标）------

    /// 全部标记图层的图层下标，按 z 序（底 → 顶）。
    pub fn mark_layer_indices(&self) -> Vec<usize> {
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.is_mark())
            .map(|(i, _)| i)
            .collect()
    }

    pub fn mark_count(&self) -> usize {
        self.layers.iter().filter(|l| l.is_mark()).count()
    }

    /// 第 `k` 个标记（z 序）对应的图层下标。
    pub fn mark_layer_index(&self, k: usize) -> Option<usize> {
        self.mark_layer_indices().get(k).copied()
    }

    pub fn mark(&self, k: usize) -> Option<&Mark> {
        self.mark_layer_index(k)
            .and_then(|i| self.layers[i].mark())
    }

    pub fn mark_mut(&mut self, k: usize) -> Option<&mut Mark> {
        let i = self.mark_layer_index(k)?;
        self.layers[i].mark_mut()
    }

    pub fn marks(&self) -> impl Iterator<Item = &Mark> {
        self.layers.iter().filter_map(|l| l.mark())
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

    pub fn set_layer_locked(&mut self, index: usize, locked: bool) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .locked;
        if from == locked {
            return Ok(());
        }
        self.execute(EditOp::SetLocked {
            index,
            from,
            to: locked,
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

    /// P0-2b2：移动也汇流到一条 `SetLayerTransform`。旧 `TranslateLayer`
    /// 只写 `layer.offset`，会让 params.center 与位图脱节（连续旋转 / 数值
    /// 面板随后读到错误中心）；参数级历史 undo 还能精确还原尺寸与角度。
    pub fn translate_layer(&mut self, index: usize, dx: i32, dy: i32) -> Result<(), EditError> {
        if dx == 0 && dy == 0 {
            return Ok(());
        }
        let layer = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?;
        if layer.is_mark() {
            // 标记没有 offset，直接平移几何。
            let from = layer.offset();
            let to = (from.0 + dx, from.1 + dy);
            return self.execute(EditOp::TranslateLayer { index, from, to });
        }
        let r = layer.raster().ok_or(EditError::LayerIndex(index))?;
        let mut to = r.transform.clone().unwrap_or_else(|| {
            LayerTransform::from_offset(r.image.clone(), r.offset.0, r.offset.1)
        });
        to.center.0 += dx as f64;
        to.center.1 += dy as f64;
        self.set_layer_transform(index, Some(to))
    }

    /// P0-1 参数化变换入口。from/to 只带参数与 `Arc<原图>`，一条历史即可
    /// 精确撤销；P0-2 的统一会话会把移动 / 缩放 / 旋转全部汇流到这里。
    /// 传 `None` = 把该层烘焙成普通位图图层（丢弃无损状态）。
    pub fn set_layer_transform(
        &mut self,
        index: usize,
        to: Option<LayerTransform>,
    ) -> Result<(), EditError> {
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        // 「变换前是普通位图图层」也必须留下可恢复的状态，否则 undo 时
        // 没有位图可还原（只有 None 会让 image 停在变换后的尺寸）。用当前
        // image 建一个 identity 状态，撤销走 1:1 索引快路径精确还原。
        let from = match self.layers[index].raster() {
            Some(r) => r.transform.clone().unwrap_or_else(|| {
                LayerTransform::from_offset(r.image.clone(), r.offset.0, r.offset.1)
            }),
            None => return Err(EditError::LayerIndex(index)),
        };
        if to.as_ref() == Some(&from) {
            return Ok(());
        }
        self.execute(EditOp::SetLayerTransform {
            index,
            from: Some(from),
            to,
        })
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

    /// "Delete" the background: its pixels become fully transparent while the
    /// layer entries (and therefore the layer stack) stay untouched.
    pub fn delete_base(&mut self) -> Result<(), EditError> {
        let from = self.snapshot();
        self.execute(EditOp::DeleteBase { from })
    }

    /// Hide/show the background layer. No-op (no history entry) when the
    /// state already matches.
    pub fn set_base_visible(&mut self, visible: bool) -> Result<(), EditError> {
        if self.layers[0].visible == visible {
            return Ok(());
        }
        self.execute(EditOp::SetBaseVisible {
            from: !visible,
            to: visible,
        })
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

    /// 已提交历史的条数。旋转会话用它判断「图层在这之后有没有被别的操作
    /// 改过」—— 改过就不能再拿会话开头那份原图继续转，否则会吃掉别人的修改。
    pub fn history_len(&self) -> usize {
        self.history.undo.len()
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
        layer.set_offset((bounds.x, bounds.y));
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

    /// 选区像素的取样源由 `target` 决定：`Base` = 背景，`Layer(i)` = 第 i 层。
    ///
    /// 和 [`Document::selection_layer`] 的区别：那个从**可见合成**里抠（合并
    /// 拷贝），这里只取单层像素。PS 的复制 / 剪切 / 提取默认只作用于当前
    /// 图层 —— 否则选区一抠就把上层的内容也带进来。图层可能只覆盖画布的
    /// 一部分，所以产出范围 = 选区包围盒 ∩ 图层矩形。
    pub fn selection_layer_on(
        &self,
        target: AdjustTarget,
        selection: &SelectionMask,
    ) -> Result<Layer, EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let mut bounds = selection
            .bounds
            .intersect(canvas_rect)
            .ok_or(EditError::EmptySelection)?;
        if let AdjustTarget::Layer(index) = target {
            let r = self
                .layers
                .get(index)
                .and_then(|l| l.raster())
                .ok_or(EditError::LayerIndex(index))?;
            let layer_rect = Rect::new(
                r.offset.0,
                r.offset.1,
                r.image.width(),
                r.image.height(),
            );
            bounds = bounds
                .intersect(layer_rect)
                .ok_or(EditError::EmptySelection)?;
        }
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
                let mut px = match target {
                    AdjustTarget::Base => self.base_pixel_at(sx, sy),
                    AdjustTarget::Layer(index) => self.layer_pixel(index, sx, sy),
                };
                // 图层在这个位置本来就是透明的：留透明，别把一个"看得见空
                // 洞"的图层抠出来。
                if px[3] == 0 {
                    continue;
                }
                any = true;
                px[3] = ((px[3] as u32 * cov as u32 + 127) / 255) as u8;
                image.put_pixel(x, y, image::Rgba(px));
            }
        }
        if !any {
            return Err(EditError::EmptySelection);
        }
        let mut layer = Layer::new("Selection", image);
        layer.set_offset((bounds.x, bounds.y));
        Ok(layer)
    }

    /// 把选区像素抠成新图层，取样源见 [`Document::selection_layer_on`]。
    pub fn extract_selection_on(
        &mut self,
        target: AdjustTarget,
        selection: &SelectionMask,
    ) -> Result<usize, EditError> {
        let mut layer = self.selection_layer_on(target, selection)?;
        layer.name = self.next_layer_name();
        let index = self.layers.len();
        self.execute(EditOp::AddLayer { layer, index })?;
        Ok(index)
    }

    /// 「删除选区像素」，只挖 `target` 那一层：base 走整份快照的
    /// `EraseBasePixels`，图层走一块像素补丁（前后两份位图各留一份，
    /// 撤销/重做都不必复制整层），覆盖率缩放 alpha，羽化边自然淡化。
    /// 锁定图层不动（调用方负责提示）。
    pub fn erase_selection_on(
        &mut self,
        target: AdjustTarget,
        selection: &SelectionMask,
    ) -> Result<(), EditError> {
        let AdjustTarget::Layer(index) = target else {
            return self.erase_selection(selection);
        };
        let layer = self.layers.get(index).ok_or(EditError::LayerIndex(index))?;
        if layer.locked {
            return Err(EditError::LayerIndex(index));
        }
        let r = layer.raster().ok_or(EditError::LayerIndex(index))?;
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let layer_rect = Rect::new(
            r.offset.0,
            r.offset.1,
            r.image.width(),
            r.image.height(),
        );
        let area = selection
            .bounds
            .intersect(canvas_rect)
            .and_then(|r| r.intersect(layer_rect))
            .ok_or(EditError::EmptySelection)?;
        if area.is_empty() || selection.area() == 0 {
            return Err(EditError::EmptySelection);
        }
        let lx = area.x - r.offset.0;
        let ly = area.y - r.offset.1;
        let (rect, before) = crop_region(
            &r.image,
            Rect::new(lx, ly, area.width, area.height),
        );
        if rect.is_empty() {
            return Err(EditError::EmptySelection);
        }
        let mut after = before.clone();
        for y in 0..rect.height {
            for x in 0..rect.width {
                let cov = selection.coverage_at(area.x + x as i32, area.y + y as i32);
                if cov == 0 {
                    continue;
                }
                let px = after.get_pixel_mut(x, y);
                px.0[3] = ((px.0[3] as u32 * (255 - cov as u32) + 127) / 255) as u8;
                if px.0[3] == 0 {
                    px.0 = [0, 0, 0, 0];
                }
            }
        }
        self.commit_patch(target, rect, before, after)
    }


    /// Pixels a color adjustment would touch (histogram source / live scope).
    /// `AdjustTarget::Base` 恒指背景层（`layers[0]`）。
    pub fn scope_image(&self, target: AdjustTarget) -> Option<&RgbaImage> {
        match target {
            AdjustTarget::Base => self.layers[0].image(),
            AdjustTarget::Layer(index) => self.layers.get(index).and_then(|layer| layer.image()),
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
            AdjustTarget::Base => self.base_image().clone(),
            AdjustTarget::Layer(index) => self
                .layers
                .get(index)
                .and_then(|layer| layer.image())
                .map(|img| img.clone())
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

    /// 拖动中的实时预览：把一块像素补丁直接写进目标并 bump epoch（渲染缓存
    /// 因此失效、下一帧就是新像素），**不**写历史。松手时由 `commit_patch`
    /// 把同一个矩形的前后两份位图补成一条撤销记录。
    pub fn write_patch(&mut self, target: AdjustTarget, patch: &Patch) {
        if patch.is_empty() {
            return;
        }
        apply_region(self, target, patch.rect, &patch.image);
        self.edit_epoch = self.edit_epoch.wrapping_add(1);
    }

    /// 提交一块已经生效的像素改动。`before` / `after` 是同一矩形的两份位图，
    /// 撤销贴回 before、重做贴回 after，不必为一次涂抹留整图层快照。
    pub fn commit_patch(
        &mut self,
        target: AdjustTarget,
        rect: Rect,
        before: RgbaImage,
        after: RgbaImage,
    ) -> Result<(), EditError> {
        if rect.width == 0 || rect.height == 0 {
            return Ok(());
        }
        self.execute(EditOp::PixelPatch {
            target,
            rect,
            before,
            after,
        })
    }

    /// 实时预览用：直接替换某图层的位图 / 偏移，不写历史，只 bump epoch。
    /// 拖动旋转手柄或角度对话框改数值时每帧调它。
    pub fn preview_layer(&mut self, index: usize, layer: Layer) -> Result<(), EditError> {
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        self.layers[index] = layer;
        self.edit_epoch = self.edit_epoch.wrapping_add(1);
        Ok(())
    }

    /// 提交一次图层整体替换（旋转手柄 / 角度对话框松手时）。
    pub fn commit_layer(
        &mut self,
        index: usize,
        before: Layer,
        after: Layer,
    ) -> Result<(), EditError> {
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        if before == after {
            return Ok(());
        }
        self.execute(EditOp::RotateLayer {
            index,
            before,
            after,
        })
    }

    /// 任意角度旋转某个图层（绕图层中心）。`expand` = true 时位图按旋转后
    /// 的包围盒放大，内容不裁切；false 时保持原尺寸。
    pub fn rotate_layer_angle(
        &mut self,
        index: usize,
        degrees: f32,
        expand: bool,
    ) -> Result<(), EditError> {
        let layer = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?;
        if degrees.abs() < 0.01 {
            return Ok(());
        }
        let before = layer.clone();
        let prev = layer.raster().ok_or(EditError::LayerIndex(index))?;
        // 落定用双三次：双线性转完一圈下来整层会明显发软（只此一次，代价
        // 摊不进拖动帧，没有理由省）。
        let (image, delta) = rotate_image_hq(&prev.image, degrees, expand);
        let mut after = before.clone();
        // after 的位图是从 before.image 重新旋转出来的，原始像素已经不再
        // 对应新的内容，烘焙掉非破坏性状态（P0-2 会把旋转也参数化）。
        after.ensure_baked();
        let (po, pi) = (prev.offset, image.clone());
        match &mut after.content {
            LayerContent::Raster(r) => {
                r.image = pi;
                r.offset = (po.0 + delta.0, po.1 + delta.1);
            }
            LayerContent::Mark(_) => return Err(EditError::LayerIndex(index)),
        }
        if after == before {
            return Ok(());
        }
        self.execute(EditOp::RotateLayer {
            index,
            before,
            after,
        })
    }

    /// 单点合成（**只含位图图层**）。
    ///
    /// 矢量标记不参与逐像素采样 —— 与统一模型之前的行为一致（旧实现的
    /// `composite_pixel` 也只看 base + layers，标记仅在 `flatten` 里绘制）。
    /// 魔术棒「取样全部图层」因此不会把标记颜色算进去。
    pub fn composite_pixel(&self, x: i32, y: i32) -> image::Rgba<u8> {
        if x < 0 || y < 0 || x >= self.canvas.width as i32 || y >= self.canvas.height as i32 {
            return image::Rgba([0, 0, 0, 0]);
        }
        let mut out = [0u8, 0, 0, 0];
        for layer in &self.layers {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            let Some(r) = layer.raster() else {
                continue;
            };
            let lx = x - r.offset.0;
            let ly = y - r.offset.1;
            if lx < 0
                || ly < 0
                || lx >= r.image.width() as i32
                || ly >= r.image.height() as i32
            {
                continue;
            }
            let src = r.image.get_pixel(lx as u32, ly as u32).0;
            blend_over(&mut out, src, layer.opacity);
        }
        image::Rgba(out)
    }

    /// 魔术棒采样：选区的形状由「某一层（或可见合成）的颜色」决定。
    ///
    /// `target = None` → 可见合成（PS 的「取样全部图层」）；`Some(Base)` /
    /// `Some(Layer(i))` → 只取那一层。坐标是画布坐标，图层偏移在这里换算，
    /// 调用方（QML/探针）给的都是用户在画布上点到的位置。
    pub fn magic_wand_selection(
        &self,
        x: i32,
        y: i32,
        opts: WandOptions,
        target: Option<AdjustTarget>,
    ) -> SelectionMask {
        match target {
            None => SelectionMask::magic_wand(self.canvas, (x, y), opts, |px, py| {
                self.composite_pixel(px, py).0
            }),
            Some(AdjustTarget::Base) => {
                SelectionMask::magic_wand(self.canvas, (x, y), opts, |px, py| {
                    self.base_pixel_at(px, py)
                })
            }
            Some(AdjustTarget::Layer(index)) => {
                SelectionMask::magic_wand(self.canvas, (x, y), opts, |px, py| {
                    self.layer_pixel(index, px, py)
                })
            }
        }
    }

    /// 快速选择的取样源，规则与 `magic_wand_selection` 相同：
    /// `None` = 可见合成（取样全部图层），`Some(..)` = 只取该层。
    pub fn quick_select_selection(
        &self,
        pen: &[(f32, f32)],
        radius: u32,
        opts: QuickSelectOptions,
        target: Option<AdjustTarget>,
    ) -> SelectionMask {
        match target {
            None => SelectionMask::quick_select(self.canvas, pen, radius, opts, |px, py| {
                self.composite_pixel(px, py).0
            }),
            Some(AdjustTarget::Base) => {
                SelectionMask::quick_select(self.canvas, pen, radius, opts, |px, py| {
                    self.base_pixel_at(px, py)
                })
            }
            Some(AdjustTarget::Layer(index)) => {
                SelectionMask::quick_select(self.canvas, pen, radius, opts, |px, py| {
                    self.layer_pixel(index, px, py)
                })
            }
        }
    }

    /// 背景像素（画布坐标 → 背景层坐标）。不可见 / 越界 = 全透明。
    pub fn base_pixel_at(&self, x: i32, y: i32) -> [u8; 4] {
        let bg = &self.layers[0];
        if !bg.visible {
            return [0, 0, 0, 0];
        }
        let Some(r) = bg.raster() else {
            return [0, 0, 0, 0];
        };
        let bx = x - r.offset.0;
        let by = y - r.offset.1;
        if bx < 0 || by < 0 || bx as u32 >= r.image.width() || by as u32 >= r.image.height() {
            return [0, 0, 0, 0];
        }
        r.image.get_pixel(bx as u32, by as u32).0
    }

    /// 单个图层的像素（画布坐标 → 图层坐标）。不可见 / 图层外 / 标记 = 全透明。
    pub fn layer_pixel(&self, index: usize, x: i32, y: i32) -> [u8; 4] {
        let Some(layer) = self.layers.get(index) else {
            return [0, 0, 0, 0];
        };
        if !layer.visible || layer.opacity <= 0.0 {
            return [0, 0, 0, 0];
        }
        let Some(r) = layer.raster() else {
            return [0, 0, 0, 0];
        };
        let lx = x - r.offset.0;
        let ly = y - r.offset.1;
        if lx < 0 || ly < 0 || lx as u32 >= r.image.width() || ly as u32 >= r.image.height() {
            return [0, 0, 0, 0];
        }
        let mut p = r.image.get_pixel(lx as u32, ly as u32).0;
        p[3] = (p[3] as f32 * layer.opacity).round().clamp(0.0, 255.0) as u8;
        p
    }

    /// 全幅合成：按 `layers` 顺序自底向上，位图按 offset + 不透明度混合，
    /// 标记按不透明度栅格化到累积图。**顺序即 z 序** —— 标记不再天然置顶。
    pub fn flatten(&self) -> RgbaImage {
        let mut out = RgbaImage::new(self.canvas.width, self.canvas.height);
        for layer in &self.layers {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            match &layer.content {
                LayerContent::Raster(r) => blend_raster_into(&mut out, r, layer.opacity),
                LayerContent::Mark(m) => {
                    if layer.opacity >= 0.999 {
                        m.draw(&mut out);
                    } else {
                        // 标记的不透明度：先画到副本，再按系数回混。
                        let mut tmp = out.clone();
                        m.draw(&mut tmp);
                        for (dst, src) in out.pixels_mut().zip(tmp.pixels()) {
                            let p = dst.0;
                            dst.0 = [
                                (p[0] as f32 * (1.0 - layer.opacity)
                                    + src.0[0] as f32 * layer.opacity)
                                    .round() as u8,
                                (p[1] as f32 * (1.0 - layer.opacity)
                                    + src.0[1] as f32 * layer.opacity)
                                    .round() as u8,
                                (p[2] as f32 * (1.0 - layer.opacity)
                                    + src.0[2] as f32 * layer.opacity)
                                    .round() as u8,
                                (p[3] as f32 * (1.0 - layer.opacity)
                                    + src.0[3] as f32 * layer.opacity)
                                    .round() as u8,
                            ];
                        }
                    }
                }
            }
        }
        out
    }
}

/// 把 `src` 按 `opacity` 叠到 `out` 上（与旧 `composite_pixel` 逐像素一致）。
fn blend_over(out: &mut [u8; 4], src: [u8; 4], opacity: f32) {
    let a = (src[3] as f32 / 255.0) * opacity;
    if a <= 0.0 {
        return;
    }
    let inv = 1.0 - a;
    for c in 0..3 {
        out[c] = (src[c] as f32 * a + out[c] as f32 * inv).round() as u8;
    }
    out[3] = ((a + out[3] as f32 / 255.0 * inv) * 255.0).round() as u8;
}

/// 把一个位图图层按 offset 混合进整幅画布。
fn blend_raster_into(out: &mut RgbaImage, r: &RasterLayer, opacity: f32) {
    let (ox, oy) = r.offset;
    let x0 = ox.max(0);
    let y0 = oy.max(0);
    let x1 = (ox + r.image.width() as i32).min(out.width() as i32);
    let y1 = (oy + r.image.height() as i32).min(out.height() as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            let src = r
                .image
                .get_pixel((x - ox) as u32, (y - oy) as u32)
                .0;
            if src[3] == 0 {
                continue;
            }
            let px = out.get_pixel_mut(x as u32, y as u32);
            blend_over(&mut px.0, src, opacity);
        }
    }
}

