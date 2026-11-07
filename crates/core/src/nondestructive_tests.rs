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

    // 第二次缩放直接从「未插值原图」取样，不是从第一次的 40x40 结果再放大。
    let from_original = image::imageops::resize(
        &src,
        80,
        80,
        image::imageops::FilterType::Lanczos3,
    );
    let destructive = image::imageops::resize(
        &image::imageops::resize(&src, 40, 40, image::imageops::FilterType::Lanczos3),
        80,
        80,
        image::imageops::FilterType::Lanczos3,
    );
    assert_eq!(
        doc.layers[0].image.as_raw(),
        from_original.as_raw(),
        "缩放→放大必须与从原图一次重采样一致"
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
