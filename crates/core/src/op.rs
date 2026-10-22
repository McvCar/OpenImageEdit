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
    ResizeDocument { from: DocumentSnapshot, to: Size },
    AddLayer { layer: Layer, index: usize },
    RemoveLayer { layer: Layer, index: usize },
    MoveLayer { from: usize, to: usize },
    SetVisible { index: usize, from: bool, to: bool },
    SetOpacity { index: usize, from: f32, to: f32 },
    TranslateLayer { index: usize, from: (i32, i32), to: (i32, i32) },
    RenameLayer { index: usize, from: String, to: String },
    /// Non-destructive "delete selection pixels": the base pixels under the
    /// selection are made fully transparent while layers stay untouched. The
    /// pre-op snapshot makes undo exact (same pattern as `ResizeDocument`).
    EraseBasePixels { from: DocumentSnapshot, selection: Selection },
    /// Scale a layer's image to `to` (Lanczos3) and move its top-left corner
    /// to `offset`, so handle-driven resizes anchor at the dragged corner /
    /// edge instead of always growing toward the bottom-right. The pre-op
    /// snapshot makes undo exact (same pattern as `ResizeDocument`).
    ScaleLayer {
        index: usize,
        from: DocumentSnapshot,
        to: Size,
        offset: (i32, i32),
    },
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
}

impl EditOp {
    pub fn apply(&self, doc: &mut Document) -> Result<(), EditError> {
        match self {
            EditOp::TransformDocument { transform } => {
                transform_document(doc, *transform);
                Ok(())
            }
            EditOp::CropDocument { to, .. } => {
                apply_view(doc, *to);
                Ok(())
            }
            EditOp::ResizeDocument { to, .. } => {
                apply_resize(doc, *to);
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
            EditOp::ScaleLayer { index, to, offset, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                if to.width < 1 || to.height < 1 {
                    return Err(EditError::InvalidCanvasSize);
                }
                layer.image = image::imageops::resize(
                    &layer.image,
                    to.width,
                    to.height,
                    image::imageops::FilterType::Lanczos3,
                );
                layer.offset = *offset;
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
                        layer.image = adjusted;
                        Ok(())
                    }
                }
            }
        }
    }

    pub fn revert(&self, doc: &mut Document) -> Result<(), EditError> {
        match self {
            EditOp::TransformDocument { transform } => {
                transform_document(doc, transform.inverted());
                Ok(())
            }
            EditOp::CropDocument { from, .. } => {
                apply_view(doc, *from);
                Ok(())
            }
            EditOp::ResizeDocument { from, .. } => {
                doc.restore(from.clone());
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
                        layer.image = from.clone();
                        Ok(())
                    }
                }
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
        self.undo.push(op);
        if self.limit > 0 && self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

