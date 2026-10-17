use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;

use decode::{decode_path, encode, probe_path, DecodeOptions, Format};

const SAMPLES: &[(&str, u32, u32)] = &[
    ("small", 1600, 1200),
    ("medium", 4000, 3000),
    ("large", 8160, 6120),
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        return usage();
    };
    match command {
        "startup" => Ok(()),
        "gen" => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("bench-data"));
            generate(&dir)
        }
        "probe" => {
            let path = expect_path(&args, 1)?;
            let t0 = Instant::now();
            let info = probe_path(&path)?;
            let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
            println!(
                "{{\"phase\":\"probe\",\"ms\":{elapsed:.3},\"format\":\"{:?}\",\"width\":{},\"height\":{}}}",
                info.format, info.width, info.height
            );
            Ok(())
        }
        "decode" => {
            let path = expect_path(&args, 1)?;
            let t0 = Instant::now();
            let decoded = decode_path(&path, DecodeOptions::default())?;
            let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
            println!(
                "{{\"phase\":\"decode\",\"ms\":{elapsed:.3},\"width\":{},\"height\":{},\"frames\":{}}}",
                decoded.info.width, decoded.info.height, decoded.frame_count
            );
            Ok(())
        }
        "run" => {
            let path = expect_path(&args, 1)?;
            let repeat = args
                .iter()
                .position(|a| a == "--repeat")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(7);
            benchmark(&path, repeat)
        }
        "coldstart" => {
            let mode = args.get(1).cloned().unwrap_or_else(|| "startup".into());
            let file = args.get(2).cloned();
            let repeat = args
                .iter()
                .position(|a| a == "--repeat")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(15);
            coldstart(&mode, file.as_deref(), repeat)
        }
        "ui" => {
            let file = args.get(1).cloned();
            let repeat = args
                .iter()
                .position(|a| a == "--repeat")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(7);
            coldstart_ui(file.as_deref(), repeat)
        }
        other => {
            eprintln!("unknown command: {other}");
            usage()
        }
    }
}

fn expect_path(args: &[String], index: usize) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = args
        .get(index)
        .map(PathBuf::from)
        .ok_or_else(|| -> Box<dyn std::error::Error> { "missing <file> argument".into() })?;
    if !path.is_file() {
        return Err(format!("no such file: {}", path.display()).into());
    }
    Ok(path)
}

fn usage() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("usage:");
    eprintln!("  imageedit-bench gen <dir>");
    eprintln!("  imageedit-bench probe <file>");
    eprintln!("  imageedit-bench decode <file>");
    eprintln!("  imageedit-bench run <file> [--repeat N]");
    eprintln!("  imageedit-bench coldstart <startup|decode|probe> [file] [--repeat N]");
    eprintln!("  imageedit-bench ui [file] [--repeat N]");
    eprintln!("  imageedit-bench startup");
    Ok(())
}

fn coldstart(mode: &str, file: Option<&str>, repeat: usize) -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let mut first = 0.0;
    let mut times = Vec::with_capacity(repeat);
    for i in 0..repeat {
        let mut cmd = Command::new(&exe);
        cmd.arg(mode);
        if let Some(file) = file {
            cmd.arg(file);
        }
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        let t0 = Instant::now();
        let status = cmd.status()?;
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if !status.success() {
            return Err(format!("child exited with {status}").into());
        }
        if i == 0 {
            first = ms;
        }
        times.push(ms);
    }
    let mut sorted = times.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    println!(
        "{{\"phase\":\"coldstart\",\"mode\":\"{mode}\",\"file\":\"{}\",\"repeat\":{repeat},\"first_ms\":{first:.3},\"min_ms\":{:.3},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"max_ms\":{:.3}}}",
        file.unwrap_or("-"),
        p(0.0),
        p(0.5),
        p(0.95),
        p(1.0)
    );
    Ok(())
}

fn coldstart_ui(file: Option<&str>, repeat: usize) -> Result<(), Box<dyn std::error::Error>> {
    let ui_exe = std::env::current_exe()?
        .parent()
        .map(|dir| dir.join("imageedit.exe"))
        .ok_or("cannot locate imageedit.exe")?;
    if !ui_exe.is_file() {
        return Err(format!("missing {}", ui_exe.display()).into());
    }
    if let Some(file) = file {
        if !Path::new(file).is_file() {
            return Err(format!("no such file: {file}").into());
        }
    }

    let mut first_frame_times = Vec::with_capacity(repeat);
    let mut exit_times = Vec::with_capacity(repeat);
    let mut in_main = 0.0_f64;
    let mut decode_ms = 0.0_f64;

    for i in 0..repeat {
        let mut cmd = Command::new(&ui_exe);
        if let Some(file) = file {
            cmd.arg(file);
        }
        cmd.arg("--selftest");
        cmd.stdout(Stdio::piped()).stderr(Stdio::null());

        let t0 = Instant::now();
        let mut child = cmd.spawn()?;
        let stdout = child.stdout.take().ok_or("no stdout pipe")?;
        let reader = std::io::BufReader::new(stdout);
        let mut first_frame = None;
        let mut main_enter = None;
        for line in std::io::BufRead::lines(reader) {
            let Ok(line) = line else { break };
            if line.contains("\"image-frame\"") {
                first_frame = Some(t0.elapsed().as_secs_f64() * 1000.0);
            } else if line.contains("\"event\":\"main-enter\"") {
                main_enter = Some(t0.elapsed().as_secs_f64() * 1000.0);
            } else if line.contains("\"event\":\"image-shown\"") || line.contains("\"shown_ms\"") {
                if let Some(value) = json_int(&line, "decode_ms") {
                    decode_ms = value as f64;
                }
            }
        }
        let status = child.wait()?;
        if !status.success() {
            return Err(format!("child exited with {status}").into());
        }
        let exit_ms = t0.elapsed().as_secs_f64() * 1000.0;
        if i == 0 {
            in_main = main_enter.unwrap_or(f64::NAN);
        }
        first_frame_times.push(first_frame.ok_or("child never reported first-frame")?);
        exit_times.push(exit_ms);
    }

    let mut sorted = first_frame_times.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    let mut sorted_exit = exit_times.clone();
    sorted_exit.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pe = |q: f64| sorted_exit[((sorted_exit.len() - 1) as f64 * q).round() as usize];
    println!(
        "{{\"phase\":\"coldstart-ui\",\"file\":\"{}\",\"repeat\":{repeat},\"main_enter_ms\":{in_main:.3},\"decode_ms\":{decode_ms:.3},\"ff_p50_ms\":{:.3},\"ff_min_ms\":{:.3},\"ff_p95_ms\":{:.3},\"exit_p50_ms\":{:.3}}}",
        file.unwrap_or("-"),
        p(0.5),
        p(0.0),
        p(0.95),
        pe(0.5),
    );
    Ok(())
}

fn json_int(line: &str, key: &str) -> Option<i64> {
    let needle = format!("\"{key}\":");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

fn generate(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    for (name, width, height) in SAMPLES {
        let image = pattern(*width, *height);
        for (ext, format) in [
            ("jpg", Format::Jpeg),
            ("png", Format::Png),
            ("webp", Format::WebP),
            ("bmp", Format::Bmp),
            ("tiff", Format::Tiff),
        ] {
            let path = dir.join(format!("{name}-{width}x{height}.{ext}"));
            let t0 = Instant::now();
            let bytes = encode(&image, format, 90)?;
            std::fs::write(&path, &bytes)?;
            println!(
                "{{\"phase\":\"gen\",\"file\":\"{}\",\"bytes\":{},\"ms\":{:.3}}}",
                path.display(),
                bytes.len(),
                t0.elapsed().as_secs_f64() * 1000.0
            );
        }
        let gif_path = dir.join(format!("{name}-{width}x{height}.gif"));
        let gif_source = image::DynamicImage::ImageRgba8(pattern(*width / 4, *height / 4));
        gif_source.save_with_format(&gif_path, Format::Gif)?;
        println!(
            "{{\"phase\":\"gen\",\"file\":\"{}\",\"bytes\":{}}}",
            gif_path.display(),
            std::fs::metadata(&gif_path)?.len()
        );
    }
    Ok(())
}

fn pattern(width: u32, height: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(width, height, |x, y| {
        let base = x.wrapping_mul(7).wrapping_add(y.wrapping_mul(13));
        let noise = x.wrapping_mul(2654435761).wrapping_add(y.wrapping_mul(40503));
        image::Rgba([
            (base % 256) as u8,
            ((base / 3) % 256) as u8,
            (noise % 256) as u8,
            255,
        ])
    })
}

fn benchmark(path: &Path, repeat: usize) -> Result<(), Box<dyn std::error::Error>> {
    let info = probe_path(path)?;
    let mut decode_ms = Vec::with_capacity(repeat);
    for _ in 0..repeat {
        let t0 = Instant::now();
        let decoded = decode_path(path, DecodeOptions::default())?;
        decode_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(decoded);
    }
    decode_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "{{\"file\":\"{}\",\"format\":\"{:?}\",\"mp\":{:.2},\"decode_min_ms\":{:.3},\"decode_median_ms\":{:.3},\"decode_max_ms\":{:.3},\"repeat\":{}}}",
        path.display(),
        info.format,
        info.pixels() as f64 / 1_000_000.0,
        decode_ms[0],
        decode_ms[decode_ms.len() / 2],
        decode_ms[decode_ms.len() - 1],
        repeat
    );
    Ok(())
}
