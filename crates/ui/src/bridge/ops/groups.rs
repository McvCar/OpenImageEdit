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

    /// 点组标题行：选中整组，主选中层为组内最上层（z 序最高）。
    pub fn select_group(mut self: Pin<&mut Self>, group_id: i32) {
        if group_id <= 0 {
            return;
        }
        let members = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => session.doc.group_members(group_id as u32),
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
}
