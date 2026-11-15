use std::collections::HashMap;

/// Multiply a layer's own state by every ancestor group's, returning
/// `(visible, opacity)`.
///
/// Flattening the group tree loses the folder records, and a folder is where
/// Photoshop puts the "this whole variant is hidden" flag. Walking the chain
/// (`group_id -> parent group id`) keeps that information on the children.
/// The depth guard is there because a corrupt file could describe a cycle.
pub(super) fn inherited_group_state(
    groups: &HashMap<u32, psd::PsdGroup>,
    parent: Option<u32>,
) -> (bool, f32) {
    let mut visible = true;
    let mut opacity = 1.0f32;
    let mut next = parent;
    for _ in 0..64 {
        let Some(group) = next.and_then(|id| groups.get(&id)) else {
            break;
        };
        visible &= group.visible();
        opacity *= group.opacity() as f32 / 255.0;
        next = group.parent_id();
    }
    (visible, opacity)
}

/// Group names designers use for annotation / spec layers.
///
/// These folders hold measurements, colour codes and font specs — data for the
/// implementer, not part of the screen. They are matched on the *group* name so
/// a real text layer called "字号28，描边2xp" (very common in Chinese UI files,
/// the name records the styling) is never mistaken for a note.
const ANNOTATION_GROUP_KEYWORDS: &[&str] = &["标注", "备注", "注释", "批注", "说明", "切图"];

fn is_annotation_name(name: &str) -> bool {
    let name = name.trim();
    ANNOTATION_GROUP_KEYWORDS
        .iter()
        .any(|kw| name.contains(kw))
}

/// Whether any ancestor group of a layer is an annotation folder.
pub(super) fn ancestor_is_annotation_group(
    groups: &HashMap<u32, psd::PsdGroup>,
    parent: Option<u32>,
) -> bool {
    let mut next = parent;
    for _ in 0..64 {
        let Some(group) = next.and_then(|id| groups.get(&id)) else {
            break;
        };
        if is_annotation_name(group.name()) {
            return true;
        }
        next = group.parent_id();
    }
    false
}
