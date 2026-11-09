// crates/core/src/layer_ops.rs
//! 图层命令：新建 / 复制 / 合并 / 拼合 / 栅格化 / 对齐 / 分布。
//!
//! 这些命令都以「一条历史」的形式落到文档上（合并类内部走
//! [`EditOp::Batch`]），所以 Ctrl+Z 一次就能退回命令之前，而不是要按十几
//! 下。几何一律在**画布坐标**里算，图层自己的 `offset` 只负责落到文档。
use super::*;

/// 对齐的参照边 / 中心。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignEdge {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

impl AlignEdge {
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "left" => Self::Left,
            "hcenter" | "cx" => Self::HCenter,
            "right" => Self::Right,
            "top" => Self::Top,
            "vcenter" | "cy" => Self::VCenter,
            "bottom" => Self::Bottom,
            _ => return None,
        })
    }
}

/// 分布方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributeAxis {
    Horizontal,
    Vertical,
}

impl DistributeAxis {
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "h" | "horizontal" => Self::Horizontal,
            "v" | "vertical" => Self::Vertical,
            _ => return None,
        })
    }
}

/// 把 `src` 图像按不透明度叠到 `out` 的 `(dx, dy)` 处（越界自动裁剪）。
pub(crate) fn blend_image_at(
    out: &mut RgbaImage,
    src: &RgbaImage,
    dx: i32,
    dy: i32,
    opacity: f32,
) {
    if opacity <= 0.0 {
        return;
    }
    let x0 = dx.max(0);
    let y0 = dy.max(0);
    let x1 = (dx + src.width() as i32).min(out.width() as i32);
    let y1 = (dy + src.height() as i32).min(out.height() as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            let s = src.get_pixel((x - dx) as u32, (y - dy) as u32).0;
            if s[3] == 0 {
                continue;
            }
            let px = out.get_pixel_mut(x as u32, y as u32);
            blend_over(&mut px.0, s, opacity);
        }
    }
}

/// 把一个图层按**画布坐标**叠到以 `origin` 为原点的图像上。
fn draw_layer_into(out: &mut RgbaImage, origin: (i32, i32), layer: &Layer) {
    match &layer.content {
        LayerContent::Raster(r) => blend_image_at(
            out,
            &r.image,
            r.offset.0 - origin.0,
            r.offset.1 - origin.1,
            layer.opacity,
        ),
        LayerContent::Mark(m) => {
            // 标记直接用画布坐标绘制：先画到同尺寸临时图再按不透明度混合，
            // 这样「标记的不透明度」与整层合成保持同一套语义。
            let mut tmp = RgbaImage::new(out.width(), out.height());
            m.translated(-origin.0 as f32, -origin.1 as f32).draw(&mut tmp);
            blend_image_at(out, &tmp, 0, 0, layer.opacity);
        }
    }
}

impl Document {
    /// 新建空（全透明）图层，盖住整个画布并插到栈顶。
    pub fn new_empty_layer(&mut self) -> Result<usize, EditError> {
        let image = RgbaImage::new(self.canvas.width, self.canvas.height);
        let mut layer = Layer::new(self.next_layer_name(), image);
        layer.set_offset((-self.view.x, -self.view.y));
        let index = self.layers.len();
        self.execute(EditOp::AddLayer { layer, index })?;
        Ok(index)
    }

    /// 复制图层：副本插到原件**上面**并偏移 8px（与 PS 一致，避免完全重叠
    /// 让人以为没生效）。
    pub fn duplicate_layer(&mut self, index: usize) -> Result<usize, EditError> {
        let src = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .duplicated();
        let mut dup = src;
        let (ox, oy) = dup.offset();
        dup.set_offset((ox + DUPLICATE_SHIFT, oy + DUPLICATE_SHIFT));
        if let Some(t) = dup.xform_mut() {
            t.center = (
                t.center.0 + DUPLICATE_SHIFT as f64,
                t.center.1 + DUPLICATE_SHIFT as f64,
            );
        }
        let at = index + 1;
        self.execute(EditOp::AddLayer {
            layer: dup,
            index: at,
        })?;
        Ok(at)
    }

    /// 向下合并：本层合成到下面一层，本层删除。
    ///
    /// 下面那层是**标记**也照样能合并 —— 先把它的几何画成位图再叠，结果层
    /// 一律是位图（PS 的「向下合并」不会因为目标层是矢量就罢工）。
    ///
    /// 返回合并后图层的新下标（= `index - 1`）。
    pub fn merge_down(&mut self, index: usize) -> Result<usize, EditError> {
        if index == 0 || index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        let below = index - 1;
        let dest_before = self.layers[below].clone();
        let src = self.layers[index].clone();

        // 结果层的底图 = 下层自己的「画布平面」（先把它的不透明度烘焙进去，
        // 否则合并后再按 dest.opacity 合成会把它的透明度算两遍）。
        let (mut dest_img, dest_offset) = dest_before.plane();
        bake_opacity(&mut dest_img, dest_before.opacity);

        // ★范围必须取两层的**并集**：上层可能比下层大、也可能偏出去，沿用
        //  下层的尺寸会把多出来的那块像素直接裁掉（视觉上「合并后少了东西」）。
        let sb = src.bounds();
        let x0 = dest_offset.0.min(sb.x);
        let y0 = dest_offset.1.min(sb.y);
        let x1 = (dest_offset.0 + dest_img.width() as i32)
            .max(sb.x + sb.width as i32);
        let y1 = (dest_offset.1 + dest_img.height() as i32)
            .max(sb.y + sb.height as i32);
        let mut merged = RgbaImage::new((x1 - x0) as u32, (y1 - y0) as u32);
        blend_image_at(
            &mut merged,
            &dest_img,
            dest_offset.0 - x0,
            dest_offset.1 - y0,
            1.0,
        );
        draw_layer_into(&mut merged, (x0, y0), &src);

        let mut after = dest_before.clone();
        after.content = LayerContent::Raster(RasterLayer {
            image: merged,
            offset: (x0, y0),
            transform: None,
        });
        after.opacity = 1.0;

        self.execute(EditOp::Batch(vec![
            EditOp::ReplaceLayer {
                index: below,
                before: dest_before,
                after,
            },
            EditOp::RemoveLayer {
                index,
                layer: src,
            },
        ]))?;
        Ok(below)
    }

    /// 合并**选中的**图层（PS 的「合并图层」，菜单项只在多选时可用）。
    ///
    /// 结果落在最高那层的 z 位上，名字 / 锁定状态沿用最高层，内容按 z 序
    /// 从低到高合成 —— 所以合并前后画面完全一致。背景层（下标 0）永远不
    /// 参与：它是文档的底，合进去就没有「背景」这个概念了。
    ///
    /// 少于两层时返回 [`EditError::BaseRequired`]（UI 会先拦一次给中文提示）。
    pub fn merge_layers(&mut self, indices: &[usize]) -> Result<usize, EditError> {
        let mut targets: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&i| i > 0 && i < self.layers.len())
            .collect();
        targets.sort_unstable();
        targets.dedup();
        if targets.len() < 2 {
            return Err(EditError::BaseRequired);
        }
        // 范围取所有目标的**并集**：图层可能各自偏出去，用单层尺寸会把多出来
        // 的那块像素裁掉（视觉上「合并后少了东西」）。
        let (mut x0, mut y0) = (i32::MAX, i32::MAX);
        let (mut x1, mut y1) = (i32::MIN, i32::MIN);
        for &i in &targets {
            let b = self.layers[i].bounds();
            x0 = x0.min(b.x);
            y0 = y0.min(b.y);
            x1 = x1.max(b.x + b.width as i32);
            y1 = y1.max(b.y + b.height as i32);
        }
        // ★并集可能退化：两层都是空的（新建的空图层 0×0，或整体在画布外）
        //   时 x1==x0 / y1==y0。这里不能报错 —— 「合并不了」是用户最没法
        //   自救的反馈。按 1×1 透明块落定：层数照减，内容为空，可撤销。
        let w = (x1 - x0).max(1) as u32;
        let h = (y1 - y0).max(1) as u32;
        let mut merged = RgbaImage::new(w, h);
        // 由低到高叠：`draw_layer_into` 会把每层自己的不透明度烘焙进去。
        for &i in &targets {
            draw_layer_into(&mut merged, (x0, y0), &self.layers[i]);
        }

        let top = *targets.last().unwrap();
        let before = self.layers[top].clone();
        let mut after = before.clone();
        after.content = LayerContent::Raster(RasterLayer {
            image: merged,
            offset: (x0, y0),
            transform: None,
        });
        after.opacity = 1.0;
        after.visible = true;

        // 被删掉的都在 top 下面（targets 升序），所以先 Replace 再按降序删，
        // 每次删除都不会影响还没处理的那些下标。
        let mut ops = vec![EditOp::ReplaceLayer {
            index: top,
            before,
            after,
        }];
        for &i in targets.iter().rev() {
            if i != top {
                ops.push(EditOp::RemoveLayer {
                    index: i,
                    layer: self.layers[i].clone(),
                });
            }
        }
        self.execute(EditOp::Batch(ops))?;
        Ok(top - (targets.len() - 1))
    }

    /// 合并所有可见图层（背景层不参与，保持垫底）为一层。
    ///
    /// 结果放在最低那个可见层的 z 位上，名字「合并图层」，不透明度 100%。
    pub fn merge_visible(&mut self) -> Result<usize, EditError> {
        let visible: Vec<usize> = (1..self.layers.len())
            .filter(|&i| self.layers[i].visible && self.layers[i].opacity > 0.0)
            .collect();
        if visible.len() < 2 {
            return Err(EditError::BaseRequired);
        }
        let origin = (-self.view.x, -self.view.y);
        let mut merged = RgbaImage::new(self.canvas.width, self.canvas.height);
        for &i in &visible {
            draw_layer_into(&mut merged, origin, &self.layers[i]);
        }

        let at = visible[0];
        let mut layer = Layer::new("合并图层", merged);
        layer.set_offset(origin);

        // 先按**从高到低**删（高下标先删，剩下的下标不受影响），再把合并
        // 结果插回原最低位。撤销时逆序执行，天然还原。
        let mut ops = Vec::new();
        for &i in visible.iter().rev() {
            ops.push(EditOp::RemoveLayer {
                index: i,
                layer: self.layers[i].clone(),
            });
        }
        ops.push(EditOp::AddLayer {
            layer,
            index: at,
        });
        self.execute(EditOp::Batch(ops))?;
        Ok(at)
    }

    /// 拼合图像：所有可见内容合进背景层，其余图层全部删除。
    pub fn flatten_stack(&mut self) -> Result<(), EditError> {
        if self.layers.len() <= 1 {
            return Err(EditError::BaseRequired);
        }
        let flat = self.flatten();
        let before = self.layers[0].clone();
        let mut after = before.clone();
        if let Some(r) = after.raster_mut() {
            r.image = flat;
            r.transform = None;
            r.offset = (-self.view.x, -self.view.y);
        }
        after.opacity = 1.0;
        after.visible = true;

        let mut ops = vec![EditOp::ReplaceLayer {
            index: 0,
            before,
            after,
        }];
        for i in (1..self.layers.len()).rev() {
            ops.push(EditOp::RemoveLayer {
                index: i,
                layer: self.layers[i].clone(),
            });
        }
        self.execute(EditOp::Batch(ops))
    }

    /// 标记栅格化：把矢量标记画成位图图层，z 位与属性保留。
    pub fn rasterize_mark(&mut self, index: usize) -> Result<usize, EditError> {
        let before = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .clone();
        let mark = match &before.content {
            LayerContent::Mark(m) => m.clone(),
            LayerContent::Raster(_) => return Err(EditError::LayerIndex(index)),
        };
        let rect = mark.canvas_rect();
        // 描边有一半在 rect 外面，四周留出 描边宽/2 + 2 的余量。
        let pad = (mark.stroke.max(1) / 2 + 2) as i32;
        let ox = rect.x - pad;
        let oy = rect.y - pad;
        let w = (rect.width as i32 + pad * 2).max(1) as u32;
        let h = (rect.height as i32 + pad * 2).max(1) as u32;
        let mut image = RgbaImage::new(w, h);
        mark.translated(-ox as f32, -oy as f32).draw(&mut image);

        let mut after = before.clone();
        after.content = LayerContent::Raster(RasterLayer {
            image,
            offset: (ox, oy),
            transform: None,
        });
        self.execute(EditOp::ReplaceLayer {
            index,
            before,
            after,
        })?;
        Ok(index)
    }

    /// 把 `indices` 里的图层按 `edge` 对齐到 `reference`（画布或选区框）。
    ///
    /// 锁定图层跳过（锁定的语义就是「别动我」）；整体算一条历史。
    pub fn align_layers(
        &mut self,
        indices: &[usize],
        edge: AlignEdge,
        reference: Rect,
    ) -> Result<(), EditError> {
        let mut ops = Vec::new();
        for &i in indices {
            let Some(layer) = self.layers.get(i) else {
                continue;
            };
            if layer.locked {
                continue;
            }
            let cur = layer.offset();
            let b = layer.bounds();
            let (dx, dy) = match edge {
                AlignEdge::Left => (reference.x - b.x, 0),
                AlignEdge::HCenter => (
                    reference.x + (reference.width as i32 - b.width as i32) / 2 - b.x,
                    0,
                ),
                AlignEdge::Right => (
                    reference.x + reference.width as i32 - (b.x + b.width as i32),
                    0,
                ),
                AlignEdge::Top => (0, reference.y - b.y),
                AlignEdge::VCenter => (
                    0,
                    reference.y + (reference.height as i32 - b.height as i32) / 2 - b.y,
                ),
                AlignEdge::Bottom => (
                    0,
                    reference.y + reference.height as i32 - (b.y + b.height as i32),
                ),
            };
            if dx == 0 && dy == 0 {
                continue;
            }
            ops.push(EditOp::TranslateLayer {
                index: i,
                from: cur,
                to: (cur.0 + dx, cur.1 + dy),
            });
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.execute(EditOp::Batch(ops))
    }

    /// 按 `axis` 等间距分布：首尾两层不动，中间各层的**中心**均匀铺开。
    pub fn distribute_layers(
        &mut self,
        indices: &[usize],
        axis: DistributeAxis,
    ) -> Result<(), EditError> {
        let mut items: Vec<(usize, f64)> = Vec::new(); // (index, 中心坐标)
        for &i in indices {
            let Some(layer) = self.layers.get(i) else {
                continue;
            };
            if layer.locked {
                continue;
            }
            let b = layer.bounds();
            let center = match axis {
                DistributeAxis::Horizontal => b.x as f64 + b.width as f64 / 2.0,
                DistributeAxis::Vertical => b.y as f64 + b.height as f64 / 2.0,
            };
            items.push((i, center));
        }
        if items.len() < 3 {
            // 少于三层没有「中间」可分布，直接当成功（不动）。
            return Ok(());
        }
        items.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let first = items[0].1;
        let last = items[items.len() - 1].1;
        let step = (last - first) / (items.len() - 1) as f64;

        let mut ops = Vec::new();
        for (k, &(i, center)) in items.iter().enumerate() {
            if k == 0 || k == items.len() - 1 {
                continue;
            }
            let target = first + step * k as f64;
            let delta = (target - center).round() as i32;
            if delta == 0 {
                continue;
            }
            let cur = self.layers[i].offset();
            let (dx, dy) = match axis {
                DistributeAxis::Horizontal => (delta, 0),
                DistributeAxis::Vertical => (0, delta),
            };
            ops.push(EditOp::TranslateLayer {
                index: i,
                from: cur,
                to: (cur.0 + dx, cur.1 + dy),
            });
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.execute(EditOp::Batch(ops))
    }
}

/// 复制图层时的默认偏移（像素）。
pub const DUPLICATE_SHIFT: i32 = 8;

/// 把图层不透明度烘焙进位图（只改 alpha，符合非预乘的合成语义）。
fn bake_opacity(image: &mut RgbaImage, opacity: f32) {
    if opacity >= 0.999 {
        return;
    }
    for px in image.pixels_mut() {
        let a = px.0[3] as f32 * opacity;
        px.0[3] = a.round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]))
    }

    fn dot(x: i32, y: i32, w: u32, h: u32, color: [u8; 4]) -> Layer {
        let mut l = Layer::new("dot", RgbaImage::from_pixel(w, h, image::Rgba(color)));
        l.set_offset((x, y));
        l
    }

    fn rect_mark() -> Mark {
        let mut m = Mark::new(MarkKind::Rect);
        m.rect = (4.0, 4.0, 12.0, 12.0);
        m.stroke = 3;
        m.color = [255, 0, 0, 255];
        m.fill = true;
        m.fill_color = [0, 255, 0, 255];
        m
    }

    #[test]
    fn new_empty_layer_is_transparent_canvas_sized_and_undoable() {
        let mut doc = Document::from_image(blank(20, 10));
        let i = doc.new_empty_layer().unwrap();
        assert_eq!(i, 1);
        let layer = &doc.layers[i];
        assert_eq!(layer.img().dimensions(), (20, 10));
        assert_eq!(layer.offset(), (0, 0));
        assert!(layer.image().unwrap().pixels().all(|p| p.0[3] == 0));
        // 全透明层不影响合成结果。
        assert_eq!(doc.flatten(), blank(20, 10));
        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 1);
    }

    #[test]
    fn duplicate_layer_offsets_copy_and_undoes() {
        let mut doc = Document::from_image(blank(32, 32));
        doc.add_layer(dot(2, 3, 4, 4, [10, 20, 30, 255])).unwrap();
        let dup = doc.duplicate_layer(1).unwrap();
        assert_eq!(dup, 2);
        assert_eq!(doc.layers[dup].offset(), (10, 11));
        assert_eq!(doc.layers[dup].img().dimensions(), (4, 4));
        assert_ne!(doc.layers[dup].id, doc.layers[1].id);
        assert!(doc.layers[dup].name.contains("副本"));
        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 2);
    }

    #[test]
    fn merge_down_bakes_pixels_and_undoes() {
        let mut doc = Document::from_image(blank(32, 32));
        doc.add_layer(dot(4, 4, 10, 10, [0, 0, 255, 255])).unwrap();
        doc.add_layer(dot(8, 8, 10, 10, [255, 0, 0, 255])).unwrap();
        let before = doc.flatten();

        let at = doc.merge_down(2).unwrap();
        assert_eq!(at, 1);
        assert_eq!(doc.layers.len(), 2);
        assert_eq!(doc.layers[1].opacity, 1.0);
        // 合并前后视觉完全一致。
        assert_eq!(doc.flatten(), before);
        assert_eq!(doc.flatten().get_pixel(9, 9).0, [255, 0, 0, 255]);

        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 3);
        assert_eq!(doc.flatten(), before);
    }

    #[test]
    fn merge_down_into_a_mark_layer_works() {
        // 下面那层是矢量标记时也要能合并：结果层变位图，视觉不变。
        let mut doc = Document::from_image(blank(32, 32));
        doc.add_mark(rect_mark()).unwrap();
        doc.add_layer(dot(6, 6, 8, 8, [255, 0, 0, 255])).unwrap();
        let before = doc.flatten();

        let at = doc.merge_down(2).unwrap();
        assert_eq!(at, 1);
        assert_eq!(doc.layers.len(), 2);
        assert!(doc.layers[1].is_raster());
        assert_eq!(doc.flatten(), before);

        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 3);
        assert!(doc.layers[1].is_mark());
        assert_eq!(doc.flatten(), before);
    }

    #[test]
    fn merge_layers_merges_only_the_selected_ones() {
        let mut doc = Document::from_image(blank(32, 32));
        doc.add_layer(dot(2, 2, 8, 8, [0, 0, 255, 255])).unwrap();
        doc.add_layer(dot(6, 6, 8, 8, [255, 0, 0, 255])).unwrap();
        doc.add_layer(dot(10, 10, 8, 8, [0, 255, 0, 255])).unwrap();
        let before = doc.flatten();

        // 只合并下面两层，最上面那层保持独立。
        let at = doc.merge_layers(&[1, 2]).unwrap();
        assert_eq!(at, 1);
        assert_eq!(doc.layers.len(), 3);
        assert!(doc.layers[1].is_raster());
        assert_eq!(doc.layers[1].opacity, 1.0);
        // 合并前后视觉逐像素一致。
        assert_eq!(doc.flatten(), before);

        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 4);
        assert_eq!(doc.flatten(), before);
    }

    #[test]
    fn merge_layers_takes_the_union_of_bounds() {
        let mut doc = Document::from_image(blank(40, 40));
        doc.add_layer(dot(0, 0, 4, 4, [10, 20, 30, 255])).unwrap();
        doc.add_layer(dot(30, 30, 4, 4, [40, 50, 60, 255])).unwrap();
        let at = doc.merge_layers(&[1, 2]).unwrap();
        let layer = &doc.layers[at];
        assert_eq!(layer.offset(), (0, 0));
        assert_eq!(layer.img().dimensions(), (34, 34));
        // 右上角那块像素没被裁掉。
        assert_eq!(doc.flatten().get_pixel(31, 31).0, [40, 50, 60, 255]);
    }

    #[test]
    fn merge_layers_needs_two_non_base_layers() {
        let mut doc = Document::from_image(blank(16, 16));
        doc.add_layer(dot(0, 0, 4, 4, [1, 2, 3, 255])).unwrap();
        // 只有一层 / 只有「背景 + 一层」都不够。
        assert!(doc.merge_layers(&[1]).is_err());
        assert!(doc.merge_layers(&[0, 1]).is_err());
        assert!(doc.merge_layers(&[]).is_err());
        assert_eq!(doc.layers.len(), 2);
    }

    #[test]
    fn merge_layers_tolerates_empty_layers() {
        // 两层都是空的（新建的空图层就是 0×0）：并集退化，但「合并不了」
        // 是用户最没法自救的反馈，必须照样减层、照样可撤销。
        let mut doc = Document::from_image(blank(16, 16));
        doc.add_layer(Layer::new("empty-a", image::RgbaImage::new(0, 0)))
            .unwrap();
        doc.add_layer(Layer::new("empty-b", image::RgbaImage::new(0, 0)))
            .unwrap();
        assert_eq!(doc.layers.len(), 3);
        let at = doc.merge_layers(&[1, 2]).unwrap();
        assert_eq!(at, 1);
        assert_eq!(doc.layers.len(), 2);
        assert!(doc.layers[1].is_raster());
        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 3);
    }

    #[test]
    fn merge_visible_keeps_hidden_layers() {
        let mut doc = Document::from_image(blank(32, 32));
        doc.add_layer(dot(0, 0, 8, 8, [0, 0, 255, 255])).unwrap();
        doc.add_layer(dot(4, 4, 8, 8, [255, 0, 0, 255])).unwrap();
        let hidden = doc.add_layer(dot(20, 20, 4, 4, [0, 255, 0, 255])).unwrap();
        doc.set_layer_visible(hidden, false).unwrap();
        let before = doc.flatten();

        let at = doc.merge_visible().unwrap();
        assert_eq!(at, 1);
        // 背景 + 合并结果 + 那个隐藏层。
        assert_eq!(doc.layers.len(), 3);
        assert_eq!(doc.layers[1].name, "合并图层");
        assert!(!doc.layers[2].visible);
        assert_eq!(doc.flatten(), before);

        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 4);
        assert_eq!(doc.flatten(), before);
    }

    #[test]
    fn flatten_stack_leaves_one_layer_and_undoes() {
        let mut doc = Document::from_image(blank(24, 24));
        doc.add_layer(dot(2, 2, 6, 6, [0, 0, 255, 255])).unwrap();
        doc.add_mark(rect_mark()).unwrap();
        let before = doc.flatten();

        doc.flatten_stack().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert!(doc.layers[0].background);
        assert_eq!(doc.flatten(), before);

        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 3);
        assert_eq!(doc.flatten(), before);
    }

    #[test]
    fn rasterize_mark_preserves_pixels_and_z() {
        let mut doc = Document::from_image(blank(24, 24));
        doc.add_layer(dot(0, 0, 4, 4, [0, 0, 255, 255])).unwrap();
        doc.add_mark(rect_mark()).unwrap();
        let before = doc.flatten();

        let i = doc.rasterize_mark(2).unwrap();
        assert_eq!(i, 2);
        assert!(doc.layers[2].is_raster());
        assert_eq!(doc.layers[2].name, "矩形");
        // 位图只包住标记本身，不是整块画布。
        assert!(doc.layers[2].img().width() < 24);
        assert_eq!(doc.flatten(), before);

        doc.undo().unwrap();
        assert!(doc.layers[2].is_mark());
    }

    #[test]
    fn align_layers_hits_requested_edge() {
        let mut doc = Document::from_image(blank(40, 40));
        let a = doc.add_layer(dot(2, 2, 6, 6, [1, 1, 1, 255])).unwrap();
        let b = doc.add_layer(dot(10, 10, 8, 4, [2, 2, 2, 255])).unwrap();
        let c = doc.add_layer(dot(20, 20, 4, 8, [3, 3, 3, 255])).unwrap();
        let canvas = Rect::new(0, 0, 40, 40);

        doc.align_layers(&[a, b, c], AlignEdge::Left, canvas)
            .unwrap();
        assert_eq!(doc.layers[a].offset().0, 0);
        assert_eq!(doc.layers[b].offset().0, 0);
        assert_eq!(doc.layers[c].offset().0, 0);

        doc.align_layers(&[a, b, c], AlignEdge::Right, canvas)
            .unwrap();
        assert_eq!(doc.layers[a].offset().0, 40 - 6);
        assert_eq!(doc.layers[b].offset().0, 40 - 8);

        doc.align_layers(&[a, b, c], AlignEdge::VCenter, canvas)
            .unwrap();
        assert_eq!(doc.layers[a].offset().1, 20 - 3);
        assert_eq!(doc.layers[b].offset().1, 20 - 2);

        // 一条命令 = 一条历史（撤销回到 VCenter 之前）。
        doc.undo().unwrap();
        assert_eq!(doc.layers[a].offset().1, 2);
    }

    #[test]
    fn align_skips_locked_layers() {
        let mut doc = Document::from_image(blank(40, 40));
        let a = doc.add_layer(dot(5, 5, 4, 4, [1, 1, 1, 255])).unwrap();
        doc.set_layer_locked(a, true).unwrap();
        doc.align_layers(&[a], AlignEdge::Left, Rect::new(0, 0, 40, 40))
            .unwrap();
        assert_eq!(doc.layers[a].offset().0, 5);
    }

    #[test]
    fn distribute_layers_spreads_centers_evenly() {
        let mut doc = Document::from_image(blank(100, 20));
        let a = doc.add_layer(dot(0, 0, 10, 10, [1, 1, 1, 255])).unwrap();
        let b = doc.add_layer(dot(20, 0, 10, 10, [2, 2, 2, 255])).unwrap();
        let c = doc.add_layer(dot(80, 0, 10, 10, [3, 3, 3, 255])).unwrap();

        doc.distribute_layers(&[a, b, c], DistributeAxis::Horizontal)
            .unwrap();
        // 首尾中心 5 与 85，中间应该落在 45。
        assert_eq!(doc.layers[a].offset().0, 0);
        assert_eq!(doc.layers[b].offset().0, 40);
        assert_eq!(doc.layers[c].offset().0, 80);

        doc.undo().unwrap();
        assert_eq!(doc.layers[b].offset().0, 20);
    }

    #[test]
    fn batch_revert_is_reverse_ordered() {
        // 先删再插：撤销必须先把插入的那层拿掉，否则下面的 index 全错位。
        let mut doc = Document::from_image(blank(10, 10));
        doc.add_layer(dot(0, 0, 2, 2, [1, 1, 1, 255])).unwrap();
        let before = doc.layers.clone();
        let mut probe = dot(5, 5, 2, 2, [9, 9, 9, 255]);
        probe.name = "probe".into();
        doc.execute(EditOp::Batch(vec![
            EditOp::RemoveLayer {
                index: 1,
                layer: doc.layers[1].clone(),
            },
            EditOp::AddLayer {
                layer: probe,
                index: 1,
            },
        ]))
        .unwrap();
        assert_eq!(doc.layers[1].name, "probe");
        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), before.len());
        assert_eq!(doc.layers[1].name, before[1].name);
    }
}
