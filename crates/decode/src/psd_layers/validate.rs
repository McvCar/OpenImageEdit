use crate::{DecodeError, DecodeOptions};

/// Mirror the header checks of `decode_psd_bytes` without touching it.
pub(super) fn check_header(bytes: &[u8], options: DecodeOptions) -> Result<(u32, u32, u16), DecodeError> {
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

/// Pre-validate the layer&mask section down to per-record / per-channel
/// lengths (M1d). `catch_unwind` stays as the last line of defence, but a
/// corrupt file should produce a normal `DecodeError` instead of a panic.
///
/// Returns non-fatal degradation notes; `Ok(vec![])` means "clean".
pub(super) fn validate_psd_layer_records(bytes: &[u8]) -> Result<Vec<String>, DecodeError> {
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
        return Ok(Vec::new());
    }
    let info_len = u32at(section_start).ok_or_else(err)? as usize;
    if info_len == 0 {
        return Ok(Vec::new());
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
    // (layer height, user-mask height, real-mask height, (channel id, length))
    let mut records: Vec<(usize, Option<usize>, Option<usize>, Vec<(i16, u32)>)> =
        Vec::with_capacity(count);
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
            let cid = u16at(p).ok_or_else(err)? as i16;
            let l = u32at(p + 2).ok_or_else(err)?;
            if l < 2 {
                return Err(DecodeError::Psd("图层通道长度异常".into()));
            }
            lengths.push((cid, l));
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
        //
        // Layer mask data carries its OWN rectangle, and its channel (`-2`) is
        // compressed against that rectangle — not against the layer rectangle.
        // Reading the mask rect here is what makes mask channels validate
        // correctly. Layout: top/left/bottom/right (4x i32), default colour,
        // flags, 2 pad bytes; a real user mask (channel `-3`) follows at +18.
        let mask_len = u32at(p).ok_or_else(err)? as usize;
        let mut mask_rows = None;
        let mut real_mask_rows = None;
        if mask_len >= 20 {
            let m_top = u32at(p + 4).ok_or_else(err)? as i32;
            let m_bottom = u32at(p + 12).ok_or_else(err)? as i32;
            mask_rows = Some((m_bottom as i64 - m_top as i64 + 1).clamp(0, 65_535) as usize);
        }
        if mask_len >= 38 {
            let r_top = u32at(p + 22).ok_or_else(err)? as i32;
            let r_bottom = u32at(p + 30).ok_or_else(err)? as i32;
            real_mask_rows = Some((r_bottom as i64 - r_top as i64 + 1).clamp(0, 65_535) as usize);
        }
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
        let scanlines = (bottom as i64 - top as i64 + 1).max(0).min(65_535) as usize;
        records.push((scanlines, mask_rows, real_mask_rows, lengths));
    }
    let mut short_channels = 0usize;
    for (scanlines, mask_rows, real_mask_rows, lengths) in records {
        for (cid, l) in lengths {
            let l = l as usize;
            let end = p.checked_add(l).ok_or_else(err)?;
            if end > info_end {
                return Err(err());
            }
            let compression = u16at(p).ok_or_else(err)?;
            if compression == 1 {
                // Vendor skips 2 rows-of-table bytes. Mask channels (`-2` /
                // `-3`) hold their OWN rectangle in the mask block, so their
                // scanline count is not the layer height; using the layer
                // height there flagged perfectly good Photoshop files.
                let rows = match cid {
                    -2 => mask_rows.unwrap_or(scanlines),
                    -3 => real_mask_rows.or(mask_rows).unwrap_or(scanlines),
                    _ => scanlines,
                };
                // A short row table is NOT fatal: the vendored reader drops
                // that single channel (the layer shows as transparent) and both
                // it and our PackBits unpacker are bounds-safe. Real files do
                // hit this (empty layers, unusual encoders), and refusing the
                // whole document loses every other layer as well — degrade and
                // report instead.
                // The row counts themselves are also not validated against the
                // remaining bytes for the same reason.
                if l.saturating_sub(2) < rows * 2 && cid >= -1 {
                    short_channels += 1;
                }
            } else if compression > 1 {
                return Err(DecodeError::Psd(format!(
                    "压缩方式 {compression} 不受支持，无法加载分层"
                )));
            }
            p = end;
        }
    }
    let mut notes = Vec::new();
    if short_channels > 0 {
        notes.push(format!(
            "{short_channels} 个图层通道数据不完整，已按空层处理"
        ));
    }
    Ok(notes)
}
