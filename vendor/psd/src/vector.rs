//! 矢量蒙版（`vmsk` / `vsms`）与形状填充（`vscg`）的解析。
//!
//! 形状图层（矩形、圆角矩形、图标）的像素**不在通道里**：Photoshop 用
//! 矢量路径实时生成。这类图层的通道数据全透明（本仓库的素材里 5 张卡片
//! 全是 alpha=0），照着通道烘焙就等于「这一层是空的」——它的描边、内阴影
//! 全部消失，而在 PS 的合并图里这些正是卡片最显眼的部分。
//!
//! 路径记录格式（PSD 规范 + 实测）：
//!
//! ```text
//! u32 version = 3
//! u32 flags                      bit0 = 反相(invert)
//! 记录序列: u16 selector + 负载
//!   0/3  子路径长度记录: u16 节点数 + 22 字节(operation/unknown/10 字节保留)
//!        0 = 闭合, 3 = 开放
//!   1/2/4/5  节点: u16 selector + 6 个 8.24 定点数
//!        (y_preceding, x_preceding, y_anchor, x_anchor, y_leaving, x_leaving)
//!   6    填充规则记录（24 字节）
//!   7    剪贴板记录（24 字节）
//!   8    初始填充规则记录（24 字节）
//! ```
//!
//! 坐标是 8.24 定点（值 / 2^24），**不是**整数像素：本仓库的素材里数值
//! 落在 0..1，即「占画布的比例」。调用方需要判断尺度，见
//! `crate::decode::psd_vector`。

use crate::descriptor;
use crate::effects::Gradient;

/// 一个路径节点：进入 / 锚点 / 离开三个控制点，(y, x) 与 PSD 里一致。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VectorKnot {
    /// 进入这个锚点用的控制点。
    pub preceding: (f32, f32),
    /// 锚点本身（曲线真正经过的点）。
    pub anchor: (f32, f32),
    /// 离开这个锚点用的控制点。
    pub leaving: (f32, f32),
}

/// 一条子路径。闭合子路径组成填充区域，开放子路径只对描边有意义。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VectorSubpath {
    /// `true` = 闭合（参与填充），`false` = 开放（只影响描边）。
    pub closed: bool,
    /// 节点序列，首尾自动相连（闭合时）。
    pub knots: Vec<VectorKnot>,
}

/// 矢量蒙版：形状图层的轮廓。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VectorMask {
    /// Photoshop 的「反相」勾选。
    pub invert: bool,
    /// 轮廓，通常是 1 条；图标可能是多条（各自的孔洞靠环绕方向表达）。
    pub subpaths: Vec<VectorSubpath>,
}

impl VectorMask {
    /// 所有控制点的包围盒 `(x0, y0, x1, y1)`，坐标系统与路径一致。
    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let mut b: Option<(f32, f32, f32, f32)> = None;
        for sp in &self.subpaths {
            for k in &sp.knots {
                for (y, x) in [k.preceding, k.anchor, k.leaving] {
                    if !x.is_finite() || !y.is_finite() {
                        continue;
                    }
                    b = Some(match b {
                        None => (x, y, x, y),
                        Some((x0, y0, x1, y1)) => {
                            (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
                        }
                    });
                }
            }
        }
        b
    }
}

/// 形状图层的填充（`vscg`）：纯色或渐变。
///
/// 两者可能都不存在 —— 那时 PS 里就是无填充的形状，只有描边。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VectorFill {
    /// 纯色填充（`SoCo`）。
    pub solid: Option<[u8; 3]>,
    /// 渐变填充（`GdFl`），几何在体里（角度、类型、色标）。
    pub gradient: Option<Gradient>,
}

/// 解析 `vmsk` / `vsms` 的负载。结构不对就返回 `None`，绝不 panic：
/// 一个坏路径不该让整个文件退回合并图。
pub fn parse_vector_mask(bytes: &[u8]) -> Option<VectorMask> {
    if bytes.len() < 8 {
        return None;
    }
    let version = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if version != 3 {
        return None;
    }
    let flags = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let mut mask = VectorMask {
        invert: flags & 1 != 0,
        subpaths: Vec::new(),
    };
    let mut i = 8usize;
    // 每条子路径的 selector 在长度记录里读到，节点用 1/2/4/5。
    let mut closed = false;
    let mut remaining = 0usize;
    while i + 2 <= bytes.len() {
        let selector = u16::from_be_bytes([bytes[i], bytes[i + 1]]);
        i += 2;
        match selector {
            // 闭合 / 开放子路径的长度记录：节点数 + 22 字节附加数据。
            0 | 3 => {
                if i + 24 > bytes.len() {
                    break;
                }
                let count = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
                i += 24;
                closed = selector == 0;
                remaining = count.min(4096);
                mask.subpaths.push(VectorSubpath {
                    closed,
                    knots: Vec::with_capacity(remaining),
                });
            }
            // 节点记录：6 个 8.24 定点数，y 在前。
            1 | 2 | 4 | 5 => {
                if i + 24 > bytes.len() {
                    break;
                }
                let mut v = [0f32; 6];
                for (k, slot) in v.iter_mut().enumerate() {
                    let o = i + k * 4;
                    let raw = i32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
                    *slot = raw as f32 / 16_777_216.0;
                }
                i += 24;
                if let Some(sp) = mask.subpaths.last_mut() {
                    if remaining > 0 {
                        remaining -= 1;
                    }
                    sp.knots.push(VectorKnot {
                        preceding: (v[0], v[1]),
                        anchor: (v[2], v[3]),
                        leaving: (v[4], v[5]),
                    });
                }
                let _ = closed;
            }
            // 填充规则 / 剪贴板 / 初始填充规则：负载都是固定的 24 字节
            // （规范里含 2 字节子类型；selector 已经单独读过，这里整段跳过，
            //  少读 2 字节会让后面整段错位 —— 曾因此一条路径都读不出来）。
            6 | 7 | 8 => {
                i += 24;
            }
            // 未知 selector：结构已经不可信，停在这里比乱读安全。
            _ => break,
        }
    }
    mask.subpaths.retain(|sp| sp.knots.len() >= 2);
    if mask.subpaths.is_empty() {
        return None;
    }
    Some(mask)
}

/// 解析形状填充的 sheet setting（`SoCo` / `GdFl`）：负载 = u32 版本（实测
/// 16）+ 描述符本体，比 `vscg` 少一层 key 头。`SoCo` 的实色在 `Clr `，
/// `GdFl` 的渐变在 `Grad` 子描述符里 —— 与 `vscg` 内层结构一致。
pub fn parse_sheet_fill(bytes: &[u8]) -> Option<VectorFill> {
    if bytes.len() < 8 {
        return None;
    }
    let desc = descriptor::parse_body(&bytes[4..])?;
    let mut fill = VectorFill::default();
    if let Some(clr) = desc.descriptor(b"Clr ") {
        fill.solid = rgb_from(clr);
    }
    if fill.solid.is_none() {
        if let Some(g) = desc.descriptor(b"Grad") {
            fill.gradient = Some(crate::effects::gradient_from_descriptor(&desc, g));
        }
    }
    if fill.solid.is_none() && fill.gradient.is_none() {
        return None;
    }
    Some(fill)
}

/// 解析 `vscg` 的负载：`4 字节 key + u32 version + descriptor body`。
pub fn parse_vector_fill(bytes: &[u8]) -> Option<VectorFill> {
    if bytes.len() < 12 {
        return None;
    }
    let desc = descriptor::parse_body(&bytes[8..])?;
    let mut fill = VectorFill::default();
    if let Some(soco) = desc.descriptor(b"SoCo") {
        fill.solid = soco.descriptor(b"Clr ").and_then(rgb_from);
    }
    if let Some(gdfl) = desc.descriptor(b"GdFl") {
        if let Some(g) = gdfl.descriptor(b"Grad") {
            fill.gradient = Some(crate::effects::gradient_from_descriptor(&gdfl, g));
        }
    }
    if fill.solid.is_none() && fill.gradient.is_none() {
        // 少数文件把填充直接放在描述符本身（没有 SoCo/GdFl 包一层）。
        if let Some(g) = desc.descriptor(b"Grad") {
            fill.gradient = Some(crate::effects::gradient_from_descriptor(&desc, g));
        } else if let Some(soco) = desc.descriptor(b"Clr ") {
            fill.solid = rgb_from(soco);
        }
    }
    if fill.solid.is_none() && fill.gradient.is_none() {
        return None;
    }
    Some(fill)
}

/// 描述符里的颜色：`Clr ` 是 0..255 的浮点三元组。
fn rgb_from(d: &descriptor::Descriptor) -> Option<[u8; 3]> {
    let mut out = [0u8; 3];
    for (i, key) in [&b"Rd  "[..], &b"Grn "[..], &b"Bl  "[..]].iter().enumerate() {
        let v = d.number(key)?;
        out[i] = v.round().clamp(0.0, 255.0) as u8;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be32(v: i32) -> [u8; 4] {
        v.to_be_bytes()
    }

    /// 一个正方形子路径：两条记录的布局必须按规范读出来。
    #[test]
    fn reads_a_closed_square_subpath() {
        let mut b = Vec::new();
        b.extend_from_slice(&3u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        // 子路径长度记录：2 个节点。
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&2u16.to_be_bytes());
        b.extend_from_slice(&1i16.to_be_bytes());
        b.extend_from_slice(&[0u8; 20]);
        // 节点：全部控制点就在锚点上（直线）。
        for (y, x) in [(0.25f32, 0.5f32), (0.75, 0.5)] {
            b.extend_from_slice(&1u16.to_be_bytes());
            for v in [y, x, y, x, y, x] {
                b.extend_from_slice(&be32((v * 16_777_216.0) as i32));
            }
        }
        let m = parse_vector_mask(&b).expect("mask");
        assert_eq!(m.subpaths.len(), 1);
        assert!(m.subpaths[0].closed);
        assert_eq!(m.subpaths[0].knots.len(), 2);
        assert!((m.subpaths[0].knots[0].anchor.1 - 0.5).abs() < 1e-5);
        let (x0, y0, x1, y1) = m.bounds().unwrap();
        assert!((x0 - 0.5).abs() < 1e-5 && (x1 - 0.5).abs() < 1e-5);
        assert!((y0 - 0.25).abs() < 1e-5 && (y1 - 0.75).abs() < 1e-5);
    }

    /// invert 是 flags 的最低位。
    #[test]
    fn invert_flag_is_read_from_the_header() {
        let mut b = Vec::new();
        b.extend_from_slice(&3u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        assert!(parse_vector_mask(&b).is_none(), "没有路径就不算蒙版");
        let mut with_path = b.clone();
        with_path.extend_from_slice(&0u16.to_be_bytes());
        with_path.extend_from_slice(&2u16.to_be_bytes());
        with_path.extend_from_slice(&1i16.to_be_bytes());
        with_path.extend_from_slice(&[0u8; 20]);
        for (y, x) in [(0.1f32, 0.1f32), (0.9, 0.9)] {
            with_path.extend_from_slice(&1u16.to_be_bytes());
            for v in [y, x, y, x, y, x] {
                with_path.extend_from_slice(&be32((v * 16_777_216.0) as i32));
            }
        }
        let m = parse_vector_mask(&with_path).expect("mask");
        assert!(m.invert);
    }

    /// 截断的块不能 panic，也不能读出半条路径。
    #[test]
    fn truncated_block_is_rejected_quietly() {
        let mut b = Vec::new();
        b.extend_from_slice(&3u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&2u16.to_be_bytes());
        b.extend_from_slice(&[0u8; 4]);
        assert!(parse_vector_mask(&b).is_none());
    }
}
