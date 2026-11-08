//! P0-4 智能参考线求解。
//!
//! 设计契约（见 `docs/变换系统重构-修改路线.md` §3.3）：
//! * 阈值是 **8 个显示像素**，换算 `threshold_image = 8 / disp_scale`；
//!   `disp_scale = previewImage.width / image_width`（显示像素 / 图像像素），
//!   在 QML 逻辑像素上计，不额外乘 dpr，因此跨 zoom / 跨屏手感一致。
//! * 双轴独立求解，可同时命中、同时画两条线。
//! * 候选数组在手势起手时生成并排序，每帧只做二分 + 常数次比较，
//!   无命中时零堆分配。
//! * 结果只叠加在会话几何上，不写文档、不进历史。
//! * 旋转后的斜边 / 等间距推断线不在 P0 范围。

/// 求解参与的轴。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapAxis {
    X,
    Y,
}

/// 候选来源，偏差相同时按此优先级取胜（数值越小越优先）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    CanvasCenter = 0,
    CanvasEdge = 1,
    LayerEdge = 2,
    LayerCenter = 3,
    Selection = 4,
    /// 拼图咬合：卡进附近图层内容之间的空窗，参考线画在空窗的墙上。
    GapEdge = 5,
}

/// 一条可吸附的轴线。`pos` 是图像坐标；`owner` 是来源标识（图层索引、
/// -1 = 画布、-2 = 选区），只用于诊断，不参与求解。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapCandidate {
    pub axis: SnapAxis,
    pub pos: f64,
    pub kind: SnapKind,
    pub owner: i32,
}

/// 命中后要画的静态洋红轴线。`from`/`to` 是垂直轴方向的图像坐标范围。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuideLine {
    pub axis: SnapAxis,
    pub pos: f64,
    pub from: f64,
    pub to: f64,
    pub kind: SnapKind,
}

/// 提示线旁的距离数值（图像像素，取整后显示）。
#[derive(Debug, Clone, PartialEq)]
pub struct DistanceLabel {
    pub x: f64,
    pub y: f64,
    pub axis: SnapAxis,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SnapResult {
    /// 需要叠加到当前手势几何上的修正量（图像坐标）。
    pub dx: f64,
    pub dy: f64,
    pub guides: Vec<GuideLine>,
    pub labels: Vec<DistanceLabel>,
}

impl SnapResult {
    pub fn is_empty(&self) -> bool {
        self.guides.is_empty() && self.dx == 0.0 && self.dy == 0.0
    }
}

/// `rect_candidates` 生成的六条边/中心线对应的位。
/// `solve_rect` 的 `active_edges` 用同一套位；`scale` 手势只置被拖的那条，
/// 保证对侧锚点坐标恒定（D16）。
pub const EDGE_LEFT: u8 = 1 << 0;
pub const EDGE_CENTER_X: u8 = 1 << 1;
pub const EDGE_RIGHT: u8 = 1 << 2;
pub const EDGE_TOP: u8 = 1 << 3;
pub const EDGE_CENTER_Y: u8 = 1 << 4;
pub const EDGE_BOTTOM: u8 = 1 << 5;
pub const EDGE_ALL: u8 = 0b11_1111;

/// 一个矩形的六条吸附候选（左/中/右、上/中/下）。
pub fn rect_candidates(x: f64, y: f64, w: f64, h: f64, owner: i32) -> Vec<SnapCandidate> {
    let (hx, hy) = (w * 0.5, h * 0.5);
    let layer = |axis, pos, kind| SnapCandidate { axis, pos, kind, owner };
    vec![
        layer(SnapAxis::X, x, SnapKind::LayerEdge),
        layer(SnapAxis::X, x + hx, SnapKind::LayerCenter),
        layer(SnapAxis::X, x + w, SnapKind::LayerEdge),
        layer(SnapAxis::Y, y, SnapKind::LayerEdge),
        layer(SnapAxis::Y, y + hy, SnapKind::LayerCenter),
        layer(SnapAxis::Y, y + h, SnapKind::LayerEdge),
    ]
}

/// 画布的 0 / 中心 / 末端六条线。
pub fn canvas_candidates(w: f64, h: f64) -> Vec<SnapCandidate> {
    let c = |axis, pos, kind| SnapCandidate { axis, pos, kind, owner: -1 };
    vec![
        c(SnapAxis::X, 0.0, SnapKind::CanvasEdge),
        c(SnapAxis::X, w * 0.5, SnapKind::CanvasCenter),
        c(SnapAxis::X, w, SnapKind::CanvasEdge),
        c(SnapAxis::Y, 0.0, SnapKind::CanvasEdge),
        c(SnapAxis::Y, h * 0.5, SnapKind::CanvasCenter),
        c(SnapAxis::Y, h, SnapKind::CanvasEdge),
    ]
}

/// 选区包围盒候选（优先级最低）。
pub fn selection_candidates(x: f64, y: f64, w: f64, h: f64) -> Vec<SnapCandidate> {
    let mut v = rect_candidates(x, y, w, h, -2);
    for c in &mut v {
        c.kind = SnapKind::Selection;
    }
    v
}

/// 起手时调用一次：按 `(axis, pos)` 排序，供每帧二分。
pub fn prepare(statics: &mut Vec<SnapCandidate>) {
    statics.sort_by(|a, b| {
        a.axis
            .cmp(&b.axis)
            .then_with(|| a.pos.total_cmp(&b.pos))
            .then((a.kind as i32).cmp(&(b.kind as i32)))
    });
}

#[inline]
fn kind_rank(k: SnapKind) -> i32 {
    k as i32
}

/// 在已排序的 `statics` 中对某个轴做二分，找与 `pos` 偏差最小的候选。
/// 只检查插入点两侧各一个元素 —— 数组按 pos 有序，最近者必在二者之中。
fn nearest(statics: &[SnapCandidate], axis: SnapAxis, pos: f64) -> Option<(SnapCandidate, f64)> {
    if statics.is_empty() {
        return None;
    }
    let at = statics
        .partition_point(|c| c.axis < axis || (c.axis == axis && c.pos < pos));
    let mut best: Option<(SnapCandidate, f64)> = None;
    for idx in [at.wrapping_sub(1), at, at + 1] {
        let Some(c) = statics.get(idx) else { continue };
        if c.axis != axis {
            continue;
        }
        let delta = c.pos - pos;
        let better = match best {
            None => true,
            Some((bc, bd)) => {
                let a = delta.abs();
                let b = bd.abs();
                a < b - 1e-9
                    || (a - b).abs() <= 1e-9 && kind_rank(c.kind) < kind_rank(bc.kind)
            }
        };
        if better {
            best = Some((*c, delta));
        }
    }
    best
}

/// 求解一个矩形手势的吸附。
///
/// * `moving` = 当前（未吸附）外接框 `(x, y, w, h)`，图像坐标；
/// * `statics` = `prepare()` 排序后的静态候选；
/// * `threshold_display_px` 恒为 8.0（显示像素）；
/// * `disp_scale` = 显示像素 / 图像像素；
/// * `active_edges` = `EDGE_*` 位掩码；`move` 用 `EDGE_ALL`，`scale` 只置被拖边。
pub fn solve_rect(
    moving: (f64, f64, f64, f64),
    statics: &[SnapCandidate],
    threshold_display_px: f64,
    disp_scale: f64,
    active_edges: u8,
) -> SnapResult {
    if active_edges == 0 || !(disp_scale > 0.0) || !disp_scale.is_finite() {
        return SnapResult::default();
    }
    let threshold = (threshold_display_px / disp_scale).abs();
    if !threshold.is_finite() || threshold <= 0.0 {
        return SnapResult::default();
    }
    let (x, y, w, h) = moving;
    let cands = rect_candidates(x, y, w, h, i32::MAX);
    let bits = [
        EDGE_LEFT,
        EDGE_CENTER_X,
        EDGE_RIGHT,
        EDGE_TOP,
        EDGE_CENTER_Y,
        EDGE_BOTTOM,
    ];
    let mut best: [Option<(SnapCandidate, f64)>; 2] = [None, None];
    for (i, c) in cands.iter().enumerate() {
        if active_edges & bits[i] == 0 {
            continue;
        }
        let axis_idx = if c.axis == SnapAxis::X { 0 } else { 1 };
        let Some((hit, delta)) = nearest(statics, c.axis, c.pos) else {
            continue;
        };
        if delta.abs() > threshold {
            continue;
        }
        let take = match best[axis_idx] {
            None => true,
            Some((_, bd)) => {
                delta.abs() < bd.abs() - 1e-9
                    || (delta.abs() - bd.abs()).abs() <= 1e-9
                        && kind_rank(hit.kind) < kind_rank(best[axis_idx].unwrap().0.kind)
            }
        };
        if take {
            best[axis_idx] = Some((hit, delta));
        }
    }
    let mut out = SnapResult::default();
    for (axis_idx, slot) in best.iter().enumerate() {
        let Some((hit, delta)) = slot else { continue };
        if axis_idx == 0 {
            out.dx = *delta;
        } else {
            out.dy = *delta;
        }
        let axis = hit.axis;
        // 轴线跨度 = 移动对象在垂直轴方向的范围；QML 再把它延长到画布边。
        let (from, to) = if axis == SnapAxis::X { (y, y + h) } else { (x, x + w) };
        out.guides.push(GuideLine {
            axis,
            pos: hit.pos,
            from,
            to,
            kind: hit.kind,
        });
        // 距离数值：吸附前移动边到目标轴的距离（实时变化，四舍五入）。
        let dist = delta.abs().round() as i64;
        out.labels.push(DistanceLabel {
            x: if axis == SnapAxis::X { hit.pos } else { x + w * 0.5 },
            y: if axis == SnapAxis::Y { hit.pos } else { y + h * 0.5 },
            axis,
            text: format!("{dist} px"),
        });
    }
    out
}

/// 供 QML 叠加层直接消费的 JSON（不重发结构、不产生额外重绘）。
pub fn guides_json(result: &SnapResult, disp_scale: f64) -> String {
    if result.guides.is_empty() {
        return "[]".to_string();
    }
    let s = if disp_scale > 0.0 { disp_scale } else { 1.0 };
    let mut parts = Vec::with_capacity(result.guides.len());
    for (i, g) in result.guides.iter().enumerate() {
        let vertical = g.axis == SnapAxis::X;
        let label = result.labels.get(i).map(|l| l.text.as_str()).unwrap_or("");
        parts.push(format!(
            r#"{{"v":{},"p":{:.2},"a":{:.2},"b":{:.2},"t":"{}"}}"#,
            vertical,
            g.pos * s,
            g.from * s,
            g.to * s,
            label
        ));
    }
    format!("[{}]", parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statics() -> Vec<SnapCandidate> {
        let mut v = canvas_candidates(1000.0, 800.0);
        v.extend(rect_candidates(200.0, 150.0, 100.0, 80.0, 3));
        prepare(&mut v);
        v
    }

    #[test]
    fn threshold_is_eight_display_pixels_at_any_zoom() {
        // 100%：8 显示像素 = 8 图像像素；400%：= 2 图像像素。
        let st = statics();
        let hit = solve_rect((492.0, 10.0, 20.0, 20.0), &st, 8.0, 1.0, EDGE_ALL);
        // 中心 502 距画布中心 500 只有 2px，比左边 492 的 8px 更近，取最小偏差。
        assert!((hit.dx + 2.0).abs() < 1e-6, "100% 应吸到中心 500");
        let miss = solve_rect((430.0, 10.0, 20.0, 20.0), &st, 8.0, 4.0, EDGE_ALL);
        assert_eq!(miss.dx, 0.0, "400% 时阈值只有 2 图像像素，远处的对象不该吸附");
        let hit25 = solve_rect((498.0, 10.0, 20.0, 20.0), &st, 8.0, 0.25, EDGE_ALL);
        assert!((hit25.dx - 2.0).abs() < 1e-6, "25% 时阈值放宽到 32 图像像素");
    }

    #[test]
    fn both_axes_snap_independently() {
        let st = statics();
        let r = solve_rect((498.0, 398.0, 20.0, 20.0), &st, 8.0, 1.0, EDGE_ALL);
        assert!((r.dx - 2.0).abs() < 1e-6);
        assert!((r.dy - 2.0).abs() < 1e-6);
        assert_eq!(r.guides.len(), 2);
        assert_eq!(r.labels.len(), 2);
    }

    #[test]
    fn scale_only_moves_the_dragged_edge_anchor_stays_put() {
        let st = statics();
        // 只让右边参与：即使左边离画布左沿很近也不该被拉过去。
        let r = solve_rect((2.0, 300.0, 518.0, 20.0), &st, 8.0, 1.0, EDGE_RIGHT);
        assert_eq!(r.dx, 0.0, "左边不许参与求解");
        let r2 = solve_rect((2.0, 300.0, 496.0, 20.0), &st, 8.0, 1.0, EDGE_RIGHT);
        assert!((r2.dx - 2.0).abs() < 1e-6, "右边 498 应吸到 500");
    }

    #[test]
    fn canvas_center_wins_ties_and_disabled_solves_are_empty() {
        let st = statics();
        let r = solve_rect((500.0, 10.0, 20.0, 20.0), &st, 8.0, 1.0, EDGE_ALL);
        assert!(!r.guides.is_empty());
        assert_eq!(r.guides[0].kind, SnapKind::CanvasCenter);
        assert!(solve_rect((500.0, 10.0, 20.0, 20.0), &st, 8.0, 1.0, 0).is_empty());
        assert!(solve_rect((500.0, 10.0, 20.0, 20.0), &st, 8.0, 0.0, EDGE_ALL).is_empty());
    }

    #[test]
    fn json_shape_is_qml_ready() {
        let st = statics();
        let r = solve_rect((498.0, 398.0, 20.0, 20.0), &st, 8.0, 2.0, EDGE_ALL);
        let s = guides_json(&r, 2.0);
        assert!(s.starts_with("[{") && s.ends_with("}]"));
        assert!(s.contains("\"p\":1000.00"), "{s}");
        assert!(guides_json(&SnapResult::default(), 1.0) == "[]");
    }
}
