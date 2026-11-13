// crates/core/src/layer.rs
use super::*;
use image::GrayImage;
use std::sync::atomic::{AtomicU32, Ordering};

/// 稳定图层 id。
///
/// 索引（`Vec` 下标）会随插入 / 删除 / 排序变化，UI 的多选集合、缩略图缓存
/// 与日志都必须用 id；历史记录（`EditOp`）内部仍用 index —— undo/redo 按栈
/// 顺序 apply，栈内索引天然自洽。
pub type LayerId = u32;

static NEXT_LAYER_ID: AtomicU32 = AtomicU32::new(1);

fn next_layer_id() -> LayerId {
    NEXT_LAYER_ID.fetch_add(1, Ordering::Relaxed)
}

/// 稳定分组 id。**组不是图层**：它只存在于 `Document::groups` 这份平行
/// 元数据里，绝不进 `doc.layers`。合成、预览缓存、PSD 写出都只遍历
/// `layers`，所以加分组不需要碰任何一条渲染路径。
pub type GroupId = u32;

static NEXT_GROUP_ID: AtomicU32 = AtomicU32::new(1);

pub fn next_group_id() -> GroupId {
    NEXT_GROUP_ID.fetch_add(1, Ordering::Relaxed)
}

/// 图层分组元数据。成员关系不单独存表，而是由每个 `Layer::group` 反查，
/// 这样删层 / 撤销 / 替换整层时归属自动跟着走，不会出现成员表与图层栈
/// 不一致的两份真值。
#[derive(Debug, Clone, PartialEq)]
pub struct LayerGroup {
    pub id: GroupId,
    pub name: String,
    /// 面板折叠状态：纯视图状态，不进撤销历史。
    pub collapsed: bool,
    /// 组的眼睛状态。真正的显隐靠批量写成员的 `visible`（合成只认成员），
    /// 这个字段只是面板图标与「显示整组」的目标值。
    pub visible: bool,
    /// 隐藏整组之前记下各成员自己的显隐，重新显示时按 id 还原 ——
    /// 否则组里原本手动隐藏的层会被「显示整组」一起点亮。
    pub hidden_snapshot: Vec<(LayerId, bool)>,
    /// 父组。PSD 的组可以嵌套，摊平会把层级变成一堆平级标题行。
    /// `None` = 顶层组；成员关系仍由 `Layer::group` 反查，这里是**组与组**
    /// 之间的父子关系，两者合起来才是完整树。
    pub parent: Option<GroupId>,
}

impl LayerGroup {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: next_group_id(),
            name: name.into(),
            collapsed: false,
            visible: true,
            hidden_snapshot: Vec::new(),
            parent: None,
        }
    }

    /// PSD 导入专用：组名可能带「空组」等标记，折叠态来自 `lsct` 的
    /// open / closed，其他字段与 `new` 一致（id 仍由这里自增分配）。
    pub fn imported(name: impl Into<String>, collapsed: bool) -> Self {
        let mut group = Self::new(name);
        group.collapsed = collapsed;
        group
    }
}
/// 位图内容：像素 + 画布相对偏移 + 非破坏性变换参数。
#[derive(Debug, Clone, PartialEq)]
pub struct RasterLayer {
    pub image: RgbaImage,
    pub offset: (i32, i32),
    /// P0-1：`Some` 表示本层还保留着未插值的原图（`LayerTransform::source`）
    /// 与累计参数，`image` 是当前的显示缓存；`None` 表示已经烘焙（做过像素
    /// 编辑 / 旋转），`image` 是唯一真值。
    ///
    /// 约定（§2.4 等价性契约）：任何**写 `image` 的像素操作之前**必须调用
    /// `ensure_baked()`，否则后续变换会从过期的原始像素重新采样，把编辑
    /// 结果抹掉。变换入口自己负责维护 `Some`，不需要 ensure_baked。
    pub transform: Option<LayerTransform>,
}

/// 图层内容：位图 or 矢量标记。
///
/// 两者共享外层的「可见 / 锁定 / 不透明度 / 顺序 / 名字」，因此面板、批量
/// 操作、排序与撤销只需要一套逻辑。
#[derive(Debug, Clone, PartialEq)]
pub enum LayerContent {
    Raster(RasterLayer),
    /// 矢量标记（矩形 / 椭圆 / 箭头 / 画笔 / 文本 / 马赛克）。
    Mark(Mark),
}

/// 图层蒙版：灰度图（0 = 完全遮住，255 = 完全显示）+ 它的画布坐标偏移。
///
/// 蒙版有自己的矩形（通常是整幅画布），与图层位图的矩形无关 —— 用图层矩形
/// 去解释蒙版通道会把蒙版错位/剪切。合成时 `alpha *= mask/255`。
#[derive(Debug, Clone, PartialEq)]
pub struct LayerMask {
    pub image: GrayImage,
    pub offset: (i32, i32),
    /// 蒙版矩形**外**的取值（PS 只存 0 或 255）。
    ///
    /// 蒙版矩形比图层小是正常现象（PS 会把矩形缩到真正涂过的区域），矩形
    /// 外按这个默认色算。一律当成 0 会把用户从没遮掉的内容藏起来。
    pub default: u8,
}

impl LayerMask {
    pub fn new(image: GrayImage, offset: (i32, i32)) -> Self {
        Self {
            image,
            offset,
            default: 0,
        }
    }

    /// 带「矩形外默认色」的构造（PSD 导入用）。
    pub fn with_default(image: GrayImage, offset: (i32, i32), default: u8) -> Self {
        Self {
            image,
            offset,
            default,
        }
    }

    /// 某个画布坐标上的覆盖系数，0.0..=1.0；蒙版矩形外取 `default`。
    pub fn coverage(&self, x: i32, y: i32) -> f32 {
        let lx = x - self.offset.0;
        let ly = y - self.offset.1;
        if lx < 0 || ly < 0 || lx >= self.image.width() as i32 || ly >= self.image.height() as i32 {
            return self.default as f32 / 255.0;
        }
        self.image.get_pixel(lx as u32, ly as u32).0[0] as f32 / 255.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    /// 仅 `layers[0]` 可为 true：文档背景层。仍可隐藏 / 解锁 / 移动，
    /// 「删除」= 清空像素（见 `EditOp::ClearBackground`）。
    pub background: bool,
    /// 与下方已合成内容混合的方式（PSD 打开时带入，默认正常）。
    pub blend: BlendMode,
    /// 图层蒙版（PSD 的 `-2` 通道），`None` = 无蒙版。
    pub mask: Option<LayerMask>,
    /// 剪贴蒙版：本层只在**下方第一个非剪贴层**的不透明区域内可见。
    ///
    /// 一组连续的剪贴层共享同一个基底层（PSD 里「剪贴蒙版」的语义）。
    pub clipping: bool,
    /// 所属分组（`None` = 不属于任何组）。组只影响面板组织与批量显隐，
    /// 不改变 z 序、不参与合成。`duplicated()` 会继承它。
    pub group: Option<GroupId>,
    pub content: LayerContent,
}

/// 等比缩到 `max` 见方并居中，其余透明。
///
/// 公开给 UI 的 PSD stage-2 后台线程：图层位图还在 worker 手上时就把 40px
/// 缩略图算好，UI 线程首次展开 100 层只做缓存命中（C6，单次 ≤16ms）。
pub fn thumbnail_image(src: &RgbaImage, max: u32) -> RgbaImage {
    let mut out = RgbaImage::new(max, max);
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 {
        return out;
    }
    let scale = (max as f32 / w.max(h) as f32).min(1.0);
    let tw = ((w as f32 * scale).round() as u32).max(1).min(max);
    let th = ((h as f32 * scale).round() as u32).max(1).min(max);
    let small = image::imageops::thumbnail(src, tw, th);
    let ox = (max - tw) / 2;
    let oy = (max - th) / 2;
    for (x, y, px) in small.enumerate_pixels() {
        out.put_pixel(x + ox, y + oy, *px);
    }
    out
}

impl Layer {
    pub fn new(name: impl Into<String>, image: RgbaImage) -> Self {
        Self {
            id: next_layer_id(),
            name: name.into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            background: false,
            blend: BlendMode::Normal,
            mask: None,
            clipping: false,
            group: None,
            content: LayerContent::Raster(RasterLayer {
                image,
                offset: (0, 0),
                transform: None,
            }),
        }
    }

    /// 文档背景层：默认锁定，`background = true`。
    pub fn background(image: RgbaImage) -> Self {
        Self {
            locked: true,
            background: true,
            ..Self::new("背景", image)
        }
    }

    /// 复制一份内容相同、但 `id` 与名字全新的图层（不进文档，只构造）。
    pub fn duplicated(&self) -> Self {
        Self {
            id: next_layer_id(),
            name: format!("{} 副本", self.name),
            ..self.clone()
        }
    }

    /// 把图层摊平成「画布平面」：图像 + 它在画布里的左上角（不烘焙不透明度）。
    ///
    /// 位图直接取显示缓存（已含变换结果）；标记按自己的画布矩形画成位图。
    /// 合并图层、生成缩略图都走这里，省得两处各写一份「标记怎么变位图」。
    pub fn plane(&self) -> (RgbaImage, (i32, i32)) {
        match &self.content {
            LayerContent::Raster(r) => (r.image.clone(), r.offset),
            LayerContent::Mark(m) => {
                let rect = m.canvas_rect();
                // 描边有一半在 rect 外面，四周留出 描边宽/2 + 2 的余量。
                let pad = (m.stroke.max(1) / 2 + 2) as i32;
                let ox = rect.x - pad;
                let oy = rect.y - pad;
                let w = (rect.width as i32 + pad * 2).max(1) as u32;
                let h = (rect.height as i32 + pad * 2).max(1) as u32;
                let mut img = RgbaImage::new(w, h);
                m.translated(-ox as f32, -oy as f32).draw(&mut img);
                (img, (ox, oy))
            }
        }
    }

    /// 面板缩略图：内容等比缩到 `max` 见方内并居中，其余透明。
    ///
    /// 用 `imageops::thumbnail`（整数重采样）而不是 Lanczos：40px 的小图
    /// 看不出差别，但快一个数量级 —— 面板每次重建会连着要好几张。
    pub fn thumbnail(&self, max: u32) -> RgbaImage {
        let max = max.max(1);
        // 位图直接引用原图（不克隆整层 —— 大图层克隆一次就是几十 MB）；
        // 标记才临时画一张小的。
        match &self.content {
            LayerContent::Raster(r) => thumbnail_image(&r.image, max),
            LayerContent::Mark(_) => {
                let (img, _) = self.plane();
                thumbnail_image(&img, max)
            }
        }
    }

    /// 矢量标记图层。
    pub fn from_mark(mark: Mark) -> Self {
        Self {
            id: next_layer_id(),
            name: mark.display_name(),
            visible: true,
            locked: false,
            opacity: 1.0,
            background: false,
            blend: BlendMode::Normal,
            mask: None,
            clipping: false,
            group: None,
            content: LayerContent::Mark(mark),
        }
    }

    // ---- 内容判别 ----------------------------------------------------

    pub fn is_mark(&self) -> bool {
        matches!(self.content, LayerContent::Mark(_))
    }

    pub fn is_raster(&self) -> bool {
        matches!(self.content, LayerContent::Raster(_))
    }

    pub fn raster(&self) -> Option<&RasterLayer> {
        match &self.content {
            LayerContent::Raster(r) => Some(r),
            LayerContent::Mark(_) => None,
        }
    }

    pub fn raster_mut(&mut self) -> Option<&mut RasterLayer> {
        match &mut self.content {
            LayerContent::Raster(r) => Some(r),
            LayerContent::Mark(_) => None,
        }
    }

    pub fn mark(&self) -> Option<&Mark> {
        match &self.content {
            LayerContent::Mark(m) => Some(m),
            LayerContent::Raster(_) => None,
        }
    }

    pub fn mark_mut(&mut self) -> Option<&mut Mark> {
        match &mut self.content {
            LayerContent::Mark(m) => Some(m),
            LayerContent::Raster(_) => None,
        }
    }

    pub fn image(&self) -> Option<&RgbaImage> {
        self.raster().map(|r| &r.image)
    }

    pub fn image_mut(&mut self) -> Option<&mut RgbaImage> {
        self.raster_mut().map(|r| &mut r.image)
    }

    /// 位图访问的便利入口（历史代码大量 `layer.image.width()` 这类写法）。
    ///
    /// 标记图层没有位图，返回一张 1×1 透明图：调用方本应先用 `is_raster()`
    /// 判断，这里只是让「图层必有位图」的旧路径不至于 panic。**写像素请走
    /// [`Layer::image_mut`]**，不要用这个返回值。
    pub fn img(&self) -> &RgbaImage {
        static EMPTY: std::sync::OnceLock<RgbaImage> = std::sync::OnceLock::new();
        match self.image() {
            Some(img) => img,
            None => EMPTY.get_or_init(|| RgbaImage::new(1, 1)),
        }
    }

    /// 变换参数的只读入口；标记层恒 `None`。
    pub fn xform(&self) -> Option<&LayerTransform> {
        self.raster().and_then(|r| r.transform.as_ref())
    }

    pub fn xform_mut(&mut self) -> Option<&mut LayerTransform> {
        self.raster_mut().and_then(|r| r.transform.as_mut())
    }

    /// 图层原点（画布坐标）。标记取它 `rect` 的左上角（整数化）。
    pub fn offset(&self) -> (i32, i32) {
        match &self.content {
            LayerContent::Raster(r) => r.offset,
            LayerContent::Mark(m) => (m.rect.0.round() as i32, m.rect.1.round() as i32),
        }
    }

    /// 只移动图层原点、不改尺寸。标记按整数位移平移（`rect` 保持原样精度）。
    pub fn set_offset(&mut self, offset: (i32, i32)) {
        match &mut self.content {
            LayerContent::Raster(r) => r.offset = offset,
            LayerContent::Mark(m) => {
                let (ox, oy) = (m.rect.0.round() as i32, m.rect.1.round() as i32);
                let (dx, dy) = ((offset.0 - ox) as f32, (offset.1 - oy) as f32);
                if dx != 0.0 || dy != 0.0 {
                    *m = m.translated(dx, dy);
                }
            }
        }
    }

    fn expect_raster(&self) -> &RasterLayer {
        debug_assert!(self.is_raster(), "raster-only API called on a mark layer");
        self.raster().expect("raster layer")
    }

    fn expect_raster_mut(&mut self) -> &mut RasterLayer {
        debug_assert!(self.is_raster(), "raster-only API called on a mark layer");
        self.raster_mut().expect("raster layer")
    }

    // ---- 几何 --------------------------------------------------------

    /// 图层占位框（画布坐标）：位图用 offset + 尺寸，标记用 `mark.rect`。
    pub fn bounds(&self) -> Rect {
        match &self.content {
            LayerContent::Raster(r) => Rect::new(
                r.offset.0,
                r.offset.1,
                r.image.width(),
                r.image.height(),
            ),
            LayerContent::Mark(m) => m.canvas_rect(),
        }
    }

    /// 不透明内容的包围盒（**源图**像素坐标 `(x, y, w, h)`）。
    /// 有变换参数时取 `transform.source`（未插值原图），否则直接扫 `image`。
    /// 标记与全透明层返回 `None`。
    pub fn content_bbox(&self, threshold: u8) -> Option<(u32, u32, u32, u32)> {
        let r = self.raster()?;
        let img = match r.transform.as_ref() {
            Some(t) => &t.source,
            None => &r.image,
        };
        crate::alpha_fit::alpha_bbox(img, threshold)
    }

    /// 内容在**画布坐标**下的轴对齐外接框。
    ///
    /// 只有未旋转（且无斜切 / 透视）时才有意义 —— 旋转后内容框是斜的，
    /// 投影吸附会失真，这里返回 `None` 让调用方退回外框吸附。
    pub fn content_bounds(&self, threshold: u8) -> Option<RectF> {
        let r = self.raster()?;
        let (cx, cy, cw, ch) = self.content_bbox(threshold)?;
        let (cx, cy, cw, ch) = (cx as f64, cy as f64, cw as f64, ch as f64);
        match r.transform.as_ref() {
            Some(t) => {
                if !t.is_affine() || t.rotation.abs() > 1e-6 || t.skew.0 != 0.0 || t.skew.1 != 0.0 {
                    return None;
                }
                let m = t.matrix();
                let pts = [
                    m.apply((cx, cy)),
                    m.apply((cx + cw, cy)),
                    m.apply((cx + cw, cy + ch)),
                    m.apply((cx, cy + ch)),
                ];
                Some(RectF::from_points(&pts))
            }
            None => Some(RectF::new(
                r.offset.0 as f64 + cx,
                r.offset.1 as f64 + cy,
                cw,
                ch,
            )),
        }
    }

    // ---- 变换状态 ----------------------------------------------------

    /// 取得（或就地建立）本层的变换状态。已烘焙层用当前 `image` 作为新原图，
    /// identity 参数对齐 `offset`，因此第一次变换前后的几何完全一致。
    pub fn transform_state(&mut self) -> LayerTransform {
        let r = self.expect_raster_mut();
        if let Some(t) = &r.transform {
            return t.clone();
        }
        let t = LayerTransform::from_offset(r.image.clone(), r.offset.0, r.offset.1);
        r.transform = Some(t.clone());
        t
    }

    /// 记录新的变换状态。显示位图由变换入口用既有插值算法生成后通过
    /// `set_transform_pixels` 写入；这里只负责同步参数与状态。
    pub fn set_transform_state(&mut self, t: LayerTransform) {
        self.expect_raster_mut().transform = Some(t);
    }

    /// 变换入口生成好显示位图后调用：同时更新 `image` / `offset` / 变换参数，
    /// 保证三者永远是一套自洽的状态。
    pub fn set_transform_pixels(
        &mut self,
        image: RgbaImage,
        offset: (i32, i32),
        t: LayerTransform,
    ) {
        let r = self.expect_raster_mut();
        r.image = image;
        r.offset = offset;
        r.transform = Some(t);
    }

    /// 自动烘焙（D11）：像素类操作写 `image` 之前调用。`image` 本身已经是
    /// HQ 变换结果，所以只需丢弃原始像素与参数 —— 一次重采样已经发生过了，
    /// 不需要再采样一次（§2.4 陷阱 2：烘焙成 identity 后再插值会清掉透明
    /// 像素的 RGB 并让整层发糊）。
    pub fn ensure_baked(&mut self) {
        if let Some(r) = self.raster_mut() {
            r.transform = None;
        }
    }
}
