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
    ///
    /// 只转**直通 alpha 的颜色**。PSD 合并图里存的是「白底预乘」的 RGB，
    /// 解预乘之后才是颜色；先转色再解预乘会让同一个颜色随 alpha 漂移
    /// （psd-tools 的 `topil(apply_icc=True)` 就是这个顺序，半透明边缘因此
    /// 对不上），那不是色彩管理，只是存储约定的算术残留。
    pub fn apply(&self, image: &mut RgbaImage) {
        let buf: &mut [u8] = image;
        self.xform.transform_in_place(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 一个与 sRGB 明显不同的 RGB profile（gamma 可调），用来让转换真的
    /// 改变像素，而不是被 sRGB 恒等掩盖。
    fn gamma_profile(gamma: f64) -> Vec<u8> {
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
        let trc = lcms2::ToneCurve::new(gamma);
        lcms2::Profile::new_rgb(&white, &primaries, &[&trc, &trc, &trc])
            .expect("RGB profile")
            .icc()
            .expect("serialize profile")
    }

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

    /// 半透明像素的转色口径：颜色与 alpha 无关。
    ///
    /// Photoshop 在文档空间合成，alpha 只是覆盖率；同一个颜色不该因为
    /// 不透明度不同而被转成不同颜色。
    #[test]
    fn semi_transparent_color_is_alpha_independent() {
        let xform = IccTransform::new(&gamma_profile(1.0)).expect("profile");
        let mut out = Vec::new();
        for a in [1u8, 5, 64, 128, 254] {
            let mut img = RgbaImage::from_pixel(1, 1, Rgba([40, 120, 200, a]));
            xform.apply(&mut img);
            let px = img.get_pixel(0, 0).0;
            assert_eq!(px[3], a, "alpha must survive the transform");
            out.push([px[0], px[1], px[2]]);
        }
        assert!(
            out.windows(2).all(|w| w[0] == w[1]),
            "same colour, different alpha must convert identically: {out:?}"
        );
    }

    /// 反例：psd-tools 的 `apply_icc=True` 先把「白底预乘」的合并图转色，
    /// 再解预乘 —— 同一个颜色会随 alpha 漂移。这个测试把那个顺序钉死为
    /// 「不等于我们的结果」，防止以后为了对齐参考数值而悄悄改成它。
    #[test]
    fn matte_first_conversion_is_not_what_we_do() {
        let xform = IccTransform::new(&gamma_profile(1.0)).expect("profile");
        let (c, a) = ([40u8, 120, 200], 64u8);

        let mut ours = RgbaImage::from_pixel(1, 1, Rgba([c[0], c[1], c[2], a]));
        xform.apply(&mut ours);
        let ours = ours.get_pixel(0, 0).0;

        // 白底预乘 -> 转色 -> 解预乘（PSD 合并图的存储约定）。
        let f = a as f32 / 255.0;
        let matte: Vec<u8> = c
            .iter()
            .map(|&x| (f * x as f32 + (1.0 - f) * 255.0).round() as u8)
            .collect();
        let mut m = RgbaImage::from_pixel(1, 1, Rgba([matte[0], matte[1], matte[2], 255]));
        xform.apply(&mut m);
        let converted = m.get_pixel(0, 0).0;
        let unmatte: Vec<u8> = (0..3)
            .map(|i| {
                ((converted[i] as f32 + a as f32 - 255.0) * 255.0 / a as f32)
                    .round()
                    .clamp(0.0, 255.0) as u8
            })
            .collect();

        assert_ne!(&unmatte[..], &ours[..3]);
    }
}
