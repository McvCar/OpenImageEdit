//! Decode timing probe: `cargo run --example dec_bench -- <file> [max_dim] [--phases]`
//!
//! Prints the raw decode time and the decoded size so the open path can be
//! profiled independently of Qt / preview compositing. `--phases` re-runs the
//! same pipeline step by step so the codec can be told apart from our own RGBA
//! expansion / resampling.

use std::io::Cursor;
use std::path::Path;
use std::time::Instant;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let phases = args.iter().any(|a| a == "--phases");
    args.retain(|a| a != "--phases");
    let path = args.first().cloned().expect("usage: dec_bench <file> [max_dim]");
    let max_dim = args.get(1).and_then(|v| v.parse::<u32>().ok());
    let opts = decode::DecodeOptions {
        max_pixels: None,
        max_dimension: max_dim,
    };
    let t0 = Instant::now();
    match decode::decode_path(Path::new(&path), opts) {
        Ok(decoded) => {
            let ms = t0.elapsed().as_millis();
            println!(
                "{} -> {}x{} (from {:?}) in {} ms",
                path,
                decoded.image.width(),
                decoded.image.height(),
                decoded.downsampled_from,
                ms
            );
        }
        Err(err) => {
            eprintln!("decode failed: {err}");
            std::process::exit(1);
        }
    }

    if phases {
        let t0 = Instant::now();
        let bytes = std::fs::read(&path).expect("read");
        let read_ms = t0.elapsed().as_millis();
        let format = decode::format_from_path(Path::new(&path)).expect("format");

        let t1 = Instant::now();
        let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
        reader.no_limits();
        let dims = reader.into_dimensions().expect("dims");
        let header_ms = t1.elapsed().as_millis();

        let t2 = Instant::now();
        let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
        reader.no_limits();
        let dynimg = reader.decode().expect("decode");
        let decode_ms = t2.elapsed().as_millis();

        let t3 = Instant::now();
        let rgba = dynimg.to_rgba8();
        let rgba_ms = t3.elapsed().as_millis();

        let t4 = Instant::now();
        let longest = rgba.width().max(rgba.height());
        let target = max_dim.unwrap_or(longest);
        let preview = if target < longest {
            let scale = target as f64 / longest as f64;
            let nw = ((rgba.width() as f64 * scale).round() as u32).max(1);
            let nh = ((rgba.height() as f64 * scale).round() as u32).max(1);
            Some(image::imageops::resize(
                &rgba,
                nw,
                nh,
                image::imageops::FilterType::Lanczos3,
            ))
        } else {
            None
        };
        let resize_ms = t4.elapsed().as_millis();

        println!(
            "phases: read {read_ms} ms | header {header_ms} ms | decode {decode_ms} ms ({}x{}) \
| to_rgba8 {rgba_ms} ms | lanczos3->{:?} {resize_ms} ms",
            dims.0,
            dims.1,
            preview.as_ref().map(|p| (p.width(), p.height()))
        );
    }
}
