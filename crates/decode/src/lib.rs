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
    pub format: Format,
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
}

pub fn detect_format(bytes: &[u8]) -> Option<Format> {
    image::guess_format(bytes).ok()
}

pub fn format_from_path(path: &Path) -> Option<Format> {
    ImageFormat::from_path(path).ok()
}

pub fn probe_bytes(bytes: &[u8]) -> Result<ImageInfo, DecodeError> {
    let format = detect_format(bytes).ok_or(DecodeError::UnsupportedFormat)?;
    let reader = ImageReader::with_format(Cursor::new(bytes), format);
    let (width, height) = reader.into_dimensions()?;
    Ok(ImageInfo {
        format,
        width,
        height,
    })
}

pub fn probe_path(path: &Path) -> Result<ImageInfo, DecodeError> {
    let format = format_from_path(path).ok_or(DecodeError::UnsupportedFormat)?;
    let reader = ImageReader::open(path)?;
    let (width, height) = reader.into_dimensions()?;
    Ok(ImageInfo {
        format,
        width,
        height,
    })
}

pub fn decode_bytes(bytes: &[u8], options: DecodeOptions) -> Result<Decoded, DecodeError> {
    let format = detect_format(bytes).ok_or(DecodeError::UnsupportedFormat)?;
    decode_with(bytes, format, options)
}

pub fn decode_path(path: &Path, options: DecodeOptions) -> Result<Decoded, DecodeError> {
    let format = format_from_path(path)
        .or_else(|| {
            std::fs::read(path)
                .ok()
                .and_then(|bytes| detect_format(&bytes))
        })
        .ok_or(DecodeError::UnsupportedFormat)?;
    let bytes = std::fs::read(path)?;
    decode_with(&bytes, format, options)
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
    let (image, downsampled_from) = match options.max_dimension {
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
    };
    let frame_count = if format == Format::Gif {
        count_gif_frames(bytes).unwrap_or(1)
    } else {
        1
    };
    Ok(Decoded {
        info: ImageInfo {
            format,
            width: image.width(),
            height: image.height(),
        },
        image,
        frame_count,
        downsampled_from,
    })
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
}
