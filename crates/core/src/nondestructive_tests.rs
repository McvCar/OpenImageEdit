//! P0-1 接线验收：非破坏性缩放、自动烘焙、参数化变换历史。
//! 对应 docs/变换系统重构-修改路线.md §2.3 / §2.4 / §5。

use crate::*;

fn checker(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        if (x / 8 + y / 8) % 2 == 0 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        }
    })
}

fn doc_with_layer(src: RgbaImage) -> Document {
    let (w, h) = (src.width(), src.height());
    let mut doc = Document::from_image(RgbaImage::new(w, h));
    doc.layers.push(Layer::new("L", src));
    doc
}

#[test]
fn scale_down_then_up_samples_the_original_pixels() {
    let src = checker(80, 80);
    let mut doc = doc_with_layer(src.clone());

    doc.scale_layer(0, Size::new(40, 40), (0, 0)).unwrap();
    assert_eq!(doc.layers[0].image.dimensions(), (40, 40));
    assert!(doc.layers[0].transform.is_some(), "缩放后必须保留原始像素");

    doc.scale_layer(0, Size::new(80, 80), (0, 0)).unwrap();
    assert_eq!(doc.layers[0].image.dimensions(), (80, 80));

    // P0-2a：回到原始尺寸走 1:1 索引拷贝，连「从原图做一次 80→80 的
    // Lanczos3」都不做 —— 后者带负旁瓣，会把单像素细线 / 文字笔画抹软。
    let destructive = image::imageops::resize(
        &image::imageops::resize(&src, 40, 40, image::imageops::FilterType::Lanczos3),
        80,
        80,
        image::imageops::FilterType::Lanczos3,
    );
    assert_eq!(
        doc.layers[0].image.as_raw(),
        src.as_raw(),
        "缩放→放大回原尺寸必须逐像素等于原图"
    );
    assert!(
        doc.layers[0]
            .transform
            .as_ref()
            .unwrap()
            .is_identity_fast_path(),
        "回到原尺寸后变换状态应是 identity"
    );
    assert_ne!(
        doc.layers[0].image.as_raw(),
        destructive.as_raw(),
        "不允许再出现两次累积插值"
    );
    // 变换状态里的源图恒为最初那张。
    assert_eq!(
        doc.layers[0].transform.as_ref().unwrap().source.dimensions(),
        (80, 80)
    );
}

#[test]
fn near_original_size_snaps_back_but_real_resizes_do_not() {
    let src = checker(80, 80);
    let mut doc = doc_with_layer(src.clone());
    doc.scale_layer(0, Size::new(40, 40), (0, 0)).unwrap();
    // QML 手柄把显示像素换算回图像像素后的典型 ±1~2px 误差。
    doc.scale_layer(0, Size::new(79, 78), (0, 0)).unwrap();
    assert_eq!(doc.layers[0].image.dimensions(), (80, 80));
    assert_eq!(doc.layers[0].image.as_raw(), src.as_raw());

    // redo 复用同一个 op，吸附必须幂等。
    let committed = doc.layers[0].clone();
    doc.undo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.layers[0], committed, "吸附后的 undo/redo 必须一致");

    // 真的想缩到 77×77 时不能把它强行吸回 80×80。
    let mut doc2 = doc_with_layer(src.clone());
    doc2.scale_layer(0, Size::new(77, 77), (0, 0)).unwrap();
    assert_eq!(doc2.layers[0].image.dimensions(), (77, 77));
    assert_ne!(doc2.layers[0].image.as_raw(), src.as_raw());

    // 带镜像时不吸附（1:1 拷贝会丢掉翻转）。
    let mut doc3 = doc_with_layer(src.clone());
    doc3.scale_layer_flipped(0, Size::new(80, 80), (0, 0), (true, false))
        .unwrap();
    assert_ne!(doc3.layers[0].image.as_raw(), src.as_raw());
}

#[test]
fn scale_undo_redo_is_idempotent() {
    let src = checker(64, 48);
    let mut doc = doc_with_layer(src);
    doc.scale_layer(0, Size::new(32, 24), (5, 7)).unwrap();
    let committed = doc.layers[0].clone();
    doc.undo().unwrap();
    assert_eq!(doc.layers[0].image.dimensions(), (64, 48));
    assert_eq!(doc.layers[0].offset, (0, 0));
    assert!(doc.layers[0].transform.is_none());
    doc.redo().unwrap();
    assert_eq!(doc.layers[0], committed, "redo 必须与首次提交逐像素一致");
}

#[test]
fn pixel_patch_bakes_before_writing() {
    let mut doc = doc_with_layer(checker(64, 64));
    doc.scale_layer(0, Size::new(32, 32), (0, 0)).unwrap();
    assert!(doc.layers[0].transform.is_some());

    let before = RgbaImage::new(1, 1);
    let mut after = RgbaImage::new(1, 1);
    after.put_pixel(0, 0, image::Rgba([1, 2, 3, 255]));
    doc.execute(EditOp::PixelPatch {
        target: AdjustTarget::Layer(0),
        rect: Rect::new(0, 0, 1, 1),
        before,
        after,
    })
    .unwrap();

    assert!(
        doc.layers[0].transform.is_none(),
        "写像素前必须烘焙，否则下次缩放会抹掉编辑结果"
    );
    assert_eq!(doc.layers[0].image.get_pixel(0, 0).0, [1, 2, 3, 255]);

    // 烘焙后的层仍可正常缩放，源图换成「编辑过的当前位图」。
    doc.scale_layer(0, Size::new(64, 64), (0, 0)).unwrap();
    assert_eq!(doc.layers[0].image.dimensions(), (64, 64));
    assert_eq!(
        doc.layers[0].transform.as_ref().unwrap().source.dimensions(),
        (32, 32)
    );
}

#[test]
fn set_layer_transform_is_one_history_step_and_roundtrips() {
    let src = checker(40, 40);
    let mut doc = doc_with_layer(src.clone());
    let mut t = LayerTransform::from_offset(src, 10, 10);
    t.scale = (2.0, 2.0);
    t.center = (50.0, 50.0);

    doc.set_layer_transform(0, Some(t.clone())).unwrap();
    assert_eq!(doc.layers[0].image.dimensions(), (80, 80));
    assert_eq!(doc.layers[0].offset, (10, 10));
    assert_eq!(doc.layers[0].transform.as_ref().unwrap().scale, (2.0, 2.0));

    doc.undo().unwrap();
    assert_eq!(doc.layers[0].image.dimensions(), (40, 40));
    assert_eq!(doc.layers[0].offset, (0, 0));
    assert!(
        doc.layers[0]
            .transform
            .as_ref()
            .unwrap()
            .is_identity_fast_path(),
        "撤销后应回到 identity 状态"
    );

    doc.redo().unwrap();
    assert_eq!(doc.layers[0].image.dimensions(), (80, 80));
    assert_eq!(doc.layers[0].transform.as_ref().unwrap().scale, (2.0, 2.0));
}

#[test]
fn transform_does_not_change_export_size() {
    let src = checker(60, 50);
    let mut doc = doc_with_layer(src);
    // 把图层缩到画布外也不改变导出尺寸（P0-3 之前仍按画布裁剪）。
    doc.scale_layer(0, Size::new(120, 100), (-20, -10)).unwrap();
    assert_eq!(doc.flatten().dimensions(), (60, 50));
    doc.scale_layer(0, Size::new(20, 20), (5, 5)).unwrap();
    assert_eq!(doc.flatten().dimensions(), (60, 50));
}

// ---- P0-5 数值面板：外接框语义（D10 / R10）-------------------------------

#[test]
fn panel_values_are_axis_aligned_bounds_after_rotation() {
    let t = LayerTransform::from_offset(checker(40, 20), 10, 10);
    let p = t.panel_values();
    assert_eq!((p.x, p.y, p.width, p.height), (10.0, 10.0, 40.0, 20.0));
    let mut r = t.clone();
    r.rotation = 90.0;
    let p90 = r.panel_values();
    assert!((p90.width - 20.0).abs() < 1e-6, "90° 外接框宽应变成原高");
    assert!((p90.height - 40.0).abs() < 1e-6, "90° 外接框高应变成原宽");
    assert!((p90.rotation - 90.0).abs() < 1e-9);
}

#[test]
fn panel_width_keeps_rotation_and_pivot_at_0_45_90_degrees() {
    for angle in [0.0, 45.0, 90.0, 123.0] {
        let mut t = LayerTransform::from_offset(checker(40, 20), 100, 80);
        t.rotation = angle;
        let before = t.bounds().pivot(ReferencePoint::TopLeft);
        let ok = t.set_panel_value_with_solve(
            Field::Width,
            80.0,
            ReferencePoint::TopLeft,
            BoundsSolve::SingleAxis,
        );
        assert!(ok, "angle {angle} 应可解");
        assert!((t.rotation - angle).abs() < 1e-9, "W/H 写入不应改 rotation");
        assert!((t.bounds().width - 80.0).abs() < 1e-6, "angle {angle} 外接框宽");
        let after = t.bounds().pivot(ReferencePoint::TopLeft);
        assert!((after.0 - before.0).abs() < 1e-6 && (after.1 - before.1).abs() < 1e-6,
                "angle {angle} 参考点应保持不动");
    }
}

#[test]
fn panel_x_y_are_plain_translations_and_undo_is_one_step() {
    let mut t = LayerTransform::from_offset(checker(40, 20), 10, 10);
    t.set_panel_value(Field::X, 50.0, ReferencePoint::TopLeft);
    t.set_panel_value(Field::Y, 70.0, ReferencePoint::TopLeft);
    let p = t.panel_values();
    assert_eq!((p.x, p.y), (50.0, 70.0));
    assert_eq!(t.scale, (1.0, 1.0), "平移不应改 scale");

    let mut doc = doc_with_layer(checker(60, 50));
    let mut to = doc.layers[0].transform.clone().unwrap_or_else(|| {
        LayerTransform::from_offset(doc.layers[0].image.clone(), 0, 0)
    });
    to.set_panel_value(Field::X, 12.0, ReferencePoint::TopLeft);
    let before = doc.history_len();
    doc.set_layer_transform(0, Some(to)).unwrap();
    assert_eq!(doc.history_len(), before + 1, "一次写入 = 一条历史");
    doc.undo().unwrap();
    let q = doc.layers[0].transform_state().panel_values();
    assert_eq!(q.x, 0.0);
}
