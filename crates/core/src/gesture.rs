//! P1-A（D14）：缩放手势的修饰键语义求解。
//!
//! **语义只在这一处**（§9.1 落点 2）：QML 只透传位掩码与光标位移，锁比例 /
//! 从中心 / 斜切的几何全部在这里解出。结果仍是一个外接框（+ 可选斜切量），
//! 落定照旧走一条 `SetLayerTransform`。
//!
//! 语义表（§3.3.4 分流 + §9.1）：
//! | 手势      | Shift          | Alt       | Ctrl                    |
//! | --------- | -------------- | --------- | ----------------------- |
//! | 拖角点    | 锁比例（默认自由）| 从中心   | 自由扭曲 → P1-A2（本期按自由处理） |
//! | 拖边中点  | —              | 从中心    | **斜切**（边沿自身方向滑动）|
//!
//! 斜切的实现走 `LayerTransform` 既有的 `skew` 字段（source 空间剪切，
//! `matrix()` 已支持）：竖边（w/e）用 `ky`——右缘随 u=w 整体平移、左缘
//! 不动；横边（n/s）用 `kx`。被拖的是 w/n 缘时剪切绕中心会把对侧也带着
//! 走，用 `center_delta` 平移补偿，把对侧锚回原位。

use crate::layer_transform::RectF;

/// 被拖动的手柄。QML 的 role 串（"nw".."se"）在 bridge 层转成这里。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleRole {
    EdgeW,
    EdgeE,
    EdgeN,
    EdgeS,
    CornerNW,
    CornerNE,
    CornerSW,
    CornerSE,
}

impl ScaleRole {
    pub fn parse(role: &str) -> Option<Self> {
        Some(match role {
            "w" => Self::EdgeW,
            "e" => Self::EdgeE,
            "n" => Self::EdgeN,
            "s" => Self::EdgeS,
            "nw" => Self::CornerNW,
            "ne" => Self::CornerNE,
            "sw" => Self::CornerSW,
            "se" => Self::CornerSE,
            _ => return None,
        })
    }

    pub fn corner(self) -> bool {
        !matches!(self, Self::EdgeW | Self::EdgeE | Self::EdgeN | Self::EdgeS)
    }

    pub fn west(self) -> bool {
        matches!(self, Self::EdgeW | Self::CornerNW | Self::CornerSW)
    }

    pub fn north(self) -> bool {
        matches!(self, Self::EdgeN | Self::CornerNW | Self::CornerNE)
    }

    /// 吸附用的边位掩码（snap.rs 的 EDGE_* 位）。
    pub fn edge_bits(self) -> u8 {
        let mut bits = 0;
        if self.west() {
            bits |= 1;
        }
        if matches!(self, Self::EdgeE | Self::CornerNE | Self::CornerSE) {
            bits |= 4;
        }
        if self.north() {
            bits |= 8;
        }
        if matches!(self, Self::EdgeS | Self::CornerSW | Self::CornerSE) {
            bits |= 32;
        }
        bits
    }
}

/// 一次求解的结果：外接框（图像坐标，恒正尺寸）+ 翻转 + 斜切量 +
/// 斜切锚定补偿（加到 `transform.center` 上）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaleSolve {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub flip: (bool, bool),
    pub skew: (f64, f64),
    pub center_delta: (f64, f64),
}

/// 旋转吸角：Shift 按住时吸附到 15° 整数倍（PS 语义），松开回连续角度。
pub fn snap_angle_15(deg: f64, shift: bool) -> f64 {
    if shift {
        (deg / 15.0).round() * 15.0
    } else {
        deg
    }
}

/// 角度吸附的两档语义（旋转手柄用）：
/// * `force`（Shift）= 硬吸，落在 `step` 的整数倍上，中间没有过渡；
/// * 否则 = **磁吸**：只有已经贴近某个整数倍（`|差| <= tol`）才吸上去，
///   其余角度原样返回。这样 0/45/90/180 这些常用角度会「自己咬住」，
///   同时又能摆出任意角度（拖过去时轻微一顿，越过就脱开）。
pub fn snap_angle(deg: f64, force: bool, step: f64, tol: f64) -> f64 {
    if !(step > 0.0) || !step.is_finite() || !deg.is_finite() {
        return deg;
    }
    let nearest = (deg / step).round() * step;
    if force || (tol > 0.0 && (deg - nearest).abs() <= tol) {
        nearest
    } else {
        deg
    }
}

/// 缩放手势语义求解。
///
/// * `start`：起手外接框（图像坐标）。
/// * `dx/dy`：光标相对按下点的位移（图像坐标）。
/// * `layer_scale`：图层当前 `(sx, sy)`（skew 换算需要；identity 传 (1,1)）。
pub fn solve_scale_gesture(
    start: RectF,
    role: ScaleRole,
    dx: f64,
    dy: f64,
    shift: bool,
    alt: bool,
    ctrl: bool,
    layer_scale: (f64, f64),
) -> ScaleSolve {
    // ---- Ctrl + 边 = 斜切：被拖边沿自身方向滑动，对侧锚定 ------------
    // 角点 + Ctrl 的「自由扭曲」需要单应采样（R8），本期按自由缩放处理。
    if ctrl && !role.corner() {
        let (sx, sy) = layer_scale;
        let (mut kx, mut ky) = (0.0_f64, 0.0_f64);
        let mut cd = (0.0_f64, 0.0_f64);
        match role {
            // 右缘 (u=w) 整体平移 ky·w：slide = ky·(sw/sx)·sy。
            ScaleRole::EdgeE => ky = dy * sx / (start.width * sy),
            // 左缘 (u=0) 不动：剪切绕中心，把左缘平移补偿到光标处。
            ScaleRole::EdgeW => {
                ky = -dy * sx / (start.width * sy);
                cd.1 = dy;
            }
            ScaleRole::EdgeS => kx = dx * sy / (start.height * sx),
            ScaleRole::EdgeN => {
                kx = -dx * sy / (start.height * sx);
                cd.0 = dx;
            }
            _ => {}
        }
        // 斜切帧的外接框：剪切后 u 方向变宽 / v 方向变高（给 viz 用）。
        let grow_x = kx.abs() * (start.height / sy) * sx;
        let grow_y = ky.abs() * (start.width / sx) * sy;
        return ScaleSolve {
            x: start.x - grow_x * if role.west() { 0.0 } else { 0.5 },
            y: start.y - grow_y * if role.north() { 0.0 } else { 0.5 },
            width: (start.width + grow_x).max(1.0),
            height: (start.height + grow_y).max(1.0),
            flip: (false, false),
            skew: (kx, ky),
            center_delta: cd,
        };
    }

    // ---- 常规缩放：边/角按位移移动，Alt 让对侧反向对称 ---------------
    let from_center = alt;
    let mut l = start.x;
    let mut r = start.x + start.width;
    let mut t = start.y;
    let mut b = start.y + start.height;
    if role.west() {
        l = start.x + dx;
        if from_center {
            r = start.x + start.width - dx;
        }
    } else if matches!(role, ScaleRole::EdgeE | ScaleRole::CornerNE | ScaleRole::CornerSE) {
        r = start.x + start.width + dx;
        if from_center {
            l = start.x - dx;
        }
    }
    if role.north() {
        t = start.y + dy;
        if from_center {
            b = start.y + start.height - dy;
        }
    } else if matches!(role, ScaleRole::EdgeS | ScaleRole::CornerSW | ScaleRole::CornerSE) {
        b = start.y + start.height + dy;
        if from_center {
            t = start.y - dy;
        }
    }

    let flip = (r < l, b < t);
    let (mut x, mut y) = (l.min(r), t.min(b));
    let (mut w, mut h) = ((r - l).abs(), (b - t).abs());

    // ---- 角点 + Shift = 锁比例（默认自由，PS 语义）-------------------
    // 镜像状态下不锁：锚点在对侧，锁比例会让框跳一下（与 P0-3 相同结论）。
    if role.corner() && shift && !flip.0 && !flip.1 && w > 0.5 && h > 0.5 {
        let ar = start.width / start.height.max(1.0);
        if w / h > ar {
            w = h * ar;
        } else {
            h = w / ar;
        }
        if from_center {
            x = start.x + start.width / 2.0 - w / 2.0;
            y = start.y + start.height / 2.0 - h / 2.0;
        } else {
            // 未拖的对侧保持原位。
            if role.west() {
                x = start.x + start.width - w;
            }
            if role.north() {
                y = start.y + start.height - h;
            }
        }
    }

    ScaleSolve {
        x,
        y,
        width: w.max(1.0),
        height: h.max(1.0),
        flip,
        skew: (0.0, 0.0),
        center_delta: (0.0, 0.0),
    }
}

/// 等比缩放里真正生效的吸附轴。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RatioSnapAxis {
    X,
    Y,
}

/// `apply_ratio_snap` 的结果：修正后的框 + 被采纳的吸附轴。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RatioSnap {
    pub rect: RectF,
    /// `None` 表示这次吸附全部放弃（会把框压到 <1px 时）。
    pub axis: Option<RatioSnapAxis>,
}

/// 把吸附修正量摊到「等比（角点 + Shift）」的缩放手势上。
///
/// `solve_rect` 给出的 `adx/ady` 是**单轴**的边位移：直接叠加到 w 或 h 上，
/// 另一轴不动，锁定好的宽高比就被拉坏了（用户看到的「等比拖一下，图被压
/// 扁 / 拉长」）。等比手势只可能满足一条对齐线，所以这里：
///
/// 1. 两轴都有修正时取**位移更小**的那一轴（它才是更近的那条参考线），
///    只有一个轴有修正时就用它；
/// 2. 该轴按吸附量移动被拖边，另一轴**按锁定比例重算**；
/// 3. 锚点与手势一致：未拖的对侧固定，`from_center`（Alt）时绕中心对称。
///
/// 结果尺寸会被夹到 ≥1px；夹到就整条吸附放弃（返回原框 + `axis: None`），
/// 免得在极小尺寸上出现「吸一下宽度没了」。
pub fn apply_ratio_snap(
    rect: RectF,
    role: ScaleRole,
    adx: f64,
    ady: f64,
    from_center: bool,
) -> RatioSnap {
    let none = RatioSnap {
        rect,
        axis: None,
    };
    let (w, h) = (rect.width.max(1.0), rect.height.max(1.0));
    if !adx.is_finite() || !ady.is_finite() || (adx == 0.0 && ady == 0.0) {
        return none;
    }
    let ar = w / h;
    if !(ar > 0.0) {
        return none;
    }
    let use_x = if adx != 0.0 && ady != 0.0 {
        adx.abs() <= ady.abs()
    } else {
        adx != 0.0
    };

    let (mut x, mut y) = (rect.x, rect.y);
    let (mut nw, mut nh) = (w, h);
    if use_x {
        // 被拖边位移直接换算成宽度变化：西缘右移 → 变窄。
        let dw = if role.west() { -adx } else { adx };
        nw = w + dw;
        if nw < 1.0 {
            return none;
        }
        nh = nw / ar;
        if nh < 1.0 {
            return none;
        }
        // 水平锚点：东缘被拖 → 西缘固定；西缘被拖 → 东缘固定。
        if from_center {
            x = rect.x + w / 2.0 - nw / 2.0;
        } else if role.west() {
            x = rect.x + w - nw;
        }
        // 竖直方向跟随：未拖的一侧固定。
        if from_center {
            y = rect.y + h / 2.0 - nh / 2.0;
        } else if role.north() {
            y = rect.y + h - nh;
        }
    } else {
        let dh = if role.north() { -ady } else { ady };
        nh = h + dh;
        if nh < 1.0 {
            return none;
        }
        nw = nh * ar;
        if nw < 1.0 {
            return none;
        }
        if from_center {
            y = rect.y + h / 2.0 - nh / 2.0;
        } else if role.north() {
            y = rect.y + h - nh;
        }
        if from_center {
            x = rect.x + w / 2.0 - nw / 2.0;
        } else if role.west() {
            x = rect.x + w - nw;
        }
    }

    RatioSnap {
        rect: RectF::new(x, y, nw, nh),
        axis: Some(if use_x { RatioSnapAxis::X } else { RatioSnapAxis::Y }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> RectF {
        RectF::new(40.0, 30.0, 200.0, 100.0)
    }

    fn solve(role: ScaleRole, dx: f64, dy: f64, shift: bool, alt: bool, ctrl: bool) -> ScaleSolve {
        solve_scale_gesture(start(), role, dx, dy, shift, alt, ctrl, (1.0, 1.0))
    }

    #[test]
    fn corner_is_free_by_default_and_locks_with_shift() {
        // PS 语义（D14）：角点默认自由，Shift 才锁比例 —— 与旧实现相反。
        let r = solve(ScaleRole::CornerSE, 80.0, 10.0, false, false, false);
        assert!((r.width - 280.0).abs() < 1e-9);
        assert!((r.height - 110.0).abs() < 1e-9);
        // 左上角（对侧）锚定。
        assert!((r.x - 40.0).abs() < 1e-9 && (r.y - 30.0).abs() < 1e-9);

        let r = solve(ScaleRole::CornerSE, 80.0, 10.0, true, false, false);
        let ar = 2.0;
        assert!((r.width / r.height - ar).abs() < 1e-9, "w={} h={}", r.width, r.height);
        // 锁比例后仍是向外扩：右下方向。
        assert!(r.width > 200.0 && r.height > 100.0);
    }

    #[test]
    fn alt_scales_from_center() {
        let r = solve(ScaleRole::EdgeE, 30.0, 0.0, false, true, false);
        // 对侧（左缘）反向移动同样距离，中心恒定。
        assert!((r.x - 10.0).abs() < 1e-9, "x={}", r.x);
        assert!((r.width - 260.0).abs() < 1e-9);
        let cx = r.x + r.width / 2.0;
        assert!((cx - (40.0 + 100.0)).abs() < 1e-9);

        let r = solve(ScaleRole::CornerNW, -20.0, -10.0, false, true, false);
        let cx = r.x + r.width / 2.0;
        let cy = r.y + r.height / 2.0;
        assert!((cx - 140.0).abs() < 1e-9 && (cy - 80.0).abs() < 1e-9);
        assert!((r.width - 240.0).abs() < 1e-9 && (r.height - 120.0).abs() < 1e-9);
    }

    #[test]
    fn shift_plus_alt_locks_ratio_around_center() {
        let r = solve(ScaleRole::CornerSE, 60.0, 5.0, true, true, false);
        let ar = 2.0;
        assert!((r.width / r.height - ar).abs() < 1e-9);
        let cx = r.x + r.width / 2.0;
        let cy = r.y + r.height / 2.0;
        assert!((cx - 140.0).abs() < 1e-9 && (cy - 80.0).abs() < 1e-9);
    }

    #[test]
    fn ctrl_edge_shears_and_anchors_opposite_edge() {
        // 东缘向下拖 30：右缘整体下移 30（ky·sw = 30），左缘不动。
        let r = solve(ScaleRole::EdgeE, 0.0, 30.0, false, false, true);
        assert!((r.skew.1 - 30.0 / 200.0).abs() < 1e-9);
        assert_eq!(r.skew.0, 0.0);
        assert_eq!(r.center_delta, (0.0, 0.0));

        // 西缘向下拖 30：对侧（右缘）锚定 → 剪切反向 + center 下移 30。
        let r = solve(ScaleRole::EdgeW, 0.0, 30.0, false, false, true);
        assert!((r.skew.1 + 30.0 / 200.0).abs() < 1e-9);
        assert!((r.center_delta.1 - 30.0).abs() < 1e-9);

        // 南缘向右拖 50：底缘右移 50（kx·sh = 50）。
        let r = solve(ScaleRole::EdgeS, 50.0, 0.0, false, false, true);
        assert!((r.skew.0 - 50.0 / 100.0).abs() < 1e-9);
        assert_eq!(r.center_delta, (0.0, 0.0));

        // 非均匀缩放：换算含 sx/sy。
        let r = solve_scale_gesture(
            start(),
            ScaleRole::EdgeE,
            0.0,
            30.0,
            false,
            false,
            true,
            (2.0, 0.5),
        );
        assert!((r.skew.1 - 30.0 * 2.0 / (200.0 * 0.5)).abs() < 1e-9);
    }

    #[test]
    fn corner_with_ctrl_falls_back_to_free_resize() {
        let r = solve(ScaleRole::CornerSE, 80.0, 10.0, false, false, true);
        assert_eq!(r.skew, (0.0, 0.0));
        assert!((r.width - 280.0).abs() < 1e-9);
    }

    #[test]
    fn flip_detected_when_dragging_past_opposite_edge() {
        let r = solve(ScaleRole::EdgeW, 260.0, 0.0, false, false, false);
        assert!(r.flip.0);
        assert!(!r.flip.1);
        assert!((r.width - 60.0).abs() < 1e-9, "w={}", r.width);
    }

    #[test]
    fn edge_bits_match_snap_constants() {
        assert_eq!(ScaleRole::EdgeW.edge_bits(), 1);
        assert_eq!(ScaleRole::EdgeE.edge_bits(), 4);
        assert_eq!(ScaleRole::EdgeN.edge_bits(), 8);
        assert_eq!(ScaleRole::EdgeS.edge_bits(), 32);
        assert_eq!(ScaleRole::CornerNW.edge_bits(), 1 | 8);
        assert_eq!(ScaleRole::CornerSE.edge_bits(), 4 | 32);
    }

    #[test]
    fn rotate_snaps_to_15_degree_steps_only_with_shift() {
        assert!((snap_angle_15(23.0, false) - 23.0).abs() < 1e-9);
        assert!((snap_angle_15(23.0, true) - 30.0).abs() < 1e-9);
        assert!((snap_angle_15(-22.0, true) - (-15.0)).abs() < 1e-9);
        assert!((snap_angle_15(44.0, true) - 45.0).abs() < 1e-9);
    }

    #[test]
    fn magnetic_angle_only_bites_near_multiples() {
        // 磁吸窗口 3°：91.4 → 90；93.6 已经越过窗口 → 原样。
        assert!((snap_angle(91.4, false, 15.0, 3.0) - 90.0).abs() < 1e-9);
        assert!((snap_angle(93.6, false, 15.0, 3.0) - 93.6).abs() < 1e-9);
        // 负角对称（-89 贴近 -90）。
        assert!((snap_angle(-89.0, false, 15.0, 3.0) - (-90.0)).abs() < 1e-9);
        // Shift = 硬吸，没有窗口限制。
        assert!((snap_angle(93.6, true, 15.0, 0.0) - 90.0).abs() < 1e-9);
        // tol = 0 等于不吸（脚本 / 数值面板路径行为不变）。
        assert!((snap_angle(91.4, false, 15.0, 0.0) - 91.4).abs() < 1e-9);
    }

    #[test]
    fn ratio_snap_keeps_aspect_and_anchor() {
        // 240×120（ar=2），右下角拖动，吸附把右缘再推 +6。
        let r = apply_ratio_snap(RectF::new(40.0, 30.0, 240.0, 120.0), ScaleRole::CornerSE, 6.0, 0.0, false);
        let (rect, axis) = (r.rect, r.axis);
        assert_eq!(axis, Some(RatioSnapAxis::X));
        assert!((rect.width - 246.0).abs() < 1e-9, "{rect:?}");
        // 另一轴按比例跟随，左上角锚定不动。
        assert!((rect.height - 123.0).abs() < 1e-9, "{rect:?}");
        assert!((rect.x - 40.0).abs() < 1e-9 && (rect.y - 30.0).abs() < 1e-9);
        assert!((rect.width / rect.height - 2.0).abs() < 1e-9);
    }

    #[test]
    fn ratio_snap_prefers_the_closer_axis() {
        // 两轴都有修正时取位移小的那条（更近的参考线），不再两轴各改一半。
        let r = apply_ratio_snap(RectF::new(40.0, 30.0, 240.0, 120.0), ScaleRole::CornerSE, 7.0, -2.0, false);
        assert_eq!(r.axis, Some(RatioSnapAxis::Y));
        assert!((r.rect.height - 118.0).abs() < 1e-9, "{:?}", r.rect);
        assert!((r.rect.width - 236.0).abs() < 1e-9, "{:?}", r.rect);
        // 底边被拖，顶边固定。
        assert!((r.rect.y - 30.0).abs() < 1e-9);
        assert!((r.rect.width / r.rect.height - 2.0).abs() < 1e-9);
    }

    #[test]
    fn ratio_snap_anchors_west_and_north_edges() {
        // 左上角拖动：吸附把西缘再推 +4 → 右 / 下缘固定。
        let r = apply_ratio_snap(RectF::new(40.0, 30.0, 240.0, 120.0), ScaleRole::CornerNW, 4.0, 0.0, false);
        assert_eq!(r.axis, Some(RatioSnapAxis::X));
        assert!((r.rect.width - 236.0).abs() < 1e-9, "{:?}", r.rect);
        assert!((r.rect.right() - 280.0).abs() < 1e-6, "{:?}", r.rect);
        assert!((r.rect.bottom() - 150.0).abs() < 1e-6, "{:?}", r.rect);
    }

    #[test]
    fn ratio_snap_from_center_stays_centered() {
        let r = apply_ratio_snap(RectF::new(40.0, 30.0, 240.0, 120.0), ScaleRole::CornerSE, 8.0, 0.0, true);
        let rect = r.rect;
        assert!((rect.width - 248.0).abs() < 1e-9);
        assert!((rect.width / rect.height - 2.0).abs() < 1e-9);
        assert!((rect.center().0 - 160.0).abs() < 1e-9 && (rect.center().1 - 90.0).abs() < 1e-9);
    }

    #[test]
    fn ratio_snap_gives_up_when_it_would_collapse() {
        // 缩到 1px 以下就整条放弃：不能为了贴线把尺寸吸没。
        let r = apply_ratio_snap(RectF::new(40.0, 30.0, 4.0, 2.0), ScaleRole::CornerNW, 10.0, 0.0, false);
        assert_eq!(r.axis, None);
        assert!((r.rect.width - 4.0).abs() < 1e-9);
    }
}
