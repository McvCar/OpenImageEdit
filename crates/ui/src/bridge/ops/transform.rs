//! P0-2b2 / P0-4 / P0-5 统一变换会话对外入口。
//!
//! 设计契约（docs/变换系统重构-修改路线.md §3.3 / §3.4 / §3.5）：
//! * 旧的 `drag_translate_*` / `scale_layer_*` / `rotate_layer_*` 保留原名，
//!   它们是 QML 与 `scripts/` 的字符串契约；本文件在其之上提供**唯一**的
//!   `transform_begin/update/end(mods)` 三件套，QML 手势汇流到这里。
//! * 修饰键只透传位掩码，语义一律在 Rust 求解（D14 / D17）。
//! * 吸附阈值 8 显示像素，求解结果不写文档、不进历史。
//! * 落定统一成 `Document::set_layer_transform`（见各旧 API 内部）。

use super::super::*;
use editcore::snap::{
    canvas_candidates, guides_json, prepare, rect_candidates, selection_candidates, solve_rect,
    SnapResult, EDGE_ALL,
};

// Qt::KeyboardModifier 位（QML 的 mouse.modifiers 原样透传）。
const QT_SHIFT: i32 = 0x0200_0000;
const QT_CTRL: i32 = 0x0400_0000;
const QT_ALT: i32 = 0x0800_0000;
pub(super) const GESTURE_MOVE: i32 = 0;
pub(super) const GESTURE_SCALE: i32 = 1;
pub(super) const GESTURE_ROTATE: i32 = 2;

fn ref_point(index: i32) -> ReferencePoint {
    use ReferencePoint::*;
    match index {
        0 => TopLeft,
        1 => TopCenter,
        2 => TopRight,
        3 => CenterLeft,
        5 => CenterRight,
        6 => BottomLeft,
        7 => BottomCenter,
        8 => BottomRight,
        _ => Center,
    }
}

/// 起手时生成并排序静态候选：画布 0/中心/边 > 其它图层 bounds > 选区。
fn prepare_snap(session: &mut Session, moving: usize) {
    let cw = session.doc.canvas.width as f64;
    let ch = session.doc.canvas.height as f64;
    let mut c = canvas_candidates(cw, ch);
    for (i, layer) in session.doc.layers.iter().enumerate() {
        if i == moving || !layer.visible {
            continue;
        }
        let b = layer.bounds();
        c.extend(rect_candidates(
            b.x as f64,
            b.y as f64,
            b.width as f64,
            b.height as f64,
            i as i32,
        ));
    }
    if let Some(sel) = session.doc.selection.as_ref() {
        let b = sel.base.bounds;
        c.extend(selection_candidates(
            b.x as f64,
            b.y as f64,
            b.width as f64,
            b.height as f64,
        ));
    }
    prepare(&mut c);
    session.snap_statics = c;
    session.snap = SnapResult::default();
}

/// 本次手势是否应该求解吸附（D17 手势分流 + D16 开关）。
fn snap_allowed(enabled: bool, gesture: i32, mods: i32) -> bool {
    // move + Ctrl = PS 的「临时禁用吸附」；scale/rotate 的 Ctrl 留给
    // P1-A 的斜切 / 透视，不在这里争键。
    enabled && !(gesture == GESTURE_MOVE && mods & QT_CTRL != 0)
}

impl qobject::Bridge {
    /// 统一会话快照（JSON）。供 `--probe-session` 与 `--script=transform-state`
    pub fn transform_state(self: Pin<&mut Self>) -> QString {
        let text = self
            .session
            .borrow()
            .as_ref()
            .map(|s| s.transform_state_json())
            .unwrap_or_else(|| r#"{"active":false,"no_document":true}"#.to_string());
        QString::from(text.as_str())
    }

    /// 丢弃进行中的变换手势（Esc / 会话被接管）。移动手势的 offset 是实时
    /// 写进文档的，必须回滚；旋转 / 缩放的文档像素本来就没动过。
    pub fn transform_abort(mut self: Pin<&mut Self>) -> bool {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return false;
            };
            let Some(ts) = session.transform.take() else {
                return false;
            };
            if let TransformSession::Move { index, start, start_center, .. } = ts {
                if let Some(layer) = session.doc.layers.get_mut(index) {
                    layer.offset = start;
                    // 与 drag_translate_update 配对的回滚：手势期间 center 也跟着
                    // 位移了，只还原 offset 会让框和内容错开。
                    if let (Some(c), Some(t)) = (start_center, layer.transform.as_mut()) {
                        t.center = c;
                    }
                }
            }
            session.snap = SnapResult::default();
            session.gesture = -1;
            session.gpu_end();
        }
        self.as_mut().set_gpu_live(false);
        self.as_mut().refresh_preview();
        true
    }

    /// QML 手势透传修饰键与显示比例。单独一个入口是为了不破坏旧的
    /// `*_begin/update` 签名（脚本别名依赖），也避免每帧重解析字符串。
    pub fn transform_mods(mut self: Pin<&mut Self>, mods: i32, disp_scale: f64) {
        let enabled = *self.snap_enabled();
        let clear = {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            session.mods = mods;
            session.disp_scale = if disp_scale > 0.0 { disp_scale } else { 1.0 };
            // Ctrl 按下 / 关吸附时，立刻撤掉上一帧的提示线。
            !snap_allowed(enabled, session.gesture, mods)
        };
        if clear {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                session.snap = SnapResult::default();
            }
        }
    }

    /// 手势起手：记录起点、预生成候选、把旧 live 会话拉起来。
    /// `gesture`: 0=move 1=scale 2=rotate；`px/py` 为图像坐标。
    pub fn transform_begin(
        mut self: Pin<&mut Self>,
        gesture: i32,
        index: i32,
        px: f64,
        py: f64,
        mods: i32,
        disp_scale: f64,
    ) -> bool {
        let enabled = *self.snap_enabled();
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return false;
            };
            session.clear_live_gestures();
            session.mods = mods;
            session.disp_scale = if disp_scale > 0.0 { disp_scale } else { 1.0 };
            session.gesture = gesture;
            session.gesture_start = (px, py);
            session.gesture_rect = session
                .doc
                .layers
                .get(index.max(0) as usize)
                .map(|l| {
                    let b = l.bounds();
                    (b.x as f64, b.y as f64, b.width as f64, b.height as f64)
                })
                .unwrap_or((0.0, 0.0, 0.0, 0.0));
            session.snap = SnapResult::default();
            if snap_allowed(enabled, gesture, mods) && gesture != GESTURE_ROTATE {
                prepare_snap(session, index.max(0) as usize);
            } else {
                session.snap_statics.clear();
            }
        }
        // 旧 API 负责建立 live 预览态（文档像素不动），三件套只统一状态层。
        match gesture {
            GESTURE_SCALE => self.as_mut().scale_layer_begin(),
            GESTURE_ROTATE => {
                self.as_mut().rotate_layer_begin(index, true);
            }
            GESTURE_MOVE => {
                self.as_mut().drag_translate_begin();
                // Tier 1：只对「移动」启用两纹理快路径（缩放 / 旋转涉及重采样，
                // 落定锐度一致性留 P1-A 统一采样器后再接）。生成失败或命中
                // 回退条件时自动走原 CPU 路径，行为不变。
                let ix = index.max(0) as usize;
                let dpr = self.dpr.get();
                let (on, su, au, rect) = {
                    let mut guard = self.session.borrow_mut();
                    let Some(session) = guard.as_mut() else {
                        return false;
                    };
                    let on = session.gpu_begin(ix, session.disp_scale, dpr);
                    (
                        on,
                        session.gpu_static_url.clone(),
                        session.gpu_active_url.clone(),
                        session.gpu_rect_json(),
                    )
                };
                if on {
                    self.as_mut().set_gpu_static_url(QString::from(su.as_str()));
                    self.as_mut().set_gpu_active_url(QString::from(au.as_str()));
                    self.as_mut().set_gpu_rect_json(QString::from(rect.as_str()));
                    self.as_mut().set_gpu_live(true);
                } else {
                    self.as_mut().set_gpu_live(false);
                }
            }
            _ => return false,
        }
        true
    }

    /// 手势推进。`move` 由这里直接驱动预览（吸附修正量叠加到位移上）；
    /// `scale` 的修正量由 `transform_snap_rect` 交给 QML 叠加到被拖边；
    /// `rotate` 只更新 mods（P0 不吸旋转斜边）。
    pub fn transform_update(
        mut self: Pin<&mut Self>,
        gesture: i32,
        px: f64,
        py: f64,
        mods: i32,
        disp_scale: f64,
    ) -> bool {
        let enabled = *self.snap_enabled();
        let (dx, dy, allowed) = {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return false;
            };
            session.mods = mods;
            session.disp_scale = if disp_scale > 0.0 { disp_scale } else { 1.0 };
            let allowed = snap_allowed(enabled, gesture, mods);
            if !allowed {
                session.snap = SnapResult::default();
            }
            (
                px - session.gesture_start.0,
                py - session.gesture_start.1,
                allowed,
            )
        };
        if gesture == GESTURE_MOVE {
            let (sx, sy) = if allowed {
                // 用「起手外接框 + 总位移」求解，而不是读文档当前 offset：
                // move 期间 drag_translate_update 会实时改 offset，拿当前值
                // 再加一遍总位移会把吸附线越推越远。
                let mut guard = self.session.borrow_mut();
                let Some(session) = guard.as_mut() else {
                    return false;
                };
                let (x, y, w, h) = session.gesture_rect;
                let r = solve_rect(
                    (x + dx, y + dy, w, h),
                    &session.snap_statics,
                    8.0,
                    session.disp_scale,
                    EDGE_ALL,
                );
                let delta = (r.dx, r.dy);
                session.snap = r;
                delta
            } else {
                (0.0, 0.0)
            };
            self.as_mut()
                .drag_translate_update((dx + sx).round() as i32, (dy + sy).round() as i32);
        }
        true
    }

    /// 八向缩放 / 数值面板专用：按当前外接框求解吸附，返回修正量 JSON。
    /// 坐标一律是**图像坐标**；QML 负责显示↔图像换算，只让被拖边参与
    /// （`edges` 用 `EDGE_*` 位），保证对侧锚点恒定。
    pub fn transform_snap_rect(
        mut self: Pin<&mut Self>,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        edges: i32,
        mods: i32,
        disp_scale: f64,
    ) -> QString {
        let enabled = *self.snap_enabled();
        let mut guard = self.session.borrow_mut();
        let Some(session) = guard.as_mut() else {
            return QString::from(r#"{"dx":0,"dy":0}"#);
        };
        session.disp_scale = if disp_scale > 0.0 { disp_scale } else { 1.0 };
        if !snap_allowed(enabled, GESTURE_SCALE, mods) {
            session.snap = SnapResult::default();
            return QString::from(r#"{"dx":0,"dy":0}"#);
        }
        // 脚本探针可单独调用（没有手势起手），此时补一次候选预生成。
        if session.snap_statics.is_empty() {
            let ix = (*self.selected_layer()).max(0) as usize;
            prepare_snap(session, ix);
        }
        let active = if edges <= 0 {
            EDGE_ALL
        } else {
            (edges as u8) & EDGE_ALL
        };
        let r = solve_rect(
            (x, y, w, h),
            &session.snap_statics,
            8.0,
            session.disp_scale,
            active,
        );
        let text = format!(r#"{{"dx":{:.4},"dy":{:.4}}}"#, r.dx, r.dy);
        session.snap = r;
        QString::from(text.as_str())
    }

    /// 手势收尾：清提示线、结束统一会话。`commit=false` 走 transform_abort。
    pub fn transform_end(mut self: Pin<&mut Self>, commit: bool, mods: i32) -> bool {
        {
            let mut guard = self.session.borrow_mut();
            if let Some(session) = guard.as_mut() {
                session.mods = mods;
                session.snap = SnapResult::default();
                session.snap_statics.clear();
                session.gesture = -1;
                session.gpu_end();
            }
        }
        self.as_mut().set_gpu_live(false);
        if !commit {
            return self.as_mut().transform_abort();
        }
        // 真正的落定仍由旧 *_end 完成（它们内部已统一到 SetLayerTransform），
        // 这样 QML 只需在 release 时按手势类型再调一次对应收尾。
        self.as_mut().refresh_preview();
        true
    }

    /// P0-5 数值面板写回：一次调用 = 一条 SetLayerTransform 历史。
    /// X/Y/W/H 都是变换后外接框（D10），写 W/H 时 rotation 保持不变（R10）。
    #[allow(clippy::too_many_arguments)]
    pub fn transform_set_params(
        mut self: Pin<&mut Self>,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        rotation: f64,
        flip_x: bool,
        flip_y: bool,
        lock_ratio: bool,
        ref_point_index: i32,
    ) -> bool {
        let Some(ix) = self.selected_guard(*self.selected_layer()) else {
            return false;
        };
        let pivot = ref_point(ref_point_index);
        let result = {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return false;
            };
            let Some(layer) = session.doc.layers.get(ix) else {
                return false;
            };
            let mut t = layer.transform.clone().unwrap_or_else(|| {
                LayerTransform::from_offset(layer.image.clone(), layer.offset.0, layer.offset.1)
            });
            let solve = if lock_ratio {
                BoundsSolve::Uniform
            } else {
                BoundsSolve::SingleAxis
            };
            // 顺序固定：先位置（纯平移），再尺寸（按当前角度反解 scale），
            // 最后角度；每一步的 pivot 都是同一个参考点，避免互相漂移。
            let ok = t.set_panel_value_with_solve(Field::X, x, pivot, solve)
                && t.set_panel_value_with_solve(Field::Y, y, pivot, solve)
                && t.set_panel_value_with_solve(Field::Width, w, pivot, solve)
                && t.set_panel_value_with_solve(Field::Height, h, pivot, solve)
                && t.set_panel_value_with_solve(Field::Rotation, rotation, pivot, solve);
            if !ok {
                return false;
            }
            t.flip = (flip_x, flip_y);
            session.doc.set_layer_transform(ix, Some(t))
        };
        if let Err(e) = result {
            self.as_mut().set_status(QString::from(e.to_string().as_str()));
            return false;
        }
        self.as_mut().refresh_preview();
        self.as_mut().set_dirty(true);
        true
    }

    /// 诊断：当前参考线（含 show_guides=false 时仍返回求解结果）。
    pub fn guides_state(self: Pin<&mut Self>) -> QString {
        let text = self
            .session
            .borrow()
            .as_ref()
            .map(|s| guides_json(&s.snap, s.disp_scale))
            .unwrap_or_else(|| "[]".to_string());
        QString::from(text.as_str())
    }
}

// 让 mods 常量在其它 ops 文件里可复用（P1-A 斜切接线时用）。
pub(super) const MOD_SHIFT: i32 = QT_SHIFT;
pub(super) const MOD_CTRL: i32 = QT_CTRL;
pub(super) const MOD_ALT: i32 = QT_ALT;

impl qobject::Bridge {
    /// ★P1-A（D14 / §9.1）：缩放手势语义求解 —— **修饰键语义只在这里**。
    ///
    /// QML 只透传起手外接框（图像坐标）、被拖手柄 role、光标位移与修饰键
    /// 位掩码；锁比例（角点+Shift）、从中心（Alt）、斜切（Ctrl+拖边）与
    /// 八向吸附全部在这一个入口解出，返回解算后的框 + 翻转 + 斜切量 JSON。
    /// 斜切手势跳过吸附（scale+Ctrl 的 Ctrl 语义就是斜切，D17）。
    #[allow(clippy::too_many_arguments)]
    pub fn transform_solve_scale(
        mut self: Pin<&mut Self>,
        sx: f64,
        sy: f64,
        sw: f64,
        sh: f64,
        role: QString,
        dx: f64,
        dy: f64,
        mods: i32,
        disp_scale: f64,
    ) -> QString {
        let Some(r) = editcore::ScaleRole::parse(role.to_string().as_str()) else {
            return QString::from(r#"{"error":"role"}"#);
        };
        let enabled = *self.snap_enabled();
        let shift = mods & QT_SHIFT != 0;
        let alt = mods & QT_ALT != 0;
        let ctrl = mods & QT_CTRL != 0;
        let ix = (*self.selected_layer()).max(0) as usize;
        let (x, y, w, h, fx, fy, kx, ky, cx, cy) = {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return QString::from(r#"{"error":"no_document"}"#);
            };
            session.disp_scale = if disp_scale > 0.0 { disp_scale } else { 1.0 };
            // 斜切换算需要图层当前 (sx, sy)：identity 层恒 (1,1)。
            let (lsx, lsy) = session
                .doc
                .layers
                .get(ix)
                .map(|l| {
                    l.transform
                        .as_ref()
                        .map(|t| (t.scale.0.max(1e-6), t.scale.1.max(1e-6)))
                        .unwrap_or((1.0, 1.0))
                })
                .unwrap_or((1.0, 1.0));
            let solved = editcore::solve_scale_gesture(
                editcore::RectF::new(sx, sy, sw, sh),
                r,
                dx,
                dy,
                shift,
                alt,
                ctrl,
                (lsx, lsy),
            );
            let (mut x, mut y) = (solved.x, solved.y);
            let (mut w, mut h) = (solved.width, solved.height);
            // 非斜切手势才吸附（Ctrl 的语义在 scale 手势里是斜切）。
            if !ctrl && enabled {
                if session.snap_statics.is_empty() {
                    prepare_snap(session, ix);
                }
                let res = solve_rect(
                    (x, y, w, h),
                    &session.snap_statics,
                    8.0,
                    session.disp_scale,
                    r.edge_bits(),
                );
                let (adx, ady) = (res.dx, res.dy);
                session.snap = res;
                if adx != 0.0 {
                    if r.edge_bits() & 4 != 0 {
                        w += adx;
                    } else if r.edge_bits() & 1 != 0 {
                        x += adx;
                        w -= adx;
                    }
                }
                if ady != 0.0 {
                    if r.edge_bits() & 32 != 0 {
                        h += ady;
                    } else if r.edge_bits() & 8 != 0 {
                        y += ady;
                        h -= ady;
                    }
                }
            } else if !enabled {
                session.snap = SnapResult::default();
            }
            (
                x,
                y,
                w.max(1.0),
                h.max(1.0),
                solved.flip.0,
                solved.flip.1,
                solved.skew.0,
                solved.skew.1,
                solved.center_delta.0,
                solved.center_delta.1,
            )
        };
        QString::from(
            format!(
                r#"{{"x":{x:.4},"y":{y:.4},"w":{w:.4},"h":{h:.4},"fx":{},"fy":{},"kx":{kx:.6},"ky":{ky:.6},"cx":{cx:.4},"cy":{cy:.4}}}"#,
                fx as i32, fy as i32
            )
            .as_str(),
        )
    }
}
