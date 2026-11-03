use image::RgbaImage;

mod adjust;
mod color;
pub mod font;
pub use adjust::{adjust_json, Adjust, AdjustTarget, Histogram, BINS};
pub use color::{cmyk_of, hsl_of, PickedColor};
pub use font::{measure_text, FONTS};


mod geometry;
mod mark;
mod layer;
mod transform;
mod op;
mod ops;
mod document;
mod selection;

pub use geometry::{Size, Rect, Selection};
pub use selection::{SelOps, SelShape, SelectionMask, SelectionState, WandOptions};
pub use mark::{MarkKind, Mark};
pub use layer::Layer;
pub use transform::Transform;
pub use op::{EditError, EditOp};
pub use document::Document;
pub(crate) use document::DocumentSnapshot;

// Cross-module private helpers (kept `pub(crate)` + re-exported so each
// sibling module's `use super::*` can call them without naming the path).
pub(crate) use mark::norm_rect;
pub(crate) use transform::transform_document;
pub(crate) use op::History;
pub(crate) use ops::{
    apply_base_to_layer, apply_canvas_resize, apply_delete_base, apply_resize, apply_view,
    clear_outside_selection, erase_base_pixels,
};

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
        assert_eq!(layer.image.width(), 16);
        assert_eq!(layer.image.height(), 12);
        assert_eq!(layer.offset, (8, 4));
        assert_eq!(layer.image.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(layer.image.get_pixel(0, 4).0, [255, 0, 0, 255]);
    }

    #[test]
    fn ellipse_selection_masks_corners() {
        let doc = Document::from_image(checker(64, 64));
        let layer = doc
            .selection_layer(&SelectionMask::ellipse(Rect::new(0, 0, 64, 64)))
            .unwrap();
        assert_eq!(layer.image.get_pixel(0, 0).0[3], 0);
        assert_eq!(layer.image.get_pixel(32, 32).0[3], 255);
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
        doc.add_layer(Layer::new("clip", checker(4, 4))).unwrap();
        assert_eq!(doc.layers.len(), 1);
        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 0);
        doc.redo().unwrap();
        assert_eq!(doc.layers.len(), 1);
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
        assert_eq!(doc.layers[index].offset, (0, 0));
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
        assert_eq!(dst.layers[pasted].offset, (4, 6));
        assert_eq!(dst.layers[pasted].image.width(), 8);
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
        assert_eq!(doc.layers[index].image.get_pixel(0, 0).0[3], 255);
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
        assert_eq!(doc.base.get_pixel(8, 8).0, [0, 0, 0, 0]);
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
        assert_eq!(layer.image.width(), 16);
        assert_eq!(layer.image.height(), 8);
        // Offset (top-left anchor) is untouched by scaling.
        assert_eq!(layer.offset, (5, 3));

        doc.undo().unwrap();
        assert_eq!(doc.layers[index].image.width(), 8);
        assert_eq!(doc.layers[index].image.height(), 4);
        assert_eq!(doc.layers[index].offset, (5, 3));
        doc.redo().unwrap();
        assert_eq!(doc.layers[index].image.width(), 16);
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
        assert_eq!(layer.image.get_pixel(0, 0), &image::Rgba([0, 0, 255, 255]));
        assert_eq!(layer.image.get_pixel(3, 0), &image::Rgba([255, 0, 0, 255]));
        // 图层自身的框不动（以自身为轴翻转），base 一个像素都没变。
        assert_eq!(layer.offset, (0, 0));
        assert_eq!(doc.base, base);

        doc.undo().unwrap();
        assert_eq!(
            doc.layers[index].image.get_pixel(0, 0),
            &image::Rgba([255, 0, 0, 255])
        );
        doc.redo().unwrap();
        assert_eq!(
            doc.layers[index].image.get_pixel(0, 0),
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
        let px = *doc.layers[index].image.get_pixel(0, 0);
        assert!(px[2] > 192 && px[0] < 64, "left edge should be blue: {px:?}");
        let right = *doc.layers[index].image.get_pixel(7, 0);
        assert!(
            right[0] > 192 && right[2] < 64,
            "right edge should be red: {right:?}"
        );

        // 撤销回到翻转前（左红）。
        doc.undo().unwrap();
        let px = *doc.layers[index].image.get_pixel(0, 0);
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
        assert_eq!(doc.layers[index].image.width(), 4);
    }

    #[test]
    fn set_base_visible_roundtrips_and_affects_composite() {
        let mut doc = Document::from_image(checker(8, 8));
        doc.set_base_visible(false).unwrap();
        assert!(doc.base_hidden);
        // Hidden base composites as fully transparent.
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 0]);

        doc.undo().unwrap();
        assert!(!doc.base_hidden);
        assert_eq!(doc.composite_pixel(0, 0).0[3], 255);
        doc.redo().unwrap();
        assert!(doc.base_hidden);

        // Setting the same state again must not push a history entry.
        let depth = doc.history.undo.len();
        doc.set_base_visible(false).unwrap();
        assert_eq!(doc.history.undo.len(), depth);
        doc.set_base_visible(true).unwrap();
        assert_eq!(doc.history.undo.len(), depth + 1);
        assert!(!doc.base_hidden);
    }

    #[test]
    fn delete_base_requires_layers_and_roundtrips() {
        let mut doc = Document::from_image(checker(16, 16));
        // No layers yet: deleting the base is rejected.
        assert_eq!(doc.delete_base(), Err(EditError::BaseRequired));

        let index = doc.add_layer(Layer::new("top", checker(4, 4))).unwrap();
        doc.delete_base().unwrap();
        assert!(doc.base_deleted);
        assert_eq!(doc.base.width(), 16);
        assert_eq!(doc.base.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(doc.layers[index].image.width(), 4);

        doc.undo().unwrap();
        assert!(!doc.base_deleted);
        assert_eq!(doc.base.get_pixel(0, 0).0, [255, 0, 0, 255]);
        doc.redo().unwrap();
        assert!(doc.base_deleted);
        assert_eq!(doc.base.get_pixel(15, 15).0, [0, 0, 0, 0]);
    }

    #[test]
    fn crop_shifts_layers_and_marks_with_the_content() {
        let mut doc = Document::from_image(checker(100, 100));
        let mut layer = Layer::new("dot", checker(10, 10));
        layer.offset = (30, 30);
        doc.add_layer(layer).unwrap();
        let mut mark = Mark::new(MarkKind::Rect);
        mark.rect = (40.0, 40.0, 60.0, 50.0);
        doc.add_mark(mark).unwrap();

        doc.crop(Rect::new(20, 20, 50, 50)).unwrap();
        // The layer kept its place relative to the image content: its canvas
        // offset moved by -(crop origin).
        assert_eq!(doc.layers[0].offset, (10, 10));
        // mark.rect is (x1, y1, x2, y2) corner coordinates.
        assert_eq!(doc.marks[0].rect, (20.0, 20.0, 40.0, 30.0));
        // Flatten now equals the pre-crop composite cropped to the rect.
        let cropped_view = {
            let mut d = Document::from_image(checker(100, 100));
            let mut l = Layer::new("dot", checker(10, 10));
            l.offset = (30, 30);
            d.add_layer(l).unwrap();
            let mut m = Mark::new(MarkKind::Rect);
            m.rect = (40.0, 40.0, 60.0, 50.0);
            d.add_mark(m).unwrap();
            image::imageops::crop_imm(&d.flatten(), 20, 20, 50, 50).to_image()
        };
        assert_eq!(doc.flatten(), cropped_view);

        // Undo puts everything back at the original canvas position.
        doc.undo().unwrap();
        assert_eq!(doc.layers[0].offset, (30, 30));
        assert_eq!(doc.marks[0].rect, (40.0, 40.0, 60.0, 50.0));
    }

    #[test]
    fn convert_base_to_layer_composites_identically_after_crop() {
        let mut doc = Document::from_image(checker(64, 64));
        doc.crop(Rect::new(12, 8, 40, 30)).unwrap();
        let cropped = doc.flatten();

        doc.convert_base_to_layer().unwrap();
        // Uncropped documents keep offset (0,0); after a crop the layer's
        // top-left sits at (-view.x, -view.y) so base[view.x + x] shows at
        // canvas x and the composite stays pixel-identical.
        assert_eq!(doc.layers[0].offset, (-12, -8));
        assert_eq!(doc.flatten(), cropped);
    }

    #[test]
    fn convert_base_to_layer_roundtrips_and_composites_identically() {
        let mut doc = Document::from_image(checker(16, 16));
        let before = doc.flatten();

        doc.convert_base_to_layer().unwrap();
        assert!(doc.base_converted);
        // The base pixels now live in a regular bottom layer named 背景.
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].name, "背景");
        assert_eq!(doc.layers[0].offset, (0, 0));
        assert_eq!(doc.layers[0].image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert!(!doc.layers[0].locked);
        // The base itself is a transparent canvas of the same size; the
        // composite output is pixel-identical.
        assert_eq!(doc.base.width(), 16);
        assert_eq!(doc.base.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(doc.canvas.width, 16);
        assert_eq!(doc.flatten(), before);

        // Converting twice must not push a second history entry.
        let depth = doc.history.undo.len();
        doc.convert_base_to_layer().unwrap();
        assert_eq!(doc.history.undo.len(), depth);

        // The converted layer behaves like any other layer: movable and
        // deletable. After deletion no trace remains in the layer list.
        doc.translate_layer(0, 4, 2).unwrap();
        assert_eq!(doc.layers[0].offset, (4, 2));
        doc.remove_layer(0).unwrap();
        assert!(doc.layers.is_empty());
        assert_eq!(doc.flatten(), RgbaImage::new(16, 16));

        // Undo the deletion: the layer (still converted) comes back.
        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].offset, (4, 2));
        assert!(doc.base_converted);

        // Undo past translate AND the conversion restores the original
        // locked-in-base state exactly.
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert!(!doc.base_converted);
        assert!(doc.layers.is_empty());
        assert_eq!(doc.base.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(doc.flatten(), before);

        // Redo re-applies the conversion.
        doc.redo().unwrap();
        assert!(doc.base_converted);
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn move_layer_is_single_reorder_op() {        // Drag-reorder in the panel relies on move_layer(from, to) being one
        // undoable step that lands the layer exactly at `to`.
        let mut doc = Document::from_image(checker(16, 16));
        doc.add_layer(Layer::new("a", checker(2, 2))).unwrap();
        doc.add_layer(Layer::new("b", checker(2, 2))).unwrap();
        doc.add_layer(Layer::new("c", checker(2, 2))).unwrap();

        doc.move_layer(2, 0).unwrap();
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["c", "a", "b"]);

        doc.undo().unwrap();
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
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
        layer.offset = (1, 1);
        doc.add_layer(layer).unwrap();

        doc.apply_transform(Transform::Rotate90Cw).unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[0].offset, (10 - (1 + 2), 1));
        assert_eq!(doc.composite_pixel(7, 1).0, [255, 255, 255, 255]);

        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(20, 10));
        assert_eq!(doc.layers[0].offset, (1, 1));
        assert_eq!(doc.composite_pixel(1, 1).0, [255, 255, 255, 255]);

        doc.redo().unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[0].offset, (7, 1));
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
        layer.offset = (1, 1);
        doc.add_layer(layer).unwrap();

        doc.apply_transform(Transform::Rotate90Ccw).unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[0].offset, (1, 20 - 1 - 2));
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
        assert_eq!(doc.base.width(), 64);
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
        layer.offset = (2, 3);
        doc.add_layer(layer).unwrap();

        doc.crop(Rect::new(5, 5, 20, 20)).unwrap();
        assert_eq!(doc.canvas, Size::new(20, 20));
        // 图层跟着内容走：画布原点移到 (5,5)，图层的画布偏移同步平移
        // -(5,5)，所以它相对图像内容的位置不变（画布 (0,0) 露出的是
        // 原图像 (5,5) 的像素，白点正好压在这个角上）。
        assert_eq!(doc.layers[0].offset, (-3, -2));
        assert_eq!(doc.composite_pixel(0, 0).0, [255, 255, 255, 255]);

        doc.reset_crop().unwrap();
        assert_eq!(doc.layers[0].offset, (2, 3));
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
        layer.offset = (0, 0);
        doc.add_layer(layer).unwrap();

        doc.crop_to_selection(&SelectionMask::ellipse(Rect::new(5, 5, 20, 16)))
            .unwrap();
        assert_eq!(doc.canvas, Size::new(20, 16));
        assert_eq!(doc.layers[0].offset, (-5, -5));
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
        assert_eq!(doc.base.dimensions(), (16, 12));
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn resize_upscale_grows_dimensions() {
        let mut doc = Document::from_image(solid(10, 5, [1, 2, 3, 255]));
        doc.resize(Size::new(30, 15)).unwrap();
        assert_eq!(doc.canvas, Size::new(30, 15));
        assert_eq!(doc.base.dimensions(), (30, 15));
        assert_eq!(doc.composite_pixel(29, 14).0, [1, 2, 3, 255]);
    }

    #[test]
    fn resize_undo_restores_base_view_and_layers() {
        let mut doc = Document::from_image(ramp(64, 48));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (6, 6);
        doc.add_layer(layer).unwrap();
        let original = doc.flatten();
        let original_layers = doc.layers.clone();

        doc.resize(Size::new(32, 24)).unwrap();
        assert_eq!(doc.canvas, Size::new(32, 24));

        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(64, 48));
        assert_eq!(doc.base.dimensions(), (64, 48));
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
        layer.offset = (10, 5);
        doc.add_layer(layer).unwrap();

        doc.resize(Size::new(20, 10)).unwrap();
        let l = &doc.layers[0];
        assert_eq!(l.offset, (5, 3));
        assert_eq!(l.image.dimensions(), (2, 2));
        assert_eq!(doc.composite_pixel(5, 3).0, [255, 255, 255, 255]);
    }

    #[test]
    fn canvas_resize_centers_content_without_resampling() {
        let mut doc = Document::from_image(ramp(8, 8));
        let original = doc.flatten();

        doc.resize_canvas(Size::new(12, 12)).unwrap();
        assert_eq!(doc.canvas, Size::new(12, 12));
        assert_eq!(doc.view, Rect::new(0, 0, 12, 12));
        assert_eq!(doc.base.dimensions(), (12, 12));
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
        layer.offset = (0, 0);
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
        assert_eq!(doc.layers[0].offset, (-2, -2));
        assert_eq!(doc.layers[0].image.dimensions(), (2, 2));

        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(8, 8));
        assert_eq!(doc.layers, original_layers);
        assert_eq!(doc.flatten(), original);
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
        layer.offset = (5, 5);
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
        let moved = doc.marks[0].translated(3.0, 1.0);
        doc.update_mark(0, moved.clone()).unwrap();
        assert_eq!(doc.marks[0].rect, (5.0, 3.0, 13.0, 9.0));
        doc.undo().unwrap();
        assert_eq!(doc.marks[0].rect, (2.0, 2.0, 10.0, 8.0));
        doc.redo().unwrap();
        assert_eq!(doc.marks[0].rect, (5.0, 3.0, 13.0, 9.0));

        // Delete + undo restores the exact mark.
        doc.delete_mark(0).unwrap();
        assert!(doc.marks.is_empty());
        doc.undo().unwrap();
        assert_eq!(doc.marks.len(), 1);
        assert_eq!(doc.marks[0], moved);
        // add→update→undo→redo left undo = [add, update]; three undos walk
        // back delete → update → add, leaving an empty (pre-add) state with
        // the whole chain replayable via redo.
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert!(doc.marks.is_empty() && doc.can_redo());
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

        let mut hidden = mark.clone();
        hidden.visible = false;
        doc.update_mark(0, hidden).unwrap();
        assert_eq!(doc.flatten().get_pixel(3, 3).0, [0, 0, 0, 255]);

        doc.undo().unwrap();
        assert_eq!(doc.flatten().get_pixel(3, 3).0, [255, 0, 0, 255]);
        assert!(doc.marks[0].visible);
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
        let m = &doc.marks[0];
        assert_eq!(m.rect.0.min(m.rect.2), 2.0); // old x0 == new y0 side
        assert_eq!(doc.canvas, Size::new(10, 20));

        doc.undo().unwrap();
        let before = doc.marks[0].clone();
        doc.resize_canvas(Size::new(30, 20)).unwrap();
        // Center anchor: dx = (30-20)/2 = 5, dy = (20-10)/2 = 5.
        assert_eq!(doc.marks[0].rect, (7.0, 7.0, 15.0, 13.0));
        doc.undo().unwrap();
        assert_eq!(doc.marks[0], before);
    }

    #[test]
    fn resize_document_scales_marks() {
        let mut doc = Document::from_image(blank(20, 10));
        doc.add_mark(rect_mark(false)).unwrap();
        doc.resize(Size::new(40, 20)).unwrap();
        assert_eq!(doc.marks[0].rect, (4.0, 4.0, 20.0, 16.0));
        assert_eq!(doc.marks[0].stroke, 4);
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
        let before = doc.base.clone();
        let params = Adjust {
            exposure: 0.5,
            ..Adjust::identity()
        };
        doc.adjust_pixels(AdjustTarget::Base, &params).unwrap();

        assert!(doc.base.get_pixel(10, 0).0[0] > before.get_pixel(10, 0).0[0]);
        assert!(doc.can_undo());
        assert_eq!(doc.canvas, Size::new(32, 8));

        doc.undo().unwrap();
        assert_eq!(doc.base.as_raw(), before.as_raw());
        doc.redo().unwrap();
        // Redo recomputes from the stored pre-op image: deterministic.
        assert!(doc.base.get_pixel(10, 0).0[0] > before.get_pixel(10, 0).0[0]);
    }

    #[test]
    fn adjust_layer_pixels_only_touch_that_layer() {
        let mut doc = Document::from_image(grey_ramp(16, 4));
        let mut layer = Layer::new("1", RgbaImage::from_pixel(8, 4, image::Rgba([90, 90, 90, 255])));
        layer.offset = (2, 0);
        doc.add_layer(layer).unwrap();
        let base_before = doc.base.clone();

        doc.adjust_pixels(
            AdjustTarget::Layer(0),
            &Adjust {
                saturation: -1.0,
                exposure: 0.4,
                ..Adjust::identity()
            },
        )
        .unwrap();

        assert_eq!(doc.base.as_raw(), base_before.as_raw());
        let px = doc.layers[0].image.get_pixel(0, 0).0;
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
        layer.offset = (0, 0);
        doc.add_layer(layer).unwrap();

        let base_hist = doc.scope_histogram(AdjustTarget::Base).unwrap();
        let layer_hist = doc.scope_histogram(AdjustTarget::Layer(0)).unwrap();
        // Base is opaque black: every pixel lands in bin 0.
        assert_eq!(base_hist.bin_counts(0)[0], 256);
        // The red layer: R saturates the top bin, G/B sit at the bottom.
        assert_eq!(layer_hist.bin_counts(0)[BINS - 1], 256);
        assert_eq!(layer_hist.bin_counts(1)[0], 256);
        assert_eq!(layer_hist.bin_counts(2)[0], 256);
    }
}
