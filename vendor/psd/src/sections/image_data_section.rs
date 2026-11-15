use crate::psd_channel::{rle_decompress, PsdChannelCompression};
use crate::sections::PsdCursor;
use crate::PsdDepth;
use thiserror::Error;

/// Represents an malformed image data
#[derive(Debug, PartialEq, Error)]
pub enum ImageDataSectionError {
    #[error(
        r#"Only 8, 16 and 32 bit depths are supported at the moment.
    If you'd like to see 1 bit depths supported - please open an issue."#
    )]
    UnsupportedDepth,

    #[error("{compression} is an invalid layer channel compression. Must be 0, 1, 2 or 3")]
    InvalidCompression { compression: u16 },

    #[error("ZIP compressed image data is corrupt or truncated")]
    InvalidZipData,
}

/// The ImageDataSection comes from the final section in the PSD that contains the pixel data
/// of the final PSD image (the one that comes from combining all of the layers).
///
/// # [Adobe Docs](https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/)
///
/// The last section of a Photoshop file contains the image pixel data.
/// Image data is stored in planar order: first all the red data, then all the green data, etc.
/// Each plane is stored in scan-line order, with no pad bytes,
///
/// | Length   | Description                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
/// |----------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
/// | 2        | Compression method: <br> 0 = Raw image data <br> 1 = RLE compressed the image data starts with the byte counts for all the scan lines (rows * channels), with each count stored as a two-byte value. The RLE compressed data follows, with each scan line compressed separately. The RLE compression is the same compression algorithm used by the Macintosh ROM routine PackBits , and the TIFF standard. <br> 2 = ZIP without prediction <br> 3 = ZIP with prediction. |
/// | Variable | The image data. Planar order = RRR GGG BBB, etc.                                                                                                                                                                                                                                                                                                                                                                                                                         |
#[derive(Debug)]
pub struct ImageDataSection {
    /// The compression method for the image.
    pub(crate) compression: PsdChannelCompression,
    /// The red channel of the final image
    pub(crate) red: ChannelBytes,
    /// The green channel of the final image
    pub(crate) green: Option<ChannelBytes>,
    /// the blue channel of the final image
    pub(crate) blue: Option<ChannelBytes>,
    /// the alpha channel of the final image.
    /// If there is no alpha channel then it is a fully opaque image.
    pub(crate) alpha: Option<ChannelBytes>,
}

impl ImageDataSection {
    /// Create an ImageDataSection from the bytes in the corresponding section in a PSD file
    /// (including the length market)
    pub fn from_bytes(
        bytes: &[u8],
        depth: PsdDepth,
        psd_width: u32,
        psd_height: u32,
        channel_count: u8,
    ) -> Result<ImageDataSection, ImageDataSectionError> {
        let mut cursor = PsdCursor::new(bytes);
        let channel_count = channel_count as usize;

        let compression = cursor.read_u16();
        let compression = PsdChannelCompression::new(compression)
            .ok_or(ImageDataSectionError::InvalidCompression { compression })?;

        // 1 位（Bitmap）通道是位打包的，调用方在解码前就应拒绝。
        if depth == PsdDepth::One {
            return Err(ImageDataSectionError::UnsupportedDepth);
        }

        let (red, green, blue, alpha) = match compression {
            PsdChannelCompression::RawData => {
                // First 2 bytes were compression bytes
                let channel_bytes = &bytes[2..];
                let channel_byte_count = channel_bytes.len();

                let bytes_per_channel = channel_byte_count / channel_count;

                // First bytes are red
                let mut red = channel_bytes[..bytes_per_channel].into();

                // Next bytes are green
                let green = if channel_count >= 2 {
                    Some(ChannelBytes::RawData(
                        channel_bytes[bytes_per_channel..2 * bytes_per_channel].into(),
                    ))
                } else {
                    None
                };

                // Then comes blue
                let blue = if channel_count >= 3 {
                    Some(ChannelBytes::RawData(
                        channel_bytes[2 * bytes_per_channel..3 * bytes_per_channel].into(),
                    ))
                } else {
                    None
                };

                // And optionally alpha bytes
                let alpha = if channel_count == 4 {
                    Some(ChannelBytes::RawData(
                        channel_bytes[3 * bytes_per_channel..4 * bytes_per_channel].to_vec(),
                    ))
                } else {
                    None
                };

                (ChannelBytes::RawData(red), green, blue, alpha)
            }
            // # [Adobe Docs](https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/)
            //
            // RLE compressed the image data starts with the byte counts for all the scan lines
            // (rows * channels), with each count stored as a two-byte value. The RLE compressed
            // data follows, with each scan line compressed separately. The RLE compression is
            // the same compression algorithm used by the Macintosh ROM routine PackBits,
            // and the TIFF standard.
            PsdChannelCompression::RleCompressed => {
                let mut red_byte_count = 0;
                let mut green_byte_count = if channel_count >= 2 { Some(0) } else { None };
                let mut blue_byte_count = if channel_count >= 3 { Some(0) } else { None };
                let mut alpha_byte_count = if channel_count == 4 { Some(0) } else { None };

                for _ in 0..psd_height {
                    red_byte_count += cursor.read_u16() as usize;
                }

                if let Some(ref mut green_byte_count) = green_byte_count {
                    for _ in 0..psd_height {
                        *green_byte_count += cursor.read_u16() as usize;
                    }
                }

                if let Some(ref mut blue_byte_count) = blue_byte_count {
                    for _ in 0..psd_height {
                        *blue_byte_count += cursor.read_u16() as usize;
                    }
                }

                if let Some(ref mut alpha_byte_count) = alpha_byte_count {
                    for _ in 0..psd_height {
                        *alpha_byte_count += cursor.read_u16() as usize;
                    }
                }

                // 2 bytes for compression level, then 2 bytes for each scanline of each channel
                // We're skipping over the bytes that describe the length of each scanling since
                // we don't currently use them. We might re-think this in the future when we
                // implement serialization of a Psd back into bytes.. But not a concern at the
                // moment.
                let channel_data_start = 2 + (channel_count * psd_height as usize * 2);

                let (red_start, red_end) =
                    (channel_data_start, channel_data_start + red_byte_count);

                let red = bytes[red_start..red_end].into();

                let green = match green_byte_count {
                    Some(green_byte_count) => {
                        let green_start = red_end;
                        let green_end = green_start + green_byte_count;
                        Some(ChannelBytes::RleCompressed(
                            bytes[green_start..green_end].into(),
                        ))
                    }
                    None => None,
                };

                let blue = match blue_byte_count {
                    Some(blue_byte_count) => {
                        let blue_start = red_end + green_byte_count.unwrap();
                        let blue_end = blue_start + blue_byte_count;
                        Some(ChannelBytes::RleCompressed(
                            bytes[blue_start..blue_end].into(),
                        ))
                    }
                    None => None,
                };

                let alpha = match alpha_byte_count {
                    Some(alpha_byte_count) => {
                        let alpha_start =
                            red_end + green_byte_count.unwrap() + blue_byte_count.unwrap();
                        let alpha_end = alpha_start + alpha_byte_count;
                        Some(ChannelBytes::RleCompressed(
                            bytes[alpha_start..alpha_end].into(),
                        ))
                    }
                    None => None,
                };

                (ChannelBytes::RleCompressed(red), green, blue, alpha)
            }
            PsdChannelCompression::ZipWithoutPrediction
            | PsdChannelCompression::ZipWithPrediction => {
                let bytes_per_sample = match depth {
                    PsdDepth::Sixteen => 2usize,
                    PsdDepth::ThirtyTwo => 4usize,
                    _ => 1usize,
                };
                let row_len = psd_width as usize * bytes_per_sample;
                let expected = row_len * psd_height as usize;
                // ZIP 数据里每个通道是一条独立的 zlib 流，按消耗字节数依次推进。
                let mut rest = &bytes[2..];
                let mut planes = Vec::with_capacity(channel_count.min(4));
                for _ in 0..channel_count.min(4) {
                    let (mut plane, used) =
                        inflate_zlib(rest, expected).ok_or(ImageDataSectionError::InvalidZipData)?;
                    if compression == PsdChannelCompression::ZipWithPrediction {
                        undo_prediction(&mut plane, row_len, bytes_per_sample);
                    }
                    planes.push(ChannelBytes::RawData(plane));
                    rest = &rest[used.min(rest.len())..];
                }
                let mut it = planes.into_iter();
                let red = it.next().ok_or(ImageDataSectionError::InvalidCompression {
                    compression: 2,
                })?;
                (red, it.next(), it.next(), it.next())
            }
        };

        // 16/32 位统一降到 8 位（RLE 先解压），下游只处理 8 位通道。
        let red = to_eight_bit(red, depth);
        let green = green.map(|c| to_eight_bit(c, depth));
        let blue = blue.map(|c| to_eight_bit(c, depth));
        let alpha = alpha.map(|c| to_eight_bit(c, depth));

        Ok(ImageDataSection {
            compression,
            red,
            green,
            blue,
            alpha,
        })
    }
}

/// 把 16/32 位平面通道降成 8 位；8 位原样返回。
///
/// 16 位用 `(v + 128) / 257`，Photoshop 的 8→16 位扩展是 `v * 257`，这样能
/// 无损往返；32 位是 0..1 的浮点（HDR 可超出，按 0..1 夹紧后四舍五入）。
/// RLE 先解压成裸字节再转换，得到的通道统一是 `RawData`。
pub(crate) fn to_eight_bit(bytes: ChannelBytes, depth: PsdDepth) -> ChannelBytes {
    let raw = match depth {
        PsdDepth::Eight | PsdDepth::One => return bytes,
        PsdDepth::Sixteen | PsdDepth::ThirtyTwo => match bytes {
            ChannelBytes::RawData(v) => v,
            ChannelBytes::RleCompressed(v) => rle_decompress(&v),
        },
    };
    let out = match depth {
        PsdDepth::Sixteen => raw
            .chunks_exact(2)
            .map(|p| ((u16::from_be_bytes([p[0], p[1]]) as u32 + 128) / 257) as u8)
            .collect(),
        PsdDepth::ThirtyTwo => raw
            .chunks_exact(4)
            .map(|p| {
                let v = f32::from_be_bytes([p[0], p[1], p[2], p[3]]);
                (v.clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect(),
        PsdDepth::Eight | PsdDepth::One => unreachable!(),
    };
    ChannelBytes::RawData(out)
}

/// 解开一条 zlib 流，返回 (数据, 消耗的输入字节数)。
///
/// PSD 的 ZIP 通道各是一条独立的 zlib 流，必须用 `total_in` 找下一条的起点
/// （`read` 解码器会预读，跨过流尾）。
pub(crate) fn inflate_zlib(data: &[u8], expected: usize) -> Option<(Vec<u8>, usize)> {
    use std::io::Read;
    let mut dec = flate2::read::ZlibDecoder::new(data);
    let mut out = Vec::with_capacity(expected.max(1));
    dec.read_to_end(&mut out).ok()?;
    // `total_in` 是解压器真正消费的压缩字节数；`read` 解码器内部有预读，
    // 不能拿底层游标的位置当下一条流的起点。
    Some((out, dec.total_in() as usize))
}

/// 撤销 PSD ZIP 预测：每行按样本字节数做前缀和（16/32 位）。
///
/// 8 位 ZIP 在 Photoshop 里不使用预测，保持原样（与 psd-tools 一致）。
pub(crate) fn undo_prediction(data: &mut [u8], row_len: usize, stride: usize) {
    if stride <= 1 || row_len == 0 {
        return;
    }
    for row in data.chunks_mut(row_len) {
        for i in stride..row.len() {
            row[i] = row[i].wrapping_add(row[i - stride]);
        }
    }
}

/// Bytes of a single planar channel, either raw or PackBits (with the
/// per-scanline length table already stripped by the layer reader).
#[derive(Debug, Clone)]
pub enum ChannelBytes {
    /// Uncompressed planar bytes, one byte per pixel of the rectangle.
    RawData(Vec<u8>),
    /// PackBits-compressed planar bytes (scanline length table removed).
    RleCompressed(Vec<u8>),
}
