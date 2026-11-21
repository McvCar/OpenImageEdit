// crates/ui/src/bridge/ops/groups.rs
//! 图层分组的桥接入口。
//!
//! 这里刻意区分两条刷新路径：
//! * 编组 / 解组 / 重命名 / 折叠 / 组锁定只改组织元数据 → `refresh_layers_only`
//!   或 `refresh_groups_only`，**不重建预览**，保真 / 校准态不会因为「分了个组」
//!   就切回按层重建的画面；
//! * 整组显隐会改成员 `visible`（合成只认成员）→ 走 `refresh_preview`。
//!
//! 普通编辑模式（一张图 + 若干随手建的层）和 PSD 分层编辑态走的是同一套
//! `Document` 方法，没有任何「只有 PSD 才能编组」的门槛。
use super::super::*;

impl qobject::Bridge {
    /// 分组命令统一的前置检查：后台加载图层期间一律禁用编辑工具。
    fn group_edit_allowed(&self) -> bool {
        !*self.editing_locked() && self.session.borrow().is_some()
    }

    fn group_hint(mut self: Pin<&mut Self>, text: &str) {
        self.as_mut().set_status(QString::from(text));
    }

    /// 把当前多选收进一个新组（≥2 个普通层，背景和已分组的层不参与）。
    pub fn group_selected_layers(mut self: Pin<&mut Self>) -> bool {
        if !self.group_edit_allowed() {
            return false;
        }
        let indices = self.selection_indices();
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err(editcore::EditError::LayerIndex(0)),
                Some(session) => session.doc.group_layers(&indices, None),
            }
        };
        match result {
            Ok(gid) => {
                self.as_mut().refresh_layers_only();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"group-create\",\"id\":{gid},\
                     \"members\":{}}}",
                    indices.len()
                );
                true
            }
            Err(e) => {
                let text = match e {
                    editcore::EditError::GroupNeedsTwoLayers => {
                        "编组要先选中两个以上普通图层：Ctrl+点击图层行，背景层不参与，已在组内的层要先解组".to_string()
                    }
                    other => other.to_string(),
                };
                self.as_mut().group_hint(&text);
                false
            }
        }
    }

    /// 解散当前选中层所属的所有分组（图层本身保留）。
    pub fn ungroup_selected_layers(mut self: Pin<&mut Self>) -> bool {
        if !self.group_edit_allowed() {
            return false;
        }
        let indices = self.selection_indices();
        let result: Result<usize, String> = {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return false;
            };
            let mut gids: Vec<u32> = Vec::new();
            for i in indices {
                if let Some(gid) = session.doc.group_of(i) {
                    if !gids.contains(&gid) {
                        gids.push(gid);
                    }
                }
            }
            // 嵌套时只解散「最外层」那批：否则点父组解组会把子组一起拆掉，
            // 与 PS 的 Ctrl+Shift+G 语义（只解一层）不一致。
            let all = gids.clone();
            gids.retain(|gid| {
                let mut next = session.doc.find_group(*gid).and_then(|g| g.parent);
                let mut guard = 0;
                while let Some(pid) = next {
                    guard += 1;
                    if guard > 64 {
                        break;
                    }
                    if all.contains(&pid) {
                        return false;
                    }
                    next = session.doc.find_group(pid).and_then(|g| g.parent);
                }
                true
            });
            let mut done = 0usize;
            let mut failure: Option<String> = None;
            for gid in &gids {
                match session.doc.ungroup(*gid) {
                    Ok(()) => done += 1,
                    Err(e) => {
                        failure = Some(e.to_string());
                        break;
                    }
                }
            }
            match failure {
                Some(e) => Err(e),
                None => Ok(done),
            }
        };
        match result {
            Ok(0) => {
                self.as_mut()
                    .group_hint("当前选中的图层不属于任何分组");
                false
            }
            Ok(n) => {
                self.as_mut().refresh_layers_only();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"group-remove\",\"count\":{n}}}"
                );
                true
            }
            Err(message) => {
                self.as_mut().group_hint(&message);
                false
            }
        }
    }

    /// 解散指定分组（组行右键 / 按钮入口）。
    pub fn ungroup_group(mut self: Pin<&mut Self>, group_id: i32) -> bool {
        if !self.group_edit_allowed() || group_id <= 0 {
            return false;
        }
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err(editcore::EditError::LayerIndex(0)),
                Some(session) => session.doc.ungroup(group_id as u32),
            }
        };
        match result {
            Ok(()) => {
                self.as_mut().refresh_layers_only();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"group-remove\",\"id\":{group_id}}}"
                );
                true
            }
            Err(e) => {
                self.as_mut().group_hint(&e.to_string());
                false
            }
        }
    }

    /// 点组标题行：选中整组（含嵌套子组的成员），主选中层为组内最上层。
    pub fn select_group(mut self: Pin<&mut Self>, group_id: i32) {
        if group_id <= 0 {
            return;
        }
        let members = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => session.doc.group_subtree_layers(group_id as u32),
                None => Vec::new(),
            }
        };
        let Some(&primary) = members.last() else {
            return;
        };
        self.as_mut().set_selected_layer(primary as i32);
        self.as_mut().set_selection_indices(&members);
        println!(
            "{{\"phase\":\"ui\",\"event\":\"group-select\",\"id\":{group_id},\
             \"count\":{}}}",
            members.len()
        );
    }

    /// 拖动图层进出分组。z 序与像素都不变，但两侧组的状态可能改变画面：
    /// * 隐藏组：core 已经把成员的 `visible` 跟着改了（可撤销）；
    /// * 隔离组（非穿透 / 组不透明度 < 100%）：合成时整组先画到缓冲再
    ///   按组模式盖回，移进移出都必须重出预览帧，否则列表变了画面没变。
    ///
    /// 两者都没有时才走轻量刷新，保真 / 校准态不被无谓地切掉。
    pub fn move_layer_to_group(mut self: Pin<&mut Self>, index: i32, group_id: i32) {
        if !self.group_edit_allowed() || index < 0 {
            return;
        }
        if group_id < 0 {
            self.as_mut().group_hint("目标分组无效");
            return;
        }
        let target = if group_id == 0 { None } else { Some(group_id as u32) };
        let (result, needs_preview) = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => (
                    Err(editcore::EditError::LayerIndex(index as usize).to_string()),
                    false,
                ),
                Some(session) => {
                    let idx = index as usize;
                    let src_flags = session.doc.group_state_flags(session.doc.group_of(idx));
                    let dst_flags = session.doc.group_state_flags(target);
                    let visible_before = session
                        .doc
                        .layers
                        .get(idx)
                        .map(|l| l.visible)
                        .unwrap_or(false);
                    let result = session
                        .doc
                        .set_layer_group(idx, target)
                        .map(|()| session.dirty = true)
                        .map_err(|e| e.to_string());
                    let visible_after = session
                        .doc
                        .layers
                        .get(idx)
                        .map(|l| l.visible)
                        .unwrap_or(false);
                    let needs_preview = visible_before != visible_after
                        || src_flags.effects
                        || dst_flags.effects;
                    (result, needs_preview)
                }
            }
        };
        if let Err(e) = result {
            self.as_mut().group_hint(&e);
            return;
        }
        if needs_preview {
            self.as_mut().refresh_preview();
        } else {
            self.as_mut().refresh_layers_only();
        }
        println!(
            "{{\"phase\":\"ui\",\"event\":\"layer-move-group\",\"index\":{index},\
             \"group\":{group_id}}}"
        );
    }

    /// 折叠 / 展开：纯视图状态，不进撤销栈、不动像素。
    pub fn set_group_collapsed(mut self: Pin<&mut Self>, group_id: i32, collapsed: bool) {
        if group_id <= 0 {
            return;
        }
        let ok = {
            let mut guard = self.session.borrow_mut();
            guard.as_mut().is_some_and(|session| {
                session
                    .doc
                    .set_group_collapsed_view(group_id as u32, collapsed)
                    .is_ok()
            })
        };
        if ok {
            self.as_mut().refresh_groups_only();
            println!(
                "{{\"phase\":\"ui\",\"event\":\"group-collapse\",\"id\":{group_id},\
                 \"collapsed\":{collapsed}}}"
            );
        }
    }

    /// 展开目标层所在的全部折叠祖先组（读文档真实状态，不看 UI 快照），
    /// 并同步刷新 groups_json / layers_json 给面板。返回是否改动过。
    pub fn reveal_layer_groups(mut self: Pin<&mut Self>, index: i32) -> bool {
        if index < 0 {
            return false;
        }
        let expanded = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => false,
                Some(session) => {
                    !session.doc.expand_layer_ancestors(index as usize).is_empty()
                }
            }
        };
        if expanded {
            self.as_mut().refresh_groups_only();
        }
        expanded
    }

    pub fn rename_group(mut self: Pin<&mut Self>, group_id: i32, name: &QString) {
        if !self.group_edit_allowed() || group_id <= 0 {
            return;
        }
        let name = name.to_string();
        if name.trim().is_empty() {
            return;
        }
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err(editcore::EditError::LayerIndex(0)),
                Some(session) => session.doc.rename_group(group_id as u32, name.trim()),
            }
        };
        if let Err(e) = result {
            self.as_mut().group_hint(&e.to_string());
            return;
        }
        self.as_mut().refresh_layers_only();
        println!(
            "{{\"phase\":\"ui\",\"event\":\"group-rename\",\"id\":{group_id}}}"
        );
    }

    /// 整组显隐：改成员 visible，会重建预览（画面确实变了）。
    pub fn set_group_visible(mut self: Pin<&mut Self>, group_id: i32, visible: bool) {
        if !self.group_edit_allowed() || group_id <= 0 {
            return;
        }
        let viewport = *self.viewport.borrow();
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err(editcore::EditError::LayerIndex(0)),
                Some(session) => {
                    let r = session.doc.set_group_visible(group_id as u32, visible);
                    if r.is_ok() {
                        session.refit_if_needed(viewport);
                        session.dirty = true;
                    }
                    r
                }
            }
        };
        if let Err(e) = result {
            self.as_mut().group_hint(&e.to_string());
            return;
        }
        self.as_mut().refresh_preview();
        println!(
            "{{\"phase\":\"ui\",\"event\":\"group-visible\",\"id\":{group_id},\
             \"visible\":{visible}}}"
        );
    }

    pub fn set_group_locked(mut self: Pin<&mut Self>, group_id: i32, locked: bool) {
        if !self.group_edit_allowed() || group_id <= 0 {
            return;
        }
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err(editcore::EditError::LayerIndex(0)),
                Some(session) => session.doc.set_group_locked(group_id as u32, locked),
            }
        };
        if let Err(e) = result {
            self.as_mut().group_hint(&e.to_string());
            return;
        }
        self.as_mut().refresh_layers_only();
        println!(
            "{{\"phase\":\"ui\",\"event\":\"group-lock\",\"id\":{group_id},\
             \"locked\":{locked}}}"
        );
    }

    /// 把整组拖进另一组（target_id=0 → 移回顶层）。parent 改动 + 子树
    /// 显隐随环境切换（core 里与单层 set_layer_group 同语义），走撤销栈。
    /// 组环境（隔离 / 不透明度 / 隐藏）任何一侧都可能改变合成，统一
    /// refresh_preview 兜住，不做「轻量刷新」判断。
    pub fn move_group_to_group(mut self: Pin<&mut Self>, group_id: i32, target_id: i32) {
        if !self.group_edit_allowed() || group_id <= 0 || target_id < 0 || group_id == target_id
        {
            return;
        }
        let target = if target_id == 0 {
            None
        } else {
            Some(target_id as u32)
        };
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no session".to_string()),
                Some(session) => session
                    .doc
                    .move_group_to_group(group_id as u32, target)
                    .map(|()| session.dirty = true)
                    .map_err(|e| e.to_string()),
            }
        };
        if let Err(e) = result {
            self.as_mut().group_hint(&e);
            return;
        }
        self.as_mut().refresh_preview();
        println!(
            "{{\"phase\":\"ui\",\"event\":\"group-move\",\"id\":{group_id},\
             \"target\":{target_id}}}"
        );
    }

    // ---- 组混合模式 / 不透明度 / 穿透（P0）---------------------------
    pub fn group_blend_mode(self: Pin<&mut Self>, group_id: i32) -> i32 {
        self.session
            .borrow()
            .as_ref()
            .and_then(|s| s.doc.find_group(group_id.max(0) as u32))
            .map(|g| g.blend.index())
            .unwrap_or(0)
    }

    pub fn set_group_blend_mode(mut self: Pin<&mut Self>, group_id: i32, mode: i32) {
        let Some(to) = editcore::BlendMode::from_index(mode) else {
            self.as_mut()
                .set_status(QString::from("未知的混合模式索引"));
            return;
        };
        if !self.group_edit_allowed() || group_id <= 0 {
            return;
        }
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err(editcore::EditError::GroupNotFound(group_id as u32).to_string()),
                Some(session) => session
                    .doc
                    .set_group_blend_isolating(group_id as u32, to)
                    .map(|()| session.dirty = true)
                    .map_err(|e| e.to_string()),
            }
        };
        if let Err(e) = result {
            self.as_mut().group_hint(&e);
            return;
        }
        self.as_mut().refresh_preview();
        println!(
            "{{\"phase\":\"ui\",\"event\":\"group-blend\",\"id\":{group_id},\
             \"mode\":{mode}}}"
        );
    }

    pub fn group_opacity(self: Pin<&mut Self>, group_id: i32) -> f32 {
        self.session
            .borrow()
            .as_ref()
            .and_then(|s| s.doc.find_group(group_id.max(0) as u32))
            .map(|g| g.opacity)
            .unwrap_or(1.0)
    }

    pub fn set_group_opacity(mut self: Pin<&mut Self>, group_id: i32, opacity: f32) {
        if !self.group_edit_allowed() || group_id <= 0 {
            return;
        }
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err(editcore::EditError::GroupNotFound(group_id as u32).to_string()),
                Some(session) => session
                    .doc
                    .set_group_opacity(group_id as u32, opacity)
                    .map(|()| session.dirty = true)
                    .map_err(|e| e.to_string()),
            }
        };
        if let Err(e) = result {
            self.as_mut().group_hint(&e);
            return;
        }
        self.as_mut().refresh_preview();
    }

    pub fn group_pass_through(self: Pin<&mut Self>, group_id: i32) -> bool {
        self.session
            .borrow()
            .as_ref()
            .and_then(|s| s.doc.find_group(group_id.max(0) as u32))
            .map(|g| g.pass_through)
            .unwrap_or(true)
    }

    pub fn set_group_pass_through(mut self: Pin<&mut Self>, group_id: i32, pass: bool) {
        if !self.group_edit_allowed() || group_id <= 0 {
            return;
        }
        let result = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err(editcore::EditError::GroupNotFound(group_id as u32).to_string()),
                Some(session) => session
                    .doc
                    .set_group_pass_through(group_id as u32, pass)
                    .map(|()| session.dirty = true)
                    .map_err(|e| e.to_string()),
            }
        };
        if let Err(e) = result {
            self.as_mut().group_hint(&e);
            return;
        }
        self.as_mut().refresh_preview();
        println!(
            "{{\"phase\":\"ui\",\"event\":\"group-passthrough\",\"id\":{group_id},\
             \"pass\":{pass}}}"
        );
    }
}
