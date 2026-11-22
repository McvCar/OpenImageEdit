//! 组样式的**运行时重烘焙**（影子层跟随刷新）。
//!
//! 加载时组样式（folder 记录上的 `lfxs` 等）被烘焙成两张影子层：投影垫在
//! 组成员下面、前景样式盖在上面（见 [`super::group_styles`]）。那只是加载
//! 瞬间的快照 —— 之后用户移动组内图层、开关成员眼睛、改不透明度、把成员
//! 拖出/拖入组，影子层还钉在原地。这里按成员**当前状态**重算联合 alpha
//! 并重新烘焙，原位替换影子层的位图。
//!
//! 约定：
//! * 影子层是**派生视图**：重烘焙直接改文档像素，不进撤销历史、不算
//!   dirty。撤销/重做成员操作后文档 epoch 已变，重跑一遍自然还原。
//! * 自身组的影子层不参与轮廓 alpha（否则环会把旧环再包一圈，越滚越大）；
//!   嵌套子组的影子层照常计入（与加载时「最深组先烘焙」的约定一致）。
//! * 效果参数从 [`editcore::LayerGroup::raw_style`] 的原始字节现场解析 ——
//!   与保存透传共用同一份字节，不需要另存一份效果规格。
//!   ⚠ 加载烘焙用的是该记录**全部**效果块的合并结果，这里只解析捕获的
//!   最优先块（`lfxs` > `lfx2` > `lmfx`）；对同时携带多代效果块且内容冲突
//!   的记录，重烘焙结果可能与加载快照略有出入（PS 自己也只读最新代）。

use std::collections::HashMap;

use image::RgbaImage;
use psd::effects::LayerEffect;

use editcore::{Document, GroupId};

/// 每个带影子层的组一个联合 alpha 指纹。只有 alpha 真的变了才重新烘焙
/// （模糊渲染不便宜）。由调用方（session）持有，跨刷新存活。
pub type RebakeHashes = HashMap<GroupId, u64>;

/// 单个样式效果块的解析：剥掉 12 字节头（8BIM 签名 + 键 + 声明长度），
/// 剩下的载荷与加载时 [`psd::effects::parse_layer_effects`] 吃到的完全
/// 一致（捕获时就是按「头 + 这段数据」整体扣下来的）。
fn parse_raw_style(raw: &[u8]) -> Option<Vec<LayerEffect>> {
    if raw.len() < 12 {
        return None;
    }
    let len = u32::from_be_bytes([raw[8], raw[9], raw[10], raw[11]]) as usize;
    let end = 12usize.saturating_add(len).min(raw.len());
    psd::effects::parse_layer_effects(&raw[12..end])
}

/// 对文档里所有带影子层的组跑一遍重烘焙。`hashes` 里**没有**的组只记
/// 指纹不烘焙 —— 加载时的烘焙就是基线（首帧不做一次无谓的全量重烘焙）。
pub fn rebake_group_styles(doc: &mut Document, hashes: &mut RebakeHashes) -> usize {
    // 最深的组先处理：父组的联合 alpha 要包含子组刚烘焙出的新环。
    let mut styled: Vec<GroupId> = doc
        .groups
        .iter()
        .filter(|g| g.raw_style.is_some())
        .map(|g| g.id)
        .collect();
    styled.sort_by_key(|&id| std::cmp::Reverse(doc.group_depth(id)));

    let mut updated = 0usize;
    for gid in styled {
        // 纯透传组（加载时没有烘焙出影子层）没有可刷新的替身。
        if !doc.layers.iter().any(|l| l.synthetic && l.group == Some(gid)) {
            continue;
        }
        let Some((alpha, size, offset)) = group_joint_alpha(doc, gid) else {
            continue;
        };
        let hash = alpha_hash(&alpha);
        match hashes.get(&gid) {
            // 没变：什么都不做。
            Some(prev) if *prev == hash => continue,
            // 首见：只记基线，不烘焙（加载快照即基线）。
            None => {
                hashes.insert(gid, hash);
                continue;
            }
            _ => {}
        }
        hashes.insert(gid, hash);
        // 效果参数从 raw_style 原始字节现场解析（与保存透传共用一份字节）。
        // 解析失败 = 这组的样式本来就没烘出来（加载时同样返回 None），跳过。
        let fx = match doc
            .find_group(gid)
            .and_then(|g| g.raw_style.as_deref())
            .and_then(parse_raw_style)
        {
            Some(fx) => fx,
            None => continue,
        };
        let Some(baked) = crate::psd_effects::bake_group_effects(&alpha, size, offset, &fx)
        else {
            continue;
        };
        if apply_group_bake(doc, gid, baked) {
            updated += 1;
        }
    }
    updated
}

/// 组成员（子树，含嵌套子组的影子环，不含本组自己的影子层）当前的联合
/// alpha，以及它的画布外接框。全空 / 超大 / 没有成员返回 `None`。
fn group_joint_alpha(
    doc: &Document,
    gid: GroupId,
) -> Option<(Vec<f32>, (u32, u32), (i32, i32))> {
    let descendants = doc.descendant_groups(gid);
    let is_member = |l: &editcore::Layer| {
        // 本组自己的环不算轮廓（否则旧环把自己包进新环，越滚越大）。
        if l.synthetic && l.group == Some(gid) {
            return false;
        }
        l.group
            .map(|g| g == gid || descendants.contains(&g))
            .unwrap_or(false)
    };
    let usable = |l: &editcore::Layer| {
        is_member(l) && l.visible && l.opacity > 0.0 && !l.clipping
    };
    let mut acc: Option<(i32, i32, i32, i32)> = None;
    for l in doc.layers.iter() {
        if !usable(l) {
            continue;
        }
        let Some(r) = l.raster() else { continue };
        let (lw, lh) = r.image.dimensions();
        if lw == 0 || lh == 0 {
            continue;
        }
        let (lx, ly) = r.offset;
        let rect = (lx, ly, lx + lw as i32, ly + lh as i32);
        acc = Some(match acc {
            None => rect,
            Some((al, at, ar, ab)) => {
                (al.min(rect.0), at.min(rect.1), ar.max(rect.2), ab.max(rect.3))
            }
        });
    }
    let (bx, by, br, bb) = acc?;
    let (w, h) = ((br - bx).max(1) as u32, (bb - by).max(1) as u32);
    if w as u64 * h as u64 > 30_000_000 {
        return None;
    }
    let mut alpha = vec![0.0f32; w as usize * h as usize];
    // 与加载时一致：alpha 是 blend 无关的过混合（a_out = a_s + a_b(1-a_s)），
    // 成员只带自己的不透明度（组不透明度由合成阶段对整组应用一次）。
    for l in doc.layers.iter() {
        if !usable(l) {
            continue;
        }
        let Some(r) = l.raster() else { continue };
        let (lw, lh) = r.image.dimensions();
        let (lx, ly) = r.offset;
        for y in 0..lh {
            let cy = ly + y as i32 - by;
            if cy < 0 || cy >= h as i32 {
                continue;
            }
            for x in 0..lw {
                let cx = lx + x as i32 - bx;
                if cx < 0 || cx >= w as i32 {
                    continue;
                }
                let a = r.image.get_pixel(x, y).0[3] as f32 / 255.0 * l.opacity;
                if a <= 0.0 {
                    continue;
                }
                let i = cy as u32 * w + cx as u32;
                alpha[i as usize] = a + alpha[i as usize] * (1.0 - a);
            }
        }
    }
    Some((alpha, (w, h), (bx, by)))
}

/// 把烘焙结果原位写回该组的影子层：子树成员区间之下最低的那张影子层当
/// 「投影」，之上最高的那张当前景样式。缺失的一侧清成 1×1 透明（效果集
/// 不变时两侧的存活性本就稳定，这里只是兜底）。影子层不参与 z 重排 ——
/// 任何索引变更都要走历史 op，派生刷新绝不碰层序。
fn apply_group_bake(
    doc: &mut Document,
    gid: GroupId,
    baked: crate::psd_effects::GroupBaked,
) -> bool {
    let descendants = doc.descendant_groups(gid);
    let own = |l: &editcore::Layer| l.synthetic && l.group == Some(gid);
    let mut run_lo: Option<usize> = None;
    let mut run_hi: Option<usize> = None;
    for (i, l) in doc.layers.iter().enumerate() {
        if own(l) {
            continue;
        }
        let member = l
            .group
            .map(|g| g == gid || descendants.contains(&g))
            .unwrap_or(false);
        if member {
            run_lo.get_or_insert(i);
            run_hi = Some(i);
        }
    }
    let syn: Vec<usize> = (0..doc.layers.len())
        .filter(|&i| own(&doc.layers[i]))
        .collect();
    if syn.is_empty() {
        return false;
    }
    let mut behind_i: Option<usize> = None;
    let mut front_i: Option<usize> = None;
    for &i in &syn {
        if run_lo.map_or(true, |lo| i < lo) && behind_i.is_none() {
            behind_i = Some(i);
        }
        if run_hi.map_or(true, |hi| i > hi) {
            front_i = Some(i);
        }
    }
    let mut changed = false;
    if let Some(i) = behind_i {
        let layer = &mut doc.layers[i];
        if let Some(r) = layer.raster_mut() {
            r.image = baked.behind.unwrap_or_else(|| RgbaImage::new(1, 1));
            r.offset = baked.offset;
            r.transform = None;
        }
        // 组的投影带着它自己的混合模式（PSD `Md  `），与加载时一致。
        layer.blend = baked.behind_blend;
        changed = true;
    }
    if let Some(i) = front_i {
        let layer = &mut doc.layers[i];
        if let Some(r) = layer.raster_mut() {
            r.image = baked.front.unwrap_or_else(|| RgbaImage::new(1, 1));
            r.offset = baked.offset;
            r.transform = None;
        }
        changed = true;
    }
    changed
}

/// 联合 alpha 的 FNV-1a 指纹（f32 位模式逐个混入）。
fn alpha_hash(alpha: &[f32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in alpha {
        h = (h ^ v.to_bits() as u64).wrapping_mul(0x100_0000_01b3);
        // 每个值再混一轮盐：单圈 FNV 对相邻相似值（大量 0.0 / 1.0）扩散不足。
        h = (h ^ 0x9e37_79b9_7f4a_7c15).wrapping_mul(0x100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use editcore::{BlendMode, PsdGroupSeed, PsdLayerSeed};

    fn solid(w: u32, h: u32, c: [u8; 4]) -> image::RgbaImage {
        image::RgbaImage::from_fn(w, h, |_, _| image::Rgba(c))
    }

    fn seed(name: &str, group: Option<u32>, synthetic: bool) -> PsdLayerSeed {
        PsdLayerSeed {
            name: name.into(),
            offset: (0, 0),
            image: solid(4, 4, [10, 20, 30, 255]),
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
            knockout: editcore::Knockout::None,
            transparency_shapes: true,
            synthetic,
        }
    }

    fn group(raw: Option<Vec<u8>>) -> PsdGroupSeed {
        PsdGroupSeed {
            psd_id: 1,
            parent: None,
            name: "组".into(),
            collapsed: false,
            visible: true,
            blend: BlendMode::Normal,
            opacity: 1.0,
            pass_through: false,
            raw_style: raw,
        }
    }

    /// 联合 alpha：只认成员位图，本组影子层与隐藏成员不计入。
    #[test]
    fn joint_alpha_excludes_own_shadow_and_hidden_members() {
        let doc = Document::from_psd_layers_with_groups(
            (8, 8),
            vec![
                seed("组外", None, false),
                seed("成员", Some(1), false),
                {
                    let mut s = seed("隐藏成员", Some(1), false);
                    s.visible = false;
                    s
                },
                seed("影子", Some(1), true),
            ],
            vec![group(Some(vec![9; 20]))],
        );
        let gid = doc.groups[0].id;
        let (alpha, (w, h), _) = group_joint_alpha(&doc, gid).expect("alpha");
        assert_eq!((w, h), (4, 4));
        // 4×4 成员全是 alpha 255 → 联合 alpha 全 1.0。
        assert!(alpha.iter().all(|a| (*a - 1.0).abs() < 1e-4));
    }

    /// 首见只记基线不烘焙；成员变化后真正烘焙并原位替换影子层。
    #[test]
    fn rebake_updates_shadow_after_member_change() {
        // 伪造一个解析不出效果的 lfxs 块：bake_group_effects 拿不到活动
        // 效果会返回 None —— 这里只验证「指纹门控 + 影子层判定」的流程，
        // 效果渲染本身的正确性由加载路径与真实 PSD 手测覆盖。
        let mut fx = Vec::new();
        fx.extend_from_slice(b"8BIM");
        fx.extend_from_slice(b"lfxs");
        fx.extend_from_slice(&16u32.to_be_bytes());
        fx.extend_from_slice(&[0u8; 16]);

        let mut doc = Document::from_psd_layers_with_groups(
            (8, 8),
            vec![
                seed("影子", Some(1), true),
                seed("成员", Some(1), false),
            ],
            vec![group(Some(fx))],
        );
        let gid = doc.groups[0].id;
        let mut hashes = RebakeHashes::new();
        // 首见：只记基线。
        assert_eq!(rebake_group_styles(&mut doc, &mut hashes), 0);
        assert!(hashes.contains_key(&gid));
        // 成员没变：指纹命中，不烘焙。
        assert_eq!(rebake_group_styles(&mut doc, &mut hashes), 0);
        // 移动成员：alpha 变 → 触发烘焙（效果块解析失败 → 无更新，但指纹
        // 已刷新；再跑一遍不再触发）。
        doc.layers[2].set_offset((2, 2));
        let _ = rebake_group_styles(&mut doc, &mut hashes);
        assert_eq!(rebake_group_styles(&mut doc, &mut hashes), 0);
        // 成员移回原位：指纹回到第一版 → 与当前记录值不同 → 再次触发。
        doc.layers[2].set_offset((0, 0));
        let _ = rebake_group_styles(&mut doc, &mut hashes);
    }

    /// 影子层判定：子树成员区间之下 / 之上各认一张，内容与 offset 原位
    /// 替换，z 序不动。
    #[test]
    fn apply_group_bake_replaces_in_place() {
        let doc = Document::from_psd_layers_with_groups(
            (8, 8),
            vec![
                seed("影子后", Some(1), true),
                seed("底层组员", Some(1), false),
                seed("顶层组员", Some(1), false),
                seed("影子前", Some(1), true),
            ],
            vec![group(None)],
        );
        let gid = doc.groups[0].id;
        // layers（底→顶）: 背景0, 影子后1, 底层组员2, 顶层组员3, 影子前4。
        let syn: Vec<usize> = (0..doc.layers.len())
            .filter(|&i| doc.layers[i].synthetic)
            .collect();
        assert_eq!(syn, vec![1, 4]);

        let baked = crate::psd_effects::GroupBaked {
            behind: Some(solid(6, 6, [255, 0, 0, 120])),
            front: None,
            behind_blend: BlendMode::Multiply,
            offset: (-1, -1),
            notes: Vec::new(),
        };
        let mut doc2 = doc.clone();
        assert!(apply_group_bake(&mut doc2, gid, baked));
        // z 序不动：synthetic 仍在 1/4。
        let syn2: Vec<usize> = (0..doc2.layers.len())
            .filter(|&i| doc2.layers[i].synthetic)
            .collect();
        assert_eq!(syn2, vec![1, 4]);
        // 影子后（1）拿到 behind 图 + Multiply；影子前（4）front 缺失 → 清空。
        let r1 = doc2.layers[1].raster().unwrap();
        assert_eq!(r1.offset, (-1, -1));
        assert_eq!(doc2.layers[1].blend, BlendMode::Multiply);
        assert_eq!(r1.image.dimensions(), (6, 6));
        let r4 = doc2.layers[4].raster().unwrap();
        assert_eq!(r4.image.dimensions(), (1, 1));
        assert_eq!(r4.image.get_pixel(0, 0).0[3], 0);
        // transform 被清掉（影子层不该带非破坏参数）。
        assert!(doc2.layers[1].raster().unwrap().transform.is_none());
    }
}
