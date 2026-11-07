// 配置项 / 棋盘格 / 注册查看器。
use super::super::*;

impl qobject::Bridge {
    pub fn register_as_viewer(mut self: Pin<&mut Self>) {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;

            let exe = match std::env::current_exe() {
                Ok(path) => path.to_string_lossy().to_string(),
                Err(_) => {
                    self.as_mut()
                        .set_status(QString::from("无法定位程序路径"));
                    return;
                }
            };
            let run_reg = |args: Vec<String>| -> bool {
                std::process::Command::new("reg")
                    .args(&args)
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
                    .map(|out| out.status.success())
                    .unwrap_or(false)
            };

            let mut all_ok = true;
            all_ok &= run_reg(vec![
                "add".into(),
                r"HKCU\Software\Classes\ImageEdit.Document".into(),
                "/ve".into(),
                "/d".into(),
                "ImageEdit 图片".into(),
                "/f".into(),
            ]);
            all_ok &= run_reg(vec![
                "add".into(),
                r"HKCU\Software\Classes\ImageEdit.Document\shell\open\command".into(),
                "/ve".into(),
                "/d".into(),
                format!("\"{exe}\" \"%1\""),
                "/f".into(),
            ]);
            for ext in [
                ".png", ".jpg", ".jpeg", ".bmp", ".gif", ".webp", ".tif", ".tiff",
            ] {
                all_ok &= run_reg(vec![
                    "add".into(),
                    format!(r"HKCU\Software\Classes\{ext}\OpenWithProgids"),
                    "/v".into(),
                    "ImageEdit.Document".into(),
                    "/t".into(),
                    "REG_SZ".into(),
                    "/d".into(),
                    "".into(),
                    "/f".into(),
                ]);
            }
            if !all_ok {
                self.as_mut()
                    .set_status(QString::from("注册失败：请尝试以管理员身份运行"));
                return;
            }
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", "ms-settings:defaultapps"])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            self.as_mut().set_status(QString::from(
                "已加入系统「打开方式」列表：请在打开的系统设置中将 ImageEdit 设为默认应用",
            ));
        }
        #[cfg(not(windows))]
        {
            self.as_mut()
                .set_status(QString::from("此功能仅支持 Windows"));
        }
    }

    pub fn update_checker_light(mut self: Pin<&mut Self>, value: &QString) {
        if *self.checker_light() == *value {
            return;
        }
        self.as_mut().set_checker_light(value.clone());
        self.as_mut().regenerate_checker_tile();
        self.persist_config();
    }

    pub fn update_checker_dark(mut self: Pin<&mut Self>, value: &QString) {
        if *self.checker_dark() == *value {
            return;
        }
        self.as_mut().set_checker_dark(value.clone());
        self.as_mut().regenerate_checker_tile();
        self.persist_config();
    }

    pub fn update_open_in_new_window(mut self: Pin<&mut Self>, value: bool) {
        if *self.open_in_new_window() == value {
            return;
        }
        self.as_mut().set_open_in_new_window(value);
        self.persist_config();
    }

    pub fn update_large_downsample(mut self: Pin<&mut Self>, value: bool) {
        if *self.large_downsample() == value {
            return;
        }
        self.as_mut().set_large_downsample(value);
        self.persist_config();
    }

    pub fn update_save_quality(mut self: Pin<&mut Self>, value: i32) {
        let q = value.clamp(1, 100);
        if *self.save_quality() == q {
            return;
        }
        self.as_mut().set_save_quality(q);
        self.persist_config();
        // Apply to the live document immediately (no reload needed).
        if let Some(s) = self.session.borrow_mut().as_mut() {
            s.save_quality = q as u8;
        }
    }

    pub fn update_ruler_bottom_up(mut self: Pin<&mut Self>, value: bool) {
        if *self.ruler_bottom_up() == value {
            return;
        }
        self.as_mut().set_ruler_bottom_up(value);
        self.persist_config();
    }

    /// D13：数值面板显示状态跨启动记忆。
    pub fn update_transform_panel_visible(mut self: Pin<&mut Self>, value: bool) {
        if *self.transform_panel_visible() == value {
            return;
        }
        self.as_mut().set_transform_panel_visible(value);
        self.persist_config();
    }

    /// D16：只切绘制开关；关显示时求解照常进行（PS 语义）。
    pub fn update_show_guides(mut self: Pin<&mut Self>, value: bool) {
        if *self.show_guides() == value {
            return;
        }
        self.as_mut().set_show_guides(value);
        self.persist_config();
        if !value {
            self.as_mut().set_guides_json(QString::from("[]"));
        }
    }

    /// D16：关吸附时求解短路，同时清掉残留提示线。
    pub fn update_snap_enabled(mut self: Pin<&mut Self>, value: bool) {
        if *self.snap_enabled() == value {
            return;
        }
        self.as_mut().set_snap_enabled(value);
        self.persist_config();
        if !value {
            self.as_mut().set_guides_json(QString::from("[]"));
        }
    }
}
