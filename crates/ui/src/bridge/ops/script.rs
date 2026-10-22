//! 探针脚本引擎：`--script=op;op;...` 自动化路径。
use super::super::*;

impl qobject::Bridge {
    pub(in crate::bridge) fn run_script(mut self: Pin<&mut Self>, script: &str) {
        for op in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            match op {
                "rotate-cw" => self.as_mut().rotate_cw(),
                "rotate-ccw" => self.as_mut().rotate_ccw(),
                "flip-h" => self.as_mut().flip_horizontal(),
                "flip-v" => self.as_mut().flip_vertical(),
                "undo" => self.as_mut().undo(),
                "redo" => self.as_mut().redo(),
                "zoom-in" => self.as_mut().zoom_in(),
                "zoom-out" => self.as_mut().zoom_out(),
                "zoom-fit" => self.as_mut().zoom_fit(),
                "zoom-actual" => self.as_mut().zoom_actual(),
                // `zoom:31` — jump to an exact zoom percentage (headless
                // preview-sharpness checks: no wheel stepping involved).
                other if other.starts_with("zoom:") => {
                    if let Some(pct) = other
                        .strip_prefix("zoom:")
                        .and_then(|v| v.trim().parse::<f64>().ok())
                    {
                        if let Some(session) = self.session.borrow_mut().as_mut() {
                            session.zoom =
                                (pct / 100.0).clamp(MIN_ZOOM, MAX_ZOOM);
                            session.fit = false;
                        }
                        self.as_mut().refresh_preview_zoom();
                    }
                }
                "save" => self.as_mut().save_edited(),
                "reset-crop" => self.as_mut().reset_crop(),
                "toggle-visible-selected" => self.as_mut().toggle_selected_visible(),
                "delete-selected" => self.as_mut().delete_selected_layer(),
                "layer-up" => self.as_mut().move_selected_up(),
                "layer-down" => self.as_mut().move_selected_down(),
                "paste" => {
                    if self.as_mut().paste_clipboard() {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-paste-ok\"}}");
                    }
                }
                "next" => self.as_mut().navigate(1),
                "prev" => self.as_mut().navigate(-1),
                "copy" => self.as_mut().copy_selected_layer(),
                "cut" => self.as_mut().cut_selected_layer(),
                // ---- 尺子 / 拾色器 ----
                "ruler-bottom-up" => self.as_mut().update_ruler_bottom_up(true),
                "ruler-top-down" => self.as_mut().update_ruler_bottom_up(false),
                "picker-open" => self.as_mut().picker_begin(),
                "picker-close" => self.as_mut().picker_close(),
                "picker-clear" => self.as_mut().picker_clear(),
                // `picker-pick:120,340` — sample one image pixel (repeatable).
                other if other.starts_with("picker-pick:") => {
                    if !*self.picker_open() {
                        self.as_mut().picker_begin();
                    }
                    let spec = other.strip_prefix("picker-pick:").unwrap_or("");
                    if let Some((x, y)) = spec.split_once(',') {
                        if let (Ok(x), Ok(y)) = (x.trim().parse::<i32>(), y.trim().parse::<i32>()) {
                            self.as_mut().picker_pick(x, y);
                        }
                    }
                }
                // Panel "unlock base" flow: converts the base into a
                // regular bottom layer (smoke-test / automation hook).
                "unlock-base" => self.as_mut().set_base_locked(false),
                // Smoke-test the markup pipeline: rect (filled) + arrow +
                // brush, restyle the rect, undo everything. Prints a mark
                // event so headless runs can assert the flow worked.
                "add-marks" => {
                    self.as_mut().add_shape_mark(
                        0, 10.0, 10.0, 60.0, 40.0, 3, &QString::from("#e24b4a"),
                        true, &QString::from("#f5e14a"),
                    );
                    self.as_mut().add_shape_mark(
                        2, 80.0, 20.0, 140.0, 60.0, 4, &QString::from("#378add"),
                        false, &QString::from("#378add"),
                    );
                    self.as_mut().add_brush_mark(
                        &QString::from("10,80 30,90 60,70 90,85"), 4,
                        &QString::from("#0f6e56"),
                    );
                    self.as_mut().select_mark(0);
                    self.as_mut().update_mark_style(
                        6, &QString::from("#111111"), true,
                        &QString::from("#ffffff"),
                    );
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"marks\",\"count\":{},\"selected\":{}}}",
                        {
                            let guard = self.session.borrow();
                            guard
                                .as_ref()
                                .map(|s| s.doc.marks.len())
                                .unwrap_or(0)
                        },
                        self.selected_mark(),
                    );
                }
                // Two ellipses (one drawn each direction) + select the second,
                // for headless selection-box alignment checks.
                "add-ellipse" => {
                    self.as_mut().add_shape_mark(
                        1, 100.0, 100.0, 260.0, 220.0, 4, &QString::from("#e24b4a"),
                        false, &QString::from("#e24b4a"),
                    );
                    self.as_mut().add_shape_mark(
                        1, 500.0, 200.0, 380.0, 320.0, 4, &QString::from("#e24b4a"),
                        false, &QString::from("#e24b4a"),
                    );
                    self.as_mut().select_mark(1);
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"marks\",\"count\":2,\"selected\":{}}}",
                        self.selected_mark(),
                    );
                }
                // Hide the first mark via the panel eye path (headless check).
                "mark-hide" => {
                    self.as_mut().set_mark_visible(0, false);
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"mark-hide\",\"count\":{}}}",
                        {
                            let guard = self.session.borrow();
                            guard
                                .as_ref()
                                .map(|s| s.doc.marks.len())
                                .unwrap_or(0)
                        },
                    );
                }
                // Batch-B smoke: one mosaic stroke + one text mark.
                "add-b2" => {
                    self.as_mut().add_mosaic_mark(
                        &QString::from("120,120 160,140 200,180 240,200"),
                        14,
                        10,
                    );
                    self.as_mut().add_text_mark(
                        300.0, 150.0, &QString::from("示例文本 Abc 123"), &QString::from("msyh"),
                        36, true, false, &QString::from("#378add"),
                    );
                    self.as_mut().add_mosaic_rect(480.0, 380.0, 660.0, 470.0, 20);
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"marks\",\"count\":{}}}",
                        {
                            let guard = self.session.borrow();
                            guard
                                .as_ref()
                                .map(|s| s.doc.marks.len())
                                .unwrap_or(0)
                        },
                    );
                }
                // Dump the flattened (marks composited) canvas to a PNG for
                // headless verification of mark rendering.
                // ---- 调整颜色 headless hooks ----
                // `adjust:exposure=40,saturation=-30` — percentages (gamma is
                // a raw ratio). Opens the panel implicitly.
                "adjust-begin" => self.as_mut().adjust_begin(),
                // Auto levels needs an open panel (it edits the live params).
                "adjust-auto" => {
                    if !*self.adjust_open() {
                        self.as_mut().adjust_begin();
                    }
                    self.as_mut().adjust_auto_levels();
                }
                "adjust-reset" => self.as_mut().adjust_reset(),
                "adjust-commit" => self.as_mut().adjust_commit(),
                "adjust-cancel" => self.as_mut().adjust_cancel(),
                other if other.starts_with("adjust:") => {
                    if !*self.adjust_open() {
                        self.as_mut().adjust_begin();
                    }
                    let spec = other.strip_prefix("adjust:").unwrap_or("");
                    for pair in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        let Some((name, value)) = pair.split_once('=') else {
                            continue;
                        };
                        let Ok(raw) = value.trim().parse::<f64>() else {
                            continue;
                        };
                        // gamma is a ratio; everything else is a percentage.
                        let value = if name.trim() == "gamma" {
                            raw
                        } else {
                            raw / 100.0
                        };
                        self.as_mut()
                            .adjust_set(&QString::from(name.trim().to_string()), value);
                    }
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"adjust-set\",\"values\":{}}}",
                        self.adjust_values(),
                    );
                }
                "dump-flat" => {
                    let guard = self.session.borrow();
                    if let Some(session) = guard.as_ref() {
                        let flat = session.doc.flatten();
                        let _ = encode(&flat, Format::Png, 92)
                            .map(|bytes| {
                                std::fs::write("tmp/flat.png", bytes).ok();
                                println!("{{\"phase\":\"ui\",\"event\":\"dump-flat\",\"size\":\"{}x{}\"}}",
                                         flat.width(), flat.height());
                            });
                    }
                }
                "marks-undo" => {                    self.as_mut().undo();
                    self.as_mut().undo();
                    self.as_mut().undo();
                    self.as_mut().undo();
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"marks-undo\",\"count\":{}}}",
                        {
                            let guard = self.session.borrow();
                            guard
                                .as_ref()
                                .map(|s| s.doc.marks.len())
                                .unwrap_or(0)
                        }
                    );
                }
                // Live-drag smoke test: select mark 0 and move it WITHOUT
                // committing, so a screenshot shows the live-rendered state.
                "mark-live" => {
                    // Seed one mark, then simulate a live drag (begin + update
                    // without commit) so headless runs can assert the live path.
                    self.as_mut().add_shape_mark(
                        0, 10.0, 10.0, 60.0, 40.0, 3, &QString::from("#e24b4a"),
                        false, &QString::from("#e24b4a"),
                    );
                    self.as_mut().select_mark(0);
                    self.as_mut().mark_drag_begin();
                    self.as_mut().mark_drag_update(30.0, 15.0, 50.0, 30.0);
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"mark-live\",\"rect\":{:?}}}",
                        {
                            let guard = self.session.borrow();
                            guard
                                .as_ref()
                                .and_then(|s| s.doc.marks.first())
                                .map(|m| m.rect)
                        }
                    );
                }
                other if other.starts_with("opacity:") => {
                    match other
                        .strip_prefix("opacity:")
                        .and_then(|s| s.trim().parse::<f32>().ok())
                    {
                        Some(v) => self.as_mut().set_selected_opacity(v, true),
                        None => println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        ),
                    }
                }
                other if other.starts_with("rename:") => {
                    let name = other.strip_prefix("rename:").unwrap_or("").trim();
                    if !name.is_empty() {
                        self.as_mut()
                            .rename_selected_layer(&QString::from(name.to_string()))
                    } else {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}")
                    }
                }
                other if other.starts_with("scale-selected:") => {
                    let nums: Vec<i32> = other
                        .strip_prefix("scale-selected:")
                        .unwrap_or("")
                        .split(',')
                        .map(str::trim)
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    if nums.len() == 2 {
                        self.as_mut()
                            .scale_selected_layer(nums[0], nums[1], false)
                    } else {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}")
                    }
                }
                other => match other
                    .strip_prefix("copy:")
                    .or_else(|| other.strip_prefix("cut:"))
                    .or_else(|| other.strip_prefix("erase:"))
                {
                    Some(spec) => {
                        let nums: Vec<i32> = spec
                            .split(',')
                            .map(str::trim)
                            .filter_map(|s| s.parse().ok())
                            .collect();
                        if nums.len() != 4 {
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                            );
                        } else if other.starts_with("copy:") {
                            self.as_mut()
                                .copy_selection_pixels(nums[0], nums[1], nums[2], nums[3]);
                        } else if other.starts_with("cut:") {
                            self.as_mut()
                                .cut_selection_pixels(nums[0], nums[1], nums[2], nums[3]);
                        } else {
                            self.as_mut()
                                .erase_selection_pixels(nums[0], nums[1], nums[2], nums[3]);
                        }
                    }
                    None => match other.strip_prefix("extract:") {
                        Some(spec) => {
                            let nums: Vec<i32> = spec
                                .split(',')
                                .map(str::trim)
                                .filter_map(|s| s.parse().ok())
                                .collect();
                            if nums.len() == 4 {
                                self.as_mut()
                                    .extract_selection(nums[0], nums[1], nums[2], nums[3])
                            } else {
                                println!(
                                    "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                )
                            }
                        }
                        None => match other.strip_prefix("select-layer:") {
                            Some(spec) => match spec.trim().parse::<i32>() {
                                Ok(n) => self.as_mut().select_layer(n),
                                Err(_) => println!(
                                    "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                ),
                            },
                            None => match other.strip_prefix("translate-selected:") {
                                Some(spec) => {
                                    let nums: Vec<i32> = spec
                                        .split(',')
                                        .map(str::trim)
                                        .filter_map(|s| s.parse().ok())
                                        .collect();
                                    if nums.len() == 2 {
                                        self.as_mut().translate_selected(nums[0], nums[1])
                                    } else {
                                        println!(
                                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                        )
                                    }
                                }
                                None => match other.strip_prefix("crop:") {
                                    Some(spec) => {
                                        let nums: Vec<i32> = spec
                                            .split(',')
                                            .map(str::trim)
                                            .filter_map(|s| s.parse().ok())
                                            .collect();
                                        if nums.len() == 4 {
                                            self.as_mut()
                                                .crop_to(nums[0], nums[1], nums[2], nums[3])
                                        } else {
                                            println!(
                                                "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                            )
                                        }
                                    }
                                    _ => match other.strip_prefix("resize:") {
                                        Some(spec) => {
                                            let nums: Vec<i32> = spec
                                                .split(',')
                                                .map(str::trim)
                                                .filter_map(|s| s.parse().ok())
                                                .collect();
                                            if nums.len() == 2 {
                                                self.as_mut().resize_to(nums[0], nums[1])
                                            } else {
                                                println!(
                                                    "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                                )
                                            }
                                        }
                                        None => println!(
                                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                                        ),
                                    },
                                },
                            },
                        },
                    },
                },
            }
        }
println!(
                "{{\"phase\":\"ui\",\"event\":\"script-done\",\"width\":{},\"height\":{},\"display\":\"{}x{}\",\"zoom\":{},\"undo\":{},\"redo\":{},\"layers\":{},\"selected_layer\":{},\"status\":\"{}\"}}",
                self.image_width(),
                self.image_height(),
                self.display_width(),
                self.display_height(),
                self.zoom_percent(),
                self.can_undo(),
                self.can_redo(),
                self.layer_count(),
                self.selected_layer(),
                self.status(),
            );
    }
}
