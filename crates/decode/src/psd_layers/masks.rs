use image::{GrayImage, Luma};
use psd::{ChannelBytes, PsdChannelKind};

/// Expand one planar channel to exactly `pixels` bytes. RLE data from the
/// vendored reader has the 2-byte-per-scanline length table already stripped.
pub(super) fn channel_plane(
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

/// 解出一层的图层蒙版（`-2` 通道），按**蒙版自己的矩形**裁剪到画布范围。
///
/// 蒙版矩形通常就是整幅画布，而图层往往只是画布上的一个小贴片 —— 用图层
/// 矩形去解释蒙版通道会把蒙版错位。取不到通道 / 蒙版被 PS 关掉时返回 `None`
/// （等价于「无蒙版」，不会误把整层遮住）。
/// 诊断用：`IMAGEEDIT_PSD_MASK_AT=x,y` 指定要打印蒙版取值的画布坐标。
pub(super) fn mask_probe() -> Option<(i32, i32)> {
    mask_probe_at("IMAGEEDIT_PSD_MASK_AT")
}

pub(super) fn mask_probe_at(key: &str) -> Option<(i32, i32)> {
    let raw = std::env::var(key).ok()?;
    let (x, y) = raw.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

pub(super) fn mask_at(m: &editcore::LayerMask, probe: Option<(i32, i32)>) -> i32 {
    match probe {
        Some((x, y)) => (m.coverage(x, y) * 255.0).round() as i32,
        None => -1,
    }
}

pub(super) fn decode_mask(
    layer: &psd::PsdLayer,
    chans: &psd::LayerChannels,
    cw: i32,
    ch: i32,
) -> Option<editcore::LayerMask> {
    if !layer.has_active_mask() {
        return None;
    }
    let m = layer.mask_rect()?;
    let (mw, mh) = (m.width(), m.height());
    if mw == 0 || mh == 0 {
        return None;
    }
    let pixels = mw as u64 * mh as u64;
    if pixels > 300_000_000 {
        return None;
    }
    let mut plane = channel_plane(
        chans,
        PsdChannelKind::UserSuppliedLayerMask,
        pixels as usize,
    )?;
    // 通道数据比矩形短（截断文件）时按 0 补齐：蒙版缺数据 = 遮住，绝不能
    // 让下面的平面索引越界。
    plane.resize(pixels as usize, 0);
    let x0 = m.left.max(0);
    let y0 = m.top.max(0);
    let x1 = ((m.right as i64 + 1).min(cw as i64)) as i32;
    let y1 = ((m.bottom as i64 + 1).min(ch as i64)) as i32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut img = GrayImage::new((x1 - x0) as u32, (y1 - y0) as u32);
    for y in y0..y1 {
        let sy = (y - m.top) as usize;
        for x in x0..x1 {
            let sx = (x - m.left) as usize;
            let v = plane[sy * mw as usize + sx];
            img.put_pixel((x - x0) as u32, (y - y0) as u32, Luma([v]));
        }
    }
    Some(editcore::LayerMask::with_default(
        img,
        (x0, y0),
        m.default_color,
    ))
}

pub(crate) fn unpack_packbits(data: &[u8], expected: usize) -> Vec<u8> {
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
