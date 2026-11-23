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
    /// Last used selection parameters (border / smooth / feather). A newly
    /// drawn selection inherits them, so the user does not have to retype the
    /// same numbers on every fresh marquee.
    pub(in crate::bridge) sel_border: u32,
    pub(in crate::bridge) sel_smooth: u32,
    pub(in crate::bridge) sel_feather: u32,
    /// Last used selection tool: "rect" / "ellipse" / "lasso" / "polygon" /
    /// "wand".
    pub(in crate::bridge) sel_tool: String,
    /// 魔术棒参数，原样存 `容差,连续,取样全部图层`（如 "32,1,0"）。
    pub(in crate::bridge) sel_wand: String,
    /// 快速选择参数，原样存 `笔刷半径,容差,扩散,取样全部图层,自动增强`
    /// （如 "24,40,160,1,1"）。对齐 PS 选项栏的两项开关。
    pub(in crate::bridge) sel_quick: String,
    /// 修复 / 油漆工具的参数，`|` 分隔：
    /// `工具|笔刷|硬度|间距|强度|扩散|容差|连续|不透明|径向|反向|渐变不透明`
    /// （如 "none|40|70|20|100|2|32|1|100|0|0|100"）。
    pub(in crate::bridge) paint_spec: String,
    // ---- P0-4 智能参考线 / P0-5 数值面板（settings.ini 跨启动记忆）----
    /// D7/D13：数值面板首次默认隐藏，按钮切换后记住上次显示状态。
    pub(in crate::bridge) transform_panel_visible: bool,
    /// D16：只控制提示线绘制，不影响吸附求解。
    pub(in crate::bridge) show_guides: bool,
    /// D16：关闭时求解直接短路，不吸附也不画线。
    pub(in crate::bridge) snap_enabled: bool,
    /// 图层面板鼠标滚轮速度倍率（默认 2.0 = 原速两倍）。
    pub(in crate::bridge) layer_scroll_speed: f32,
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
                        "sel_border" => {
                            if let Ok(v) = value.parse::<u32>() {
                                cfg.sel_border = v.min(400);
                            }
                        }
                        "sel_smooth" => {
                            if let Ok(v) = value.parse::<u32>() {
                                cfg.sel_smooth = v.min(400);
                            }
                        }
                        "sel_feather" => {
                            if let Ok(v) = value.parse::<u32>() {
                                cfg.sel_feather = v.min(250);
                            }
                        }
                        "sel_tool" => match value {
                            "rect" | "ellipse" | "lasso" | "polygon" | "wand" | "quick" => {
                                cfg.sel_tool = value.to_string()
                            }
                            _ => {}
                        },
                        "sel_wand" => {
                            if let Some(v) = normalize_wand(value) {
                                cfg.sel_wand = v;
                            }
                        },
                        "sel_quick" => {
                            if let Some(v) = normalize_quick(value) {
                                cfg.sel_quick = v;
                            }
                        },
                        "paint_spec" if !value.is_empty() => {
                            cfg.paint_spec = value.to_string()
                        }
                        "transform_panel_visible" => {
                            cfg.transform_panel_visible = value == "true"
                        }
                        "show_guides" => cfg.show_guides = value != "false",
                        "snap_enabled" => cfg.snap_enabled = value != "false",
                        "layer_scroll_speed" => {
                            if let Ok(v) = value.parse::<f32>() {
                                if v.is_finite() {
                                    cfg.layer_scroll_speed = v.clamp(0.25, 8.0);
                                }
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
                "checker_light={}\nchecker_dark={}\nopen_in_new_window={}\nsave_quality={}\nruler_bottom_up={}\nmax_dimension={}\nsel_border={}\nsel_smooth={}\nsel_feather={}\nsel_tool={}\nsel_wand={}\nsel_quick={}\npaint_spec={}\ntransform_panel_visible={}\nshow_guides={}\nsnap_enabled={}\nlayer_scroll_speed={}\n",
                self.checker_light,
                self.checker_dark,
                self.open_in_new_window,
                self.save_quality,
                self.ruler_bottom_up,
                self.max_dimension,
                self.sel_border,
                self.sel_smooth,
                self.sel_feather,
                self.sel_tool,
                self.sel_wand,
                self.sel_quick,
                self.paint_spec,
                self.transform_panel_visible,
                self.show_guides,
                self.snap_enabled,
                self.layer_scroll_speed
            );
            let _ = std::fs::write(path, text);
        }
    }
}

/// 把 `容差,连续,取样全部图层` 规范化成 "32,1,0"；非法输入返回 None
/// （调用方保留默认值）。写在配置里的是原样字符串，QML 直接解析。
pub(in crate::bridge) fn normalize_wand(spec: &str) -> Option<String> {
    let mut it = spec.split(',');
    let tol: i32 = it.next()?.trim().parse().ok()?;
    let contig = it.next().unwrap_or("1").trim();
    let all = it.next().unwrap_or("1").trim();
    let flag = |s: &str| if s == "1" || s.eq_ignore_ascii_case("true") { 1 } else { 0 };
    Some(format!("{},{},{}", tol.clamp(0, 255), flag(contig), flag(all)))
}

/// 把 `笔刷,容差,扩散,取样全部图层,自动增强` 规范化成 "24,40,160,1,1"。
pub(in crate::bridge) fn normalize_quick(spec: &str) -> Option<String> {
    let mut it = spec.split(',');
    let radius: i32 = it.next()?.trim().parse().ok()?;
    let tol: i32 = it.next().unwrap_or("40").trim().parse().ok()?;
    let grow: i32 = it.next().unwrap_or("160").trim().parse().ok()?;
    let flag = |s: &str| if s == "1" || s.eq_ignore_ascii_case("true") { 1 } else { 0 };
    let all = flag(it.next().unwrap_or("1").trim());
    let enhance = flag(it.next().unwrap_or("1").trim());
    Some(format!(
        "{},{},{},{},{}",
        radius.clamp(2, 200),
        tol.clamp(0, 255),
        grow.clamp(0, 1000),
        all,
        enhance
    ))
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
