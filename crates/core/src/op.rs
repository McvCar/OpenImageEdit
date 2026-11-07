// crates/core/src/op.rs
use super::*;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("layer index {0} out of range")]
    LayerIndex(usize),
    #[error("mark index {0} out of range")]
    MarkIndex(usize),
    #[error("selection is empty")]
    EmptySelection,
    #[error("crop rect is empty or outside the image")]
    EmptyCrop,
    #[error("target size must be at least 1x1")]
    InvalidCanvasSize,
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
    #[error("the base layer needs at least one other layer")]
    BaseRequired,
}


#[derive(Debug, Clone, PartialEq)]
pub enum EditOp {
    TransformDocument { transform: Transform },
    CropDocument { from: Rect, to: Rect },
    /// 裁剪到选区**轮廓**（不是包围盒）：画布缩到选区的包围盒，同时把
    /// 轮廓之外的像素按覆盖率清成透明 —— 椭圆/套索/羽化选区裁完是透明
    /// 外形，而不是一块矩形。base 与所有图层一起处理。
    /// 选区坐标是裁剪前的画布坐标；`from` 快照让 undo 精确还原。
    CropToSelection { from: DocumentSnapshot, selection: SelectionMask },
    ResizeDocument { from: DocumentSnapshot, to: Size },
    AddLayer { layer: Layer, index: usize },
    RemoveLayer { layer: Layer, index: usize },
    MoveLayer { from: usize, to: usize },
    SetVisible { index: usize, from: bool, to: bool },
    SetOpacity { index: usize, from: f32, to: f32 },
    TranslateLayer { index: usize, from: (i32, i32), to: (i32, i32) },
    /// P0-1 参数化变换历史：from/to 只存参数 + `Arc<原图>` 的浅引用，
    /// 一条 op 取代整文档快照。P0-2 统一会话启用后，移动 / 缩放 / 旋转
    /// 全部汇流到这里；P0-1 期间 `ScaleLayer` 仍走 Lanczos3 适配层。
    SetLayerTransform {
        index: usize,
        from: Option<LayerTransform>,
        to: Option<LayerTransform>,
    },
    RenameLayer { index: usize, from: String, to: String },
    /// Non-destructive "delete selection pixels": the base pixels under the
    /// selection fade out (scaled by the mask's alpha coverage, so a feathered
    /// selection gets a soft edge) while layers stay untouched. The pre-op
    /// snapshot makes undo exact (same pattern as `ResizeDocument`).
    EraseBasePixels { from: DocumentSnapshot, selection: SelectionMask },
    /// Scale a layer's image to `to` (Lanczos3) and move its top-left corner
    /// to `offset`, so handle-driven resizes anchor at the dragged corner /
    /// edge instead of always growing toward the bottom-right. The pre-op
    /// snapshot makes undo exact (same pattern as `ResizeDocument`).
    ScaleLayer {
        index: usize,
        from: DocumentSnapshot,
        to: Size,
        offset: (i32, i32),
        /// Handles dragged past the opposite edge: mirror the resampled
        /// bitmap on that axis (negative width / height in the UI).
        flip: (bool, bool),
    },
    /// Flip / rotate ONE layer's bitmap around its own centre (PS: with a
    /// layer selected, "flip horizontal" transforms that layer only, the
    /// base and the other layers stay put). The offset is untouched — the
    /// layer occupies the same box, its pixels are just mirrored inside it.
    /// Purely self-inverse for flips; rotations undo through `inverted()`.
    TransformLayer { index: usize, transform: Transform },
    /// "Delete" the base layer: the canvas becomes fully transparent while
    /// layers stay untouched. Undo restores the previous base pixels.
    DeleteBase { from: DocumentSnapshot },
    /// Hide/show the base layer (eye toggle in the panel). Cheap flag flip,
    /// no pixels involved; snapshot-free undo via from/to.
    SetBaseVisible { from: bool, to: bool },
    /// Change the canvas resolution WITHOUT resampling any pixels: content
    /// keeps its size and is re-anchored to the center of the new canvas
    /// (clipped when shrinking, transparent padding when growing). The
    /// pre-op snapshot makes undo exact (same pattern as `ResizeDocument`).
    CanvasResize { from: DocumentSnapshot, to: Size },
    /// "Unlock" the base layer: its pixels become a regular layer at the
    /// bottom of the stack (movable / scalable / deletable like any other
    /// layer) and the base itself becomes a transparent canvas of the same
    /// size. The pre-op snapshot makes undo exact.
    BaseToLayer { from: DocumentSnapshot },
    /// Add a markup annotation on top of the composition. `index` is always
    /// the current list length (marks stack in creation order).
    AddMark { index: usize, mark: Mark },
    /// Restyle / move / resize an existing annotation. `from` is the mark
    /// before the change (undo restores it verbatim).
    UpdateMark { index: usize, from: Mark, to: Mark },
    /// Remove an annotation; the removed mark is kept for undo.
    DeleteMark { index: usize, mark: Mark },
    /// Bake a color adjustment into the target's pixels ("调整颜色" panel).
    /// Only the pre-op image of the scope is stored; redo recomputes
    /// `adjust(from)`, so a 50 MP base costs one extra image instead of two.
    AdjustPixels {
        target: AdjustTarget,
        from: RgbaImage,
        params: Adjust,
    },
    /// 像素级修补（污点修复 / 修复画笔 / 修补 / 油漆桶 / 渐变）：只保存
    /// 改动矩形内的前后两份位图。撤销/重做就是把那一块贴回去 —— 不必为
    /// 每一次涂抹留整图层快照。`rect` 是目标位图自己的坐标（base 用 view
    /// 换算后的坐标，图层用图层内坐标）。
    PixelPatch {
        target: AdjustTarget,
        rect: Rect,
        before: RgbaImage,
        after: RgbaImage,
    },
    /// 图层整体旋转（任意角度）：位图尺寸与左上角偏移都会变，直接存旋转
    /// 前后的整层状态，撤销/重做都是整体替换。
    RotateLayer {
        index: usize,
        before: Layer,
        after: Layer,
    },
    /// Replace the whole selection (new marquee / invert / clear / move /
    /// resize). Selection is document state in PS and here too, so Ctrl+Z
    /// walks it back together with the pixel edits in one unified history.
    SetSelection {
        from: Option<SelectionState>,
        to: Option<SelectionState>,
    },
    /// 边界 / 平滑 / 羽化 参数调整。单独一种 op 是为了合并：数字框连点几
    /// 下箭头不该塞进五条历史（见 `History::push`）。
    SelectionOptions { from: SelOps, to: SelOps },
}

impl EditOp {
    pub fn apply(&self, doc: &mut Document) -> Result<(), EditError> {
        match self {
            EditOp::TransformDocument { transform } => {
                transform_document(doc, *transform);
                // 坐标空间翻了/转了，旧的 alpha 覆盖图立刻就是错的：宁可
                // 让选区消失，也别让它指向错位的一块区域。
                doc.selection = None;
                Ok(())
            }
            EditOp::CropDocument { to, .. } => {
                apply_view(doc, *to);
                doc.selection = None;
                Ok(())
            }
            EditOp::CropToSelection { selection, .. } => {
                // 顺序很重要：先按**裁剪前**的画布坐标挖掉轮廓外的像素，
                // 再动 view（一旦 apply_view 改了原点，坐标就对不上了）。
                clear_outside_selection(doc, selection);
                let canvas_rect = Rect::new(0, 0, doc.canvas.width, doc.canvas.height);
                let area = selection
                    .bounds
                    .intersect(canvas_rect)
                    .ok_or(EditError::EmptyCrop)?;
                apply_view(
                    doc,
                    Rect::new(
                        doc.view.x + area.x,
                        doc.view.y + area.y,
                        area.width,
                        area.height,
                    ),
                );
                doc.selection = None;
                Ok(())
            }
            EditOp::ResizeDocument { to, .. } => {
                apply_resize(doc, *to);
                doc.selection = None;
                Ok(())
            }
            EditOp::AddLayer { layer, index } => {
                if *index > doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.insert(*index, layer.clone());
                Ok(())
            }
            EditOp::RemoveLayer { index, .. } => {
                if *index >= doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.remove(*index);
                Ok(())
            }
            EditOp::MoveLayer { from, to } => {
                if *from >= doc.layers.len() || *to >= doc.layers.len() {
                    return Err(EditError::LayerIndex((*from).max(*to)));
                }
                let layer = doc.layers.remove(*from);
                doc.layers.insert(*to, layer);
                Ok(())
            }
            EditOp::SetVisible { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.visible = *to;
                Ok(())
            }
            EditOp::SetOpacity { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.opacity = to.clamp(0.0, 1.0);
                Ok(())
            }
            EditOp::TranslateLayer { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.offset = *to;
                Ok(())
            }
            EditOp::SetLayerTransform { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                match to {
                    Some(t) => {
                        let (image, offset) = t.render_bounds(true);
                        layer.set_transform_pixels(image, offset, t.clone());
                    }
                    None => layer.ensure_baked(),
                }
                Ok(())
            }
            EditOp::RenameLayer { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.name = to.clone();
                Ok(())
            }
            EditOp::EraseBasePixels { selection, .. } => {
                erase_base_pixels(doc, selection);
                Ok(())
            }
            EditOp::ScaleLayer { index, from, to, offset, flip } => {
                if to.width < 1 || to.height < 1 {
                    return Err(EditError::InvalidCanvasSize);
                }
                // P0-1 非破坏性缩放：旧实现每次都对「当前 image」再采一次，
                // 缩小→放大就累积了两次插值，越缩越糊。现在无论缩多少次，
                // 输入恒定是变换前快照里保存的未插值原图。
                let prev = from
                    .layers
                    .get(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                let mut t = prev.transform.clone().unwrap_or_else(|| {
                    LayerTransform::from_offset(prev.image.clone(), prev.offset.0, prev.offset.1)
                });
                // 旋转 / 斜切后的外接框不能用目标宽高简单反解 scale，这部分
                // 仍走旧路径（从当前位图重采样），保持既有几何数值不变。
                let pure_axis_aligned = t.rotation.abs() < 1e-9
                    && t.skew == (0.0, 0.0)
                    && t.perspective.is_none();
                if !pure_axis_aligned {
                    let layer = doc
                        .layers
                        .get_mut(*index)
                        .ok_or(EditError::LayerIndex(*index))?;
                    let mut image = image::imageops::resize(
                        &prev.image,
                        to.width,
                        to.height,
                        image::imageops::FilterType::Lanczos3,
                    );
                    if flip.0 {
                        image = image::imageops::flip_horizontal(&image);
                    }
                    if flip.1 {
                        image = image::imageops::flip_vertical(&image);
                    }
                    layer.image = image;
                    layer.offset = *offset;
                    layer.ensure_baked();
                    return Ok(());
                }
                // 累计翻转一次生成：从原始像素出发，不能把之前已翻过的
                // image 当输入，否则第二次缩放会把翻转丢掉。
                let cumulative = (t.flip.0 ^ flip.0, t.flip.1 ^ flip.1);
                let source = t.source.clone();
                let mut image = image::imageops::resize(
                    source.as_ref(),
                    to.width,
                    to.height,
                    image::imageops::FilterType::Lanczos3,
                );
                if cumulative.0 {
                    image = image::imageops::flip_horizontal(&image);
                }
                if cumulative.1 {
                    image = image::imageops::flip_vertical(&image);
                }
                let (sw, sh) = t.source_size();
                t.scale = (to.width as f64 / sw, to.height as f64 / sh);
                t.center = (
                    offset.0 as f64 + to.width as f64 / 2.0,
                    offset.1 as f64 + to.height as f64 / 2.0,
                );
                t.flip = cumulative;
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.set_transform_pixels(image, *offset, t);
                Ok(())
            }
            EditOp::TransformLayer { index, transform } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.ensure_baked();
                layer.image = transform.apply_to_image(&layer.image);
                Ok(())
            }
            EditOp::DeleteBase { .. } => {
                apply_delete_base(doc);
                Ok(())
            }
            EditOp::SetBaseVisible { to, .. } => {
                doc.base_hidden = *to;
                Ok(())
            }
            EditOp::CanvasResize { to, .. } => {
                apply_canvas_resize(doc, *to);
                doc.selection = None;
                Ok(())
            }
            EditOp::BaseToLayer { .. } => {
                apply_base_to_layer(doc);
                Ok(())
            }
            EditOp::AddMark { index, mark } => {
                if *index > doc.marks.len() {
                    return Err(EditError::MarkIndex(*index));
                }
                doc.marks.insert(*index, mark.clone());
                Ok(())
            }
            EditOp::UpdateMark { index, to, .. } => {
                let m = doc
                    .marks
                    .get_mut(*index)
                    .ok_or(EditError::MarkIndex(*index))?;
                *m = to.clone();
                Ok(())
            }
            EditOp::DeleteMark { index, .. } => {
                if *index >= doc.marks.len() {
                    return Err(EditError::MarkIndex(*index));
                }
                doc.marks.remove(*index);
                Ok(())
            }
            EditOp::AdjustPixels {
                target, from, params,
            } => {
                let adjusted = params.apply_to(from);
                match target {
                    AdjustTarget::Base => {
                        doc.base = adjusted;
                        Ok(())
                    }
                    AdjustTarget::Layer(index) => {
                        let layer = doc
                            .layers
                            .get_mut(*index)
                            .ok_or(EditError::LayerIndex(*index))?;
                        layer.ensure_baked();
                        layer.image = adjusted;
                        Ok(())
                    }
                }
            }
            EditOp::PixelPatch {
                target,
                rect,
                after,
                ..
            } => {
                apply_region(doc, *target, *rect, after);
                Ok(())
            }
            EditOp::RotateLayer { index, after, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                *layer = after.clone();
                Ok(())
            }
            EditOp::SetSelection { to, .. } => {
                doc.selection = to.clone();
                Ok(())
            }
            EditOp::SelectionOptions { to, .. } => {
                if let Some(st) = doc.selection.as_mut() {
                    st.set_ops(*to);
                }
                Ok(())
            }
        }
    }

    pub fn revert(&self, doc: &mut Document) -> Result<(), EditError> {
        match self {
            EditOp::TransformDocument { transform } => {
                transform_document(doc, transform.inverted());
                doc.selection = None;
                Ok(())
            }
            EditOp::CropDocument { from, .. } => {
                apply_view(doc, *from);
                doc.selection = None;
                Ok(())
            }
            EditOp::CropToSelection { from, .. } => {
                // 像素被挖空过，换 view 还原不回来 —— 整份快照回滚。
                doc.restore(from.clone());
                doc.selection = None;
                Ok(())
            }
            EditOp::ResizeDocument { from, .. } => {
                doc.restore(from.clone());
                doc.selection = None;
                Ok(())
            }
            EditOp::AddLayer { index, .. } => {
                if *index >= doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.remove(*index);
                Ok(())
            }
            EditOp::RemoveLayer { layer, index } => {
                if *index > doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.insert(*index, layer.clone());
                Ok(())
            }
            EditOp::MoveLayer { from, to } => {
                if *from >= doc.layers.len() || *to >= doc.layers.len() {
                    return Err(EditError::LayerIndex((*from).max(*to)));
                }
                let layer = doc.layers.remove(*to);
                doc.layers.insert(*from, layer);
                Ok(())
            }
            EditOp::SetVisible { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.visible = *from;
                Ok(())
            }
            EditOp::SetOpacity { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.opacity = *from;
                Ok(())
            }
            EditOp::TranslateLayer { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.offset = *from;
                Ok(())
            }
            EditOp::SetLayerTransform { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                match from {
                    Some(t) => {
                        let (image, offset) = t.render_bounds(true);
                        layer.set_transform_pixels(image, offset, t.clone());
                    }
                    None => layer.ensure_baked(),
                }
                Ok(())
            }
            EditOp::RenameLayer { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.name = from.clone();
                Ok(())
            }
            EditOp::EraseBasePixels { from, .. } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::ScaleLayer { from, .. } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::TransformLayer { index, transform } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.ensure_baked();
                layer.image = transform.inverted().apply_to_image(&layer.image);
                Ok(())
            }
            EditOp::DeleteBase { from } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::SetBaseVisible { from, .. } => {
                doc.base_hidden = *from;
                Ok(())
            }
            EditOp::CanvasResize { from, .. } => {
                doc.restore(from.clone());
                doc.selection = None;
                Ok(())
            }
            EditOp::BaseToLayer { from } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::AddMark { index, .. } => {
                if *index >= doc.marks.len() {
                    return Err(EditError::MarkIndex(*index));
                }
                doc.marks.remove(*index);
                Ok(())
            }
            EditOp::UpdateMark { index, from, .. } => {
                let m = doc
                    .marks
                    .get_mut(*index)
                    .ok_or(EditError::MarkIndex(*index))?;
                *m = from.clone();
                Ok(())
            }
            EditOp::DeleteMark { index, mark } => {
                if *index > doc.marks.len() {
                    return Err(EditError::MarkIndex(*index));
                }
                doc.marks.insert(*index, mark.clone());
                Ok(())
            }
            EditOp::AdjustPixels { target, from, .. } => {
                match target {
                    AdjustTarget::Base => {
                        doc.base = from.clone();
                        Ok(())
                    }
                    AdjustTarget::Layer(index) => {
                        let layer = doc
                            .layers
                            .get_mut(*index)
                            .ok_or(EditError::LayerIndex(*index))?;
                        layer.ensure_baked();
                        layer.image = from.clone();
                        Ok(())
                    }
                }
            }
            EditOp::PixelPatch {
                target,
                rect,
                before,
                ..
            } => {
                apply_region(doc, *target, *rect, before);
                Ok(())
            }
            EditOp::RotateLayer { index, before, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                *layer = before.clone();
                Ok(())
            }
            EditOp::SetSelection { from, .. } => {
                doc.selection = from.clone();
                Ok(())
            }
            EditOp::SelectionOptions { from, .. } => {
                if let Some(st) = doc.selection.as_mut() {
                    st.set_ops(*from);
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct History {
    pub(crate) undo: Vec<EditOp>,
    pub(crate) redo: Vec<EditOp>,
    pub(crate) limit: usize,
}

impl History {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit,
        }
    }

    pub(crate) fn push(&mut self, op: EditOp) {
        // 边界/平滑/羽化 的内联数字框每点一下就是一个新值：连着点十下不该
        // 留十条历史。连续两次参数调整合并成一条（保留最早的 from、最新的
        // to），Ctrl+Z 一次退回调整前的状态 —— 和 PS 的数字框手感一致。
        if let EditOp::SelectionOptions { to, .. } = &op {
            if let Some(EditOp::SelectionOptions { to: prev, .. }) = self.undo.last_mut() {
                *prev = *to;
                self.redo.clear();
                return;
            }
        }
        self.undo.push(op);
        if self.limit > 0 && self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

