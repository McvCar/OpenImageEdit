use image::RgbaImage;

mod adjust;
pub mod alpha_fit;
mod blend;
mod color;
pub mod font;
pub use adjust::{adjust_json, Adjust, AdjustTarget, Histogram, BINS};
pub use blend::{blend_pixel, BlendMode};
pub use color::{cmyk_of, hsl_of, PickedColor};
pub use font::{measure_text, FONTS};


mod geometry;
mod gesture;
mod mark;
mod layer;
mod layer_ops;
pub mod layer_transform;
pub mod snap;
mod transform;
mod op;
mod ops;
mod document;
mod selection;
mod paint;
mod retouch;

pub use geometry::{Size, Rect, Selection};
pub use gesture::{
    apply_ratio_snap, snap_angle, snap_angle_15, solve_scale_gesture, RatioSnap, RatioSnapAxis,
    ScaleRole, ScaleSolve,
};
pub use selection::{
    QuickSelectOptions, SelOps, SelShape, SelectionMask, SelectionState, WandOptions,
};
pub use mark::{MarkKind, Mark};
pub use layer::{thumbnail_image, Layer, LayerContent, LayerId, RasterLayer};
pub use layer_ops::{AlignEdge, DistributeAxis};
pub use layer_transform::{
    probe_lines as transform_probe_lines, Affine2, BoundsSolve, Field, LayerTransform, PanelValues,
    RectF, ReferencePoint, TransformParams, TRANSFORM_SERIAL_VERSION,
};
pub use transform::{
    rotate_image, rotate_image_hq, rotate_into, rotate_into_centered, rotated_aabb_size,
    rotated_box_size, round_center, sample_affine, Transform,
};
pub use paint::{crop_region, flood_fill, gradient_fill, Patch};
pub use retouch::{
    heal_clone, patch_fill, spot_heal, stroke_mask, BrushSpec, StrokeMask,
};
pub use op::{EditError, EditOp};
pub use document::{Document, PsdLayerSeed};
pub(crate) use document::DocumentSnapshot;

// Cross-module private helpers (kept `pub(crate)` + re-exported so each
// sibling module's `use super::*` can call them without naming the path).
pub(crate) use layer_ops::blend_image_at;
pub(crate) use document::{blend_over, blend_over_with};
pub(crate) use mark::norm_rect;
pub(crate) use transform::transform_document;
pub(crate) use op::History;
pub(crate) use ops::{
    apply_canvas_resize, apply_delete_base, apply_region, apply_resize,
    apply_view, clear_outside_selection, erase_base_pixels,
};

#[cfg(test)]
mod nondestructive_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            if (x / 8 + y / 8) % 2 == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        })
    }

    #[test]
    fn extract_rect_selection_produces_layer_of_selection_bounds() {
        let mut doc = Document::from_image(checker(64, 32));
        let index = doc
            .extract_selection(&SelectionMask::rect(Rect::new(8, 4, 16, 12)))
            .unwrap();
        let layer = &doc.layers[index];
        assert_eq!(layer.image().unwrap().width(), 16);
        assert_eq!(layer.image().unwrap().height(), 12);
        assert_eq!(layer.offset(), (8, 4));
        assert_eq!(layer.image().unwrap().get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(layer.image().unwrap().get_pixel(0, 4).0, [255, 0, 0, 255]);
    }

    #[test]
    fn ellipse_selection_masks_corners() {
        let doc = Document::from_image(checker(64, 64));
        let layer = doc
            .selection_layer(&SelectionMask::ellipse(Rect::new(0, 0, 64, 64)))
            .unwrap();
        assert_eq!(layer.image().unwrap().get_pixel(0, 0).0[3], 0);
        assert_eq!(layer.image().unwrap().get_pixel(32, 32).0[3], 255);
    }

    #[test]
    fn lasso_selection_contains_polygon_interior() {
        let sel = Selection::Lasso(vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ]);
        assert!(sel.contains(5.0, 5.0));
        assert!(!sel.contains(15.0, 5.0));
        assert_eq!(sel.bounds(), Rect::new(0, 0, 10, 10));
    }

    #[test]
    fn history_roundtrips_layer_ops() {
        let mut doc = Document::from_image(checker(16, 16));
        assert_eq!(doc.layers.len(), 1); // 背景
        doc.add_layer(Layer::new("clip", checker(4, 4))).unwrap();
        assert_eq!(doc.layers.len(), 2);
        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 1);
        doc.redo().unwrap();
        assert_eq!(doc.layers.len(), 2);
    }

    #[test]
    fn undo_restores_scalar_layer_properties() {
        let mut doc = Document::from_image(checker(16, 16));
        let index = doc.add_layer(Layer::new("clip", checker(4, 4))).unwrap();
        doc.set_layer_opacity(index, 0.4).unwrap();
        doc.set_layer_visible(index, false).unwrap();
        doc.translate_layer(index, 3, -2).unwrap();
        doc.rename_layer(index, "renamed").unwrap();

        doc.undo().unwrap();
        assert_eq!(doc.layers[index].name, "clip");
        doc.undo().unwrap();
        assert_eq!(doc.layers[index].offset(), (0, 0));
        doc.undo().unwrap();
        assert!(doc.layers[index].visible);
        doc.undo().unwrap();
        assert_eq!(doc.layers[index].opacity, 1.0);
    }

    #[test]
    fn cross_document_paste_keeps_pixels_and_offset() {
        let mut src = Document::from_image(checker(32, 32));
        let index = src
            .extract_selection(&SelectionMask::rect(Rect::new(4, 6, 8, 8)))
            .unwrap();
        let layer = src.layers[index].clone();

        let mut dst = Document::from_image(checker(16, 16));
        let pasted = dst.add_layer(layer).unwrap();
        assert_eq!(dst.layers[pasted].offset(), (4, 6));
        assert_eq!(dst.layers[pasted].image().unwrap().width(), 8);
        assert_eq!(dst.composite_pixel(4, 6).0, [255, 0, 0, 255]);
    }

    #[test]
    fn erase_selection_makes_base_pixels_transparent_and_keeps_layers() {
        let mut doc = Document::from_image(checker(32, 16));
        let index = doc
            .extract_selection(&SelectionMask::rect(Rect::new(20, 2, 8, 8)))
            .unwrap();

        doc.erase_selection(&SelectionMask::rect(Rect::new(4, 4, 8, 8)))
            .unwrap();
        // Erased base area is transparent (no layer above it there).
        assert_eq!(doc.composite_pixel(4, 4).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(11, 11).0, [0, 0, 0, 0]);
        // Outside the selection the base is untouched.
        assert_eq!(doc.composite_pixel(0, 0).0[3], 255);
        // The layer extracted before erasing is untouched.
        assert_eq!(doc.layers[index].image().unwrap().get_pixel(0, 0).0[3], 255);
        assert_eq!(doc.composite_pixel(20, 2).0[3], 255);
    }

    #[test]
    fn erase_selection_clips_to_canvas_and_roundtrips_undo_redo() {
        let mut doc = Document::from_image(checker(16, 16));
        let original = doc.flatten();

        // Selection partially outside the canvas: only the inside part erases.
        doc.erase_selection(&SelectionMask::rect(Rect::new(12, 12, 20, 20)))
            .unwrap();
        assert_eq!(doc.composite_pixel(12, 12).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(15, 15).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(11, 11).0[3], 255);

        doc.undo().unwrap();
        assert_eq!(doc.flatten(), original);
        doc.redo().unwrap();
        assert_eq!(doc.composite_pixel(12, 12).0, [0, 0, 0, 0]);
    }

    #[test]
    fn selection_is_undoable_document_state() {
        let mut doc = Document::from_image(checker(32, 32));
        assert!(doc.selection.is_none());
        // Ctrl+Z 一开始什么都不该有。
        assert_eq!(doc.undo(), Err(EditError::NothingToUndo));

        doc.set_selection(Some(SelectionState::new(SelectionMask::rect(Rect::new(
            4, 4, 10, 10,
        )))))
        .unwrap();
        assert_eq!(doc.selection.as_ref().unwrap().mask().bounds, Rect::new(4, 4, 10, 10));

        // 取消选择也是一步历史，撤销后选区要回来。
        doc.set_selection(None).unwrap();
        assert!(doc.selection.is_none());
        doc.undo().unwrap();
        assert_eq!(doc.selection.as_ref().unwrap().mask().bounds, Rect::new(4, 4, 10, 10));
        doc.redo().unwrap();
        assert!(doc.selection.is_none());

        // 撤回最初那一步：选区彻底消失。
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert!(doc.selection.is_none());
        doc.redo().unwrap();
        assert!(doc.selection.is_some());
    }

    #[test]
    fn selection_ops_coalesce_into_one_history_step() {
        let mut doc = Document::from_image(checker(32, 32));
        doc.set_selection(Some(SelectionState::new(SelectionMask::rect(Rect::new(
            0, 0, 20, 20,
        )))))
        .unwrap();
        // 数字框连点三下：只留一条历史。
        for k in [4u32, 8, 12] {
            doc.set_selection_ops(SelOps {
                smooth: k,
                ..Default::default()
            })
            .unwrap();
        }
        assert_eq!(doc.selection.as_ref().unwrap().ops.smooth, 12);
        doc.undo().unwrap();
        // 一次撤销退回到"还没调过"的状态，而不是 8。
        assert_eq!(doc.selection.as_ref().unwrap().ops.smooth, 0);
        assert!(doc.selection.is_some(), "选区本身不该被参数撤销带走");
    }

    #[test]
    fn crop_and_rotate_drop_the_stale_selection() {
        let mut doc = Document::from_image(checker(32, 32));
        doc.set_selection(Some(SelectionState::new(SelectionMask::rect(Rect::new(
            4, 4, 10, 10,
        )))))
        .unwrap();
        doc.crop(Rect::new(2, 2, 16, 16)).unwrap();
        // 画布原点动了，旧坐标的 alpha 图立刻是错的。
        assert!(doc.selection.is_none());
    }

    #[test]
    fn erase_selection_rejects_empty_and_outside_selections() {
        let mut doc = Document::from_image(checker(16, 16));
        assert_eq!(
            doc.erase_selection(&SelectionMask::rect(Rect::new(0, 0, 0, 0))),
            Err(EditError::EmptySelection)
        );
        assert_eq!(
            doc.erase_selection(&SelectionMask::rect(Rect::new(40, 40, 4, 4))),
            Err(EditError::EmptySelection)
        );
        assert_eq!(doc.can_undo(), false);
    }

    #[test]
    fn erase_selection_composes_with_crop_view_mapping() {
        // Erase must map canvas coords through `view` after a crop.
        let mut doc = Document::from_image(ramp(32, 32));
        doc.crop(Rect::new(8, 8, 16, 16)).unwrap();
        doc.erase_selection(&SelectionMask::rect(Rect::new(0, 0, 4, 4)))
            .unwrap();
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 0]);
        // base pixel at (8,8) is what canvas (0,0) maps to.
        assert_eq!(doc.base_image().get_pixel(8, 8).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(4, 4).0, [12, 12, 0, 255]);
    }

    #[test]
    fn scale_layer_resamples_content_and_keeps_offset() {
        let mut doc = Document::from_image(checker(32, 32));
        let index = doc
            .add_layer(Layer::new("clip", checker(8, 4)))
            .unwrap();
        doc.translate_layer(index, 5, 3).unwrap();

        doc.scale_layer(index, Size::new(16, 8), (5, 3)).unwrap();
        let layer = &doc.layers[index];
        assert_eq!(layer.image().unwrap().width(), 16);
        assert_eq!(layer.image().unwrap().height(), 8);
        // Offset (top-left anchor) is untouched by scaling.
        assert_eq!(layer.offset(), (5, 3));

        doc.undo().unwrap();
        assert_eq!(doc.layers[index].image().unwrap().width(), 8);
        assert_eq!(doc.layers[index].image().unwrap().height(), 4);
        assert_eq!(doc.layers[index].offset(), (5, 3));
        doc.redo().unwrap();
        assert_eq!(doc.layers[index].image().unwrap().width(), 16);
    }

    #[test]
    fn transform_layer_flips_only_that_layer_and_undoes() {
        let base = solid(8, 4, [10, 20, 30, 255]);
        let mut doc = Document::from_image(base.clone());
        // 左红右蓝的一层：镜像后左半必须是蓝的。
        let image = RgbaImage::from_fn(4, 2, |x, _| {
            if x < 2 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        let index = doc.add_layer(Layer::new("clip", image)).unwrap();

        doc.transform_layer(index, Transform::FlipHorizontal).unwrap();
        let layer = &doc.layers[index];
        assert_eq!(layer.image().unwrap().get_pixel(0, 0), &image::Rgba([0, 0, 255, 255]));
        assert_eq!(layer.image().unwrap().get_pixel(3, 0), &image::Rgba([255, 0, 0, 255]));
        // 图层自身的框不动（以自身为轴翻转），base 一个像素都没变。
        assert_eq!(layer.offset(), (0, 0));
        assert_eq!(*doc.base_image(), base);

        doc.undo().unwrap();
        assert_eq!(
            doc.layers[index].image().unwrap().get_pixel(0, 0),
            &image::Rgba([255, 0, 0, 255])
        );
        doc.redo().unwrap();
        assert_eq!(
            doc.layers[index].image().unwrap().get_pixel(0, 0),
            &image::Rgba([0, 0, 255, 255])
        );
    }

    #[test]
    fn scale_layer_flipped_mirrors_the_resampled_bitmap() {
        let mut doc = Document::from_image(solid(16, 16, [0, 0, 0, 255]));
        let image = RgbaImage::from_fn(4, 2, |x, _| {
            if x < 2 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        let index = doc.add_layer(Layer::new("clip", image)).unwrap();

        doc.scale_layer_flipped(index, Size::new(8, 4), (0, 0), (true, false))
            .unwrap();
        let px = *doc.layers[index].image().unwrap().get_pixel(0, 0);
        assert!(px[2] > 192 && px[0] < 64, "left edge should be blue: {px:?}");
        let right = *doc.layers[index].image().unwrap().get_pixel(7, 0);
        assert!(
            right[0] > 192 && right[2] < 64,
            "right edge should be red: {right:?}"
        );

        // 撤销回到翻转前（左红）。
        doc.undo().unwrap();
        let px = *doc.layers[index].image().unwrap().get_pixel(0, 0);
        assert!(px[0] > 192 && px[2] < 64, "undo should restore red: {px:?}");
    }

    #[test]
    fn scale_layer_rejects_bad_size_and_index() {
        let mut doc = Document::from_image(checker(16, 16));
        let index = doc.add_layer(Layer::new("clip", checker(4, 4))).unwrap();
        let undo_depth = doc.history.undo.len();
        assert_eq!(
            doc.scale_layer(index, Size::new(0, 4), (0, 0)),
            Err(EditError::InvalidCanvasSize)
        );
        assert_eq!(
            doc.scale_layer(9, Size::new(4, 4), (0, 0)),
            Err(EditError::LayerIndex(9))
        );
        // Rejected calls must not touch the history stack.
        assert_eq!(doc.history.undo.len(), undo_depth);
        assert_eq!(doc.layers[index].image().unwrap().width(), 4);
    }

    #[test]
    fn set_base_visible_roundtrips_and_affects_composite() {
        let mut doc = Document::from_image(checker(8, 8));
        doc.set_base_visible(false).unwrap();
        assert!(doc.base_hidden());
        // Hidden base composites as fully transparent.
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 0]);

        doc.undo().unwrap();
        assert!(!doc.base_hidden());
        assert_eq!(doc.composite_pixel(0, 0).0[3], 255);
        doc.redo().unwrap();
        assert!(doc.base_hidden());

        // Setting the same state again must not push a history entry.
        let depth = doc.history.undo.len();
        doc.set_base_visible(false).unwrap();
        assert_eq!(doc.history.undo.len(), depth);
        doc.set_base_visible(true).unwrap();
        assert_eq!(doc.history.undo.len(), depth + 1);
        assert!(!doc.base_hidden());
    }

    #[test]
    fn delete_base_clears_pixels_and_roundtrips() {
        let mut doc = Document::from_image(checker(16, 16));
        // 背景是一层：即使只有它一个，「删除背景」也只是把像素清空。
        let index = doc.add_layer(Layer::new("top", checker(4, 4))).unwrap();
        doc.delete_base().unwrap();
        assert!(doc.base_deleted());
        assert_eq!(doc.base_image().width(), 16);
        assert_eq!(doc.base_image().get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(doc.layers[index].image().unwrap().width(), 4);

        doc.undo().unwrap();
        assert!(!doc.base_deleted());
        assert_eq!(doc.base_image().get_pixel(0, 0).0, [255, 0, 0, 255]);
        doc.redo().unwrap();
        assert!(doc.base_deleted());
        assert_eq!(doc.base_image().get_pixel(15, 15).0, [0, 0, 0, 0]);
    }

    #[test]
    fn crop_shifts_layers_and_marks_with_the_content() {
        let mut doc = Document::from_image(checker(100, 100));
        let mut layer = Layer::new("dot", checker(10, 10));
        layer.set_offset((30, 30));
        doc.add_layer(layer).unwrap();
        let mut mark = Mark::new(MarkKind::Rect);
        mark.rect = (40.0, 40.0, 60.0, 50.0);
        doc.add_mark(mark).unwrap();

        doc.crop(Rect::new(20, 20, 50, 50)).unwrap();
        // The layer kept its place relative to the image content: its canvas
        // offset moved by -(crop origin).
        assert_eq!(doc.layers[1].offset(), (10, 10));
        // mark.rect is (x1, y1, x2, y2) corner coordinates.
        assert_eq!(doc.mark(0).unwrap().rect, (20.0, 20.0, 40.0, 30.0));
        // Flatten now equals the pre-crop composite cropped to the rect.
        let cropped_view = {
            let mut d = Document::from_image(checker(100, 100));
            let mut l = Layer::new("dot", checker(10, 10));
            l.set_offset((30, 30));
            d.add_layer(l).unwrap();
            let mut m = Mark::new(MarkKind::Rect);
            m.rect = (40.0, 40.0, 60.0, 50.0);
            d.add_mark(m).unwrap();
            image::imageops::crop_imm(&d.flatten(), 20, 20, 50, 50).to_image()
        };
        assert_eq!(doc.flatten(), cropped_view);

        // Undo puts everything back at the original canvas position.
        doc.undo().unwrap();
        assert_eq!(doc.layers[1].offset(), (30, 30));
        assert_eq!(doc.mark(0).unwrap().rect, (40.0, 40.0, 60.0, 50.0));
    }

    #[test]
    fn background_offset_tracks_crop() {
        let mut doc = Document::from_image(checker(64, 64));
        doc.crop(Rect::new(12, 8, 40, 30)).unwrap();
        // 背景是普通图层：裁剪只是平移画布原点，像素原封不动，
        // offset 恒等于 (-view.x, -view.y)，与其它位图图层同一套规则。
        assert_eq!(doc.layers[0].offset(), (-12, -8));
        assert!(doc.layers[0].background);
        assert!(doc.layers[0].locked);
        assert_eq!(doc.layers[0].image().unwrap().dimensions(), (64, 64));
        assert_eq!(doc.canvas, Size::new(40, 30));
    }

    #[test]
    fn background_is_a_regular_locked_layer() {
        let mut doc = Document::from_image(checker(16, 16));
        let before = doc.flatten();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].name, "背景");
        assert!(doc.layers[0].locked && doc.layers[0].background);
        assert_eq!(doc.flatten(), before);

        // 解锁 = 一条可撤销的锁定属性变更，不改结构、不动索引。
        doc.set_layer_locked(0, false).unwrap();
        assert!(!doc.layers[0].locked);
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.flatten(), before);
        doc.undo().unwrap();
        assert!(doc.layers[0].locked);

        // 解锁后它就是普通图层：可移动、可改不透明度、可隐藏。
        doc.set_layer_locked(0, false).unwrap();
        doc.translate_layer(0, 4, 2).unwrap();
        assert_eq!(doc.layers[0].offset(), (4, 2));
        doc.set_layer_visible(0, false).unwrap();
        assert!(!doc.layers[0].visible);
        doc.set_layer_visible(0, true).unwrap();

        // 背景层不能从栈里删除（RemoveLayer 恒拒绝 0 号位）。
        assert!(doc.remove_layer(0).is_err());
        assert_eq!(doc.layers.len(), 1);

        // 「删除背景」= 清空像素，图层条目保留。
        doc.delete_base().unwrap();
        assert_eq!(doc.base_image().get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(doc.layers.len(), 1);
        doc.undo().unwrap();
        assert_eq!(doc.layers[0].image().unwrap().get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn move_layer_is_single_reorder_op() {        // Drag-reorder in the panel relies on move_layer(from, to) being one
        // undoable step that lands the layer exactly at `to`.
        let mut doc = Document::from_image(checker(16, 16));
        doc.add_layer(Layer::new("a", checker(2, 2))).unwrap();
        doc.add_layer(Layer::new("b", checker(2, 2))).unwrap();
        doc.add_layer(Layer::new("c", checker(2, 2))).unwrap();

        // 0 号位是背景，不可被占用：用户层在 1.. 之间排序。
        doc.move_layer(3, 1).unwrap();
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["背景", "c", "a", "b"]);

        doc.undo().unwrap();
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["背景", "a", "b", "c"]);
    }

    #[test]
    fn rotate_four_times_is_identity() {
        let mut doc = Document::from_image(corner_marked(20, 10));
        let original = doc.flatten();
        for _ in 0..4 {
            doc.apply_transform(Transform::Rotate90Cw).unwrap();
        }
        assert_eq!(doc.canvas, Size::new(20, 10));
        assert_eq!(doc.flatten(), original);
    }

    #[test]
    fn rotate90_swaps_canvas_and_moves_marked_corner() {
        let mut doc = Document::from_image(corner_marked(20, 10));
        assert_eq!(doc.composite_pixel(0, 0).0, [255, 255, 255, 255]);

        doc.apply_transform(Transform::Rotate90Cw).unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.composite_pixel(9, 0).0, [255, 255, 255, 255]);
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 255]);

        doc.apply_transform(Transform::Rotate90Ccw).unwrap();
        assert_eq!(doc.canvas, Size::new(20, 10));
        assert_eq!(doc.composite_pixel(0, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn flips_are_self_inverse_and_move_marked_corner() {
        for transform in [Transform::FlipHorizontal, Transform::FlipVertical] {
            let mut doc = Document::from_image(corner_marked(16, 8));
            doc.apply_transform(transform).unwrap();
            let expected = match transform {
                Transform::FlipHorizontal => (15, 0),
                _ => (0, 7),
            };
            assert_eq!(doc.composite_pixel(expected.0, expected.1).0, [255, 255, 255, 255]);
            doc.apply_transform(transform).unwrap();
            assert_eq!(doc.composite_pixel(0, 0).0, [255, 255, 255, 255]);
        }
    }

    #[test]
    fn transform_moves_layers_and_undo_restores_everything() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            20,
            10,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])),
        );
        layer.set_offset((1, 1));
        doc.add_layer(layer).unwrap();

        doc.apply_transform(Transform::Rotate90Cw).unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[1].offset(), (10 - (1 + 2), 1));
        assert_eq!(doc.composite_pixel(7, 1).0, [255, 255, 255, 255]);

        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(20, 10));
        assert_eq!(doc.layers[1].offset(), (1, 1));
        assert_eq!(doc.composite_pixel(1, 1).0, [255, 255, 255, 255]);

        doc.redo().unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[1].offset(), (7, 1));
    }

    #[test]
    fn rotate_ccw_moves_layer_to_expected_place() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            20,
            10,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])),
        );
        layer.set_offset((1, 1));
        doc.add_layer(layer).unwrap();

        doc.apply_transform(Transform::Rotate90Ccw).unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[1].offset(), (1, 20 - 1 - 2));
        assert_eq!(doc.composite_pixel(1, 17).0, [255, 255, 255, 255]);
    }

    fn corner_marked(width: u32, height: u32) -> RgbaImage {
        let mut image = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
        image.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
        image
    }

    fn ramp(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([x as u8, y as u8, 0, 255])
        })
    }

    #[test]
    fn crop_shrinks_canvas_and_shifts_visible_region() {
        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();

        assert_eq!(doc.canvas, Size::new(20, 12));
        assert_eq!(doc.view, Rect::new(10, 6, 20, 12));
        assert_eq!(doc.base_image().width(), 64);
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 6, 0, 255]);
        assert_eq!(doc.composite_pixel(19, 11).0, [29, 17, 0, 255]);
    }

    #[test]
    fn crop_undo_and_redo_restore_canvas_and_pixels() {
        let mut doc = Document::from_image(ramp(64, 48));
        let original = doc.flatten();

        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(64, 48));
        assert_eq!(doc.flatten(), original);

        doc.redo().unwrap();
        assert_eq!(doc.canvas, Size::new(20, 12));
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 6, 0, 255]);
    }

    #[test]
    fn crop_composes_and_clamps_to_the_image() {
        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.crop(Rect::new(2, 2, 100, 100)).unwrap();

        assert_eq!(doc.canvas, Size::new(18, 10));
        assert_eq!(doc.view, Rect::new(12, 8, 18, 10));
        assert_eq!(doc.composite_pixel(0, 0).0, [12, 8, 0, 255]);

        let mut doc = Document::from_image(ramp(16, 16));
        assert_eq!(doc.crop(Rect::new(20, 20, 4, 4)), Err(EditError::EmptyCrop));
        assert_eq!(doc.crop(Rect::new(0, 0, 0, 4)), Err(EditError::EmptyCrop));
    }

    #[test]
    fn reset_crop_shows_the_full_image_again() {
        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.reset_crop().unwrap();

        assert_eq!(doc.canvas, Size::new(64, 48));
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(doc.composite_pixel(63, 47).0, [63, 47, 0, 255]);
        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(20, 12));
    }

    #[test]
    fn crop_moves_layers_relative_to_the_new_origin() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            40,
            30,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
        );
        layer.set_offset((2, 3));
        doc.add_layer(layer).unwrap();

        doc.crop(Rect::new(5, 5, 20, 20)).unwrap();
        assert_eq!(doc.canvas, Size::new(20, 20));
        // 图层跟着内容走：画布原点移到 (5,5)，图层的画布偏移同步平移
        // -(5,5)，所以它相对图像内容的位置不变（画布 (0,0) 露出的是
        // 原图像 (5,5) 的像素，白点正好压在这个角上）。
        assert_eq!(doc.layers[1].offset(), (-3, -2));
        assert_eq!(doc.composite_pixel(0, 0).0, [255, 255, 255, 255]);

        doc.reset_crop().unwrap();
        assert_eq!(doc.layers[1].offset(), (2, 3));
        assert_eq!(doc.composite_pixel(2, 3).0, [255, 255, 255, 255]);
    }

    #[test]
    fn crop_to_ellipse_selection_leaves_transparent_corners() {
        // 纯色底 + 椭圆选区：裁到包围盒，椭圆之外必须透明。
        let mut doc = Document::from_image(solid(64, 48, [200, 100, 50, 255]));
        let sel = SelectionMask::ellipse(Rect::new(10, 6, 20, 12));
        doc.crop_to_selection(&sel).unwrap();

        assert_eq!(doc.canvas, Size::new(20, 12));
        assert_eq!(doc.view, Rect::new(10, 6, 20, 12));
        // 包围盒四角在椭圆外 → 透明；椭圆中心仍是原色。
        assert_eq!(doc.composite_pixel(0, 0).0[3], 0);
        assert_eq!(doc.composite_pixel(19, 11).0[3], 0);
        assert_eq!(doc.composite_pixel(10, 6).0, [200, 100, 50, 255]);
    }

    #[test]
    fn crop_to_selection_undo_restores_pixels_and_canvas() {
        let mut doc = Document::from_image(solid(64, 48, [7, 8, 9, 255]));
        let original = doc.flatten();
        doc.crop_to_selection(&SelectionMask::ellipse(Rect::new(10, 6, 20, 12)))
            .unwrap();
        doc.undo().unwrap();

        assert_eq!(doc.canvas, Size::new(64, 48));
        assert_eq!(doc.flatten(), original);
        // 被挖空的角落也回来了。
        assert_eq!(doc.composite_pixel(0, 0).0[3], 255);
    }

    #[test]
    fn crop_to_selection_punches_layers_too() {
        // 图层盖满整块画布：轮廓外的部分也必须透明，否则"裁剪"看起来没生效。
        let mut doc = Document::from_image(solid(40, 30, [0, 0, 0, 255]));
        let mut layer = Layer::new("full", solid(40, 30, [255, 255, 255, 255]));
        layer.set_offset((0, 0));
        doc.add_layer(layer).unwrap();

        doc.crop_to_selection(&SelectionMask::ellipse(Rect::new(5, 5, 20, 16)))
            .unwrap();
        assert_eq!(doc.canvas, Size::new(20, 16));
        assert_eq!(doc.layers[0].offset(), (-5, -5));
        // 角落在椭圆外：图层像素已挖空 → 合成 alpha 0（不再是白点）。
        assert_eq!(doc.composite_pixel(0, 0).0[3], 0);
        assert_eq!(doc.composite_pixel(10, 8).0[3], 255);
    }

    #[test]
    fn crop_then_rotate_keeps_the_view_on_the_same_content() {        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.apply_transform(Transform::Rotate90Cw).unwrap();

        assert_eq!(doc.canvas, Size::new(12, 20));
        assert_eq!(doc.view, Rect::new(48 - 6 - 12, 10, 12, 20));
        // 裁剪区左上角的内容在旋转后落到右上角。
        assert_eq!(doc.composite_pixel(11, 0).0, [10, 6, 0, 255]);
        assert_eq!(doc.composite_pixel(0, 19).0, [29, 17, 0, 255]);

        doc.apply_transform(Transform::Rotate90Ccw).unwrap();
        assert_eq!(doc.canvas, Size::new(20, 12));
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 6, 0, 255]);
    }

    fn solid(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, image::Rgba(color))
    }

    #[test]
    fn resize_sets_canvas_and_drops_crop() {
        let mut doc = Document::from_image(solid(64, 48, [10, 20, 30, 255]));
        doc.crop(Rect::new(8, 6, 32, 24)).unwrap();
        assert_eq!(doc.canvas, Size::new(32, 24));

        doc.resize(Size::new(16, 12)).unwrap();
        assert_eq!(doc.canvas, Size::new(16, 12));
        assert_eq!(doc.view, Rect::new(0, 0, 16, 12));
        assert_eq!(doc.base_image().dimensions(), (16, 12));
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn resize_upscale_grows_dimensions() {
        let mut doc = Document::from_image(solid(10, 5, [1, 2, 3, 255]));
        doc.resize(Size::new(30, 15)).unwrap();
        assert_eq!(doc.canvas, Size::new(30, 15));
        assert_eq!(doc.base_image().dimensions(), (30, 15));
        assert_eq!(doc.composite_pixel(29, 14).0, [1, 2, 3, 255]);
    }

    #[test]
    fn resize_undo_restores_base_view_and_layers() {
        let mut doc = Document::from_image(ramp(64, 48));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
        );
        layer.set_offset((6, 6));
        doc.add_layer(layer).unwrap();
        let original = doc.flatten();
        let original_layers = doc.layers.clone();

        doc.resize(Size::new(32, 24)).unwrap();
        assert_eq!(doc.canvas, Size::new(32, 24));

        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(64, 48));
        assert_eq!(doc.base_image().dimensions(), (64, 48));
        assert_eq!(doc.layers, original_layers);
        assert_eq!(doc.flatten(), original);

        doc.redo().unwrap();
        assert_eq!(doc.canvas, Size::new(32, 24));
    }

    #[test]
    fn resize_rejects_zero_dimensions() {
        let mut doc = Document::from_image(solid(16, 16, [0, 0, 0, 255]));
        assert_eq!(
            doc.resize(Size::new(0, 16)),
            Err(EditError::InvalidCanvasSize)
        );
        assert_eq!(
            doc.resize(Size::new(16, 0)),
            Err(EditError::InvalidCanvasSize)
        );
        assert_eq!(doc.canvas, Size::new(16, 16));
    }

    #[test]
    fn resize_scales_layer_offset_and_image() {
        let mut doc = Document::from_image(solid(40, 20, [0, 0, 0, 255]));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
        );
        layer.set_offset((10, 5));
        doc.add_layer(layer).unwrap();

        doc.resize(Size::new(20, 10)).unwrap();
        let l = &doc.layers[1];
        assert_eq!(l.offset(), (5, 3));
        assert_eq!(l.image().unwrap().dimensions(), (2, 2));
        assert_eq!(doc.composite_pixel(5, 3).0, [255, 255, 255, 255]);
    }

    #[test]
    fn canvas_resize_centers_content_without_resampling() {
        let mut doc = Document::from_image(ramp(8, 8));
        let original = doc.flatten();

        doc.resize_canvas(Size::new(12, 12)).unwrap();
        assert_eq!(doc.canvas, Size::new(12, 12));
        assert_eq!(doc.view, Rect::new(0, 0, 12, 12));
        assert_eq!(doc.base_image().dimensions(), (12, 12));
        // Old content is re-anchored to the center (offset +2/+2) and every
        // pixel is copied verbatim (no resampling).
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    doc.composite_pixel(x + 2, y + 2).0,
                    original.get_pixel(x as u32, y as u32).0
                );
            }
        }
        // Corners are transparent padding.
        assert_eq!(doc.composite_pixel(0, 0).0[3], 0);
        assert_eq!(doc.composite_pixel(11, 11).0[3], 0);
    }

    #[test]
    fn canvas_resize_shrink_clips_to_center_and_undoes() {
        let mut doc = Document::from_image(ramp(8, 8));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])),
        );
        layer.set_offset((0, 0));
        doc.add_layer(layer).unwrap();
        let original = doc.flatten();
        let original_layers = doc.layers.clone();

        doc.resize_canvas(Size::new(4, 4)).unwrap();
        assert_eq!(doc.canvas, Size::new(4, 4));
        // Composition re-anchored to the center: old (x+2, y+2) → new (x, y).
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(
                    doc.composite_pixel(x, y).0,
                    original.get_pixel((x + 2) as u32, (y + 2) as u32).0
                );
            }
        }
        // Layer shifted by (-2, -2); its pixels are untouched (still 2x2).
        assert_eq!(doc.layers[1].offset(), (-2, -2));
        assert_eq!(doc.layers[1].image().unwrap().dimensions(), (2, 2));

        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(8, 8));
        assert_eq!(doc.layers, original_layers);
        assert_eq!(doc.flatten(), original);
    }

    /// ★P0 验收：`flatten` 与改造前的算法逐像素一致。
    ///
    /// 改造前 = 背景（按 view 对齐）+ layers[1..] 依次叠加 + 最后统一画
    /// 所有标记。这里用 `image::imageops::overlay` 独立复算一遍当参照，
    /// 而不是拿新代码跟自己比。
    #[test]
    fn flatten_matches_legacy_composition() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            24,
            24,
            image::Rgba([10, 20, 30, 255]),
        ));
        let mut layer = Layer::new(
            "patch",
            RgbaImage::from_pixel(6, 6, image::Rgba([200, 100, 50, 255])),
        );
        layer.set_offset((4, 4));
        doc.add_layer(layer).unwrap();
        doc.add_mark(rect_mark(true)).unwrap();

        let mut legacy = doc.base_image().clone();
        image::imageops::overlay(&mut legacy, doc.layers[1].img(), 4, 4);
        doc.mark(0).unwrap().draw(&mut legacy);

        assert_eq!(doc.flatten(), legacy);
    }

    /// 标记进入 z 序之后才有的能力：它会被后加的位图图层盖住，也能自己
    /// 挪回上面。改造前标记恒在最顶层，这两个断言都不可能成立。
    #[test]
    fn mark_takes_part_in_z_order() {
        let mut doc = Document::from_image(blank(16, 16));
        let mi = doc.add_mark(rect_mark(true)).unwrap();
        let mark_layer = doc.mark_layer_index(mi).unwrap();
        let mut patch = Layer::new(
            "patch",
            RgbaImage::from_pixel(16, 16, image::Rgba([9, 9, 9, 255])),
        );
        patch.set_offset((0, 0));
        let patch_layer = doc.add_layer(patch).unwrap();

        // 后加的位图图层在标记之上 → 实心绿标记被整块盖住。
        assert_eq!(doc.flatten().get_pixel(5, 5).0, [9, 9, 9, 255]);

        // 把位图图层挪到标记下面 → 标记重新露出来（实心填充色）。
        doc.move_layer(patch_layer, mark_layer).unwrap();
        assert_eq!(doc.flatten().get_pixel(5, 5).0, [0, 255, 0, 255]);
    }

    /// 标记图层也走统一的 `offset` / `translate_layer`：拖动标记必须真的
    /// 动，而且能撤销（以前标记不在 layers 里，`offset()` 恒 (0,0)，拖动
    /// 被判定成「没位移」直接回滚）。
    #[test]
    fn mark_layer_moves_through_layer_api() {
        let mut doc = Document::from_image(blank(32, 32));
        doc.add_mark(rect_mark(false)).unwrap();
        let li = doc.mark_layer_index(0).unwrap();
        assert_eq!(doc.layers[li].offset(), (2, 2));

        // 直接改 offset：按位移平移标记。
        doc.layers[li].set_offset((10, 12));
        assert_eq!(doc.mark(0).unwrap().rect, (10.0, 12.0, 18.0, 18.0));

        // 走带历史的那条路：拖动落定 + 撤销。
        doc.translate_layer(li, 4, -2).unwrap();
        assert_eq!(doc.layers[li].offset(), (14, 10));
        doc.undo().unwrap();
        assert_eq!(doc.layers[li].offset(), (10, 12));
    }

    #[test]
    fn flatten_composites_translated_layer() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            16,
            16,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])),
        );
        layer.set_offset((5, 5));
        doc.add_layer(layer).unwrap();
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(5, 5).0, [255, 255, 255, 255]);
        assert_eq!(flat.get_pixel(0, 0).0, [0, 0, 0, 255]);
    }

    fn rect_mark(fill: bool) -> Mark {
        let mut m = Mark::new(MarkKind::Rect);
        m.rect = (2.0, 2.0, 10.0, 8.0);
        m.stroke = 2;
        m.color = [255, 0, 0, 255];
        m.fill = fill;
        m.fill_color = [0, 255, 0, 255];
        m
    }

    fn blank(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]))
    }

    #[test]
    fn rect_mark_renders_stroke_fill_and_history_roundtrips() {
        let mut doc = Document::from_image(blank(16, 16));
        let index = doc.add_mark(rect_mark(true)).unwrap();
        assert_eq!(index, 0);

        let flat = doc.flatten();
        // Stroke bands (2px) on the border, fill color in the interior.
        assert_eq!(flat.get_pixel(2, 2).0, [255, 0, 0, 255]);
        assert_eq!(flat.get_pixel(9, 3).0, [255, 0, 0, 255]);
        assert_eq!(flat.get_pixel(5, 5).0, [0, 255, 0, 255]);
        // Outside the mark: untouched.
        assert_eq!(flat.get_pixel(0, 0).0, [0, 0, 0, 255]);

        // Move + undo + redo roundtrip.
        let moved = doc.mark(0).unwrap().translated(3.0, 1.0);
        doc.update_mark(0, moved.clone()).unwrap();
        assert_eq!(doc.mark(0).unwrap().rect, (5.0, 3.0, 13.0, 9.0));
        doc.undo().unwrap();
        assert_eq!(doc.mark(0).unwrap().rect, (2.0, 2.0, 10.0, 8.0));
        doc.redo().unwrap();
        assert_eq!(doc.mark(0).unwrap().rect, (5.0, 3.0, 13.0, 9.0));

        // Delete + undo restores the exact mark.
        doc.delete_mark(0).unwrap();
        assert!(doc.mark_count() == 0);
        doc.undo().unwrap();
        assert_eq!(doc.mark_count(), 1);
        assert_eq!(*doc.mark(0).unwrap(), moved);
        // add→update→undo→redo left undo = [add, update]; three undos walk
        // back delete → update → add, leaving an empty (pre-add) state with
        // the whole chain replayable via redo.
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert!(doc.mark_count() == 0 && doc.can_redo());
    }

    #[test]
    fn rect_mark_without_fill_keeps_interior_transparent() {
        let mut doc = Document::from_image(blank(16, 16));
        doc.add_mark(rect_mark(false)).unwrap();
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(5, 5).0, [0, 0, 0, 255]);
        assert_eq!(flat.get_pixel(2, 4).0, [255, 0, 0, 255]);
    }

    #[test]
    fn hidden_mark_is_skipped_and_undo_restores_it() {
        let mut doc = Document::from_image(blank(32, 32));
        let mark = rect_mark(false);
        doc.add_mark(mark.clone()).unwrap();
        // (3,3) sits in the 2px stroke ring of rect (2,2)-(10,8).
        assert_eq!(doc.flatten().get_pixel(3, 3).0, [255, 0, 0, 255]);

        // 可见性现在是图层属性（Mark 不再自带 visible 字段）。
        let li = doc.mark_layer_index(0).unwrap();
        doc.set_layer_visible(li, false).unwrap();
        assert_eq!(doc.flatten().get_pixel(3, 3).0, [0, 0, 0, 255]);

        doc.undo().unwrap();
        assert_eq!(doc.flatten().get_pixel(3, 3).0, [255, 0, 0, 255]);
        assert!(doc.layers[li].visible);
    }

    #[test]
    fn ellipse_mark_paints_ring_and_interior() {
        let mut doc = Document::from_image(blank(32, 32));
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Ellipse);
            m.rect = (4.0, 4.0, 28.0, 28.0);
            m.stroke = 2;
            m.color = [255, 0, 0, 255];
            m.fill = true;
            m.fill_color = [0, 0, 255, 255];
            m
        })
        .unwrap();
        let flat = doc.flatten();
        // Center = fill; just inside the outline = stroke; outside = base.
        assert_eq!(flat.get_pixel(16, 16).0, [0, 0, 255, 255]);
        assert_eq!(flat.get_pixel(16, 5).0, [255, 0, 0, 255]);
        assert_eq!(flat.get_pixel(1, 1).0, [0, 0, 0, 255]);
    }

    #[test]
    fn arrow_mark_draws_shaft_and_head() {
        let mut doc = Document::from_image(blank(48, 24));
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Arrow);
            m.rect = (4.0, 12.0, 40.0, 12.0);
            m.points = vec![(4.0, 12.0), (40.0, 12.0)];
            m.stroke = 2;
            m.color = [255, 255, 0, 255];
            m
        })
        .unwrap();
        let flat = doc.flatten();
        // Shaft midpoint and inside the head triangle near the end point.
        assert_eq!(flat.get_pixel(20, 12).0, [255, 255, 0, 255]);
        assert_eq!(flat.get_pixel(38, 12).0, [255, 255, 0, 255]);
        assert_eq!(flat.get_pixel(20, 2).0, [0, 0, 0, 255]);
    }

    #[test]
    fn brush_mark_stamps_a_thick_line() {
        let mut doc = Document::from_image(blank(32, 32));
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Brush);
            m.rect = (4.0, 16.0, 28.0, 16.0);
            m.points = vec![(4.0, 16.0), (28.0, 16.0)];
            m.stroke = 4;
            m.color = [0, 255, 255, 255];
            m
        })
        .unwrap();
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(16, 16).0, [0, 255, 255, 255]);
        // 4px stroke → ±2px around the path (pixel centers at .5 offsets).
        assert_eq!(flat.get_pixel(16, 17).0, [0, 255, 255, 255]);
        assert_eq!(flat.get_pixel(16, 21).0, [0, 0, 0, 255]);
    }

    #[test]
    fn mosaic_mark_averages_block_content() {
        // Black base with a 2x2 white square at (8..10, 8..10): the cell
        // (8..12, 8..12) averages to floor((255*4 + 0*12) / 16) = 63.
        let mut base = blank(32, 32);
        for y in 8..10 {
            for x in 8..10 {
                base.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let mut doc = Document::from_image(base);
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Mosaic);
            m.rect = (8.0, 8.0, 12.0, 12.0);
            m.points = vec![(10.0, 10.0)];
            m.stroke = 8;
            m.cell = 4;
            m
        })
        .unwrap();
        let flat = doc.flatten();
        for y in 8..12 {
            for x in 8..12 {
                assert_eq!(flat.get_pixel(x, y).0, [63, 63, 63, 255]);
            }
        }
        // Outside the mosaic band: untouched.
        assert_eq!(flat.get_pixel(5, 5).0, [0, 0, 0, 255]);
        assert_eq!(flat.get_pixel(20, 20).0, [0, 0, 0, 255]);
    }

    #[test]
    fn mosaic_stamp_partially_outside_canvas_does_not_panic() {
        // Regression: stamp cells lying entirely outside the image skipped
        // their average push in pass 1, so pass 2 indexed avgs[] out of
        // bounds (len < cells.len()) — an abort during live drag past the
        // canvas edge. Empty cells must keep the two lists 1:1.
        let mut base = blank(32, 32);
        for y in 2..4 {
            for x in 28..30 {
                base.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let mut doc = Document::from_image(base);
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Mosaic);
            m.rect = (28.0, 0.0, 36.0, 8.0);
            m.points = vec![(32.0, 4.0)]; // center exactly on the right edge
            m.stroke = 8;
            m.cell = 4;
            m
        })
        .unwrap();
        let flat = doc.flatten();
        // Cells overlapping the canvas still average their pixels (white dot
        // + black -> mid gray), out-of-canvas cells are skipped silently.
        let p = flat.get_pixel(29, 3).0;
        assert!(p[0] > 0 && p[0] < 255, "expected blended cell, got {p:?}");
        assert_eq!(flat.get_pixel(3, 3).0, [0, 0, 0, 255]);
    }

    #[test]
    fn mosaic_region_mode_pixelates_whole_rect() {
        // Empty points = region mode: the entire rect is pixelated, not a
        // stamp path. A 2px checkerboard averages to ~127 in every 4px cell.
        let mut base = blank(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let v = if ((x / 2 + y / 2) % 2) == 0 { 255u8 } else { 0u8 };
                base.put_pixel(x, y, image::Rgba([v, v, v, 255]));
            }
        }
        let mut doc = Document::from_image(base);
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Mosaic);
            m.rect = (8.0, 8.0, 24.0, 24.0);
            m.points = Vec::new();
            m.cell = 4;
            m
        })
        .unwrap();
        let flat = doc.flatten();
        // Every pixel inside the rect is a flat cell average (~127), i.e.
        // the hard checkerboard texture is gone.
        for y in 8..24 {
            for x in 8..24 {
                let p = flat.get_pixel(x, y).0;
                assert_eq!(p[0], p[1]);
                assert_eq!(p[1], p[2]);
                assert!(p[0] > 100 && p[0] < 155, "cell average at ({x},{y}): {p:?}");
            }
        }
        // Outside the rect: untouched (raw checkerboard extremes remain).
        assert_eq!(flat.get_pixel(4, 4).0, [255, 255, 255, 255]);
        assert_eq!(flat.get_pixel(30, 32).0, [0, 0, 0, 255]);
    }

    #[test]
    fn text_mark_rasterizes_colored_pixels() {
        let mut doc = Document::from_image(blank(160, 80));
        let (tw, th) = editcore_measures();
        doc.add_mark({
            let mut m = Mark::new(MarkKind::Text);
            m.text = "测试Test".into();
            m.font = "msyh".into();
            m.font_size = 32.0;
            m.color = [0, 255, 0, 255];
            m.rect = (4.0, 4.0, 4.0 + tw, 4.0 + th);
            m
        })
        .unwrap();
        let flat = doc.flatten();
        let mut greens = 0;
        for y in 0..80 {
            for x in 0..160 {
                if flat.get_pixel(x, y).0 == [0, 255, 0, 255] {
                    greens += 1;
                }
            }
        }
        assert!(greens > 20, "expected anti-aliased ink, got {greens} px");
    }

    // Helper so the text test stays readable: natural size at 32px.
    fn editcore_measures() -> (f32, f32) {
        crate::font::measure_text("测试Test", "msyh", 32.0, false, false)
    }

    #[test]
    fn marks_follow_rotate_and_canvas_resize() {
        let mut doc = Document::from_image(blank(20, 10));
        doc.add_mark(rect_mark(false)).unwrap();

        doc.apply_transform(Transform::Rotate90Cw).unwrap();
        // Canvas is 10x20 now; the mark maps with the same point transform.
        let m = &doc.mark(0).unwrap();
        assert_eq!(m.rect.0.min(m.rect.2), 2.0); // old x0 == new y0 side
        assert_eq!(doc.canvas, Size::new(10, 20));

        doc.undo().unwrap();
        let before = doc.mark(0).unwrap().clone();
        doc.resize_canvas(Size::new(30, 20)).unwrap();
        // Center anchor: dx = (30-20)/2 = 5, dy = (20-10)/2 = 5.
        assert_eq!(doc.mark(0).unwrap().rect, (7.0, 7.0, 15.0, 13.0));
        doc.undo().unwrap();
        assert_eq!(*doc.mark(0).unwrap(), before);
    }

    #[test]
    fn resize_document_scales_marks() {
        let mut doc = Document::from_image(blank(20, 10));
        doc.add_mark(rect_mark(false)).unwrap();
        doc.resize(Size::new(40, 20)).unwrap();
        assert_eq!(doc.mark(0).unwrap().rect, (4.0, 4.0, 20.0, 16.0));
        assert_eq!(doc.mark(0).unwrap().stroke, 4);
    }

    // ---- 调整颜色 (color adjustment) ----

    fn grey_ramp(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, _| {
            let v = (x * 255 / width.max(1)) as u8;
            image::Rgba([v, v, v, 255])
        })
    }

    #[test]
    fn adjust_base_pixels_roundtrips_undo_and_redo() {
        let mut doc = Document::from_image(grey_ramp(32, 8));
        let before = doc.base_image().clone();
        let params = Adjust {
            exposure: 0.5,
            ..Adjust::identity()
        };
        doc.adjust_pixels(AdjustTarget::Base, &params).unwrap();

        assert!(doc.base_image().get_pixel(10, 0).0[0] > before.get_pixel(10, 0).0[0]);
        assert!(doc.can_undo());
        assert_eq!(doc.canvas, Size::new(32, 8));

        doc.undo().unwrap();
        assert_eq!(doc.base_image().as_raw(), before.as_raw());
        doc.redo().unwrap();
        // Redo recomputes from the stored pre-op image: deterministic.
        assert!(doc.base_image().get_pixel(10, 0).0[0] > before.get_pixel(10, 0).0[0]);
    }

    #[test]
    fn adjust_layer_pixels_only_touch_that_layer() {
        let mut doc = Document::from_image(grey_ramp(16, 4));
        let mut layer = Layer::new("1", RgbaImage::from_pixel(8, 4, image::Rgba([90, 90, 90, 255])));
        layer.set_offset((2, 0));
        doc.add_layer(layer).unwrap();
        let base_before = doc.base_image().clone();

        doc.adjust_pixels(
            AdjustTarget::Layer(1),
            &Adjust {
                saturation: -1.0,
                exposure: 0.4,
                ..Adjust::identity()
            },
        )
        .unwrap();

        assert_eq!(doc.base_image().as_raw(), base_before.as_raw());
        let px = doc.layers[1].image().unwrap().get_pixel(0, 0).0;
        assert!(px[0] > 90);
        assert_eq!(px[0], px[1]);
        assert_eq!(px[1], px[2]);
    }

    #[test]
    fn adjust_pixels_rejects_bad_layer_and_skips_neutral() {
        let mut doc = Document::from_image(blank(8, 8));
        assert!(matches!(
            doc.adjust_pixels(AdjustTarget::Layer(3), &Adjust::identity()),
            Err(EditError::LayerIndex(3))
        ));
        // A neutral parameter set is not a history entry.
        doc.adjust_pixels(AdjustTarget::Base, &Adjust::identity()).unwrap();
        assert!(!doc.can_undo());
    }

    #[test]
    fn scope_histogram_tracks_the_selected_scope() {
        let mut doc = Document::from_image(blank(16, 16));
        let mut layer = Layer::new("1", RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 0, 255])));
        layer.set_offset((0, 0));
        doc.add_layer(layer).unwrap();

        let base_hist = doc.scope_histogram(AdjustTarget::Base).unwrap();
        let layer_hist = doc.scope_histogram(AdjustTarget::Layer(1)).unwrap();
        // Base is opaque black: every pixel lands in bin 0.
        assert_eq!(base_hist.bin_counts(0)[0], 256);
        // The red layer: R saturates the top bin, G/B sit at the bottom.
        assert_eq!(layer_hist.bin_counts(0)[BINS - 1], 256);
        assert_eq!(layer_hist.bin_counts(1)[0], 256);
        assert_eq!(layer_hist.bin_counts(2)[0], 256);
    }
}

#[cfg(test)]
mod paint_tests {
    use super::*;

    fn two_halves() -> RgbaImage {
        RgbaImage::from_fn(20, 20, |x, _| {
            if x < 10 {
                image::Rgba([200, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 200, 255])
            }
        })
    }

    #[test]
    fn flood_fill_touches_only_the_contiguous_half() {
        let img = two_halves();
        let patch = flood_fill(
            &img,
            (0, 0),
            (2, 2),
            10,
            true,
            None,
            [0, 255, 0, 255],
            1.0,
        )
        .unwrap();
        // 右半边是蓝色，容差 10 不会越过去。
        assert_eq!(patch.rect, Rect::new(0, 0, 10, 20));
        assert_eq!(patch.image.get_pixel(0, 0).0, [0, 255, 0, 255]);
        // 原图未被改动（补丁只是返回值）。
        assert_eq!(img.get_pixel(0, 0).0, [200, 0, 0, 255]);
    }

    #[test]
    fn flood_fill_respects_selection_coverage() {
        let img = two_halves();
        let sel = SelectionMask::rect(Rect::new(5, 0, 5, 20));
        let patch = flood_fill(
            &img,
            (0, 0),
            (2, 2),
            10,
            true,
            Some(&sel),
            [0, 0, 0, 255],
            1.0,
        )
        .unwrap();
        assert_eq!(patch.rect, Rect::new(5, 0, 5, 20));
    }

    #[test]
    fn gradient_runs_from_dark_to_light() {
        let img = RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 255]));
        let patch = gradient_fill(
            &img,
            (0, 0),
            Size::new(10, 10),
            None,
            (0.0, 0.0),
            (10.0, 0.0),
            [0, 0, 0, 255],
            [255, 255, 255, 255],
            false,
            false,
            1.0,
            false,
        )
        .unwrap();
        let left = patch.image.get_pixel(0, 0).0[0];
        let right = patch.image.get_pixel(9, 0).0[0];
        assert!(right > left + 100, "left={left} right={right}");
    }

    #[test]
    fn spot_heal_pulls_surrounding_color_over_the_spot() {
        let mut img = RgbaImage::from_pixel(40, 40, image::Rgba([180, 180, 180, 255]));
        for y in 18..23 {
            for x in 18..23 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let mask = stroke_mask(
            &[(20.0, 20.0)],
            (0, 0),
            Size::new(40, 40),
            BrushSpec {
                size: 10.0,
                hardness: 0.6,
                spacing: 0.2,
            },
        )
        .unwrap();
        let patch = spot_heal(&img, &mask, 1.0).unwrap();
        let c = patch
            .image
            .get_pixel(
                (20 - patch.rect.x) as u32,
                (20 - patch.rect.y) as u32,
            )
            .0;
        assert!(
            c[0] > 120,
            "healed pixel should approach the gray background, got {c:?}"
        );
    }

    #[test]
    fn heal_clone_copies_texture_but_keeps_destination_color() {
        // 左边是黑白条纹（纹理），右边是均匀灰。修复画笔的语义是「取源的
        // 纹理、留目标的明暗色彩」，所以涂过的地方应该出现明暗起伏，但
        // 平均色仍接近灰。
        let mut img = RgbaImage::from_pixel(60, 20, image::Rgba([128, 128, 128, 255]));
        for y in 0..20 {
            for x in 0..20 {
                let v = if x % 4 < 2 { 40 } else { 220 };
                img.put_pixel(x, y, image::Rgba([v, v, v, 255]));
            }
        }
        let mask = stroke_mask(
            &[(40.0, 10.0)],
            (0, 0),
            Size::new(60, 20),
            BrushSpec {
                size: 8.0,
                hardness: 1.0,
                spacing: 0.2,
            },
        )
        .unwrap();
        let patch = heal_clone(&img, &mask, (40.0, 10.0), (10.0, 10.0), 1.0).unwrap();
        let mut lo = 255u8;
        let mut hi = 0u8;
        for y in 7..13 {
            for x in 37..43 {
                let c = patch
                    .image
                    .get_pixel((x - patch.rect.x) as u32, (y - patch.rect.y) as u32)
                    .0;
                lo = lo.min(c[0]);
                hi = hi.max(c[0]);
            }
        }
        assert!(hi - lo > 60, "texture should come through, lo={lo} hi={hi}");
    }

    #[test]
    fn patch_fill_copies_the_offset_region() {
        let mut img = RgbaImage::from_pixel(60, 20, image::Rgba([255, 255, 255, 255]));
        for y in 0..20 {
            for x in 0..20 {
                img.put_pixel(x, y, image::Rgba([0, 120, 0, 255]));
            }
        }
        let region = SelectionMask::rect(Rect::new(40, 0, 10, 20));
        let patch = patch_fill(&img, (0, 0), &region, (-40, 0), 0, 1.0).unwrap();
        let c = patch.image.get_pixel((40 - patch.rect.x) as u32, 10).0;
        assert!(c[1] > 80, "expected green sample, got {c:?}");
    }

    #[test]
    fn rotate_image_hq_matches_geometry_but_keeps_more_detail() {
        // 4px 周期的高频条纹：双线性相当于一次低通，对比度必掉；双三次
        // 带负瓣，掉的应该明显更少（这正是"转完发软"的量化形式）。
        let mut img = RgbaImage::from_pixel(240, 240, image::Rgba([255, 255, 255, 255]));
        for x in (0..240).step_by(4) {
            for y in 0..240 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let (bil, d1) = rotate_image(&img, 37.0, true);
        let (hq, d2) = rotate_image_hq(&img, 37.0, true);
        // 几何完全一致：尺寸与偏移不能变，否则图层会跳。
        assert_eq!(bil.dimensions(), hq.dimensions());
        assert_eq!(d1, d2);
        let detail = |im: &RgbaImage| -> f64 {
            // 取中间一条横带，算相邻像素差的平均绝对值（越高越锐）。
            let y = im.height() / 2;
            let mut sum = 0f64;
            for x in 1..im.width() {
                let a = im.get_pixel(x, y).0[0] as f64;
                let b = im.get_pixel(x - 1, y).0[0] as f64;
                sum += (a - b).abs();
            }
            sum / (im.width() - 1) as f64
        };
        let (db, dh) = (detail(&bil), detail(&hq));
        assert!(dh > db * 1.05, "hq should be sharper: bilinear={db:.1} hq={dh:.1}");
    }

    #[test]
    fn rotate_image_expands_and_keeps_the_center() {
        let mut img = RgbaImage::from_pixel(10, 10, image::Rgba([255, 255, 255, 255]));
        img.put_pixel(5, 5, image::Rgba([0, 0, 0, 255]));
        let (out, delta) = rotate_image(&img, 45.0, true);
        // 对角线 ceil(10√2)=15 → **向上取偶** 16：偶数尺寸让内容中心恒落在
        // 整像素上（见 expanded_size 的说明），否则拖动帧会逐帧抖半像素。
        assert_eq!(out.width(), 16, "对角线 15 取偶 → 16");
        assert_eq!(out.height(), 16);
        // 黑点仍存在，且位于输出画面的中心附近（旋转不移动中心）。
        let mut dark = (0u32, 0u32, 0u32);
        for y in 0..out.height() {
            for x in 0..out.width() {
                let c = out.get_pixel(x, y).0;
                if c[0] < 60 {
                    dark.0 += x;
                    dark.1 += y;
                    dark.2 += 1;
                }
            }
        }
        assert!(dark.2 > 0, "the dark dot should survive the rotation");
        let cx = dark.0 as f32 / dark.2 as f32;
        let cy = dark.1 as f32 / dark.2 as f32;
        assert!((cx - out.width() as f32 / 2.0).abs() < 1.5, "cx={cx}");
        assert!((cy - out.height() as f32 / 2.0).abs() < 1.5, "cy={cy}");
        // 左上角向外扩，偏移为负。
        assert!(delta.0 <= 0 && delta.1 <= 0);
    }

    /// 拖动预览的稳定性契约：整段手势里输出尺寸与偏移必须**完全不变**，
    /// 否则每帧 ±1px 的尺寸/偏移跳变就是肉眼看到的"抖动"。
    #[test]
    fn rotate_into_keeps_output_size_and_offset_stable() {
        // 100×60 的实心块：扫一圈角度，输出尺寸与偏移都该是同一个值。
        let img = RgbaImage::from_pixel(100, 60, image::Rgba([200, 120, 40, 255]));
        let (nw, nh) = rotated_box_size(100, 60, true);
        assert_eq!((nw, nh), (120, 120), "对角线正方形：ceil(hypot(100,60)) 取偶 + 2px 余量");
        let mut first: Option<(i32, i32)> = None;
        for deg in [0.0f32, 3.25, 17.0, 45.0, 61.5, 89.9, 120.0, 180.0] {
            let rad = deg.to_radians();
            let (out, delta) = rotate_into(&img, rad.sin(), rad.cos(), nw, nh);
            assert_eq!((out.width(), out.height()), (nw, nh), "size at {deg}");
            match first {
                None => first = Some(delta),
                Some(d) => assert_eq!(d, delta, "offset drifted at {deg}°"),
            }
            // 内容仍居中：实心块的重心落在画布中心 ±1px。
            let (mut sx, mut sy, mut n) = (0f64, 0f64, 0f64);
            for y in 0..out.height() {
                for x in 0..out.width() {
                    if out.get_pixel(x, y).0[3] > 0 {
                        sx += x as f64;
                        sy += y as f64;
                        n += 1.0;
                    }
                }
            }
            assert!(n > 0.0);
            assert!((sx / n - nw as f64 / 2.0).abs() < 1.5, "cx off at {deg}°");
            assert!((sy / n - nh as f64 / 2.0).abs() < 1.5, "cy off at {deg}°");
        }
    }

    /// 直角旋转必须是**精确**的：尺寸交换、偏移正好是半差、像素不插值。
    ///
    /// 这条曾经是 bug：`sin(90°)` 是 6.1e-17 而不是 0，`800*|cos| + 600*|sin|`
    /// 算出 600.00000000000006，`ceil` 直接进到 601 —— 每转一次图层长 1px。
    #[test]
    fn quarter_turn_is_exact_not_grown_by_rounding() {
        let mut img = RgbaImage::from_pixel(800, 600, image::Rgba([30, 40, 50, 255]));
        for y in 0..20 {
            for x in 0..20 {
                img.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        for (deg, want) in [
            (90.0f32, (600u32, 800u32)),
            (180.0, (800, 600)),
            (270.0, (600, 800)),
            (360.0, (800, 600)),
        ] {
            let (out, delta) = rotate_image_hq(&img, deg, true);
            assert_eq!(
                out.dimensions(),
                want,
                "{deg}° 后尺寸应正好是 {want:?}，多 1px 就是累积膨胀"
            );
            // 偏移 = (旧尺寸 - 新尺寸) / 2，直角下必然整除。
            assert_eq!(
                delta,
                (
                    (800 - want.0 as i32) / 2,
                    (600 - want.1 as i32) / 2
                ),
                "{deg}° 的偏移不能有半像素误差"
            );
        }
    }

    /// 用户"转一圈"：四次松手提交，每次都以未插值的原图 + 累计角度重算
    /// （`rotate_layer_finish` 的 keep 分支就是这样推进 `RotateOrigin` 的）。
    /// 一圈之后尺寸、偏移、像素都必须回到出发点。
    #[test]
    fn full_turn_over_origin_returns_to_start() {
        let mut base = RgbaImage::from_pixel(640, 480, image::Rgba([200, 160, 90, 255]));
        for y in 0..40 {
            for x in 0..40 {
                base.put_pixel(x, y, image::Rgba([10, 220, 10, 255]));
            }
        }
        let mut last = base.clone();
        for step in 1..=4 {
            let deg = 90.0 * step as f32;
            let (out, delta) = rotate_image_hq(&base, deg, true);
            // 每次提交后 offset = 原始 offset + delta（不是逐次累加）。
            assert_eq!(delta, ((640 - out.width() as i32) / 2, (480 - out.height() as i32) / 2));
            last = out;
        }
        assert_eq!(last.dimensions(), (640, 480), "转满一圈尺寸必须回到原值");
        let mut worst = 0i32;
        for (a, b) in last.pixels().zip(base.pixels()) {
            for c in 0..4 {
                worst = worst.max((a.0[c] as i32 - b.0[c] as i32).abs());
            }
        }
        assert!(worst <= 1, "转满一圈像素误差 {worst} 过大，说明在反复重采样");
    }

    #[test]
    fn rotate_layer_angle_is_undoable() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            20,
            10,
            image::Rgba([10, 20, 30, 255]),
        ));
        doc.add_layer(Layer::new(
            "l",
            RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255])),
        ))
        .unwrap();
        let before = doc.layers[0].clone();
        doc.rotate_layer_angle(0, 30.0, true).unwrap();
        assert!(doc.layers[0].image().unwrap().width() > 8);
        doc.undo().unwrap();
        assert_eq!(doc.layers[0], before);
    }

    #[test]
    fn pixel_patch_undo_restores_only_the_rectangle() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            20,
            20,
            image::Rgba([10, 10, 10, 255]),
        ));
        let before = crop_region(doc.base_image(), Rect::new(4, 4, 6, 6));
        let after = RgbaImage::from_pixel(6, 6, image::Rgba([200, 0, 0, 255]));
        doc.commit_patch(AdjustTarget::Base, before.0, before.1, after)
            .unwrap();
        assert_eq!(doc.base_image().get_pixel(5, 5).0, [200, 0, 0, 255]);
        doc.undo().unwrap();
        assert_eq!(doc.base_image().get_pixel(5, 5).0, [10, 10, 10, 255]);
        assert_eq!(doc.base_image().get_pixel(0, 0).0, [10, 10, 10, 255]);
    }
}

