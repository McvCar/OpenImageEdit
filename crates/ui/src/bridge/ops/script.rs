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
                // `move-layer:dx,dy` —— 走画布拖动图层那条真实链路
                // （begin → update → end(true)），用来复现「框拖走了、图层没动」。
                // 配合 `layer-info:` 前后各打一次就能看出 offset 有没有落地。
                other if other.starts_with("move-layer:") => {
                    let n = parse_nums(other);
                    if n.len() >= 2 {
                        let (dx, dy) = (n[0].round() as i32, n[1].round() as i32);
                        self.as_mut().drag_translate_begin();
                        self.as_mut().drag_translate_update(dx, dy);
                        self.as_mut().drag_translate_end(true);
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"move-layer\",\"dx\":{dx},\"dy\":{dy}}}"
                        );
                    } else {
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        )
                    }
                }
                // `solve-scale:sx,sy,sw,sh,role,dx,dy,mods[,disp]` —— P1-A：
                // 直接调用缩放手势语义求解器，打印解算 JSON（框 + 翻转 +
                // 斜切 + 锚定补偿）。吸附候选按当前文档现场生成。
                other if other.starts_with("solve-scale:") => {
                    let body = other.trim_start_matches("solve-scale:");
                    let parts: Vec<&str> = body.split(',').collect();
                    if parts.len() >= 8 {
                        let nums: Vec<f64> =
                            parts[..4].iter().filter_map(|p| p.parse().ok()).collect();
                        let tail: Vec<f64> = parts[5..8]
                            .iter()
                            .filter_map(|p| p.parse().ok())
                            .collect();
                        if nums.len() == 4 && tail.len() == 3 {
                            let disp = parts
                                .get(8)
                                .and_then(|p| p.parse().ok())
                                .unwrap_or(1.0_f64);
                            let out = self.as_mut().transform_solve_scale(
                                nums[0], nums[1], nums[2], nums[3],
                                QString::from(parts[4]),
                                tail[0], tail[1], tail[2] as i32, disp,
                            );
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"solve-scale\",\"out\":{}}}",
                                out.to_string()
                            );
                        } else {
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                            )
                        }
                    } else {
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        )
                    }
                }
                // `skew-layer:dx_or_dy,role` —— P1-A：走完整斜切链路（solve →
                // begin → update(skew) → end(skew)），验证落定后 skew 进
                // params、原图不烘焙。`role` 取 w/e/n/s；位移按轴自动选取。
                other if other.starts_with("skew-layer:") => {
                    let body = other.trim_start_matches("skew-layer:");
                    let parts: Vec<&str> = body.split(',').collect();
                    if parts.len() >= 2 {
                        let delta = parts[0].parse::<f64>().unwrap_or(0.0);
                        let role = parts[1];
                        let ix = (*self.selected_layer()).max(0) as usize;
                        let box0 = {
                            let guard = self.session.borrow();
                            guard
                                .as_ref()
                                .and_then(|s| s.doc.layers.get(ix))
                                .map(|l| {
                                    let b = l.bounds();
                                    (b.x as f64, b.y as f64,
                                     b.width as f64, b.height as f64)
                                })
                                .unwrap_or((0.0, 0.0, 1.0, 1.0))
                        };
                        let (dx, dy) = if role.contains("w") || role.contains("e") {
                            (0.0, delta)
                        } else {
                            (delta, 0.0)
                        };
                        let out = self.as_mut().transform_solve_scale(
                            box0.0, box0.1, box0.2, box0.3,
                            QString::from(role), dx, dy,
                            super::transform::MOD_CTRL, 1.0,
                        );
                        // solve-scale 的 JSON 结构固定（transform.rs 打印），
                        // 用一个小读取器取数，避免为脚本探针引入 serde_json。
                        let text = out.to_string();
                        let num = |key: &str| -> f64 {
                            let pat = format!("\"{}\":", key);
                            match text.find(&pat) {
                                Some(i) => {
                                    let rest = &text[i + pat.len()..];
                                    let end = rest
                                        .find([',', '}'])
                                        .unwrap_or(rest.len());
                                    rest[..end].parse().unwrap_or(0.0)
                                }
                                None => 0.0,
                            }
                        };
                        let (x, y, w, h) = (
                            num("x").round() as i32,
                            num("y").round() as i32,
                            (num("w").round() as i32).max(1),
                            (num("h").round() as i32).max(1),
                        );
                        let (kx, ky) = (num("kx"), num("ky"));
                        let (cx, cy) = (num("cx"), num("cy"));
                        let edges = if role.contains("w") {
                            1
                        } else if role.contains("e") {
                            4
                        } else if role.contains("n") {
                            8
                        } else {
                            32
                        };
                        self.as_mut().scale_layer_begin();
                        self.as_mut().scale_layer_update(
                            w, h, x, y, false, false, kx, ky, cx, cy,
                        );
                        self.as_mut().scale_layer_end(
                            true, w, h, x, y, false, false, edges, kx, ky,
                        );
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"skew-layer\",\"kx\":{kx:.6},\
                             \"ky\":{ky:.6},\"box\":[{x},{y},{w},{h}]}}"
                        );
                    } else {
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        )
                    }
                }
                // `scale-layer:w,h` —— 模拟拖 8 点手柄把图层缩放到 w×h
                // （保持中心不变），走真实 begin → update → end 链路。用来
                // 复现「旋转 → 拉伸 → 再旋转」时外接矩形被二次烘焙、框越来
                // 越大。配合 layer-info 前后各打一次读尺寸。
                other if other.starts_with("scale-layer:") => {
                    let n = parse_nums(other);
                    if n.len() >= 2 {
                        let w = n[0].round().max(1.0) as i32;
                        let h = n[1].round().max(1.0) as i32;
                        let ix = (*self.selected_layer()).max(0) as usize;
                        let center = {
                            let guard = self.session.borrow();
                            guard.as_ref().and_then(|s| s.doc.layers.get(ix)).map(|l| {
                                (
                                    l.offset.0 as f64 + l.image.width() as f64 / 2.0,
                                    l.offset.1 as f64 + l.image.height() as f64 / 2.0,
                                )
                            })
                        };
                        let (ox, oy) = match center {
                            Some((cx, cy)) => (
                                editcore::round_center(cx - w as f64 / 2.0) as i32,
                                editcore::round_center(cy - h as f64 / 2.0) as i32,
                            ),
                            None => (0, 0),
                        };
                        self.as_mut().scale_layer_begin();
                        self.as_mut().scale_layer_update(w, h, ox, oy, false, false, 0.0, 0.0, 0.0, 0.0);
                        self.as_mut().scale_layer_end(true, w, h, ox, oy, false, false, 0, 0.0, 0.0);
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"scale-layer\",\"w\":{w},\"h\":{h},\
                             \"ox\":{ox},\"oy\":{oy}}}"
                        );
                    } else {
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        )
                    }
                }
                // `scale-hold:w,h[,ox,oy]` —— 只 begin + update、**不 end**：把
                // 拖动中的那一帧留在屏幕上（和 `rotate-hold` 同一个用途），用来
                // 核对「拖动过程中活动层是否画在自己该在的位置、且压在其他层
                // 之上」。落帧 URL 打进日志，测试直接读那张 BMP 做像素断言。
                other if other.starts_with("scale-hold:") => {
                    let n = parse_nums(other);
                    if n.len() >= 2 {
                        let w = n[0].round().max(1.0) as i32;
                        let h = n[1].round().max(1.0) as i32;
                        let ix = (*self.selected_layer()).max(0) as usize;
                        let (ox, oy) = if n.len() >= 4 {
                            (n[2].round() as i32, n[3].round() as i32)
                        } else {
                            let cur = {
                                let guard = self.session.borrow();
                                guard.as_ref().and_then(|s| s.doc.layers.get(ix)).map(|l| {
                                    (
                                        l.offset.0 as f64 + l.image.width() as f64 / 2.0,
                                        l.offset.1 as f64 + l.image.height() as f64 / 2.0,
                                    )
                                })
                            };
                            match cur {
                                Some((cx, cy)) => (
                                    editcore::round_center(cx - w as f64 / 2.0) as i32,
                                    editcore::round_center(cy - h as f64 / 2.0) as i32,
                                ),
                                None => (0, 0),
                            }
                        };
                        self.as_mut().scale_layer_begin();
                        self.as_mut().scale_layer_update(
                            w, h, ox, oy, false, false, 0.0, 0.0, 0.0, 0.0,
                        );
                        let url = {
                            let guard = self.session.borrow();
                            guard
                                .as_ref()
                                .and_then(|s| s.last_render.as_ref())
                                .map(|c| c.url.clone())
                                .unwrap_or_default()
                        };
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"scale-hold\",\"w\":{w},\"h\":{h},\
                             \"ox\":{ox},\"oy\":{oy},\"url\":\"{}\"}}",
                            url.replace('\\', "/")
                        );
                    } else {
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        );
                    }
                }
                // `gpu-gesture[:dx,dy]` —— 走一次真实移动手势
                // （transform_begin → 5×transform_update → transform_end），
                // 用来验证 Tier 1「静态层 + 活动层」两纹理快路径：日志必须
                // 出现 gpu-begin 与多条 gpu-skip，且拖动帧没有 render-profile。
                other if other.starts_with("gpu-gesture") => {
                    let n = parse_nums(other);
                    let (dx, dy) = (
                        n.first().copied().unwrap_or(40.0),
                        n.get(1).copied().unwrap_or(30.0),
                    );
                    let ix = (*self.selected_layer()).max(0) as i32;
                    let center = {
                        let guard = self.session.borrow();
                        guard.as_ref().and_then(|s| s.doc.layers.get(ix.max(0) as usize)).map(|l| {
                            (
                                l.offset.0 as f64 + l.image.width() as f64 / 2.0,
                                l.offset.1 as f64 + l.image.height() as f64 / 2.0,
                            )
                        })
                    };
                    match center {
                        Some((cx, cy)) => {
                            let began = self.as_mut().transform_begin(0, ix, cx, cy, 0, 1.0);
                            for i in 1..=5 {
                                let t = i as f64 / 5.0;
                                self.as_mut().transform_update(0, cx + dx * t, cy + dy * t, 0, 1.0);
                            }
                            let landed = self.as_mut().transform_end(true, 0);
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"gpu-gesture\",\"began\":{began},\
                                 \"landed\":{landed},\"dx\":{dx},\"dy\":{dy}}}"
                            );
                        }
                        None => println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        ),
                    }
                }
                // `move-gesture:dx,dy[,mods]` —— 走完整的「移动」三件套
                //（transform_begin → 5×transform_update → transform_end），
                // 用来验证修饰键（Ctrl 临时关吸附）与二次拖动的一致性。
                other if other.starts_with("move-gesture") => {
                    let n = parse_nums(other);
                    let (dx, dy) = (
                        n.first().copied().unwrap_or(40.0),
                        n.get(1).copied().unwrap_or(30.0),
                    );
                    let mods = n.get(2).copied().unwrap_or(0.0) as i32;
                    let ix = (*self.selected_layer()).max(0) as i32;
                    let start = {
                        let guard = self.session.borrow();
                        guard.as_ref().and_then(|s| s.doc.layers.get(ix.max(0) as usize)).map(|l| {
                            (l.offset.0 as f64 + l.image.width() as f64 / 2.0,
                             l.offset.1 as f64 + l.image.height() as f64 / 2.0)
                        })
                    };
                    if let Some((cx, cy)) = start {
                        let began = self.as_mut().transform_begin(0, ix, cx, cy, mods, 1.0);
                        for i in 1..=5 {
                            let t = i as f64 / 5.0;
                            self.as_mut()
                                .transform_update(0, cx + dx * t, cy + dy * t, mods, 1.0);
                        }
                        self.as_mut().drag_translate_end(true);
                        let landed = self.as_mut().transform_end(true, mods);
                        let (off, guides) = {
                            let guard = self.session.borrow();
                            match guard.as_ref() {
                                Some(s) => (
                                    s.doc
                                        .layers
                                        .get(ix.max(0) as usize)
                                        .map(|l| l.offset)
                                        .unwrap_or((0, 0)),
                                    s.snap.guides.len(),
                                ),
                                None => ((0, 0), 0),
                            }
                        };
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"move-gesture\",\"began\":{began},\
                             \"landed\":{landed},\"mods\":{mods},\"ox\":{},\"oy\":{},\
                             \"guides\":{guides}}}",
                            off.0, off.1
                        );
                    } else {
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        );
                    }
                }
                // `sel-state` —— 当前选区是否还在（裁剪 / 提取 / 剪切后必须消失）。
                "sel-state" => {
                    let guard = self.session.borrow();
                    let text = match guard.as_ref().and_then(|s| s.doc.selection.as_ref()) {
                        None => r#"{"active":false}"#.to_string(),
                        Some(st) => {
                            let b = st.base.bounds;
                            format!(
                                r#"{{"active":true,"x":{},"y":{},"w":{},"h":{}}}"#,
                                b.x, b.y, b.width, b.height
                            )
                        }
                    };
                    println!("{{\"phase\":\"ui\",\"event\":\"sel-state\",\"state\":{text}}}");
                }
                // `transform-state` —— P0-2b1 统一会话诊断：打印 kind / index /
                // special / cap。手势落定后必须是 active=false（无残留）。
                "transform-state" => {
                    let state = self
                        .session
                        .borrow()
                        .as_ref()
                        .map(|s| s.transform_state_json())
                        .unwrap_or_else(|| {
                            r#"{"active":false,"no_document":true}"#.to_string()
                        });
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"transform-state\",\"state\":{state}}}"
                    );
                }
                // `transform-abort` —— 统一取消。`rotate-hold` 故意不 finish，
                // 用它验证会话能被一次 take 清干净、不留临时合成帧。
                "transform-abort" => {
                    let had = self.as_mut().transform_abort();
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"transform-abort\",\"had\":{had}}}"
                    );
                }
                // `transform-params:x,y,w,h,rot,lock,ref` —— P0-5 数值面板写入
                //（一次调用一条历史），供无 GUI 回归脚本验证外接框语义。
                other if other.starts_with("transform-params:") => {
                    let n = parse_nums(other);
                    let g = |i: usize, d: f64| n.get(i).copied().unwrap_or(d);
                    let ok = self.as_mut().transform_set_params(
                        g(0, 0.0), g(1, 0.0), g(2, 100.0), g(3, 100.0),
                        g(4, 0.0), false, false, g(5, 1.0) != 0.0,
                        g(6, 0.0) as i32,
                    );
                    let values = self
                        .session
                        .borrow()
                        .as_ref()
                        .map(|s| s.transform_panel_json(*self.selected_layer()))
                        .unwrap_or_else(|| "{}".to_string());
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"transform-params\",\"ok\":{ok},\"values\":{values}}}"
                    );
                }
                // `snap-probe:x,y,w,h,edges` —— P0-4 求解器端到端（8 显示像素）。
                other if other.starts_with("snap-probe:") => {
                    let n = parse_nums(other);
                    let g = |i: usize| n.get(i).copied().unwrap_or(0.0);
                    let text = self.as_mut().transform_snap_rect(
                        g(0), g(1), g(2), g(3), g(4) as i32, 0, 1.0,
                    );
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"snap-probe\",\"result\":{}}}",
                        text.to_string()
                    );
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
                            // 没有 params 的普通位图图层也要报中心：默认取
                            // `offset + 位图尺寸/2`（此时它等价于 params 中心），
                            // 否则「旋转前 / 旋转后」的漂移回归会拿 0 当基线。
                            let (rot, cx, cy, sx, sy, sw, sh, has_t) =
                                match layer.transform.as_ref() {
                                    Some(t) => (
                                        t.rotation,
                                        t.center.0,
                                        t.center.1,
                                        t.scale.0,
                                        t.scale.1,
                                        t.source.width(),
                                        t.source.height(),
                                        true,
                                    ),
                                    None => (
                                        0.0,
                                        layer.offset.0 as f64 + layer.image.width() as f64 / 2.0,
                                        layer.offset.1 as f64 + layer.image.height() as f64 / 2.0,
                                        1.0,
                                        1.0,
                                        layer.image.width(),
                                        layer.image.height(),
                                        false,
                                    ),
                                };
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"layer-info\",\"index\":{ix},\
                                 \"w\":{},\"h\":{},\"ox\":{},\"oy\":{},\"history\":{},\
                                 \"rot\":{rot:.4},\"cx\":{cx:.3},\"cy\":{cy:.3},\
                                 \"sx\":{sx:.6},\"sy\":{sy:.6},\"sw\":{sw},\"sh\":{sh},\
                                 \"params\":{has_t}}}",
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
                // `layer-dump:index:path` —— 把图层位图原样存成 PNG。
                // 诊断「拖动帧里图层那一块是空的」时，先确认图层位图本身有
                // 没有内容（别把「合成没画」当成「位图透明」）。
                other if other.starts_with("layer-dump:") => {
                    let body = other.trim_start_matches("layer-dump:");
                    if let Some((ix, path)) = body.split_once(':') {
                        let ix: usize = ix.trim().parse().unwrap_or(0);
                        let guard = self.session.borrow();
                        if let Some(session) = guard.as_ref() {
                            if let Some(layer) = session.doc.layers.get(ix) {
                                let ok = layer.image.save(path.trim()).is_ok();
                                println!(
                                    "{{\"phase\":\"ui\",\"event\":\"layer-dump\",\"index\":{ix},\
                                     \"ok\":{ok},\"w\":{},\"h\":{}}}",
                                    layer.image.width(),
                                    layer.image.height()
                                );
                            }
                        }
                    }
                }
                // `snap-dump:path` —— 存缩放手势起手快照（live 合成用的那张）。
                other if other.starts_with("snap-dump:") => {
                    let path = other.trim_start_matches("snap-dump:").trim().to_string();
                    let guard = self.session.borrow();
                    if let Some(session) = guard.as_ref() {
                        let snap = match &session.transform {
                            Some(crate::bridge::session::TransformSession::Scale(ls)) => {
                                Some(&ls.snapshot)
                            }
                            _ => None,
                        };
                        match snap {
                            Some(img) => {
                                let ok = img.save(&path).is_ok();
                                println!(
                                    "{{\"phase\":\"ui\",\"event\":\"snap-dump\",\"ok\":{ok},\
                                     \"w\":{},\"h\":{}}}",
                                    img.width(),
                                    img.height()
                                );
                            }
                            None => println!(
                                "{{\"phase\":\"ui\",\"event\":\"snap-dump\",\"ok\":false,\
                                 \"reason\":\"no-live-scale\"}}"
                            ),
                        }
                    }
                }
                // `rotate-drag:index,deg1,deg2,...` —— **一次** begin + 多次
                // update，模拟「按住旋柄持续旋转」。每帧都会落一张预览 BMP，
                // 供 `probe_rotate_jitter.py` 逐帧比内容中心：中心必须单调
                // （不来回跳），否则就是每帧重算转轴导致的抖动。
                other if other.starts_with("rotate-drag:") => {
                    let n = parse_nums(other);
                    if n.len() >= 2 {
                        let ix = n[0] as i32;
                        self.as_mut().rotate_layer_begin(ix, true);
                        for deg in &n[1..] {
                            self.as_mut().rotate_layer_update(*deg, true, 0);
                            println!(
                                "{{\"phase\":\"ui\",\"event\":\"rotate-drag\",\"degrees\":{deg:.2}}}"
                            );
                        }
                    } else {
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        )
                    }
                }
                // `rotate-hold:index,degrees[,expand]` —— 只 begin + update，
                // **不 finish**：把拖动中的那一帧留在屏幕上，用来截图核对
                // 「拖动中的内容位置」和松手后是否一致（以前预览用的是锚点的
                // offset，拖动时内容停在旧位置，松手才跳进框里）。
                other if other.starts_with("rotate-hold:") => {
                    let n = parse_nums(other);
                    if n.len() >= 2 {
                        let (ix, deg) = (n[0] as i32, n[1]);
                        let expand = n.get(2).copied().unwrap_or(1.0) > 0.5;
                        let base = self.as_mut().rotate_layer_begin(ix, true);
                        self.as_mut().rotate_layer_update(deg, expand, 0);
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"rotate-hold\",\"base\":{base:.2},\
                             \"degrees\":{deg:.2}}}"
                        );
                    } else {
                        println!(
                            "{{\"phase\":\"ui\",\"event\":\"script-unknown\",\"op\":\"{other}\"}}"
                        )
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
                        // P1-A：第 4 参 = 修饰键位掩码（Shift 15° 吸角）。
                        let mods = n.get(3).copied().unwrap_or(0.0) as i32;
                        self.as_mut().rotate_layer_begin(ix, true);
                        self.as_mut().rotate_layer_update(deg, expand, mods);
                        self.as_mut().rotate_layer_finish(true, deg, expand, mods);
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
