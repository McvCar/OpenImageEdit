use std::sync::Arc;

use image::RgbaImage;

use editcore::{Adjust, AdjustTarget, Document, Layer, LayerContent, LayerMask, Size};

use crate::scale::{crop_region, resize_to};

impl super::PreviewComposer {
    /// 背景层（`layers[0]`）的预览位图。
    ///
    /// 正常状态（背景未被移动）时走「按 `view` 裁剪 + 重采样」的快路径，避免
    /// 50MP 背景每帧整幅拷贝；被解锁拖走 / 缩放过时退回普通图层路径
    /// （`scaled_layer` + `blend_layer`），几何才是对的。
    pub(crate) fn base_buffer(&mut self, doc: &Document, target: Size, sx: f64, sy: f64) -> RgbaImage {
        let bg = &doc.layers[0];
        if !bg.visible || bg.opacity <= 0.0 {
            return RgbaImage::new(target.width, target.height);
        }
        let Some(r) = bg.raster() else {
            return RgbaImage::new(target.width, target.height);
        };
        let aligned = r.offset.0 == -doc.view.x && r.offset.1 == -doc.view.y;
        // 背景层带蒙版时不能走「整幅裁剪重采样」的快路径（快路径不做蒙版
        // 合成），退回普通图层路径。
        if !aligned || bg.mask.is_some() {
            let mut out = RgbaImage::new(target.width, target.height);
            let (ox, oy, nw, nh) =
                Self::preview_dst(r.offset, (r.image.width(), r.image.height()), sx, sy);
            let src = self.scaled_layer(0, &r.image, nw, nh);
            let mask = self.masked_layer(0, bg, nw, nh);
            let mask_ref = mask.as_deref().map(|m| (&m[..], nw));
            Self::blend_layer(
                &mut out,
                target,
                &src,
                ox,
                oy,
                bg.opacity,
                bg.blend,
                None,
                mask_ref,
                None,
            );
            return out;
        }
        let nw = ((doc.view.width as f64 * sx).round() as u32).max(1);
        let nh = ((doc.view.height as f64 * sy).round() as u32).max(1);
        let img = if let Some(img) = Self::cached_hit(&self.base, nw, nh) {
            (*img).clone()
        } else {
            // "Reset crop" leaves `view` equal to the whole bitmap; cropping
            // then is a pure full-size copy, so hand the base to the resampler
            // directly (a 50 MP base would otherwise be memcpy'd on every
            // render — the copy was ~35 ms of a 1100×688 preview render).
            let full_view = doc.view.x <= 0
                && doc.view.y <= 0
                && doc.view.width >= r.image.width()
                && doc.view.height >= r.image.height();
            let img = if full_view {
                resize_to(&r.image, target)
            } else {
                resize_to(&crop_region(&r.image, doc.view), target)
            };
            self.base = Some((nw, nh, Arc::new(img.clone())));
            img
        };
        if bg.opacity >= 0.999 {
            return img;
        }
        let mut out = RgbaImage::new(img.width(), img.height());
        Self::blend_layer(&mut out, target, &img, 0, 0, bg.opacity, bg.blend, None, None, None);
        out
    }

    /// 把 `layers[1..]` 按 z 序叠到 `out` 上（背景已由 `base_buffer` 画好）。
    /// 位图走缓存 + `blend_layer`，标记按同一套不透明度规则栅格化 ——
    /// 标记不再「恒置顶」，顺序就是图层顺序。
    pub(crate) fn blend_upper_layers(
        &mut self,
        doc: &Document,
        out: &mut RgbaImage,
        target: Size,
        sx: f64,
        sy: f64,
        skip: Option<usize>,
        adjust: Option<(AdjustTarget, Adjust)>,
    ) {
        // 剪贴蒙版基线覆盖率。只有「下一层是剪贴层」时才需要，避免每帧都为
        // 整条图层的每一层重采样一遍覆盖率。
        let mut clip: Option<Vec<u8>> = None;
        for (li, layer) in doc.layers.iter().enumerate().skip(1) {
            if !layer.clipping {
                let next_is_clip = doc
                    .layers
                    .get(li + 1)
                    .map(|next| next.clipping)
                    .unwrap_or(false);
                clip = if next_is_clip {
                    // 隐藏 / 全透明基底 → 整条剪贴链一起消失（与 core 渲染
                    // 的 fill_clip_coverage 同语义）。覆盖率仍要占位（Some
                    // 全 0）：传 None 会让链上的剪贴层被当成「无剪贴」按
                    // 不裁剪画出来。
                    if layer.visible && layer.opacity > 0.0 {
                        Some(self.clip_coverage(li, layer, target, sx, sy))
                    } else {
                        Some(vec![
                            0u8;
                            target.width as usize * target.height as usize
                        ])
                    }
                } else {
                    None
                };
            }
            if !layer.visible || layer.opacity <= 0.0 || skip == Some(li) {
                continue;
            }
            let clip_alpha = if layer.clipping {
                clip.as_deref()
            } else {
                None
            };
            match &layer.content {
                LayerContent::Mark(m) => {
                    if layer.opacity >= 0.999 {
                        m.draw_scaled(out, sx as f32, sy as f32);
                    } else {
                        let mut tmp = out.clone();
                        m.draw_scaled(&mut tmp, sx as f32, sy as f32);
                        for (dst, src) in out.pixels_mut().zip(tmp.pixels()) {
                            let p = dst.0;
                            dst.0 = [
                                (p[0] as f32 * (1.0 - layer.opacity)
                                    + src.0[0] as f32 * layer.opacity)
                                    .round() as u8,
                                (p[1] as f32 * (1.0 - layer.opacity)
                                    + src.0[1] as f32 * layer.opacity)
                                    .round() as u8,
                                (p[2] as f32 * (1.0 - layer.opacity)
                                    + src.0[2] as f32 * layer.opacity)
                                    .round() as u8,
                                (p[3] as f32 * (1.0 - layer.opacity)
                                    + src.0[3] as f32 * layer.opacity)
                                    .round() as u8,
                            ];
                        }
                    }
                }
                LayerContent::Raster(r) => {
                    let (ox, oy, nw, nh) =
                        Self::preview_dst(r.offset, (r.image.width(), r.image.height()), sx, sy);
                    let src = self.scaled_layer(li, &r.image, nw, nh);
                    let mask = self.masked_layer(li, layer, nw, nh);
                    let mask_ref = mask.as_deref().map(|m| (&m[..], nw));
                    // 内容之外的样式笔迹：这些像素按样式自己的混合模式混合。
                    let style = layer.style.as_ref().map(|s| {
                        let scaled = self.scaled_style(
                            li,
                            &s.mask,
                            (r.image.width(), r.image.height()),
                            nw,
                            nh,
                        );
                        (scaled, s.blend)
                    });
                    let style_ref = style.as_ref().map(|(m, b)| (&m[..], *b));
                    match adjust {
                        Some((AdjustTarget::Layer(index), params)) if index == li => {
                            let mut adjusted = (*src).clone();
                            params.apply_in_place(&mut adjusted);
                            Self::blend_layer(
                                out,
                                target,
                                &adjusted,
                                ox,
                                oy,
                                layer.opacity,
                                layer.blend,
                                clip_alpha,
                                mask_ref,
                                style_ref,
                            );
                        }
                        _ => Self::blend_layer(
                            out,
                            target,
                            &src,
                            ox,
                            oy,
                            layer.opacity,
                            layer.blend,
                            clip_alpha,
                            mask_ref,
                            style_ref,
                        ),
                    }
                }
            }
        }
    }

    /// 位图在预览里的目标矩形 `(x, y, w, h)`（整数预览像素）。
    ///
    /// ★两端各自取整：起点 `floor`、终点 `ceil`；**不能**用「起点四舍五入 +
    ///   宽高四舍五入」。后者会让图层比真实落点少覆盖不到 1 个预览像素，而
    ///   背景上被挖走的那块洞是按面积比降采样的（洞边缘自带分数 alpha），
    ///   差出来的那半行就透出棋盘格 —— 在 13% 这类缩小档下正好是一条浅色
    ///   「缝」，看起来像「剪切出来的内容粘回去没贴严」（用户报的就是这个）。
    ///
    /// 实测：120×80 的粘贴层放在 (100,100)，缩到 13% 时真实占位是
    /// x 13.0..28.6 / y 13.0..23.4；旧算法给出 16×10 @ (13,13)，只盖到
    /// 23 行，第 23 行留下 alpha=0.62 的缺口。改成向外取整后是 16×11，
    /// 洞被完全盖住。两个相邻图层宁可重叠 <1 像素，也不该留缝。
    ///
    /// 1:1 与整数倍缩放下两端本来就落在整数上，行为完全不变。
    fn preview_dst(offset: (i32, i32), size: (u32, u32), sx: f64, sy: f64) -> (i32, i32, u32, u32) {
        const EPS: f64 = 1e-6;
        let x0 = (offset.0 as f64 * sx + EPS).floor();
        let y0 = (offset.1 as f64 * sy + EPS).floor();
        let x1 = ((offset.0 as f64 + size.0 as f64) * sx - EPS).ceil();
        let y1 = ((offset.1 as f64 + size.1 as f64) * sy - EPS).ceil();
        let w = (x1 - x0).round().max(1.0) as u32;
        let h = (y1 - y0).round().max(1.0) as u32;
        (x0 as i32, y0 as i32, w, h)
    }

    /// Cached preview-scaled copy of a layer bitmap, stored in slot `li`.
    fn scaled_layer(&mut self, li: usize, img: &RgbaImage, nw: u32, nh: u32) -> Arc<RgbaImage> {
        let slot = self.layers.get(li).cloned().flatten();
        if let Some(cached) = Self::cached_hit(&slot, nw, nh) {
            return cached;
        }
        let scaled = Arc::new(resize_to(img, Size::new(nw, nh)));
        if li < self.layers.len() {
            self.layers[li] = Some((nw, nh, scaled.clone()));
        }
        scaled
    }

    /// Alpha-over blend of `src` at `(ox, oy)` into `out_buf`, applying
    /// `opacity` at draw time. Row-slice fast path: whole rows are handed
    /// out as byte slices so the per-tick cost stays low even in debug
    /// builds.
    ///
    /// `clip` 是剪贴蒙版基线层的覆盖率（预览尺度，与 `out_buf` 同尺寸）；
    /// `mask` 是图层蒙版的覆盖率（**图层自己的矩形尺寸**，宽度一并传进来）。
    /// `style` 是「内容之外的样式笔迹」掩码（图层矩形尺寸）连同样式自己的
    /// 混合模式：那些像素改用它混合，见 [`editcore::StylePixels`]。
    /// 两者都是「src alpha 乘数」，先乘蒙版再乘剪贴 —— 与
    /// `Document::flatten_in_doc_space` 的 `composite_raster` 同序。
    /// 基底层 / 无剪贴层自己混合时对应参数传 `None`。
    fn blend(
        out_buf: &mut [u8],
        target: Size,
        src: &RgbaImage,
        ox: i32,
        oy: i32,
        opacity: f32,
        mode: editcore::BlendMode,
        clip: Option<&[u8]>,
        mask: Option<(&[u8], u32)>,
        style: Option<(&[u8], editcore::BlendMode)>,
    ) {
        let nw = src.width() as i32;
        let nh = src.height() as i32;
        let x0 = ox.max(0) as u32;
        let x1 = (ox + nw).min(target.width as i32).max(0) as u32;
        if x0 >= x1 {
            return;
        }
        let src_stride = nw as usize * 4;
        let src_buf: &[u8] = src.as_raw();
        let out_stride = target.width as usize * 4;
        // 有样式掩码时不能走 Normal 快路径：每个像素的模式可能不同。
        let normal = mode.is_normal() && style.is_none();
        for ty in 0..target.height {
            let ly = ty as i64 - oy as i64;
            if ly < 0 || ly >= nh as i64 {
                continue;
            }
            let srow = &src_buf[ly as usize * src_stride..(ly as usize + 1) * src_stride];
            let orow =
                &mut out_buf[ty as usize * out_stride..(ty as usize + 1) * out_stride];
            for tx in x0..x1 {
                let lx = tx as i64 - ox as i64;
                let so = lx as usize * 4;
                let oo = tx as usize * 4;
                if normal {
                    // 与 `composite_raster` + `blend_pixel_f32` 完全同序：
                    // alpha = (a/255 × 蒙版) × 剪贴，再乘图层不透明度。
                    let mut alpha = srow[so + 3] as f32 / 255.0;
                    if let Some((m, mw)) = mask {
                        alpha *= m[ly as usize * mw as usize + lx as usize] as f32 / 255.0;
                    }
                    if let Some(c) = clip {
                        let ci = ty as usize * target.width as usize + tx as usize;
                        alpha *= c[ci] as f32 / 255.0;
                    }
                    let sa = alpha * opacity;
                    if sa <= 0.0 {
                        continue;
                    }
                    let inv = 1.0 - sa;
                    for c in 0..3 {
                        orow[oo + c] =
                            (srow[so + c] as f32 * sa + orow[oo + c] as f32 * inv).round() as u8;
                    }
                    orow[oo + 3] = ((sa + orow[oo + 3] as f32 / 255.0 * inv) * 255.0).round() as u8;
                } else {
                    let mut dst = [
                        orow[oo],
                        orow[oo + 1],
                        orow[oo + 2],
                        orow[oo + 3],
                    ];
                    let mut px = [
                        srow[so],
                        srow[so + 1],
                        srow[so + 2],
                        srow[so + 3],
                    ];
                    if let Some((m, mw)) = mask {
                        let mcov = m[ly as usize * mw as usize + lx as usize] as u32;
                        px[3] = ((px[3] as u32 * mcov + 127) / 255) as u8;
                    }
                    if let Some(c) = clip {
                        let cov = c[ty as usize * target.width as usize + tx as usize] as u32;
                        px[3] = ((px[3] as u32 * cov + 127) / 255) as u8;
                    }
                    let px_mode = match style {
                        Some((s, smode)) if s[ly as usize * nw as usize + lx as usize] != 0 => {
                            smode
                        }
                        _ => mode,
                    };
                    // 溶解：按目标像素坐标抖动成 0/1，再走 Normal；1:1 预览下
                    // `tx` 就是文档坐标，与 flatten 的抖动图案逐像素一致。
                    let (px, op, px_mode) = if px_mode.is_dissolve() {
                        let a = px[3] as f32 / 255.0 * opacity;
                        if editcore::dissolve_hit(tx as i32, ty as i32, a) {
                            ([px[0], px[1], px[2], 255], 1.0, editcore::BlendMode::Normal)
                        } else {
                            ([0, 0, 0, 0], 0.0, editcore::BlendMode::Normal)
                        }
                    } else {
                        (px, opacity, px_mode)
                    };
                    editcore::blend_pixel(&mut dst, px, op, px_mode);
                    orow[oo..oo + 4].copy_from_slice(&dst);
                }
            }
        }
    }

    pub(crate) fn blend_layer(
        out: &mut RgbaImage,
        target: Size,
        src: &RgbaImage,
        ox: i32,
        oy: i32,
        opacity: f32,
        mode: editcore::BlendMode,
        clip: Option<&[u8]>,
        mask: Option<(&[u8], u32)>,
        style: Option<(&[u8], editcore::BlendMode)>,
    ) {
        let mut raw = std::mem::take(out).into_raw();
        Self::blend(&mut raw, target, src, ox, oy, opacity, mode, clip, mask, style);
        *out = RgbaImage::from_raw(target.width, target.height, raw)
            .expect("preview buffer size preserved");
    }

    /// 蒙版在画布区间 `[x0, x1) × [y0, y1)` 上的平均覆盖率（u8）。
    ///
    /// 区间端点向外取整后逐像素取**字节**平均（不是取采样点），与位图的降
    /// 采样口径一致：`downscale_box` 的一个输出像素正好覆盖源图同一段区间，
    /// 蒙版按同一段区间取平均才不会在缩略图上「透出/多挡」半行。
    ///
    /// 缩放比恰好为 1 时区间里只有一个像素，结果逐字节等于
    /// `mask.coverage()` 的 u8 值，1:1 预览才能和 flatten 逐字节一致。
    fn mask_cov_range(mask: &LayerMask, x0: f64, x1: f64, y0: f64, y1: f64) -> u8 {
        let cx0 = x0.floor() as i32;
        let cx1 = (x1.ceil() as i32).max(cx0 + 1);
        let cy0 = y0.floor() as i32;
        let cy1 = (y1.ceil() as i32).max(cy0 + 1);
        let mut sum = 0u64;
        let mut n = 0u64;
        for y in cy0..cy1 {
            for x in cx0..cx1 {
                let lx = x - mask.offset.0;
                let ly = y - mask.offset.1;
                let v = if lx < 0
                    || ly < 0
                    || lx >= mask.image.width() as i32
                    || ly >= mask.image.height() as i32
                {
                    mask.default
                } else {
                    mask.image.get_pixel(lx as u32, ly as u32).0[0]
                };
                sum += v as u64;
                n += 1;
            }
        }
        ((sum + n / 2) / n.max(1)) as u8
    }

    /// 一层位图在预览尺度上的蒙版覆盖率数组（长度 = `nw * nh`）。
    ///
    /// 输出像素 `(tx, ty)` 覆盖源图局部区间 `[tx*sw/nw, (tx+1)*sw/nw)`，
    /// 加图层 offset 后交给 [`Self::mask_cov_range`] —— 与 `resize_to` 对同一
    /// 层位图做的区间平均严格对应，缩小时不会因 `preview_dst` 的 floor/ceil
    /// 而错位。
    fn mask_cov_for_rect(
        mask: &LayerMask,
        offset: (i32, i32),
        sw: u32,
        sh: u32,
        nw: u32,
        nh: u32,
    ) -> Vec<u8> {
        let mut cov = vec![255u8; nw as usize * nh as usize];
        if nw == 0 || nh == 0 {
            return cov;
        }
        let rx = sw as f64 / nw as f64;
        let ry = sh as f64 / nh as f64;
        for ty in 0..nh {
            let y0 = offset.1 as f64 + ty as f64 * ry;
            let y1 = offset.1 as f64 + (ty as f64 + 1.0) * ry;
            for tx in 0..nw {
                let x0 = offset.0 as f64 + tx as f64 * rx;
                let x1 = offset.0 as f64 + (tx as f64 + 1.0) * rx;
                cov[ty as usize * nw as usize + tx as usize] =
                    Self::mask_cov_range(mask, x0, x1, y0, y1);
            }
        }
        cov
    }

    /// 图层蒙版在预览尺度上的 alpha 乘数（长度 = 该图层缩放后的像素数）。
    ///
    /// 每帧为每个有蒙版的图层重算一次是整块 O(像素) 的活，所以按
    /// `(offset.x, offset.y, nw, nh)` 缓存到 `self.masks[li]`：拖动只改
    /// offset 时缓存失效一次，纯重绘（offset / 尺寸都没变）直接命中；换
    /// epoch / 换预览尺寸由 `prepare()` 统一清空。全 255（蒙版没有真正遮挡）
    /// 时返回 `None`，让热路径不付任何逐像素代价。
    ///
    /// 矢量标记不在这里处理：`flatten_in_doc_space` 的 Mark 分支不应用蒙版，
    /// 预览保持同一语义。
    fn masked_layer(&mut self, li: usize, layer: &Layer, nw: u32, nh: u32) -> Option<Arc<Vec<u8>>> {
        let Some(mask) = layer.mask.as_ref() else {
            return None;
        };
        let Some(r) = layer.raster() else {
            return None;
        };
        if let Some((cx, cy, cw, ch, cov)) = self.masks.get(li).cloned().flatten() {
            if cx == r.offset.0 && cy == r.offset.1 && cw == nw && ch == nh {
                return Some(cov);
            }
        }
        let cov = Self::mask_cov_for_rect(
            mask,
            r.offset,
            r.image.width(),
            r.image.height(),
            nw,
            nh,
        );
        if cov.iter().all(|&c| c == 255) {
            if li < self.masks.len() {
                self.masks[li] = None;
            }
            return None;
        }
        let arc = Arc::new(cov);
        if li < self.masks.len() {
            self.masks[li] = Some((r.offset.0, r.offset.1, nw, nh, arc.clone()));
        }
        Some(arc)
    }

    /// 剪贴基线层的覆盖率（预览尺度 u8 alpha）。
    ///
    /// 与 `Document::flatten_in_doc_space` 里的 `fill_clip_coverage` 同语义：
    /// 覆盖率 = 基底位图 alpha × 基底自己的图层蒙版；隐藏 / 全透明的基底给出
    /// 全 0，于是整条剪贴链一起消失；基底只是「半透明」（opacity < 1）时覆盖
    /// 率仍给满，不乘不透明度。
    ///
    /// 覆盖率必须落在**预览尺度**上：图层在预览里是缩小后混合的，如果拿文
    /// 档尺度的覆盖率去索引就会错位，剪贴内容会跑到基线框外面。
    fn clip_coverage(
        &mut self,
        li: usize,
        layer: &Layer,
        target: Size,
        sx: f64,
        sy: f64,
    ) -> Vec<u8> {
        if !layer.visible || layer.opacity <= 0.0 {
            return vec![0u8; target.width as usize * target.height as usize];
        }
        // 基底带烘焙样式时，边界取它的**内容**形状：样式额外画出来的外描边 /
        // 投影不算内容，否则剪贴内容会铺到描边上（与 flatten 侧
        // `fill_clip_coverage` 同口径）。
        if let Some(shape) = layer.clip_shape.as_ref() {
            return self.clip_shape_coverage(li, layer, shape, target, sx, sy);
        }
        let mut cov = vec![0u8; target.width as usize * target.height as usize];
        let (src, ox, oy, mcov) = match &layer.content {
            LayerContent::Raster(r) => {
                let (ox, oy, nw, nh) =
                    Self::preview_dst(r.offset, (r.image.width(), r.image.height()), sx, sy);
                let src = self.scaled_layer(li, &r.image, nw, nh);
                let mcov = self.masked_layer(li, layer, nw, nh);
                (src, ox, oy, mcov)
            }
            LayerContent::Mark(_) => {
                let (plane, offset) = layer.plane();
                let (ox, oy, nw, nh) =
                    Self::preview_dst(offset, (plane.width(), plane.height()), sx, sy);
                // 与 `fill_clip_coverage` 一样，标记基底也要乘自己的蒙版。
                let mcov = layer.mask.as_ref().map(|mask| {
                    Arc::new(Self::mask_cov_for_rect(
                        mask,
                        offset,
                        plane.width(),
                        plane.height(),
                        nw,
                        nh,
                    ))
                });
                (
                    Arc::new(resize_to(&plane, Size::new(nw, nh))),
                    ox,
                    oy,
                    mcov,
                )
            }
        };
        let mw = src.width();
        let tw = target.width as i32;
        let th = target.height as i32;
        for ty in 0..src.height() as i32 {
            let py = oy + ty;
            if py < 0 || py >= th {
                continue;
            }
            for tx in 0..src.width() as i32 {
                let px = ox + tx;
                if px < 0 || px >= tw {
                    continue;
                }
                let mut a = src.get_pixel(tx as u32, ty as u32).0[3];
                if a > 0 {
                    if let Some(m) = mcov.as_deref() {
                        let m = m[ty as usize * mw as usize + tx as usize] as u32;
                        a = ((a as u32 * m + 127) / 255) as u8;
                    }
                }
                if a > 0 {
                    cov[py as usize * tw as usize + px as usize] = a;
                }
            }
        }
        cov
    }

    /// 剪贴基底的「内容形状」在预览尺度上的覆盖率。
    ///
    /// 与 [`Self::clip_coverage`] 的位图路径同语义（同样的预览矩形、同样的
    /// 蒙版顺序），只是边界换成烘焙样式**之前**的内容 alpha。形状的
    /// 区间平均借 `resize_to` 走同一条降采样路径，缩小时两者的边缘口径一致。
    fn clip_shape_coverage(
        &mut self,
        li: usize,
        layer: &Layer,
        shape: &LayerMask,
        target: Size,
        sx: f64,
        sy: f64,
    ) -> Vec<u8> {
        let mut cov = vec![0u8; target.width as usize * target.height as usize];
        let (sw, sh) = shape.image.dimensions();
        let (ox, oy, nw, nh) = Self::preview_dst(shape.offset, (sw, sh), sx, sy);
        let scaled = self.scaled_clip_shape(li, shape, nw, nh);
        let mcov = layer
            .mask
            .as_ref()
            .map(|m| Self::mask_cov_for_rect(m, shape.offset, sw, sh, nw, nh));
        let tw = target.width as i32;
        let th = target.height as i32;
        for ty in 0..nh as i32 {
            let py = oy + ty;
            if py < 0 || py >= th {
                continue;
            }
            for tx in 0..nw as i32 {
                let px = ox + tx;
                if px < 0 || px >= tw {
                    continue;
                }
                let i = ty as usize * nw as usize + tx as usize;
                let a = scaled[i];
                if a == 0 {
                    continue;
                }
                let a = match mcov.as_deref() {
                    Some(m) => ((a as u32 * m[i] as u32 + 127) / 255) as u8,
                    None => a,
                };
                if a > 0 {
                    cov[py as usize * tw as usize + px as usize] = a;
                }
            }
        }
        cov
    }

    /// 内容形状缩到预览尺度（单通道 alpha），按 `(offset, w, h)` 缓存。
    ///
    /// 单通道借用 RGBA 的重采样路径：alpha 的区间平均与同尺寸位图完全同
    /// 口径，1:1 时逐字节不变（预览与 flatten 才能逐字节对上）。
    fn scaled_clip_shape(
        &mut self,
        li: usize,
        shape: &LayerMask,
        nw: u32,
        nh: u32,
    ) -> Arc<Vec<u8>> {
        let key = (shape.offset.0, shape.offset.1, nw, nh);
        if let Some((cx, cy, cw, ch, cov)) = self.clip_shapes.get(li).cloned().flatten() {
            if (cx, cy, cw, ch) == key {
                return cov;
            }
        }
        let (sw, sh) = shape.image.dimensions();
        let mut plane = RgbaImage::from_pixel(sw, sh, image::Rgba([0, 0, 0, 0]));
        for (dst, src) in plane.pixels_mut().zip(shape.image.pixels()) {
            dst.0[3] = src.0[0];
        }
        let scaled = resize_to(&plane, Size::new(nw, nh));
        let cov = Arc::new(scaled.pixels().map(|p| p.0[3]).collect::<Vec<u8>>());
        if li < self.clip_shapes.len() {
            self.clip_shapes[li] = Some((
                shape.offset.0,
                shape.offset.1,
                nw,
                nh,
                cov.clone(),
            ));
        }
        cov
    }

    /// 「内容之外的样式笔迹」缩到预览尺度（单通道），按 `(w, h)` 缓存。
    fn scaled_style(
        &mut self,
        li: usize,
        mask: &[u8],
        size: (u32, u32),
        nw: u32,
        nh: u32,
    ) -> Arc<Vec<u8>> {
        if let Some((cw, ch, cov)) = self.styles.get(li).cloned().flatten() {
            if (cw, ch) == (nw, nh) {
                return cov;
            }
        }
        let mut plane = RgbaImage::from_pixel(size.0, size.1, image::Rgba([0, 0, 0, 0]));
        for (dst, src) in plane.as_mut().chunks_exact_mut(4).zip(mask.iter()) {
            dst[3] = *src;
        }
        let scaled = resize_to(&plane, Size::new(nw, nh));
        let cov = Arc::new(scaled.pixels().map(|p| p.0[3]).collect::<Vec<u8>>());
        if li < self.styles.len() {
            self.styles[li] = Some((nw, nh, cov.clone()));
        }
        cov
    }
}
