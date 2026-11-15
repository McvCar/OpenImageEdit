use image::{GrayImage, RgbaImage};

use super::ContentAlpha;

/// 「背后样式画在内容之外」的像素 + 样式自己的混合模式，交给合成阶段。
///
/// `coverage` 只含投影 / 外发光（`Baked::behind_coverage`）。再按内容 alpha
/// 过滤一次：内容之内的投影是被内容盖住的（`knocks_out` 也已经把它清掉），
/// 那里的背景是图层内容，烘焙时混合已经对了。
fn style_plane(
    content: &ContentAlpha,
    offset: (i32, i32),
    size: (u32, u32),
    coverage: &[u8],
    blend: editcore::BlendMode,
) -> Option<editcore::StylePixels> {
    let (w, h) = size;
    if coverage.len() != (w as usize) * (h as usize) {
        return None;
    }
    let mut mask = coverage.to_vec();
    let mut any = false;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = (y as u32 * w + x as u32) as usize;
            if mask[i] == 0 {
                continue;
            }
            if content.at(offset.0 + x, offset.1 + y) > 0.0 {
                mask[i] = 0;
            } else {
                any = true;
            }
        }
    }
    if !any {
        return None;
    }
    Some(editcore::StylePixels { blend, mask })
}

/// 单层图层样式（`lfx2`）烘焙的结果：位图、偏移、降级说明，以及合成阶段
/// 需要的「内容」平面与「背后」笔迹。
pub(super) struct LayerStyleBake {
    pub image: RgbaImage,
    pub offset: (i32, i32),
    pub notes: Vec<String>,
    pub content_alpha: Option<ContentAlpha>,
    pub clip_shape: Option<editcore::LayerMask>,
    pub style: Option<editcore::StylePixels>,
    pub baked: bool,
}

/// 把一层上的图层样式（阴影 / 描边 / 叠加）画进它的像素。
///
/// Photoshop stores shadow / stroke / overlay as parameters; without
/// baking them the layer looks flat (a title loses its gradient fill,
/// a button loses its outline). `bake` renders them into the bitmap and
/// reports any style it could not draw.
pub(super) fn bake_layer_styles(
    layer: &psd::PsdLayer,
    image: RgbaImage,
    offset: (i32, i32),
    canvas: (i32, i32),
    mask: Option<&editcore::LayerMask>,
    patterns: &crate::psd_pattern::PatternStore,
    no_styles: bool,
) -> LayerStyleBake {
    let (cw, ch) = canvas;
    let mut image = image;
    let mut layer_offset = offset;
    let mut notes: Vec<String> = Vec::new();
    // 烘焙前留一份 alpha：剪贴调整层按「内容」限幅，见字段注释。
    let pre_alpha = if layer.layer_effects().is_some() {
        Some(ContentAlpha {
            alpha: image.pixels().map(|p| p.0[3]).collect::<Vec<u8>>(),
            offset: layer_offset,
            size: (image.width(), image.height()),
        })
    } else {
        None
    };
    let mut content_alpha = None;
    let mut clip_shape = None;
    let mut style = None;
    let mut baked = false;
    if let Some(effects) =
        layer
            .layer_effects()
            .filter(|_| !no_styles && (layer.transparency_shapes() || mask.is_some()))
    {
        // 「透明形状图层」关掉（`tsly = 0`）后，PS 不再按图层透明度判断样式
        // 的内外区域，而是按图层蒙版；没有蒙版时样式完全不显示（上面的
        // filter 已跳过）。这里把蒙版覆盖率做成样式作用 alpha，坐标从图层
        // 矩形映射回画布。
        let mut tsly_alpha = None;
        if !layer.transparency_shapes() {
            if let Some(m) = mask {
                let (iw, ih) = (image.width(), image.height());
                let mut plane = vec![0.0f32; iw as usize * ih as usize];
                for y in 0..ih {
                    for x in 0..iw {
                        plane[(y * iw + x) as usize] =
                            m.coverage(offset.0 + x as i32, offset.1 + y as i32);
                    }
                }
                tsly_alpha = Some(plane);
            }
        }
        // 诊断：`IMAGEEDIT_PSD_DUMP_FX=<图层名子串>` 打印解析到的样式参数。
        // 保真差异按块归因到某个图层后，用它确认「是解析错了还是画错了」。
        if let Ok(want) = std::env::var("IMAGEEDIT_PSD_DUMP_FX") {
            if !want.is_empty() && layer.name().contains(&want) {
                for e in effects.iter().filter(|e| e.enabled) {
                    println!(
                        "{{\"fx\":\"{}\",\"kind\":\"{:?}\",\"blend\":\"{:?}\",\"op\":{:.3},\
                         \"ang\":{:.1},\"dist\":{:.2},\"size\":{:.2},\"choke\":{:.1},\
                         \"color\":[{},{},{}],\"pos\":\"{:?}\",\"fill\":\"{:?}\",\
                         \"grad\":{},\"knock\":{}}}",
                        layer.name(),
                        e.kind,
                        e.blend,
                        e.opacity,
                        e.angle,
                        e.distance,
                        e.size,
                        e.choke,
                        e.color[0],
                        e.color[1],
                        e.color[2],
                        e.stroke_position,
                        e.stroke_fill,
                        e.gradient.is_some(),
                        e.knocks_out
                    );
                }
            }
        }
        crate::psd_effects::set_trace_label(layer.name());
        // 只描边的形状（`fillEnabled = false`）位图是空的，但 PS 仍然按
        // **矢量路径**给它描边、加内阴影 —— 样式的作用 alpha 要取路径
        // 覆盖率，不能取空像素，否则整圈的样式都会消失。
        //
        // 但**带描边**的形状不能这么做：它真正可见的内容是那圈描边
        // （`stroke_shape` 已经画进位图），路径覆盖率会把整个内部都算成
        // 内容 —— 实测 `矩形 13`（全画布细边框）的内阴影/颜色叠加因此
        // 铺满整个内区，MAE 从 3.98 涨到 5.44。
        let has_visible_stroke = layer
            .stroke_style()
            .map(|s| s.enabled && s.width > 0.0)
            .unwrap_or(false);
        let path_alpha = if layer.vector_fill().is_none() && !has_visible_stroke {
            layer.vector_mask().and_then(|vm| {
                crate::psd_vector::path_alpha(
                    &image,
                    layer_offset,
                    (cw as u32, ch as u32),
                    vm,
                )
            })
        } else {
            None
        };
        if let Some(baked_effects) = crate::psd_effects::bake_with_alpha(
            &image,
            layer_offset,
            effects,
            if layer.transparency_shapes() {
                path_alpha.as_deref()
            } else {
                tsly_alpha.as_deref()
            },
            Some(patterns),
        ) {
            if let Some(pre) = pre_alpha.as_ref() {
                // 剪贴基底用的边界：烘焙前的原始内容 alpha（样式一点都
                // 不算），不是 `content_alpha` 那份「扣掉样式笔迹」的平面。
                clip_shape = GrayImage::from_raw(pre.size.0, pre.size.1, pre.alpha.clone())
                    .map(|img| editcore::LayerMask::new(img, pre.offset));
                content_alpha = Some(ContentAlpha::from_baked(
                    pre,
                    baked_effects.offset,
                    baked_effects.image.dimensions(),
                    &baked_effects.effect_coverage,
                ));
                // 内容之外的样式笔迹（投影 / 外发光）：烘焙把它们按 Normal
                // 画进了图层位图，背景是透明的，`Multiply` 退化成 Normal。
                // 记下这些像素与样式自己的混合模式，合成时改用它。
                if let Some(mode) = crate::psd_effects::behind_blend(effects) {
                    style = style_plane(
                        pre,
                        baked_effects.offset,
                        baked_effects.image.dimensions(),
                        &baked_effects.behind_coverage,
                        mode,
                    );
                    if let (Some(s), Ok(want)) =
                        (style.as_ref(), std::env::var("IMAGEEDIT_PSD_DUMP_STYLE"))
                    {
                        if want == "*" || layer.name().contains(&want) {
                            let (w, h) = baked_effects.image.dimensions();
                            let n = s.mask.iter().filter(|v| **v != 0).count();
                            println!(
                                "{{\"style\":\"{}\",\"blend\":\"{:?}\",\"px\":{},\
                                 \"rect\":[{},{},{},{}]}}",
                                layer.name(),
                                s.blend,
                                n,
                                baked_effects.offset.0,
                                baked_effects.offset.1,
                                w,
                                h
                            );
                        }
                    }
                }
            }
            image = baked_effects.image;
            layer_offset = baked_effects.offset;
            notes.extend(baked_effects.notes);
            baked = true;
        }
    }
    LayerStyleBake {
        image,
        offset: layer_offset,
        notes,
        content_alpha,
        clip_shape,
        style,
        baked,
    }
}
