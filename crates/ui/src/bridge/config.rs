// config 相关项。所有项标 pub(in crate::bridge)，mod.rs 再导出。
use super::*;

/// User preferences persisted across runs (plain key=value file under
/// %APPDATA%/ImageEdit so no extra dependency is needed).
pub(in crate::bridge) struct AppConfig {
    pub(in crate::bridge) checker_light: String,
    pub(in crate::bridge) checker_dark: String,
    pub(in crate::bridge) open_in_new_window: bool,
    /// Save quality (1-100). Lossy formats (JPEG/WebP) use it directly;
    /// lossless PNG maps it to the compression effort (< 50 = max
    /// compression / smallest file).
    pub(in crate::bridge) save_quality: u8,
    /// Ruler Y direction: `false` = origin top-left (screen order),
    /// `true` = origin bottom-left (Y grows upward).
    pub(in crate::bridge) ruler_bottom_up: bool,
    /// Downsampling-first open cap for the longest edge. `0` disables it and
    /// keeps the original resolution (files above 300 MP are still rejected).
    pub(in crate::bridge) max_dimension: i32,
}

impl AppConfig {
    pub(in crate::bridge) fn path() -> Option<PathBuf> {
        std::env::var_os("APPDATA")
            .map(|root| PathBuf::from(root).join("ImageEdit").join("settings.ini"))
    }

    pub(in crate::bridge) fn load() -> Self {
        let mut cfg = Self::default();
        if let Some(path) = Self::path() {
            if let Ok(text) = std::fs::read_to_string(path) {
                for line in text.lines() {
                    let (key, value) = match line.split_once('=') {
                        Some((k, v)) => (k.trim(), v.trim()),
                        None => continue,
                    };
                    match key {
                        "checker_light" if !value.is_empty() => {
                            cfg.checker_light = value.to_string()
                        }
                        "checker_dark" if !value.is_empty() => cfg.checker_dark = value.to_string(),
                        "open_in_new_window" => cfg.open_in_new_window = value == "true",
                        "ruler_bottom_up" => cfg.ruler_bottom_up = value == "true",
                        "save_quality" => {
                            if let Ok(q) = value.parse::<u8>() {
                                cfg.save_quality = q.clamp(1, 100);
                            }
                        }
                        "max_dimension" => {
                            if let Ok(d) = value.parse::<i32>() {
                                cfg.max_dimension = d.max(0);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        cfg
    }

    pub(in crate::bridge) fn save(&self) {
        if let Some(path) = Self::path() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let text = format!(
                "checker_light={}\nchecker_dark={}\nopen_in_new_window={}\nsave_quality={}\nruler_bottom_up={}\nmax_dimension={}\n",
                self.checker_light,
                self.checker_dark,
                self.open_in_new_window,
                self.save_quality,
                self.ruler_bottom_up,
                self.max_dimension
            );
            let _ = std::fs::write(path, text);
        }
    }
}

pub(in crate::bridge) fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
