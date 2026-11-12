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
    #[error("layer group {0} not found")]
    GroupNotFound(GroupId),
    #[error("grouping needs at least two ordinary layers")]
    GroupNeedsTwoLayers,
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
    /// 图层锁定开关（可撤销）。锁定的层拒绝移动 / 缩放 / 删除 / 排序，
    /// 现在它也进历史 —— 误锁一下能 Ctrl+Z 回来。
    SetLocked { index: usize, from: bool, to: bool },
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
    /// 整份替换分组元数据（组数量很少，全量存最省事且撤销精确）。
    /// 与 `SetLayerGroup` 一起包进 `Batch`，就是一次可撤销的编组 / 解组。
    SetGroups { from: Vec<LayerGroup>, to: Vec<LayerGroup> },
    /// 单层归属变更。`from`/`to` 都记下来，撤销时精确还原。
    SetLayerGroup { index: usize, from: Option<GroupId>, to: Option<GroupId> },
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
    /// "删除背景"：像素清空成透明，图层条目本身保留（栈结构不变）。
    /// 撤销用快照精确还原。
    DeleteBase { from: DocumentSnapshot },
    /// 显示 / 隐藏背景层（眼睛图标）。纯标志翻转，不走快照。
    SetBaseVisible { from: bool, to: bool },
    /// Change the canvas resolution WITHOUT resampling any pixels: content
    /// keeps its size and is re-anchored to the center of the new canvas
    /// (clipped when shrinking, transparent padding when growing). The
    /// pre-op snapshot makes undo exact (same pattern as `ResizeDocument`).
    CanvasResize { from: DocumentSnapshot, to: Size },
    /// Restyle / move / resize a markup layer. `index` is the **layer index**,
    /// `from`/`to` are the mark before and after (layer attributes untouched).
    UpdateMark { index: usize, from: Mark, to: Mark },
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
    /// 整层替换：位图 + 偏移 + 变换参数（旋转手柄落定、合并图层、栅格化
    /// 标记都走这条）。before/after 各存一份，撤销/重做都是整体替换。
    ReplaceLayer {
        index: usize,
        before: Layer,
        after: Layer,
    },
    /// 一串原子编辑当成**一条**历史：对齐 / 分布 / 批量显隐这类「一条命令
    /// 改多层」的操作走这里，Ctrl+Z 一次全部退回。
    ///
    /// apply 顺序执行，revert 逆序回滚；中途任何一步失败，已应用的部分先
    /// 回滚再报错，不留半截状态。
    Batch(Vec<EditOp>),
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
    /// 纯分组元数据操作不改像素：`Document::execute/undo/redo` 据此跳过
    /// `edit_epoch`，避免编组 / 解组把预览缓存与原生合成校准一起废掉。
    pub fn is_metadata_only(&self) -> bool {
        match self {
            EditOp::SetGroups { .. } | EditOp::SetLayerGroup { .. } => true,
            EditOp::Batch(ops) => ops.iter().all(|op| op.is_metadata_only()),
            _ => false,
        }
    }

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
                // 背景层恒占 0 号位：往 0 号插入会把背景挤到 1 号，破坏
                // 「layers[0] 是背景」这条不变量。
                if *index == 0 || *index > doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.insert(*index, layer.clone());
                Ok(())
            }
            EditOp::RemoveLayer { index, .. } => {
                if *index == 0 || *index >= doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.remove(*index);
                Ok(())
            }
            EditOp::MoveLayer { from, to } => {
                if *from >= doc.layers.len() || *to >= doc.layers.len() || *from == 0 || *to == 0 {
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
            EditOp::SetLocked { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.locked = *to;
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
            EditOp::TranslateLayer { index, from, to } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                // 位图层的 `offset` 是绝对坐标；标记图层没有 offset，按
                // from → to 的差值平移几何。
                match &mut layer.content {
                    LayerContent::Raster(r) => r.offset = *to,
                    LayerContent::Mark(m) => {
                        *m = m.translated((to.0 - from.0) as f32, (to.1 - from.1) as f32);
                    }
                }
                Ok(())
            }
            EditOp::SetLayerTransform { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                if !layer.is_raster() {
                    return Err(EditError::LayerIndex(*index));
                }
                match to {
                    Some(t) => {
                        // P0-2b2：统一用兼容后端，落定像素与旧三条路径逐像素一致。
                        let (image, offset) = t.render_compatible();
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
            EditOp::SetGroups { to, .. } => {
                doc.groups = to.clone();
                Ok(())
            }
            EditOp::SetLayerGroup { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.group = *to;
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
                let prev = prev.raster().ok_or(EditError::LayerIndex(*index))?;
                let mut t = prev.transform.clone().unwrap_or_else(|| {
                    LayerTransform::from_offset(prev.image.clone(), prev.offset.0, prev.offset.1)
                });
                // 累计翻转一次生成：从原始像素出发，不能把之前已翻过的
                // image 当输入，否则第二次缩放会把翻转丢掉。
                let cumulative = (t.flip.0 ^ flip.0, t.flip.1 ^ flip.1);
                let (sw, sh) = t.source_size();
                // 回到原始尺寸时走 1:1 索引拷贝。两个原因：
                //  ① image::imageops::resize 即使 80→80 也会跑一遍 Lanczos3，
                //    负旁瓣会把文字笔画 / 单像素细线抹软（11-07 复测：像素
                //    块能还原、文字还原不了，根因就在这里）；
                //  ② QML 手柄把显示像素换算回图像像素时有 ±1~2px 误差，
                //    不吸附就永远差一点，用户拖回"100%"仍要吃一次插值。
                // 只吸附无翻转、无旋转、且两轴都接近原尺寸的情况；redo 复用
                // 同一个 op，吸附结果幂等。
                let snap_back = !cumulative.0
                    && !cumulative.1
                    && t.rotation.abs() < 1e-9
                    && (to.width as f64 - sw).abs() <= 2.0
                    && (to.height as f64 - sh).abs() <= 2.0;
                let (final_w, final_h) = if snap_back {
                    (sw, sh)
                } else {
                    (to.width as f64, to.height as f64)
                };
                t.flip = cumulative;
                // ★旋转 / 斜切过的图层不再退回「从当前位图再采一次」的老路
                //（那会让「拉伸 → 旋转 → 再拉伸」每轮多一次插值：框越拉越大、
                // 图越拉越糊）。统一按目标外接框反解 scale，source 恒为原图。
                let target = RectF::new(
                    offset.0 as f64,
                    offset.1 as f64,
                    final_w,
                    final_h,
                );
                if !t.set_bounds_rect(target, 0) {
                    return Err(EditError::InvalidCanvasSize);
                }
                let (image, placed) = t.render_compatible();
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.set_transform_pixels(image, placed, t);
                Ok(())
            }
            EditOp::TransformLayer { index, transform } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.ensure_baked();
                let Some(img) = layer.image_mut() else {
                    return Err(EditError::LayerIndex(*index));
                };
                *img = transform.apply_to_image(img);
                Ok(())
            }
            EditOp::DeleteBase { .. } => {
                apply_delete_base(doc);
                Ok(())
            }
            EditOp::SetBaseVisible { to, .. } => {
                doc.layers[0].visible = *to;
                Ok(())
            }
            EditOp::CanvasResize { to, .. } => {
                apply_canvas_resize(doc, *to);
                doc.selection = None;
                Ok(())
            }
            EditOp::UpdateMark { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::MarkIndex(*index))?;
                match &mut layer.content {
                    LayerContent::Mark(m) => {
                        *m = to.clone();
                        Ok(())
                    }
                    LayerContent::Raster(_) => Err(EditError::MarkIndex(*index)),
                }
            }
            EditOp::AdjustPixels {
                target, from, params,
            } => {
                let adjusted = params.apply_to(from);
                match target {
                    AdjustTarget::Base => {
                        doc.layers[0].ensure_baked();
                        match doc.layers[0].image_mut() {
                            Some(img) => {
                                *img = adjusted;
                                Ok(())
                            }
                            None => Err(EditError::LayerIndex(0)),
                        }
                    }
                    AdjustTarget::Layer(index) => {
                        let layer = doc
                            .layers
                            .get_mut(*index)
                            .ok_or(EditError::LayerIndex(*index))?;
                        layer.ensure_baked();
                        match layer.image_mut() {
                            Some(img) => {
                                *img = adjusted;
                                Ok(())
                            }
                            None => Err(EditError::LayerIndex(*index)),
                        }
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
            EditOp::ReplaceLayer { index, after, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                *layer = after.clone();
                Ok(())
            }
            EditOp::Batch(ops) => {
                for (done, op) in ops.iter().enumerate() {
                    if let Err(e) = op.apply(doc) {
                        // 回滚已经生效的那几步，避免「批量命令一半成功」。
                        for prev in ops[..done].iter().rev() {
                            let _ = prev.revert(doc);
                        }
                        return Err(e);
                    }
                }
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
                if *index == 0 || *index > doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.insert(*index, layer.clone());
                Ok(())
            }
            EditOp::MoveLayer { from, to } => {
                if *from >= doc.layers.len() || *to >= doc.layers.len() || *from == 0 || *to == 0 {
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
            EditOp::SetLocked { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.locked = *from;
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
            EditOp::TranslateLayer { index, from, to } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                match &mut layer.content {
                    LayerContent::Raster(r) => r.offset = *from,
                    LayerContent::Mark(m) => {
                        *m = m.translated((from.0 - to.0) as f32, (from.1 - to.1) as f32);
                    }
                }
                Ok(())
            }
            EditOp::SetLayerTransform { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                if !layer.is_raster() {
                    return Err(EditError::LayerIndex(*index));
                }
                match from {
                    Some(t) => {
                        // P0-2b2：统一用兼容后端，落定像素与旧三条路径逐像素一致。
                        let (image, offset) = t.render_compatible();
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
            EditOp::SetGroups { from, .. } => {
                doc.groups = from.clone();
                Ok(())
            }
            EditOp::SetLayerGroup { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.group = *from;
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
                let Some(img) = layer.image_mut() else {
                    return Err(EditError::LayerIndex(*index));
                };
                *img = transform.inverted().apply_to_image(img);
                Ok(())
            }
            EditOp::DeleteBase { from } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::SetBaseVisible { from, .. } => {
                doc.layers[0].visible = *from;
                Ok(())
            }
            EditOp::CanvasResize { from, .. } => {
                doc.restore(from.clone());
                doc.selection = None;
                Ok(())
            }
            EditOp::UpdateMark { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::MarkIndex(*index))?;
                match &mut layer.content {
                    LayerContent::Mark(m) => {
                        *m = from.clone();
                        Ok(())
                    }
                    LayerContent::Raster(_) => Err(EditError::MarkIndex(*index)),
                }
            }
            EditOp::AdjustPixels { target, from, .. } => {
                match target {
                    AdjustTarget::Base => {
                        doc.layers[0].ensure_baked();
                        match doc.layers[0].image_mut() {
                            Some(img) => {
                                *img = from.clone();
                                Ok(())
                            }
                            None => Err(EditError::LayerIndex(0)),
                        }
                    }
                    AdjustTarget::Layer(index) => {
                        let layer = doc
                            .layers
                            .get_mut(*index)
                            .ok_or(EditError::LayerIndex(*index))?;
                        layer.ensure_baked();
                        match layer.image_mut() {
                            Some(img) => {
                                *img = from.clone();
                                Ok(())
                            }
                            None => Err(EditError::LayerIndex(*index)),
                        }
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
            EditOp::ReplaceLayer { index, before, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                *layer = before.clone();
                Ok(())
            }
            EditOp::Batch(ops) => {
                // 逆序回滚：后面的子步可能依赖前面的结果（先插层后改层），
                // 反着退才回到起点。
                for op in ops.iter().rev() {
                    op.revert(doc)?;
                }
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

