//! 首图预热：主窗口还没建完就开始解码。
//!
//! 打开一张 8160×6120 的 JPEG 有两块成本：解码（-O3 下约 0.6 s）和
//! Qt/QML 启动（`window-visible` ≈ 0.5 s、`qml-completed` ≈ 0.9 s）。
//! 这两块原来是完全串行的 —— `start()` 由 QML 的 Component.onCompleted
//! 触发，解码要等整个界面树建完才开始，于是"图出现"的时间 =
//! QML 启动 + 解码 ≈ 1.7 s。
//!
//! 这里在 `main()` 里（单实例握手之后、QApplication 之前）就把第一张图
//! 丢给解码线程；`load_path` 遇到同一个路径时直接接管那个结果，两块成本
//! 重叠，打开时间压到 ≈ max(QML 启动, 解码) ≈ 1.0 s。
//!
//! 只有"普通双击打开"走这条路：自动化运行（任何 `--` 开关）本来就是同步
//! 解码，预热只会白跑一遍。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use decode::decode_path;

use super::session::{DecodeMsg, DOWNSAMPLE_EDGE};
use super::{AppConfig, DecodeOptions};

/// 至多一个（一个进程只预热"本窗口要打开的那一张"）。
static SLOT: Mutex<Option<(PathBuf, Receiver<DecodeMsg>)>> = Mutex::new(None);

/// Start decoding `path` on a worker thread. The result is parked in `SLOT`
/// until `take` claims it; if nobody claims it the thread just exits.
pub(in crate::bridge) fn spawn(path: &Path) {
    let opts = DecodeOptions {
        max_pixels: Some(300_000_000),
        max_dimension: if AppConfig::load().max_dimension != 0 {
            Some(DOWNSAMPLE_EDGE)
        } else {
            None
        },
    };
    let owned = path.to_path_buf();
    let thread_path = owned.clone();
    let (tx, rx) = mpsc::channel::<DecodeMsg>();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let result = decode_path(&thread_path, opts).map_err(|e| e.to_string());
        let ms = t0.elapsed().as_millis().min(i32::MAX as u128) as i32;
        let _ = tx.send(DecodeMsg {
            path: thread_path,
            result: result.map(|d| (d, ms)),
        });
    });
    println!(
        "{{\"phase\":\"ui\",\"event\":\"prewarm\",\"ms\":{}}}",
        super::elapsed_ms()
    );
    if let Ok(mut slot) = SLOT.lock() {
        *slot = Some((owned, rx));
    }
}

/// Claim the prewarmed decode for `path`, if it is still running (or already
/// finished) and belongs to this very file.
pub(in crate::bridge) fn take(path: &Path) -> Option<Receiver<DecodeMsg>> {
    let mut slot = SLOT.lock().ok()?;
    if !slot.as_ref().is_some_and(|(p, _)| same_path(p, path)) {
        return None;
    }
    let (_, rx) = slot.take()?;
    println!(
        "{{\"phase\":\"ui\",\"event\":\"prewarm-hit\",\"ms\":{}}}",
        super::elapsed_ms()
    );
    Some(rx)
}

/// The prewarm path comes straight from `argv`, the lookup path from a `QUrl`
/// (`file:///D:/...` → `D:/...`). Normalize both to an absolute, unified
/// separator, case-insensitive form — Windows paths are case-insensitive and
/// the drive letter casing is not stable across those two sources.
fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        std::path::absolute(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .replace('/', "\\")
            .to_lowercase()
    };
    norm(a) == norm(b)
}
