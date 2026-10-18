use std::io::Cursor;
use std::path::Path;

use image::AnimationDecoder;
use image::{ImageFormat, ImageReader, RgbaImage};

pub use image::ImageFormat as Format;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("unsupported format")]
    UnsupportedFormat,
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
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            max_pixels: Some(300_000_000),
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
        // has no effect here (same for TIFF/BMP/GIF).
        Format::WebP => image.write_to(&mut out, Format::WebP)?,
        Format::Tiff => image.write_to(&mut out, Format::Tiff)?,
        Format::Bmp => image.write_to(&mut out, Format::Bmp)?,
        Format::Gif => image.write_to(&mut out, Format::Gif)?,
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
}
