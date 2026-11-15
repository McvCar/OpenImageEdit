/// Exposed for the diagnostic test below.
#[cfg(test)]
pub(crate) fn parse_layer_effects_for_debug(bytes: &[u8]) -> Option<Vec<psd::effects::LayerEffect>> {
    psd::effects::parse_layer_effects(bytes)
}

#[cfg(test)]
mod debug_probe {
    /// Diagnostic: parse every `vstk` block in a file and report what the
    /// shape-stroke reader accepts.
    ///
    /// ```text
    /// set IMAGEEDIT_INSPECT_PSD=<file.psd>
    /// cargo test -p imageedit-decode debug_vstk -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn debug_vstk() {
        let Ok(path) = std::env::var("IMAGEEDIT_INSPECT_PSD") else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let (mut blocks, mut parsed) = (0, 0);
        let mut i = 0usize;
        while let Some(off) = find(&bytes[i..], b"vstk") {
            let at = i + off;
            i = at + 4;
            let Some(len_bytes) = bytes.get(at + 4..at + 8) else { break };
            let len = u32::from_be_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
            let Some(block) = bytes.get(at + 8..at + 8 + len) else { continue };
            blocks += 1;
            match psd::vector::parse_vector_stroke(block) {
                Some(s) => {
                    parsed += 1;
                    if parsed <= 4 {
                        println!(
                            "ok fill_enabled={} stroke_enabled={} width={} align={:?} opacity={} solid={:?} grad={}",
                            s.fill_enabled, s.enabled, s.width, s.align, s.opacity, s.fill.solid, s.fill.gradient.is_some()
                        );
                    }
                }
                None => {
                    if blocks <= 4 {
                        println!("fail head={:?}", &block[..block.len().min(20)]);
                    }
                }
            }
        }
        println!("vstk blocks {blocks} parsed {parsed}");
    }

    /// Diagnostic: dump the contour (`Trns`) sub-descriptors of every `lfx2`
    /// block, so the renderer can be written against the shapes the target
    /// file actually carries.
    ///
    /// ```text
    /// set IMAGEEDIT_INSPECT_PSD=<file.psd>
    /// cargo test -p imageedit-decode debug_lfx2_contours -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn debug_lfx2_contours() {
        use psd::descriptor::{parse_block2, Descriptor, DescriptorValue};
        let Ok(path) = std::env::var("IMAGEEDIT_INSPECT_PSD") else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let (mut blocks, mut effects, mut with_contour, mut non_linear) = (0, 0, 0, 0);
        let mut shapes: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut printed = 0;
        let mut i = 0usize;
        while let Some(off) = find(&bytes[i..], b"lfx2") {
            let at = i + off;
            i = at + 4;
            let Some(len_bytes) = bytes.get(at + 4..at + 8) else { break };
            let len = u32::from_be_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
            let Some(block) = bytes.get(at + 8..at + 8 + len) else { continue };
            let Some(root) = parse_block2(block) else { continue };
            blocks += 1;
            for (key, value) in &root.items {
                let DescriptorValue::Descriptor(d) = value else { continue };
                effects += 1;
                let key_s = String::from_utf8_lossy(key).to_string();
                if seen_keys.insert(key_s.clone()) {
                    println!(
                        "keys on {}: {:?}",
                        key_s,
                        d.items
                            .iter()
                            .map(|(k, v)| {
                                let t = match v {
                                    DescriptorValue::Descriptor(_) => "Objc",
                                    DescriptorValue::List(_) => "VlLs",
                                    DescriptorValue::Unit { .. } => "UntF",
                                    DescriptorValue::Double(_) => "doub",
                                    DescriptorValue::Integer(_) => "long",
                                    DescriptorValue::Bool(_) => "bool",
                                    DescriptorValue::Text(_) => "TEXT",
                                    DescriptorValue::Enum { .. } => "enum",
                                    DescriptorValue::Opaque => "opaque",
                                };
                                format!("{}:{t}", String::from_utf8_lossy(k))
                            })
                            .collect::<Vec<_>>()
                    );
                }
                let contour = d
                    .get(b"TrnS")
                    .or_else(|| d.get(b"Trns"))
                    .and_then(|v| match v {
                        DescriptorValue::Descriptor(c) => Some(c),
                        _ => None,
                    });
                let Some(c) = contour else { continue };
                with_contour += 1;
                let pts = contour_points(c);
                if pts.len() > 2 || pts.iter().any(|(x, y)| (x - y).abs() > 0.02) {
                    non_linear += 1;
                }
                let key_s = format!(
                    "{}/{} pts={}",
                    String::from_utf8_lossy(key),
                    String::from_utf8_lossy(&c.class_id),
                    pts.len()
                );
                *shapes.entry(key_s).or_insert(0) += 1;
                if printed < 6 && (pts.len() > 2 || pts.iter().any(|(x, y)| (x - y).abs() > 0.02)) {
                    printed += 1;
                    println!(
                        "contour on {} children {:?} pts {:?}",
                        String::from_utf8_lossy(key),
                        c.items.iter().map(|(k, _)| String::from_utf8_lossy(k).to_string()).collect::<Vec<_>>(),
                        pts
                    );
                }
            }
        }
        println!("lfx2 blocks {blocks} effects {effects} with-contour {with_contour} non-linear {non_linear}");
        let mut list: Vec<_> = shapes.into_iter().collect();
        list.sort_by(|a, b| b.1.cmp(&a.1));
        for (k, v) in list {
            println!("{v:>5}  {k}");
        }
        let _ = (parse_block2 as fn(&[u8]) -> Option<Descriptor>, &mut printed);
    }

    /// Normalised (x, y) control points of a contour descriptor (`Crv ` list of
    /// `Hrzn` / `Vrtc` pairs; Photoshop's plane is 0..255 with y down).
    fn contour_points(c: &psd::descriptor::Descriptor) -> Vec<(f32, f32)> {
        use psd::descriptor::DescriptorValue;
        let mut out = Vec::new();
        let Some(list) = c.list(b"Crv ") else { return out };
        for item in list {
            let DescriptorValue::Descriptor(p) = item else { continue };
            let (Some(x), Some(y)) = (p.number(b"Hrzn"), p.number(b"Vrtc")) else { continue };
            out.push((x as f32, y as f32));
        }
        out
    }

    /// Diagnostic: parse every `lfx2` block in a file and report how many the
    /// descriptor reader accepts.
    ///
    /// ```text
    /// set IMAGEEDIT_INSPECT_PSD=<file.psd>
    /// cargo test -p imageedit-decode debug_lfx2 -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn debug_lfx2() {
        let Ok(path) = std::env::var("IMAGEEDIT_INSPECT_PSD") else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let (mut seen, mut ok, mut total_effects) = (0usize, 0usize, 0usize);
        let mut i = 0usize;
        while let Some(off) = find(&bytes[i..], b"lfx2") {
            let at = i + off;
            i = at + 4;
            let Some(len_bytes) = bytes.get(at + 4..at + 8) else {
                break;
            };
            let len = u32::from_be_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
            let Some(block) = bytes.get(at + 8..at + 8 + len) else {
                continue;
            };
            seen += 1;
            match super::parse_layer_effects_for_debug(block) {
                Some(effects) => {
                    ok += 1;
                    total_effects += effects.len();
                }
                None => {
                    if seen <= 3 {
                        println!("block at {at} len {len} head {:02x?}", &block[..block.len().min(24)]);
                    }
                }
            }
        }
        println!("lfx2 blocks {seen} parsed {ok} effects {total_effects}");
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

}
