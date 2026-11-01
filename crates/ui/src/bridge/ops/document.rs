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
                        let write_result = encode(&layer.image, Format::Png, session.save_quality)
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

    pub fn save_edited(mut self: Pin<&mut Self>) {
        let result = self
            .session
            .borrow()
            .as_ref()
            .map(Session::save)
            .unwrap_or_else(|| Err("nothing to save".to_string()));
        if result.is_ok() {
            if let Some(s) = self.session.borrow_mut().as_mut() {
                s.dirty = false;
            }
            self.as_mut().set_dirty(false);
            self.as_mut().update_file_info();
        }
        self.as_mut().report_save(result);
    }

    pub fn save_as(mut self: Pin<&mut Self>, url: &QUrl) {
        let path = PathBuf::from(url.to_local_file_or_default().to_string());
        let result = {
            let guard = self.session.borrow();
            match guard.as_ref() {
                None => Err("nothing to save".to_string()),
                Some(session) => {
                    // Refuse a hand-typed .psd target the same way Ctrl+S
                    // refuses the source file — there is no PSD encoder.
                    if is_psd_path(&path) {
                        Err(PSD_SAVE_HINT.to_string())
                    } else {
                        let format = format_from_path(&path).unwrap_or(Format::Png);
                        session.write_to(&path, format).map(|()| path.clone())
                    }
                }
            }
        };
        if result.is_ok() {
            if let Some(s) = self.session.borrow_mut().as_mut() {
                s.dirty = false;
                s.source = Some(path.clone());
            }
            self.as_mut().set_dirty(false);
            self.as_mut().update_file_info();
        }
        self.as_mut().report_save(result);
    }

    pub fn poll_decode(mut self: Pin<&mut Self>) {
        let msg = {
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
                    None
                }
            }
        };
        if msg.is_none() {
            return;
        }
        self.as_mut().set_loading(false);
        let Some(msg) = msg else {
            return;
        };
        let viewport = *self.viewport.borrow();
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
                *self.session.borrow_mut() = Some(session);
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
                }
            }
            Err(err) => {
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
}
