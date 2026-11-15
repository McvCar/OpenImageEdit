//! PSD 图案（图像资源 `Patt` / `Pat2` / `Pat3`，id 2491..2493）。
//!
//! 矢量填充（`vscg` 的 `PtFl`）和图层样式的图案叠加都只存**图案引用**
//! （ID / 名称 / 缩放 / 角度），像素本身在这里。图案通道与图层通道一样是
//! 逐行 PackBits，所以解压复用 `psd_layers::unpack_packbits`。
//!
//! 只支持 8 位 RGB / 灰度图案；其它模式（16/32 位、CMYK、索引）直接跳过，
//! 而不是猜颜色 —— 猜错会把整块区域涂成错误颜色，比不画更糟。

use image::RgbaImage;
use psd::{ImageResource, VectorPattern};

/// 一张图案的像素与元数据。
#[derive(Debug, Clone)]
pub struct PatternTile {
    pub id: String,
    pub name: String,
    pub image: RgbaImage,
}

/// 文件里的全部图案，按 ID / 名称索引。
#[derive(Debug, Default, Clone)]
pub struct PatternStore {
    pub tiles: Vec<PatternTile>,
}

impl PatternStore {
    /// 从图像资源里收集图案。没有图案资源时返回空表。
    pub fn from_resources(resources: &[ImageResource]) -> Self {
        let mut tiles = Vec::new();
        for r in resources {
            let ImageResource::Raw(raw) = r else {
                continue;
            };
            if !(2491..=2493).contains(&raw.resource_id()) {
                continue;
            }
            tiles.extend(parse_patterns(raw.data()));
        }
        Self { tiles }
    }

    /// 按 ID 找，找不到再按名称找（同一个图案在不同 PS 版本里可能只留其一）。
    pub fn find(&self, r: &VectorPattern) -> Option<&PatternTile> {
        if !r.id.is_empty() {
            if let Some(t) = self.tiles.iter().find(|t| t.id == r.id) {
                return Some(t);
            }
        }
        if !r.name.is_empty() {
            return self.tiles.iter().find(|t| t.name == r.name);
        }
        None
    }

    /// 为一次绘制准备采样器。`None` = 文件里没有这张图案的像素。
    pub fn paint(&self, r: &VectorPattern) -> Option<PatternPaint<'_>> {
        let tile = self.find(r)?;
        Some(PatternPaint {
            tile: &tile.image,
            scale: r.scale,
            angle: r.angle,
            phase: (0.0, 0.0),
        })
    }
}

/// 图案铺贴采样器。
///
/// 坐标是**画布**像素；`anchor` 是图案原点（PS 的「与图层对齐」用图层
/// 左上角，否则用文档原点）。缩放按百分比（100 = 原尺寸），角度顺时针。
#[derive(Clone, Copy)]
pub struct PatternPaint<'a> {
    pub tile: &'a RgbaImage,
    pub scale: f32,
    pub angle: f32,
    /// 图案相位（像素，先于旋转/缩放施加）。
    pub phase: (f32, f32),
}

impl PatternPaint<'_> {
    /// 取 `(x, y)` 处的图案像素；超出图案矩阵时平铺回绕。
    pub fn sample(&self, x: f32, y: f32, anchor: (f32, f32)) -> [u8; 4] {
        let w = self.tile.width() as i64;
        let h = self.tile.height() as i64;
        if w == 0 || h == 0 {
            return [0, 0, 0, 0];
        }
        let s = (self.scale / 100.0).clamp(0.01, 100.0);
        let rad = self.angle.to_radians();
        let (sin, cos) = rad.sin_cos();
        let dx = x + self.phase.0 - anchor.0;
        let dy = y + self.phase.1 - anchor.1;
        // 屏幕 → 图案：逆旋转，再除以缩放。
        let rx = (dx * cos + dy * sin) / s;
        let ry = (-dx * sin + dy * cos) / s;
        let px = rx.floor() as i64;
        let py = ry.floor() as i64;
        let px = px.rem_euclid(w) as u32;
        let py = py.rem_euclid(h) as u32;
        self.tile.get_pixel(px, py).0
    }
}

/// 解析一段 `Patt` / `Pat2` / `Pat3` 资源负载。
pub fn parse_patterns(data: &[u8]) -> Vec<PatternTile> {
    let mut out = Vec::new();
    // 资源以 4 字节版本号（1）开头。
    if data.len() < 4 {
        return out;
    }
    let mut i = 4usize;
    while i + 4 <= data.len() {
        let len = be_u32(&data[i..]).unwrap_or(0) as usize;
        i += 4;
        if len == 0 || i + len > data.len() {
            break;
        }
        if let Some(t) = parse_one(&data[i..i + len]) {
            out.push(t);
        }
        i += len;
        // 每个长度块按 4 字节对齐（长度字段本身已对齐）。
        i += (4 - len % 4) % 4;
    }
    out
}

fn parse_one(b: &[u8]) -> Option<PatternTile> {
    let mut c = Cursor::new(b);
    c.u32()?; // version
    let mode = c.u32()?;
    c.u16()?; // height（真实尺寸以 VirtualMemoryArrayList 的矩形为准）
    c.u16()?; // width
    let name = c.unicode_string()?;
    let id = c.pascal_string()?;
    if mode == 2 {
        // 索引色表：长度 + 数据。
        let len = c.u32()? as usize;
        c.skip(len)?;
    }
    let (w, h, channels) = c.memory_array_list()?;
    let written: Vec<&Vec<u8>> = channels.iter().filter_map(|c| c.as_ref()).collect();
    if written.is_empty() || w == 0 || h == 0 {
        return None;
    }
    let mut rgba = vec![255u8; (w * h * 4) as usize];
    match mode {
        3 => {
            // RGB：写出的通道按 R,G,B[,A] 排列；最后一个是透明蒙版。
            if written.len() < 3 {
                return None;
            }
            let alpha = if written.len() > 3 {
                Some(&written[written.len() - 1])
            } else {
                None
            };
            for i in 0..(w * h) as usize {
                rgba[i * 4] = written[0][i];
                rgba[i * 4 + 1] = written[1][i];
                rgba[i * 4 + 2] = written[2][i];
                if let Some(a) = alpha {
                    rgba[i * 4 + 3] = a[i];
                }
            }
        }
        1 => {
            // 灰度：第一个通道是亮度，最后一个是透明蒙版。
            let alpha = if written.len() > 1 {
                Some(&written[written.len() - 1])
            } else {
                None
            };
            for i in 0..(w * h) as usize {
                let l = written[0][i];
                rgba[i * 4] = l;
                rgba[i * 4 + 1] = l;
                rgba[i * 4 + 2] = l;
                if let Some(a) = alpha {
                    rgba[i * 4 + 3] = a[i];
                }
            }
        }
        _ => return None,
    }
    let image = RgbaImage::from_raw(w, h, rgba)?;
    Some(PatternTile { id, name, image })
}

struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, i: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.i.checked_add(n)?;
        if end > self.b.len() {
            return None;
        }
        let s = &self.b[self.i..end];
        self.i = end;
        Some(s)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn unicode_string(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        let raw = self.take(n.checked_mul(2)?)?;
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        Some(String::from_utf16_lossy(&units))
    }
    fn pascal_string(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        let s = String::from_utf8_lossy(self.take(n)?).into_owned();
        // Pascal 字符串按 2 字节对齐。
        if (1 + n) % 2 == 1 {
            self.skip(1)?;
        }
        Some(s)
    }

    /// VirtualMemoryArrayList → `(width, height, 每通道像素)`。
    fn memory_array_list(&mut self) -> Option<(u32, u32, Vec<Option<Vec<u8>>>)> {
        self.u32()?; // version = 3
        let len = self.u32()? as usize;
        let body = self.take(len)?;
        let mut c = Cursor::new(body);
        let top = c.u32()?;
        let left = c.u32()?;
        let bottom = c.u32()?;
        let right = c.u32()?;
        let count = c.u32()? as usize;
        let w = right.saturating_sub(left);
        let h = bottom.saturating_sub(top);
        if w == 0 || h == 0 || w as u64 * h as u64 > 16_000_000 {
            return None;
        }
        let mut channels = Vec::with_capacity(count + 2);
        for _ in 0..count + 2 {
            let Some(ch) = c.memory_array(w, h) else {
                break;
            };
            channels.push(ch);
        }
        Some((w, h, channels))
    }

    fn memory_array(&mut self, w: u32, h: u32) -> Option<Option<Vec<u8>>> {
        let is_written = self.u32()?;
        let len = self.u32()? as usize;
        let body = self.take(len)?;
        if is_written == 0 {
            return Some(None);
        }
        let mut c = Cursor::new(body);
        let depth = c.u32()?;
        c.u32()?; // top
        c.u32()?; // left
        c.u32()?; // bottom
        c.u32()?; // right
        c.u16()?; // pixel depth
        let compression = c.u8()?;
        if depth != 8 {
            return Some(None);
        }
        let expected = w as usize * h as usize;
        let raw = c.take(body.len().saturating_sub(c.i))?;
        let pixels = match compression {
            0 => {
                if raw.len() < expected {
                    return Some(None);
                }
                raw[..expected].to_vec()
            }
            1 => {
                // 8 位 RLE：先 h 个 u16 行长度，再逐行 PackBits。
                let table = h as usize * 2;
                if raw.len() < table {
                    return Some(None);
                }
                let mut out = Vec::with_capacity(expected);
                // 行偏移需要累计前面的行长度。
                let mut off = table;
                for row in 0..h as usize {
                    let n = u16::from_be_bytes([raw[row * 2], raw[row * 2 + 1]]) as usize;
                    let end = (off + n).min(raw.len());
                    let mut line = crate::psd_layers::unpack_packbits(&raw[off..end], w as usize);
                    line.resize(w as usize, 0);
                    out.extend_from_slice(&line);
                    off = end;
                }
                out
            }
            _ => return Some(None),
        };
        Some(Some(pixels))
    }
}

fn be_u32(b: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(..4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一张 2x2 的 RGB 图案资源（version + length 块）。
    fn build_pattern(id: &str, rgb: [[u8; 3]; 4]) -> Vec<u8> {
        let mut pattern = Vec::new();
        pattern.extend_from_slice(&1u32.to_be_bytes()); // version
        pattern.extend_from_slice(&3u32.to_be_bytes()); // RGB
        pattern.extend_from_slice(&2u16.to_be_bytes()); // height
        pattern.extend_from_slice(&2u16.to_be_bytes()); // width
        pattern.extend_from_slice(&1u32.to_be_bytes()); // name chars
        pattern.extend_from_slice(&[0x00, b'P']); // "P"
                                                  // Pascal id，按 2 字节对齐。
        pattern.push(id.len() as u8);
        pattern.extend_from_slice(id.as_bytes());
        if (1 + id.len()) % 2 == 1 {
            pattern.push(0);
        }
        // VirtualMemoryArrayList
        let mut body = Vec::new();
        for v in [0u32, 0, 2, 2] {
            body.extend_from_slice(&v.to_be_bytes());
        }
        body.extend_from_slice(&3u32.to_be_bytes()); // 3 个通道
        for ch in 0..5 {
            let written = ch < 3;
            body.extend_from_slice(&(written as u32).to_be_bytes());
            if !written {
                body.extend_from_slice(&0u32.to_be_bytes());
                continue;
            }
            let mut arr = Vec::new();
            arr.extend_from_slice(&8u32.to_be_bytes()); // depth
            for v in [0u32, 0, 2, 2] {
                arr.extend_from_slice(&v.to_be_bytes());
            }
            arr.extend_from_slice(&8u16.to_be_bytes());
            arr.push(0); // raw
            for px in &rgb {
                arr.push(px[ch]);
            }
            body.extend_from_slice(&(arr.len() as u32).to_be_bytes());
            body.extend_from_slice(&arr);
        }
        pattern.extend_from_slice(&3u32.to_be_bytes()); // list version
        pattern.extend_from_slice(&(body.len() as u32).to_be_bytes());
        pattern.extend_from_slice(&body);

        let mut out = Vec::new();
        out.extend_from_slice(&1u32.to_be_bytes()); // resource version
        out.extend_from_slice(&(pattern.len() as u32).to_be_bytes());
        out.extend_from_slice(&pattern);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    #[test]
    fn parses_rgb_pattern_and_tiles() {
        let data = build_pattern(
            "abc",
            [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]],
        );
        let tiles = parse_patterns(&data);
        assert_eq!(tiles.len(), 1);
        let t = &tiles[0];
        assert_eq!(t.id, "abc");
        assert_eq!(t.image.dimensions(), (2, 2));
        assert_eq!(t.image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(t.image.get_pixel(1, 0).0, [0, 255, 0, 255]);
        assert_eq!(t.image.get_pixel(0, 1).0, [0, 0, 255, 255]);

        let store = PatternStore { tiles };
        let r = VectorPattern {
            id: "abc".into(),
            name: "P".into(),
            scale: 100.0,
            angle: 0.0,
            align: true,
        };
        let paint = store.paint(&r).unwrap();
        // 平铺：2x2 之外的坐标回绕。
        assert_eq!(paint.sample(0.0, 0.0, (0.0, 0.0)), [255, 0, 0, 255]);
        assert_eq!(paint.sample(2.0, 0.0, (0.0, 0.0)), [255, 0, 0, 255]);
        assert_eq!(paint.sample(3.0, 1.0, (0.0, 0.0)), [255, 255, 0, 255]);
        // 200% 缩放：每个图案像素占 2x2。
        let r2 = VectorPattern { scale: 200.0, ..r };
        let paint2 = store.paint(&r2).unwrap();
        assert_eq!(paint2.sample(3.0, 0.0, (0.0, 0.0)), [0, 255, 0, 255]);
    }

    #[test]
    fn skips_unsupported_modes_and_bad_blocks() {
        let mut data = build_pattern("x", [[0, 0, 0]; 4]);
        // 改 image mode 为 CMYK（4），解析应跳过。
        let mode_at = 4 + 4 + 4;
        data[mode_at..mode_at + 4].copy_from_slice(&4u32.to_be_bytes());
        assert!(parse_patterns(&data).is_empty());
        assert!(parse_patterns(&[]).is_empty());
        assert!(parse_patterns(&[0, 0, 0, 1]).is_empty());
    }
}
