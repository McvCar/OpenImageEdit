use image::RgbaImage;

use editcore::{Adjust, AdjustTarget, Document, Layer, LayerMask, Rect, Size};

use super::*;
use crate::scale::downscale_box;

fn solid(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
    RgbaImage::from_pixel(width, height, image::Rgba(color))
}

#[test]
fn fit_never_upscales_small_images() {
    let size = display_size(Size::new(100, 100), Size::new(1920, 1080), Zoom::Fit);
    assert_eq!(size, Size::new(100, 100));
}

#[test]
fn fit_preserves_aspect_ratio() {
    let size = display_size(Size::new(4000, 2000), Size::new(1000, 1000), Zoom::Fit);
    assert_eq!(size, Size::new(1000, 500));
}

#[test]
fn crop_region_clamps_to_image() {
    let image = solid(100, 50, [255, 0, 0, 255]);
    let cropped = crop_region(&image, Rect::new(90, 40, 30, 30));
    assert_eq!(cropped.dimensions(), (10, 10));
}

#[test]
fn downscale_near_transparent_hole_stays_straight_white() {
    // [white, white, white, hole]: the last output pixel covers one white
    // and one transparent pixel. Straight-alpha averaging would darken it
    // to gray (the hole's RGB=0 bleeds in); premultiplied resampling must
    // keep it white at ~50% alpha — no dark rim around erased regions.
    let mut img = solid(4, 1, [255, 255, 255, 255]);
    img.put_pixel(3, 0, image::Rgba([0, 0, 0, 0]));
    let out = resize_to(&img, Size::new(2, 1));
    let edge = out.get_pixel(1, 0).0;
    assert!(
        (127..=128).contains(&edge[3]),
        "half coverage expected, got {:?}",
        edge
    );
    assert!(
        edge[0] >= 250 && edge[1] >= 250 && edge[2] >= 250,
        "edge must stay white (premultiplied resample), got {:?}",
        edge
    );
}

/// 剪切→粘贴回原位 (13% 缩放下「边缘一条浅色缝」的回归)。
///
/// 背景挖掉 (100,100)–(220,180)，再放一张**完全盖住这个洞**的图层。
/// 合成结果必须处处不透明：洞的分数 alpha 只要露出哪怕一行，透出来的
/// 就是棋盘格，宏观看上去就是粘贴内容「没贴严」。
///
/// 这条同时钉住 `preview_dst` 的取整策略：120×80 的图层在 0.13 倍下
/// 真实占位是 y 13.0..23.4，按「尺寸四舍五入」只盖 10 行（到 23 行
/// 就漏了 0.4 行 alpha），向外取整才盖满 11 行。
#[test]
fn pasted_layer_seals_erased_hole_at_fractional_zoom() {
    let mut doc = Document::from_image(solid(600, 400, [70, 120, 200, 255]));
    for y in 100..180 {
        for x in 100..220 {
            doc.base_image_mut().put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
        }
    }
    let mut pasted = Layer::new("pasted", solid(120, 80, [70, 120, 200, 255]));
    pasted.set_offset((100, 100));
    doc.add_layer(pasted).unwrap();

    let target = display_size(doc.canvas, Size::new(1100, 700), Zoom::Factor(0.13));
    assert_eq!(target, Size::new(78, 52));
    let out = PreviewComposer::new().composite(&doc, target);
    let semi: Vec<(u32, u32, u8)> = out
        .enumerate_pixels()
        .filter(|(_, _, p)| p.0[3] != 255)
        .map(|(x, y, p)| (x, y, p.0[3]))
        .collect();
    assert!(
        semi.is_empty(),
        "预览出现半透明像素（会透出棋盘格 = 那条「缝」）：{:?}",
        &semi[..semi.len().min(8)]
    );
}

#[test]
/// 降采样的输出像素必须与源图严格对齐：洞从 x=100 开始、缩到 0.37 倍
/// 时真实边界在 37.0，所以输出第 36 列必须**全不透明**。旧实现用
/// `imageops::thumbnail`，它在整数边界上多吃一个源像素，给出 alpha=170，
/// 于是洞的边缘向相邻像素渗出 2/3 的半透明 —— 透出棋盘格就是那条缝。
#[test]
fn downscale_keeps_edge_on_exact_pixel() {
    let make = |w: u32| {
        let mut img = solid(w, 4, [255, 255, 255, 255]);
        for y in 0..4 {
            for x in 100..w {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            }
        }
        img
    };
    // 600 -> 222（0.37 倍）：边界 37.0
    let out = resize_to(&make(600), Size::new(222, 2));
    assert_eq!(out.get_pixel(36, 1).0[3], 255, "洞左边一列必须全不透明");
    assert_eq!(out.get_pixel(37, 1).0[3], 0, "洞内部必须全透明");
    // 600 -> 450（0.75 倍）：边界 75.0
    let out = resize_to(&make(600), Size::new(450, 3));
    assert_eq!(out.get_pixel(74, 1).0[3], 255, "洞左边一列必须全不透明");
    assert_eq!(out.get_pixel(75, 1).0[3], 0, "洞内部必须全透明");
}

#[test]
fn fully_opaque_image_skips_premultiply_roundtrip() {
    // Opaque bitmaps must produce byte-identical results to the plain
    // area filter (no rounding drift from premultiply/unpremultiply).
    let img = solid(64, 48, [30, 144, 255, 255]);
    let a = resize_to(&img, Size::new(16, 12));
    let b = downscale_box(&img, 16, 12);
    assert_eq!(a.as_raw(), b.as_raw());
}

#[test]
fn document_render_matches_canvas_ratio() {
    let mut doc = Document::from_image(solid(2000, 1000, [10, 20, 30, 255]));
    doc.add_layer(Layer::new("overlay", solid(100, 100, [0, 255, 0, 255])))
        .unwrap();
    let rendered = render_for_viewport(&doc, Viewport::new(500, 500, Zoom::Fit));
    assert_eq!(rendered.dimensions(), (500, 250));
}

/// 剪贴蒙版必须在「预览合成」里生效。
///
/// 回归对象：`PreviewComposer` 曾经只做逐层 alpha-over，完全不看
/// `Layer::clipping`，于是被剪贴的纹理层（PSD 里那些 151×268 的斜条纹
/// 智能对象）在窗口里整片铺到底，PS 合并图里却只出现在圆角按钮内 ——
/// 屏幕上就是一条刺眼的斜条纹带，而 `dump-flat`（走 flatten）看不出来。
#[test]
fn preview_respects_clipping_mask() {
    let mut doc = Document::from_image(solid(40, 40, [255, 255, 255, 255]));
    let mut base = Layer::new("base", solid(10, 10, [255, 0, 0, 255]));
    base.set_offset((5, 5));
    doc.add_layer(base).unwrap();
    let mut clipped = Layer::new("clipped", solid(40, 40, [0, 0, 255, 255]));
    clipped.clipping = true;
    doc.add_layer(clipped).unwrap();

    // 0.5 倍预览：基线红块占 (2,2)–(7,7)。
    let out = PreviewComposer::new().composite(&doc, Size::new(20, 20));
    assert_eq!(
        out.get_pixel(0, 0).0,
        [255, 255, 255, 255],
        "剪贴层漏到基线框外"
    );
    assert_eq!(
        out.get_pixel(19, 19).0,
        [255, 255, 255, 255],
        "剪贴层漏到基线框外"
    );
    let inside = out.get_pixel(5, 5).0;
    assert!(
        inside[2] > 200 && inside[0] < 60,
        "框内应是剪贴层的蓝色，得到 {:?}",
        inside
    );

    // 1:1 时预览必须与 flatten（保存/保真口径）逐像素一致。
    let preview = PreviewComposer::new().composite(&doc, doc.canvas);
    let flat = doc.flatten_in_doc_space();
    assert_eq!(preview.as_raw(), flat.as_raw(), "预览与 flatten 的剪贴结果不一致");
}

/// 图层蒙版必须在「预览合成」里生效，且 1:1 时与 flatten 逐字节一致。
///
/// 回归对象：`PreviewComposer` 曾经完全不看 `Layer::mask`，带蒙版的层在
/// 预览里是「蒙版透明处仍然整块可见」；而保存 / 保真走
/// `flatten_in_doc_space`，于是窗口与导出图对不上。
///
/// 蒙版取 0 / 64 / 128 / 192 / 255 五档分数值：只要预览的覆盖率换算写错
/// 一个字节（例如把 `coverage()` 的 f32 直接截断成 0/1），逐字节断言就会
/// 失败。
#[test]
fn preview_matches_flatten_with_layer_mask_at_actual_size() {
    use image::{GrayImage, Luma};
    let mut doc = Document::from_image(solid(16, 8, [250, 250, 250, 255]));
    let mut layer = Layer::new("masked", solid(16, 8, [200, 30, 40, 255]));
    let mut mask = GrayImage::new(16, 8);
    for y in 0..8u32 {
        for x in 0..16u32 {
            mask.put_pixel(
                x,
                y,
                Luma([match x % 5 {
                    0 => 0,
                    1 => 64,
                    2 => 128,
                    3 => 192,
                    _ => 255,
                }]),
            );
        }
    }
    layer.mask = Some(LayerMask::new(mask, (0, 0)));
    doc.add_layer(layer).unwrap();

    let preview = PreviewComposer::new().composite(&doc, doc.canvas);
    let flat = doc.flatten_in_doc_space();
    assert_eq!(
        preview.as_raw(),
        flat.as_raw(),
        "1:1 预览与 flatten_in_doc_space 不一致（蒙版）"
    );

    // 钉住「蒙版确实乘进了 alpha」：0 处完全露出底图，255 处完全是本层。
    assert_eq!(preview.get_pixel(0, 0).0, [250, 250, 250, 255]);
    assert_eq!(preview.get_pixel(4, 0).0, [200, 30, 40, 255]);
    // 128 处应是严格按 alpha=128/255 的正片叠底结果。
    let half = preview.get_pixel(2, 0).0;
    assert_eq!(half, [225, 140, 145, 255], "蒙版半遮的混合结果不对");
}

/// 蒙版覆盖率按**预览像素覆盖的画布区间**取平均，而不是点采样。
///
/// 3×1 的层在 2×1 预览里，第一个预览像素覆盖画布 x 0..1.5：蒙版是
/// [0, 255, 255]，正确结果是一半覆盖（128/255），不是 0 或 255。
#[test]
fn preview_mask_coverage_follows_preview_scale() {
    use image::{GrayImage, Luma};
    let mut doc = Document::from_image(solid(3, 1, [255, 255, 255, 255]));
    let mut layer = Layer::new("masked", solid(3, 1, [0, 0, 0, 255]));
    let mut mask = GrayImage::new(3, 1);
    mask.put_pixel(0, 0, Luma([0]));
    mask.put_pixel(1, 0, Luma([255]));
    mask.put_pixel(2, 0, Luma([255]));
    layer.mask = Some(LayerMask::new(mask, (0, 0)));
    doc.add_layer(layer).unwrap();

    let out = PreviewComposer::new().composite(&doc, Size::new(2, 1));
    assert_eq!(out.get_pixel(1, 0).0, [0, 0, 0, 255], "右格应被完全遮住");
    let edge = out.get_pixel(0, 0).0;
    assert_eq!(edge[3], 255);
    assert_eq!(edge[0], 127, "半覆盖的预览像素应混成中灰，得到 {:?}", edge);
}

/// 蒙版与剪贴同时存在时：先乘蒙版、再乘剪贴覆盖率。
///
/// 基底红块自己带蒙版（右半遮住），上面压一层剪贴蓝层。正确结果：
/// - 基底可见处 → 蓝（剪贴层）；
/// - 基底被蒙版遮住处 → 白底（剪贴层跟着消失，而不是漏出蓝）；
/// - 基底矩形外 → 白底。
/// 若顺序反过来（先剪贴再蒙版），右侧会错误地留下蓝色。
#[test]
fn preview_applies_mask_before_clip() {
    use image::{GrayImage, Luma};
    let mut doc = Document::from_image(solid(8, 8, [255, 255, 255, 255]));
    let mut base = Layer::new("base", solid(6, 6, [255, 0, 0, 255]));
    base.set_offset((1, 1));
    let mut mask = GrayImage::new(6, 6);
    for y in 0..6u32 {
        for x in 0..6u32 {
            mask.put_pixel(x, y, Luma([if x < 3 { 255 } else { 0 }]));
        }
    }
    base.mask = Some(LayerMask::new(mask, (1, 1)));
    doc.add_layer(base).unwrap();
    let mut clipped = Layer::new("clipped", solid(8, 8, [0, 0, 255, 255]));
    clipped.clipping = true;
    doc.add_layer(clipped).unwrap();

    let out = PreviewComposer::new().composite(&doc, doc.canvas);
    assert_eq!(
        out.get_pixel(2, 2).0,
        [0, 0, 255, 255],
        "基底可见处应显示剪贴层"
    );
    assert_eq!(
        out.get_pixel(5, 2).0,
        [255, 255, 255, 255],
        "基底被蒙版遮住，剪贴层必须一起消失"
    );
    assert_eq!(
        out.get_pixel(0, 0).0,
        [255, 255, 255, 255],
        "基底矩形外应露出白底"
    );
    assert_eq!(
        out.as_raw(),
        doc.flatten_in_doc_space().as_raw(),
        "1:1 预览与 flatten 的蒙版 + 剪贴叠加结果不一致"
    );
}

/// 剪贴基底带烘焙样式时，预览的边界必须取基底的「内容」形状。
///
/// 回归对象：`矩形 5` 有 8px Outside 描边，烘焙后位图 alpha 比填充大一圈；
/// 剪贴的斜纹层按位图 alpha 限幅，于是花纹越过圆角、压到粉色描边上
/// （PS 里描边是硬边界）。这里钉住两件事：1:1 预览与 flatten 逐字节一致，
/// 且放大的描边带里一点剪贴内容都不能有。
#[test]
fn preview_clip_base_uses_content_shape_not_baked_effects() {
    use image::{GrayImage, Luma};
    let mut doc = Document::from_image(solid(20, 20, [255, 255, 255, 255]));
    // 基底：12×12 全不透明（= 内容 + 烘焙描边），内容形状只有中间 6×6。
    let mut base = Layer::new("base", solid(12, 12, [255, 0, 0, 255]));
    base.set_offset((4, 4));
    let mut shape = GrayImage::new(12, 12);
    for y in 3..9u32 {
        for x in 3..9u32 {
            shape.put_pixel(x, y, Luma([255]));
        }
    }
    base.clip_shape = Some(LayerMask::new(shape, (4, 4)));
    doc.add_layer(base).unwrap();
    let mut clipped = Layer::new("clipped", solid(20, 20, [0, 0, 255, 255]));
    clipped.clipping = true;
    doc.add_layer(clipped).unwrap();

    // 描边带（内容形状之外、烘焙位图之内）必须是基底的红色，不能是蓝色。
    let preview = PreviewComposer::new().composite(&doc, doc.canvas);
    assert_eq!(
        preview.get_pixel(4, 4).0,
        [255, 0, 0, 255],
        "描边带里漏进了剪贴内容"
    );
    assert_eq!(preview.get_pixel(7, 7).0, [0, 0, 255, 255], "内容形状内应是剪贴层");
    assert_eq!(
        preview.as_raw(),
        doc.flatten_in_doc_space().as_raw(),
        "1:1 预览与 flatten 的剪贴基线不一致（内容形状）"
    );

    // 缩略预览同样只能落在内容形状里。
    let small = PreviewComposer::new().composite(&doc, Size::new(10, 10));
    assert_eq!(small.get_pixel(2, 2).0, [255, 0, 0, 255], "缩略预览里剪贴内容溢到描边");
}

/// 投影画在内容之外的部分要按样式自己的混合模式（PSD `Md  `）与下层混合。
///
/// 回归对象：标题栏那条奶油色投影 `(255,231,200)`，PSD 里是 `Multiply`，
/// 压在黄色面板上应得 `(255,208,~100)`；把它当 Normal 盖上去会变成一条
/// 不透明奶油带。这里钉住预览与 flatten 两条路径都对。
#[test]
fn preview_blends_outside_style_pixels_with_the_effect_mode() {
    let mut doc = Document::from_image(solid(8, 8, [255, 230, 130, 255]));
    let mut bar = Layer::new("bar", solid(8, 8, [255, 231, 200, 255]));
    bar.style = Some(editcore::StylePixels {
        blend: editcore::BlendMode::Multiply,
        mask: vec![255; 64],
    });
    doc.add_layer(bar).unwrap();

    let preview = PreviewComposer::new().composite(&doc, doc.canvas);
    let px = preview.get_pixel(3, 3).0;
    assert!(
        (px[1] as i32 - 208).abs() <= 2 && (px[2] as i32 - 100).abs() <= 3,
        "样式像素没有按 Multiply 混合下层: {px:?}"
    );
    assert_eq!(
        preview.as_raw(),
        doc.flatten_in_doc_space().as_raw(),
        "1:1 预览与 flatten 的样式混合结果不一致"
    );

    // 掩码为 0 的像素仍走图层混合模式（Normal）：保持投影色。
    let mut plain = Layer::new("plain", solid(8, 8, [255, 231, 200, 255]));
    plain.style = None;
    let mut doc2 = Document::from_image(solid(8, 8, [255, 230, 130, 255]));
    doc2.add_layer(plain).unwrap();
    let out2 = PreviewComposer::new().composite(&doc2, doc2.canvas);
    assert_eq!(out2.get_pixel(3, 3).0, [255, 231, 200, 255]);
}

#[test]
fn preview_matches_flatten_for_dissolve() {
    let mut doc = Document::from_image(solid(16, 16, [0, 0, 0, 255]));
    let mut layer = Layer::new("diss", solid(16, 16, [255, 0, 0, 128]));
    layer.blend = editcore::BlendMode::Dissolve;
    doc.add_layer(layer).unwrap();

    let flat = doc.flatten_in_doc_space();
    let preview = PreviewComposer::new().composite(&doc, doc.canvas);
    assert_eq!(
        preview.as_raw(),
        flat.as_raw(),
        "1:1 预览的溶解抖动图案与 flatten 不一致"
    );

    let kept = flat.pixels().filter(|p| p.0[0] > 200 && p.0[3] == 255).count();
    let partial = flat.pixels().filter(|p| p.0[3] == 128).count();
    assert_eq!(partial, 0, "溶解不应留下半透明像素");
    assert!((90..=166).contains(&kept), "保留像素数异常: {kept}");
}

#[test]
fn preview_matches_base_when_no_layers_are_visible() {
    let mut doc = Document::from_image(solid(100, 50, [10, 20, 30, 255]));
    let mut hidden = Layer::new("hidden", solid(100, 50, [200, 0, 0, 255]));
    hidden.visible = false;
    doc.add_layer(hidden).unwrap();
    let preview = composite_for_preview(&doc, Size::new(40, 20));
    assert_eq!(preview.dimensions(), (40, 20));
    assert_eq!(preview.get_pixel(0, 0).0, [10, 20, 30, 255]);
}

#[test]
fn preview_fast_path_honors_the_crop_view() {
    let mut doc = Document::from_image(solid(100, 50, [10, 20, 30, 255]));
    let mut hidden = Layer::new("hidden", solid(100, 50, [200, 0, 0, 255]));
    hidden.visible = false;
    doc.add_layer(hidden).unwrap();

    doc.crop(Rect::new(40, 10, 30, 20)).unwrap();
    let preview = composite_for_preview(&doc, Size::new(30, 20));
    assert_eq!(preview.dimensions(), (30, 20));

    // 还原裁剪后回到全尺寸底图。
    doc.reset_crop().unwrap();
    let restored = composite_for_preview(&doc, Size::new(100, 50));
    assert_eq!(restored.dimensions(), (100, 50));
    assert_eq!(restored.get_pixel(0, 0).0, [10, 20, 30, 255]);
}

#[test]
fn preview_includes_visible_layers() {
    let mut doc = Document::from_image(solid(40, 40, [10, 20, 30, 255]));
    let mut layer = Layer::new("dot", solid(10, 10, [255, 255, 255, 255]));
    layer.set_offset((0, 0));
    doc.add_layer(layer).unwrap();
    let preview = composite_for_preview(&doc, Size::new(40, 40));
    assert_eq!(preview.get_pixel(2, 2).0, [255, 255, 255, 255]);
    assert_eq!(preview.get_pixel(20, 20).0, [10, 20, 30, 255]);
}

#[test]
fn composer_cache_follows_live_drag_and_epoch_invalidation() {
    let mut doc = Document::from_image(solid(40, 40, [10, 20, 30, 255]));
    let mut layer = Layer::new("dot", solid(10, 10, [255, 0, 0, 255]));
    layer.set_offset((0, 0));
    doc.add_layer(layer).unwrap();
    let target = Size::new(40, 40);

    let mut composer = PreviewComposer::new();
    let first = composer.composite(&doc, target);
    assert_eq!(first.get_pixel(2, 2).0, [255, 0, 0, 255]);

    // Live drag tick: offset changes WITHOUT an epoch bump (history-free
    // mutation) — the cached scaled bitmap must still land at the new
    // offset.
    doc.layers[1].set_offset((20, 20));
    let second = composer.composite(&doc, target);
    assert_eq!(second.get_pixel(22, 22).0, [255, 0, 0, 255]);
    assert_eq!(second.get_pixel(2, 2).0, [10, 20, 30, 255]);

    // Committed edit bumps the epoch: cache is rebuilt and reflects the
    // new content.
    doc.set_layer_opacity(1, 0.5).unwrap();
    let third = composer.composite(&doc, target);
    let px = third.get_pixel(22, 22).0;
    // 0.5 alpha red over the base color: 255*0.5+10*0.5=132.5→133,
    // 20*0.5=10, 30*0.5=15.
    assert_eq!(px[0], 133);
    assert_eq!(px[1], 10);
    assert_eq!(px[2], 15);
}

#[test]
fn live_scale_preview_stretches_snapshot_without_touching_doc() {        let mut doc = Document::from_image(solid(40, 40, [10, 20, 30, 255]));
    let mut layer = Layer::new("dot", solid(10, 10, [255, 0, 0, 255]));
    layer.set_offset((10, 10));
    doc.add_layer(layer).unwrap();
    let target = Size::new(40, 40);
    let snapshot = doc.layers[1].img().clone();

    let mut composer = PreviewComposer::new();
    // Warm the caches with a normal composite, then request a live
    // resize of the layer to 20x20 at offset (0, 0).
    let before = composer.composite(&doc, target);
    let epoch_before = doc.edit_epoch;
    let live = composer.composite_live_scale(
        &doc,
        target,
        1,
        &snapshot,
        Size::new(20, 20),
        (0, 0),
    );

    // The preview shows the snapshot stretched into the requested rect.
    assert_eq!(live.get_pixel(2, 2).0, [255, 0, 0, 255]);
    assert_eq!(live.get_pixel(18, 18).0, [255, 0, 0, 255]);
    // Outside the live rect: the untouched base.
    assert_eq!(live.get_pixel(30, 30).0, [10, 20, 30, 255]);

    // The document itself is never modified during the gesture.
    assert_eq!(doc.layers[1].img().dimensions(), (10, 10));
    assert_eq!(doc.layers[1].offset(), (10, 10));
    assert_eq!(doc.edit_epoch, epoch_before);

    // Cancelling (back to a plain composite) restores the committed
    // layout — the live slot must not poison the layer cache.
    let restored = composer.composite(&doc, target);
    assert_eq!(restored.get_pixel(15, 15).0, [255, 0, 0, 255]);
    assert_eq!(restored.get_pixel(2, 2).0, [10, 20, 30, 255]);
    // `before` and `restored` agree pixel for pixel.
    assert_eq!(before.as_raw(), restored.as_raw());
}

#[test]
fn adjust_preview_is_identity_neutral_and_never_touches_the_doc() {
    let doc = Document::from_image(solid(40, 30, [90, 120, 200, 255]));
    let target = Size::new(20, 15);
    let mut composer = PreviewComposer::new();
    let plain = composer.composite(&doc, target);
    let neutral = composer.composite_with_adjust(
        &doc,
        target,
        AdjustTarget::Base,
        &Adjust::identity(),
    );
    assert_eq!(plain.as_raw(), neutral.as_raw());
    assert_eq!(doc.base_image().get_pixel(5, 5).0, [90, 120, 200, 255]);
}

#[test]
fn adjust_preview_scopes_to_base_or_layer() {
    let mut doc = Document::from_image(solid(40, 30, [100, 100, 100, 255]));
    let mut layer = Layer::new("dot", solid(10, 10, [150, 150, 150, 255]));
    layer.set_offset((0, 0));
    doc.add_layer(layer).unwrap();
    let target = Size::new(40, 30);
    let mut composer = PreviewComposer::new();

    let brighter = Adjust {
        exposure: 0.8,
        ..Adjust::identity()
    };
    // Base scope: the far corner brightens, the layer stays as it was.
    let base_adjusted =
        composer.composite_with_adjust(&doc, target, AdjustTarget::Base, &brighter);
    assert!(base_adjusted.get_pixel(30, 20).0[0] > 100);
    assert_eq!(base_adjusted.get_pixel(2, 2).0, [150, 150, 150, 255]);

    // Layer scope: only the layer's rect changes. `layers[0]` is the
    // background, so the user layer lives at index 1.
    let layer_adjusted =
        composer.composite_with_adjust(&doc, target, AdjustTarget::Layer(1), &brighter);
    assert!(layer_adjusted.get_pixel(2, 2).0[0] > 150);
    assert_eq!(layer_adjusted.get_pixel(30, 20).0, [100, 100, 100, 255]);
}

#[test]
fn adjust_preview_leaves_markup_annotations_untouched() {
    let mut doc = Document::from_image(solid(40, 30, [100, 100, 100, 255]));
    let mut mark = editcore::Mark::new(editcore::MarkKind::Rect);
    mark.rect = (2.0, 2.0, 20.0, 20.0);
    mark.stroke = 6;
    mark.color = [255, 0, 0, 255];
    doc.add_mark(mark).unwrap();
    let target = Size::new(40, 30);
    let mut composer = PreviewComposer::new();
    let out = composer.composite_with_adjust(
        &doc,
        target,
        AdjustTarget::Base,
        &Adjust {
            saturation: -1.0,
            ..Adjust::identity()
        },
    );
    // The red stroke stays red: marks are annotations, not photo pixels.
    assert_eq!(out.get_pixel(2, 10).0, [255, 0, 0, 255]);
    // The grey base is desaturated but unchanged (already neutral grey).
    assert_eq!(out.get_pixel(30, 25).0, [100, 100, 100, 255]);
}
