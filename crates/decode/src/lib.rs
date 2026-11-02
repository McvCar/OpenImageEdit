use std::io::Cursor;
use std::path::Path;

use image::AnimationDecoder;
use image::{ImageFormat, ImageReader, RgbaImage};

pub use image::ImageFormat as Format;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("unsupported format")]
    UnsupportedFormat,
    /// Formats the app can open but not write (DDS / HDR in image 0.25).
    /// Surfaces as a clear status message instead of a generic failure.
    #[error("{0} 为只读格式，可打开查看，请另存为 PNG / TIFF 等格式")]
    ReadOnlyFormat(String),
    /// PSD is preview-only (flattened composite); this error carries the
    /// concrete reason a particular file can't be previewed.
    #[error("PSD 打开失败：{0}")]
    Psd(String),
    #[error("image is {width}x{height} ({pixels} px), exceeds limit of {limit} px")]
    TooLarge {
        width: u32,
        height: u32,
        pixels: u64,
        limit: u64,
    },
    #[error("image decode failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Format of the decoded source. Everything `image` handles natively shares
/// its `ImageFormat`; PSD is parsed by the `psd` crate and has no
/// `ImageFormat` variant of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Image(Format),
    Psd,
}

impl PartialEq<Format> for SourceFormat {
    fn eq(&self, other: &Format) -> bool {
        matches!(self, SourceFormat::Image(f) if f == other)
    }
}

impl PartialEq<SourceFormat> for Format {
    fn eq(&self, other: &SourceFormat) -> bool {
        other == self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DecodeOptions {
    pub max_pixels: Option<u64>,
    /// Downsampling-first open: when the longest edge exceeds this value the
    /// image is decoded once, then resampled (Lanczos3, aspect preserved)
    /// before it enters the document. Bounds memory and every downstream
    /// operation (preview composite, undo snapshots, transforms) to the
    /// working resolution. `None` keeps the original pixels.
    pub max_dimension: Option<u32>,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            max_pixels: Some(300_000_000),
            max_dimension: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    pub format: SourceFormat,
    pub width: u32,
    pub height: u32,
}

impl ImageInfo {
    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

#[derive(Debug)]
pub struct Decoded {
    pub info: ImageInfo,
    pub image: RgbaImage,
    pub frame_count: usize,
    /// Original (width, height) when `max_dimension` shrank the bitmap,
    /// `None` when the image kept its source resolution.
    pub downsampled_from: Option<(u32, u32)>,
    /// Status-bar hint the decoder wants the user to see after opening
    /// (e.g. "PSD preview: layers were merged"). `None` for plain formats.
    pub note: Option<String>,
}

pub fn detect_format(bytes: &[u8]) -> Option<Format> {
    image::guess_format(bytes).ok()
}

pub fn format_from_path(path: &Path) -> Option<Format> {
    ImageFormat::from_path(path).ok()
}

pub fn probe_bytes(bytes: &[u8]) -> Result<ImageInfo, DecodeError> {
    if let Some((width, height)) = psd_dimensions(bytes) {
        return Ok(ImageInfo {
            format: SourceFormat::Psd,
            width,
            height,
        });
    }
    let format = detect_format(bytes).ok_or(DecodeError::UnsupportedFormat)?;
    let reader = ImageReader::with_format(Cursor::new(bytes), format);
    let (width, height) = reader.into_dimensions()?;
    Ok(ImageInfo {
        format: SourceFormat::Image(format),
        width,
        height,
    })
}

pub fn probe_path(path: &Path) -> Result<ImageInfo, DecodeError> {
    if is_psd_path(path) {
        let bytes = std::fs::read(path)?;
        return probe_bytes(&bytes);
    }
    // Content first, extension second. Files whose extension lies are common
    // (`.png` holding JPEG bytes, renamed `.jpg` screenshots …) and the old
    // extension-first order handed the wrong decoder to `into_dimensions`,
    // which surfaced as "this image cannot be opened". `with_guessed_format`
    // reads the magic bytes and only keeps the extension guess when the
    // content is unrecognisable (TGA and friends have no signature).
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let format = reader.format().ok_or(DecodeError::UnsupportedFormat)?;
    let (width, height) = reader.into_dimensions()?;
    Ok(ImageInfo {
        format: SourceFormat::Image(format),
        width,
        height,
    })
}

pub fn decode_bytes(bytes: &[u8], options: DecodeOptions) -> Result<Decoded, DecodeError> {
    if bytes.starts_with(PSD_MAGIC) {
        return decode_psd_bytes(bytes, options);
    }
    let format = detect_format(bytes).ok_or(DecodeError::UnsupportedFormat)?;
    decode_with(bytes, format, options)
}

pub fn decode_path(path: &Path, options: DecodeOptions) -> Result<Decoded, DecodeError> {
    let bytes = std::fs::read(path)?;
    if bytes.starts_with(PSD_MAGIC) {
        return decode_psd_bytes(&bytes, options);
    }
    // Same "content first" rule as `probe_path`: a `.png` that actually holds
    // JPEG data must be decoded by the JPEG decoder, otherwise `decode_with`
    // fails with a signature error and the user only sees "cannot open".
    let format = detect_format(&bytes)
        .or_else(|| format_from_path(path))
        .ok_or(DecodeError::UnsupportedFormat)?;
    decode_with(&bytes, format, options)
}

/// Longest-edge downsampling shared by the `image` and PSD paths: decode
/// happens at full resolution, then Lanczos3 shrinks before the bitmap
/// enters the document. Returns `(image, original_dimensions)`.
fn apply_max_dimension(
    image: RgbaImage,
    max_dimension: Option<u32>,
) -> (RgbaImage, Option<(u32, u32)>) {
    match max_dimension {
        Some(limit) if limit > 0 => {
            let (w, h) = (image.width(), image.height());
            let longest = w.max(h);
            if longest > limit {
                let scale = limit as f64 / longest as f64;
                let nw = ((w as f64 * scale).round() as u32).max(1);
                let nh = ((h as f64 * scale).round() as u32).max(1);
                (
                    image::imageops::resize(&image, nw, nh, image::imageops::FilterType::Lanczos3),
                    Some((w, h)),
                )
            } else {
                (image, None)
            }
        }
        _ => (image, None),
    }
}

fn decode_with(
    bytes: &[u8],
    format: Format,
    options: DecodeOptions,
) -> Result<Decoded, DecodeError> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    if let Some(limit) = options.max_pixels {
        let (width, height) = reader.into_dimensions()?;
        if width as u64 * height as u64 > limit {
            return Err(DecodeError::TooLarge {
                width,
                height,
                pixels: width as u64 * height as u64,
                limit,
            });
        }
        reader = ImageReader::with_format(Cursor::new(bytes), format);
    }

    let image = reader.decode()?.to_rgba8();
    // Downsampling-first: only the header probe above can reject early, so
    // the shrink happens right after decode (before the bitmap is handed to
    // the document). Lanczos3 keeps the working copy sharp enough to edit.
    let (image, downsampled_from) = apply_max_dimension(image, options.max_dimension);
    let frame_count = if format == Format::Gif {
        count_gif_frames(bytes).unwrap_or(1)
    } else {
        1
    };
    Ok(Decoded {
        info: ImageInfo {
            format: SourceFormat::Image(format),
            width: image.width(),
            height: image.height(),
        },
        image,
        frame_count,
        downsampled_from,
        note: None,
    })
}

// ---- PSD (preview-only) ----

const PSD_MAGIC: &[u8; 4] = b"8BPS";

fn is_psd_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("psd"))
}

/// Parse the 26-byte PSD file header: (width, height, depth, color mode).
/// Widths/heights/mode are big-endian per the Adobe spec.
fn psd_header(bytes: &[u8]) -> Option<(u32, u32, u16, u16)> {
    if bytes.len() < 26 || !bytes.starts_with(PSD_MAGIC) {
        return None;
    }
    let u16be = |o: usize| u16::from_be_bytes([bytes[o], bytes[o + 1]]);
    let u32be =
        |o: usize| u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    Some((u32be(18), u32be(14), u16be(22), u16be(24)))
}

fn psd_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let (width, height, _, _) = psd_header(bytes)?;
    Some((width, height))
}

fn psd_mode_name(mode: u16) -> &'static str {
    match mode {
        0 => "Bitmap",
        1 => "灰度",
        2 => "Indexed",
        3 => "RGB",
        4 => "CMYK",
        7 => "Multichannel",
        8 => "Duotone",
        9 => "Lab",
        _ => "未知模式",
    }
}

/// PSD preview-only path: parse with the `psd` crate and return the
/// flattened composite (channel data of the merged image, RLE or raw).
/// Layer editing and saving back to PSD are out of scope.
///
/// Public so the Explorer thumbnail shell extension (`crates/thumbnail`)
/// reuses the exact same guarded pipeline.
///
/// The `psd` crate signals malformed input by panicking (slice index out of
/// range) instead of returning errors, and the release profile builds with
/// `panic = "abort"` — so a corrupt file would take the whole app down.
/// `validate_psd_sections` bounds every length/offset the composite path
/// touches up front and turns those cases into a clear `DecodeError`.
pub fn decode_psd_bytes(bytes: &[u8], options: DecodeOptions) -> Result<Decoded, DecodeError> {
    let (width, height, depth, mode) =
        psd_header(bytes).ok_or(DecodeError::UnsupportedFormat)?;
    let channels = u16::from_be_bytes([bytes[12], bytes[13]]);
    if width == 0 || height == 0 {
        return Err(DecodeError::Psd("画布尺寸为 0".into()));
    }
    if depth != 8 {
        return Err(DecodeError::Psd(format!(
            "仅支持 8 位/通道（此文件为 {depth} 位），请先在 Photoshop 中转换为 8 位"
        )));
    }
    if !matches!(mode, 1 | 3) {
        return Err(DecodeError::Psd(format!(
            "仅支持 RGB / 灰度模式（此文件为 {}），请先在 Photoshop 中转换",
            psd_mode_name(mode)
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
    validate_psd_sections(bytes, width, height, channels)?;
    // The vendored psd fork fixes the known panics, but the crate is riddled
    // with unchecked slices — treat any residual panic as a decode error.
    // (Release builds with panic="unwind" for exactly this.)
    let psd = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        psd::Psd::from_bytes(bytes)
    }))
    .ok()
    .and_then(|r| r.ok())
    .ok_or_else(|| DecodeError::Psd("无法解析此 PSD（文件数据异常或使用了不支持的压缩）".into()))?;
    let rgba = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| psd.rgba()))
        .ok()
        .ok_or_else(|| DecodeError::Psd("无法解析此 PSD（图像数据异常）".into()))?;
    let image = RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| DecodeError::Psd("合成图数据与画布尺寸不符".into()))?;
    let (image, downsampled_from) = apply_max_dimension(image, options.max_dimension);
    Ok(Decoded {
        info: ImageInfo {
            format: SourceFormat::Psd,
            width: image.width(),
            height: image.height(),
        },
        image,
        frame_count: 1,
        downsampled_from,
        note: Some("PSD 预览：图层已合并，请另存为 PNG 等格式导出".into()),
    })
}

/// Bound every offset the `psd` crate will index into. Header layout
/// (big-endian): 26-byte header, then color-mode / image-resources /
/// layer-and-mask sections (each `u32 length + data`), then the image data
/// section (`u16 compression` + channel data).
fn validate_psd_sections(
    bytes: &[u8],
    width: u32,
    height: u32,
    channels: u16,
) -> Result<(), DecodeError> {
    if channels == 0 || channels > 56 {
        return Err(DecodeError::Psd(format!("通道数异常：{channels}")));
    }
    let u32be = |o: usize| {
        if o + 4 > bytes.len() {
            return Err(DecodeError::Psd("文件头部区段被截断".into()));
        }
        Ok(u32::from_be_bytes([
            bytes[o],
            bytes[o + 1],
            bytes[o + 2],
            bytes[o + 3],
        ]))
    };
    let mut off = 26usize;
    for name in ["色彩模式", "图像资源", "图层与蒙版"] {
        let len = u32be(off)? as usize;
        off += 4;
        let end = off.saturating_add(len);
        if end > bytes.len() {
            return Err(DecodeError::Psd(format!(
                "{name}区段长度 {len} 超出文件边界，文件已损坏"
            )));
        }
        off = end;
    }
    if off + 2 > bytes.len() {
        return Err(DecodeError::Psd("图像数据区段被截断".into()));
    }
    let compression = u16::from_be_bytes([bytes[off], bytes[off + 1]]);
    let data = off + 2;
    let pixels = width as usize * height as usize;
    let chans = channels as usize;
    match compression {
        0 => {
            // Raw planar: exactly channels * pixels bytes are read.
            if bytes.len().saturating_sub(data) < chans * pixels {
                return Err(DecodeError::Psd("原始通道数据不足，文件已损坏".into()));
            }
        }
        1 => {
            // RLE: `channels * height` big-endian u16 row counts first, then
            // the packed rows. The crate sums each channel's counts and slices
            // with the result — reject anything that would index out of range.
            let table_len = chans * height as usize * 2;
            if bytes.len().saturating_sub(data) < table_len {
                return Err(DecodeError::Psd("RLE 行长度表被截断，文件已损坏".into()));
            }
            let mut total = table_len;
            for c in 0..chans * height as usize {
                let o = data + c * 2;
                let row = u16::from_be_bytes([bytes[o], bytes[o + 1]]) as usize;
                total = total.saturating_add(row);
                if total > bytes.len() - data {
                    return Err(DecodeError::Psd("RLE 行长度超出数据边界，文件已损坏".into()));
                }
            }
        }
        other => {
            return Err(DecodeError::Psd(format!(
                "压缩方式 {other} 不受支持（仅 RAW / RLE），请另存为兼容性 PSD"
            )));
        }
    }
    Ok(())
}

fn count_gif_frames(bytes: &[u8]) -> Option<usize> {
    let decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).ok()?;
    Some(decoder.into_frames().collect_frames().ok()?.len())
}

pub fn encode(
    image: &RgbaImage,
    format: Format,
    quality: u8,
) -> Result<Vec<u8>, DecodeError> {
    let mut out = Cursor::new(Vec::new());
    match format {
        Format::Jpeg => {
            let rgb = image::DynamicImage::ImageRgba8(image.clone()).to_rgb8();
            let mut encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
            encoder.encode_image(&rgb)?;
        }
        // Lossless: `quality` maps to compression effort. >= 50 keeps the
        // default compression (unchanged historical behavior); < 50 spends
        // more time on `Best` compression to produce the smallest file.
        Format::Png => {
            use image::codecs::png::{CompressionType, FilterType, PngEncoder};
            use image::{ExtendedColorType, ImageEncoder};
            let compression = if quality >= 50 {
                CompressionType::Default
            } else {
                CompressionType::Best
            };
            let encoder = PngEncoder::new_with_quality(&mut out, compression, FilterType::Adaptive);
            encoder.write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                ExtendedColorType::Rgba8,
            )?;
        }
        // image 0.25's bundled WebP encoder is lossless-only, so `quality`
        // has no effect here (same for TIFF/BMP/GIF/TGA/QOI/PNM/ICO).
        Format::WebP => image.write_to(&mut out, Format::WebP)?,
        Format::Tiff => image.write_to(&mut out, Format::Tiff)?,
        Format::Bmp => image.write_to(&mut out, Format::Bmp)?,
        Format::Gif => image.write_to(&mut out, Format::Gif)?,
        Format::Tga => image.write_to(&mut out, Format::Tga)?,
        Format::Qoi => image.write_to(&mut out, Format::Qoi)?,
        Format::Pnm => image.write_to(&mut out, Format::Pnm)?,
        Format::Ico => image.write_to(&mut out, Format::Ico)?,
        // Read-only in image 0.25: DDS has no encoder (BCn compression is a
        // GPU-domain job), HDR only accepts f32 input we don't keep around.
        Format::Dds | Format::Hdr => {
            return Err(DecodeError::ReadOnlyFormat(format!("{format:?}")))
        }
        _ => return Err(DecodeError::UnsupportedFormat),
    }
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([
                (x % 256) as u8,
                (y % 256) as u8,
                ((x + y) % 256) as u8,
                255,
            ])
        })
    }

    fn roundtrip(format: Format) -> ImageInfo {
        let src = gradient(97, 61);
        let bytes = encode(&src, format, 90).expect("encode");
        let info = probe_bytes(&bytes).expect("probe");
        assert_eq!(info.format, format, "detected format mismatch");
        let decoded = decode_bytes(&bytes, DecodeOptions::default()).expect("decode");
        assert_eq!(decoded.info, info);
        assert_eq!(decoded.image.dimensions(), (97, 61));
        info
    }

    #[test]
    fn decodes_png() {
        assert_eq!(roundtrip(Format::Png).width, 97);
    }

    #[test]
    fn decodes_jpeg() {
        assert_eq!(roundtrip(Format::Jpeg).height, 61);
    }

    #[test]
    fn decodes_bmp() {
        roundtrip(Format::Bmp);
    }

    #[test]
    fn decodes_tiff() {
        roundtrip(Format::Tiff);
    }

    #[test]
    fn decodes_webp() {
        roundtrip(Format::WebP);
    }

    #[test]
    fn decodes_gif() {
        roundtrip(Format::Gif);
    }

    #[test]
    fn png_low_quality_is_smaller_and_still_lossless() {
        let src = gradient(97, 61);
        let hi = encode(&src, Format::Png, 90).unwrap();
        let lo = encode(&src, Format::Png, 30).unwrap();
        // Best-compression tier must never grow the file.
        assert!(lo.len() <= hi.len());
        // Pixels are untouched either way (PNG stays lossless).
        let decoded = decode_bytes(&lo, DecodeOptions::default()).unwrap();
        assert_eq!(decoded.image.as_raw(), src.as_raw());
    }

    #[test]
    fn rejects_oversized_images() {
        let src = gradient(64, 64);
        let bytes = encode(&src, Format::Png, 90).unwrap();
        let result = decode_bytes(
            &bytes,
            DecodeOptions {
                max_pixels: Some(100),
                max_dimension: None,
            },
        );
        match result {
            Err(DecodeError::TooLarge { pixels, limit, .. }) => {
                assert_eq!(pixels, 4096);
                assert_eq!(limit, 100);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_bytes() {
        assert!(matches!(
            decode_bytes(b"not an image", DecodeOptions::default()),
            Err(DecodeError::UnsupportedFormat)
        ));
    }

    #[test]
    fn roundtrips_interchange_formats() {
        // QOI / PNM / ICO have magic bytes: full decode_bytes pipeline
        // (magic detect + decode). TGA has NO magic in the format itself,
        // so guess_format can't see it — decode it with the explicit
        // format the extension-based open path uses.
        for format in [Format::Qoi, Format::Pnm, Format::Ico] {
            let src = gradient(33, 21);
            let bytes = encode(&src, format, 90).unwrap();
            let decoded = decode_bytes(&bytes, DecodeOptions::default()).unwrap();
            assert_eq!(decoded.info.format, format, "{format:?}");
            assert_eq!(decoded.image, src, "{format:?}");
        }
        let src = gradient(33, 21);
        let bytes = encode(&src, Format::Tga, 90).unwrap();
        let decoded = ImageReader::with_format(Cursor::new(bytes), Format::Tga)
            .decode()
            .unwrap()
            .to_rgba8();
        assert_eq!(decoded, src);
    }

    #[test]
    fn hdr_decodes_to_rgba8() {
        // Generate a real Radiance file via the f32 DynamicImage path, then
        // read it back through our decode pipeline (u8 conversion included).
        let float = image::DynamicImage::ImageRgb32F(
            image::Rgb32FImage::from_fn(8, 6, |x, _| {
                image::Rgb([(x as f32 / 8.0) * 3.0, 0.25, 0.5]) // >1.0 = HDR
            }),
        );
        let mut bytes = Vec::new();
        float
            .write_to(&mut std::io::Cursor::new(&mut bytes), Format::Hdr)
            .unwrap();
        let decoded = decode_bytes(&bytes, DecodeOptions::default()).unwrap();
        assert_eq!(decoded.info.format, Format::Hdr);
        assert_eq!((decoded.info.width, decoded.info.height), (8, 6));
        // (0,0): value 0.0 clamps to 0. (7,0): 3.0*7/8 clamps to 255.
        assert_eq!(decoded.image.get_pixel(0, 0).0[0], 0);
        assert_eq!(decoded.image.get_pixel(7, 0).0[0], 255);
    }

    #[test]
    fn dds_dxt5_decodes() {
        // Hand-assembled minimal DXT5 DDS (4x4, single block):
        // alpha block = a0 255 / a1 0 with zero indices -> every pixel a0;
        // color block = c0 == pure red (0xF800), zero indices -> every pixel
        // c0. Exercises the FourCC path that engine textures actually use.
        let mut dds = Vec::new();
        dds.extend_from_slice(b"DDS ");
        dds.extend_from_slice(&124u32.to_le_bytes()); // header size
        dds.extend_from_slice(&0x1007u32.to_le_bytes()); // CAPS|HEIGHT|WIDTH|PIXELFORMAT
        dds.extend_from_slice(&4u32.to_le_bytes()); // height
        dds.extend_from_slice(&4u32.to_le_bytes()); // width
        dds.extend_from_slice(&16u32.to_le_bytes()); // pitch (bytes per row)
        dds.extend_from_slice(&0u32.to_le_bytes()); // depth
        dds.extend_from_slice(&0u32.to_le_bytes()); // mipmaps
        dds.extend_from_slice(&[0u8; 44]); // reserved
        dds.extend_from_slice(&32u32.to_le_bytes()); // pixel format size
        dds.extend_from_slice(&0x4u32.to_le_bytes()); // DDPF_FOURCC
        dds.extend_from_slice(b"DXT5"); // fourcc
        dds.extend_from_slice(&[0u8; 20]); // bitcounts/masks unused for FourCC
        dds.extend_from_slice(&0x1000u32.to_le_bytes()); // caps texture
        dds.extend_from_slice(&0u32.to_le_bytes()); // caps2
        dds.extend_from_slice(&0u32.to_le_bytes()); // caps3
        dds.extend_from_slice(&0u32.to_le_bytes()); // caps4
        dds.extend_from_slice(&0u32.to_le_bytes()); // reserved2
        assert_eq!(dds.len(), 128);
        // One DXT5 block: 8B alpha + 8B color, all index bits zero.
        let mut block = vec![255u8, 0]; // a0, a1
        block.extend_from_slice(&[0u8; 6]); // alpha indices -> a0
        block.extend_from_slice(&0xF800u16.to_le_bytes()); // c0 red
        block.extend_from_slice(&0xF800u16.to_le_bytes()); // c1
        block.extend_from_slice(&[0u8; 4]); // color indices -> c0
        dds.extend_from_slice(&block);
        assert_eq!(dds.len(), 128 + 16);

        let decoded = decode_bytes(&dds, DecodeOptions::default()).unwrap();
        assert_eq!(decoded.info.format, Format::Dds);
        assert_eq!((decoded.info.width, decoded.info.height), (4, 4));
        for p in decoded.image.pixels() {
            assert_eq!(p.0, [255, 0, 0, 255]);
        }
    }

    #[test]
    fn dds_and_hdr_save_are_rejected_clearly() {
        let src = gradient(8, 6);
        assert!(matches!(
            encode(&src, Format::Dds, 90),
            Err(DecodeError::ReadOnlyFormat(_))
        ));
        assert!(matches!(
            encode(&src, Format::Hdr, 90),
            Err(DecodeError::ReadOnlyFormat(_))
        ));
    }

    #[test]
    fn downsamples_when_longest_edge_exceeds_limit() {
        let src = gradient(200, 100);
        let bytes = encode(&src, Format::Png, 90).unwrap();
        let decoded = decode_bytes(
            &bytes,
            DecodeOptions {
                max_pixels: None,
                max_dimension: Some(100),
            },
        )
        .unwrap();
        assert_eq!(decoded.image.dimensions(), (100, 50));
        assert_eq!(decoded.info.width, 100);
        assert_eq!(decoded.downsampled_from, Some((200, 100)));
    }

    #[test]
    fn keeps_original_when_within_limit() {
        let src = gradient(97, 61);
        let bytes = encode(&src, Format::Png, 90).unwrap();
        let decoded = decode_bytes(
            &bytes,
            DecodeOptions {
                max_pixels: None,
                max_dimension: Some(8192),
            },
        )
        .unwrap();
        assert_eq!(decoded.image.as_raw(), src.as_raw());
        assert_eq!(decoded.downsampled_from, None);
    }

    /// Minimal valid 8-bit RGB PSD: header + empty color-mode/resource/
    /// layer sections + raw planar image data (no compression). This is
    /// exactly what "Maximize Compatibility" stores as the merged composite.
    fn minimal_rgb_psd(width: u32, height: u32, pixels: &[[u8; 3]]) -> Vec<u8> {
        let mut psd = Vec::new();
        psd.extend_from_slice(b"8BPS");
        psd.extend_from_slice(&1u16.to_be_bytes()); // version (PSD)
        psd.extend_from_slice(&[0u8; 6]); // reserved
        psd.extend_from_slice(&3u16.to_be_bytes()); // channels: R,G,B
        psd.extend_from_slice(&height.to_be_bytes());
        psd.extend_from_slice(&width.to_be_bytes());
        psd.extend_from_slice(&8u16.to_be_bytes()); // depth
        psd.extend_from_slice(&3u16.to_be_bytes()); // mode: RGB
        psd.extend_from_slice(&0u32.to_be_bytes()); // color mode data: empty
        psd.extend_from_slice(&0u32.to_be_bytes()); // image resources: empty
        psd.extend_from_slice(&0u32.to_be_bytes()); // layer & mask: empty
        psd.extend_from_slice(&0u16.to_be_bytes()); // compression: raw
        for plane in 0..3 {
            for px in pixels {
                psd.push(px[plane]);
            }
        }
        psd
    }

    #[test]
    fn psd_preview_decodes_composite() {
        let (w, h) = (8u32, 6u32);
        let mut pixels = vec![[0u8; 3]; (w * h) as usize];
        for (i, px) in pixels.iter_mut().enumerate() {
            px[0] = (i % 256) as u8;
            px[1] = ((i * 7) % 256) as u8;
            px[2] = ((i * 13) % 256) as u8;
        }
        let bytes = minimal_rgb_psd(w, h, &pixels);
        let info = probe_bytes(&bytes).unwrap();
        assert_eq!(info.format, SourceFormat::Psd);
        assert_eq!((info.width, info.height), (w, h));

        let decoded = decode_bytes(&bytes, DecodeOptions::default()).unwrap();
        assert_eq!(decoded.info.format, SourceFormat::Psd);
        assert_eq!(decoded.image.dimensions(), (w, h));
        assert_eq!(decoded.frame_count, 1);
        assert!(decoded.note.is_some(), "PSD preview must carry the note");
        for (i, px) in pixels.iter().enumerate() {
            let got = decoded.image.get_pixel((i as u32) % w, (i as u32) / w).0;
            assert_eq!(got, [px[0], px[1], px[2], 255], "pixel {i}");
        }
    }

    #[test]
    fn psd_rejects_16bit_and_cmyk_with_clear_errors() {
        let (w, h) = (4u32, 4u32);
        let mut pixels = vec![[0u8; 3]; (w * h) as usize];
        let mut depth16 = minimal_rgb_psd(w, h, &pixels);
        depth16[22] = 0;
        depth16[23] = 16; // depth field
        assert!(matches!(
            decode_bytes(&depth16, DecodeOptions::default()),
            Err(DecodeError::Psd(_))
        ));

        let mut cmyk = minimal_rgb_psd(w, h, &pixels);
        cmyk[24] = 0;
        cmyk[25] = 4; // color mode: CMYK
        match decode_bytes(&cmyk, DecodeOptions::default()) {
            Err(DecodeError::Psd(msg)) => assert!(msg.contains("CMYK"), "{msg}"),
            other => panic!("expected Psd error, got {other:?}"),
        }
    }

    #[test]
    fn psd_corrupt_rle_is_rejected_not_panicking() {
        // The psd crate panics (slice index) on malformed RLE row tables, and
        // release builds use panic=abort — validation must reject first.
        let (w, h) = (4u32, 4u32);
        let pixels = vec![[10u8, 20, 30]; (w * h) as usize];
        let mut bytes = minimal_rgb_psd(w, h, &pixels);
        let comp_off = 26 + 12; // header + 3 empty sections -> compression u16
        bytes[comp_off] = 0x00; // compression = 1 (RLE)...
        bytes[comp_off + 1] = 0x01;
        for b in &mut bytes[comp_off + 2..comp_off + 2 + 24] {
            *b = 0xFF; // row counts of 65535 -> sums far past the file end
        }
        match decode_bytes(&bytes, DecodeOptions::default()) {
            Err(DecodeError::Psd(msg)) => assert!(msg.contains("RLE"), "{msg}"),
            other => panic!("expected Psd error, got {other:?}"),
        }
    }

    #[test]
    fn psd_downsample_path_applies_max_dimension() {
        let (w, h) = (200u32, 100u32);
        let bytes = minimal_rgb_psd(w, h, &vec![[30, 60, 90]; (w * h) as usize]);
        let decoded = decode_bytes(
            &bytes,
            DecodeOptions {
                max_pixels: None,
                max_dimension: Some(100),
            },
        )
        .unwrap();
        assert_eq!(decoded.image.dimensions(), (100, 50));
        assert_eq!(decoded.downsampled_from, Some((200, 100)));
    }
}
