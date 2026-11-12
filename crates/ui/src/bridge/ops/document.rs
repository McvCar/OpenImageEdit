// 载入 / 解码 / 保存 / 文件信息。
use super::super::*;

impl qobject::Bridge {
    pub fn image_shown(mut self: Pin<&mut Self>) {
        let total = elapsed_ms();
        self.as_mut().set_shown_ms(total);
        println!(
            "{{\"phase\":\"ui\",\"decode_ms\":{},\"shown_ms\":{total},\"width\":{},\"height\":{},\"zoom\":{},\"status\":\"{}\"}}",
            self.decode_ms(),
            self.image_width(),
            self.image_height(),
            self.zoom_percent(),
            self.status(),
        );
    }

    pub fn mark(self: Pin<&mut Self>, name: &QString) {
        println!(
            "{{\"phase\":\"ui\",\"event\":\"mark\",\"name\":\"{}\",\"ms\":{}}}",
            name,
            elapsed_ms()
        );
    }

    pub fn open_file(mut self: Pin<&mut Self>, url: &QUrl) {
        let path = PathBuf::from(url.to_local_file_or_default().to_string());
        self.as_mut().load_path(&path);
    }

    pub fn navigate(mut self: Pin<&mut Self>, dir: i32) {
        let current = self
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.source.clone());
        let Some(current) = current else {
            self.as_mut()
                .set_status(QString::from("尚未打开文件，无法切换"));
            return;
        };
        let Some(parent) = current.parent().map(|p| p.to_path_buf()) else {
            return;
        };
        let is_image = |p: &Path| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| {
                    matches!(
                        e.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff"
                    )
                })
                .unwrap_or(false)
        };
        let mut siblings: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && is_image(&path) {
                    siblings.push(path);
                }
            }
        }
        siblings.sort_by_key(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        });
        if siblings.len() < 2 {
            self.as_mut()
                .set_status(QString::from("当前目录没有其他图片"));
            return;
        }
        let cur_key = current
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let idx = siblings
            .iter()
            .position(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default()
                    == cur_key
            })
            .unwrap_or(0);
        let len = siblings.len() as i32;
        let next = (idx as i32 + dir).rem_euclid(len) as usize;
        self.as_mut().load_path(&siblings[next]);
    }

    pub fn export_layer_png(mut self: Pin<&mut Self>, index: i32) {
        let result: Result<PathBuf, String> = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                None => Err("没有可导出的图层".into()),
                Some(session) => {
                    if index < 0 || index as usize >= session.doc.layers.len() {
                        Err("图层不存在".into())
                    } else {
                        let layer = &session.doc.layers[index as usize];
                        let dir = match &session.source {
                            Some(src) => match src.parent() {
                                Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
                                _ => std::path::PathBuf::from("."),
                            },
                            // No source file: export into the default save dir.
                            None => match session.default_save_path().0.parent() {
                                Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
                                _ => std::path::PathBuf::from("."),
                            },
                        };
                        // Number: numeric name wins, else stack position (1-based).
                        let number = layer
                            .name
                            .parse::<u32>()
                            .ok()
                            .unwrap_or((index + 1) as u32);
                        let mut path = dir.join(format!("layout_{}.png", number));
                        let mut suffix = 2u32;
                        while path.exists() {
                            path = dir.join(format!("layout_{}_{}.png", number, suffix));
                            suffix += 1;
                        }
                        let write_result = encode(&layer.img(), Format::Png, session.save_quality)
                            .map_err(|e| e.to_string())
                            .and_then(|bytes| {
                                std::fs::write(&path, bytes).map_err(|e| e.to_string())
                            })
                            .map(|()| path);
                        write_result
                    }
                }
            }
        };
        match result {
            Ok(path) => {
                self.as_mut().set_status(QString::from(format!(
                    "已导出图层：{}",
                    path.to_string_lossy()
                )));
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"layer-exported\",\"path\":\"{}\"}}",
                    path.to_string_lossy().replace('\\', "/")
                );
            }
            Err(message) => {
                self.as_mut().set_status(QString::from(message.as_str()));
            }
        }
    }

    /// Ctrl+S: overwrite the source file. Encoding is dispatched to a worker
    /// thread, so a 12MP / 10-layer PSD never freezes the window (C5).
    pub fn save_edited(mut self: Pin<&mut Self>) {
        let source = {
            let guard = self.session.borrow();
            match guard.as_ref().and_then(|s| s.source.clone()) {
                Some(p) => p,
                None => {
                    drop(guard);
                    self.as_mut().report_save(Err("没有可保存的文件".into()));
                    return;
                }
            }
        };
        self.as_mut().begin_save(source, false);
    }

    /// Save-As (hand-typed `.psd` goes through the very same explicit
    /// `SaveFormat` dispatch as Ctrl+S — no `unwrap_or(Png)` anywhere).
    pub fn save_as(mut self: Pin<&mut Self>, url: &QUrl) {
        let path = PathBuf::from(url.to_local_file_or_default().to_string());
        self.as_mut().begin_save(path, true);
    }

    /// Validate + snapshot + spawn the encode worker. `report_save` is only
    /// called later from `poll_save`, so the "saved" event always means the
    /// bytes really hit the disk.
    pub(in crate::bridge) fn begin_save(mut self: Pin<&mut Self>, path: PathBuf, update_source: bool) {
        if *self.saving() {
            self.as_mut()
                .set_status(QString::from("上一次保存尚未完成，请稍候"));
            return;
        }
        if *self.editing_locked() {
            self.as_mut().psd_locked_hint();
            return;
        }
        let prepared = {
            let guard = self.session.borrow();
            let Some(session) = guard.as_ref() else {
                drop(guard);
                self.as_mut().report_save(Err("没有可保存的文档".into()));
                return;
            };
            match session.save_target(&path) {
                Ok(format) => {
                    // 校准生效时位图导出必须给出「所见」的 PS 合并图，而不是
                    // 按层重建的结果（§17）。PSD 写回例外：它要保留完整的
                    // 图层栈，用 1 层合并图会毁掉原文件。
                    let snapshot = if !matches!(format, SaveFormat::Psd)
                        && session.calibration_active()
                    {
                        session
                            .calibration
                            .as_ref()
                            .map(|c| c.reference.clone())
                            .unwrap_or_else(|| session.doc.clone())
                    } else {
                        session.doc.clone()
                    };
                    (
                        snapshot,
                        session.doc.edit_epoch,
                        session.id,
                        session.save_quality,
                        format,
                    )
                }
                Err(err) => {
                    drop(guard);
                    self.as_mut().report_save(Err(err));
                    return;
                }
            }
        };
        let (doc, epoch, session_id, quality, format) = prepared;
        let rx = spawn_save_job(&doc, format, quality);
        *self.save_job.borrow_mut() = Some(SaveJob {
            path,
            format,
            epoch,
            session_id,
            update_source,
            rx,
        });
        self.as_mut().set_saving(true);
        self.as_mut()
            .set_status(QString::from("正在后台编码并保存…"));
        println!(
            "{{\"phase\":\"ui\",\"event\":\"save-started\",\"is_psd\":{},\"ms\":{}}}",
            matches!(format, SaveFormat::Psd),
            elapsed_ms()
        );
    }

    /// Drop an in-flight save when the document is replaced (finally-style:
    /// no stale `source` / `dirty` can ever land on the NEW session).
    pub(in crate::bridge) fn cancel_save(mut self: Pin<&mut Self>, reason: &str) {
        if self.save_job.borrow().is_none() {
            return;
        }
        *self.save_job.borrow_mut() = None;
        self.as_mut().set_saving(false);
        println!(
            "{{\"phase\":\"ui\",\"event\":\"save-cancelled\",\"reason\":\"{}\",\"ms\":{}}}",
            reason,
            elapsed_ms()
        );
    }

    /// Drain the background encode job on the 50 ms timer. The atomic write
    /// and the state update stay on the UI thread and are both guarded by the
    /// captured edit epoch / session id.
    pub fn poll_save(mut self: Pin<&mut Self>) {
        let next = {
            let job = self.save_job.borrow();
            let Some(j) = job.as_ref() else {
                return;
            };
            j.rx.try_recv()
        };
        let msg = match next {
            Ok(m) => m,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.as_mut().cancel_save("channel-closed");
                self.as_mut()
                    .report_save(Err("保存线程意外结束，原文件未被修改".into()));
                return;
            }
        };
        let Some(job) = self.save_job.borrow_mut().take() else {
            return;
        };
        self.as_mut().set_saving(false);
        if msg.is_psd {
            println!(
                "{{\"phase\":\"ui\",\"event\":\"psd-encode-ms\",\"value\":{}}}",
                msg.encode_ms
            );
        }
        let bytes = match msg.bytes {
            Ok(v) => v,
            Err(err) => {
                self.as_mut().report_save(Err(err));
                return;
            }
        };
        if let Err(err) = atomic_write(&job.path, &bytes) {
            self.as_mut().report_save(Err(err));
            return;
        }
        // Only clear `dirty` when the document did not change while the
        // encode was running, and only touch the session that started it.
        let mut still_current = false;
        if let Some(session) = self.session.borrow_mut().as_mut() {
            if session.id == job.session_id {
                still_current = session.doc.edit_epoch == job.epoch;
                if still_current {
                    session.dirty = false;
                }
                if job.update_source {
                    session.source = Some(job.path.clone());
                }
                if matches!(job.format, SaveFormat::Psd) {
                    // The file now represents this document's layer stack.
                    session.psd_layered = true;
                }
            }
        }
        if still_current {
            self.as_mut().set_dirty(false);
        }
        self.as_mut().update_file_info();
        // PSD 写出的「层数 / 分组数」要如实报出来：分组只是本编辑器的元
        // 数据，PSD 里会被展开成平级图层 —— 不提示的话，用户会以为是丢了。
        let psd_report = if matches!(job.format, SaveFormat::Psd) {
            let (layers, groups) = self
                .session
                .borrow()
                .as_ref()
                .map(|s| (s.doc.layers.len(), s.doc.groups.len()))
                .unwrap_or((0, 0));
            println!(
                "{{\"phase\":\"ui\",\"event\":\"psd-saved\",\"path\":\"{}\",\"layers\":{},\"groups\":{}}}",
                job.path.display().to_string().replace('\\', "/"),
                layers,
                groups
            );
            Some((layers, groups))
        } else {
            None
        };
        self.as_mut().report_save(Ok(job.path.clone()));
        if let Some((layers, groups)) = psd_report {
            let text = if groups > 0 {
                format!(
                    "已保存 PSD：{}（{layers} 个图层；{groups} 个分组已展开，不写入 PSD）",
                    job.path.display()
                )
            } else {
                format!(
                    "已保存 PSD：{}（{layers} 个图层）",
                    job.path.display()
                )
            };
            self.as_mut().set_status(QString::from(text.as_str()));
        }
    }

    /// Drop queued thumbnails when the document is replaced (C2-c style
    /// finally cleanup): no stale session ids can ever land in the cache.
    pub(in crate::bridge) fn cancel_thumbs(mut self: Pin<&mut Self>, reason: &str) {
        let left = self.thumb_queue.borrow_mut().drain(..).count();
        if left > 0 {
            println!(
                "{{\"phase\":\"ui\",\"event\":\"thumbs-cancelled\",\"reason\":\"{}\",\"left\":{}}}",
                reason, left
            );
        }
        self.as_mut().set_thumbs_pending(false);
    }

    /// C6: write at most 6 pre-rendered 40px thumbnails per UI tick. The
    /// expensive `imageops::thumbnail` already ran on the stage-2 worker;
    /// this loop is only ~8 tiny PNG encodes + HashMap inserts.
    pub fn poll_thumbs(mut self: Pin<&mut Self>) {
        // Doc allows <=8; 6 keeps a comfortable margin under the 16 ms
        // single-UI-occupancy gate even on a loaded machine.
        const BATCH: usize = 6;
        let batch: Vec<(u32, u32, u64, RgbaImage)> = {
            let mut q = self.thumb_queue.borrow_mut();
            (0..BATCH).filter_map(|_| q.pop_front()).collect()
        };
        if batch.is_empty() {
            self.as_mut().set_thumbs_pending(false);
            return;
        }
        let t0 = Instant::now();
        {
            let mut cache = self.thumb_cache.borrow_mut();
            for (sid, lid, epoch, img) in batch {
                let path = std::env::temp_dir().join(format!(
                    "imageedit-thumb-{sid}-{lid}-{epoch}.png"
                ));
                if img.save(&path).is_err() {
                    continue;
                }
                let key = (sid, lid);
                if let Some((_, _, old)) = cache.get(&key) {
                    if old != &path {
                        let _ = std::fs::remove_file(old);
                    }
                }
                cache.insert(key, (epoch, file_url(&path), path));
            }
        }
        let epoch = self.thumb_epoch().wrapping_add(1);
        self.as_mut().set_thumb_epoch(epoch);
        let empty = self.thumb_queue.borrow().is_empty();
        self.as_mut().set_thumbs_pending(!empty);
        println!(
            "{{\"phase\":\"ui\",\"event\":\"layer-thumb-batch-ms\",\"value\":{},\"remaining\":{}}}",
            t0.elapsed().as_millis(),
            self.thumb_queue.borrow().len()
        );
    }

    pub fn clear_psd_notice(mut self: Pin<&mut Self>) {
        self.as_mut().set_psd_notice(QString::from(""));
    }

    /// The user accepted "仍然覆盖" in the lossy-PSD save warning. Keep the
    /// acceptance for the rest of this document (a fresh load resets it).
    pub fn ack_psd_write_loss(mut self: Pin<&mut Self>) {
        self.as_mut().set_psd_lossy_ack(true);
        println!("{{\"phase\":\"ui\",\"event\":\"psd-lossy-save-accepted\"}}");
    }

    pub fn poll_decode(mut self: Pin<&mut Self>) {
        // 拿到结果（或上一轮就已按住的）之后，还要等 QML 报上 viewport。
        // 预热会让解码在布局之前就完成：那时 viewport 还是 0×0，直接套用会
        // 按 fit_zoom=1.0 打开 —— 白渲染一张 8160×6120 的预览（实测多花
        // 430ms、写 200MB 临时 BMP），然后窗口贴合图片时又全片重渲染一次。
        // 所以结果先存在 pending_decode 里，viewport 真值到了再落地；
        // 最多等 2s（40 次轮询），窗口极小/最小化时也不会永远挂着。
        if self.pending_decode.borrow().is_none() {
            let received = {
                let mut job = self.decode_job.borrow_mut();
                let Some(rx) = job.as_ref() else {
                    return;
                };
                match rx.try_recv() {
                    Ok(msg) => {
                        *job = None;
                        Some(msg)
                    }
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        *job = None;
                        return;
                    }
                }
            };
            let Some(msg) = received else {
                return;
            };
            *self.pending_decode.borrow_mut() = Some((msg, 0));
        }
        let viewport = *self.viewport.borrow();
        let has_viewport = viewport.width > 0 && viewport.height > 0;
        let waited = self
            .pending_decode
            .borrow()
            .as_ref()
            .map_or(0, |(_, n)| *n);
        // A failed decode has no geometry to fit, so surface it right away.
        let failed = self
            .pending_decode
            .borrow()
            .as_ref()
            .is_some_and(|(msg, _)| msg.result.is_err());
        if !has_viewport && !failed && waited < 40 {
            if let Some((_, n)) = self.pending_decode.borrow_mut().as_mut() {
                *n = waited + 1;
            }
            return;
        }
        let Some((msg, _)) = self.pending_decode.borrow_mut().take() else {
            return;
        };
        // A PSD stage-2 job travels with the stage-1 message. Take it BEFORE
        // consuming `msg.result` (both are partial moves of the same value).
        let layer_job = msg.layer_job;
        self.as_mut().set_loading(false);
        match msg.result {
            Ok((decoded, decode_ms)) => {
                let Decoded {
                    info,
                    downsampled_from,
                    ..
                } = decoded;
                let (session, _) = Session::from_decoded(
                    &msg.path,
                    decoded,
                    viewport,
                    (*self.save_quality()).clamp(1, 100) as u8,
                    self.dpr.get(),
                );
                self.as_mut().set_load_failed(false);
                self.as_mut().set_decode_ms(decode_ms);
                if is_psd_path(&msg.path) {
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"psd-stage1-ms\",\"value\":{}}}",
                        decode_ms
                    );
                }
                *self.session.borrow_mut() = Some(session);
                // 保真模式：阶段 1 解出来的就是 PSD 自带合并图。只要这次没有
                // 顺带跑阶段 2（旧探针钩子除外），画面就保持逐像素一致，
                // 也不启动图层解析。「进入分层编辑」由用户显式触发。
                let psd = is_psd_path(&msg.path);
                let fidelity = psd && layer_job.is_none() && downsampled_from.is_none();
                self.as_mut().set_fidelity_mode(fidelity);
                // 只有「以合并图打开」的文档才缓存了可换回的另一半；旧探针的
                // 自动装配路径没有保真文档，不能开放切换入口。
                self.as_mut().set_has_psd_layers(fidelity);
                if fidelity {
                    if let Some(session) = self.session.borrow_mut().as_mut() {
                        session.note = Some(
                            "保真显示：PSD 自带合并图（与 Photoshop 一致）· 视图菜单可进入分层编辑"
                                .to_string(),
                        );
                    }
                }
                println!(
                    "{{\"phase\":\"ui\",\"event\":\"fidelity-state\",\"value\":{},\"canLayer\":{}}}",
                    fidelity,
                    fidelity
                );
                // 诊断：旧 Session 已在这一行被替换掉，读数里若没有回落，
                // 说明有东西替旧文档继续持有那批图层像素。
                mem::log("after-session");
                self.as_mut().update_file_info();
                self.as_mut().refresh_preview();
                // LAST: the QML window-fit handler reads `image_width` /
                // `display_*`, which refresh_preview just published.
                self.as_mut().bump_load_serial();
                if let Some((w, h)) = downsampled_from {
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"downsampled\",\"from\":\"{}x{}\",\"to\":\"{}x{}\"}}",
                        w, h, info.width, info.height
                    );
                    // C8: read-only downsampled PSD gets a visible explanation
                    // instead of a silently disabled Ctrl+S.
                    if is_psd_path(&msg.path) {
                        self.as_mut().set_psd_notice(QString::from(format!(
                            "此 PSD 以降采样预览打开（原图 {w}×{h}，当前 {}×{}）。\n为避免写回时丢失分辨率，本文件为只读：Ctrl+S 与另存为 PSD 均不可用，可另存为 PNG / JPEG。",
                            info.width, info.height
                        )));
                    }
                }
                // The first frame is on screen. Stage 2 is still running, so
                // lock EVERY editing tool until it installs the layer stack
                // (user decision 2026-11-12). Viewing / zoom / pan / open are
                // not gated by this flag.
                if let Some(job) = layer_job {
                    *self.psd_job.borrow_mut() = Some(job);
                    self.as_mut().set_psd_layers_pending(true);
                    self.as_mut().set_editing_locked(true);
                    self.as_mut().set_psd_progress(QString::from(
                        "正在加载图层…（完成前编辑工具暂不可用）",
                    ));
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"editing-locked\",\"ms\":{}}}",
                        elapsed_ms()
                    );
                } else {
                    // Non-PSD (or a PSD opened without layering): no lock may
                    // survive from an earlier document.
                    self.as_mut().finish_psd_layers("composite-only");
                }
                // 自动化钩子：让保真 ↔ 分层的切换也能在无人工点击的探针里跑。
                // 值为 enter 时停在分层态，roundtrip 时装配完立即换回保真态。
                if let Ok(want) = std::env::var("IMAGEEDIT_FIDELITY_PROBE") {
                    if want == "enter" || want == "roundtrip" || want == "roundtrip2" {
                        self.as_mut().enter_layered_editing();
                    }
                }
            }
            Err(err) => {
                self.as_mut().finish_psd_layers("decode-failed");
                *self.session.borrow_mut() = None;
                eprintln!("{{\"phase\":\"ui\",\"error\":\"decode-failed\",\"detail\":\"{err}\"}}");
                let status = format!("decode-error: {err}");
                self.as_mut().set_status(QString::from(status.as_str()));
                self.as_mut().set_preview_url(QString::from(""));
                self.as_mut().set_load_failed(true);
                self.as_mut().update_file_info();
            }
        }
    }

    /// finally-style unlock shared by EVERY stage-2 exit path (success,
    /// failure, skip, switch file, handoff, channel closed). Dropping the
    /// receiver discards the superseded job without touching Qt state.
    pub(in crate::bridge) fn finish_psd_layers(mut self: Pin<&mut Self>, reason: &str) {
        *self.psd_job.borrow_mut() = None;
        self.as_mut().set_psd_layers_pending(false);
        if *self.editing_locked() {
            self.as_mut().set_editing_locked(false);
            println!(
                "{{\"phase\":\"ui\",\"event\":\"editing-unlocked\",\"reason\":\"{}\",\"ms\":{}}}",
                reason,
                elapsed_ms()
            );
        }
        self.as_mut().set_psd_progress(QString::from(""));
    }

    /// User escape hatch: stop waiting for the layers and edit the composite
    /// preview right away (C2-b).
    pub fn skip_psd_layers(mut self: Pin<&mut Self>) {
        if !*self.psd_layers_pending() {
            return;
        }
        if *self.fidelity_mode() {
            // 保真模式下阶段 2 是用户主动发起的「进入分层编辑」，跳过 =
            // 取消这次切换，画面本来就是合并图，谈不上降级或只读。
            self.as_mut().set_status(QString::from(
                "已取消进入分层编辑，保持保真显示（PS 合并图）",
            ));
            self.as_mut().finish_psd_layers("layered-cancelled");
            return;
        }
        self.as_mut().set_status(QString::from(
            "已跳过图层加载，以合并预览继续编辑；写回 PSD 仍不可用",
        ));
        self.as_mut().set_psd_notice(QString::from(
            "已跳过图层加载：当前只有合并预览，编辑后可另存为 PNG / JPEG。\n为避免破坏原文件的图层，写回 PSD 暂时不可用；重新打开并等待加载完成即可恢复。",
        ));
        self.as_mut().finish_psd_layers("skipped");
    }

    /// One-shot hint when an edit is attempted while the lock is held.
    pub fn psd_locked_hint(mut self: Pin<&mut Self>) {
        self.as_mut().set_status(QString::from(
            "图层仍在后台加载，编辑工具暂不可用（可点「跳过图层加载」立即编辑）",
        ));
    }

    /// 保真 → 分层。已经解析过就直接把缓存的图层栈换回来（毫秒级、不重读
    /// 文件）；第一次才按需启动阶段 2，解析期间沿用原有 editingLocked 闸门。
    pub fn enter_layered_editing(mut self: Pin<&mut Self>) {
        if !*self.has_psd_layers() || !*self.fidelity_mode() {
            return;
        }
        let cached = self
            .inactive_doc
            .borrow()
            .as_ref()
            .is_some_and(|d| d.psd_layered);
        if cached {
            self.as_mut().swap_display_docs();
            self.as_mut().publish_mode_switch(false);
            println!("{{\"phase\":\"ui\",\"event\":\"fidelity-enter\",\"mode\":\"cache\"}}");
            return;
        }
        // 只有「需要重新解析」的路径才怕并发作业；缓存换回不需要等作业收尾。
        if *self.psd_layers_pending() {
            return;
        }
        let Some(path) = self
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.source.clone())
        else {
            return;
        };
        let opts = self.decode_opts();
        let Some(job) = spawn_psd_layer_job(&path, opts) else {
            self.as_mut()
                .set_status(QString::from("当前文件不是可分层的 PSD"));
            return;
        };
        *self.psd_job.borrow_mut() = Some(job);
        self.as_mut().set_psd_layers_pending(true);
        self.as_mut().set_editing_locked(true);
        self.as_mut().set_psd_progress(QString::from(
            "正在加载图层…（完成前编辑工具暂不可用）",
        ));
        self.as_mut()
            .set_status(QString::from("正在进入分层编辑…"));
        println!(
            "{{\"phase\":\"ui\",\"event\":\"editing-locked\",\"ms\":{}}}",
            elapsed_ms()
        );
        println!("{{\"phase\":\"ui\",\"event\":\"fidelity-enter\",\"mode\":\"parse\"}}");
    }

    /// 分层 → 保真。分层栈（连同它的撤销记录）原样缓存，再切回来时不用重解析。
    pub fn return_to_fidelity(mut self: Pin<&mut Self>) {
        // 注意：装配刚完成时 finish_psd_layers("done") 还没跑，此刻
        // psd_layers_pending 仍为 true。菜单本身已用 !psdLayersPending 把关，
        // 这里不能再拿它当守卫，否则阶段 2 完成当帧的往返切换会被吞掉。
        if !*self.has_psd_layers() || *self.fidelity_mode() {
            return;
        }
        self.as_mut().swap_display_docs();
        self.as_mut().publish_mode_switch(true);
        // 探针：换回保真后再进一次分层，验证缓存路径（不重读文件、不重解析）。
        if std::env::var("IMAGEEDIT_FIDELITY_PROBE").ok().as_deref() == Some("roundtrip2") {
            self.as_mut().enter_layered_editing();
        }
    }

    /// 「视图 → PS 原合成校准」开关。返回切换后校准是否生效。
    pub fn toggle_calibrated_preview(mut self: Pin<&mut Self>) -> bool {
        let currently_on = self
            .session
            .borrow()
            .as_ref()
            .is_some_and(|s| s.calibration.as_ref().is_some_and(|c| c.enabled));
        let next = !currently_on;
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.set_calibration_enabled(next);
        }
        let active = self
            .session
            .borrow()
            .as_ref()
            .is_some_and(|s| s.calibration_active());
        self.as_mut().set_calibrated_preview(active);
        self.as_mut().refresh_preview();
        self.as_mut().set_status(QString::from(if !next {
            "已关闭原合成校准：当前显示按层重建的画面"
        } else if active {
            "已开启原合成校准：当前显示 PSD 自带合并图"
        } else {
            "图层栈已被修改，无法恢复校准；请切回保真显示查看原图"
        }));
        println!(
            "{{\"phase\":\"ui\",\"event\":\"calibration-toggle\",\"enabled\":{},\"active\":{}}}",
            next, active
        );
        active
    }

    /// 把当前显示的那份 Document 与缓存的另一份互换。History 在 Document
    /// 内部，所以两边的 Ctrl+Z 各自独立，切换不丢编辑、不串历史。
    fn swap_display_docs(mut self: Pin<&mut Self>) {
        let Some(stored) = self.inactive_doc.borrow_mut().take() else {
            self.as_mut()
                .set_status(QString::from("另一份显示文档不可用，请重新打开此文件"));
            return;
        };
        let mut guard = self.session.borrow_mut();
        let Some(session) = guard.as_mut() else {
            *self.inactive_doc.borrow_mut() = Some(stored);
            return;
        };
        let to_layered = stored.psd_layered;
        let old = std::mem::replace(&mut session.doc, stored.doc);
        // 切回分层时，刚被换出去的那份就是保真态的合并图；重新挂成显示
        // 基准（install_calibration 会检查分层栈是否仍处于原始状态）。
        let reference = if to_layered { Some(old.clone()) } else { None };
        let old_state = InactiveDoc {
            doc: old,
            dirty: session.dirty,
            note: session.note.clone(),
            psd_layered: session.psd_layered,
            psd_lossy: session.psd_lossy,
        };
        *self.inactive_doc.borrow_mut() = Some(old_state);
        session.dirty = stored.dirty;
        session.note = stored.note.clone();
        session.psd_layered = stored.psd_layered;
        session.psd_lossy = stored.psd_lossy;
        // 合成缓存 / GPU 快路径都按旧文档的 epoch 建立，换文档后必须整体
        // 失效，否则 gpuLive 会永久盖住旧纹理（历史踩过的坑）。
        session.composer = render::PreviewComposer::new();
        session.last_render = None;
        session.transform = None;
        session.sel_flip_sign = (false, false);
        session.adjust = None;
        session.preview_hidden_mark = -1;
        session.brush_live = None;
        session.gradient_live = None;
        session.patch_live = None;
        session.patch_region = None;
        session.heal_source = None;
        session.rotate_origin = None;
        session.fit_content = None;
        session.quick_mask = false;
        session.snap_statics.clear();
        session.snap = editcore::snap::SnapResult::default();
        session.mods = 0;
        session.disp_scale = 1.0;
        session.gesture = -1;
        session.gesture_index = -1;
        session.gesture_start = (0.0, 0.0);
        session.gesture_rect = (0.0, 0.0, 0.0, 0.0);
        session.fit_occ = editcore::alpha_fit::Occupancy::default();
        session.gpu_live = false;
        session.gpu_static_url.clear();
        session.gpu_active_url.clear();
        session.gpu_index = 0;
        session.gpu_rect = (0.0, 0.0, 0.0, 0.0, 1.0);
        session.sel_rev = session.sel_rev.wrapping_add(1);
        match reference {
            Some(reference) => session.install_calibration(reference),
            None => session.clear_calibration(),
        }
    }

    /// 换文档之后统一发布 UI 状态（图层面板 / 缩略图 / 写回守卫 / 预览）。
    fn publish_mode_switch(mut self: Pin<&mut Self>, to_fidelity: bool) {
        self.as_mut().cancel_thumbs("mode-switch");
        self.as_mut().set_thumbs_pending(false);
        self.as_mut().clear_psd_notice();
        let lossy = self
            .session
            .borrow()
            .as_ref()
            .is_some_and(|s| s.psd_lossy);
        self.as_mut().set_psd_write_lossy(lossy);
        self.as_mut().set_psd_lossy_ack(false);
        let has_annotations_now = !to_fidelity && !self.annotation_ids.borrow().is_empty();
        self.as_mut().set_has_annotations(has_annotations_now);
        self.as_mut().set_selected_layer(-1);
        self.as_mut().sync_layer_selection();
        self.as_mut().set_fidelity_mode(to_fidelity);
        self.as_mut().refresh_preview();
        self.as_mut().set_status(QString::from(if to_fidelity {
            "已返回保真显示：画面为 PSD 自带合并图（与 Photoshop 一致）"
        } else {
            "已进入分层编辑：显示 PS 原合成（校准），编辑图层后自动切换为重建预览"
        }));
        println!(
            "{{\"phase\":\"ui\",\"event\":\"fidelity-mode\",\"value\":{},\"ms\":{}}}",
            to_fidelity,
            elapsed_ms()
        );
    }

    /// Drain stage-2 messages on the UI thread. Called by the same 50 ms
    /// timer as `poll_decode` (no new timer is introduced).
    pub fn poll_psd_layers(mut self: Pin<&mut Self>) {
        loop {
            let next = {
                let job = self.psd_job.borrow();
                let Some(rx) = job.as_ref() else {
                    return;
                };
                rx.try_recv()
            };
            match next {
                Ok(PsdLayerMsg::Progress { done, total }) => {
                    let text = format!("正在加载图层 {done}/{total}…（完成前编辑工具暂不可用）");
                    self.as_mut().set_psd_progress(QString::from(text.as_str()));
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"psd-layer-progress\",\"done\":{},\"total\":{}}}",
                        done, total
                    );
                    // Automation-only: simulate the user pressing
                    // 「跳过图层加载」 after watching progress advance. Fire
                    // near the end (not on the first tick) so the escape
                    // happens while the lock is actually held and a queued
                    // Done message can never be processed afterwards.
                    if done > 0
                        && done >= total.saturating_sub(1)
                        && std::env::var("IMAGEEDIT_PSD_AUTO_SKIP").is_ok()
                    {
                        self.as_mut().skip_psd_layers();
                        return;
                    }
                }
                Ok(PsdLayerMsg::Done {
                    result,
                    elapsed_ms: stage2_ms,
                    thumbs,
                }) => {
                    println!(
                        "{{\"phase\":\"ui\",\"event\":\"psd-stage2-ms\",\"value\":{}}}",
                        stage2_ms
                    );
                    match result {
                        Ok(set) if !set.layers.is_empty() => {
                            self.as_mut().install_psd_layers(set, &thumbs)
                        }
                        Ok(_) => {
                            self.as_mut().set_fidelity_mode(true);
                            self.as_mut().set_status(QString::from(
                                "此 PSD 没有可编辑图层，已按合并预览打开",
                            ));
                            self.as_mut().set_psd_notice(QString::from(
                                "此 PSD 没有可编辑图层，已按合并预览打开。\n可正常编辑并另存为 PNG / JPEG；写回 PSD 不可用。",
                            ));
                        }
                        Err(err) => {
                            self.as_mut().set_fidelity_mode(true);
                            self.as_mut().set_status(QString::from(format!(
                                "图层加载失败，已按合并预览打开并解锁编辑：{err}"
                            )));
                            self.as_mut().set_psd_notice(QString::from(format!(
                                "图层加载失败：{err}\n\n已按合并预览打开并解锁编辑；为避免破坏原文件，写回 PSD 不可用，可另存为 PNG / JPEG。"
                            )));
                        }
                    }
                    self.as_mut().finish_psd_layers("done");
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.as_mut().set_fidelity_mode(true);
                    self.as_mut().set_status(QString::from(
                        "图层加载线程已结束，已按合并预览打开并解锁编辑",
                    ));
                    self.as_mut().finish_psd_layers("channel-closed");
                    return;
                }
            }
        }
    }

    /// Replace the composite placeholder document with the real layer stack.
    /// Loading is not an edit: history is reset (new Document), zoom / fit /
    /// viewport are kept so the window does not jump.
    pub(in crate::bridge) fn install_psd_layers(
        mut self: Pin<&mut Self>,
        set: decode::PsdLayerSet,
        thumbs: &[RgbaImage],
    ) {
        let t0 = Instant::now();
        // Defensive assertion: `--script` / external commands can bypass the
        // QML gate, so never overwrite edits that happened while loading.
        // 保真模式例外：合并图那份 Document 会被完整缓存起来（连同它自己的
        // 撤销栈），用户切回来时编辑还在，所以这里允许它带着编辑被换下。
        let (epoch, dirty) = {
            let guard = self.session.borrow();
            guard
                .as_ref()
                .map(|s| (s.doc.edit_epoch, s.dirty))
                .unwrap_or((0, false))
        };
        if !*self.fidelity_mode() && (epoch != 0 || dirty) {
            self.as_mut().set_status(QString::from(
                "加载期间文档被修改，已保留合并预览；请重新打开此 PSD 以使用分层编辑",
            ));
            return;
        }
        let count = set.layer_count();
        let bytes = set.bytes;
        let degraded = set.degraded.clone();
        let canvas = set.canvas;
        // Annotation layers ("标注 / 备注" folders) are notes for the
        // implementer, not artwork. Remember their stable ids so the view menu
        // can hide / show them after the stack is editable.
        let annotation_flags: Vec<bool> = set.layers.iter().map(|l| l.annotation).collect();
        let mut doc = Document::from_psd_layers(canvas, set.into_seeds());
        let mut annotation_ids: Vec<u32> = Vec::new();
        for (i, flag) in annotation_flags.iter().enumerate() {
            if !*flag {
                continue;
            }
            // `from_psd_layers` prepends the background layer.
            if let Some(layer) = doc.layers.get(i + 1) {
                annotation_ids.push(layer.id);
            }
        }
        // Hide them up front when the toggle is on — directly, not through a
        // history op: the freshly assembled document must keep `edit_epoch` at
        // 0 so the queued thumbnails below still match their cache keys.
        if !annotation_ids.is_empty() && *self.annotations_hidden() {
            for id in &annotation_ids {
                if let Some(layer) = doc.layers.iter_mut().find(|l| l.id == *id) {
                    layer.visible = false;
                }
            }
        }
        // C7: keep an explicit hint for very large stacks. Realtime preview
        // rebuild is O(layers), so warn instead of silently freezing.
        let heavy = count > 32 || bytes > 256 * 1024 * 1024;
        let thumb_t0 = Instant::now();
        let mut seeded = 0usize;
        let annotation_layers_present = !annotation_ids.is_empty();
        {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let reference = if *self.fidelity_mode() {
                // 保真模式下用户可能已经改过合并图：整份 Document 缓存起来，
                // 「返回保真显示」时连编辑与撤销栈一起换回来，绝不静默丢弃。
                // 同时留一份作为分层态的「原生预览校准」显示基准（§17）。
                let old = std::mem::replace(&mut session.doc, doc);
                let reference = old.clone();
                *self.inactive_doc.borrow_mut() = Some(InactiveDoc {
                    doc: old,
                    dirty: session.dirty,
                    note: session.note.clone(),
                    psd_layered: session.psd_layered,
                    psd_lossy: session.psd_lossy,
                });
                Some(reference)
            } else {
                // 同步 / --script 路径直出分层，没有保真态可参照。
                session.doc = doc;
                None
            };
            // The real layer stack is now in memory; Ctrl+S may write it back.
            session.psd_layered = true;
            // Publish the annotation-layer bookkeeping for the view menu.
            *self.annotation_ids.borrow_mut() = annotation_ids;
            // Cached previews were built from the 1-layer placeholder.
            session.composer = render::PreviewComposer::new();
            session.last_render = None;
            session.gpu_live = false;
            session.gpu_static_url.clear();
            session.gpu_active_url.clear();
            session.dirty = false;
            // C6: the worker already downscaled every layer to 40px. Queue
            // them for batched writing (<=6 rows per 50 ms tick) instead of
            // doing 100 PNG encodes in this single UI call. `edit_epoch` of a
            // freshly assembled document is 0, so keys match the first query.
            {
                let mut queue = self.thumb_queue.borrow_mut();
                queue.clear();
                for (i, thumb) in thumbs.iter().enumerate() {
                    let Some(layer) = session.doc.layers.get(i + 1) else {
                        continue;
                    };
                    queue.push_back((session.id, layer.id, 0u64, thumb.clone()));
                    seeded += 1;
                }
            }
            let mut note = format!("PSD 分层可编辑：背景 + {count} 个图层");
            if !degraded.is_empty() {
                note.push_str(&format!("；已降级 {} 项：{}", degraded.len(), degraded.join("、")));
            }
            if heavy {
                note.push_str(&format!(
                    "；图层较多（{count} 层 / {} MB），实时预览重建可能变慢",
                    bytes / 1_048_576
                ));
            }
            session.note = Some(note.clone());
            match reference {
                Some(reference) => session.install_calibration(reference),
                None => session.clear_calibration(),
            }
        }
        self.as_mut().set_thumbs_pending(seeded > 0);
        self.as_mut().set_has_annotations(annotation_layers_present);
        // 阶段 2 装配完成后画面换成按层重建的结果；保真那份仍在缓存里。
        self.as_mut().set_fidelity_mode(false);
        // The stack is editable, but a write-back would bake in every loss the
        // load reported (groups, masks, blend modes, clipped pixels). QML asks
        // for confirmation once per document before overwriting the source.
        self.as_mut().set_psd_write_lossy(!degraded.is_empty());
        self.as_mut().set_psd_lossy_ack(false);
        println!(
            "{{\"phase\":\"ui\",\"event\":\"layer-thumb-first-ms\",\"value\":{},\"layers\":{},\"seeded\":{}}}",
            thumb_t0.elapsed().as_millis(),
            count,
            seeded
        );
        self.as_mut().set_selected_layer(-1);
        self.as_mut().sync_layer_selection();
        self.as_mut().refresh_preview();
        self.as_mut().set_status(QString::from(format!(
            "显示 PS 原合成（校准）；编辑图层后切换为重建预览 · 已装配 {count} 层{}",
            if heavy {
                format!("；图层较多（{count} 层 / {} MB），实时预览可能变慢", bytes / 1_048_576)
            } else {
                String::new()
            }
        )));
        println!(
            "{{\"phase\":\"ui\",\"event\":\"psd-layer-count\",\"value\":{}}}",
            count
        );
        println!(
            "{{\"phase\":\"ui\",\"event\":\"psd-layer-bytes\",\"value\":{}}}",
            bytes
        );
        // 诊断：装配完成后的驻留量，与 `load-start` / `after-session` 对比。
        mem::log("after-install");
        if !degraded.is_empty() {
            self.as_mut().set_psd_notice(QString::from(format!(
                "已还原 {count} 个可编辑图层，其中 {} 项按降级处理：\n{}",
                degraded.len(),
                degraded.join("\n")
            )));
        }
        println!(
            "{{\"phase\":\"ui\",\"event\":\"psd-degraded\",\"value\":{}}}",
            degraded.len()
        );
        println!(
            "{{\"phase\":\"ui\",\"event\":\"psd-install-ms\",\"value\":{}}}",
            t0.elapsed().as_millis()
        );
        println!(
            "{{\"phase\":\"ui\",\"event\":\"fidelity-mode\",\"value\":false,\"ms\":{}}}",
            elapsed_ms()
        );
        // 探针：模拟一次真实编辑造成的两个可观察变化（epoch bump + dirty），
        // 验证校准会 sticky 失效、画面切回按层重建。
        if std::env::var("IMAGEEDIT_CALIBRATION_PROBE").ok().as_deref() == Some("edit") {
            if let Some(session) = self.session.borrow_mut().as_mut() {
                session.doc.edit_epoch = session.doc.edit_epoch.wrapping_add(1);
                session.dirty = true;
            }
            self.as_mut().refresh_preview();
            println!("{{\"phase\":\"ui\",\"event\":\"calibration-probe-edit\"}}");
        }
        // 探针：装配完立即换回保真，用来验证双向切换、缓存与预览失效。
        if matches!(
            std::env::var("IMAGEEDIT_FIDELITY_PROBE").ok().as_deref(),
            Some("roundtrip") | Some("roundtrip2")
        ) {
            self.as_mut().return_to_fidelity();
        }
    }
}

