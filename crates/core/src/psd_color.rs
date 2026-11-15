//! PSD 内嵌 ICC（图像资源 `1039`）-> sRGB 的色彩管理。
//!
//! Photoshop 在**文档色彩空间**里合成，只在最终显示时才转到屏幕空间。
//! 我们两条路径都实现，用环境变量 `IMAGEEDIT_PSD_ICC` 选择（默认见
//! [`DEFAULT_MODE`]）：
//!
//! - `off`：完全不转换（引入色彩管理前的行为，用于消融 / 回退）；
//! - `decode`：解码时就把每个图层位图转到 sRGB，层间混合因此在 sRGB 里做；
//! - `display`：图层保持文档空间，合成完成后再转（与 PS 的顺序一致）。
//!
//! 没有内嵌 profile 的文件三条路径完全一致：`Document::icc` 为 `None`，
//! 每条转换都直接返回，不建 transform、不碰像素。

use std::fmt;

use image::RgbaImage;
use lcms2::{DisallowCache, Flags, GlobalContext, Intent, PixelFormat, Profile, Transform};

pub const ICC_ENV: &str = "IMAGEEDIT_PSD_ICC";

/// 默认路径：显示时转。跨样本实测（含 logo.psd 与不带 profile 的样本）
/// 见提交信息；`off` / `decode` 可随时用环境变量回退或做消融。
pub const DEFAULT_MODE: IccMode = IccMode::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IccMode {
    Off,
    Decode,
    Display,
}

impl IccMode {
    /// 读开关。非法值按默认处理（拼错环境变量不应该悄悄关掉色彩管理）。
    pub fn from_env() -> Self {
        match std::env::var(ICC_ENV).ok().as_deref() {
            Some("off") => IccMode::Off,
            Some("decode") => IccMode::Decode,
            Some("display") => IccMode::Display,
            _ => DEFAULT_MODE,
        }
    }
}

/// 文档色彩空间 -> sRGB 的缓存变换。
///
/// 只活在 `Document` / `PsdLayerSet` 里，按文档建一次，之后每次合成复用。
/// `NO_CACHE` 让 lcms 的 transform 可以跨线程共用（`Sync`），
/// 预览合成、后台保存都不需要额外锁。
pub struct IccTransform {
    // 两个 profile 必须活到 transform 之后：lcms 的句柄不持有它们的引用计数。
    _src: Profile,
    _dst: Profile,
    xform: Transform<u8, u8, GlobalContext, DisallowCache>,
    raw: Vec<u8>,
}

impl fmt::Debug for IccTransform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IccTransform")
            .field("bytes", &self.raw.len())
            .finish()
    }
}

/// 跨线程共享的句柄。[`Profile`] 的图形句柄不是 `Sync`，而文档会跟着
/// 后台保存线程跑，所以包一层 `Mutex`（锁只在整幅转换时持有，UI 线程
/// 没有争用）。
pub type SharedIcc = std::sync::Arc<std::sync::Mutex<IccTransform>>;

impl IccTransform {
    /// 建一个共享句柄；坏 profile 返回 `None`。
    pub fn shared(profile: &[u8]) -> Option<SharedIcc> {
        Self::new(profile).map(|t| std::sync::Arc::new(std::sync::Mutex::new(t)))
    }

    /// 解析内嵌 profile 并建到 sRGB 的变换。
    ///
    /// 坏 profile / 非 RGB profile 都返回 `None`：调用方按「没有 profile」
    /// 继续，绝不能因为色彩管理失败而让文件打不开。
    pub fn new(profile: &[u8]) -> Option<Self> {
        let src = Profile::new_icc(profile).ok()?;
        if src.color_space() != lcms2::ColorSpaceSignature::RgbData {
            return None;
        }
        let dst = Profile::new_srgb();
        // `NO_CACHE` 是让 transform 可以跨线程共用（`Sync`）的前提。
        let xform: Transform<u8, u8, GlobalContext, DisallowCache> =
            Transform::<u8, u8, GlobalContext, DisallowCache>::new_flags_context(
                GlobalContext::new(),
                &src,
                PixelFormat::RGBA_8,
                &dst,
                PixelFormat::RGBA_8,
                Intent::Perceptual,
                Flags::NO_CACHE | Flags::COPY_ALPHA,
            )
            .ok()?;
        Some(Self {
            _src: src,
            _dst: dst,
            xform,
            raw: profile.to_vec(),
        })
    }

    /// 原始 ICC 字节（显示时转的写回要原样保留 profile）。
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// 原地把整幅 RGBA 从文档空间转到 sRGB。alpha 原样复制。
    pub fn apply(&self, image: &mut RgbaImage) {
        let buf: &mut [u8] = image;
        self.xform.transform_in_place(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn no_profile_means_no_transform() {
        assert!(IccTransform::new(&[]).is_none());
        // 垃圾字节也不能 panic，只是拿不到变换。
        assert!(IccTransform::new(b"not an icc profile").is_none());
    }

    #[test]
    fn srgb_profile_is_near_identity() {
        let bytes = Profile::new_srgb().icc().expect("serialize sRGB");
        let xform = IccTransform::new(&bytes).expect("parse sRGB");
        assert_eq!(xform.raw(), &bytes[..]);
        let mut img = RgbaImage::from_fn(4, 1, |x, _| {
            Rgba([x as u8 * 60, 128, 200 - x as u8 * 40, 128])
        });
        let before = img.clone();
        xform.apply(&mut img);
        for (a, b) in before.pixels().zip(img.pixels()) {
            for c in 0..3 {
                assert!(
                    (a.0[c] as i16 - b.0[c] as i16).abs() <= 1,
                    "sRGB->sRGB moved a channel: {:?} -> {:?}",
                    a.0,
                    b.0
                );
            }
            assert_eq!(a.0[3], b.0[3], "alpha must be copied through");
        }
    }
}
