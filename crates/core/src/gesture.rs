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

    fn corner(self) -> bool {
        !matches!(self, Self::EdgeW | Self::EdgeE | Self::EdgeN | Self::EdgeS)
    }

    fn west(self) -> bool {
        matches!(self, Self::EdgeW | Self::CornerNW | Self::CornerSW)
    }

    fn north(self) -> bool {
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
}
