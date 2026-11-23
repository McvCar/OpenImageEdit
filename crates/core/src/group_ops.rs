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
        let ops = self.set_layer_group_ops(index, group)?;
        if ops.is_empty() {
            return Ok(());
        }
        self.execute(EditOp::Batch(ops))
    }

    /// `set_layer_group` 的 op 序列构造（供「归组 + 落位」组合复用）。
    /// 归属没变时返回空。
    fn set_layer_group_ops(
        &self,
        index: usize,
        group: Option<GroupId>,
    ) -> Result<Vec<EditOp>, EditError> {
        if let Some(gid) = group {
            self.find_group(gid)
                .ok_or(EditError::GroupNotFound(gid))?;
        }
        let layer = self.layers.get(index).ok_or(EditError::LayerIndex(index))?;
        let from = layer.group;
        if from == group {
            return Ok(Vec::new());
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
        Ok(ops)
    }

    /// 归组 + z 落位一步完成（面板跨组拖层）。先改归属（含隐藏环境的
    /// 显隐切换），再 `MoveLayer` 到 `to_z`；合成一条 Batch = 一次撤销。
    ///
    /// ★QML 必须走这个组合入口，不能拆两次 qinvokable：归组会同步触发
    /// layers_json 重建、销毁正在拖的 delegate，第二次调用根本不执行
    /// （用户报的「换分组固定在目标分组最上面」）。`to_z` 语义与
    /// `MoveLayer.to` 一致（remove 后的最终文档索引）；`to_z == index`
    /// 表示只归组不落位。
    pub fn move_layer_to_group_at(
        &mut self,
        index: usize,
        group: Option<GroupId>,
        to_z: usize,
    ) -> Result<(), EditError> {
        let mut ops = self.set_layer_group_ops(index, group)?;
        if to_z != index {
            if to_z == 0 || to_z >= self.layers.len() {
                return Err(EditError::LayerIndex(to_z));
            }
            ops.push(EditOp::MoveLayer {
                from: index,
                to: to_z,
            });
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.execute(EditOp::Batch(ops))
    }

    /// 把整个分组（连同其子组）移进另一组（`target=Some`）或移回顶层
    /// （`target=None`）。只改 `Group::parent` 这一份元数据 —— 成员的
    /// `Layer.group` 都指向自己的直属组，不用动；z 序也保持原样（与
    /// 单层 `set_layer_group` 同语义：编组 / 归组不改变画面布局）。
    ///
    /// 渲染状态随环境切换：
    /// * 移进隐藏环境（目标链上有隐藏组）：子树里所有可见成员跟着隐藏
    ///   （`SetVisible`），与「拖层进隐藏组」行为一致；
    /// * 移出隐藏环境：按原链最近隐藏组的 `hidden_snapshot` 恢复成员
    ///   自身意愿（不在快照里 = 想显示），与单层移出的还原逻辑相同。
    /// 两侧都是隔离组（不透明度 / 混合 / 非穿透）时画面也要变，bridge
    /// 侧统一 `refresh_preview` 兜住。
    pub fn move_group_to_group(
        &mut self,
        gid: GroupId,
        target: Option<GroupId>,
    ) -> Result<(), EditError> {
        let ops = self.move_group_parent_ops(gid, target)?;
        if ops.is_empty() {
            return Ok(());
        }
        self.execute(EditOp::Batch(ops))
    }

    /// `move_group_to_group` 的 op 序列构造（供「归组 + 排序」组合复用）。
    /// 已经在目标父组里时返回空。
    fn move_group_parent_ops(
        &mut self,
        gid: GroupId,
        target: Option<GroupId>,
    ) -> Result<Vec<EditOp>, EditError> {
        if self.find_group(gid).is_none() {
            return Err(EditError::GroupNotFound(gid));
        }
        if let Some(tid) = target {
            if self.find_group(tid).is_none() {
                return Err(EditError::GroupNotFound(tid));
            }
        }
        // 防环：目标链上出现自己 = 把组塞进它自己的子树，parent 会成环。
        {
            let mut next = target;
            let mut guard = 0;
            while let Some(id) = next {
                guard += 1;
                if guard > 64 {
                    break;
                }
                if id == gid {
                    return Err(EditError::GroupCycle(gid));
                }
                next = self.find_group(id).and_then(|g| g.parent);
            }
        }
        // 「环境」= 组自己的祖先链（不含 gid 自身）：gid 自己的显隐是用户
        // 设置的独立属性，移动组不改它的眼睛状态。
        let from_flags = self
            .find_group(gid)
            .and_then(|g| g.parent)
            .map(|pid| self.group_state_flags(Some(pid)))
            .unwrap_or_default();
        let to_flags = self.group_state_flags(target);
        // ★from 必须在改 parent 之前取：from/to 相同的 SetGroups 会让撤销
        // 「恢复」到改后状态（单测抓出来过）。
        let from_groups = self.groups.clone();
        {
            let group = self
                .groups
                .iter_mut()
                .find(|g| g.id == gid)
                .expect("group existence checked above");
            if group.parent == target {
                return Ok(Vec::new()); // 已经在那个组里，无事可做
            }
            group.parent = target;
        }
        let mut ops = vec![EditOp::SetGroups {
            from: from_groups.clone(),
            to: self.groups.clone(),
        }];
        // 显隐随环境切换（成员显隐见 doc 注释；显隐单独成 op，撤销/重做
        // 时 Batch 顺序 apply、逆序回滚，与组 parent 变化保持一致）。
        let members = self.group_subtree_layers(gid);
        if to_flags.hidden && !from_flags.hidden {
            for &i in &members {
                if self.layers[i].visible {
                    ops.push(EditOp::SetVisible {
                        index: i,
                        from: true,
                        to: false,
                    });
                }
            }
        } else if !to_flags.hidden && from_flags.hidden {
            let snapshot = from_flags
                .hidden_group
                .and_then(|hgid| self.find_group(hgid))
                .map(|g| g.hidden_snapshot.clone())
                .unwrap_or_default();
            for &i in &members {
                let layer = &self.layers[i];
                let restore = snapshot
                    .iter()
                    .find(|(id, _)| *id == layer.id)
                    .map(|(_, v)| *v)
                    .unwrap_or(true);
                if layer.visible != restore {
                    ops.push(EditOp::SetVisible {
                        index: i,
                        from: layer.visible,
                        to: restore,
                    });
                }
            }
        }
        Ok(ops)
    }

    /// 组「归组 + z 排序」一步完成（面板把组拖进别的组并落在指定缝位）。
    /// 合成一条 Batch = 一次撤销；QML 拆两次调用会因 delegate 销毁丢掉
    /// 第二步（与 `move_layer_to_group_at` 同理）。
    pub fn move_group_to_group_at(
        &mut self,
        gid: GroupId,
        target: Option<GroupId>,
        anchor: Option<(usize, bool)>,
    ) -> Result<(), EditError> {
        let mut ops = self.move_group_parent_ops(gid, target)?;
        if let Some((a, above)) = anchor {
            if let Some(op) = self.move_group_z_op(gid, a, above)? {
                ops.push(op);
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.execute(EditOp::Batch(ops))
    }

    /// 组 z 序调整（面板里拖组标题行与同级组换位）：整棵子树的成员作为
    /// 一个块，**保持相对 z 序**搬到目标位置。不改变任何 `group` /
    /// `parent`——纯排序；一次 Batch = 一条撤销。
    ///
    /// `anchor` 是缝上面那行的**文档索引**（层行 → 该层；组标题行 → 该组
    /// 最高成员），`above` 决定块显示在锚点显示位的上方（true）还是下方
    /// （false）。成员分散也安全：模拟数组逐成员生成 MoveLayer，每次的
    /// from/to 都是该步执行时的真实状态。
    pub fn move_group_z(
        &mut self,
        gid: GroupId,
        anchor: usize,
        above: bool,
    ) -> Result<(), EditError> {
        match self.move_group_z_op(gid, anchor, above)? {
            Some(op) => self.execute(op),
            None => Ok(()),
        }
    }

    /// `move_group_z` 的 op 构造（供「归组 + 排序」组合复用）。无操作时
    /// 返回 `None`。
    fn move_group_z_op(
        &self,
        gid: GroupId,
        anchor: usize,
        above: bool,
    ) -> Result<Option<EditOp>, EditError> {
        if self.find_group(gid).is_none() {
            return Err(EditError::GroupNotFound(gid));
        }
        let idxs = self.group_subtree_layers(gid);
        let n = self.layers.len();
        if idxs.is_empty() || idxs[0] == 0 || idxs.len() >= n {
            return Ok(None); // 空组 / 含背景 / 块占满，无事可做
        }
        if idxs.contains(&anchor) {
            return Ok(None); // 锚点在被拖的组块里（拖到自己成员上）—— 无操作
        }
        let k = idxs.len();
        // 锚点在块移走后的索引：块里比它小的成员个数决定平移量。
        let m = anchor.min(n - 1);
        let shifted = m - idxs.iter().filter(|&&i| i < m).count();
        // 块（升序）目标占 [t, t+k-1]，组标题行显示在块最高成员上方。
        //   above=false（块显示在锚点行下面）→ 块插到 rest 的锚点位置，
        //     锚点被顶走 k 位、块最高成员紧贴锚点最终位下面：t = shifted
        //   above=true（块显示在锚点行上面）→ 块最低成员紧贴锚点上面：
        //     t = shifted + 1
        // （与层拖动 k=1 时 toZ = m'∓1 完全一致；146 组随机场景验证过。）
        let t = if above {
            shifted + 1
        } else {
            shifted
        };
        let t = (t.max(1) as usize).min(n - k);
        // 目标排列 = 其余成员保持相对序，块（升序）插到 t。
        let mut to_order = Vec::with_capacity(n);
        // ★块成员一律由插入段负责落位（`contains` 持续跳过）：旧写法
        // `bi < k && contains` 在插块点 t 落到块成员区间之前（组向上拖）
        // 时，插块后 bi==k、后面的块成员被重复 push → to_order 不是
        // 排列 → ReorderLayers 报 LayerIndex（用户报「同级分组调序异常」）。
        let mut inserted = false;
        for i in 0..n {
            if !inserted && to_order.len() >= t {
                for &b in &idxs {
                    to_order.push(b);
                }
                inserted = true;
            }
            if idxs.contains(&i) {
                continue; // 块成员跳过，由插入段统一落位
            }
            to_order.push(i);
        }
        if !inserted {
            for &b in &idxs {
                to_order.push(b);
            }
        }
        let from_order: Vec<usize> = {
            // revert 需要「新位置 i ← 旧位置 from_order[i]」的逆映射：
            // from_order[old_idx] = new_pos。
            let mut inv = vec![0usize; n];
            for (p, &old_idx) in to_order.iter().enumerate() {
                inv[old_idx] = p;
            }
            inv
        };
        if to_order.iter().enumerate().all(|(i, &v)| v == i) {
            return Ok(None); // 块已在目标位，无操作
        }
        Ok(Some(EditOp::ReorderLayers { from_order, to_order }))
    }
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

    /// `layers[index]`（剪贴层）所在剪贴链的基底覆盖率（0..1）。
    /// 供 GPU 快路径把剪贴活动层烘进画布坐标时逐点采样，语义与
    /// [`Document::hit_alpha_at`] 里的剪贴限幅完全一致。
    pub fn clip_coverage_at(&self, index: usize, x: i32, y: i32) -> f32 {
        let Some(layer) = self.layers.get(index) else {
            return 0.0;
        };
        if !layer.clipping {
            return 1.0;
        }
        let gid = layer.group;
        self.layers[..index]
            .iter()
            .rev()
            .take_while(|l| l.group == gid)
            .find(|l| !l.clipping)
            .map(|b| {
                if !b.visible || b.opacity <= 0.0 {
                    return 0.0f32;
                }
                match &b.clip_shape {
                    Some(s) => {
                        s.coverage(x, y)
                            * b.mask
                                .as_ref()
                                .map(|m| m.coverage(x, y))
                                .unwrap_or(1.0)
                    }
                    None => b.hit_alpha_at(x, y) as f32 / 255.0,
                }
            })
            .unwrap_or(0.0)
        .clamp(0.0, 1.0)
    }

    /// 画布点选的文档级命中：`layers[index]` 在画布坐标 `(x, y)` 处实际
    /// 可见的 alpha（0–255）。
    ///
    /// 在 `Layer::hit_alpha_at`（内容 + 图层蒙版）之上补上**剪贴链限幅**：
    /// 剪贴层只有落在下方最近非剪贴基底的内容形状里才算命中 —— 与渲染
    /// （`composite_range` / `fill_clip_coverage`）同一语义。否则「点空白」
    /// 会选中剪贴蒙版裁掉的透明区（用户报的 bug：蒙版裁剪区域被点击到）。
    /// 非剪贴层原样透传，行为不变。
    pub fn hit_alpha_at(&self, index: usize, x: i32, y: i32) -> u8 {
        let Some(layer) = self.layers.get(index) else {
            return 0;
        };
        let a = layer.hit_alpha_at(x, y);
        if a == 0 || !layer.clipping {
            return a;
        }
        // 渲染语义：剪贴链共享「下方最近非剪贴层」的覆盖率；隐藏 / 全透明
        // 基底 → 整条链一起消失（覆盖率 0）。基底搜索不跨组：剪贴蒙版是
        // 组内语义，链必须连续（PSD 亦如此），组边界上没有基底。
        let cov = self.clip_coverage_at(index, x, y);
        ((a as f32 / 255.0) * cov * 255.0).round() as u8
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
    fn move_group_to_group_nests_rejects_cycles_and_syncs_visibility() {
        let mut doc = doc_with_layers(4);
        let g1 = doc.group_layers(&[1, 2], None).unwrap();
        let g2 = doc.group_layers(&[3, 4], None).unwrap();
        assert_eq!(doc.find_group(g2).unwrap().parent, None);

        // 嵌套：组 2 进组 1。成员直属组不变，z 序不变。
        doc.move_group_to_group(g2, Some(g1)).unwrap();
        assert_eq!(doc.find_group(g2).unwrap().parent, Some(g1));
        assert_eq!(doc.layers[3].group, Some(g2));
        assert_eq!(doc.layers[4].group, Some(g2));

        // 防环：组 1 不能移进自己的后代组 2。
        assert_eq!(
            doc.move_group_to_group(g1, Some(g2)),
            Err(EditError::GroupCycle(g1))
        );
        assert_eq!(doc.find_group(g1).unwrap().parent, None);

        // 显隐同步：隐藏外层组 1 → 嵌套成员（层 3/4）一起隐藏；显示时还原。
        doc.set_group_visible(g1, false).unwrap();
        assert!(!doc.layers[1].visible && !doc.layers[3].visible);
        doc.set_group_visible(g1, true).unwrap();
        assert!(doc.layers[1].visible && doc.layers[3].visible);

        // 移回顶层 + 撤销/重做走 parent。
        doc.move_group_to_group(g2, None).unwrap();
        assert_eq!(doc.find_group(g2).unwrap().parent, None);
        doc.undo().unwrap();
        assert_eq!(doc.find_group(g2).unwrap().parent, Some(g1));
        doc.redo().unwrap();
        assert_eq!(doc.find_group(g2).unwrap().parent, None);
    }

    #[test]
    fn move_group_z_moves_block_keeps_order_and_undo() {
        let mut doc = doc_with_layers(5);
        // 层：背景(0), L0(1), L1(2), L2(3), L3(4), L4(5)。
        let g1 = doc.group_layers(&[1, 2], None).unwrap(); // L0, L1
        let g2 = doc.group_layers(&[3, 4], None).unwrap(); // L2, L3

        // 组 1 的块排到 L4 下面（above=false）：块整体下移、显示上组 1
        // 标题行紧贴 L4 的下一行，组内相对序保持。
        doc.move_group_z(g1, 5, false).unwrap();
        assert_eq!(
            doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            vec!["背景", "L2", "L3", "L0", "L1", "L4"]
        );
        // 纯排序：归属不变。
        assert_eq!(doc.layers[1].group, Some(g2));
        assert_eq!(doc.layers[3].group, Some(g1));

        // 撤销恢复原排列；重做再生效。
        doc.undo().unwrap();
        assert_eq!(
            doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            vec!["背景", "L0", "L1", "L2", "L3", "L4"]
        );
        doc.redo().unwrap();
        assert_eq!(doc.layers[3].name, "L0");

        // above=true：组 2 的块（现在文档索引 3,4）排到 L0（索引 1）
        // 上面 → 块最低成员紧贴 L0 的上一行。
        doc.move_group_z(g2, 3, true).unwrap();
        assert_eq!(
            doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            vec!["背景", "L0", "L2", "L3", "L1", "L4"]
        );

        // 锚点在被拖的块里 → 无操作（不进历史）。
        let epoch = doc.edit_epoch;
        doc.move_group_z(g2, 3, true).unwrap();
        assert_eq!(doc.edit_epoch, epoch);
    }

    #[test]
    fn move_layer_to_group_at_combines_regroup_and_move() {
        let mut doc = doc_with_layers(5);
        // 层：背景(0), L0(1), L1(2), L2(3), L3(4), L4(5)。
        let g1 = doc.group_layers(&[1, 2], None).unwrap(); // L0, L1

        // 把 L4 归入组 1 并移到 idx3（L2 前面）——归组不动 z，落位由
        // MoveLayer 完成；一次调用一条历史。
        doc.move_layer_to_group_at(5, Some(g1), 3).unwrap();
        assert_eq!(doc.layers[3].name, "L4");
        assert_eq!(
            doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            vec!["背景", "L0", "L1", "L4", "L2", "L3"]
        );
        assert_eq!(doc.layers[3].group, Some(g1));
        assert_eq!(doc.group_members(g1), vec![1, 2, 3]);

        // 一次撤销同时恢复归属与 z 序（单条 Batch 的关键验收）。
        doc.undo().unwrap();
        assert_eq!(doc.layers[5].name, "L4");
        assert_eq!(doc.layers[5].group, None);
        assert_eq!(
            doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            vec!["背景", "L0", "L1", "L2", "L3", "L4"]
        );
        doc.redo().unwrap();
        assert_eq!(doc.layers[3].name, "L4");
        // 回到初始状态，继续下面的用例。
        doc.undo().unwrap();

        // to_z 越界 → 报错且无副作用（归属也不变）。
        assert_eq!(
            doc.move_layer_to_group_at(5, Some(g1), 99),
            Err(EditError::LayerIndex(99))
        );
        assert_eq!(doc.layers[5].group, None);

        // to_z == index → 只归组不落位（等价 set_layer_group）。
        doc.move_layer_to_group_at(5, Some(g1), 5).unwrap();
        assert_eq!(doc.layers[5].name, "L4");
        assert_eq!(doc.layers[5].group, Some(g1));
        doc.undo().unwrap();
        assert_eq!(doc.layers[5].group, None);

        // 拖进隐藏组：可见成员跟着隐藏（一次调用内完成）。
        doc.set_group_visible(g1, false).unwrap();
        doc.move_layer_to_group_at(5, Some(g1), 3).unwrap();
        assert!(!doc.layers[3].visible);
        doc.set_group_visible(g1, true).unwrap();
        assert!(doc.layers[3].visible);
    }

    #[test]
    fn move_group_to_group_at_combines_parent_and_z() {
        let mut doc = doc_with_layers(5);
        let g1 = doc.group_layers(&[1, 2], None).unwrap(); // L0, L1
        let g2 = doc.group_layers(&[3, 4], None).unwrap(); // L2, L3

        // 组 2 进组 1，同时块排到 L1（idx2）下面。
        doc.move_group_to_group_at(g2, Some(g1), Some((2, false)))
            .unwrap();
        assert_eq!(doc.find_group(g2).unwrap().parent, Some(g1));
        assert_eq!(
            doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            vec!["背景", "L0", "L2", "L3", "L1", "L4"]
        );

        // 一次撤销同时恢复 parent 与 z 序；重做再生效。
        doc.undo().unwrap();
        assert_eq!(doc.find_group(g2).unwrap().parent, None);
        assert_eq!(
            doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            vec!["背景", "L0", "L1", "L2", "L3", "L4"]
        );
        doc.redo().unwrap();
        assert_eq!(doc.find_group(g2).unwrap().parent, Some(g1));
        assert_eq!(doc.layers[2].name, "L2");

        // anchor=None → 只归组（等价 move_group_to_group）。
        doc.undo().unwrap();
        doc.move_group_to_group_at(g2, Some(g1), None).unwrap();
        assert_eq!(doc.find_group(g2).unwrap().parent, Some(g1));
        assert_eq!(
            doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            vec!["背景", "L0", "L1", "L2", "L3", "L4"]
        );

        // target=None → 只排序不改归属。
        doc.undo().unwrap();
        doc.move_group_to_group_at(g2, None, Some((2, false))).unwrap();
        assert_eq!(doc.find_group(g2).unwrap().parent, None);
        assert_eq!(doc.layers[2].name, "L2");

        // 已在目标父组 → 整体无操作（不进历史）。
        let epoch = doc.edit_epoch;
        doc.move_group_to_group_at(g2, None, None).unwrap();
        assert_eq!(doc.edit_epoch, epoch);
    }

    #[test]
    fn move_group_into_hidden_group_hides_subtree_members() {
        let mut doc = doc_with_layers(4);
        let g1 = doc.group_layers(&[1, 2], None).unwrap();
        let g2 = doc.group_layers(&[3, 4], None).unwrap();
        doc.set_group_visible(g1, false).unwrap(); // 组 1 隐藏（记 snapshot）
        doc.move_group_to_group(g2, Some(g1)).unwrap();
        // 移进隐藏组：可见成员跟着隐藏；组 1 再显示时按 snapshot 恢复。
        assert!(!doc.layers[3].visible && !doc.layers[4].visible);
        doc.set_group_visible(g1, true).unwrap();
        assert!(doc.layers[3].visible && doc.layers[4].visible);
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
            own_visible: true,
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
            synthetic: false,
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
                    raw_style: None,
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
                    raw_style: None,
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

    #[test]
    fn merge_group_turns_group_into_plain_layer_and_undoes() {
        let mut doc = doc_with_layers(3);
        let before_pixels = doc.flatten();
        let gid = doc.group_layers(&[1, 2], None).unwrap();
        assert_eq!(doc.groups.len(), 1);

        let at = doc.merge_group(gid).unwrap();
        // 组消失、结果层不带任何组归属、层数减一、落位在原最高成员处。
        assert!(doc.find_group(gid).is_none(), "组应被移除");
        assert!(doc.groups.is_empty());
        assert_eq!(doc.layers[at].group, None, "结果层应变成普通图层");
        assert_eq!(doc.layers.len(), 3, "2 层合成 1 层 + 背景 + 未组的 L2 = 3");
        assert_eq!(doc.flatten(), before_pixels, "合并前后画面必须一致");

        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 4, "撤销后层数复原");
        assert!(doc.find_group(gid).is_some(), "撤销后组复原");
        assert_eq!(doc.group_members(gid), vec![1, 2]);
        assert_eq!(doc.flatten(), before_pixels);
    }

    #[test]
    fn merge_group_with_nested_child_removes_both_groups() {
        let mut doc = doc_with_layers(3);
        let before_pixels = doc.flatten();
        // 先建外层（含 1/2/3），再在同一组内把 2/3 编成子组 —— 跨组混选
        // 会被 group_layers 拒绝，嵌套只能这样建。
        let outer = doc.group_layers(&[1, 2, 3], None).unwrap();
        let inner = doc.group_layers(&[2, 3], None).unwrap();
        assert_eq!(doc.find_group(inner).unwrap().parent, Some(outer));
        // 外层子树 = 层1 + 层2 + 层3（内层成员）。
        assert_eq!(doc.group_subtree_layers(outer).len(), 3);

        doc.merge_group(outer).unwrap();
        assert!(doc.find_group(outer).is_none());
        assert!(doc.find_group(inner).is_none(), "后代组也要一起删掉，不能留孤儿组");
        assert_eq!(doc.layers.len(), 2, "3 层合成 1 层 + 背景");
        assert_eq!(doc.flatten(), before_pixels);

        doc.undo().unwrap();
        assert!(doc.find_group(outer).is_some());
        assert!(doc.find_group(inner).is_some());
        assert_eq!(doc.flatten(), before_pixels);
    }
}
