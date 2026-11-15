use super::*;
use image::Rgba;

struct TestLayer {
    name: &'static str,
    top: i32,
    left: i32,
    w: u32,
    h: u32,
    rgba: Vec<u8>,
    opacity: u8,
    visible: bool,
}

fn push_u16(v: &mut Vec<u8>, n: u16) {
    v.extend_from_slice(&n.to_be_bytes());
}
fn push_i16(v: &mut Vec<u8>, n: i16) {
    v.extend_from_slice(&n.to_be_bytes());
}
fn push_u32(v: &mut Vec<u8>, n: u32) {
    v.extend_from_slice(&n.to_be_bytes());
}
fn push_i32(v: &mut Vec<u8>, n: i32) {
    v.extend_from_slice(&n.to_be_bytes());
}
fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

/// `layers` is given in file order, i.e. bottom -> top: that is how a PSD
/// stores its records, and what `decode_psd_layers` must hand back.
fn make_psd(w: u32, h: u32, layers: &[TestLayer]) -> Vec<u8> {
    let mut records = Vec::new();
    let mut channels = Vec::new();
    for layer in layers.iter() {
        let n = (layer.w * layer.h) as usize;
        let planes: Vec<Vec<u8>> = (0..4)
            .map(|c| (0..n).map(|i| layer.rgba[i * 4 + c]).collect::<Vec<u8>>())
            .collect();
        let mut data = Vec::new();
        for plane in &planes {
            push_u16(&mut data, 0);
            data.extend_from_slice(plane);
        }
        let mut rec = Vec::new();
        push_i32(&mut rec, layer.top);
        push_i32(&mut rec, layer.left);
        push_i32(&mut rec, layer.top + layer.h as i32);
        push_i32(&mut rec, layer.left + layer.w as i32);
        push_u16(&mut rec, 4);
        for (id, plane) in [0i16, 1, 2, -1].iter().zip(planes.iter()) {
            push_i16(&mut rec, *id);
            push_u32(&mut rec, (2 + plane.len()) as u32);
        }
        rec.extend_from_slice(b"8BIMnorm");
        rec.push(layer.opacity);
        rec.push(0);
        // Adobe spec: bit 3 = Photoshop 5+, bit 1 SET = HIDDEN.
        rec.push(if layer.visible { 8 } else { 8 | 2 });
        rec.push(0);
        let mut extra = Vec::new();
        push_u32(&mut extra, 0);
        push_u32(&mut extra, 0);
        let name = layer.name.as_bytes();
        extra.push(name.len() as u8);
        extra.extend_from_slice(name);
        pad4(&mut extra);
        push_u32(&mut rec, extra.len() as u32);
        rec.extend_from_slice(&extra);
        records.extend_from_slice(&rec);
        channels.extend_from_slice(&data);
    }
    let mut info = Vec::new();
    push_u32(&mut info, (2 + records.len() + channels.len()) as u32);
    push_i16(&mut info, layers.len() as i16);
    info.extend_from_slice(&records);
    info.extend_from_slice(&channels);

    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    push_u16(&mut out, 1);
    out.extend_from_slice(&[0; 6]);
    push_u16(&mut out, 4);
    push_u32(&mut out, h);
    push_u32(&mut out, w);
    push_u16(&mut out, 8);
    push_u16(&mut out, 3);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, (info.len() + 4) as u32);
    out.extend_from_slice(&info);
    push_u32(&mut out, 0);
    push_u16(&mut out, 0);
    for c in 0..4 {
        out.extend(std::iter::repeat((c * 40 + 17) as u8).take((w * h) as usize));
    }
    out
}

fn solid(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..w * h {
        v.extend_from_slice(&color);
    }
    v
}

#[test]
fn parses_layers_with_attributes_and_pixels() {
    let bytes = make_psd(
        8,
        6,
        &[
            TestLayer {
                name: "bottom",
                top: 0,
                left: 0,
                w: 4,
                h: 3,
                rgba: solid(4, 3, [10, 20, 30, 255]),
                opacity: 255,
                visible: true,
            },
            TestLayer {
                name: "top",
                top: 1,
                left: 2,
                w: 3,
                h: 2,
                rgba: solid(3, 2, [200, 100, 50, 128]),
                opacity: 128,
                visible: false,
            },
        ],
    );
    let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("parse");
    assert_eq!(set.canvas, (8, 6));
    assert_eq!(set.layers.len(), 2);
    assert_eq!(set.layers[0].name, "bottom");
    assert_eq!(set.layers[0].offset, (0, 0));
    assert!((set.layers[0].opacity - 1.0).abs() < 1e-6);
    assert!(set.layers[0].visible);
    assert_eq!(set.layers[0].image.get_pixel(0, 0), &Rgba([10, 20, 30, 255]));
    assert_eq!(set.layers[1].name, "top");
    assert_eq!(set.layers[1].offset, (2, 1));
    assert!(!set.layers[1].visible);
    assert!((set.layers[1].opacity - 128.0 / 255.0).abs() < 1e-6);
    assert_eq!(set.layers[1].image.get_pixel(0, 0), &Rgba([200, 100, 50, 128]));
    assert_eq!(set.bytes, (4 * 3 + 3 * 2) * 4);
}

#[test]
fn clips_negative_offsets_and_out_of_bounds_layers() {
    let bytes = make_psd(
        6,
        4,
        &[
            TestLayer {
                name: "clipped",
                top: -2,
                left: -1,
                w: 5,
                h: 5,
                rgba: solid(5, 5, [7, 8, 9, 255]),
                opacity: 255,
                visible: true,
            },
            TestLayer {
                name: "outside",
                top: 20,
                left: 20,
                w: 2,
                h: 2,
                rgba: solid(2, 2, [1, 1, 1, 255]),
                opacity: 255,
                visible: true,
            },
        ],
    );
    let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("parse");
    // The fully-outside layer is dropped; the negative one is cropped.
    let clipped = set
        .layers
        .iter()
        .find(|l| l.name == "clipped")
        .expect("clipped layer");
    assert_eq!(clipped.offset, (0, 0));
    assert_eq!(clipped.image.dimensions(), (4, 3));
    assert!(!set.degraded.is_empty());
}

#[test]
fn memory_budget_is_per_layer_rectangle() {
    let small: Vec<TestLayer> = (0..10)
        .map(|i| TestLayer {
            name: "s",
            top: i * 9,
            left: i * 9,
            w: 10,
            h: 10,
            rgba: solid(10, 10, [i as u8, 0, 0, 255]),
            opacity: 255,
            visible: true,
        })
        .collect();
    let bytes = make_psd(100, 100, &small);
    let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("parse");
    assert_eq!(set.layer_count(), 10);
    // 10 * 10x10 layers = 4KB, NOT 10 * full canvas (400KB).
    assert_eq!(set.bytes, 10 * 10 * 10 * 4);
    assert!(set.bytes < 100 * 100 * 4);
}

#[test]
fn progress_reports_every_layer() {
    let bytes = make_psd(
        4,
        4,
        &[TestLayer {
            name: "a",
            top: 0,
            left: 0,
            w: 4,
            h: 4,
            rgba: solid(4, 4, [1, 2, 3, 255]),
            opacity: 255,
            visible: true,
        }],
    );
    let mut seen = Vec::new();
    decode_psd_layers_with_progress(&bytes, DecodeOptions::default(), |d, t| seen.push((d, t)))
        .expect("parse");
    assert_eq!(seen, vec![(0, 1), (1, 1)]);
}

#[test]
fn corrupt_section_length_is_an_error_not_a_panic() {
    let bytes = make_psd(
        4,
        4,
        &[TestLayer {
            name: "a",
            top: 0,
            left: 0,
            w: 4,
            h: 4,
            rgba: solid(4, 4, [1, 2, 3, 255]),
            opacity: 255,
            visible: true,
        }],
    );
    let mut bad = bytes.clone();
    bad[34..38].copy_from_slice(&0x7fff_ffffu32.to_be_bytes());
    assert!(decode_psd_layers(&bad, DecodeOptions::default()).is_err());
    // The composite-only path is unchanged (C1).
    let decoded = crate::decode_psd_bytes(&bytes, DecodeOptions::default()).expect("decode");
    assert_eq!(decoded.image.dimensions(), (4, 4));
    assert!(decoded.note.unwrap().contains("图层已合并"));
}

#[test]
fn vendored_fixtures_parse_without_panicking() {
    const FIXTURES: &[(&str, &[u8])] = &[
        ("green-1x1", include_bytes!("../../../../vendor/psd/tests/fixtures/green-1x1.psd")),
        (
            "chinese-name",
            include_bytes!("../../../../vendor/psd/tests/fixtures/green-chinese-layer-name-1x1.psd"),
        ),
        (
            "negative-offset",
            include_bytes!("../../../../vendor/psd/tests/fixtures/negative-top-left-layer.psd"),
        ),
        (
            "out-of-bounds",
            include_bytes!("../../../../vendor/psd/tests/fixtures/out-of-bounds-layer.psd"),
        ),
        (
            "rle-3-layer",
            include_bytes!("../../../../vendor/psd/tests/fixtures/rle-3-layer-8x8.psd"),
        ),
        (
            "one-channel",
            include_bytes!("../../../../vendor/psd/tests/fixtures/one-channel-1x1.psd"),
        ),
        (
            "non-utf8-name",
            include_bytes!("../../../../vendor/psd/tests/fixtures/non-utf8-pascal-string.psd"),
        ),
        (
            "two-layers",
            include_bytes!("../../../../vendor/psd/tests/fixtures/two-layers-red-green-1x1.psd"),
        ),
        (
            "transparent-above",
            include_bytes!("../../../../vendor/psd/tests/fixtures/transparent-above-opaque.psd"),
        ),
        (
            "odd-pascal",
            include_bytes!("../../../../vendor/psd/tests/fixtures/odd-length-pascal-string.psd"),
        ),
    ];
    // Collect every problem in one run instead of stopping at the first
    // fixture, so a single test run shows all compatibility gaps.
    let mut problems = Vec::new();
    for (name, bytes) in FIXTURES {
        let result = std::panic::catch_unwind(|| {
            decode_psd_layers(bytes, DecodeOptions::default())
        });
        match result {
            Err(_) => problems.push(format!("{name}: PANIC")),
            Ok(Err(e)) => {
                // 16-bit files are deliberately refused as layered
                // documents (§7 degradation contract).
                if *name != "one-channel" {
                    problems.push(format!("{name}: {e}"));
                }
            }
            Ok(Ok(set)) => {
                // Flat fixtures (no Layer & Mask section) legitimately
                // report zero layers; the caller keeps the composite.
                let flat_ok = matches!(*name, "non-utf8-name" | "odd-pascal");
                if set.layer_count() < 1 && !flat_ok {
                    problems.push(format!("{name}: no layers"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn unicode_layer_name_survives() {
    let bytes = include_bytes!("../../../../vendor/psd/tests/fixtures/green-chinese-layer-name-1x1.psd");
    let set = decode_psd_layers(bytes, DecodeOptions::default()).expect("parse");
    assert!(set.layers.iter().any(|l| l.name.chars().any(|c| c as u32 > 127)));
}

/// 新补齐的图层模式必须带着自己的模式进到 `PsdLayerData`，而不是被当成
/// Normal 悄悄降级（以前 `fsub`/`fdiv` 会走 `map_blend_mode` 的兜底）。
#[test]
fn extended_blend_modes_survive_decoding() {
    let cases: [(&[u8], editcore::BlendMode); 2] = [
        (
            include_bytes!("../../../../vendor/psd/tests/fixtures/blending/blue-red-1x1-subtract.psd"),
            editcore::BlendMode::Subtract,
        ),
        (
            include_bytes!("../../../../vendor/psd/tests/fixtures/blending/blue-red-1x1-divide.psd"),
            editcore::BlendMode::Divide,
        ),
    ];
    for (bytes, want) in cases {
        let set = decode_psd_layers(bytes, DecodeOptions::default()).expect("parse");
        let top = set.layers.iter().find(|l| l.name == "Top Layer").expect("top layer");
        assert_eq!(top.blend, want);
    }
}

#[test]
fn into_seeds_assembles_an_editable_document() {
    let bytes = make_psd(
        6,
        5,
        &[
            TestLayer {
                name: "bottom",
                top: 0,
                left: 0,
                w: 6,
                h: 5,
                rgba: solid(6, 5, [10, 20, 30, 255]),
                opacity: 255,
                visible: true,
            },
            TestLayer {
                name: "top",
                top: 1,
                left: 2,
                w: 2,
                h: 2,
                rgba: solid(2, 2, [90, 80, 70, 255]),
                opacity: 128,
                visible: false,
            },
        ],
    );
    let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("parse");
    let canvas = set.canvas;
    let doc = editcore::Document::from_psd_layers(canvas, set.into_seeds());
    assert_eq!(doc.layers.len(), 3);
    assert!(doc.layers[0].background);
    assert_eq!(doc.layers[1].name, "bottom");
    assert_eq!(doc.layers[2].name, "top");
    assert_eq!(doc.layers[2].offset(), (2, 1));
    assert!(!doc.layers[2].visible);
    assert!((doc.layers[2].opacity - 128.0 / 255.0).abs() < 1e-6);
}

/// One RLE channel: `rows` scanlines of `width` pixels. `truncate` keeps
/// only that many bytes of the body, which is how a broken row-length
/// table looks on disk.
fn packbits_channel(rows: u32, width: u32, value: u8, truncate: Option<usize>) -> Vec<u8> {
    let mut body = Vec::new();
    for _ in 0..rows {
        push_u16(&mut body, 2);
    }
    for _ in 0..rows {
        body.push((1i16 - width as i16) as i8 as u8);
        body.push(value);
    }
    if let Some(n) = truncate {
        body.truncate(n);
    }
    let mut out = Vec::new();
    push_u16(&mut out, 1); // RLE
    out.extend_from_slice(&body);
    out
}

/// 每行取不同值的通道：用来验证「行表按谁的高度剥」和「平面 stride」两件事。
/// 值 = 行号 * 10，任何错位都会在断言里立刻显形（均匀值则查不出来）。
fn packbits_channel_rows(rows: u32, width: u32) -> Vec<u8> {
    let mut body = Vec::new();
    for _ in 0..rows {
        push_u16(&mut body, 2);
    }
    for r in 0..rows {
        body.push((1i16 - width as i16) as i8 as u8);
        body.push((r * 10) as u8);
    }
    let mut out = Vec::new();
    push_u16(&mut out, 1); // RLE
    out.extend_from_slice(&body);
    out
}

/// A one-layer PSD with an optional user-mask channel. Photoshop stores the
/// mask rectangle separately from the layer rectangle, so the mask channel
/// legitimately has fewer scanlines than the layer — that combination used
/// to reject the whole file ("RLE 行长度表被截断").
fn make_psd_with_mask(side: u32, mask_side: Option<u32>, truncate_red: bool) -> Vec<u8> {
    let red = if truncate_red { Some(2) } else { None };
    let mut chans: Vec<(i16, Vec<u8>)> = vec![
        (-1, packbits_channel(side, side, 255, None)),
        (0, packbits_channel(side, side, 200, red)),
        (1, packbits_channel(side, side, 100, None)),
        (2, packbits_channel(side, side, 50, None)),
    ];
    let mask_len = match mask_side {
        Some(m) => {
            chans.push((-2, packbits_channel_rows(m, m)));
            20usize
        }
        None => 0,
    };

    let mut rec = Vec::new();
    push_i32(&mut rec, 0);
    push_i32(&mut rec, 0);
    push_i32(&mut rec, side as i32);
    push_i32(&mut rec, side as i32);
    push_u16(&mut rec, chans.len() as u16);
    for (id, data) in &chans {
        push_i16(&mut rec, *id);
        push_u32(&mut rec, data.len() as u32);
    }
    rec.extend_from_slice(b"8BIMnorm");
    rec.push(255);
    rec.push(0);
    rec.push(8); // bit 3 = Photoshop 5+, bit 1 clear = visible
    rec.push(0);
    let mut extra = Vec::new();
    push_u32(&mut extra, mask_len as u32);
    if let Some(m) = mask_side {
        push_i32(&mut extra, 0);
        push_i32(&mut extra, 0);
        // 与图层矩形同一约定：右下角存的是**边界**（宽 = right - left）。
        // 写 `m - 1` 会让解码端按「最后一个像素」理解，整块蒙版错一行一列
        // ——真实文件（`0302-界面UI-1.psd`）里 right 就是 left + 宽度。
        push_i32(&mut extra, m as i32);
        push_i32(&mut extra, m as i32);
        extra.push(0); // default colour
        extra.push(0); // flags
        extra.extend_from_slice(&[0, 0]);
    }
    push_u32(&mut extra, 0);
    extra.push(5);
    extra.extend_from_slice(b"masks");
    pad4(&mut extra);
    push_u32(&mut rec, extra.len() as u32);
    rec.extend_from_slice(&extra);

    let mut channels = Vec::new();
    for (_, data) in &chans {
        channels.extend_from_slice(data);
    }
    let mut info = Vec::new();
    push_u32(&mut info, (2 + rec.len() + channels.len()) as u32);
    push_i16(&mut info, 1);
    info.extend_from_slice(&rec);
    info.extend_from_slice(&channels);

    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    push_u16(&mut out, 1);
    out.extend_from_slice(&[0; 6]);
    push_u16(&mut out, 4);
    push_u32(&mut out, side);
    push_u32(&mut out, side);
    push_u16(&mut out, 8);
    push_u16(&mut out, 3);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, (info.len() + 4) as u32);
    out.extend_from_slice(&info);
    push_u32(&mut out, 0);
    push_u16(&mut out, 0);
    for c in 0..4u8 {
        out.extend(std::iter::repeat(c * 40 + 17).take((side * side) as usize));
    }
    out
}

#[test]
fn mask_channel_with_its_own_rect_is_accepted() {
    let bytes = make_psd_with_mask(8, Some(2), false);
    let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("mask channel parses");
    assert_eq!(set.layer_count(), 1);
    // A 2x2 mask on an 8x8 layer must not be reported as corruption.
    assert!(
        !set.degraded.iter().any(|n| n.contains("不完整")),
        "unexpected degrade notes: {:?}",
        set.degraded
    );
    // The mask itself must come out at its own size (2x2), not at the
    // layer's 8x8: the channel is compressed against the mask rectangle.
    let mask = set.layers[0].mask.as_ref().expect("用户蒙版应被保留");
    assert_eq!(mask.image.dimensions(), (2, 2));
    assert_eq!(mask.offset, (0, 0));
    // 行取值各不相同，所以「行表剥错 2 字节」或「stride 多一列」都会让
    // 这里对不上（实测真实文件的蒙版就错在这两处）。
    for y in 0..2u32 {
        assert_eq!(
            mask.image.get_pixel(0, y).0[0],
            (y * 10) as u8,
            "mask row {y} decoded at the wrong offset"
        );
    }
    assert_eq!(set.masked, 1);
}

/// A layer without `-2` must not sprout an empty mask (which would hide it).
#[test]
fn layer_without_mask_channel_has_no_mask() {
    let bytes = make_psd_with_mask(8, None, false);
    let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("parses");
    assert!(set.layers[0].mask.is_none());
    assert_eq!(set.masked, 0);
}

#[test]
fn truncated_channel_degrades_instead_of_failing_the_file() {
    let bytes = make_psd_with_mask(8, None, true);
    let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("其他图层仍可加载");
    assert_eq!(set.layer_count(), 1);
    assert!(
        set.degraded.iter().any(|n| n.contains("通道数据不完整")),
        "expected a degrade note, got {:?}",
        set.degraded
    );
}

/// Diagnostic benchmark for real user files (ignored by default):
///
/// ```text
/// set IMAGEEDIT_PROFILE_PSD=<file.psd>
/// cargo test -p imageedit-decode profile_real_psd -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn profile_real_psd() {
    let Ok(path) = std::env::var("IMAGEEDIT_PROFILE_PSD") else {
        return;
    };
    let t = std::time::Instant::now();
    let bytes = std::fs::read(&path).expect("read");
    println!("read      {:>8.1} ms  ({} MB)", ms(t), bytes.len() / 1_048_576);
    let t = std::time::Instant::now();
    let psd = psd::Psd::from_bytes(&bytes).expect("parse");
    println!("vendor    {:>8.1} ms  ({} layers)", ms(t), psd.layers().len());
    let t = std::time::Instant::now();
    let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("layers");
    println!(
        "full      {:>8.1} ms  ({} layers, {} MB)",
        ms(t),
        set.layer_count(),
        set.bytes / 1_048_576
    );
    let visible_masked = set.layers.iter().filter(|l| l.mask.is_some() && l.visible).count();
    let visible_clipped = set.layers.iter().filter(|l| l.clipping && l.visible).count();
    println!(
        "masks     {:>8} (visible {}), clipping {:>4} (visible {})",
        set.masked, visible_masked, set.clipped, visible_clipped
    );
    let t = std::time::Instant::now();
    let _ = decode_psd_layers(&bytes, DecodeOptions::default()).expect("layers");
    println!("full #2   {:>8.1} ms  (warm)", ms(t));
}

/// Per-layer transparency census for real user files (ignored by default).
///
/// A canvas that comes out blank while the panel lists hundreds of layers
/// almost always means the layers decoded to fully transparent bitmaps
/// (vector / text / smart-object layers carry no channel data of their
/// own). This prints the numbers instead of guessing.
///
/// ```text
/// set IMAGEEDIT_INSPECT_PSD=<file.psd>
/// cargo test -p imageedit-decode inspect_real_psd -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn inspect_real_psd() {
    let Ok(path) = std::env::var("IMAGEEDIT_INSPECT_PSD") else {
        return;
    };
    let bytes = std::fs::read(&path).expect("read");
    let set = decode_psd_layers(&bytes, DecodeOptions::default()).expect("layers");
    println!(
        "canvas {:?}  layers {}  composite {}",
        set.canvas,
        set.layer_count(),
        set.has_composite
    );
    if let Ok(psd) = psd::Psd::from_bytes(&bytes) {
        let groups = psd.groups();
        let mut in_group = 0usize;
        let mut hidden_inherited = 0usize;
        for l in psd.layers() {
            if let Some(gid) = l.parent_id() {
                in_group += 1;
                let (vis, op) = inherited_group_state(groups, Some(gid));
                if !vis || op < 0.999 {
                    hidden_inherited += 1;
                }
            }
        }
        println!(
            "groups {}  layers_in_group {}  inherited_hidden_or_faded {}",
            groups.len(),
            in_group,
            hidden_inherited
        );
    }
    let mut empty = 0usize;
    let mut visible_empty = 0usize;
    let mut rows: Vec<(u64, String, (u32, u32), (i32, i32), bool, f32)> = Vec::new();
    for l in &set.layers {
        let nz = l.image.pixels().filter(|p| p[3] > 8).count() as u64;
        if nz == 0 {
            empty += 1;
            if l.visible {
                visible_empty += 1;
            }
        }
        rows.push((nz, l.name.clone(), l.image.dimensions(), l.offset, l.visible, l.opacity));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    println!("全透明层 {empty} / {}（其中可见 {visible_empty}）", set.layers.len());
    for r in rows.iter().take(12) {
        println!(
            "  {:>9} px  {}  {:?} off={:?} vis={} op={:.2}",
            r.0, r.1, r.2, r.3, r.4, r.5
        );
    }
    println!("degraded: {:?}", set.degraded);
    println!(
        "前5: {:?}",
        set.layers.iter().take(5).map(|l| l.name.clone()).collect::<Vec<_>>()
    );
    println!(
        "后5: {:?}",
        set.layers.iter().rev().take(5).map(|l| l.name.clone()).collect::<Vec<_>>()
    );

    // Colour census for the biggest layers: all-white RGBA would composite
    // to a blank canvas even though the alpha mask is perfectly fine.
    for r in rows.iter().take(4) {
        if let Some(l) = set.layers.iter().find(|l| l.name == r.1 && l.image.dimensions() == r.2) {
            let mut min = [255u8; 4];
            let mut max = [0u8; 4];
            let mut sum = [0u64; 4];
            let n = l.image.pixels().count() as u64;
            for p in l.image.pixels() {
                for c in 0..4 {
                    min[c] = min[c].min(p[c]);
                    max[c] = max[c].max(p[c]);
                    sum[c] += p[c] as u64;
                }
            }
            let mean: Vec<u64> = (0..4).map(|c| sum[c] / n.max(1)).collect();
            let center = l.image.get_pixel(l.image.width() / 2, l.image.height() / 2);
            println!(
                "  [{}] {:?} min={:?} max={:?} mean={:?} center={:?}",
                l.name, l.image.dimensions(), min, max, mean, center
            );
        }
    }

    // TEMP diagnostic: dump one layer by name so its pixels can be compared
    // with the reference renderer.
    if let Ok(want) = std::env::var("IMAGEEDIT_DUMP_LAYER") {
        for l in &set.layers {
            if l.name == want {
                let name = format!("C:/_APP_CODE/our_{}.png", want.replace(' ', "_"));
                l.image.save(&name).expect("dump layer");
                println!("dump -> {name} {:?} offset {:?}", l.image.dimensions(), l.offset);
            }
        }
    }

    // Composite through the **real** pipeline (core flatten, blend modes
    // included) so this file is directly comparable with the composite
    // Photoshop baked into the PSD. A hand-rolled source-over loop here used
    // to hide every blend-mode problem.
    let doc = editcore::Document::from_psd_layers(set.canvas, set.clone().into_seeds());
    let out = doc.flatten();
    let path = std::path::Path::new(&path)
        .file_stem()
        .map(|s| std::path::PathBuf::from(format!("C:/_APP_CODE/composite-{}.png", s.to_string_lossy())))
        .unwrap_or_else(|| std::path::PathBuf::from("C:/_APP_CODE/composite-check.png"));
    out.save(&path).expect("save composite");
    println!("composite -> {}", path.display());
}

fn ms(t: std::time::Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
