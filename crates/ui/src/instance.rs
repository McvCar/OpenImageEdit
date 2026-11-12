//! Single-instance hand-off over a loopback socket (std only, no new crates).
//!
//! Double-clicking an image in Explorer starts a NEW process. Every window
//! therefore used to live in its own process, which broke the process-wide
//! layer clipboard (copy in one Explorer-launched window, paste in another →
//! "剪贴板为空") and left N copies of the app running.
//!
//! Fix: the first process binds a listener on `127.0.0.1:<ephemeral>` and
//! records the port in `%APPDATA%/ImageEdit/instance`. A later process that was
//! launched with exactly one image path connects, sends the path, and exits;
//! the running process turns it into either a new document window or a
//! replacement of the current one, following the existing "在单独的新窗口显示"
//! setting.
//!
//! Deliberately NOT a general IPC channel: one line in, one line out, localhost
//! only, and the receiver validates that the path exists. Runs started for
//! automation (`--probe-*`, `--script=…`, `--selftest`, any `--` flag) skip this
//! module entirely so tests and probes can run side by side with the user's
//! session.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Reply the running instance sends back once it queued the path. A magic
/// string (instead of a bare "ok") keeps a cold start from mistaking some
/// unrelated service that happens to own the recorded port for an instance.
const ACK: &str = "imageedit-instance-ok";

/// Health probe: the client sends this (instead of a path) to ask "is the
/// recorded instance still there?". The receiver answers [`ACK`] and queues
/// nothing. Needed because a hand-off can legitimately fail while the other
/// instance is healthy (the dropped file no longer exists), and in that case
/// we must NOT overwrite the recorded port — see [`acquire`].
const PING: &str = "imageedit-instance-ping";

/// How long a second process waits to reach the running instance before giving
/// up and starting on its own.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
/// Paths longer than this are rejected by the receiver (defensive).
const MAX_MESSAGE: usize = 32 * 1024;

/// Files received from other processes, drained by the QML side.
fn pending() -> &'static Mutex<Vec<PathBuf>> {
    static PENDING: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

fn handshake_file() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|root| {
        PathBuf::from(root)
            .join("ImageEdit")
            .join("instance")
    })
}

/// Hand `path` to the already running instance.
///
/// Returns `true` when the other instance acknowledged the path, in which case
/// this process must exit without creating any window.
pub fn try_handoff(path: &Path) -> bool {
    let Some(file) = handshake_file() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(&file) else {
        return false;
    };
    let Some(port) = text
        .lines()
        .next()
        .and_then(|line| line.trim().parse::<u16>().ok())
    else {
        return false;
    };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) else {
        // Stale file from a crashed process: fall through and take over.
        return false;
    };
    let _ = stream.set_write_timeout(Some(CONNECT_TIMEOUT));
    let _ = stream.set_read_timeout(Some(CONNECT_TIMEOUT));
    let line = format!("{}\n", path.to_string_lossy());
    if stream.write_all(line.as_bytes()).is_err() {
        return false;
    }
    // The receiver answers "ok" once it queued the path.
    let mut ack = String::new();
    if BufReader::new(&stream).read_line(&mut ack).is_err() {
        return false;
    }
    ack.trim() == ACK
}

/// Ask the recorded instance whether it is still alive.
///
/// Only a valid [`ACK`] counts: a stale file left by a process that was killed,
/// or some unrelated service that happens to own the port, must read as "no
/// instance" so the new process can take over.
fn live_port() -> Option<u16> {
    let file = handshake_file()?;
    let text = std::fs::read_to_string(&file).ok()?;
    let port = text.lines().next()?.trim().parse::<u16>().ok()?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()?;
    let _ = stream.set_write_timeout(Some(CONNECT_TIMEOUT));
    let _ = stream.set_read_timeout(Some(CONNECT_TIMEOUT));
    stream.write_all(format!("{PING}\n").as_bytes()).ok()?;
    let mut ack = String::new();
    BufReader::new(&stream).read_line(&mut ack).ok()?;
    if ack.trim() == ACK {
        Some(port)
    } else {
        None
    }
}

/// Become the instance that later processes hand their files to.
///
/// Idempotent: the first call binds and spawns the accept thread, later calls
/// (there are none today, but multi-window code may call it defensively) are
/// no-ops.
pub fn acquire() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    // ★ 已有实例活着，就把地址留给它。手递失败并不代表对面死了 —— 拖进来的
    // 文件已经被删（rejected）、路径超长、500ms 超时，都会走到这里。旧实现
    // 无条件改写端口文件，正在运行的实例从此收不到任何双击：用户看到的就是
    // 「双击打不开 + 凭空多开一个窗口」，新窗口同样收不到下一个文件。
    // 探测用 PING，不占用待打开队列。
    if let Some(port) = live_port() {
        println!(
            "{{\"phase\":\"ui\",\"event\":\"instance-live-elsewhere\",\"port\":{port},\"pid\":{}}}",
            std::process::id()
        );
        return;
    }
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else {
        return;
    };
    let Ok(addr) = listener.local_addr() else {
        return;
    };
    if let Some(file) = handshake_file() {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text = format!("{}\n{}\n", addr.port(), std::process::id());
        let _ = std::fs::write(&file, text);
    }
    println!(
        "{{\"phase\":\"ui\",\"event\":\"instance-primary\",\"port\":{},\"pid\":{}}}",
        addr.port(),
        std::process::id()
    );
    std::thread::Builder::new()
        .name("imageedit-instance".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                handle_connection(stream);
            }
        })
        .ok();
}

fn handle_connection(stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.len() > MAX_MESSAGE {
        return;
    }
    // Health probe: answer, queue nothing, log nothing (a probe is not an
    // open request, and it fires on every double-click that fails to hand off).
    if line.trim() == PING {
        let mut stream = reader.into_inner();
        let _ = stream.write_all(format!("{ACK}\n").as_bytes());
        return;
    }
    let path = PathBuf::from(line.trim_end_matches(['\r', '\n']));
    let accepted = !path.as_os_str().is_empty() && path.is_file();
    if accepted {
        pending().lock().unwrap().push(path.clone());
        println!(
            "{{\"phase\":\"ui\",\"event\":\"external-open\",\"file\":\"{}\"}}",
            path.to_string_lossy().replace('\\', "/")
        );
    } else {
        println!(
            "{{\"phase\":\"ui\",\"event\":\"external-open-rejected\",\"file\":\"{}\"}}",
            path.to_string_lossy().replace('\\', "/")
        );
    }
    let mut stream = reader.into_inner();
    let reply = if accepted {
        format!("{ACK}\n")
    } else {
        "no\n".to_string()
    };
    let _ = stream.write_all(reply.as_bytes());
}

/// Pop the next file handed over by another process ("" when none).
pub fn take_pending() -> Option<PathBuf> {
    let mut queue = pending().lock().ok()?;
    if queue.is_empty() {
        None
    } else {
        Some(queue.remove(0))
    }
}

/// Queue a path for the primary window to open, same drain path as a
/// single-instance hand-off. Used when a fresh process was launched with
/// several files (Explorer multi-select): the first opens via the CLI scan,
/// the rest land here.
pub fn queue_pending(path: PathBuf) {
    if let Ok(mut queue) = pending().lock() {
        queue.push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test so the shared `pending()` queue cannot be observed by a
    /// concurrently running sibling test.
    #[test]
    fn handoff_accepts_existing_files_and_rejects_missing_ones() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let (stream, _) = listener.accept().unwrap();
                handle_connection(stream);
            }
        });

        fn send(addr: SocketAddr, line: &str) -> String {
            let mut client = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).unwrap();
            client.write_all(format!("{line}\n").as_bytes()).unwrap();
            let mut ack = String::new();
            BufReader::new(&client).read_line(&mut ack).unwrap();
            ack.trim().to_string()
        }

        let file = std::env::temp_dir().join("imageedit-instance-test.png");
        std::fs::write(&file, b"x").unwrap();

        assert_eq!(send(addr, &file.to_string_lossy()), ACK);
        assert_eq!(send(addr, "D:/definitely/not/here.png"), "no");
        // 探针：回 ACK，但**不**把 PING 字符串当成路径塞进待打开队列
        // （否则主窗口会拿一行 "imageedit-instance-ping" 去打开文件）。
        assert_eq!(send(addr, PING), ACK);
        server.join().unwrap();

        assert_eq!(take_pending().as_deref(), Some(file.as_path()));
        assert!(take_pending().is_none(), "rejected path must not be queued");
        let _ = std::fs::remove_file(&file);
    }
}
