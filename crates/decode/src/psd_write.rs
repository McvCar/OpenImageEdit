//! PSD (8-bit RGB) encoder and save-format dispatch.
//!
//! Implements the minimal write subset from `docs/PSD分层可编辑-B方案.md`
//! appendix A. All integers are big-endian. Layer channels and the merged
//! composite are written **raw** (`compression = 0`, planar R/G/B/A): the
//! vendored read-only parser strips a 2-byte compression marker and, for RLE,
//! a `2 * scanlines` row table, so raw planar is the safest common ground.
//!
//! A flattened composite is ALWAYS written into the Image Data section so a
//! reader that ignores layers still shows the correct picture.

use image::RgbaImage;
// The core package declares `[lib] name = "editcore"`, so `editcore` is the
// extern crate name (not `imageedit_core`).
use editcore::Document;

// ---- big-endian push helpers -------------------------------------------

fn push_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}
fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn push_i16(out: &mut Vec<u8>, v: i16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn push_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn push_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(b);
}
fn push_zeros(out: &mut Vec<u8>, n: usize) {
    out.resize(out.len() + n, 0);
}

/// Append the four planes (all R, then all G, then all B, then all A) of an
/// interleaved RGBA image, in row-major pixel order.
fn push_planar(out: &mut Vec<u8>, image: &RgbaImage) {
    let raw = image.as_raw();
    for ch in 0..4usize {
        let mut i = ch;
        while i < raw.len() {
            out.push(raw[i]);
            i += 4;
        }
    }
}

/// Pascal layer name is a compatibility placeholder: every non-ASCII
/// character becomes `?`. The real (Unicode) name lives in `8BIMluni`.
fn ascii_placeholder(name: &str) -> Vec<u8> {
    let mut v = Vec::new();
    for c in name.chars() {
        if v.len() == 255 {
            break; // Pascal length is a u8
        }
        v.push(if c.is_ascii() { c as u8 } else { b'?' });
    }
    v
}

/// A document layer clipped to the canvas, with its channels already split
/// into raw planar bytes. `bottom`/`right` are EXCLUSIVE canvas coordinates
/// (the vendored parser subtracts one when reading them).
struct PreparedLayer {
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
    name: String,
    opacity: u8,
    visible: bool,
    /// 剪贴蒙版（PSD 的 clipping 字节：0 = 基底，1 = 剪贴到下层）。
    clipping: bool,
    blend: editcore::BlendMode,
    planes: [Vec<u8>; 4],
    /// 图层蒙版（`-2` 通道 + 它自己的矩形）。
    mask: Option<PreparedMask>,
    /// 混合颜色带（Blend If）：全默认时 `None`，写回长度 0。
    blend_if: Option<editcore::BlendIf>,
    /// 挖空（`knko`）：`None` = 关闭，不写这个键。
    knockout: editcore::Knockout,
    /// 所属分组（写 PSD 时要按它插 `lsct` 分隔记录）。
    group: Option<editcore::GroupId>,
}

/// 图层蒙版的落盘数据。
///
/// PSD 的蒙版通道按**蒙版自己的矩形**压缩，和图层矩形无关；蒙版矩形写的是
/// 右下**边界**（right = left + 宽），vendored 解析器读的时候会减 1 换算成
/// 「最后一个像素」，与图层矩形同一约定。
struct PreparedMask {
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
    /// 蒙版矩形外的取值（PS 只存 0 / 255）。
    default: u8,
    /// 灰度平面，行优先，长度 = 宽 × 高。
    plane: Vec<u8>,
}

/// 一条 PSD 层记录：真图层，或者分组的分隔记录（文件夹行 / 组底边界）。
enum PsdRecord {
    Layer(PreparedLayer),
    /// `section`：1 = 展开的文件夹，2 = 折叠的文件夹，3 = 组底边界。
    ///
    /// 文件夹行（1/2）同时带着组自己的混合模式与不透明度：PSD 的组属性就
    /// 存在这条记录上，不写回去的话「组正片叠底 + 50%」保存再打开会变成
    /// 普通组，而成员的不透明度已经不再替组承担这份衰减。
    Divider {
        name: String,
        section: u32,
        /// 已经是 4 字节 PSD 键：穿透组要写 `pass`，不能退化成 `norm`。
        blend: &'static [u8; 4],
        opacity: u8,
    },
}

/// Build the PSD layer records. A PSD file stores layer records bottom-to-top,
/// which is exactly `doc.layers` order, so they are written in order. (The
/// vendored parser reverses the records as it reads them, and
/// `psd_layers::build_layers` reverses that back — reversing here as well would
/// flip the stack on every save/reopen round-trip.)
fn prepare_layers(doc: &Document) -> Result<Vec<PreparedLayer>, crate::DecodeError> {
    let cw = doc.canvas.width as i64;
    let ch = doc.canvas.height as i64;
    let mut out = Vec::new();

    for (idx, layer) in doc.layers.iter().enumerate() {
        let (plane, (ox, oy)) = layer.plane();
        let pw = plane.width() as i64;
        let ph = plane.height() as i64;
        if pw == 0 || ph == 0 {
            continue;
        }
        // `layers[0]` is a SYNTHETIC transparent background created by
        // `Document::from_psd_layers` (the unified layer model requires a
        // permanent 0 slot). Writing it out as a real PSD layer would add one
        // empty "背景" layer on EVERY save / reopen cycle. It contributes
        // nothing to `doc.flatten()`, so skip it while it is fully transparent
        // — an opaque (user-filled) background is still written normally.
        if idx == 0 && plane.as_raw().chunks_exact(4).all(|p| p[3] == 0) {
            continue;
        }
        let (x0, y0) = (ox as i64, oy as i64);
        let (x1, y1) = (x0 + pw, y0 + ph);
        let nx0 = x0.clamp(0, cw);
        let ny0 = y0.clamp(0, ch);
        let nx1 = x1.clamp(0, cw);
        let ny1 = y1.clamp(0, ch);
        if nx1 <= nx0 || ny1 <= ny0 {
            // Fully outside the canvas: skip the layer entirely. layer_count
            // is derived from `out`, so it always matches the record count.
            continue;
        }
        let (sx, sy) = ((nx0 - x0) as u32, (ny0 - y0) as u32);
        let (w, h) = ((nx1 - nx0) as u32, (ny1 - ny0) as u32);
        let n = (w * h) as usize;
        let mut planes: [Vec<u8>; 4] = [
            Vec::with_capacity(n),
            Vec::with_capacity(n),
            Vec::with_capacity(n),
            Vec::with_capacity(n),
        ];
        let raw = plane.as_raw();
        let src_w = plane.width();
        for yy in 0..h {
            for xx in 0..w {
                let idx = (((sy + yy) * src_w + (sx + xx)) * 4) as usize;
                planes[0].push(raw[idx]);
                planes[1].push(raw[idx + 1]);
                planes[2].push(raw[idx + 2]);
                planes[3].push(raw[idx + 3]);
            }
        }
        out.push(PreparedLayer {
            top: ny0 as i32,
            left: nx0 as i32,
            bottom: ny1 as i32, // exclusive (parser subtracts one)
            right: nx1 as i32,  // exclusive (parser subtracts one)
            name: layer.name.clone(),
            opacity: (layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
            visible: layer.visible,
            clipping: layer.clipping,
            blend: layer.blend,
            planes,
            // 蒙版按它自己的矩形原样写：模型里的蒙版已经裁剪到画布范围，
            // 矩形外默认色（`default`）也要一起保留，否则重开会藏掉内容。
            mask: layer.mask.as_ref().and_then(|m| {
                let (mw, mh) = m.image.dimensions();
                if mw == 0 || mh == 0 {
                    return None;
                }
                Some(PreparedMask {
                    top: m.offset.1,
                    left: m.offset.0,
                    bottom: m.offset.1 + mh as i32, // boundary (parser subtracts one)
                    right: m.offset.0 + mw as i32,  // boundary (parser subtracts one)
                    default: m.default,
                    plane: m.image.as_raw().clone(),
                })
            }),
            blend_if: layer.blend_if.clone(),
            knockout: layer.knockout,
            group: layer.group,
        });
    }

    if out.len() > i16::MAX as usize {
        return Err(crate::DecodeError::Psd("图层数超过 PSD 上限 (32767)".into()));
    }
    Ok(out)
}

/// PSD's four-character key for a blend mode.
///
/// The mapping is the inverse of `psd_layers::map_blend_mode`; anything the
/// reader had to fall back to Normal already carries a degradation note, so
/// silently writing `norm` for it is consistent with what the user was told.
fn blend_key(mode: editcore::BlendMode) -> &'static [u8; 4] {
    use editcore::BlendMode as M;
    match mode {
        M::Normal => b"norm",
        M::Dissolve => b"diss",
        M::Multiply => b"mul ",
        M::Screen => b"scrn",
        M::Overlay => b"over",
        M::Darken => b"dark",
        M::Lighten => b"lite",
        M::ColorDodge => b"div ",
        M::ColorBurn => b"idiv",
        M::LinearDodge => b"lddg",
        M::LinearBurn => b"lbrn",
        M::DarkerColor => b"dkCl",
        M::LighterColor => b"lgCl",
        M::SoftLight => b"sLit",
        M::HardLight => b"hLit",
        M::VividLight => b"vLit",
        M::LinearLight => b"lLit",
        M::PinLight => b"pLit",
        M::HardMix => b"hMix",
        M::Difference => b"diff",
        M::Exclusion => b"smud",
        M::Subtract => b"fsub",
        M::Divide => b"fdiv",
        M::Hue => b"hue ",
        M::Saturation => b"sat ",
        M::Color => b"colr",
        M::Luminosity => b"lum ",
    }
}

/// Write one Layer Record (geometry + channel declarations + blend mode +
/// extra data). Channel DATA bytes follow ALL records, so `encode_psd` emits
/// them after this has run for every layer.
fn write_layer_record(out: &mut Vec<u8>, l: &PreparedLayer) -> Result<(), crate::DecodeError> {
    // Each raw channel = 2-byte compression marker + one byte per layer pixel.
    // The declared `channel_length` INCLUDES the 2-byte marker: the parser
    // computes `channel_length - 2` to obtain the planar byte count.
    let channel_len = 2u64 + l.planes[0].len() as u64;
    if channel_len > u32::MAX as u64 {
        return Err(crate::DecodeError::Psd("层通道数据过大".into()));
    }
    write_record(
        out,
        (l.top, l.left, l.bottom, l.right),
        channel_len as u32,
        blend_key(l.blend),
        l.opacity,
        l.clipping,
        // Adobe's layer flags bit 1 is INVERTED: SET means HIDDEN. Photoshop
        // writes bit 3 (`1 for Photoshop 5.0 and later`) on visible layers and
        // never sets bit 1 unless the layer is hidden. Writing the vendor
        // parser's old (wrong) `2 = visible` convention made every layer appear
        // hidden in Photoshop / Krita / GIMP.
        if l.visible { 8 } else { 8 | 2 },
        &l.name,
        l.mask.as_ref(),
        l.blend_if.as_ref(),
        l.knockout,
        None,
    );
    Ok(())
}

/// 分组分隔记录：空矩形 + 4 个「只有压缩标记」的通道 + `8BIMlsct`。
///
/// Photoshop 的落盘顺序是**自底向上**，一个组在文件里长这样：
/// `[lsct 3 组底边界] 成员… [lsct 1|2 文件夹行（带组名）]`。所以组底的
/// 边界要在最低成员之前写，文件夹行在最高成员之后写（见 `prepare_records`）。
fn write_divider_record(
    out: &mut Vec<u8>,
    name: &str,
    section: u32,
    blend: &'static [u8; 4],
    opacity: u8,
) {
    // 组混合模式落在 `lsct` 块里（PS 的约定）：穿透组记录字段是 `norm`，
    // 真正的 `pass` 写在 lsct 的混合键上；其他模式两处一致。读回来时解码器
    // 优先用 lsct 的键，否则「穿透」会退化成普通隔离组。
    let record_blend: &'static [u8; 4] = if blend == b"pass" { b"norm" } else { blend };
    write_record(
        out,
        (0, 0, 0, 0),
        2, // 空通道：只有 2 字节压缩标记
        record_blend,
        opacity,
        false,
        8,
        name,
        None,
        None,
        editcore::Knockout::None,
        Some((section, blend)),
    );
}

/// 单条记录的公共写手（图层与分隔记录只差 `lsct` / 蒙版 / 剪贴位）。
#[allow(clippy::too_many_arguments)]
fn write_record(
    out: &mut Vec<u8>,
    rect: (i32, i32, i32, i32),
    channel_len: u32,
    blend: &[u8; 4],
    opacity: u8,
    clipping: bool,
    flags: u8,
    name: &str,
    mask: Option<&PreparedMask>,
    blend_if: Option<&editcore::BlendIf>,
    knockout: editcore::Knockout,
    lsct: Option<(u32, &'static [u8; 4])>,
) {
    let (top, left, bottom, right) = rect;
    push_i32(out, top);
    push_i32(out, left);
    push_i32(out, bottom); // exclusive (parser subtracts one)
    push_i32(out, right);  // exclusive (parser subtracts one)
    // 有蒙版时多声明一条 `-2` 通道；它的长度按**蒙版矩形**（而不是图层矩形）算。
    let mut channels: Vec<(i16, u32)> = vec![
        (0, channel_len),
        (1, channel_len),
        (2, channel_len),
        (-1, channel_len),
    ];
    if let Some(m) = mask {
        channels.push((-2, 2 + m.plane.len() as u32));
    }
    push_u16(out, channels.len() as u16);
    for (id, len) in channels {
        push_i16(out, id);
        push_u32(out, len);
    }
    push_bytes(out, b"8BIM");
    push_bytes(out, blend);
    push_u8(out, opacity); // 0 = transparent, 255 = opaque
    push_u8(out, if clipping { 1 } else { 0 }); // 0 = base, 1 = clipping
    push_u8(out, flags);
    push_u8(out, 0); // filler

    // ---- extra data ----
    let mut extra = Vec::new();
    // 图层蒙版数据块：20 字节 = 矩形(16) + 默认色(1) + 标志(1) + 补位(2)。
    // 长度为 0 表示「没有蒙版」。
    push_u32(&mut extra, if mask.is_some() { 20 } else { 0 });
    if let Some(m) = mask {
        push_i32(&mut extra, m.top);
        push_i32(&mut extra, m.left);
        // 与图层矩形一样存右下**边界**；解析端会减 1 换成最后一个像素。
        push_i32(&mut extra, m.bottom);
        push_i32(&mut extra, m.right);
        push_u8(&mut extra, m.default);
        push_u8(&mut extra, 0); // flags: bit1=0 -> 蒙版启用
        push_zeros(&mut extra, 2);
    }
    // 混合颜色带：长度 + 每组 4 字节（8 位文档每个值 1 字节）。顺序是
    // 「合成源、合成目标，然后每个通道各一对」；alpha 那对留默认，它和
    // 逐像素 alpha 遮罩重复。全默认写长度 0，与 PS 的习惯一致。
    let mut ranges: Vec<u8> = Vec::new();
    if let Some(b) = blend_if {
        for r in [
            b.source,
            b.destination,
            b.source_channels[0],
            b.destination_channels[0],
            b.source_channels[1],
            b.destination_channels[1],
            b.source_channels[2],
            b.destination_channels[2],
            editcore::BlendRange::DEFAULT,
            editcore::BlendRange::DEFAULT,
        ] {
            ranges.extend_from_slice(&r.to_psd());
        }
    }
    push_u32(&mut extra, ranges.len() as u32);
    push_bytes(&mut extra, &ranges);
    let ascii = ascii_placeholder(name);
    push_u8(&mut extra, ascii.len() as u8);
    push_bytes(&mut extra, &ascii);
    // Pascal name (1 length byte + bytes) is padded to a multiple of 4.
    push_zeros(&mut extra, (4 - (1 + ascii.len()) % 4) % 4);

    // `8BIMluni`: Unicode layer name (UTF-16BE). The parser calls
    // `read_unicode_string_padding(1)` which reads a u32 code-unit count +
    // 2*len bytes, then `seek(pos + additional_layer_info_len)`. We pad the
    // whole block to a 4-byte boundary and declare that padded length.
    push_bytes(&mut extra, b"8BIM");
    push_bytes(&mut extra, b"luni");
    let units: Vec<u16> = name.encode_utf16().collect();
    let data_no_pad = 4usize + 2 * units.len();
    let pad = (4 - data_no_pad % 4) % 4;
    push_u32(&mut extra, (data_no_pad + pad) as u32);
    push_u32(&mut extra, units.len() as u32);
    for u in units {
        push_u16(&mut extra, u);
    }
    push_zeros(&mut extra, pad);

    // `8BIMlsct`：分组分隔设置。类型 + 8BIM + 组混合键 = 12 字节，无需补位。
    if let Some((section, blend)) = lsct {
        push_bytes(&mut extra, b"8BIM");
        push_bytes(&mut extra, b"lsct");
        push_u32(&mut extra, 12);
        push_u32(&mut extra, section);
        push_bytes(&mut extra, b"8BIM");
        push_bytes(&mut extra, blend);
    }

    // `8BIMknko`：挖空设置（4 字节：1 = 浅、2 = 深）。必须放在 Pascal 图层名
    // 之后的「附加信息块」区；关掉时不写这个键，与 PS 的习惯一致。
    let ko = match knockout {
        editcore::Knockout::None => 0u32,
        editcore::Knockout::Shallow => 1,
        editcore::Knockout::Deep => 2,
    };
    if ko != 0 {
        push_bytes(&mut extra, b"8BIM");
        push_bytes(&mut extra, b"knko");
        push_u32(&mut extra, 4);
        push_u32(&mut extra, ko);
    }

    push_u32(out, extra.len() as u32); // extra data length
    push_bytes(out, &extra);
}

/// 把图层与分组拼成一条自底向上的记录序列。
///
/// 组在 PSD 里是「两条分隔记录夹住成员」：底部一条 `lsct 3`，顶部一条带
/// 组名的 `lsct 1|2`。嵌套组天然按同样规则递归（外层的组底更靠下）。
fn prepare_records(doc: &editcore::Document, layers: Vec<PreparedLayer>) -> Vec<PsdRecord> {
    // 组的祖先链（外层 → 内层）。
    fn chain(doc: &editcore::Document, leaf: editcore::GroupId) -> Vec<editcore::GroupId> {
        let mut out = Vec::new();
        let mut next = Some(leaf);
        let mut guard = 0;
        while let Some(gid) = next {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(group) = doc.find_group(gid) else {
                break;
            };
            out.push(group.id);
            next = group.parent;
        }
        out.reverse();
        out
    }

    let mut records: Vec<PsdRecord> = Vec::with_capacity(layers.len());
    // 当前打开的组（外层 → 内层）
    let mut open: Vec<editcore::GroupId> = Vec::new();
    let mut written: std::collections::HashSet<editcore::GroupId> = Default::default();

    for layer in layers {
        let want = match layer.group {
            Some(gid) => chain(doc, gid),
            None => Vec::new(),
        };
        // 共同前缀保持不变，剩下的先关（内 → 外，文件夹行），再开（外 → 内，
        // 组底边界）。顺序反了 Photoshop 会读成错乱的嵌套。
        let common = open
            .iter()
            .zip(want.iter())
            .take_while(|(a, b)| a == b)
            .count();
        for gid in open[common..].iter().rev() {
            if let Some(group) = doc.find_group(*gid) {
                records.push(PsdRecord::Divider {
                    name: group.name.clone(),
                    section: if group.collapsed { 2 } else { 1 },
                    blend: group_blend_key(group),
                    opacity: group_opacity(group),
                });
            }
        }
        for gid in &want[common..] {
            written.insert(*gid);
            records.push(PsdRecord::Divider {
                name: "</Layer group>".to_string(),
                section: 3,
                blend: b"norm",
                opacity: 255,
            });
        }
        open = want;
        records.push(PsdRecord::Layer(layer));
    }
    // 收尾：还开着的组从内往外关。
    for gid in open.iter().rev() {
        if let Some(group) = doc.find_group(*gid) {
            records.push(PsdRecord::Divider {
                name: group.name.clone(),
                section: if group.collapsed { 2 } else { 1 },
                blend: group_blend_key(group),
                opacity: group_opacity(group),
            });
        }
    }
    // 一个成员都没写出的组（空组 / 成员全在画布外）：补一对空壳记录，
    // 否则重开文件时这个组会凭空消失。
    for group in &doc.groups {
        if written.contains(&group.id) {
            continue;
        }
        records.push(PsdRecord::Divider {
            name: "</Layer group>".to_string(),
            section: 3,
            blend: b"norm",
            opacity: 255,
        });
        records.push(PsdRecord::Divider {
            name: group.name.clone(),
            section: if group.collapsed { 2 } else { 1 },
            blend: group_blend_key(group),
            opacity: group_opacity(group),
        });
    }
    records
}

/// 组不透明度落到 PSD 文件夹行的 0..=255 字节。
fn group_opacity(group: &editcore::LayerGroup) -> u8 {
    (group.opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// 组的 4 字节 PSD 混合键。穿透不是一种 `BlendMode`，必须单独写 `pass`；
/// 写 `norm` 会让「穿透组」在重开时变成「正常模式的隔离组」，画面会变。
fn group_blend_key(group: &editcore::LayerGroup) -> &'static [u8; 4] {
    if group.pass_through {
        b"pass"
    } else {
        blend_key(group.blend)
    }
}

/// 图像资源段：目前只写回文档内嵌的 ICC（资源 1039）。
///
/// `display` 路径的图层像素是**文档色彩空间**，profile 必须原样带回去；
/// 少了它 PS 会按 sRGB 解读，整幅偏色（`decode` 路径的图层已经是 sRGB，
/// `Document::icc` 为空，这里就与旧行为一致写长度 0）。
fn push_image_resources(out: &mut Vec<u8>, doc: &Document) {
    let Some(guard) = doc.icc.as_ref().and_then(|icc| icc.lock().ok()) else {
        push_u32(out, 0);
        return;
    };
    let data = guard.raw();
    let mut block = Vec::with_capacity(data.len() + 12);
    push_bytes(&mut block, b"8BIM");
    push_i16(&mut block, 1039);
    // Pascal 名字：空名字也要占两个字节（长度字节 + 1 字节补齐）。
    push_zeros(&mut block, 2);
    push_u32(&mut block, data.len() as u32);
    push_bytes(&mut block, data);
    if data.len() % 2 == 1 {
        block.push(0);
    }
    push_u32(out, block.len() as u32);
    push_bytes(out, &block);
}

/// Encode a document as an 8-bit RGB PSD, one PSD layer per document layer
/// (including layer 0). Always writes a flattened composite in the Image Data
/// section so readers without layer support still show the right picture.
pub fn encode_psd(doc: &Document, _quality: u8) -> Result<Vec<u8>, crate::DecodeError> {
    let cw = doc.canvas.width;
    let ch = doc.canvas.height;
    if cw == 0 || ch == 0 {
        return Err(crate::DecodeError::Psd("画布尺寸为 0".into()));
    }
    let composite = doc.flatten_in_doc_space();
    if composite.width() != cw || composite.height() != ch {
        return Err(crate::DecodeError::Psd("合成图尺寸与画布不一致".into()));
    }
    let layers = prepare_layers(doc)?;
    let records = prepare_records(doc, layers);

    let mut out = Vec::new();
    // ---- File Header (26 bytes) ----
    push_bytes(&mut out, b"8BPS");
    push_u16(&mut out, 1); // version 1 (PSD, not PSB)
    push_zeros(&mut out, 6); // reserved
    push_u16(&mut out, 4); // channels: R, G, B, A
    push_u32(&mut out, ch);
    push_u32(&mut out, cw);
    push_u16(&mut out, 8); // depth: 8 bits
    push_u16(&mut out, 3); // color mode: RGB

    // ---- Color Mode Data / Image Resources ----
    push_u32(&mut out, 0);
    push_image_resources(&mut out, doc);

    // ---- Layer & Mask Info ----
    let mut layer_info = Vec::new();
    // Positive layer_count: the parser takes abs(); a negative count only
    // signals a "first alpha = merged result" convention we don't rely on,
    // because our composite alpha lives in the Image Data section.
    if records.len() > i16::MAX as usize {
        return Err(crate::DecodeError::Psd("图层数超过 PSD 上限 (32767)".into()));
    }
    push_i16(&mut layer_info, records.len() as i16);
    for record in &records {
        match record {
            PsdRecord::Layer(l) => write_layer_record(&mut layer_info, l)?,
            PsdRecord::Divider {
                name,
                section,
                blend,
                opacity,
            } => write_divider_record(&mut layer_info, name, *section, *blend, *opacity),
        }
    }
    // All channel DATA follows all records, in the same order the channel
    // declarations above were written. Dividers declare four empty channels,
    // so each contributes exactly the 2-byte compression marker.
    for record in &records {
        match record {
            PsdRecord::Layer(l) => {
                for plane in &l.planes {
                    push_u16(&mut layer_info, 0); // compression = RAW
                    push_bytes(&mut layer_info, plane);
                }
                // 蒙版通道（`-2`）的数据跟在 RGBA 之后，按声明的顺序。
                if let Some(m) = &l.mask {
                    push_u16(&mut layer_info, 0);
                    push_bytes(&mut layer_info, &m.plane);
                }
            }
            PsdRecord::Divider { .. } => {
                for _ in 0..4 {
                    push_u16(&mut layer_info, 0);
                }
            }
        }
    }
    // Adobe spec: the Layer Info length is rounded up to a multiple of 2,
    // and the extra pad byte is physically present at the end of the block.
    if layer_info.len() % 2 == 1 {
        push_u8(&mut layer_info, 0);
    }
    let mut lm_body = Vec::new();
    push_u32(&mut lm_body, layer_info.len() as u32); // Layer Info length
    push_bytes(&mut lm_body, &layer_info);
    push_u32(&mut lm_body, 0); // Global Layer Mask / Additional Info length
    // section_len covers everything after its own 4-byte field.
    push_u32(&mut out, lm_body.len() as u32);
    push_bytes(&mut out, &lm_body);

    // ---- Image Data: merged composite, raw planar R/G/B/A ----
    push_u16(&mut out, 0); // compression = RAW
    push_planar(&mut out, &composite);

    Ok(out)
}

/// Dispatch: Psd -> encode_psd, Image(f) -> crate::encode(&doc.flatten(), f, quality).
pub fn encode_document(
    doc: &Document,
    format: crate::SaveFormat,
    quality: u8,
) -> Result<Vec<u8>, crate::DecodeError> {
    match format {
        crate::SaveFormat::Psd => encode_psd(doc, quality),
        crate::SaveFormat::Image(f) => crate::encode(&doc.flatten(), f, quality),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;
    use editcore::{Document, Layer};

    fn gradient(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([
                (x % 256) as u8,
                (y % 256) as u8,
                ((x + y) % 256) as u8,
                255,
            ])
        })
    }

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_fn(w, h, |_, _| image::Rgba(c))
    }

    fn assert_same(a: &RgbaImage, b: &RgbaImage) {
        assert_eq!(a.dimensions(), b.dimensions());
        assert_eq!(a.as_raw(), b.as_raw());
    }

    fn be_u32(b: &[u8], o: usize) -> usize {
        u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize
    }

    /// Offsets of the flags byte in every layer record of a PSD written by
    /// `encode_psd` (records are top-to-bottom in the file).
    fn layer_flag_offsets(b: &[u8]) -> Vec<usize> {
        let mut o = 26 + 4 + be_u32(b, 26) + 4 + be_u32(b, 30);
        o += 4; // Layer & Mask section length
        o += 4; // Layer Info length
        let count = i16::from_be_bytes([b[o], b[o + 1]]).unsigned_abs() as usize;
        o += 2;
        let mut out = Vec::new();
        for _ in 0..count {
            let channels = u16::from_be_bytes([b[o + 16], b[o + 17]]) as usize;
            // 16B rect + 2B channel count + 6B/channel + 8B blend + 2B
            // (opacity, clipping) => flags byte.
            let flags = o + 16 + 2 + 6 * channels + 8 + 2;
            out.push(flags);
            let extra = be_u32(b, flags + 2);
            o = flags + 2 + 4 + extra;
        }
        out
    }

    /// 1) Single-layer document: composite decoded by `decode_psd_bytes`
    /// equals `doc.flatten()` pixel-for-pixel.
    #[test]
    fn single_layer_composite_matches_flatten() {
        let doc = Document::from_image(gradient(97, 61));
        let flat = doc.flatten();
        let bytes = encode_psd(&doc, 90).expect("encode_psd");
        let decoded = crate::decode_psd_bytes(&bytes, crate::DecodeOptions::default())
            .expect("decode_psd_bytes");
        assert_same(&decoded.image, &flat);
    }

    /// 2) Multi-layer (semi-transparent, negative offset, out-of-bounds,
    /// Chinese names): both the flattened composite AND the per-layer
    /// metadata (name / offset / visible / opacity) round-trip.
    #[test]
    fn multi_layer_composite_and_layer_metadata() {
        let (cw, ch) = (16u32, 12u32);
        let mut doc = Document::from_image(solid(cw, ch, [10, 20, 30, 255]));

        let mut l1 = Layer::new("普通层", solid(6, 5, [200, 100, 50, 255]));
        l1.set_offset((2, 3));
        doc.layers.push(l1);

        let mut l2 = Layer::new("半透明层", solid(8, 8, [0, 255, 0, 128]));
        l2.set_offset((4, 2));
        l2.opacity = 0.5;
        doc.layers.push(l2);

        // Negative offset: clipped to (0, 0), size becomes 4x5.
        let mut l3 = Layer::new("中文图层", solid(6, 6, [255, 0, 0, 255]));
        l3.set_offset((-2, -1));
        l3.visible = false;
        doc.layers.push(l3);

        // Partially out of bounds on the right: offset stays (13, 2),
        // width clipped from 6 to 3.
        let mut l4 = Layer::new("右越界层", solid(6, 6, [0, 0, 255, 255]));
        l4.set_offset((13, 2));
        l4.opacity = 0.75;
        doc.layers.push(l4);

        // Fully outside: skipped by the encoder (layer_count must stay 5).
        let mut l5 = Layer::new("完全界外", solid(4, 4, [1, 2, 3, 255]));
        l5.set_offset((30, 30));
        doc.layers.push(l5);

        let flat = doc.flatten();
        let bytes = encode_psd(&doc, 90).expect("encode_psd");
        let decoded = crate::decode_psd_bytes(&bytes, crate::DecodeOptions::default())
            .expect("decode_psd_bytes");
        assert_same(&decoded.image, &flat);

        // Expected layers (bottom -> top), mirroring the clip/skip rule.
        let expected: Vec<(&str, (i32, i32), bool, f32)> = vec![
            ("背景", (0, 0), true, 1.0),
            ("普通层", (2, 3), true, 1.0),
            ("半透明层", (4, 2), true, 0.5),
            ("中文图层", (0, 0), false, 1.0),
            ("右越界层", (13, 2), true, 0.75),
        ];
        let set = crate::psd_layers::decode_psd_layers(
            &bytes,
            crate::DecodeOptions::default(),
        )
        .expect("decode_psd_layers");
        assert_eq!(set.layers.len(), expected.len(), "layer count");
        for (i, (name, off, visible, opacity)) in expected.iter().enumerate() {
            let got = &set.layers[i];
            assert_eq!(&got.name, name, "layer {i} name");
            assert_eq!(got.offset, *off, "layer {i} offset");
            assert_eq!(got.visible, *visible, "layer {i} visible");
            assert!(
                (got.opacity - opacity).abs() <= 1.0 / 255.0 + 1e-6,
                "layer {i} opacity {} vs {}",
                got.opacity,
                opacity
            );
        }
    }

    /// 2d) 组自己的混合模式 / 不透明度 / 穿透标记必须写回文件夹行并读回来。
    /// 成员不再替组承担不透明度，丢了这几个字段保存再打开画面就会变。
    #[test]
    fn group_blend_opacity_and_pass_through_round_trip() {
        use editcore::{BlendMode, PsdGroupSeed, PsdLayerSeed};
        let (cw, ch) = (8u32, 8u32);
        let member = PsdLayerSeed {
            name: "成员".into(),
            offset: (0, 0),
            image: solid(cw, ch, [10, 20, 30, 255]),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            group: Some(1),
            mask: None,
            clipping: false,
            clip_shape: None,
            style: None,
            blend_if: None,
            knockout: editcore::Knockout::None,
        };
        let doc = Document::from_psd_layers_with_groups(
            (cw, ch),
            vec![member],
            vec![PsdGroupSeed {
                psd_id: 1,
                parent: None,
                name: "组".into(),
                collapsed: true,
                visible: true,
                blend: BlendMode::Multiply,
                opacity: 0.5,
                pass_through: false,
            }],
        );
        let bytes = encode_psd(&doc, 90).expect("encode_psd");
        let set = crate::psd_layers::decode_psd_layers(&bytes, crate::DecodeOptions::default())
            .expect("decode_psd_layers");
        let group = set
            .groups
            .iter()
            .find(|g| g.name == "组")
            .expect("组记录应写回");
        assert_eq!(group.blend, BlendMode::Multiply);
        assert!((group.opacity - 0.5).abs() < 0.01, "opacity {}", group.opacity);
        assert!(!group.pass_through, "非穿透组不能退化成 pass");
        assert!(group.collapsed, "折叠状态应写回 lsct 2");
    }

    /// 2e) 混合颜色带写回：范围表按「合成源、合成目标、各通道源/目标」顺序
    /// 落成 4 字节一组，读回来必须一致（含拆分值与 RGB 口径）。
    #[test]
    fn blend_if_round_trip() {
        use editcore::{BlendIf, BlendMode, BlendRange, PsdLayerSeed};
        let seed = PsdLayerSeed {
            name: "带".into(),
            offset: (0, 0),
            image: solid(4, 4, [90, 90, 90, 255]),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            group: None,
            mask: None,
            clipping: false,
            clip_shape: None,
            style: None,
            blend_if: None,
            knockout: editcore::Knockout::None,
        };
        let mut doc = Document::from_psd_layers((4, 4), vec![seed]);
        let want = BlendIf {
            source: BlendRange {
                black: [16, 64],
                white: [200, 240],
            },
            destination: BlendRange {
                black: [0, 0],
                white: [128, 128],
            },
            rgb: true,
            source_channels: [
                BlendRange::DEFAULT,
                BlendRange {
                    black: [8, 8],
                    white: [250, 250],
                },
                BlendRange::DEFAULT,
            ],
            destination_channels: [BlendRange::DEFAULT; 3],
        };
        doc.layers[1].blend_if = Some(want.clone());
        let bytes = encode_psd(&doc, 90).expect("encode_psd");
        let set = crate::psd_layers::decode_psd_layers(&bytes, crate::DecodeOptions::default())
            .expect("decode_psd_layers");
        assert_eq!(set.layers[0].blend_if.as_ref(), Some(&want));
    }

    /// 2f) 挖空写回：`8BIMknko` 4 字节（1 浅 / 2 深），关闭时不写键，
    /// 读回来必须一致。
    #[test]
    fn knockout_round_trip() {
        use editcore::{BlendMode, Knockout, PsdLayerSeed};
        let seed = PsdLayerSeed {
            name: "挖".into(),
            offset: (0, 0),
            image: solid(4, 4, [90, 90, 90, 255]),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            group: None,
            mask: None,
            clipping: false,
            clip_shape: None,
            style: None,
            blend_if: None,
            knockout: Knockout::None,
        };
        for want in [Knockout::Shallow, Knockout::Deep] {
            let mut doc = Document::from_psd_layers((4, 4), vec![seed.clone()]);
            doc.layers[1].knockout = want;
            let bytes = encode_psd(&doc, 90).expect("encode_psd");
            let set =
                crate::psd_layers::decode_psd_layers(&bytes, crate::DecodeOptions::default())
                    .expect("decode_psd_layers");
            assert_eq!(set.layers[0].knockout, want);
        }
        let doc = Document::from_psd_layers((4, 4), vec![seed]);
        let bytes = encode_psd(&doc, 90).expect("encode_psd");
        let set = crate::psd_layers::decode_psd_layers(&bytes, crate::DecodeOptions::default())
            .expect("decode_psd_layers");
        assert_eq!(set.layers[0].knockout, Knockout::None);
    }

    /// 2b) Adobe's layer-flags bit 1 is INVERTED (SET = hidden). Photoshop
    /// writes bit 3 on every visible layer and never sets bit 1 unless the
    /// layer is hidden. Writing the vendored parser's old wrong convention
    /// made our files show every layer as hidden in third-party readers.
    #[test]
    fn encoder_writes_spec_compliant_visibility_flags() {
        let mut doc = Document::from_image(solid(4, 4, [1, 2, 3, 255]));
        let mut hidden = Layer::new("隐藏", solid(2, 2, [9, 9, 9, 255]));
        hidden.set_offset((1, 1));
        hidden.visible = false;
        doc.layers.push(hidden);
        let mut shown = Layer::new("显示", solid(2, 2, [8, 8, 8, 255]));
        shown.set_offset((0, 0));
        doc.layers.push(shown);
        let bytes = encode_psd(&doc, 90).expect("encode_psd");
        let flags: Vec<u8> = layer_flag_offsets(&bytes)
            .iter()
            .map(|&o| bytes[o])
            .collect();
        // File order is bottom-to-top: 背景 (visible), 隐藏 (hidden), 显示 (visible).
        assert_eq!(flags.len(), 3);
        assert_eq!(flags[0] & 2, 0, "visible layer must NOT set bit 1");
        assert_eq!(flags[1] & 2, 2, "hidden layer MUST set bit 1");
        assert_eq!(flags[2] & 2, 0, "background must NOT set bit 1");
        assert_eq!(flags[0] & 8, 8, "Photoshop 5+ bit 3 should be set");
    }

    /// 2c) Real-world PSDs use both conventions for bit 3: Photoshop writes
    /// 8/10, older encoders write 0/2. Either way bit 1 must mean HIDDEN.
    #[test]
    fn decoder_reads_third_party_visibility_flags() {
        let mut doc = Document::from_image(solid(4, 4, [1, 2, 3, 255]));
        let mut hidden = Layer::new("隐藏", solid(2, 2, [9, 9, 9, 255]));
        hidden.set_offset((1, 1));
        hidden.visible = false;
        doc.layers.push(hidden);
        let mut shown = Layer::new("显示", solid(2, 2, [8, 8, 8, 255]));
        shown.set_offset((0, 0));
        doc.layers.push(shown);
        let mut bytes = encode_psd(&doc, 90).expect("encode_psd");
        let offsets = layer_flag_offsets(&bytes);
        // Top-to-bottom in the file: shown, hidden, background.
        bytes[offsets[0]] = 0; // old-style visible
        bytes[offsets[1]] = 2; // old-style hidden
        bytes[offsets[2]] = 8; // Photoshop-style visible
        let set = crate::decode_psd_layers(&bytes, crate::DecodeOptions::default())
            .expect("decode_psd_layers");
        // `set.layers` is bottom-to-top: background, hidden, shown.
        assert_eq!(set.layers.len(), 3);
        assert!(set.layers[0].visible, "Photoshop-style flags=8 => visible");
        assert!(!set.layers[1].visible, "flags=2 => hidden");
        assert!(set.layers[2].visible, "flags=0 => visible");
    }

    /// 2d) The synthetic transparent background (`layers[0]`) must not be
    /// written as a real PSD layer, otherwise every save/reopen cycle grows
    /// the layer stack by one. Two generations must stay at the same count.
    #[test]
    fn layered_roundtrip_does_not_accumulate_backgrounds() {
        use editcore::PsdLayerSeed;
        let seed = |name: &str| PsdLayerSeed {
            name: name.into(),
            offset: (1, 1),
            image: solid(2, 2, [30, 60, 90, 255]),
            visible: true,
            opacity: 1.0,
            blend: editcore::BlendMode::Normal,
            group: None,
            mask: None,
            clipping: false,
            clip_shape: None,
            style: None,
            blend_if: None,
            knockout: editcore::Knockout::None,
        };
        let doc = Document::from_psd_layers((4, 4), vec![seed("上层"), seed("下层")]);
        assert_eq!(doc.layers.len(), 3, "background + 2 seeds");

        let bytes = encode_psd(&doc, 90).expect("first encode");
        let set = crate::decode_psd_layers(&bytes, crate::DecodeOptions::default())
            .expect("first decode");
        assert_eq!(set.layers.len(), 2, "synthetic background must be skipped");

        let doc2 = Document::from_psd_layers(set.canvas, set.into_seeds());
        let bytes2 = encode_psd(&doc2, 90).expect("second encode");
        let set2 = crate::decode_psd_layers(&bytes2, crate::DecodeOptions::default())
            .expect("second decode");
        assert_eq!(set2.layers.len(), 2, "second generation must not grow");
    }

    /// 2e) 图层蒙版（`-2` 通道，含自己的矩形与矩形外默认色）与剪贴位必须
    /// 写回 PSD —— 保存后重开不能丢。
    #[test]
    fn mask_and_clipping_roundtrip() {
        use editcore::LayerMask;
        use image::{GrayImage, Luma};

        let mut doc = Document::from_image(solid(8, 6, [10, 20, 30, 255]));
        let mask_px: Vec<u8> = vec![0, 64, 128, 255];
        let mask = GrayImage::from_fn(2, 2, |x, y| Luma([mask_px[(y * 2 + x) as usize]]));
        let mut top = Layer::new("蒙版层", solid(4, 3, [200, 10, 10, 255]));
        top.set_offset((3, 2));
        top.clipping = true;
        // 蒙版矩形比图层小，矩形外按 255 显示：两个信息都必须原样写回。
        top.mask = Some(LayerMask::with_default(mask, (1, 1), 255));
        doc.layers.push(top);

        let bytes = encode_psd(&doc, 90).expect("encode_psd");
        let set = crate::psd_layers::decode_psd_layers(&bytes, crate::DecodeOptions::default())
            .expect("decode_psd_layers");
        assert_eq!(set.layers.len(), 2, "背景 + 蒙版层");
        let got = &set.layers[1];
        assert!(got.clipping, "剪贴位没写回");
        let m = got.mask.as_ref().expect("图层蒙版没写回");
        assert_eq!(m.offset, (1, 1), "蒙版矩形偏移");
        assert_eq!(m.default, 255, "蒙版矩形外默认色");
        assert_eq!(m.image.dimensions(), (2, 2), "蒙版按自己的矩形写");
        assert_eq!(m.image.as_raw(), &mask_px, "蒙版像素");
        assert_eq!(set.masked, 1);
        assert_eq!(set.clipped, 1);
    }

    /// 3) Extreme small canvas: a 1x1 document.
    #[test]
    fn one_by_one_document() {
        let doc = Document::from_image(solid(1, 1, [7, 8, 9, 200]));
        let flat = doc.flatten();
        let bytes = encode_psd(&doc, 0).expect("encode_psd");
        let decoded = crate::decode_psd_bytes(&bytes, crate::DecodeOptions::default())
            .expect("decode_psd_bytes");
        assert_same(&decoded.image, &flat);
    }

    /// 4) Corrupt the Layer & Mask / Layer Info length fields: the guarded
    /// decoder must return Err rather than panic. `validate_psd_sections`
    /// rejects the out-of-bounds section length up front.
    #[test]
    fn corrupt_layer_info_length_returns_err() {
        let doc = Document::from_image(gradient(8, 6));
        let mut bytes = encode_psd(&doc, 90).expect("encode_psd");
        // bytes[34..38] = Layer & Mask section_len, [38..42] = Layer Info len.
        for o in [34usize, 38usize] {
            bytes[o..o + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::decode_psd_bytes(&bytes, crate::DecodeOptions::default())
        }));
        assert!(result.is_ok(), "decoder must not panic");
        assert!(result.unwrap().is_err(), "corrupt length must be rejected");
    }

    /// Dispatch: Psd keeps writing PSD bytes; Image(f) goes through the
    /// existing `encode()` on the flattened composite.
    #[test]
    fn encode_document_dispatches() {
        let doc = Document::from_image(gradient(8, 6));
        let flat = doc.flatten();

        let psd = encode_document(&doc, crate::SaveFormat::Psd, 0).expect("Psd");
        assert!(psd.starts_with(b"8BPS"));
        let decoded = crate::decode_psd_bytes(&psd, crate::DecodeOptions::default())
            .expect("decode_psd_bytes");
        assert_same(&decoded.image, &flat);

        let png = encode_document(&doc, crate::SaveFormat::Image(crate::Format::Png), 90)
            .expect("Image(Png)");
        assert!(png.starts_with(b"\x89PNG"));
        let decoded = crate::decode_bytes(&png, crate::DecodeOptions::default())
            .expect("decode_bytes");
        assert_same(&decoded.image, &flat);
    }

    /// 5) `display` 路径写回：像素必须是**文档色彩空间**，profile 原样保留。
    ///
    /// 用 gamma 1.0 的 RGB profile 凸出两者的差别：转 sRGB 会把中间调显著
    /// 提亮，所以「写 display 像素」和「写 doc 像素」不可能碰巧相等。
    #[test]
    fn write_back_keeps_doc_space_pixels_and_icc() {
        let white = lcms2::CIExyY {
            x: 0.3127,
            y: 0.3290,
            Y: 1.0,
        };
        let primaries = lcms2::CIExyYTRIPLE {
            Red: lcms2::CIExyY {
                x: 0.64,
                y: 0.33,
                Y: 1.0,
            },
            Green: lcms2::CIExyY {
                x: 0.30,
                y: 0.60,
                Y: 1.0,
            },
            Blue: lcms2::CIExyY {
                x: 0.15,
                y: 0.06,
                Y: 1.0,
            },
        };
        let gamma = lcms2::ToneCurve::new(1.0);
        let profile = lcms2::Profile::new_rgb(&white, &primaries, &[&gamma, &gamma, &gamma])
            .expect("gamma 1.0 RGB profile");
        let icc = profile.icc().expect("serialize profile");

        let mut doc = Document::from_image(solid(8, 4, [128, 64, 200, 255]));
        doc.icc = editcore::IccTransform::shared(&icc);
        assert_ne!(
            doc.flatten().as_raw(),
            doc.flatten_in_doc_space().as_raw(),
            "the fake profile must visibly change display pixels"
        );

        let bytes = encode_psd(&doc, 90).expect("encode_psd");
        let psd = psd::Psd::from_bytes(&bytes).expect("re-parse");
        assert_eq!(psd.icc_profile(), Some(&icc[..]));
        // 用 psd 的原始合并图（不走解码器的 ICC 转换）确认写入的是文档空间。
        assert_eq!(psd.rgba(), doc.flatten_in_doc_space().into_raw());
    }
}
