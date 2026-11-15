use std::sync::Arc;

use image::imageops::FilterType;
use image::RgbaImage;

use editcore::{Adjust, AdjustTarget, Document, Rect, Size};

impl super::PreviewComposer {
    /// Preview composite while a layer is being live-resized.
    ///
    /// The dragged layer is drawn from `state.snapshot` stretched straight
    /// to `state.to`/`state.offset` (converted to preview pixels), so the
    /// document bitmap itself is NEVER resampled during the gesture — the
    /// epoch stays put and base plus all other layers keep their cached
    /// scaled bitmaps. Each tick only pays one Nearest stretch of the
    /// snapshot into the layer's preview rect + the blend; the quality
    /// Triangle resample happens exactly once, in
    /// `Document::scale_layer` when the gesture commits.
    pub fn composite_live_scale(
        &mut self,
        doc: &Document,
        target: Size,
        layer_index: usize,
        snapshot: &RgbaImage,
        to: Size,
        offset: (i32, i32),
    ) -> RgbaImage {
        self.composite_live_scale_adjusted(
            doc,
            target,
            layer_index,
            snapshot,
            to,
            offset,
            None,
            (false, false),
        )
    }

    /// [`PreviewComposer::composite_live_scale`] plus an optional live color
    /// adjustment (the "调整颜色" panel can stay open while a layer handle is
    /// dragged) and mirroring for handles dragged past the opposite edge.
    #[allow(clippy::too_many_arguments)]
    pub fn composite_live_scale_adjusted(
        &mut self,
        doc: &Document,
        target: Size,
        layer_index: usize,
        snapshot: &RgbaImage,
        to: Size,
        offset: (i32, i32),
        adjust: Option<(AdjustTarget, Adjust)>,
        flip: (bool, bool),
    ) -> RgbaImage {
        let Some((sx, sy)) = self.prepare(doc, target) else {
            return RgbaImage::new(target.width.max(1), target.height.max(1));
        };
        let drag_opacity = doc
            .layers
            .get(layer_index)
            .map(|l| if l.visible { l.opacity } else { 0.0 })
            .unwrap_or(0.0);
        // The dragged layer keeps its own blend mode, otherwise a `Multiply`
        // layer would flash to Normal for the duration of the gesture.
        let drag_blend = doc
            .layers
            .get(layer_index)
            .map(|l| l.blend)
            .unwrap_or_default();
        let mut out = self.base_buffer(doc, target, sx, sy);
        if let Some((AdjustTarget::Base, params)) = adjust {
            params.apply_in_place(&mut out);
        }
        self.blend_upper_layers(doc, &mut out, target, sx, sy, Some(layer_index), adjust);
        if drag_opacity > 0.0 {
            let nw = ((to.width as f64 * sx).round() as u32).max(1);
            let nh = ((to.height as f64 * sy).round() as u32).max(1);
            // 缓存键只有 (w, h)：翻转标记变了就必须重算，否则镜像前后会
            // 复用同一张未翻转的位图。
            let hit = if self.live_flip == flip {
                Self::cached_hit(&self.live, nw, nh)
            } else {
                None
            };
            let src = match hit {
                Some(img) => img,
                None => {
                    // P1-A 质量分层（D14/R11）：拖动帧用双线性（Triangle），
                    // 不再 Nearest 马赛克；落定仍走 HQ（Lanczos3/sample HQ），
                    // 「不许把落定画质降到拖动水平」的反向约束照旧。
                    let mut img = image::imageops::resize(
                        snapshot,
                        nw,
                        nh,
                        FilterType::Triangle,
                    );
                    if flip.0 {
                        img = image::imageops::flip_horizontal(&img);
                    }
                    if flip.1 {
                        img = image::imageops::flip_vertical(&img);
                    }
                    let img = Arc::new(img);
                    self.live = Some((nw, nh, img.clone()));
                    self.live_flip = flip;
                    img
                }
            };
            let ox = (offset.0 as f64 * sx).round() as i32;
            let oy = (offset.1 as f64 * sy).round() as i32;
            let stretched = match adjust {
                Some((AdjustTarget::Layer(index), params)) if index == layer_index => {
                    let mut adjusted = (*src).clone();
                    params.apply_in_place(&mut adjusted);
                    adjusted
                }
                _ => (*src).clone(),
            };
            Self::blend_layer(&mut out, target, &stretched, ox, oy, drag_opacity, drag_blend, None, None, None);
        }
        doc.apply_display_color(&mut out);
        out
    }

    /// 拖动旋转时的实时合成（文档完全不动，和 `live_scale` 一个套路）。
    ///
    /// 顺序是「先把图层缩到预览尺寸，再在小图上转」：单帧代价只跟预览像素
    /// 数有关，原图多大都一样。反过来（先转整层再缩放）在 2200×1500 的图上
    /// 每帧要 40ms 以上 —— 那正是旋转手柄一顿一顿的原因。
    ///
    /// P1-A：斜切拖动帧的实时合成（Ctrl+拖边）。文档像素不动：活动层按
    /// 「起手快照 → 预览尺寸 → LayerTransform（与落定同一个 sample_affine
    /// 采样器，hq=false 双线性）」画出，其余图层照旧。`skew` 是本次手势
    /// 的累计剪切量，`center_delta` 是 W/N 缘的对侧锚定补偿（图像坐标）。
    pub fn composite_live_skew_adjusted(
        &mut self,
        doc: &Document,
        target: Size,
        layer_index: usize,
        snapshot: &RgbaImage,
        skew: (f64, f64),
        center_delta: (f64, f64),
        adjust: Option<(AdjustTarget, Adjust)>,
    ) -> RgbaImage {
        let Some((sx, sy)) = self.prepare(doc, target) else {
            return RgbaImage::new(target.width.max(1), target.height.max(1));
        };
        let drag_opacity = doc
            .layers
            .get(layer_index)
            .map(|l| if l.visible { l.opacity } else { 0.0 })
            .unwrap_or(0.0);
        // The dragged layer keeps its own blend mode, otherwise a `Multiply`
        // layer would flash to Normal for the duration of the gesture.
        let drag_blend = doc
            .layers
            .get(layer_index)
            .map(|l| l.blend)
            .unwrap_or_default();
        let mut out = self.base_buffer(doc, target, sx, sy);
        if let Some((AdjustTarget::Base, params)) = adjust {
            params.apply_in_place(&mut out);
        }
        self.blend_upper_layers(doc, &mut out, target, sx, sy, Some(layer_index), adjust);
        if drag_opacity > 0.0 {
            let Some(layer) = doc.layers.get(layer_index) else {
                return out;
            };
            // 起手参数（手势不写文档，layer.transform 就是 begin 状态）。
            let Some(lr) = layer.raster() else {
                return out;
            };
            let base = lr.transform.clone().unwrap_or_else(|| {
                editcore::LayerTransform::from_offset(lr.image.clone(), lr.offset.0, lr.offset.1)
            });
            let (sw, sh) = base.source_size();
            let nw = ((sw * base.scale.0.abs() * sx).round() as u32).max(1);
            let nh = ((sh * base.scale.1.abs() * sy).round() as u32).max(1);
            // 复用 live 缓存（键 (w,h)，flip 恒 (false,false)：斜切不镜像，
            // 若上一段缩放手势留了 flip=true 的缓存，键不匹配会自动重算）。
            let scaled = if self.live_flip == (false, false) {
                Self::cached_hit(&self.live, nw, nh)
            } else {
                None
            };
            let scaled = match scaled {
                Some(img) => img,
                None => {
                    let img = Arc::new(image::imageops::resize(
                        snapshot,
                        nw,
                        nh,
                        FilterType::Triangle,
                    ));
                    self.live = Some((nw, nh, img.clone()));
                    self.live_flip = (false, false);
                    img
                }
            };
            // 预览空间的变换：source 已是「预览尺寸的显示位图」，scale 归 1，
            // center / 补偿量按 (sx, sy) 折算，旋转 / 斜切角度原样保留。
            //（source 字段直接换绑 Arc，避免为构造再拷一次位图。）
            let mut t = editcore::LayerTransform::new(RgbaImage::new(1, 1));
            t.source = scaled;
            t.rotation = base.rotation;
            t.skew = (base.skew.0 + skew.0, base.skew.1 + skew.1);
            t.center = (
                (base.center.0 + center_delta.0) * sx,
                (base.center.1 + center_delta.1) * sy,
            );
            let active = t.render(
                Rect {
                    x: 0,
                    y: 0,
                    width: target.width,
                    height: target.height,
                },
                false,
            );
            let stretched = match adjust {
                Some((AdjustTarget::Layer(index), params)) if index == layer_index => {
                    let mut adjusted = active;
                    params.apply_in_place(&mut adjusted);
                    adjusted
                }
                _ => active,
            };
            Self::blend_layer(&mut out, target, &stretched, 0, 0, drag_opacity, drag_blend, None, None, None);
        }
        doc.apply_display_color(&mut out);
        out
    }

    pub fn composite_live_rotate_adjusted(
        &mut self,
        doc: &Document,
        target: Size,
        layer_index: usize,
        snapshot: &RgbaImage,
        center: (f64, f64),
        degrees: f32,
        expand: bool,
        token: u64,
        adjust: Option<(AdjustTarget, Adjust)>,
    ) -> RgbaImage {
        let Some((sx, sy)) = self.prepare(doc, target) else {
            return RgbaImage::new(target.width.max(1), target.height.max(1));
        };
        let drag_opacity = doc
            .layers
            .get(layer_index)
            .map(|l| if l.visible { l.opacity } else { 0.0 })
            .unwrap_or(0.0);
        // The dragged layer keeps its own blend mode, otherwise a `Multiply`
        // layer would flash to Normal for the duration of the gesture.
        let drag_blend = doc
            .layers
            .get(layer_index)
            .map(|l| l.blend)
            .unwrap_or_default();
        let mut out = self.base_buffer(doc, target, sx, sy);
        if let Some((AdjustTarget::Base, params)) = adjust {
            params.apply_in_place(&mut out);
        }
        self.blend_upper_layers(doc, &mut out, target, sx, sy, Some(layer_index), adjust);
        if drag_opacity > 0.0 {
            // 预览尺度下的图层尺寸：Triangle 是「每个输出像素读 2×2」，代价
            // 随输出走，不会因为原图巨大而爆炸。
            let pw = ((snapshot.width() as f64 * sx).round() as u32).max(1);
            let ph = ((snapshot.height() as f64 * sy).round() as u32).max(1);
            // 缩放结果按 (会话号, w, h) 缓存：拖动时这三个值都不变，每帧省掉
            // 一次整层降采样（debug 下这是最大的一笔开销）。
            let hit = self
                .rotate_small
                .as_ref()
                .is_some_and(|(t, w, h, _)| *t == token && *w == pw && *h == ph);
            if !hit {
                let small = image::imageops::resize(snapshot, pw, ph, FilterType::Triangle);
                self.rotate_small = Some((token, pw, ph, Arc::new(small)));
            }
            let small = &self.rotate_small.as_ref().expect("just filled").3;
            // ★固定输出尺寸（对角线正方形、取偶 + 2px 余量）：整段拖拽里它
            // 不变，采样网格也就恒定，每帧只有角度在动。尺寸取偶是硬要求 ——
            // 奇数尺寸会让内容中心落在半像素上，见 `expanded_size` 的说明。
            let (nw, nh) = editcore::rotated_box_size(pw, ph, expand);
            let rad = degrees.to_radians();
            // ★关键：把内容中心摆到**落定后**的中心（画布坐标 * 预览倍率）。
            // 落定同样是 `round(center - W/2)`，W 恒为偶数 → 两边都锚在
            // `round(center)` 上，拖动不抖、松手不挪。
            let cxp = center.0 * sx;
            let cyp = center.1 * sy;
            let ox = editcore::round_center(cxp - nw as f64 / 2.0) as i32;
            let oy = editcore::round_center(cyp - nh as f64 / 2.0) as i32;
            let rotated = editcore::rotate_into_centered(
                small,
                rad.sin(),
                rad.cos(),
                nw,
                nh,
                ((cxp - ox as f64) as f32, (cyp - oy as f64) as f32),
            );
            // 诊断开关：`set IMAGEEDIT_ROTATE_TRACE=1` 才打印每帧几何，
            // 正常使用不刷日志（拖动时每帧一行会淹掉别的输出）。
            if std::env::var_os("IMAGEEDIT_ROTATE_TRACE").is_some() {
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"rotate-live\",\"deg\":{degrees:.2},\"s\":[{pw}x{ph}],\
                     \"out\":[{nw}x{nh}],\"place\":[{ox},{oy}],\"c\":[{cxp:.2},{cyp:.2}],\
                     \"dc\":[{:.2},{:.2}],\"sx\":{sx:.4}}}",
                    cxp - ox as f64,
                    cyp - oy as f64
                );
            }
            let src = match adjust {
                Some((AdjustTarget::Layer(index), params)) if index == layer_index => {
                    let mut adjusted = rotated;
                    params.apply_in_place(&mut adjusted);
                    adjusted
                }
                _ => rotated,
            };
            Self::blend_layer(&mut out, target, &src, ox, oy, drag_opacity, drag_blend, None, None, None);
        }
        doc.apply_display_color(&mut out);
        out
    }
}
