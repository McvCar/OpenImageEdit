//! 独立变换状态层的纯数学部分（P0-0）。
//!
//! 设计契约见 `docs/变换系统重构-修改路线.md` §2.1 / §3.1：
//!
//! * **params 是唯一事实源**（D9）：`LayerTransform` 的字段是唯一被历史/序列化
//!   保存的东西，`matrix()` / `inverse()` 每次由参数推导，不存裸矩阵。
//! * 变换期间 `source` 只读、从不写回，因此「缩小再放大」不会累积插值，
//!   「旋转→拉伸→再旋转」不会把外接矩形二次烘焙进位图。
//! * 采样与命中测试统一走 `inverse()`，矩阵约定：连续坐标（像素中心为
//!   `(x + 0.5, y + 0.5)`）下 `M = T(center) · R · S · F · K · T(-w/2, -h/2)`。
//!   源图边角 `(0,0)…(w,h)` 经 `M` 映射即画布上的几何边界。
//! * `perspective` 与 `TRANSFORM_SERIAL_VERSION` 现在只预留（R8）：P0 恒为
//!   `None`，`matrix()` 仍是仿射；P1-A 把返回类型升级成 `Affine | Homography`
//!   时不用改数据模型或迁移历史。
//!
//! 本阶段**不接线**：`Document` / `Layer` / 渲染 / UI 都不引用本模块，
//! 只有单元测试、`--probe-transform` 与回归脚本会用到。

use std::sync::Arc;

use image::RgbaImage;

use crate::geometry::Rect;

/// 历史/序列化格式版本。P1-A 引入透视矩阵时递增，旧记录按版本解码。
pub const TRANSFORM_SERIAL_VERSION: u32 = 1;
const PARAMS_MAGIC: &[u8; 4] = b"IET1";

/// 2×3 仿射变换，行主序：
/// `x' = a*x + c*y + e`，`y' = b*x + d*y + f`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine2 {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Affine2 {
    pub const fn identity() -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 }
    }

    pub const fn translation(tx: f64, ty: f64) -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: tx, f: ty }
    }

    pub fn rotation(degrees: f64) -> Self {
        let r = degrees.to_radians();
        let (s, c) = (r.sin(), r.cos());
        Self { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    pub const fn scaling(sx: f64, sy: f64) -> Self {
        Self { a: sx, b: 0.0, c: 0.0, d: sy, e: 0.0, f: 0.0 }
    }

    /// 斜切：`x' = x + kx*y`，`y' = y + ky*x`。
    pub const fn shear(kx: f64, ky: f64) -> Self {
        Self { a: 1.0, b: ky, c: kx, d: 1.0, e: 0.0, f: 0.0 }
    }

    /// 组合 `self ∘ rhs`（先施加 `rhs`）。
    pub fn mul(self, rhs: Affine2) -> Affine2 {
        Affine2 {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            e: self.a * rhs.e + self.c * rhs.f + self.e,
            f: self.b * rhs.e + self.d * rhs.f + self.f,
        }
    }

    pub fn apply(self, p: (f64, f64)) -> (f64, f64) {
        (self.a * p.0 + self.c * p.1 + self.e, self.b * p.0 + self.d * p.1 + self.f)
    }

    /// 退化（scale=0、斜切奇异）时返回 `None`，调用方不得继续取样。
    pub fn inverse(self) -> Option<Affine2> {
        let det = self.a * self.d - self.b * self.c;
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        Some(Affine2 {
            a: self.d / det,
            b: -self.b / det,
            c: -self.c / det,
            d: self.a / det,
            e: (self.c * self.f - self.d * self.e) / det,
            f: (self.b * self.e - self.a * self.f) / det,
        })
    }

    pub const fn as_array(self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }
}

/// 浮点矩形：允许负坐标、允许越出画布（D1/D3）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl RectF {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    pub fn right(self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(self) -> f64 {
        self.y + self.height
    }

    pub fn center(self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn from_points(points: &[(f64, f64); 4]) -> Self {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for &(x, y) in points {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        Self::new(x0, y0, x1 - x0, y1 - y0)
    }

    pub fn pivot(self, rp: ReferencePoint) -> (f64, f64) {
        let (fx, fy) = rp.factors();
        (self.x + self.width * fx, self.y + self.height * fy)
    }

    /// 后续阶段求 `bounds ∩ canvas` 用：向外取整到整数像素格。
    pub fn round_out(self) -> Rect {
        let x = self.x.floor() as i32;
        let y = self.y.floor() as i32;
        let r = self.right().ceil() as i32;
        let b = self.bottom().ceil() as i32;
        Rect::new(x, y, (r - x).max(0) as u32, (b - y).max(0) as u32)
    }
}

/// 3×3 参考点，数值面板改尺寸时决定哪个角/边不动。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReferencePoint {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    #[default]
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl ReferencePoint {
    pub const fn factors(self) -> (f64, f64) {
        use ReferencePoint::*;
        match self {
            TopLeft => (0.0, 0.0),
            TopCenter => (0.5, 0.0),
            TopRight => (1.0, 0.0),
            CenterLeft => (0.0, 0.5),
            Center => (0.5, 0.5),
            CenterRight => (1.0, 0.5),
            BottomLeft => (0.0, 1.0),
            BottomCenter => (0.5, 1.0),
            BottomRight => (1.0, 1.0),
        }
    }
}

/// 数值面板字段（D10：X/Y/W/H 都是变换后外接框语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    X,
    Y,
    Width,
    Height,
    Rotation,
    ScaleX,
    ScaleY,
}

/// 写 W/H 时的反解策略（R10：90° 附近单轴解会病态，自动退回等比）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoundsSolve {
    /// 两个 scale 同乘 `target/current`，任意角度都稳定，保持长宽比。
    #[default]
    Uniform,
    /// 只改对应那一轴的 scale，另一轴保持不动。
    SingleAxis,
}

/// D10：数值面板读出的值。`x/y` = 外接框左上角，`width/height` = 外接框尺寸。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelValues {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
}

/// 只含数值的快照，供 `EditOp::SetLayerTransform` 存一条历史 / 序列化。
/// 不含位图：`Arc<RgbaImage>` 让快照浅拷贝（§2.3）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformParams {
    pub version: u32,
    pub center: (f64, f64),
    pub scale: (f64, f64),
    pub rotation: f64,
    pub flip: (bool, bool),
    pub skew: (f64, f64),
    /// P1-A 透视：3×3 单应去掉 `[8]=1` 后的 8 个系数。P0 恒为 `None`。
    pub perspective: Option<[f64; 8]>,
}

impl TransformParams {
    pub fn to_bytes(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 4 + 9 * 8 + 2 + 1 + 8 * 8);
        out.extend_from_slice(PARAMS_MAGIC);
        out.extend_from_slice(&self.version.to_le_bytes());
        for v in [
            self.center.0, self.center.1,
            self.scale.0, self.scale.1,
            self.rotation,
            self.skew.0, self.skew.1,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(u8::from(self.flip.0));
        out.push(u8::from(self.flip.1));
        match self.perspective {
            None => out.push(0),
            Some(h) => {
                out.push(1);
                for v in h {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        const HEAD: usize = 4 + 4 + 7 * 8 + 2 + 1;
        if bytes.len() < HEAD || &bytes[..4] != PARAMS_MAGIC {
            return None;
        }
        let version = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
        if version != TRANSFORM_SERIAL_VERSION {
            return None;
        }
        let mut p = 8;
        let take_f64 = |at: &mut usize| -> Option<f64> {
            let v = f64::from_le_bytes(bytes[*at..*at + 8].try_into().ok()?);
            *at += 8;
            Some(v)
        };
        let (cx, cy) = (take_f64(&mut p)?, take_f64(&mut p)?);
        let (sx, sy) = (take_f64(&mut p)?, take_f64(&mut p)?);
        let rotation = take_f64(&mut p)?;
        let (kx, ky) = (take_f64(&mut p)?, take_f64(&mut p)?);
        let flip = (bytes[p] != 0, bytes[p + 1] != 0);
        let has_persp = bytes[p + 2] != 0;
        p += 3;
        let perspective = if has_persp {
            let mut h = [0.0f64; 8];
            for slot in &mut h {
                *slot = take_f64(&mut p)?;
            }
            Some(h)
        } else {
            None
        };
        let params = Self { version, center: (cx, cy), scale: (sx, sy), rotation, flip, skew: (kx, ky), perspective };
        if !params.is_finite() {
            return None;
        }
        Some(params)
    }

    fn is_finite(self) -> bool {
        let mut ok = self.center.0.is_finite()
            && self.center.1.is_finite()
            && self.scale.0.is_finite()
            && self.scale.1.is_finite()
            && self.rotation.is_finite()
            && self.skew.0.is_finite()
            && self.skew.1.is_finite();
        if let Some(h) = self.perspective {
            ok &= h.iter().all(|v| v.is_finite());
        }
        ok
    }
}

/// 图层变换状态。P0-0 独立存在，P0-1 起挂进 `Layer`。
#[derive(Debug, Clone)]
pub struct LayerTransform {
    /// 未插值原图；变换期间只读。快照克隆只增引用计数。
    pub source: Arc<RgbaImage>,
    /// 原图中心在画布坐标系的位置。
    pub center: (f64, f64),
    /// 相对原图尺寸的倍率；翻转走 `flip`，这里保持正数。
    pub scale: (f64, f64),
    /// 累计角度（度，不归一，避免反复归一引入漂移）。
    pub rotation: f64,
    pub flip: (bool, bool),
    /// P0 恒为 0；P1-A 的 Ctrl 斜切启用。
    pub skew: (f64, f64),
    /// P0 恒为 None；P1-A 的 Ctrl 透视启用（R8 预留）。
    pub perspective: Option<[f64; 8]>,
}

impl LayerTransform {
    pub fn new(source: RgbaImage) -> Self {
        Self::from_arc(Arc::new(source))
    }

    pub fn from_arc(source: Arc<RgbaImage>) -> Self {
        let (w, h) = (source.width() as f64, source.height() as f64);
        Self {
            center: (w / 2.0, h / 2.0),
            source,
            scale: (1.0, 1.0),
            rotation: 0.0,
            flip: (false, false),
            skew: (0.0, 0.0),
            perspective: None,
        }
    }

    /// 与现有 `Layer { image, offset }` 对齐的构造：identity 时源图落在 `(x, y)`。
    pub fn from_offset(source: RgbaImage, x: i32, y: i32) -> Self {
        let mut t = Self::new(source);
        let (w, h) = t.source_size();
        t.center = (x as f64 + w / 2.0, y as f64 + h / 2.0);
        t
    }

    pub fn source_size(self: &Self) -> (f64, f64) {
        (self.source.width() as f64, self.source.height() as f64)
    }

    /// 现有 `Layer::offset` 的等价量（可为负 / 非整数，仅 identity 快路径有意义）。
    pub fn offset(&self) -> (f64, f64) {
        let (w, h) = self.source_size();
        (self.center.0 - w / 2.0, self.center.1 - h / 2.0)
    }

    /// P1 前永远是仿射。P1-A 会把 `matrix/inverse` 的返回类型升级成枚举。
    pub fn is_affine(&self) -> bool {
        self.perspective.is_none()
    }

    /// `source px → canvas px`（连续坐标，像素中心 0.5 约定）。
    pub fn matrix(&self) -> Affine2 {
        let (w, h) = self.source_size();
        let (fx, fy) = (if self.flip.0 { -1.0 } else { 1.0 }, if self.flip.1 { -1.0 } else { 1.0 });
        let mut m = Affine2::translation(-w / 2.0, -h / 2.0);
        m = Affine2::shear(self.skew.0, self.skew.1).mul(m);
        m = Affine2::scaling(self.scale.0 * fx, self.scale.1 * fy).mul(m);
        m = Affine2::rotation(self.rotation).mul(m);
        Affine2::translation(self.center.0, self.center.1).mul(m)
    }

    /// `canvas px → source px`。退化时 `None`（不可取样、不可命中）。
    pub fn inverse(&self) -> Option<Affine2> {
        if !self.is_affine() {
            return None;
        }
        self.matrix().inverse()
    }

    pub fn map_point(&self, p: (f64, f64)) -> (f64, f64) {
        self.matrix().apply(p)
    }

    /// 输出画布像素 `(x, y)` 的**中心**反变换到源图连续坐标，交给取样器。
    pub fn canvas_pixel_to_source(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        self.inverse().map(|inv| inv.apply((x + 0.5, y + 0.5)))
    }

    /// 旋转后的真实四角：左上、右上、右下、左下（供 8 点 / 旋柄 / 参考线）。
    pub fn quad(&self) -> [(f64, f64); 4] {
        let (w, h) = self.source_size();
        let m = self.matrix();
        [
            m.apply((0.0, 0.0)),
            m.apply((w, 0.0)),
            m.apply((w, h)),
            m.apply((0.0, h)),
        ]
    }

    /// 轴对齐外接框，允许负坐标 / 超出画布（D1）。
    pub fn bounds(&self) -> RectF {
        RectF::from_points(&self.quad())
    }

    /// 命中测试：逆变换 + 原图矩形 + alpha 阈值（比现在的矩形 `topLayerAt` 准）。
    pub fn hit(&self, p: (f64, f64)) -> bool {
        self.hit_alpha(p, 8)
    }

    pub fn hit_alpha(&self, p: (f64, f64), threshold: u8) -> bool {
        let (u, v) = match self.inverse().map(|inv| inv.apply(p)) {
            Some(q) => q,
            None => return false,
        };
        let (w, h) = (self.source.width(), self.source.height());
        if !(u >= 0.0 && v >= 0.0 && u < w as f64 && v < h as f64) {
            return false;
        }
        let px = self.source.get_pixel((u as u32).min(w - 1), (v as u32).min(h - 1));
        px[3] >= threshold
    }

    /// D10：外接框左上角 + 外接框尺寸 + 累计角度。
    pub fn panel_values(&self) -> PanelValues {
        let b = self.bounds();
        PanelValues { x: b.x, y: b.y, width: b.width, height: b.height, rotation: self.rotation }
    }

    /// 数值面板回写（等比解，任意角度稳定）。
    pub fn set_panel_value(&mut self, field: Field, v: f64, pivot: ReferencePoint) -> bool {
        self.set_panel_value_with_solve(field, v, pivot, BoundsSolve::Uniform)
    }

    pub fn set_panel_value_with_solve(
        &mut self,
        field: Field,
        v: f64,
        pivot: ReferencePoint,
        solve: BoundsSolve,
    ) -> bool {
        if !v.is_finite() {
            return false;
        }
        let old = self.bounds();
        let old_pivot = old.pivot(pivot);
        let ok = match field {
            Field::X => {
                self.center.0 += v - old.x;
                true
            }
            Field::Y => {
                self.center.1 += v - old.y;
                true
            }
            Field::Rotation => {
                self.rotation = v;
                true
            }
            Field::ScaleX => set_positive(&mut self.scale.0, v),
            Field::ScaleY => set_positive(&mut self.scale.1, v),
            Field::Width | Field::Height => self.solve_bounds(field, v, solve),
        };
        if !ok {
            return false;
        }
        // 改尺寸 / 角度后让参考点保持不动（X/Y 是纯平移，不需要）。
        if !matches!(field, Field::X | Field::Y) {
            let new_pivot = self.bounds().pivot(pivot);
            self.center.0 += old_pivot.0 - new_pivot.0;
            self.center.1 += old_pivot.1 - new_pivot.1;
        }
        true
    }

    /// 按「当前 rotation 不变」反解 scale（R10）。
    ///
    /// 任意仿射都有 `bounds.width = |m.a|·w + |m.c|·h`、
    /// `bounds.height = |m.b|·w + |m.d|·h`。改 sx 时只有矩阵**第一列**
    /// 按比例缩放，改 sy 时只有第二列缩放 —— 于是可以直接从矩阵列反解，
    /// 不需要 `cos/sin` 分解，也不会在 90° 附近除零（那里自动退回等比解）。
    fn solve_bounds(&mut self, field: Field, v: f64, solve: BoundsSolve) -> bool {
        if !self.is_affine() || v <= 0.0 {
            return false;
        }
        let (w, h) = self.source_size();
        if w <= 0.0 || h <= 0.0 {
            return false;
        }
        let b = self.bounds();
        let m = self.matrix();
        if solve == BoundsSolve::Uniform {
            let cur = if field == Field::Width { b.width } else { b.height };
            if cur <= f64::EPSILON {
                return false;
            }
            let r = v / cur;
            self.scale.0 *= r;
            self.scale.1 *= r;
            return true;
        }
        // 单轴解要求没有斜切：有斜切时两列都被 sx 影响，列不再独立。
        if self.skew.0 != 0.0 || self.skew.1 != 0.0 {
            return false;
        }
        let (fixed, driven) = if field == Field::Width {
            (m.c.abs() * h, m.a.abs() * w)
        } else {
            (m.b.abs() * w, m.d.abs() * h)
        };
        // 90° 附近 driven ≈ 0：该轴几乎不响应目标尺寸，退回等比避免抖动/爆值。
        if driven <= 1e-6 * (fixed + driven + 1.0) {
            let cur = if field == Field::Width { b.width } else { b.height };
            if cur <= f64::EPSILON {
                return false;
            }
            let r = v / cur;
            self.scale.0 *= r;
            self.scale.1 *= r;
            return true;
        }
        let r = (v - fixed) / driven;
        if !r.is_finite() || r <= 0.0 {
            return false;
        }
        if field == Field::Width {
            self.scale.0 *= r;
        } else {
            self.scale.1 *= r;
        }
        true
    }

    /// 烘焙 / 1:1 快路径判据（§2.4 陷阱 2）：identity 层必须直接索引拷贝，
    /// 不能再走一遍插值，否则整幅发糊。
    pub fn is_identity_fast_path(&self) -> bool {
        let rem = self.rotation.rem_euclid(360.0);
        let angle_ok = rem < 1e-9 || rem > 360.0 - 1e-9;
        let (ox, oy) = self.offset();
        angle_ok
            && self.is_affine()
            && !self.flip.0
            && !self.flip.1
            && self.skew == (0.0, 0.0)
            && (self.scale.0 - 1.0).abs() < 1e-9
            && (self.scale.1 - 1.0).abs() < 1e-9
            && (ox - ox.round()).abs() < 1e-9
            && (oy - oy.round()).abs() < 1e-9
    }

    pub fn params(&self) -> TransformParams {
        TransformParams {
            version: TRANSFORM_SERIAL_VERSION,
            center: self.center,
            scale: self.scale,
            rotation: self.rotation,
            flip: self.flip,
            skew: self.skew,
            perspective: self.perspective,
        }
    }

    pub fn from_params(params: TransformParams, source: Arc<RgbaImage>) -> Option<Self> {
        if params.version != TRANSFORM_SERIAL_VERSION || !params.is_finite() {
            return None;
        }
        Some(Self {
            source,
            center: params.center,
            scale: params.scale,
            rotation: params.rotation,
            flip: params.flip,
            skew: params.skew,
            perspective: params.perspective,
        })
    }

    /// 按指定画布矩形逆变换取样（core 烘焙 / render 预览共用同一采样器，
    /// 见 `crate::transform::sample_affine`）。
    ///
    /// §2.4 陷阱 2：identity（含烘焙后的图层）必须 **1:1 索引拷贝**，不能再
    /// 走一遍插值 —— 否则透明像素的 RGB 会被 normalize 清掉、整层边缘也会
    /// 平白发糊。
    pub fn render(&self, out: Rect, hq: bool) -> RgbaImage {
        if self.is_identity_fast_path() {
            return self.copy_identity(out);
        }
        match self.inverse() {
            Some(inv) => crate::transform::sample_affine(&self.source, &inv, out, hq),
            None => RgbaImage::new(out.width, out.height),
        }
    }

    /// 纯整数平移：把 `out ∩ source 矩形` 按行 memcpy，其余保持全透明。
    fn copy_identity(&self, out: Rect) -> RgbaImage {
        let mut dst = RgbaImage::new(out.width, out.height);
        if out.is_empty() {
            return dst;
        }
        let (ox, oy) = self.offset();
        let (sx, sy) = (out.x as i64 - ox as i64, out.y as i64 - oy as i64);
        let (sw, sh) = (self.source.width() as i64, self.source.height() as i64);
        let (x0, y0) = (sx.max(0) as u32, sy.max(0) as u32);
        let (x1, y1) = (
            (sx + out.width as i64).min(sw).max(0) as u32,
            (sy + out.height as i64).min(sh).max(0) as u32,
        );
        if x1 <= x0 || y1 <= y0 {
            return dst;
        }
        let src: &[u8] = self.source.as_raw();
        let dst_raw: &mut [u8] = &mut dst;
        let row_len = (x1 - x0) as usize * 4;
        let dst_stride = out.width as usize * 4;
        let src_stride = self.source.width() as usize * 4;
        for y in y0..y1 {
            let s = y as usize * src_stride + x0 as usize * 4;
            let d = (y as i64 - sy) as usize * dst_stride + (x0 as i64 - sx) as usize * 4;
            dst_raw[d..d + row_len].copy_from_slice(&src[s..s + row_len]);
        }
        dst
    }

    /// 渲染整个外接框，返回位图与它在画布中的左上角（可为负）。
    pub fn render_bounds(&self, hq: bool) -> (RgbaImage, (i32, i32)) {
        let r = self.bounds().round_out();
        let img = self.render(r, hq);
        (img, (r.x, r.y))
    }

    /// P0-2b2 落定渲染：参数化历史的 **唯一** 像素后端。
    ///
    /// `render_bounds(true)` 走 `sample_affine` HQ，而 P0-2a 之前 UI 的三条
    /// 落定路径分别用 `imageops::resize(Lanczos3)`（缩放）与
    /// `rotate_image_hq`（旋转）。统一历史时若直接换采样器，用户已经测过的
    /// 「文字缩回原尺寸不糊 / 旋转不累积模糊」会悄悄变画质。这里按参数
    /// 分派到既有 HQ 后端，保证「落定前后逐像素一致」，同时历史只剩
    /// `SetLayerTransform` 一条；斜切 / 透视 / 组合变换才走通用取样器。
    pub fn render_compatible(&self) -> (RgbaImage, (i32, i32)) {
        let affine = self.is_affine();
        let no_rotation = self.rotation.abs() < 1e-9;
        let no_skew = self.skew == (0.0, 0.0);
        let (sw, sh) = self.source_size();
        let (sx, sy) = (self.scale.0.abs(), self.scale.1.abs());
        let w = (sw * sx).round().max(1.0) as u32;
        let h = (sh * sy).round().max(1.0) as u32;
        let place = |width: u32, height: u32| {
            (
                (self.center.0 - width as f64 / 2.0).round() as i32,
                (self.center.1 - height as f64 / 2.0).round() as i32,
            )
        };
        if affine && no_rotation && no_skew {
            // 回到原始尺寸（±2px 手柄误差吸附）时 1:1 索引拷贝，
            // 与 ScaleLayer 的 P0-2a 修复保持逐像素一致。
            let snap_back = !self.flip.0
                && !self.flip.1
                && (w as f64 - sw).abs() <= 2.0
                && (h as f64 - sh).abs() <= 2.0;
            let mut image = if snap_back {
                (*self.source).clone()
            } else {
                image::imageops::resize(
                    self.source.as_ref(),
                    w,
                    h,
                    image::imageops::FilterType::Lanczos3,
                )
            };
            if self.flip.0 {
                image = image::imageops::flip_horizontal(&image);
            }
            if self.flip.1 {
                image = image::imageops::flip_vertical(&image);
            }
            // 吸附时输出尺寸是原图尺寸，不能用 QML 换算出的 ±2px 值摆放。
            let offset = place(image.width(), image.height());
            return (image, offset);
        }
        if affine && no_skew && !self.flip.0 && !self.flip.1 && !no_rotation {
            // 任意 scale + 旋转：先把未插值原图按 Lanczos3 缩放到目标尺度，
            // 再从那张图一次性 HQ 旋转 —— 与旧「当前位图 → rotate_image_hq」
            // 逐像素一致，但输入恒为原图，连续旋转永不累积模糊。
            let base = if (sx - 1.0).abs() < 1e-9 && (sy - 1.0).abs() < 1e-9 {
                (*self.source).clone()
            } else {
                image::imageops::resize(
                    self.source.as_ref(),
                    w,
                    h,
                    image::imageops::FilterType::Lanczos3,
                )
            };
            let (image, _) = crate::transform::rotate_image_hq(&base, self.rotation as f32, true);
            let offset = place(image.width(), image.height());
            return (image, offset);
        }
        self.render_bounds(true)
    }

    pub fn params_bytes(&self) -> Vec<u8> {
        self.params().to_bytes()
    }

    pub fn from_params_bytes(bytes: &[u8], source: Arc<RgbaImage>) -> Option<Self> {
        Self::from_params(TransformParams::from_bytes(bytes)?, source)
    }
}

impl PartialEq for LayerTransform {
    fn eq(&self, other: &Self) -> bool {
        self.params() == other.params()
            && (Arc::ptr_eq(&self.source, &other.source)
                || self.source.as_raw() == other.source.as_raw())
    }
}

fn set_positive(slot: &mut f64, v: f64) -> bool {
    if v <= 0.0 {
        return false;
    }
    *slot = v;
    true
}

/// 基线探针输出（`--probe-transform`）。不依赖 Qt / QML，可在 P0-1 接线前
/// 记录现有几何期望值，也供 `scripts/transform_probe.sh` 做回归断言。
pub fn probe_lines() -> Vec<String> {
    fn f(v: f64) -> f64 {
        if v.abs() < 5e-7 { 0.0 } else { v }
    }
    fn num(v: f64) -> String {
        format!("{:.6}", f(v))
    }
    fn arr<const N: usize>(values: [f64; N]) -> String {
        let parts: Vec<String> = values.iter().map(|&v| num(v)).collect();
        format!("[{}]", parts.join(","))
    }
    fn arr4(values: [(f64, f64); 4]) -> String {
        let parts: Vec<String> = values
            .iter()
            .flat_map(|&(x, y)| [num(x), num(y)])
            .collect();
        format!("[{}]", parts.join(","))
    }
    fn case(name: &str, t: &LayerTransform) -> (String, bool) {
        let m = t.matrix();
        let inv_err = match t.inverse() {
            Some(inv) => {
                let p = (37.25, 61.75);
                let q = m.apply(p);
                let back = inv.apply(q);
                (back.0 - p.0).abs().max((back.1 - p.1).abs())
            }
            None => f64::INFINITY,
        };
        let b = t.bounds();
        let pv = t.panel_values();
        let (w, h) = t.source_size();
        // 任意仿射的外接框都可由矩阵列长解析求出。
        let analytic_w = m.a.abs() * w + m.c.abs() * h;
        let analytic_h = m.b.abs() * w + m.d.abs() * h;
        let bounds_ok = (b.width - analytic_w).abs() < 1e-6 && (b.height - analytic_h).abs() < 1e-6;
        let hit_ok = t.hit(t.center) && !t.hit((t.center.0 + 10_000.0, t.center.1));
        let bytes = t.params_bytes();
        let serial_ok = LayerTransform::from_params_bytes(&bytes, t.source.clone())
            .map(|back| back.params_bytes() == bytes && back.matrix().as_array() == m.as_array())
            .unwrap_or(false);
        let line = format!(
            r#"{{"event":"transform-case","case":"{}","matrix":{},"quad":{},"bounds":{},"panel":{},"source":{},"inv_err":{},"bounds_ok":{},"hit_ok":{},"serial_ok":{}}}"#,
            name,
            arr(m.as_array()),
            arr4(t.quad()),
            arr([b.x, b.y, b.width, b.height]),
            arr([pv.x, pv.y, pv.width, pv.height, pv.rotation]),
            arr([w, h]),
            num(inv_err),
            bounds_ok,
            hit_ok,
            serial_ok,
        );
        (line, inv_err < 1e-9 && bounds_ok && hit_ok && serial_ok)
    }

    let source = || RgbaImage::from_pixel(100, 80, image::Rgba([10, 20, 30, 255]));
    let mut lines = Vec::new();
    let mut all_ok = true;
    let mut n = 0usize;
    let mut push = |name: &str, t: &LayerTransform| {
        let (line, ok) = case(name, t);
        lines.push(line);
        all_ok &= ok;
        n += 1;
    };

    push("identity", &LayerTransform::new(source()));

    let mut t = LayerTransform::new(source());
    t.rotation = 90.0;
    push("rotate-90", &t);

    // 连续旋转：角度累加、包围盒按解析式变化，源图尺寸恒定（不再二次烘焙）。
    for deg in [17.0, 40.0, 51.0, 88.0] {
        t.rotation = deg;
        push(&format!("rotate-{}", deg as i32), &t);
    }

    // 缩小再放大：source 不变，bounds 回到 100×80（清晰度无损的结构性前提）。
    t.rotation = 0.0;
    t.scale = (0.4, 0.4);
    push("scale-out", &t);
    t.scale = (2.5, 2.5);
    t.scale = (1.0, 1.0);
    push("scale-out-in", &t);

    // 旋转 → 非等比拉伸 → 再旋转：外接框不膨胀。
    t.rotation = 45.0;
    t.scale = (1.8, 0.6);
    push("rotate-scale", &t);
    t.rotation = 88.0;
    push("rotate-scale-rotate", &t);

    // 翻转 / 斜切 / 越出画布 / 透视字段预留。
    t = LayerTransform::from_offset(source(), -30, -20);
    t.flip.0 = true;
    push("flip-outside", &t);
    t = LayerTransform::new(source());
    t.skew = (0.2, 0.0);
    push("skew", &t);
    t = LayerTransform::new(source());
    t.perspective = Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
    let persp_roundtrip = LayerTransform::from_params_bytes(&t.params_bytes(), t.source.clone())
        .map(|back| back.perspective == t.perspective)
        .unwrap_or(false);
    lines.push(format!(
        r#"{{"event":"transform-case","case":"perspective-reserved","serial_ok":{},"is_affine":{}}}"#,
        persp_roundtrip, t.is_affine()
    ));
    all_ok &= persp_roundtrip && !t.is_affine();
    n += 1;

    // 数值面板：写 W 时保持 rotation 不变；TopLeft 参考点坐标不动。
    let mut p = LayerTransform::new(source());
    p.rotation = 30.0;
    let before = p.panel_values();
    let pivot_ok = p.set_panel_value(Field::Width, 200.0, ReferencePoint::TopLeft);
    let after = p.panel_values();
    let panel_ok = pivot_ok
        && (after.width - 200.0).abs() < 1e-6
        && (after.rotation - 30.0).abs() < 1e-9
        && (after.x - before.x).abs() < 1e-6
        && (after.y - before.y).abs() < 1e-6;
    lines.push(format!(
        r#"{{"event":"transform-case","case":"panel-pivot","panel_ok":{},"w":{},"angle":{}}}"#,
        panel_ok, num(after.width), num(after.rotation)
    ));
    all_ok &= panel_ok;
    n += 1;

    // §2.4 的输出等价性契约（--probe-bake 的地基）：烘焙成 identity 位图后
    // 必须走 1:1 索引，不能被二次插值，否则整幅发糊。
    let mut b = LayerTransform::from_offset(source(), 10, 5);
    b.rotation = 27.0;
    b.scale = (0.8, 1.3);
    let (baked, origin) = b.render_bounds(true);
    let rect = Rect::new(origin.0, origin.1, baked.width(), baked.height());
    let direct = b.render(rect, true);
    let identity = LayerTransform::from_offset(baked.clone(), origin.0, origin.1);
    // §2.4 陷阱 2 的核心：烘焙成 identity 后必须逐像素 1:1，HQ 与拖动帧都一样。
    let once_more = identity.render(rect, true);
    let once_more_bl = identity.render(rect, false);
    let diff = |a: &RgbaImage, c: &RgbaImage| -> u32 {
        a.as_raw()
            .iter()
            .zip(c.as_raw())
            .map(|(x, y)| x.abs_diff(*y) as u32)
            .max()
            .unwrap_or(0)
    };
    let direct_diff = diff(&direct, &baked);
    let identity_diff = diff(&baked, &once_more);
    let identity_diff_bl = diff(&baked, &once_more_bl);
    let max_diff = direct_diff.max(identity_diff).max(identity_diff_bl);
    let bake_ok = max_diff == 0 && identity.is_identity_fast_path();
    lines.push(format!(
        r#"{{"event":"transform-case","case":"bake-equivalence","bake_ok":{},"max_diff":{},"hq_diff":{},"bl_diff":{},"fast_path":{},"w":{},"h":{}}}"#,
        bake_ok, max_diff, identity_diff, identity_diff_bl,
        identity.is_identity_fast_path(), baked.width(), baked.height()
    ));
    all_ok &= bake_ok;
    n += 1;

    lines.push(format!(
        r#"{{"event":"probe-transform-done","version":{},"cases":{},"all_ok":{}}}"#,
        TRANSFORM_SERIAL_VERSION, n, all_ok
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn src() -> RgbaImage {
        RgbaImage::from_pixel(100, 80, Rgba([10, 20, 30, 255]))
    }

    fn assert_close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    fn assert_point(a: (f64, f64), b: (f64, f64)) {
        assert_close(a.0, b.0);
        assert_close(a.1, b.1);
    }

    #[test]
    fn identity_maps_source_to_itself() {
        let t = LayerTransform::new(src());
        assert_eq!(t.matrix(), Affine2::identity());
        assert_eq!(t.quad(), [(0.0, 0.0), (100.0, 0.0), (100.0, 80.0), (0.0, 80.0)]);
        assert_eq!(t.bounds(), RectF::new(0.0, 0.0, 100.0, 80.0));
        assert!(t.is_identity_fast_path());
        assert_point(t.inverse().unwrap().apply((12.5, 40.5)), (12.5, 40.5));
    }

    #[test]
    fn from_offset_matches_legacy_layer_offset() {
        let t = LayerTransform::from_offset(src(), -30, -20);
        assert_eq!(t.offset(), (-30.0, -20.0));
        assert_eq!(t.bounds(), RectF::new(-30.0, -20.0, 100.0, 80.0));
        assert_eq!(t.matrix(), Affine2::translation(-30.0, -20.0));
        assert!(t.is_identity_fast_path());
    }

    #[test]
    fn rotation_90_bounds_and_negative_coordinates() {
        let mut t = LayerTransform::new(src());
        t.rotation = 90.0;
        let b = t.bounds();
        assert_close(b.x, 10.0);
        assert_close(b.y, -10.0);
        assert_close(b.width, 80.0);
        assert_close(b.height, 100.0);
        // 旋 90° 不是 1:1 快路径，但整数 90° 可走索引重排（由采样层实现）。
        assert!(!t.is_identity_fast_path());
        assert_point(t.inverse().unwrap().apply(t.center), (50.0, 40.0));
    }

    #[test]
    fn matrix_inverse_roundtrip_for_all_param_combinations() {
        let mut t = LayerTransform::from_offset(src(), 137, -42);
        t.scale = (1.7, 0.35);
        t.rotation = 37.0;
        t.flip = (true, false);
        t.skew = (0.15, -0.08);
        let m = t.matrix();
        let inv = t.inverse().expect("non-degenerate");
        for p in [(0.0, 0.0), (13.25, 71.5), (100.0, 80.0), (-5.5, 900.0)] {
            assert_point(inv.apply(m.apply(p)), p);
        }
        // 组合顺序：先平移到中心、再旋转/缩放/斜切、最后平移回原图中心。
        assert_point(m.apply((50.0, 40.0)), t.center);
    }

    #[test]
    fn degenerate_scale_has_no_inverse() {
        let mut t = LayerTransform::new(src());
        t.scale.0 = 0.0;
        assert!(t.inverse().is_none());
        assert!(!t.hit((10.0, 10.0)));
    }

    #[test]
    fn scale_down_then_up_never_bakes_source() {
        let mut t = LayerTransform::new(src());
        for (sx, sy) in [(0.4, 0.4), (2.5, 2.5), (1.0, 1.0), (0.1, 0.1), (10.0, 10.0), (1.0, 1.0)] {
            t.scale = (sx, sy);
            assert_eq!((t.source.width(), t.source.height()), (100, 80));
        }
        assert_eq!(t.bounds(), RectF::new(0.0, 0.0, 100.0, 80.0));
        assert!(t.is_identity_fast_path());
    }

    #[test]
    fn cumulative_rotation_does_not_grow_bounds() {
        let mut t = LayerTransform::new(src());
        for deg in [17.0, 40.0, 51.0, 88.0, 17.0, 40.0] {
            t.rotation = deg;
            let m = t.matrix();
            let b = t.bounds();
            let expected_w = m.a.abs() * 100.0 + m.c.abs() * 80.0;
            let expected_h = m.b.abs() * 100.0 + m.d.abs() * 80.0;
            assert_close(b.width, expected_w);
            assert_close(b.height, expected_h);
            assert_eq!((t.source.width(), t.source.height()), (100, 80));
        }
    }

    #[test]
    fn rotate_scale_rotate_equals_direct_params() {
        let mut a = LayerTransform::new(src());
        a.rotation = 45.0;
        a.scale = (1.8, 0.6);
        a.rotation = 88.0;
        let mut b = LayerTransform::new(src());
        b.rotation = 88.0;
        b.scale = (1.8, 0.6);
        let (ab, bb) = (a.bounds(), b.bounds());
        assert_close(ab.width, bb.width);
        assert_close(ab.height, bb.height);
        for (x, y) in a.matrix().as_array().iter().zip(b.matrix().as_array()) {
            assert_close(*x, y);
        }
    }

    #[test]
    fn hit_uses_inverse_and_alpha() {
        let mut t = LayerTransform::new(src());
        t.rotation = 30.0;
        assert!(t.hit(t.center));
        assert!(!t.hit((t.center.0 + 10_000.0, t.center.1)));
        let mut transparent = RgbaImage::new(100, 80);
        transparent.put_pixel(50, 40, Rgba([255, 0, 0, 200]));
        let t2 = LayerTransform::new(transparent);
        assert!(t2.hit((50.5, 40.5)));
        assert!(!t2.hit((10.5, 10.5)));
        assert!(!t2.hit_alpha((50.5, 40.5), 254));
    }

    #[test]
    fn panel_values_are_bounds_and_writes_preserve_pivot() {
        let mut t = LayerTransform::new(src());
        t.rotation = 30.0;
        let before = t.panel_values();
        assert!(t.set_panel_value(Field::Width, 200.0, ReferencePoint::TopLeft));
        let after = t.panel_values();
        assert_close(after.width, 200.0);
        assert_close(after.rotation, 30.0);
        assert_close(after.x, before.x);
        assert_close(after.y, before.y);
        // 等比解下 H 按同一倍数变化。
        assert_close(after.height / before.height, after.width / before.width);
        assert!(t.set_panel_value(Field::X, -12.0, ReferencePoint::Center));
        assert_close(t.panel_values().x, -12.0);
    }

    #[test]
    fn single_axis_solve_is_stable_near_90_degrees() {
        for deg in [0.0, 45.0, 89.9, 90.0, 90.1, 137.0, 180.0] {
            let mut t = LayerTransform::new(src());
            t.rotation = deg;
            let before = t.bounds();
            assert!(t.set_panel_value_with_solve(
                Field::Width,
                before.width * 1.5,
                ReferencePoint::Center,
                BoundsSolve::SingleAxis,
            ));
            let after = t.bounds();
            assert_close(after.width, before.width * 1.5);
            assert!(t.scale.0.is_finite() && t.scale.0 > 0.0);
            assert_close(t.bounds().center().0, before.center().0);
        }
    }

    #[test]
    fn params_serialization_roundtrip_preserves_matrix_and_perspective() {
        let mut t = LayerTransform::from_offset(src(), 17, -9);
        t.scale = (0.75, 2.25);
        t.rotation = 123.5;
        t.flip = (false, true);
        t.skew = (0.1, -0.2);
        t.perspective = Some([1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        let bytes = t.params_bytes();
        let back = LayerTransform::from_params_bytes(&bytes, t.source.clone()).expect("decode");
        assert_eq!(back.params(), t.params());
        assert_eq!(back.perspective, t.perspective);
        assert_eq!(back.matrix().as_array(), t.matrix().as_array());
        assert_eq!(back.bounds(), t.bounds());
    }

    #[test]
    fn identity_render_never_interpolates_transparent_pixels() {
        // 全透明像素里残留的 RGB 必须原样保留：走一遍采样 normalize 会把它
        // 清成 (0,0,0,0)，就是 §2.4「identity 被二次插值」的可见症状。
        let mut img = RgbaImage::new(4, 4);
        img.put_pixel(1, 1, Rgba([10, 20, 30, 0]));
        let t = LayerTransform::from_offset(img.clone(), -3, 7);
        assert!(t.is_identity_fast_path());
        for hq in [false, true] {
            let out = t.render(Rect::new(-3, 7, 4, 4), hq);
            assert_eq!(out.as_raw(), img.as_raw(), "hq={hq}");
        }
        let part = t.render(Rect::new(-2, 8, 3, 3), true);
        assert_eq!(*part.get_pixel(0, 0), *img.get_pixel(1, 1));
        assert_eq!(*part.get_pixel(2, 2), Rgba([0, 0, 0, 0]));
    }

    #[test]
    fn params_reject_bad_version_magic_or_nan() {
        let t = LayerTransform::new(src());
        let mut bytes = t.params_bytes();
        assert!(TransformParams::from_bytes(&bytes[..3]).is_none());
        bytes[4] = 99;
        assert!(LayerTransform::from_params_bytes(&bytes, t.source.clone()).is_none());
        let mut nan = t.params();
        nan.rotation = f64::NAN;
        assert!(TransformParams::from_bytes(&nan.to_bytes()).is_none());
    }

    #[test]
    fn probe_is_self_consistent() {
        let lines = probe_lines();
        let done = lines.last().expect("done line");
        assert!(done.contains(r#""all_ok":true"#), "{done}");
        assert!(lines.iter().any(|l| l.contains(r#""case":"rotate-90""#)));
        assert!(lines.iter().any(|l| l.contains(r#""case":"scale-out-in""#)));
    }
}
