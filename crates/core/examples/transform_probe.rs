//! P0-0 基线探针：只依赖 editcore 的纯数学模块，不启动 Qt / QML。
//!
//! ```text
//! cargo run -q -p imageedit-core --example transform_probe -- --probe-transform
//! ```
//! 主程序也支持同名开关（`imageedit.exe --probe-transform`），两者输出一致，
//! 便于在 P0-1 接线前后逐字段核对几何基线。

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--probe-transform") || args.is_empty() {
        for line in editcore::transform_probe_lines() {
            println!("{line}");
        }
    } else {
        eprintln!("usage: transform_probe [--probe-transform]");
        std::process::exit(2);
    }
}
