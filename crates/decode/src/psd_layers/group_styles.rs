use std::collections::HashMap;

use super::PsdLayerData;

/// Bake the styles carried on **folder records** (`lfx2` on a group) into two
/// extra raster layers: the shadow below the group's lowest member, the
/// foreground effects (inner shadow / glow, overlays, stroke) above its
/// highest one.
///
/// Photoshop renders a group's styles once around the group's composite
/// outline — one ring around every child, not one per child. The group is a
/// contiguous run in the stack, so inserting the two rasters inside that run
/// composites exactly like Photoshop's group container would. Groups are
/// processed deepest first, so a parent's joint alpha already includes the
/// rings of the groups nested inside it.
///
/// Returns the number of rasters inserted and the degradation notes.
pub(super) fn bake_group_style_layers(
    layers: &mut Vec<PsdLayerData>,
    groups: &HashMap<u32, psd::PsdGroup>,
    canvas: (u32, u32),
) -> (usize, Vec<String>) {
    // Deepest group first: a parent drawn after its children must see the
    // children's baked rings in its own joint alpha.
    let depth_of = |mut id: u32| -> usize {
        let mut d = 0;
        while let Some(g) = groups.get(&id) {
            match g.parent_id() {
                Some(p) => {
                    id = p;
                    d += 1;
                    if d > 64 {
                        break;
                    }
                }
                None => break,
            }
        }
        d
    };
    let mut styled: Vec<&psd::PsdGroup> = groups
        .values()
        .filter(|g| {
            g.layer_effects()
                .map(|fx| fx.iter().any(|e| e.enabled))
                .unwrap_or(false)
        })
        .collect();
    styled.sort_by_key(|g| std::cmp::Reverse(depth_of(g.id())));

    let mut notes: Vec<String> = Vec::new();
    let mut inserted = 0usize;
    for group in styled {
        let Some(fx) = group.layer_effects() else { continue };
        let fx: Vec<psd::effects::LayerEffect> =
            fx.iter().filter(|e| e.enabled).cloned().collect();
        if fx.is_empty() {
            continue;
        }
        // The group and everything nested inside it: a member's `group` field
        // is the PSD id of its immediate parent, so collect the subtree first.
        let mut descendants: Vec<u32> = vec![group.id()];
        let mut changed = true;
        while changed {
            changed = false;
            for g in groups.values() {
                if let Some(p) = g.parent_id() {
                    if descendants.contains(&p) && !descendants.contains(&g.id()) {
                        descendants.push(g.id());
                        changed = true;
                    }
                }
            }
        }
        let Some(b) = members_union(layers, &descendants) else {
            continue;
        };
        let (w, h) = ((b.2 - b.0).max(1) as u32, (b.3 - b.1).max(1) as u32);
        if w as u64 * h as u64 > 30_000_000 {
            notes.push("组样式需要过大的画布，已跳过".into());
            continue;
        }
        // Joint alpha of the visible members, over-blended (alpha composition
        // is blend-mode independent — every PSD blend keeps `a_out = a_s +
        // a_b*(1-a_s)`). 成员只带自己的不透明度：组不透明度现在由合成阶段
        // 对整个组（连同这里烘焙出的样式层）应用一次，若在这里再乘一遍就是
        // 双重衰减；祖先组的样式同理交给各自的组处理。
        // Clipping members are skipped: their own bitmap reaches far past the
        // base they clip to, and only the base's coverage is visible — counting
        // the clipper whole would draw the group ring around a shape the
        // composite never shows.
        let mut alpha = vec![0.0f32; (w * h) as usize];
        for layer in layers.iter() {
            if !layer.visible || layer.opacity <= 0.0 || layer.clipping {
                continue;
            }
            let Some(g) = layer.group else { continue };
            if !descendants.contains(&g) {
                continue;
            }
            let (lw, lh) = layer.image.dimensions();
            let (lx, ly) = layer.offset;
            for y in 0..lh {
                let cy = ly + y as i32 - b.1;
                if cy < 0 || cy >= h as i32 {
                    continue;
                }
                for x in 0..lw {
                    let cx = lx + x as i32 - b.0;
                    if cx < 0 || cx >= w as i32 {
                        continue;
                    }
                    let a = layer.image.get_pixel(x, y).0[3] as f32 / 255.0 * layer.opacity;
                    if a <= 0.0 {
                        continue;
                    }
                    let i = (cy as u32 * w + cx as u32) as usize;
                    alpha[i] = a + alpha[i] * (1.0 - a);
                }
            }
        }
        if alpha.iter().all(|a| *a <= 0.0) {
            continue;
        }
        let Some(baked) =
            crate::psd_effects::bake_group_effects(&alpha, (w, h), (b.0, b.1), &fx)
        else {
            continue;
        };
        notes.extend(baked.notes);
        let group_visible = group_chain_visible(groups, group.id());
        let is_member =
            |l: &PsdLayerData| l.group.map(|g| descendants.contains(&g)).unwrap_or(false);
        // Rasters move as each is inserted, so look the run bounds up per one.
        if let Some(front) = baked.front {
            if let Some(last) = layers.iter().rposition(is_member) {
                layers.insert(
                    last + 1,
                    PsdLayerData {
                        annotation: false,
                        group: Some(group.id()),
                        name: format!("组「{}」样式(烘焙)", group.name()),
                        offset: baked.offset,
                        image: front,
                        visible: group_visible,
                        // 烘焙样式是组的「影子成员」：跟随组的显隐快照还原，
                        // 自身旗标恒真（否则外层组显隐一轮后样式层会丢）。
                        own_visible: true,
                        opacity: 1.0,
                        blend: editcore::BlendMode::Normal,
                        degraded: Vec::new(),
                        mask: None,
                        clipping: false,
                        content_alpha: None,
                        clip_shape: None,
                        style: None,
                        blend_if: None,
                        knockout: editcore::Knockout::None,
                        transparency_shapes: true,
                        synthetic: true,
                    },
                );
                inserted += 1;
            }
        }
        if let Some(behind) = baked.behind {
            if let Some(first) = layers.iter().position(is_member) {
                layers.insert(
                    first,
                    PsdLayerData {
                        annotation: false,
                        group: Some(group.id()),
                        name: format!("组「{}」投影(烘焙)", group.name()),
                        offset: baked.offset,
                        image: behind,
                        visible: group_visible,
                        // 同上：影子成员，自身旗标恒真。
                        own_visible: true,
                        opacity: 1.0,
                        // 组的投影带着它自己的混合模式（PSD `Md  `）：PS 里
                        // 投影与组下方的画面混合，当成 Normal 图层盖上去会把
                        // 下层染成投影色。
                        blend: baked.behind_blend,
                        degraded: Vec::new(),
                        mask: None,
                        clipping: false,
                        content_alpha: None,
                        clip_shape: None,
                        style: None,
                        blend_if: None,
                        knockout: editcore::Knockout::None,
                        transparency_shapes: true,
                        synthetic: true,
                    },
                );
                inserted += 1;
            }
        }
    }
    let _ = canvas;
    (inserted, notes)
}

/// Bounding box (left, top, right, bottom — exclusive right/bottom) of every
/// member's bitmap, `None` when the group paints nothing.
fn members_union(layers: &[PsdLayerData], descendants: &[u32]) -> Option<(i32, i32, i32, i32)> {
    let mut acc: Option<(i32, i32, i32, i32)> = None;
    for layer in layers {
        if !layer.visible || layer.opacity <= 0.0 || layer.clipping {
            continue;
        }
        let Some(g) = layer.group else { continue };
        if !descendants.contains(&g) {
            continue;
        }
        let (lw, lh) = layer.image.dimensions();
        if lw == 0 || lh == 0 {
            continue;
        }
        let (lx, ly) = layer.offset;
        let (l, t, r, btm) = (lx, ly, lx + lw as i32, ly + lh as i32);
        acc = Some(match acc {
            None => (l, t, r, btm),
            Some((al, at, ar, ab)) => (al.min(l), at.min(t), ar.max(r), ab.max(btm)),
        });
    }
    acc
}

/// Whether a group and all of its ancestors have the eye on.
fn group_chain_visible(groups: &HashMap<u32, psd::PsdGroup>, mut id: u32) -> bool {
    let mut guard = 0;
    loop {
        match groups.get(&id) {
            None => return true,
            Some(g) => {
                if !g.visible() {
                    return false;
                }
                match g.parent_id() {
                    Some(p) => {
                        id = p;
                        guard += 1;
                        if guard > 64 {
                            return true;
                        }
                    }
                    None => return true,
                }
            }
        }
    }
}
