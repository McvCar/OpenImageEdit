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
                // 删除"当前选区"的像素（不重设选区，探针验证反选后
                // erase 的作用范围用）。
                "erase-sel" => self.as_mut().erase_selection_pixels(),
                // Ctrl+拖动选区的手势核心：把选区像素剪成独立图层（原位挖空），
                // 并选中新图层。探针里配合 toggle-visible-selected + save
                // 可以断言"原位置确实空了"。
                "cut-sel" => {
                    self.as_mut().cut_selection_to_layer();
                }
                // `cursor-dump:C:/tmp/cur` — 把三枚自定义光标渲染成 PNG。
                // PrintWindow 抓不到系统光标，只能直接导出绘制缓冲目检。
                other if other.starts_with("cursor-dump:") => {
                    super::cursor::dump_cursors(other.trim_start_matches("cursor-dump:").trim())
                }
                // `cursor:3` — 强制接管画布光标（不用按键，探针断言用）。
                other if other.starts_with("cursor:") => {
                    if let Ok(kind) = other.trim_start_matches("cursor:").trim().parse::<i32>() {
                        self.as_mut().cursor_install();
                        self.as_mut().cursor_set_kind(kind);
                    }
                }
                // `sel-dump` — 断言用：bounds / ops / rev。
                "sel-dump" => {
                    let (rect, ops, rev) = {
                        let guard = self.session.borrow();
                        let session = guard.as_ref();
                        (
                            session
                                .and_then(|s| s.doc.selection.as_ref())
                                .map(|st| st.mask().bounds)
                                .unwrap_or_default(),
                            session
                                .and_then(|s| s.doc.selection.as_ref())
                                .map(|st| (st.ops.border, st.ops.smooth, st.ops.feather))
                                .unwrap_or((0, 0, 0)),
                            session.map(|s| s.sel_rev).unwrap_or(0),
                        )
                    };
                    let active = {
                        let guard = self.session.borrow();
                        guard
                            .as_ref()
                            .and_then(|s| s.doc.selection.as_ref())
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
                // `wand:x,y[,tol[,contig[,sampleAll]]]@mode`
                // 魔术棒：默认容差 32、连续、取样全部图层。
                other if other.starts_with("wand:") => {
                    let (body, mode) = split_mode(other);
                    let nums = parse_nums(body);
                    if nums.len() >= 2 {
                        let tol = nums.get(2).copied().unwrap_or(32.0) as i32;
                        let contig = nums.get(3).copied().unwrap_or(1.0) != 0.0;
                        let all = nums.get(4).copied().unwrap_or(1.0) != 0.0;
                        self.as_mut()
                            .sel_magic_wand(nums[0], nums[1], tol, contig, all, mode);
                    } else {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}")
                    }
                }
                // `sel-tool:wand` — 切到指定选区工具。
                // QML 的 selTool 初值绑定在 selToolSaved 上，改这个属性即可
                // 让标记栏/右键菜单露出对应的参数区（截图目检用）。
                other if other.starts_with("sel-tool:") => {
                    let name = other["sel-tool:".len()..].trim();
                    self.as_mut()
                        .set_sel_tool_saved(QString::from(name));
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"sel-tool\",\"tool\":\"{}\"}}",
                        name
                    );
                }
                // `qsel:x1,y1 x2,y2 ...|radius,tol,grow@mode`
                // 快速选择：一条笔迹（空格分隔的坐标点），`|` 后面是参数，
                // 省略则用 24,40,160。整条涂抹按一次撤销提交。
                other if other.starts_with("qsel:") => {
                    let (body, mode) = split_mode(other);
                    let rest = body.trim_start_matches("qsel:");
                    let (pts_part, args_part) = match rest.split_once('|') {
                        Some((a, b)) => (a, Some(b)),
                        None => (rest, None),
                    };
                    // 注意：不能用 parse_nums —— 它只解析第一个冒号后面的内容，
                    // 而这里的参数块（"24,40,160"）没有冒号。
                    let args: Vec<f64> = args_part
                        .map(|s| {
                            s.split(',')
                                .filter_map(|t| t.trim().parse::<f64>().ok())
                                .collect()
                        })
                        .unwrap_or_default();
                    let radius = args.first().copied().unwrap_or(24.0);
                    let tol = args.get(1).copied().unwrap_or(40.0) as i32;
                    let grow = args.get(2).copied().unwrap_or(160.0) as i32;
                    let pts: Vec<(f64, f64)> = pts_part
                        .split_whitespace()
                        .filter_map(|p| {
                            let (x, y) = p.split_once(',')?;
                            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
                        })
                        .collect();
                    if pts.is_empty() {
                        println!("{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}");
                    } else {
                        for (px, py) in &pts {
                            self.as_mut()
                                .quick_select_brush(*px, *py, radius, tol, grow, mode);
                        }
                        self.as_mut().quick_select_commit();
                        let area = self
                            .session
                            .borrow()
                            .as_ref()
                            .and_then(|s| s.doc.selection.as_ref().map(|st| st.mask().area()))
                            .unwrap_or(0);
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"quick-select\",\"points\":{},\"radius\":{},\"tolerance\":{},\"grow\":{},\"mode\":{},\"area\":{}}}",
                            pts.len(),
                            radius,
                            tol,
                            grow,
                            mode,
                            area
                        );
                    }
                }
                other if other.starts_with("sel-resize:") => {
                    let nums = parse_nums(other);
                    if nums.len() == 4 {
                        self.as_mut().sel_resize(nums[0], nums[1], nums[2], nums[3]);
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
                // `select-layer:0` — 选中某个图层（探针：验证"选中图层后翻转
                // 只作用于该层"这类依赖选中态的路径）。
                other if other.starts_with("select-layer:") => {
                    if let Some(ix) = other
                        .strip_prefix("select-layer:")
                        .and_then(|v| v.trim().parse::<i32>().ok())
                    {
                        self.as_mut().set_selected_layer(ix);
                        self.as_mut().refresh_layers_only();
                    }
                }
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
                // ---- 修复 / 油漆 / 图层旋转探针 ----
                // `spot:x,y[,size][,strength]` — 单点污点修复（begin 即落一帧）。
                other if other.starts_with("spot:") => {
                    let n = parse_nums(other);
                    let a = |i: usize, d: f64| n.get(i).copied().unwrap_or(d);
                    self.as_mut().retouch_begin(
                        &QString::from("spot"),
                        a(0, 0.0),
                        a(1, 0.0),
                        a(2, 40.0),
                        70.0,
                        20.0,
                        a(3, 100.0),
                    );
                    self.as_mut().retouch_end();
                    println!("{{\"phase\":\"ui\",\"event\":\"spot-heal\"}}");
                }
                // `heal:size,sx,sy,x1,y1,x2,y2[,x3,y3…]` — 修复画笔：
                // 先在 (sx,sy) 取源，再沿折线涂（每段都走 move 的完整重算）。
                other if other.starts_with("heal:") => {
                    let n = parse_nums(other);
                    if n.len() >= 6 {
                        let pts: Vec<(f64, f64)> = n[3..]
                            .chunks(2)
                            .filter(|c| c.len() == 2)
                            .map(|c| (c[0], c[1]))
                            .collect();
                        if !pts.is_empty() {
                            self.as_mut().retouch_set_source(n[1], n[2]);
                            self.as_mut().retouch_begin(
                                &QString::from("heal"),
                                pts[0].0,
                                pts[0].1,
                                n[0],
                                70.0,
                                20.0,
                                100.0,
                            );
                            for p in &pts[1..] {
                                self.as_mut().retouch_move(p.0, p.1);
                            }
                            self.as_mut().retouch_end();
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"heal-clone\",\"points\":{}}}",
                                pts.len()
                            );
                        }
                    }
                }
                // `patch-fill:x,y,w,h,dx,dy,feather` — 圈一块矩形区域再拖 dx/dy。
                other if other.starts_with("patch-fill:") => {
                    let n = parse_nums(other);
                    if n.len() == 7 {
                        let (x, y, w, h) = (n[0], n[1], n[2], n[3]);
                        let poly = format!(
                            "{x},{y} {},{} {},{} {},{}",
                            x + w,
                            y,
                            x + w,
                            y + h,
                            x,
                            y + h
                        );
                        self.as_mut().patch_set_region(&QString::from(poly.as_str()));
                        self.as_mut().patch_drag_begin(x + w / 2.0, y + h / 2.0, n[6] as i32);
                        self.as_mut().patch_drag_move(n[4], n[5]);
                        let ok = self.as_mut().patch_drag_end();
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"patch-fill\",\"ok\":{ok}}}"
                        );
                    }
                }
                // `bucket:x,y[,tol][,contiguous][,R][,G][,B][,opacity]`
                other if other.starts_with("bucket:") => {
                    let n = parse_nums(other);
                    if n.len() >= 2 {
                        let a = |i: usize, d: f64| n.get(i).copied().unwrap_or(d);
                        let hex = format!(
                            "#{:02X}{:02X}{:02X}",
                            a(4, 255.0) as u8,
                            a(5, 0.0) as u8,
                            a(6, 0.0) as u8
                        );
                        self.as_mut().bucket_fill(
                            n[0],
                            n[1],
                            a(2, 32.0) as i32,
                            a(3, 1.0) > 0.5,
                            a(7, 100.0),
                            &QString::from(hex.as_str()),
                        );
                    }
                }
                // `gradient:x0,y0,x1,y1,radial,reverse,RRGGBB,RRGGBB[,opacity]`
                other if other.starts_with("gradient:") => {
                    let body = other.trim_start_matches("gradient:");
                    let parts: Vec<&str> = body.split(',').map(str::trim).collect();
                    if parts.len() == 9 {
                        let f = |i: usize, d: f64| parts[i].parse::<f64>().unwrap_or(d);
                        let c0 = format!("#{}", parts[6].trim_start_matches('#'));
                        let c1 = format!("#{}", parts[7].trim_start_matches('#'));
                        self.as_mut().gradient_begin(
                            f(0, 0.0),
                            f(1, 0.0),
                            f(4, 0.0) > 0.5,
                            f(5, 0.0) > 0.5,
                            f(8, 100.0),
                            &QString::from(c0.as_str()),
                            &QString::from(c1.as_str()),
                        );
                        self.as_mut().gradient_move(f(2, 10.0), f(3, 10.0));
                        self.as_mut().gradient_end();
                        println!("{{\"phase\":\"ui\",\"event\":\"gradient\"}}");
                    }
                }
                // `layer-info:index[,tag]` —— 打印图层位图尺寸与 offset。
                // 诊断"转一圈图层变大 / 放手后位置漂移"时直接读这几项，
                // 别再从截图上量像素。
                other if other.starts_with("layer-info:") => {
                    let n = parse_nums(other);
                    let ix = n.first().copied().unwrap_or(0.0) as i32;
                    let guard = self.session.borrow();
                    if let Some(session) = guard.as_ref() {
                        if let Some(layer) = session.doc.layers.get(ix.max(0) as usize) {
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"layer-info\",\"index\":{ix},\
                                 \"w\":{},\"h\":{},\"ox\":{},\"oy\":{},\"history\":{}}}",
                                layer.image.width(),
                                layer.image.height(),
                                layer.offset.0,
                                layer.offset.1,
                                session.doc.history_len()
                            );
                        }
                    }
                }
                // `layer-alpha:index` —— 图层 alpha 总和 / 不透明像素数。
                // 验证「选区擦除到底挖的哪一层」：擦图层时总和会掉，擦背景
                // 时图层总和不变。
                other if other.starts_with("layer-alpha:") => {
                    let n = parse_nums(other);
                    let ix = n.first().copied().unwrap_or(0.0) as i32;
                    let guard = self.session.borrow();
                    if let Some(session) = guard.as_ref() {
                        if let Some(layer) = session.doc.layers.get(ix.max(0) as usize) {
                            let mut sum: u64 = 0;
                            let mut opaque: u64 = 0;
                            for px in layer.image.pixels() {
                                sum += px.0[3] as u64;
                                if px.0[3] > 0 {
                                    opaque += 1;
                                }
                            }
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"layer-alpha\",\"index\":{ix},\
                                 \"sum\":{sum},\"opaque\":{opaque}}}"
                            );
                        }
                    }
                }
                // `rotate-handle:index,degrees[,expand]` —— 模拟拖一次画布旋柄
                // （begin keep_session → update → finish apply）。连写两条就等于
                // 用户来回拖两次，用来验证「不会越拖越糊」。
                other if other.starts_with("rotate-handle:") => {
                    let n = parse_nums(other);
                    if n.len() >= 2 {
                        let (ix, deg) = (n[0] as i32, n[1]);
                        let expand = n.get(2).copied().unwrap_or(1.0) > 0.5;
                        self.as_mut().rotate_layer_begin(ix, true);
                        self.as_mut().rotate_layer_update(deg, expand);
                        self.as_mut().rotate_layer_finish(true, deg, expand);
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"rotate-handle\",\"degrees\":{deg:.2}}}"
                        );
                    } else {
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        )
                    }
                }
                // `rotate-layer:index,degrees[,expand]`
                other if other.starts_with("rotate-layer:") => {
                    let n = parse_nums(other);
                    if n.len() >= 2 {
                        self.as_mut().rotate_layer_angle(
                            n[0] as i32,
                            n[1],
                            n.get(2).copied().unwrap_or(1.0) > 0.5,
                        );
                    } else {
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        )
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
