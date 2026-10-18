# ImageEdit

一款用 Rust + Qt6/QML 编写的 Windows 桌面图片编辑器：轻量、快速、无广告。

![platform](https://img.shields.io/badge/platform-Windows%2010%2B-blue) ![license](https://img.shields.io/badge/license-MIT-green) ![rust](https://img.shields.io/badge/rust-1.82%2B-orange)

## 功能特性

- **图层** — 添加/删除/重命名/复制图层，独立的不透明度、可见性、锁定；背景图层可解锁转为普通图层；拖动、四角四边手柄缩放（拖动期间实时预览）。
- **选区与裁剪** — 矩形选区、裁剪画布、提取选区为新图层、复制/粘贴（支持跨窗口）。
- **标记工具** — 矩形、椭圆、箭头、画笔、马赛克（划选区整块打码）、文本（双击就地编辑）；笔画粗细/颜色、字体面板（字体/字号/加粗/斜体/下划线/对齐）。标记与图层统一交互：随时点选、拖动、缩放，`Delete` 删除。
- **色彩调整** — RGB 直方图 + 黑场/Gamma/白场手柄、曝光/对比度/高光/阴影/饱和度/色温/色调/褐色/清晰度滑块、自动色阶，全部实时预览，关闭面板或确认后烘焙为可撤销操作。
- **尺子与拾色器** — 画布上测量两点距离与角度；吸管取色，支持 HEX/RGB/HSL/CMYK 一键复制。
- **撤销/重做** — 全部编辑操作可撤销（调色、图层变换、标记、裁剪等）。
- **多文档窗口** — 可同时打开多个编辑窗口，设置里可选"在新窗口打开"。
- **图片浏览** — `←`/`→` 在同目录图片间切换。
- **格式支持** — PNG / JPEG / BMP / GIF / TIFF / WebP 读写（WebP 导出为无损编码）；JPEG 导出质量可调。
- **文件关联** — 安装包支持勾选将上述格式关联到 ImageEdit。

## 下载安装

从 [Releases](../../releases) 页面获取：

- `imageedit-x.y.z-win64-setup.exe` — 安装包（简体中文向导，可勾选文件关联，per-user 安装无需管理员权限）。
- `imageedit-x.y-win64.zip` — 绿色版，解压即用。

系统要求：Windows 10 或更高版本（x64）。

## 从源码构建

### 依赖

| 依赖 | 版本 | 说明 |
| --- | --- | --- |
| Rust | 1.82+ | `rust-toolchain.toml` 已固定 stable + rustfmt/clippy |
| Qt | 6.8.x (msvc2022_64) | 官方在线安装器或 `aqtinstall` |
| CMake | 3.21+ | cxx-qt 代码生成需要 |
| Visual Studio 2022 Build Tools | — | 含 MSVC 编译器与 Windows SDK（rc.exe） |
| NSIS 3 | 可选 | 仅打包安装器时需要 |

### 步骤

```powershell
# 1. 安装 Qt 6.8.x msvc2022_64 后，把路径指给构建脚本（默认 D:\Qt\6.8.3\msvc2022_64）
$env:IMAGEEDIT_QT_ROOT = "C:\Qt\6.8.3\msvc2022_64"

# 2. 加载构建环境（配置 qmake、/utf-8 编译选项、清理冲突的 CC/CXX）
. scripts/env.ps1

# 3. 构建 / 运行
cargo run --release -p imageedit-ui -- <图片路径>

# 4. 测试
cargo test --workspace
```

打正式包（release 构建去掉控制台窗口 + windeployqt 依赖收集 + 裁剪 + 自测 + zip）：

```powershell
. scripts/env.ps1
powershell -ExecutionPolicy Bypass -File scripts/package.ps1
```

产出位于 `dist/`：绿色版 zip 与（若安装了 NSIS，再执行
`tools/nsis-3.11/Bin/makensis.exe -INPUTCHARSET UTF8 installer/imageedit.nsi`）安装包。

## 快捷键

| 按键 | 功能 |
| --- | --- |
| `Ctrl+S` / `Ctrl+Shift+S` | 保存 / 另存为 |
| `Ctrl+Z` / `Ctrl+Y` | 撤销 / 重做 |
| `Ctrl+W` / `Ctrl+Q` | 关闭窗口 / 退出（有未保存更改时先询问） |
| `←` / `→` | 浏览同目录图片（无活动手势时） |
| `Enter` | 确认裁剪 / 落定粘贴内容 / 确认框默认动作（保存） |
| `Esc` | 逐级取消（弹窗 → 手势 → 选中） |
| `Delete` | 删除选中的标记 |
| `F11` | 全屏 |

有未保存更改时，标题栏会显示 `*`，此时关闭/切换图片会弹出保存确认框（回车默认"保存并继续"）。

## 架构

| crate | 职责 |
| --- | --- |
| `crates/core` | 文档模型：图层、历史（撤销/重做）、标记几何与渲染、字体排版、色彩调整管线 |
| `crates/decode` | 图片解码/编码封装（基于 `image` crate） |
| `crates/render` | 预览合成器：带缓存的图层缩放与混合、live 手势期间零文档改动的预览 |
| `crates/ui` | cxx-qt 桥（Session/Bridge）+ QML 界面（`qml/main.qml`） |
| `crates/bench` | 性能基准工具 |

`docs/` 目录保留了开发过程中的设计文档（中文）。

## 许可

本项目代码以 [MIT](LICENSE) 许可发布。

运行时依赖 [Qt 6](https://www.qt.io/)（LGPLv3 开源版本），以动态链接方式使用；如需遵守 LGPL 重新链接 Qt，可参照上文从源码构建的步骤自行编译。第三方组件清单见 [ThirdPartyNotices.md](ThirdPartyNotices.md)。

## 参与贡献

欢迎 Issue 与 PR，见 [CONTRIBUTING.md](CONTRIBUTING.md)。
