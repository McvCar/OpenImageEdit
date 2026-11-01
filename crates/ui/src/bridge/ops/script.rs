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
                // ---- 选区探针（无头验证用，拖拽手势无法脚本化） ----
                "sel-all" => self.as_mut().sel_select_all(),
                "sel-none" => self.as_mut().sel_clear(),
                "sel-invert" => self.as_mut().sel_invert(),
                "sel-quickmask" => self.as_mut().sel_toggle_quick_mask(),
                "crop-sel" => self.as_mut().crop_to_selection(),
                // `sel-dump` — 断言用：bounds / ops / rev。
                "sel-dump" => {
                    let (rect, ops, rev) = {
                        let guard = self.session.borrow();
                        let session = guard.as_ref();
                        (
                            session
                                .and_then(|s| s.selection.as_ref())
                                .map(|st| st.mask().bounds)
                                .unwrap_or_default(),
                            session
                                .and_then(|s| s.selection.as_ref())
                                .map(|st| (st.ops.border, st.ops.smooth, st.ops.feather))
                                .unwrap_or((0, 0, 0)),
                            session.map(|s| s.sel_rev).unwrap_or(0),
                        )
                    };
                    let active = {
                        let guard = self.session.borrow();
                        guard
                            .as_ref()
                            .and_then(|s| s.selection.as_ref())
                            .map(|st| !st.is_empty())
                            .unwrap_or(false)
                    };
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"sel-dump\",\"active\":{active},\
                         \"rect\":\"{},{},{},{}\",\"border\":{},\"smooth\":{},\"feather\":{},\"rev\":{rev}}}",
                        rect.x, rect.y, rect.width, rect.height, ops.0, ops.1, ops.2
                    );
                }
                // `sel-outline` — 打印轮廓 JSON（蚂蚁线数据源）。
                "sel-outline" => {
                    let json = self.as_mut().sel_outline().to_string();
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"sel-outline\",\"len\":{},\"rings\":{}}}",
                        json.len(),
                        json.matches("],[[").count() + 1
                    );
                }
                other if other.starts_with("sel-rect:") || other.starts_with("sel-ellipse:") => {
                    let (body, mode) = split_mode(other);
                    let nums = parse_nums(body);
                    if nums.len() == 4 {
                        if other.starts_with("sel-rect:") {
                            self.as_mut().sel_set_rect(
                                nums[0], nums[1], nums[2], nums[3], mode,
                            );
                        } else {
                            self.as_mut().sel_set_ellipse(
                                nums[0], nums[1], nums[2], nums[3], mode,
                            );
                        }
                    } else {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}")
                    }
                }
                other if other.starts_with("sel-polygon:") => {
                    let (body, mode) = split_mode(other);
                    let pts = body.strip_prefix("sel-polygon:").unwrap_or("");
                    // "x,y x,y x,y"（空格分隔点，避免吃掉脚本的 `;`）
                    let norm = pts.trim().replace(' ', ",");
                    let nums: Vec<f32> = norm
                        .split(',')
                        .map(str::trim)
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    if nums.len() >= 6 && nums.len() % 2 == 0 {
                        let pair: Vec<String> = nums
                            .chunks(2)
                            .map(|c| format!("{},{}", c[0], c[1]))
                            .collect();
                        self.as_mut()
                            .sel_set_polygon(&QString::from(pair.join(" ")), mode);
                    } else {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}")
                    }
                }
                other if other.starts_with("sel-border:")
                    || other.starts_with("sel-smooth:")
                    || other.starts_with("sel-feather:") =>
                {
                    let v = other
                        .rsplit(':')
                        .next()
                        .and_then(|s| s.trim().parse::<i32>().ok());
                    match (v, other) {
                        (Some(k), o) if o.starts_with("sel-border:") => {
                            self.as_mut().sel_set_border(k)
                        }
                        (Some(k), o) if o.starts_with("sel-smooth:") => {
                            self.as_mut().sel_set_smooth(k)
                        }
                        (Some(k), _) => self.as_mut().sel_set_feather(k),
                        _ => println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        ),
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
                        } else {
                            // 旧脚本形式把矩形当选区：先落一个矩形选区再执行。
                            self.as_mut().sel_set_rect(
                                nums[0] as f64,
                                nums[1] as f64,
                                (nums[0] + nums[2]) as f64,
                                (nums[1] + nums[3]) as f64,
                                0,
                            );
                            if other.starts_with("copy:") {
                                self.as_mut().copy_selection_pixels();
                            } else if other.starts_with("cut:") {
                                self.as_mut().cut_selection_pixels();
                            } else {
                                self.as_mut().erase_selection_pixels();
                            }
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
                                self.as_mut().sel_set_rect(
                                    nums[0] as f64,
                                    nums[1] as f64,
                                    (nums[0] + nums[2]) as f64,
                                    (nums[1] + nums[3]) as f64,
                                    0,
                                );
                                self.as_mut().extract_selection()
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

/// `"sel-rect:1,2,3,4@2"` → `("sel-rect:1,2,3,4", 2)`. The `@mode` suffix
/// carries the PS-style combine mode (0 replace / 1 add / 2 subtract /
/// 3 intersect) without colliding with the `,`-separated coordinates.
fn split_mode(op: &str) -> (&str, i32) {
    match op.split_once('@') {
        Some((body, mode)) => (body, mode.trim().parse().unwrap_or(0)),
        None => (op, 0),
    }
}

/// Numbers after the first `:` of a script op.
fn parse_nums(op: &str) -> Vec<f64> {
    op.split_once(':')
        .map(|(_, rest)| {
            rest.split(',')
                .map(str::trim)
                .filter_map(|s| s.parse::<f64>().ok())
                .collect()
        })
        .unwrap_or_default()
}
