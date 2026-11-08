// crates/core/src/layer.rs
use super::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub image: RgbaImage,
    pub offset: (i32, i32),
    pub opacity: f32,
    pub visible: bool,
    /// UI-only guard: locked layers reject move / scale / delete in the
    /// bridge. Plain flag, not part of undo history.
    pub locked: bool,
    /// P0-1: 非破坏性变换状态。`Some` 表示本层还保留着未插值的原图
    /// (`LayerTransform::source`) 与累计参数，`image` 是它当前的显示缓存；
    /// `None` 表示已经烘焙（做过像素编辑 / 旋转），`image` 是唯一真值。
    ///
    /// 约定（§2.4 等价性契约）：任何**写 `image` 的像素操作之前**必须调用
    /// `ensure_baked()`，否则后续变换会从过期的原始像素重新采样，把编辑
    /// 结果抹掉。变换入口自己负责维护 `Some`，不需要 ensure_baked。
    pub transform: Option<LayerTransform>,
}

impl Layer {
    pub fn new(name: impl Into<String>, image: RgbaImage) -> Self {
        Self {
            name: name.into(),
            image,
            offset: (0, 0),
            opacity: 1.0,
            visible: true,
            locked: false,
            transform: None,
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(
            self.offset.0,
            self.offset.1,
            self.image.width(),
            self.image.height(),
        )
    }

    /// 不透明内容的包围盒（**源图**像素坐标 `(x, y, w, h)`）。
    ///
    /// 有变换参数时取 `transform.source`（未插值原图），否则直接扫 `image`。
    /// 全透明层返回 `None`。
    pub fn content_bbox(&self, threshold: u8) -> Option<(u32, u32, u32, u32)> {
        let img = match self.transform.as_ref() {
            Some(t) => &t.source,
            None => &self.image,
        };
        crate::alpha_fit::alpha_bbox(img, threshold)
    }

    /// 内容在**画布坐标**下的轴对齐外接框。
    ///
    /// 只有未旋转（且无斜切 / 透视）时才有意义 —— 旋转后内容框是斜的，
    /// 投影吸附会失真，这里返回 `None` 让调用方退回外框吸附。
    pub fn content_bounds(&self, threshold: u8) -> Option<RectF> {
        let (cx, cy, cw, ch) = self.content_bbox(threshold)?;
        let (cx, cy, cw, ch) = (cx as f64, cy as f64, cw as f64, ch as f64);
        match self.transform.as_ref() {
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
                self.offset.0 as f64 + cx,
                self.offset.1 as f64 + cy,
                cw,
                ch,
            )),
        }
    }

    /// 取得（或就地建立）本层的变换状态。已烘焙层用当前 `image` 作为新原图，
    /// identity 参数对齐 `offset`，因此第一次变换前后的几何完全一致。
    pub fn transform_state(&mut self) -> LayerTransform {
        if let Some(t) = &self.transform {
            return t.clone();
        }
        let t = LayerTransform::from_offset(self.image.clone(), self.offset.0, self.offset.1);
        self.transform = Some(t.clone());
        t
    }

    /// 记录新的变换状态。P0-1 的显示位图由变换入口用既有插值算法生成后
    /// 通过 `set_transform_pixels` 写入；这里只负责同步参数与状态。
    pub fn set_transform_state(&mut self, t: LayerTransform) {
        self.transform = Some(t);
    }

    /// 变换入口生成好显示位图后调用：同时更新 `image` / `offset` / 变换参数，
    /// 保证三者永远是一套自洽的状态。
    pub fn set_transform_pixels(
        &mut self,
        image: RgbaImage,
        offset: (i32, i32),
        t: LayerTransform,
    ) {
        self.image = image;
        self.offset = offset;
        self.transform = Some(t);
    }

    /// 自动烘焙（D11）：像素类操作写 `image` 之前调用。`image` 本身已经是
    /// HQ 变换结果，所以只需丢弃原始像素与参数 —— 一次重采样已经发生过了，
    /// 不需要再采样一次（§2.4 陷阱 2：烘焙成 identity 后再插值会清掉透明
    /// 像素的 RGB 并让整层发糊）。
    pub fn ensure_baked(&mut self) {
        self.transform = None;
    }
}
