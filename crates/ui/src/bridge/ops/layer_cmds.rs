// 图层命令层（P1）：新建 / 复制 / 合并 / 拼合 / 栅格化 / 对齐 / 分布 +
// 多选与批量属性。几何与撤销都在 core（`Document::*`），这里只负责
// 「拿当前多选 → 调 core → 刷新面板与预览」。
use super::super::*;

/// 面板多选的模式（QML 传进来）：0 = 单选替换（默认分支），1 = Ctrl
/// 切换，2 = Shift 连选。
pub(in crate::bridge) const SEL_TOGGLE: i32 = 1;
pub(in crate::bridge) const SEL_RANGE: i32 = 2;
/// 画布 Ctrl+点专用：**并进**多选并设为主选中层，绝不从集合里摘掉。
///
/// 为什么不用 `SEL_TOGGLE`：画布上的 Ctrl 还兼「拖动时临时关吸附」
/// （A18-1），起手那一刻分不清用户是要多选还是要多选 + 拖。若在按下时就
/// 切换，点中的层可能被摘掉、拖动便落到别的层上。于是按下只做加法，
/// 「摘掉」留到松手且没有位移时再补（见 `CanvasSurface.onReleased`）。
pub(in crate::bridge) const SEL_ADD_PRIMARY: i32 = 3;

impl qobject::Bridge {
    /// 当前多选在图层的**下标**（升序、去重、已剔除失效条目）。
    ///
    /// 空集合时退化成「主选中层」，这样所有只有一个选中层的旧路径都还能
    /// 照旧工作 —— 多选是附加能力，不是替代品。
    pub(in crate::bridge) fn selection_indices(&self) -> Vec<usize> {
        let ids = self.selection_ids.borrow().clone();
        let guard = self.session.borrow();
        let Some(session) = guard.as_ref() else {
            return Vec::new();
        };
        let mut out: Vec<usize> = ids
            .iter()
            .filter_map(|id| session.doc.layers.iter().position(|l| l.id == *id))
            .collect();
        out.sort_unstable();
        out.dedup();
        if out.is_empty() {
            let primary = *self.selected_layer();
            if primary >= 0 && (primary as usize) < session.doc.layers.len() {
                out.push(primary as usize);
            }
        }
        out
    }

    /// 把一组下标写回多选状态（内部统一走稳定 id），并发布给 QML。
    fn set_selection_indices(mut self: Pin<&mut Self>, indices: &[usize]) {
        let ids: Vec<u32> = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => indices
                    .iter()
                    .filter_map(|&i| session.doc.layers.get(i).map(|l| l.id))
                    .collect(),
                None => Vec::new(),
            }
        };
        *self.selection_ids.borrow_mut() = ids.clone();
        self.as_mut().publish_layer_selection(&ids);
    }

    /// 发布多选属性（`layerSelectionIds` / `selectedLayerCount`）。
    pub(in crate::bridge) fn publish_layer_selection(mut self: Pin<&mut Self>, ids: &[u32]) {
        let json = ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        self.as_mut()
            .set_layer_selection_ids(QString::from(format!("[{json}]").as_str()));
        self.as_mut().set_selected_layer_count(ids.len() as i32);
    }

    /// 图层增删 / 重排之后重新校准多选（同 id 的层可能已经消失）。
    /// 让多选集合跟上文档现状：剔除已消失的图层，并且**跟住主选中层**。
    ///
    /// 后半句很关键。改主选中层的路径不止 `select_layer_ex` 一条（画布点选、
    /// 剪贴板、上移/下移、脚本 `select-layer`），它们都只写 `selected_layer`。
    /// 如果这里只做「剔除失效 id」，多选集就会停在上一轮——于是紧接着的
    /// 删除 / 对齐 / 批量显隐会作用到**上一轮选的那几层**上（删错层）。
    /// 主选中层不在集合里 = 换了人，直接把集合收敛成它自己。
    pub(in crate::bridge) fn sync_layer_selection(mut self: Pin<&mut Self>) {
        let primary = *self.selected_layer();
        let (alive, primary_id) = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                Some(session) => {
                    let alive: Vec<u32> = session.doc.layers.iter().map(|l| l.id).collect();
                    let pid = if primary >= 0 {
                        session
                            .doc
                            .layers
                            .get(primary as usize)
                            .map(|l| l.id)
                    } else {
                        None
                    };
                    (alive, pid)
                }
                None => (Vec::new(), None),
            }
        };
        let mut ids = self.selection_ids.borrow_mut();
        ids.retain(|id| alive.contains(id));
        match primary_id {
            Some(pid) if !ids.contains(&pid) => {
                ids.clear();
                ids.push(pid);
            }
            Some(_) => {}
            None => ids.clear(),
        }
        let snapshot = ids.clone();
        drop(ids);
        self.as_mut().publish_layer_selection(&snapshot);
    }

    /// 单选 / Ctrl 切换 / Shift 连选。`index < 0` = 清空选中。
    pub fn select_layer_ex(mut self: Pin<&mut Self>, index: i32, mode: i32) {
        if index < 0 {
            *self.selection_ids.borrow_mut() = Vec::new();
            self.as_mut().set_selected_layer(-1);
            self.as_mut().publish_layer_selection(&[]);
            return;
        }
        let Some(ix) = self.selected_guard(index) else {
            return;
        };
        let (len, prev) = {
            let guard = self.session.borrow();
            let len = guard.as_ref().map(|s| s.doc.layers.len()).unwrap_or(0);
            (len, *self.selected_layer())
        };
        match mode {
            SEL_TOGGLE => {
                let mut current: Vec<usize> = self.as_mut().selection_indices();
                if current.is_empty() {
                    current = vec![ix];
                } else if let Some(pos) = current.iter().position(|&i| i == ix) {
                    current.remove(pos);
                } else {
                    current.push(ix);
                }
                // 取消掉的是主选中层时，把主选中层让给集合里剩下的最后一层。
                let primary = if current.contains(&ix) { ix } else { *current.last().unwrap_or(&ix) };
                self.as_mut().set_selected_layer(primary as i32);
                self.as_mut().set_selection_indices(&current);
            }
            SEL_ADD_PRIMARY => {
                // 画布 Ctrl+点：并进集合 + 当主选中层，只加不减。
                let mut current: Vec<usize> = self.as_mut().selection_indices();
                if !current.contains(&ix) {
                    current.push(ix);
                }
                current.sort_unstable();
                current.dedup();
                self.as_mut().set_selected_layer(index);
                self.as_mut().set_selection_indices(&current);
            }
            SEL_RANGE if prev >= 0 => {
                // Shift 连选按**面板顺序**（自顶向下）取连续区间。
                let (a, b) = (prev as usize, ix);
                let (lo, hi) = (a.min(b), a.max(b));
                let range: Vec<usize> = (lo..=hi.min(len.saturating_sub(1))).collect();
                self.as_mut().set_selected_layer(index);
                self.as_mut().set_selection_indices(&range);
            }
            _ => {
                self.as_mut().set_selected_layer(index);
                self.as_mut().set_selection_indices(&[ix]);
            }
        }
        println!(
            "{{\"phase\":\"ui\",\"event\":\"layer-select\",\"index\":{ix},\"mode\":{mode},\
             \"count\":{}}}",
            *self.selected_layer_count()
        );
    }

    /// 新建空图层（插到栈顶并选中）。
    pub fn new_layer(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let result: Result<usize, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => session.doc.new_empty_layer().map_err(|e| e.to_string()),
            }
        };
        match result {
            Ok(ix) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                        s.dirty = true;
                    }
                }
                self.as_mut().set_selected_layer(ix as i32);
                self.as_mut().refresh_preview();
                self.as_mut().set_selection_indices(&[ix]);
                println!("{{\"phase\":\"ui\",\"event\":\"layer-new\",\"index\":{ix}}}");
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    /// 复制选中的图层（多选时逐个复制）。副本插到原件上方并偏移 8px。
    pub fn duplicate_selected_layer(mut self: Pin<&mut Self>) {
        let targets = self.as_mut().selection_indices();
        let viewport = *self.viewport.borrow();
        let result: Result<Vec<usize>, String> = (|| {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return Err("no document".into());
            };
            let mut made = Vec::new();
            // 从高到低复制：插在上方会让后面的下标整体上移，逆序处理
            // 才不会把下一个要复制的层指错。
            for &i in targets.iter().rev() {
                match session.doc.duplicate_layer(i) {
                    Ok(at) => made.push(at),
                    Err(e) => return Err(e.to_string()),
                }
            }
            made.reverse();
            session.dirty = true;
            Ok(made)
        })();
        match result {
            Ok(made) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                let primary = made.last().copied().unwrap_or(0);
                self.as_mut().set_selected_layer(primary as i32);
                self.as_mut().refresh_preview();
                self.as_mut().set_selection_indices(&made);
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"layer-duplicated\",\"count\":{}}}",
                    made.len()
                );
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    /// 向下合并主选中层（多选时从最上面那层开始逐个向下合并）。
    pub fn merge_selected_down(mut self: Pin<&mut Self>) {
        let targets = self.as_mut().selection_indices();
        let Some(&top) = targets.last() else {
            return;
        };
        let viewport = *self.viewport.borrow();
        let result: Result<usize, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if session.doc.layers.get(top).map(|l| l.locked).unwrap_or(true) {
                        Err("图层已锁定，先解锁再合并".into())
                    } else {
                        match session.doc.merge_down(top) {
                            Ok(at) => {
                                session.dirty = true;
                                Ok(at)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        match result {
            Ok(at) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                self.as_mut().set_selected_layer(at as i32);
                self.as_mut().refresh_preview();
                self.as_mut().set_selection_indices(&[at]);
                println!("{{\"phase\":\"ui\",\"event\":\"layer-merged-down\",\"at\":{at}}}");
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    /// 合并当前选中的多个图层（PS 的「合并图层」）。
    ///
    /// 与「向下合并」的区别：这里只认**选中的那些**，不碰相邻层；结果落在
    /// 最高选中层的 z 位，名字 / 锁定沿用最高层。
    pub fn merge_selected_layers(mut self: Pin<&mut Self>) {
        let selected = self.as_mut().selection_indices();
        let targets: Vec<usize> = selected.iter().copied().filter(|&i| i > 0).collect();
        if targets.len() < 2 {
            // ★要说清楚「为什么」：手上明明高亮着 2 行，点下去却没反应，最常见
            //   的原因就是其中一行是背景（多选集合里允许有它，但合并不吃它）。
            let message = if selected.len() > targets.len() {
                format!(
                    "背景层不参与合并：已选 {} 个图层，其中可合并的只有 {} 个，再选一个图层",
                    selected.len(),
                    targets.len()
                )
            } else {
                "合并需要选中两个以上的图层：Ctrl+点击图层行（画布上的图层也可以）多选"
                    .to_string()
            };
            self.as_mut().set_status(QString::from(message.as_str()));
            return;
        }
        let viewport = *self.viewport.borrow();
        let result: Result<usize, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    if targets
                        .iter()
                        .any(|&i| session.doc.layers.get(i).map(|l| l.locked).unwrap_or(true))
                    {
                        Err("选中的图层里有锁定层，先解锁再合并".into())
                    } else {
                        match session.doc.merge_layers(&targets) {
                            Ok(at) => {
                                session.dirty = true;
                                Ok(at)
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            }
        };
        match result {
            Ok(at) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                self.as_mut().set_selected_layer(at as i32);
                self.as_mut().refresh_preview();
                self.as_mut().set_selection_indices(&[at]);
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"layers-merged\",\"at\":{at},\"count\":{}}}",
                    targets.len()
                );
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    /// 合并所有可见图层为一层（背景层保留垫底）。
    pub fn merge_visible_layers(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let result: Result<usize, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => match session.doc.merge_visible() {
                    Ok(at) => {
                        session.dirty = true;
                        Ok(at)
                    }
                    Err(e) => Err(e.to_string()),
                },
            }
        };
        match result {
            Ok(at) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                self.as_mut().set_selected_layer(at as i32);
                self.as_mut().refresh_preview();
                self.as_mut().set_selection_indices(&[at]);
                println!("{{\"phase\":\"ui\",\"event\":\"layers-merged-visible\",\"at\":{at}}}");
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    /// 拼合图像：所有内容合进背景层。
    pub fn flatten_layers(mut self: Pin<&mut Self>) {
        let viewport = *self.viewport.borrow();
        let result: Result<(), String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => session.doc.flatten_stack().map_err(|e| e.to_string()).map(|()| {
                    session.dirty = true;
                }),
            }
        };
        match result {
            Ok(()) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                self.as_mut().set_selected_layer(-1);
                self.as_mut().refresh_preview();
                self.as_mut().set_selection_indices(&[]);
                println!("{{\"phase\":\"ui\",\"event\":\"layers-flattened\"}}");
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    /// 把选中的标记图层栅格化成位图（多选时逐个处理）。
    pub fn rasterize_selected_mark(mut self: Pin<&mut Self>) {
        let targets = self.as_mut().selection_indices();
        let viewport = *self.viewport.borrow();
        let result: Result<usize, String> = (|| {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return Err("no document".into());
            };
            let mut done = 0usize;
            for &i in targets.iter().rev() {
                if session.doc.layers.get(i).map(|l| l.is_mark()).unwrap_or(false) {
                    match session.doc.rasterize_mark(i) {
                        Ok(_) => done += 1,
                        Err(e) => return Err(e.to_string()),
                    }
                }
            }
            if done == 0 {
                Err("选中的不是标记图层".into())
            } else {
                session.dirty = true;
                Ok(done)
            }
        })();
        match result {
            Ok(done) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                self.as_mut().refresh_preview();
                self.as_mut().sync_layer_selection();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"mark-rasterized\",\"count\":{done}}}"
                );
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }


    /// 图层缩略图的 file:// URL（面板每行一张）。
    ///
    /// 走「按需生成 + 缓存」而不是塞进 `layers_json`：`layers_json` 每次
    /// refresh 都会重建（拖动帧也是），在里面缩图会把拖动拖死。这一版按
    /// (会话, 图层 id, 文档 epoch) 缓存，只有真的改了像素才重出一张，并且
    /// 顺手把同一层的旧文件删掉，临时目录不会越堆越多。
    pub fn layer_thumb_url(self: Pin<&mut Self>, index: i32) -> QString {
        const THUMB_EDGE: u32 = 40;
        let (sid, lid, epoch) = {
            let guard = self.session.borrow();
            let Some(session) = guard.as_ref() else {
                return QString::default();
            };
            let Some(layer) = session.doc.layers.get(index.max(0) as usize) else {
                return QString::default();
            };
            (session.id, layer.id, session.doc.edit_epoch)
        };
        let key = (sid, lid);
        if let Some((cached_epoch, url, _)) = self.thumb_cache.borrow().get(&key) {
            if *cached_epoch == epoch {
                return QString::from(url.as_str());
            }
        }
        let thumb = {
            let guard = self.session.borrow();
            let Some(session) = guard.as_ref() else {
                return QString::default();
            };
            let Some(layer) = session.doc.layers.get(index.max(0) as usize) else {
                return QString::default();
            };
            layer.thumbnail(THUMB_EDGE)
        };
        let path = std::env::temp_dir().join(format!(
            "imageedit-thumb-{sid}-{lid}-{epoch}.png"
        ));
        if thumb.save(&path).is_err() {
            return QString::default();
        }
        let url = file_url(&path);
        {
            let mut cache = self.thumb_cache.borrow_mut();
            if let Some((_, _, old_path)) = cache.get(&key) {
                if old_path != &path {
                    let _ = std::fs::remove_file(old_path);
                }
            }
            cache.insert(key, (epoch, url.clone(), path));
            // 图层多 + 反复编辑时别让缓存无限长（正常情况下就只有几十条）。
            if cache.len() > 512 {
                cache.clear();
            }
        }
        QString::from(url.as_str())
    }

    /// 对齐：参照框 = 选区（有的话）否则整幅画布。
    pub fn align_selected_layers(mut self: Pin<&mut Self>, edge: &QString) {
        let Some(edge) = editcore::AlignEdge::from_token(edge.to_string().as_str()) else {
            self.as_mut().set_status(QString::from("未知的对齐方式"));
            return;
        };
        self.as_mut().align_or_distribute(|doc, targets, reference| {
            doc.align_layers(&targets, edge, reference)
        });
    }

    /// 分布：首尾不动，中间按中心等距铺开。
    pub fn distribute_selected_layers(mut self: Pin<&mut Self>, axis: &QString) {
        let Some(axis) = editcore::DistributeAxis::from_token(axis.to_string().as_str()) else {
            self.as_mut().set_status(QString::from("未知的分布方向"));
            return;
        };
        self.as_mut().align_or_distribute(|doc, targets, _reference| {
            doc.distribute_layers(&targets, axis)
        });
    }

    /// 对齐 / 分布共用的执行 + 刷新外壳。
    fn align_or_distribute<F>(mut self: Pin<&mut Self>, run: F)
    where
        F: FnOnce(&mut editcore::Document, Vec<usize>, Rect) -> Result<(), editcore::EditError>,
    {
        let targets = self.as_mut().selection_indices();
        let viewport = *self.viewport.borrow();
        let result: Result<usize, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    // 参照框的取法（用户定的语义）：
                    //  · 选中两层以上 → 用**这些图层自己的并集包围盒**，也就是
                    //    「谁最靠边谁不动，其余靠过去」。以前一律按画布对齐，
                    //    两层挨在一起时会各自被推到画布边缘，完全不是想要的。
                    //  · 只有一层 → 沿用旧语义（有选区按选区，否则整幅画布）。
                    let reference = if targets.len() >= 2 {
                        union_bounds(&session.doc, &targets).unwrap_or_else(|| {
                            Rect::new(0, 0, session.doc.canvas.width, session.doc.canvas.height)
                        })
                    } else {
                        session
                            .doc
                            .selection
                            .as_ref()
                            .map(|s| s.base.bounds)
                            .filter(|b| b.width > 0 && b.height > 0)
                            .unwrap_or_else(|| {
                                Rect::new(0, 0, session.doc.canvas.width, session.doc.canvas.height)
                            })
                    };
                    match run(&mut session.doc, targets.clone(), reference) {
                        Ok(()) => {
                            session.dirty = true;
                            Ok(targets.len())
                        }
                        Err(e) => Err(e.to_string()),
                    }
                }
            }
        };
        match result {
            Ok(n) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                self.as_mut().refresh_preview();
                self.as_mut().sync_layer_selection();
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"layer-align\",\"count\":{n}}}"
                );
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    /// 批量显示 / 隐藏（一条历史）。
    pub fn set_selected_layers_visible(mut self: Pin<&mut Self>, visible: bool) {
        let targets = self.as_mut().selection_indices();
        let viewport = *self.viewport.borrow();
        let result: Result<(), String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let mut ops = Vec::new();
                    for &i in &targets {
                        let Some(layer) = session.doc.layers.get(i) else {
                            continue;
                        };
                        if layer.visible != visible {
                            ops.push(editcore::EditOp::SetVisible {
                                index: i,
                                from: layer.visible,
                                to: visible,
                            });
                        }
                    }
                    if ops.is_empty() {
                        Ok(())
                    } else {
                        session
                            .doc
                            .execute(editcore::EditOp::Batch(ops))
                            .map_err(|e| e.to_string())
                            .map(|()| {
                                session.dirty = true;
                            })
                    }
                }
            }
        };
        match result {
            Ok(()) => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                self.as_mut().refresh_preview();
            }
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }

    /// 删除选中的所有图层（多选时一次删完，一条历史）。
    pub fn delete_selected_layers(mut self: Pin<&mut Self>) {
        let targets = self.as_mut().selection_indices();
        let viewport = *self.viewport.borrow();
        let result: Result<usize, String> = {
            let mut guard = self.session.borrow_mut();
            match guard.as_mut() {
                None => Err("no document".into()),
                Some(session) => {
                    let mut ops = Vec::new();
                    // 从高到低删，下标才不会在删除过程中错位。
                    for &i in targets.iter().rev() {
                        if i == 0 {
                            continue; // 背景层不支持删除，只能清空像素
                        }
                        let Some(layer) = session.doc.layers.get(i) else {
                            continue;
                        };
                        if layer.locked {
                            continue;
                        }
                        ops.push(editcore::EditOp::RemoveLayer {
                            index: i,
                            layer: layer.clone(),
                        });
                    }
                    if ops.is_empty() {
                        Ok(0)
                    } else {
                        let n = ops.len();
                        session
                            .doc
                            .execute(editcore::EditOp::Batch(ops))
                            .map_err(|e| e.to_string())
                            .map(|()| {
                                session.dirty = true;
                                n
                            })
                    }
                }
            }
        };
        match result {
            Ok(n) if n > 0 => {
                {
                    let mut guard = self.session.borrow_mut();
                    if let Some(s) = guard.as_mut() {
                        s.refit_if_needed(viewport);
                    }
                }
                self.as_mut().set_selected_layer(-1);
                self.as_mut().refresh_preview();
                self.as_mut().set_selection_indices(&[]);
                println!("{{\"phase\":\"ui\",\"event\":\"layers-deleted\",\"count\":{n}}}");
            }
            Ok(_) => {}
            Err(message) => self.as_mut().set_status(QString::from(message.as_str())),
        }
    }
}

/// 一组图层在画布坐标里的并集包围盒（`None` = 没有一层的面积大于 0）。
///
/// 对齐/分布没有用 `Document` 的方法来做，因为这里只需要「读」几何：目标层
/// 可能被锁定（锁定层不动，但它的位置仍是有意义的参照）。
fn union_bounds(doc: &editcore::Document, indices: &[usize]) -> Option<Rect> {
    let (mut x0, mut y0) = (i32::MAX, i32::MAX);
    let (mut x1, mut y1) = (i32::MIN, i32::MIN);
    let mut any = false;
    for &i in indices {
        let Some(layer) = doc.layers.get(i) else {
            continue;
        };
        let b = layer.bounds();
        if b.width == 0 || b.height == 0 {
            continue;
        }
        any = true;
        x0 = x0.min(b.x);
        y0 = y0.min(b.y);
        x1 = x1.max(b.x + b.width as i32);
        y1 = y1.max(b.y + b.height as i32);
    }
    any.then(|| Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
}
