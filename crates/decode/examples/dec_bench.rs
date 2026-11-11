//! Decode timing probe: `cargo run --example dec_bench -- <file> [max_dim] [--phases] [--json]`
//!
//! Prints the raw decode time and the decoded size so the open path can be
//! profiled independently of Qt / preview compositing. `--phases` re-runs the
//! same pipeline step by step so the codec can be told apart from our own RGBA
//! expansion / resampling.
//!
//! `--json` emits one machine-readable line for the PSD performance baseline
//! (M0 of the layered-editing plan). PSD has no `image::ImageFormat` decoder,
//! so `--phases` is skipped for it instead of panicking on `format_from_path`.

use std::io::Cursor;
use std::path::Path;
use std::time::Instant;

fn main() {
    let mut raw: Vec<String> = std::env::args().skip(1).collect();
    let phases = raw.iter().any(|a| a == "--phases");
    let json = raw.iter().any(|a| a == "--json");
    let psd_layers = raw.iter().any(|a| a == "--psd-layers");
    raw.retain(|a| !a.starts_with("--"));
    let path = raw.first().cloned().expect("usage: dec_bench <file> [max_dim] [--phases] [--json]");
    let max_dim = raw.get(1).and_then(|v| v.parse::<u32>().ok());
    let opts = decode::DecodeOptions {
        max_pixels: None,
        max_dimension: max_dim,
    };
    let file_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let t0 = Instant::now();
    match decode::decode_path(Path::new(&path), opts) {
        Ok(decoded) => {
            let ms = t0.elapsed().as_millis();
            if json {
                println!(
                    "{{\"event\":\"dec-bench\",\"phase\":\"psd-stage1-ms\",\"stage1_ms\":{},\
\"width\":{},\"height\":{},\"file_bytes\":{},\"downsampled\":{},\"path\":{}}}",
                    ms,
                    decoded.image.width(),
                    decoded.image.height(),
                    file_bytes,
                    decoded.downsampled_from.is_some(),
                    json_quote(&path)
                );
            } else {
                println!(
                    "{} -> {}x{} (from {:?}) in {} ms",
                    path,
                    decoded.image.width(),
                    decoded.image.height(),
                    decoded.downsampled_from,
                    ms
                );
            }
        }
        Err(err) => {
            eprintln!("decode failed: {err}");
            std::process::exit(1);
        }
    }

    if psd_layers {
        let t_read = Instant::now();
        let bytes = std::fs::read(&path).expect("read");
        let read_ms = t_read.elapsed().as_millis();
        let t_parse = Instant::now();
        let mut ticks = 0usize;
        match decode::decode_psd_layers_with_progress(&bytes, opts, |_, _| ticks += 1) {
            Ok(set) => {
                let parse_ms = t_parse.elapsed().as_millis();
                if json {
                    println!(
                        "{{\"event\":\"dec-bench\",\"phase\":\"psd-stage2-ms\",\"stage2_ms\":{},\
\"read_ms\":{},\"layer_count\":{},\"layer_bytes\":{},\"degraded\":{},\"progress_ticks\":{},\
\"path\":{}}}",
                        parse_ms,
                        read_ms,
                        set.layer_count(),
                        set.bytes,
                        set.degraded.len(),
                        ticks,
                        json_quote(&path)
                    );
                } else {
                    println!(
                        "layers: {} in {} ms (read {} ms, {} bytes of layer bitmaps, {} degradation notes)",
                        set.layer_count(),
                        parse_ms,
                        read_ms,
                        set.bytes,
                        set.degraded.len()
                    );
                }
            }
            Err(err) => {
                eprintln!("layer parse failed: {err}");
                std::process::exit(1);
            }
        }
    }

    if phases {
        let is_psd = path.rsplit('.').next().map(|e| e.eq_ignore_ascii_case("psd")).unwrap_or(false);
        let format = decode::format_from_path(Path::new(&path));
        if is_psd || format.is_none() {
            println!("phases: skipped (PSD / unknown format has no image-codec path)");
            return;
        }
        let format = format.unwrap();
        let t0 = Instant::now();
        let bytes = std::fs::read(&path).expect("read");
        let read_ms = t0.elapsed().as_millis();

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

fn json_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
