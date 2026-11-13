// crates/core/src/group_ops.rs
//! 图层分组：只改「组织元数据」，不动 z 序、不动像素。
//!
//! 设计约束（见 docs）：组**不是** `LayerContent` 的新变体，也不进
//! `Document::layers`。合成、预览缓存、PSD 写出都只遍历 `layers`，因此这里
//! 所有操作对渲染零影响；成员关系通过 `Layer::group` 反查，删层 / 撤销 /
//! 整层替换时归属自动跟着走，不需要维护第二份成员表。
use super::*;

impl Document {
    pub fn find_group(&self, id: GroupId) -> Option<&LayerGroup> {
        self.groups.iter().find(|g| g.id == id)
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
    fn nested_groups_from_psd_keep_parent_links_and_cover_subtree() {
        let seed = |name: &str, group: Option<u32>| PsdLayerSeed {
            name: name.into(),
            offset: (0, 0),
            image: RgbaImage::from_pixel(4, 4, Rgba([9, 9, 9, 255])),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            group,
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
                },
                PsdGroupSeed {
                    psd_id: 7,
                    parent: None,
                    name: "外层".into(),
                    collapsed: false,
                    visible: true,
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
}
