// P0-2b1：统一变换会话的对外入口。
//
// 旧的 `scale_layer_*` / `drag_translate_*` / `rotate_layer_*` 全部保留原名
// （QML 与 scripts/ 里的字符串契约不能动），但内部已经改为读写同一个
// `Session::transform`（见 session.rs 的 TransformSession）。这里提供两个
// 会话级操作：诊断快照与统一取消。
use super::super::*;

impl qobject::Bridge {
    /// 统一会话快照（JSON）。供 `--probe-session` 与 `--script=transform-state`
    /// 断言「同一时刻最多只有一个手势、且落定后没有残留」。
    pub fn transform_state(self: Pin<&mut Self>) -> QString {
        let text = self
            .session
            .borrow()
            .as_ref()
            .map(|s| s.transform_state_json())
            .unwrap_or_else(|| r#"{"active":false,"no_document":true}"#.to_string());
        QString::from(text.as_str())
    }

    /// 丢弃进行中的变换手势。缩放 / 旋转的临时预览帧直接随会话消失；
    /// 移动手势的 offset 是实时写进文档的，必须回滚到起手位置，等价于
    /// `drag_translate_end(false)`（不产生历史）。
    pub fn transform_abort(mut self: Pin<&mut Self>) -> bool {
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return false;
            };
            let Some(ts) = session.transform.take() else {
                return false;
            };
            if let TransformSession::Move { index, start, .. } = ts {
                if let Some(layer) = session.doc.layers.get_mut(index) {
                    layer.offset = start;
                }
            }
        }
        self.as_mut().refresh_preview();
        true
    }
}
