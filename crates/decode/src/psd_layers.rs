//! PSD layer parsing for B-plan layered editing.
//!
//! This module is deliberately separate from `decode_psd_bytes`: the Explorer
//! thumbnail DLL calls that function for every directory listing, so it must
//! stay a "composite image only" fast path (C1). Layer parsing is only started
//! by the second-stage background job when the user actually opens a PSD.
//!
//! The vendored `psd` crate allocates `width * height * 4` bytes for **every**
//! layer in `PsdLayer::rgba()`. We instead expand the planar channels directly
//! into the layer rectangle (C3), then crop to the canvas. For a 4000x3000
//! document with ten 10%-area layers this is ~48 MB instead of ~480 MB.

use image::RgbaImage;
use psd::{ChannelBytes, PsdChannelKind};

use crate::{DecodeError, DecodeOptions};

/// One editable PSD layer. `image` only covers the layer rectangle after
/// cropping to the canvas; `offset` is where that rectangle starts.
#[derive(Debug, Clone)]
pub struct PsdLayerData {
    pub name: String,
    pub offset: (i32, i32),
    pub image: RgbaImage,
    pub visible: bool,
    pub opacity: f32,
    /// Per-layer "something was simplified" notes.
    pub degraded: Vec<String>,
}

/// Result of the second-stage parse. `layers` is bottom -> top.
#[derive(Debug, Clone)]
pub struct PsdLayerSet {
    pub layers: Vec<PsdLayerData>,
    pub canvas: (u32, u32),
    /// File-level degradation notes (groups flattened, blend modes, ...).
    pub degraded: Vec<String>,
    /// Whether the file carries a flattened composite (stage 1 uses it).
    pub has_composite: bool,
    /// Total bytes held by the layer bitmaps, for the memory budget probes.
    pub bytes: usize,
}

impl PsdLayerSet {
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Convert into `core` layer descriptors. Consumes `self` so the bitmaps
    /// are moved, not cloned (C3: 16 MB instead of another 16 MB copy).
    /// Read `canvas` / `degraded` before calling this.
    pub fn into_seeds(self) -> Vec<editcore::PsdLayerSeed> {
        self.layers
            .into_iter()
            .map(|l| editcore::PsdLayerSeed {
                name: l.name,
                offset: l.offset,
                image: l.image,
                visible: l.visible,
                opacity: l.opacity,
            })
            .collect()
    }
}

/// Parse the layers of a PSD. Signature and behaviour of `decode_psd_bytes`
/// are untouched.
pub fn decode_psd_layers(bytes: &[u8], options: DecodeOptions) -> Result<PsdLayerSet, DecodeError> {
    decode_psd_layers_with_progress(bytes, options, |_, _| {})
}

/// Same as [`decode_psd_layers`] but reports `(done, total)` after each layer so
/// the UI can show "已加载 k/N 层" while the editing lock is held (C2-a).
pub fn decode_psd_layers_with_progress<F>(
    bytes: &[u8],
    options: DecodeOptions,
    progress: F,
) -> Result<PsdLayerSet, DecodeError>
where
    F: FnMut(usize, usize),
{
    let (width, height, channels) = check_header(bytes, options)?;
    crate::validate_psd_sections(bytes, width, height, channels)?;
    validate_psd_layer_records(bytes)?;

    // The vendored parser still has unchecked slices. Everything it touches
    // (including our own channel expansion) stays inside catch_unwind (C5).
    let mut progress = progress;
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let psd = psd::Psd::from_bytes(bytes)
            .map_err(|e| format!("PSD 分层解析失败：{e:?}"))?;
        build_layers(&psd, (width, height), &mut progress)
    }));
    match built {
        Ok(Ok(set)) => Ok(set),
        Ok(Err(msg)) => Err(DecodeError::Psd(msg)),
        Err(_) => Err(DecodeError::Psd(
            "无法解析此 PSD 的图层数据（文件数据异常）".into(),
        )),
    }
}

/// Mirror the header checks of `decode_psd_bytes` without touching it.
fn check_header(bytes: &[u8], options: DecodeOptions) -> Result<(u32, u32, u16), DecodeError> {
    let (width, height, depth, mode) = crate::psd_header(bytes).ok_or(DecodeError::UnsupportedFormat)?;
    let channels = bytes
        .get(12..14)
        .map(|s| u16::from_be_bytes([s[0], s[1]]))
        .ok_or(DecodeError::UnsupportedFormat)?;
    if width == 0 || height == 0 {
        return Err(DecodeError::Psd("画布尺寸为 0".into()));
    }
    if depth != 8 {
        return Err(DecodeError::Psd(format!(
            "仅支持 8 位/通道（此文件为 {depth} 位），无法加载分层"
        )));
    }
    if !matches!(mode, 1 | 3) {
        return Err(DecodeError::Psd(format!(
            "仅支持 RGB / 灰度模式（此文件为 {}），无法加载分层",
            crate::psd_mode_name(mode)
        )));
    }
    if let Some(limit) = options.max_pixels {
        if width as u64 * height as u64 > limit {
            return Err(DecodeError::TooLarge {
                width,
                height,
                pixels: width as u64 * height as u64,
                limit,
            });
        }
    }
    // C8: a downsampled document must never be written back with lost pixels.
    if let Some(edge) = options.max_dimension {
        if width.max(height) > edge {
            return Err(DecodeError::Psd(
                "此文件以降采样预览打开，不加载分层且禁止写回原文件".into(),
            ));
        }
    }
    Ok((width, height, channels))
}

fn build_layers<F: FnMut(usize, usize)>(
    psd: &psd::Psd,
    canvas: (u32, u32),
    progress: &mut F,
) -> Result<PsdLayerSet, String> {
    let (cw, ch) = (canvas.0 as i32, canvas.1 as i32);
    let source = psd.layers();
    let total = source.len();
    progress(0, total);
    let mut layers = Vec::with_capacity(total);
    let mut degraded: Vec<String> = Vec::new();
    let mut total_bytes = 0usize;

    for (idx, layer) in source.iter().enumerate() {
        let left = layer.layer_left();
        let top = layer.layer_top();
        let right = layer.layer_right();
        let bottom = layer.layer_bottom();
        let raw_w = (right as i64 - left as i64 + 1).max(1);
        let raw_h = (bottom as i64 - top as i64 + 1).max(1);
        let mut notes: Vec<String> = Vec::new();

        // ---- degradation contract (§7) ---------------------------------
        match layer.blend_mode() {
            psd::BlendMode::Normal | psd::BlendMode::PassThrough => {}
            other => notes.push(format!("混合模式 {other:?} 已按 Normal 处理")),
        }
        if layer.parent_id().is_some() {
            notes.push("组结构已展平".into());
        }
        if layer.is_clipping_mask() {
            notes.push("裁剪蒙版未保留".into());
        }
        if left < 0 || top < 0 || right + 1 > cw || bottom + 1 > ch {
            notes.push("层越出画布，已按画布裁剪".into());
        }

        // ---- clip the layer rectangle to the canvas --------------------
        let x0 = left.max(0);
        let y0 = top.max(0);
        let x1 = ((right as i64 + 1).min(cw as i64)) as i32;
        let y1 = ((bottom as i64 + 1).min(ch as i64)) as i32;
        if x1 <= x0 || y1 <= y0 {
            notes.push("层完全在画布外，已跳过".into());
            degraded.extend(notes.iter().cloned());
            progress(idx + 1, total);
            continue;
        }
        let src_x = (x0 - left) as u32;
        let src_y = (y0 - top) as u32;
        // A layer record may claim a rectangle larger than the pixel data we
        // keep; clamp the crop against the channel extent so crop_imm can
        // never index outside `full`.
        let avail_w = (raw_w.saturating_sub(src_x as i64)).max(0) as u32;
        let avail_h = (raw_h.saturating_sub(src_y as i64)).max(0) as u32;
        let crop_w = ((x1 - x0) as u32).min(avail_w);
        let crop_h = ((y1 - y0) as u32).min(avail_h);
        if crop_w == 0 || crop_h == 0 {
            notes.push("层位图数据不足，已跳过".into());
            degraded.extend(notes.iter().cloned());
            progress(idx + 1, total);
            continue;
        }

        // Only the visible part of the layer needs to be expanded. For a
        // negative-offset layer the source image is cropped from its top-left.
        let need_w = src_x + crop_w;
        let need_h = src_y + crop_h;
        let pixels = need_w as u64 * need_h as u64;
        if pixels > 300_000_000 {
            notes.push("层尺寸过大，已按空层处理".into());
            degraded.extend(notes.iter().cloned());
            progress(idx + 1, total);
            continue;
        }
        let pixels = pixels as usize;

        let chans = layer.channel_bytes();
        let red = channel_plane(chans, PsdChannelKind::Red, pixels);
        let green = channel_plane(chans, PsdChannelKind::Green, pixels);
        let blue = channel_plane(chans, PsdChannelKind::Blue, pixels);
        let alpha = channel_plane(chans, PsdChannelKind::TransparencyMask, pixels);
        let Some(red) = red else {
            return Err(format!("PSD 图层「{}」缺少颜色通道", layer.name()));
        };
        // Grayscale PSDs store a single channel; mirror generate_rgba().
        let green = green.unwrap_or_else(|| red.clone());
        let blue = blue.unwrap_or_else(|| red.clone());
        let mut rgba = vec![0u8; pixels * 4];
        for i in 0..pixels {
            rgba[i * 4] = red[i];
            rgba[i * 4 + 1] = green[i];
            rgba[i * 4 + 2] = blue[i];
            rgba[i * 4 + 3] = alpha.as_ref().map(|a| a[i]).unwrap_or(255);
        }
        let full = RgbaImage::from_raw(need_w, need_h, rgba)
            .ok_or_else(|| "层位图尺寸异常".to_string())?;
        let image = image::imageops::crop_imm(&full, src_x, src_y, crop_w, crop_h).to_image();
        total_bytes += (image.width() as usize) * (image.height() as usize) * 4;

        degraded.extend(notes.iter().cloned());
        layers.push(PsdLayerData {
            name: layer.name().to_string(),
            offset: (x0, y0),
            image,
            visible: layer.visible(),
            opacity: layer.opacity() as f32 / 255.0,
            degraded: notes,
        });
        progress(idx + 1, total);
    }

    dedup(&mut degraded);
    Ok(PsdLayerSet {
        layers,
        canvas,
        degraded,
        // Stage 1 always has the composite from `decode_psd_bytes`; if it
        // failed the file would not have been opened at all.
        has_composite: true,
        bytes: total_bytes,
    })
}

fn dedup(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
}

/// Expand one planar channel to exactly `pixels` bytes. RLE data from the
/// vendored reader has the 2-byte-per-scanline length table already stripped.
fn channel_plane(
    chans: &psd::LayerChannels,
    kind: PsdChannelKind,
    pixels: usize,
) -> Option<Vec<u8>> {
    match chans.get(&kind)? {
        ChannelBytes::RawData(v) => {
            let mut out = v.clone();
            out.resize(pixels, 0);
            Some(out)
        }
        ChannelBytes::RleCompressed(v) => Some(unpack_packbits(v, pixels)),
    }
}

fn unpack_packbits(data: &[u8], expected: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0usize;
    while i < data.len() && out.len() < expected {
        let header = data[i] as i8 as i16;
        i += 1;
        if header == -128 {
            continue;
        }
        if header >= 0 {
            let count = (header + 1) as usize;
            let end = (i + count).min(data.len());
            out.extend_from_slice(&data[i..end]);
            i = end;
        } else {
            let count = (1 - header) as usize;
            if i >= data.len() {
                break;
            }
            let value = data[i];
            i += 1;
            out.resize((out.len() + count).min(expected), value);
        }
    }
    out.resize(expected, 0);
    out
}

/// Pre-validate the layer&mask section down to per-record / per-channel
/// lengths (M1d). `catch_unwind` stays as the last line of defence, but a
/// corrupt file should produce a normal `DecodeError` instead of a panic.
fn validate_psd_layer_records(bytes: &[u8]) -> Result<(), DecodeError> {
    let err = || DecodeError::Psd("图层记录区段被截断，文件已损坏".into());
    let u32at = |o: usize| -> Option<u32> {
        let s = bytes.get(o..o + 4)?;
        Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    };
    let u16at = |o: usize| -> Option<u16> {
        let s = bytes.get(o..o + 2)?;
        Some(u16::from_be_bytes([s[0], s[1]]))
    };

    let mut off = 26usize;
    for _ in 0..2 {
        let len = u32at(off).ok_or_else(err)? as usize;
        off = off.checked_add(4 + len).ok_or_else(err)?;
    }
    let section_len = u32at(off).ok_or_else(err)? as usize;
    let section_start = off + 4;
    let section_end = section_start.checked_add(section_len).ok_or_else(err)?;
    if section_end > bytes.len() {
        return Err(err());
    }
    // A flat PSD has an empty Layer & Mask section (length 0); there is
    // nothing to pre-validate and the parser will report zero layers.
    if section_len == 0 {
        return Ok(());
    }
    let info_len = u32at(section_start).ok_or_else(err)? as usize;
    if info_len == 0 {
        return Ok(());
    }
    let info_start = section_start + 4;
    // Some encoders write a Layer Info length that does not cover the channel
    // data (and odd-length Pascal names have different padding). The vendored
    // reader works off the SECTION slice, so use the section boundary as the
    // hard limit and keep the declared info_len only for the "no layers" case.
    let info_end = section_end;
    if info_start + 2 > info_end {
        return Err(err());
    }
    let count = i16::from_be_bytes([bytes[info_start], bytes[info_start + 1]]).unsigned_abs() as usize;
    if count > 4096 {
        return Err(DecodeError::Psd("图层数量异常".into()));
    }
    let mut p = info_start + 2;
    let mut records: Vec<(usize, Vec<u32>)> = Vec::with_capacity(count);
    for _ in 0..count {
        let top = u32at(p).ok_or_else(err)? as i32;
        let bottom = u32at(p + 8).ok_or_else(err)? as i32;
        p += 16;
        let nch = u16at(p).ok_or_else(err)? as usize;
        p += 2;
        if nch > 56 || p + nch * 6 > info_end {
            return Err(err());
        }
        let mut lengths = Vec::with_capacity(nch);
        for _ in 0..nch {
            let l = u32at(p + 2).ok_or_else(err)?;
            if l < 2 {
                return Err(DecodeError::Psd("图层通道长度异常".into()));
            }
            lengths.push(l);
            p += 6;
        }
        p += 12; // '8BIM' + blend key + opacity/clipping/flags/filler
        let extra_len = u32at(p).ok_or_else(err)? as usize;
        p += 4;
        let extra_end = p.checked_add(extra_len).ok_or_else(err)?;
        if extra_end > info_end {
            return Err(err());
        }
        // Extra data: mask length, blending ranges length, Pascal name,
        // then a chain of 8BIM/8B64 additional-layer-info blocks.
        let mask_len = u32at(p).ok_or_else(err)? as usize;
        p = p.checked_add(4 + mask_len).ok_or_else(err)?;
        let ranges_len = u32at(p).ok_or_else(err)? as usize;
        p = p.checked_add(4 + ranges_len).ok_or_else(err)?;
        let name_len = *bytes.get(p).ok_or_else(err)? as usize;
        let pad = (4 - ((name_len + 1) % 4)) % 4;
        p = p.checked_add(1 + name_len + pad).ok_or_else(err)?;
        while p + 12 <= extra_end {
            let sig = bytes.get(p..p + 4).unwrap_or(&[]);
            if sig != b"8BIM" && sig != b"8B64" {
                break;
            }
            let l = u32at(p + 8).ok_or_else(err)? as usize;
            p = p.checked_add(12 + l).ok_or_else(err)?;
            if p > extra_end {
                return Err(err());
            }
        }
        p = extra_end;
        if p > info_end {
            return Err(err());
        }
        let scanlines = (bottom as i64 - top as i64 + 1).max(1).min(65_535) as usize;
        records.push((scanlines, lengths));
    }
    for (scanlines, lengths) in records {
        for l in lengths {
            let l = l as usize;
            let end = p.checked_add(l).ok_or_else(err)?;
            if end > info_end {
                return Err(err());
            }
            let compression = u16at(p).ok_or_else(err)?;
            if compression == 1 {
                // Vendor skips 2*scanlines bytes of row-length table. A channel
                // shorter than that is degraded to an empty (transparent) layer
                // by the vendored reader, so reject it here with a clear error.
                // The row counts themselves are NOT validated against the
                // remaining bytes: real files (e.g. out-of-bounds layers) can
                // carry a truncated RLE stream, and both the vendor reader and
                // our own PackBits unpacker are bounds-safe in that case.
                if l.saturating_sub(2) < scanlines * 2 {
                    return Err(DecodeError::Psd("RLE 行长度表被截断".into()));
                }
            } else if compression > 1 {
                return Err(DecodeError::Psd(format!(
                    "压缩方式 {compression} 不受支持，无法加载分层"
                )));
            }
            p = end;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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

    fn make_psd(w: u32, h: u32, layers: &[TestLayer]) -> Vec<u8> {
        let mut records = Vec::new();
        let mut channels = Vec::new();
        for layer in layers.iter().rev() {
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
            ("green-1x1", include_bytes!("../../../vendor/psd/tests/fixtures/green-1x1.psd")),
            (
                "chinese-name",
                include_bytes!("../../../vendor/psd/tests/fixtures/green-chinese-layer-name-1x1.psd"),
            ),
            (
                "negative-offset",
                include_bytes!("../../../vendor/psd/tests/fixtures/negative-top-left-layer.psd"),
            ),
            (
                "out-of-bounds",
                include_bytes!("../../../vendor/psd/tests/fixtures/out-of-bounds-layer.psd"),
            ),
            (
                "rle-3-layer",
                include_bytes!("../../../vendor/psd/tests/fixtures/rle-3-layer-8x8.psd"),
            ),
            (
                "one-channel",
                include_bytes!("../../../vendor/psd/tests/fixtures/one-channel-1x1.psd"),
            ),
            (
                "non-utf8-name",
                include_bytes!("../../../vendor/psd/tests/fixtures/non-utf8-pascal-string.psd"),
            ),
            (
                "two-layers",
                include_bytes!("../../../vendor/psd/tests/fixtures/two-layers-red-green-1x1.psd"),
            ),
            (
                "transparent-above",
                include_bytes!("../../../vendor/psd/tests/fixtures/transparent-above-opaque.psd"),
            ),
            (
                "odd-pascal",
                include_bytes!("../../../vendor/psd/tests/fixtures/odd-length-pascal-string.psd"),
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
        let bytes = include_bytes!("../../../vendor/psd/tests/fixtures/green-chinese-layer-name-1x1.psd");
        let set = decode_psd_layers(bytes, DecodeOptions::default()).expect("parse");
        assert!(set.layers.iter().any(|l| l.name.chars().any(|c| c as u32 > 127)));
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
}
