// crates/core/src/group_ops.rs
//! 图层分组：只改「组织元数据」，不动 z 序、不动像素。
//!
//! 设计约束（见 docs）：组**不是** `LayerContent` 的新变体，也不进
//! `Document::layers`。合成、预览缓存、PSD 写出都只遍历 `layers`，因此这里
//! 所有操作对渲染零影响；成员关系通过 `Layer::group` 反查，删层 / 撤销 /
//! 整层替换时归属自动跟着走，不需要维护第二份成员表。
use super::*;

/// 一个组（含其祖先链）对外表现出的状态。拖动图层进出组时要按它同步
/// 成员的显隐与预览刷新：组隐藏 = 成员必须跟着隐藏；组被隔离
/// （非穿透 / 不透明度 < 100%）= 渲染会变，必须重出预览帧。
#[derive(Debug, Clone, Copy, Default)]
pub struct GroupStateFlags {
    pub hidden: bool,
    pub effects: bool,
    /// 最近一个处于隐藏状态的组 id，用来读 `hidden_snapshot` 还原显隐。
    pub hidden_group: Option<GroupId>,
}

impl Document {
    pub fn find_group(&self, id: GroupId) -> Option<&LayerGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// 从给定组往上走完整祖先链，收集隐藏 / 隔离状态。带环路保护。
    pub fn group_state_flags(&self, gid: Option<GroupId>) -> GroupStateFlags {
        let mut flags = GroupStateFlags::default();
        let mut cur = gid;
        let mut guard = 0;
        while let Some(id) = cur {
            let Some(group) = self.find_group(id) else {
                break;
            };
            if !group.visible {
                flags.hidden = true;
                if flags.hidden_group.is_none() {
                    flags.hidden_group = Some(id);
                }
            }
            if group.isolated() {
                flags.effects = true;
            }
            cur = group.parent;
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        flags
    }

    /// 组成员在文档栈里的下标（升序，即 z 序从低到高）。
    pub fn group_members(&self, id: GroupId) -> Vec<usize> {
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.group == Some(id))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn group_of(&self, index: usize) -> Option<GroupId> {
        self.layers.get(index)?.group
    }

    /// 直接的子组（不含孙子）。
    pub fn child_groups(&self, id: GroupId) -> Vec<GroupId> {
        self.groups
            .iter()
            .filter(|g| g.parent == Some(id))
            .map(|g| g.id)
            .collect()
    }

    /// 组的深度（顶层 = 0）。带环路保护，坏数据不会把面板递归打穿。
    pub fn group_depth(&self, id: GroupId) -> usize {
        let mut depth = 0usize;
        let mut next = self.find_group(id).and_then(|g| g.parent);
        while let Some(pid) = next {
            depth += 1;
            if depth > 64 {
                break;
            }
            next = self.find_group(pid).and_then(|g| g.parent);
        }
        depth
    }

    /// 组及其所有后代组（不含自己）。
    pub fn descendant_groups(&self, id: GroupId) -> Vec<GroupId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        let mut guard = 0;
        while let Some(next) = stack.pop() {
            guard += 1;
            if guard > 4096 {
                break;
            }
            for child in self.child_groups(next) {
                if !out.contains(&child) && child != id {
                    out.push(child);
                    stack.push(child);
                }
            }
        }
        out
    }

    /// 「整组」语义要覆盖嵌套：组自己的成员 + 所有后代组的成员，升序。
    /// 合成/显隐/锁定/点选都走这个，只有面板缩进用 `group_members`。
    pub fn group_subtree_layers(&self, id: GroupId) -> Vec<usize> {
        let mut ids = self.descendant_groups(id);
        ids.push(id);
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.group.map(|g| ids.contains(&g)).unwrap_or(false))
            .map(|(i, _)| i)
            .collect()
    }

    /// 把选中的多个层归到一个新组。**不重排 `layers`**：编组只是打标签，
    /// 保持每个像素的 z 序原样（保真优先，编组不该改变画面）。
    /// 背景层、越界下标会被跳过；跨组混选（一半在 A 组、一半在 B 组或散层）
    /// 也跳过，因为没法判断新组该挂在哪。
    ///
    /// 同一个组里的若干层再编组 = 在它里面建子组（PS 也是这样），新组的
    /// 父组就是那个共同组。不足 2 层报错。
    pub fn group_layers(
        &mut self,
        indices: &[usize],
        name: Option<String>,
    ) -> Result<GroupId, EditError> {
        let mut picked = indices.to_vec();
        picked.sort_unstable();
        picked.dedup();
        let mut clean = Vec::new();
        // 三态：还没见过 / 见过且都属于同一个组（含 None）/ 出现了分歧。
        let mut common: Option<Option<GroupId>> = None;
        let mut mixed = false;
        for i in picked {
            let layer = self.layers.get(i).ok_or(EditError::LayerIndex(i))?;
            if i == 0 || layer.background {
                continue;
            }
            match common {
                None => common = Some(layer.group),
                Some(seen) if seen == layer.group => {}
                _ => {
                    mixed = true;
                    continue;
                }
            }
            clean.push(i);
        }
        if mixed || clean.len() < 2 {
            return Err(EditError::GroupNeedsTwoLayers);
        }
        let parent = common.flatten();
        if let Some(pid) = parent {
            if self.find_group(pid).is_none() {
                return Err(EditError::GroupNotFound(pid));
            }
        }
        let name = name.unwrap_or_else(|| format!("组 {}", self.groups.len() + 1));
        let mut group = LayerGroup::new(name);
        group.parent = parent;
        let gid = group.id;
        let mut groups = self.groups.clone();
        groups.push(group);
        // SetGroups 放第一个：撤销时它是最后被 revert 的，先恢复各层归属
        // 再恢复组表，顺序天然自洽。
        let mut ops = vec![EditOp::SetGroups {
            from: self.groups.clone(),
            to: groups,
        }];
        for i in &clean {
            ops.push(EditOp::SetLayerGroup {
                index: *i,
                from: self.layers[*i].group,
                to: Some(gid),
            });
        }
        self.execute(EditOp::Batch(ops))?;
        Ok(gid)
    }

    /// 解散分组：只摘掉组的标签，图层本身与 z 序完全保留。
    /// 子组不会被一起删掉，而是挂到被解散组的父组上（否则它们会悬空，
    /// 变成面板里再也渲染不出来的孤儿组）。
    pub fn ungroup(&mut self, gid: GroupId) -> Result<(), EditError> {
        let group = self
            .find_group(gid)
            .ok_or(EditError::GroupNotFound(gid))?;
        let parent = group.parent;
        let members = self.group_members(gid);
        let mut groups = self.groups.clone();
        groups.retain(|g| g.id != gid);
        for child in groups.iter_mut() {
            if child.parent == Some(gid) {
                child.parent = parent;
            }
        }
        let mut ops: Vec<EditOp> = members
            .iter()
            .map(|&i| EditOp::SetLayerGroup {
                index: i,
                from: Some(gid),
                to: None,
            })
            .collect();
        ops.push(EditOp::SetGroups {
            from: self.groups.clone(),
            to: groups,
        });
        self.execute(EditOp::Batch(ops))
    }

    pub fn rename_group(
        &mut self,
        gid: GroupId,
        name: impl Into<String>,
    ) -> Result<(), EditError> {
        let mut groups = self.groups.clone();
        let group = groups
            .iter_mut()
            .find(|g| g.id == gid)
            .ok_or(EditError::GroupNotFound(gid))?;
        let name = name.into();
        if group.name == name {
            return Ok(());
        }
        group.name = name;
        self.execute(EditOp::SetGroups {
            from: self.groups.clone(),
            to: groups,
        })
    }

    /// 把单个图层移入 / 移出分组（`None` = 顶层）。不重排 `layers`、不动像素。
    ///
    /// ★隐藏组靠批量写成员 `visible` 生效（合成只认成员），所以拖进隐藏组
    /// 必须同步把该层隐藏；从隐藏组拖出来则按 `hidden_snapshot` 还原（没有
    /// 记录就显示）。这两个 `SetVisible` 与归组放在同一批里，撤销 / 重做
    /// 会一起精确回滚。组自身有混合效果（隔离组）时不改成员字段，渲染差异
    /// 交给 bridge 的 `refresh_preview` 处理。
    pub fn set_layer_group(
        &mut self,
        index: usize,
        group: Option<GroupId>,
    ) -> Result<(), EditError> {
        if let Some(gid) = group {
            self.find_group(gid)
                .ok_or(EditError::GroupNotFound(gid))?;
        }
        let layer = self.layers.get(index).ok_or(EditError::LayerIndex(index))?;
        let from = layer.group;
        if from == group {
            return Ok(());
        }
        let from_flags = self.group_state_flags(from);
        let to_flags = self.group_state_flags(group);
        let layer_id = layer.id;
        let was_visible = layer.visible;

        let visible_to = if to_flags.hidden && !from_flags.hidden {
            if was_visible { Some(false) } else { None }
        } else if !to_flags.hidden && from_flags.hidden {
            let restore = from_flags
                .hidden_group
                .and_then(|gid| self.find_group(gid))
                .and_then(|g| {
                    g.hidden_snapshot
                        .iter()
                        .find(|(id, _)| *id == layer_id)
                        .map(|(_, v)| *v)
                })
                .unwrap_or(true);
            if was_visible != restore { Some(restore) } else { None }
        } else {
            None
        };

        let mut ops = vec![EditOp::SetLayerGroup {
            index,
            from,
            to: group,
        }];
        if let Some(to) = visible_to {
            ops.push(EditOp::SetVisible {
                index,
                from: was_visible,
                to,
            });
        }
        self.execute(EditOp::Batch(ops))
    }

    /// 组混合模式 / 不透明度 / 穿透：三项都改变合成结果，走统一历史。
    pub fn set_group_blend(&mut self, gid: GroupId, blend: BlendMode) -> Result<(), EditError> {
        let from = self
            .find_group(gid)
            .ok_or(EditError::GroupNotFound(gid))?
            .blend;
        if from == blend {
            return Ok(());
        }
        self.execute(EditOp::SetGroupBlend {
            id: gid,
            from,
            to: blend,
        })
    }

    /// 面板改组混合模式：非「正常」模式在 PS 里会同时关掉穿透，否则
    /// `blend_group` 根本不会被调用，用户选了 Multiply 却看不到变化。
    /// 两步放进同一批历史，撤销一次就回到原来的模式 + 穿透状态。
    pub fn set_group_blend_isolating(
        &mut self,
        gid: GroupId,
        blend: BlendMode,
    ) -> Result<(), EditError> {
        let (from_blend, from_pass) = {
            let group = self
                .find_group(gid)
                .ok_or(EditError::GroupNotFound(gid))?;
            (group.blend, group.pass_through)
        };
        let mut ops = Vec::new();
        if from_blend != blend {
            ops.push(EditOp::SetGroupBlend {
                id: gid,
                from: from_blend,
                to: blend,
            });
        }
        if blend != BlendMode::Normal && from_pass {
            ops.push(EditOp::SetGroupPassThrough {
                id: gid,
                from: true,
                to: false,
            });
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.execute(EditOp::Batch(ops))
    }

    pub fn set_group_opacity(&mut self, gid: GroupId, opacity: f32) -> Result<(), EditError> {
        let from = self
            .find_group(gid)
            .ok_or(EditError::GroupNotFound(gid))?
            .opacity;
        let to = opacity.clamp(0.0, 1.0);
        if (from - to).abs() < f32::EPSILON {
            return Ok(());
        }
        self.execute(EditOp::SetGroupOpacity {
            id: gid,
            from,
            to,
        })
    }

    pub fn set_group_pass_through(&mut self, gid: GroupId, pass: bool) -> Result<(), EditError> {
        let from = self
            .find_group(gid)
            .ok_or(EditError::GroupNotFound(gid))?
            .pass_through;
        if from == pass {
            return Ok(());
        }
        self.execute(EditOp::SetGroupPassThrough {
            id: gid,
            from,
            to: pass,
        })
    }

    /// 折叠 / 展开是纯视图状态：不进撤销栈、不动像素版本（PS 也是这个语义）。
    /// UI 侧另有 `groupViewEpoch` 负责刷新面板。
    pub fn set_group_collapsed_view(
        &mut self,
        gid: GroupId,
        collapsed: bool,
    ) -> Result<(), EditError> {
        let group = self
            .groups
            .iter_mut()
            .find(|g| g.id == gid)
            .ok_or(EditError::GroupNotFound(gid))?;
        group.collapsed = collapsed;
        Ok(())
    }

    /// 展开某图层所在组及其全部折叠祖先，返回实际被展开的组 id。
    /// 用文档里的真实组状态（不依赖 UI 侧可能过期的 groups_json 快照），
    /// 供「画布选层后定位到面板行」使用。折叠是视图态，不进撤销栈。
    pub fn expand_layer_ancestors(&mut self, index: usize) -> Vec<GroupId> {
        let Some(layer) = self.layers.get(index) else {
            return Vec::new();
        };
        let mut gid = layer.group;
        let mut out = Vec::new();
        let mut guard = 0;
        while let Some(id) = gid {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(g) = self.groups.iter_mut().find(|g| g.id == id) else {
                break;
            };
            gid = g.parent;
            if g.collapsed {
                g.collapsed = false;
                out.push(id);
            }
        }
        out
    }

    /// 整组显隐。合成只认成员的 `visible`，所以这里批量写成员；
    /// 隐藏前把各成员自己的显隐存进 `hidden_snapshot`，重新显示时按稳定
    /// id 还原 —— 组里原本手动隐藏的层不会被「显示整组」误点亮。
    pub fn set_group_visible(&mut self, gid: GroupId, visible: bool) -> Result<(), EditError> {
        let before = self
            .find_group(gid)
            .ok_or(EditError::GroupNotFound(gid))?
            .clone();
        if before.visible == visible {
            return Ok(());
        }
        // 整组 = 自己的成员 + 所有后代组的成员（嵌套组的眼睛要一次点亮到底）。
        let members = self.group_subtree_layers(gid);
        let mut groups = self.groups.clone();
        let mut ops: Vec<EditOp> = Vec::new();
        {
            let group = groups
                .iter_mut()
                .find(|g| g.id == gid)
                .expect("group existence checked above");
            group.visible = visible;
            if visible {
                let saved = std::mem::take(&mut group.hidden_snapshot);
                let target_of = |layer: &Layer| {
                    if saved.is_empty() {
                        true
                    } else {
                        saved
                            .iter()
                            .find(|(id, _)| *id == layer.id)
                            .map(|(_, v)| *v)
                            .unwrap_or(true)
                    }
                };
                for &i in &members {
                    let layer = &self.layers[i];
                    let target = target_of(layer);
                    if layer.visible != target {
                        ops.push(EditOp::SetVisible {
                            index: i,
                            from: layer.visible,
                            to: target,
                        });
                    }
                }
            } else {
                group.hidden_snapshot = members
                    .iter()
                    .map(|&i| (self.layers[i].id, self.layers[i].visible))
                    .collect();
                for &i in &members {
                    let layer = &self.layers[i];
                    if layer.visible {
                        ops.push(EditOp::SetVisible {
                            index: i,
                            from: true,
                            to: false,
                        });
                    }
                }
            }
        }
        ops.push(EditOp::SetGroups {
            from: self.groups.clone(),
            to: groups,
        });
        self.execute(EditOp::Batch(ops))
    }

    /// 面板眼睛该显示的显隐 —— 图层「自身的可见意愿」。
    ///
    /// 合成只认成员的 `visible`，所以隐藏组时成员被批量写成 false；但 PS 里
    /// 组内图层的眼睛仍显示各自原状（只有组的眼睛变闭）。自身意愿保存在
    /// `hidden_snapshot` 里：任一隐藏祖先组的快照认为它可见，就显示睁眼。
    pub fn layer_display_visible(&self, index: usize) -> bool {
        let Some(layer) = self.layers.get(index) else {
            return false;
        };
        if layer.visible {
            return true;
        }
        let mut gid = layer.group;
        let mut guard = 0;
        while let Some(id) = gid {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(g) = self.find_group(id) else {
                break;
            };
            if !g.visible
                && g.hidden_snapshot
                    .iter()
                    .any(|(lid, v)| *lid == layer.id && *v)
            {
                return true;
            }
            gid = g.parent;
        }
        false
    }

    /// 点面板眼睛：组隐藏时改的是「自身可见意愿」（`hidden_snapshot`），
    /// 不动合成真值 `layer.visible`（它必须保持 false，否则组隐藏失效）。
    /// 不在隐藏组里时走常规 `set_layer_visible`。整批进历史，可撤销。
    pub fn set_layer_eye(&mut self, index: usize, visible: bool) -> Result<(), EditError> {
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        let layer_id = self.layers[index].id;
        let mut gid = self.layers[index].group;
        let mut guard = 0;
        while let Some(id) = gid {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(g) = self.find_group(id) else {
                break;
            };
            if !g.visible {
                let mut groups = self.groups.clone();
                if let Some(group) = groups.iter_mut().find(|g| g.id == id) {
                    if let Some((_, v)) = group
                        .hidden_snapshot
                        .iter_mut()
                        .find(|(lid, _)| *lid == layer_id)
                    {
                        if *v != visible {
                            *v = visible;
                            return self.execute(EditOp::SetGroups {
                                from: self.groups.clone(),
                                to: groups,
                            });
                        }
                    }
                }
                return Ok(());
            }
            gid = g.parent;
        }
        self.set_layer_visible(index, visible)
    }

    /// 整组锁定 / 解锁（批量写成员 `locked`，可撤销）。同样覆盖嵌套子组。
    pub fn set_group_locked(&mut self, gid: GroupId, locked: bool) -> Result<(), EditError> {
        if self.find_group(gid).is_none() {
            return Err(EditError::GroupNotFound(gid));
        }
        let members = self.group_subtree_layers(gid);
        let mut ops: Vec<EditOp> = Vec::new();
        for i in members {
            let layer = &self.layers[i];
            if layer.locked != locked {
                ops.push(EditOp::SetLocked {
                    index: i,
                    from: layer.locked,
                    to: locked,
                });
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.execute(EditOp::Batch(ops))
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn doc_with_layers(n: usize) -> Document {
        let mut doc = Document::from_image(RgbaImage::from_pixel(8, 8, Rgba([10, 20, 30, 255])));
        for k in 0..n {
            let color = Rgba([100 + k as u8 * 20, 0, 0, 255]);
            doc.add_layer(Layer::new(
                format!("L{k}"),
                RgbaImage::from_pixel(4, 4, color),
            ))
            .unwrap();
        }
        doc
    }

    #[test]
    fn grouping_tags_layers_and_undo_redo_roundtrips() {
        let mut doc = doc_with_layers(3);
        let before = doc.flatten();
        let epoch = doc.edit_epoch;
        let gid = doc.group_layers(&[1, 3], None).unwrap();
        assert_eq!(doc.groups.len(), 1);
        assert_eq!(doc.groups[0].name, "组 1");
        assert_eq!(doc.group_members(gid), vec![1, 3]);
        assert_eq!(doc.group_of(2), None);
        assert_eq!(doc.layers[1].group, Some(gid));
        assert_eq!(doc.layers[3].group, Some(gid));
        // 编组不改像素、不改 z 序、不 bump 像素版本。
        assert_eq!(doc.flatten(), before);
        assert_eq!(doc.edit_epoch, epoch);

        doc.undo().unwrap();
        assert!(doc.groups.is_empty());
        assert_eq!(doc.layers[1].group, None);
        assert_eq!(doc.layers[3].group, None);
        assert_eq!(doc.edit_epoch, epoch, "撤销分组同样不动像素版本");

        doc.redo().unwrap();
        assert_eq!(doc.group_members(gid), vec![1, 3]);
    }

    #[test]
    fn grouping_rejects_background_and_single_layer() {
        let mut doc = doc_with_layers(2);
        assert_eq!(
            doc.group_layers(&[0, 1], None),
            Err(EditError::GroupNeedsTwoLayers)
        );
        assert_eq!(
            doc.group_layers(&[1], None),
            Err(EditError::GroupNeedsTwoLayers)
        );
        assert!(doc.groups.is_empty());
    }

    #[test]
    fn layer_already_in_a_group_is_skipped() {
        let mut doc = doc_with_layers(4);
        let first = doc.group_layers(&[1, 2], None).unwrap();
        assert_eq!(
            doc.group_layers(&[2, 3], None),
            Err(EditError::GroupNeedsTwoLayers),
            "已在组内的层不参与第二次编组"
        );
        assert_eq!(doc.group_members(first), vec![1, 2]);
    }

    #[test]
    fn hidden_group_keeps_member_eye_icons() {
        let mut doc = doc_with_layers(3);
        doc.set_layer_visible(2, false).unwrap();
        let gid = doc.group_layers(&[1, 2, 3], None).unwrap();
        doc.set_group_visible(gid, false).unwrap();
        // 合成真值：全隐藏；眼睛图标：保持各自被隐藏前的状态。
        assert!(doc.group_members(gid).iter().all(|&i| !doc.layers[i].visible));
        assert!(doc.layer_display_visible(1), "原本显示的成员眼睛仍是睁眼");
        assert!(!doc.layer_display_visible(2), "原本手动隐藏的成员仍是闭眼");
        assert!(doc.layer_display_visible(3));

        // 组隐藏期间点成员眼睛：改的是「自身意愿」，不改合成真值。
        doc.set_layer_eye(1, false).unwrap();
        assert!(!doc.layer_display_visible(1), "点眼睛应立刻反映到图标");
        assert!(!doc.layers[1].visible, "合成真值保持隐藏");
        doc.set_layer_eye(2, true).unwrap();
        assert!(doc.layer_display_visible(2));

        // 显示整组后：按自身意愿还原，不是一律点亮。
        doc.set_group_visible(gid, true).unwrap();
        assert!(!doc.layers[1].visible, "刚点的闭眼成员保持隐藏");
        assert!(doc.layers[2].visible, "刚点的睁眼成员恢复显示");
        assert!(doc.layers[3].visible);
    }

    #[test]
    fn ungroup_keeps_layers_and_order() {
        let mut doc = doc_with_layers(3);
        let ids: Vec<u32> = doc.layers.iter().map(|l| l.id).collect();
        let gid = doc.group_layers(&[1, 2, 3], Some("按钮组".into())).unwrap();
        assert_eq!(doc.find_group(gid).unwrap().name, "按钮组");
        doc.ungroup(gid).unwrap();
        assert!(doc.groups.is_empty());
        assert_eq!(doc.layers.iter().map(|l| l.id).collect::<Vec<_>>(), ids);
        assert!(doc.layers.iter().all(|l| l.group.is_none()));
    }

    #[test]
    fn group_visibility_restores_individually_hidden_member() {
        let mut doc = doc_with_layers(3);
        doc.set_layer_visible(2, false).unwrap();
        let gid = doc.group_layers(&[1, 2, 3], None).unwrap();
        assert!(!doc.layers[2].visible);
        assert!(doc.layers[1].visible);

        doc.set_group_visible(gid, false).unwrap();
        assert!(doc.group_members(gid).iter().all(|&i| !doc.layers[i].visible));
        assert_eq!(doc.find_group(gid).unwrap().hidden_snapshot.len(), 3);

        doc.set_group_visible(gid, true).unwrap();
        assert!(doc.layers[1].visible);
        assert!(!doc.layers[2].visible, "原本手动隐藏的成员要保持隐藏");
        assert!(doc.layers[3].visible);
        assert!(doc.find_group(gid).unwrap().hidden_snapshot.is_empty());

        doc.undo().unwrap();
        assert!(doc.group_members(gid).iter().all(|&i| !doc.layers[i].visible));
    }

    #[test]
    fn moving_layer_into_hidden_group_hides_it_and_out_restores_visibility() {
        let mut doc = doc_with_layers(3);
        let gid = doc.group_layers(&[1, 2], None).unwrap();
        doc.set_group_visible(gid, false).unwrap();
        assert!(!doc.layers[1].visible && !doc.layers[2].visible);
        assert!(doc.layers[3].visible, "组外那层仍是显示的");

        // 拖进隐藏组：显隐必须跟着组走，且与归组在同一批历史里。
        doc.set_layer_group(3, Some(gid)).unwrap();
        assert_eq!(doc.group_of(3), Some(gid));
        assert!(!doc.layers[3].visible, "拖进隐藏组的层也要隐藏");
        doc.undo().unwrap();
        assert_eq!(doc.group_of(3), None);
        assert!(doc.layers[3].visible, "撤销归组要一并还原显隐");
        doc.redo().unwrap();
        assert_eq!(doc.group_of(3), Some(gid));
        assert!(!doc.layers[3].visible);

        // 从隐藏组拖出来：恢复显示；重新显示整个组时也不该有孤立的隐藏层。
        doc.set_layer_group(3, None).unwrap();
        assert!(doc.layers[3].visible);
        doc.set_group_visible(gid, true).unwrap();
        assert!(doc.layers.iter().all(|l| l.visible));
    }

    #[test]
    fn isolated_group_flags_mark_render_affecting_moves() {
        let mut doc = doc_with_layers(2);
        let gid = doc.group_layers(&[1, 2], None).unwrap();
        let plain = doc.group_state_flags(Some(gid));
        assert!(!plain.hidden && !plain.effects, "默认穿透组不改变渲染");

        doc.set_group_pass_through(gid, false).unwrap();
        let isolated = doc.group_state_flags(Some(gid));
        assert!(isolated.effects, "非穿透组要触发预览重绘");
        assert!(!doc.group_state_flags(None).effects);

        // 面板里只改混合模式：core 会同时关掉穿透，让模式真正参与合成。
        doc.set_group_pass_through(gid, true).unwrap();
        doc.set_group_blend_isolating(gid, BlendMode::Multiply)
            .unwrap();
        assert!(!doc.find_group(gid).unwrap().pass_through);
        assert!(doc.find_group(gid).unwrap().isolated());
        assert!(doc.group_state_flags(Some(gid)).effects);
        doc.undo().unwrap();
        assert_eq!(doc.find_group(gid).unwrap().blend, BlendMode::Normal);
        assert!(doc.find_group(gid).unwrap().pass_through);
    }

    #[test]
    fn nested_groups_from_psd_keep_parent_links_and_cover_subtree() {
        let seed = |name: &str, group: Option<u32>| PsdLayerSeed {
            name: name.into(),
            offset: (0, 0),
            image: RgbaImage::from_pixel(4, 4, Rgba([9, 9, 9, 255])),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            group,
            mask: None,
            clipping: false,
            clip_shape: None,
            style: None,
            blend_if: None,
            knockout: Knockout::None,
            transparency_shapes: true,
        };
        // 底 -> 顶：底层无组，中层在「内层」组，顶层在「外层」组。
        let mut doc = Document::from_psd_layers_with_groups(
            (8, 8),
            vec![seed("松", None), seed("内", Some(9)), seed("外", Some(7))],
            vec![
                PsdGroupSeed {
                    psd_id: 9,
                    parent: Some(7),
                    name: "内层".into(),
                    collapsed: true,
                    visible: true,
                    blend: BlendMode::Normal,
                    opacity: 1.0,
                    pass_through: true,
                },
                PsdGroupSeed {
                    psd_id: 7,
                    parent: None,
                    name: "外层".into(),
                    collapsed: false,
                    visible: true,
                    blend: BlendMode::Normal,
                    opacity: 1.0,
                    pass_through: true,
                },
            ],
        );
        assert_eq!(doc.groups.len(), 2);
        let outer = doc.groups.iter().find(|g| g.name == "外层").unwrap().id;
        let inner = doc.groups.iter().find(|g| g.name == "内层").unwrap().id;
        assert!(doc.find_group(inner).unwrap().collapsed, "PSD 的关闭态要保留");
        assert_eq!(doc.find_group(inner).unwrap().parent, Some(outer));
        assert_eq!(doc.layers[1].group, None);
        assert_eq!(doc.layers[2].group, Some(inner));
        assert_eq!(doc.layers[3].group, Some(outer));
        assert_eq!(doc.group_depth(inner), 1);
        assert_eq!(doc.group_depth(outer), 0);
        assert_eq!(doc.descendant_groups(outer), vec![inner]);
        // 「整组」必须一起盖住嵌套成员，否则眼睛只关一半。
        assert_eq!(doc.group_subtree_layers(outer), vec![2, 3]);
        assert_eq!(doc.group_members(outer), vec![3], "直接成员只有一层");
        doc.set_group_visible(outer, false).unwrap();
        assert!(!doc.layers[2].visible && !doc.layers[3].visible);
        doc.set_group_visible(outer, true).unwrap();
        assert!(doc.layers[2].visible && doc.layers[3].visible);
        // 解散父组：子组挂到父组的父组（这里就是顶层），不能变孤儿。
        doc.ungroup(outer).unwrap();
        assert_eq!(doc.groups.len(), 1);
        assert_eq!(doc.find_group(inner).unwrap().parent, None);
        assert_eq!(doc.layers[3].group, None);
        assert_eq!(doc.layers[2].group, Some(inner));
    }

    #[test]
    fn snapshot_restores_groups_after_pixel_op_undo() {
        let mut doc = doc_with_layers(2);
        let gid = doc.group_layers(&[1, 2], None).unwrap();
        doc.resize(Size::new(16, 16)).unwrap();
        doc.undo().unwrap();
        assert_eq!(doc.group_members(gid), vec![1, 2]);
        assert_eq!(doc.groups.len(), 1);
    }

    #[test]
    fn rename_group_is_undoable_and_lock_is_batched() {
        let mut doc = doc_with_layers(2);
        let gid = doc.group_layers(&[1, 2], None).unwrap();
        doc.rename_group(gid, "标题").unwrap();
        assert_eq!(doc.find_group(gid).unwrap().name, "标题");
        doc.undo().unwrap();
        assert_eq!(doc.find_group(gid).unwrap().name, "组 1");

        doc.set_group_locked(gid, true).unwrap();
        assert!(doc.group_members(gid).iter().all(|&i| doc.layers[i].locked));
        doc.undo().unwrap();
        assert!(doc.group_members(gid).iter().all(|&i| !doc.layers[i].locked));
    }

    #[test]
    fn expand_layer_ancestors_opens_whole_collapsed_chain() {
        let mut doc = doc_with_layers(2);
        // 外层含层1 + 内层，内层含层2（见 doc_with_layers 的组结构）。
        let gid = doc.group_layers(&[1, 2], None).unwrap();
        doc.set_group_collapsed_view(gid, true).unwrap();
        assert!(doc.find_group(gid).unwrap().collapsed);

        let expanded = doc.expand_layer_ancestors(1);
        assert!(expanded.contains(&gid), "应展开层1所在的折叠组");
        assert!(!doc.find_group(gid).unwrap().collapsed);
        // 折叠是视图态：不进撤销栈。撤销建组后该组已不存在，谈不上恢复折叠。
        doc.undo().unwrap();
        assert!(doc.find_group(gid).is_none());
    }
}
