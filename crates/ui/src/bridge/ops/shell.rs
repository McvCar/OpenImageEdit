// 窗口级外部打开事件。
use super::super::*;

impl qobject::Bridge {
    pub fn start(mut self: Pin<&mut Self>) {
        let selftest = std::env::args().any(|a| a == "--selftest");
        self.as_mut().set_auto_quit(selftest);

        let script = std::env::args().find_map(|a| {
            a.strip_prefix("--script=")
                .map(|value| value.to_string())
        });
        let file = std::env::args().skip(1).find(|a| !a.starts_with("--"));
        let Some(file) = file else {
            self.as_mut().set_status(QString::from("no-input"));
            println!("{{\"phase\":\"ui\",\"event\":\"no-input\"}}");
            return;
        };
        // Automation runs (any -- flag: selftest, scripts, probes) decode
        // synchronously because the script ops right after expect a loaded
        // document. Plain double-click runs decode in the background so a
        // huge image never blocks the window from showing up.
        let automation = std::env::args().any(|a| a.starts_with("--"));
        if automation {
            self.as_mut().load_path_sync(Path::new(&file));
        } else {
            self.as_mut().load_path(Path::new(&file));
        }

        if let Some(script) = script {
            self.as_mut().run_script(&script);
        }
    }

    pub fn take_external_open(self: Pin<&mut Self>) -> QString {
        let Some(path) = crate::instance::take_pending() else {
            return QString::default();
        };
        println!(
            "{{\"phase\":\"ui\",\"event\":\"external-open-consumed\",\"file\":\"{}\"}}",
            path.to_string_lossy().replace('\\', "/")
        );
        let url = file_url(&path);
        QString::from(url.as_str())
    }
}
