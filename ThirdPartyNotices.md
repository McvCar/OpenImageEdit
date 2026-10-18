# 第三方组件声明

ImageEdit 依赖下列优秀的开源项目。Rust 依赖的完整版本清单见
[Cargo.lock](Cargo.lock)，各项许可证以其仓库声明为准。

## 运行时依赖

| 组件 | 许可证 | 用途 |
| --- | --- | --- |
| [Qt 6.8](https://www.qt.io/)（QtGui / QtQml / QtQuick） | LGPL-3.0 | 图形界面框架（动态链接） |
| [cxx](https://github.com/dtolnay/cxx) | MIT / Apache-2.0 | Rust ↔ C++ 互操作 |
| [cxx-qt](https://github.com/KDAB/cxx-qt) / cxx-qt-lib | MIT / Apache-2.0 | Qt 的 Rust 绑定与 QML 桥 |
| [image](https://github.com/image-rs/image) | MIT / Apache-2.0 | 图片解码/编码 |
| [anyhow](https://github.com/dtolnay/anyhow) | MIT / Apache-2.0 | 错误处理 |
| [thiserror](https://github.com/dtolnay/thiserror) | MIT / Apache-2.0 | 错误派生 |
| [tracing](https://github.com/tokio-rs/tracing) | MIT | 日志 |
| [winresource](https://github.com/muruu/winresource) | MIT / Apache-2.0 | Windows 资源（图标）嵌入 |

### 关于 Qt（LGPLv3）

本项目使用 Qt 开源版本，并以**动态链接**方式分发（安装包/绿色版中的
Qt6*.dll 与 QML 模块来自 Qt 6.8.3 msvc2022_64）。根据 LGPLv3，您有权
替换或重新链接这些 Qt 库：参照 [README](README.md#从源码构建) 的构建
步骤，从 [qt.io](https://www.qt.io/download-open-source) 或
`aqtinstall` 获取同版本 Qt 源码与二进制，替换 DLL 后重新运行
`scripts/package.ps1` 即可。

## 构建期工具（不随产物分发）

| 组件 | 许可证 | 用途 |
| --- | --- | --- |
| [NSIS 3](https://nsis.sourceforge.io/) | zlib/libpng | 构建 Windows 安装包（`installer/`） |
| [aqtinstall](https://github.com/miurahr/aqtinstall) | MIT | CI 中安装 Qt（可选） |
