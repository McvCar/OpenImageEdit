# 贡献指南

感谢考虑为 ImageEdit 贡献代码！

## 开发环境

请先按 [README](README.md#从源码构建) 的"从源码构建"一节装好
Rust 1.82+、Qt 6.8.x (msvc2022_64)、CMake 与 MSVC Build Tools。

```powershell
$env:IMAGEEDIT_QT_ROOT = "C:\Qt\6.8.3\msvc2022_64"   # 你的 Qt 安装路径
. scripts/env.ps1                                       # 每个新 shell 都要重新加载
cargo test --workspace                                  # 提交前跑一遍
```

> `env.ps1` 会清理与 MSVC 冲突的 `CC`/`CXX` 变量并设置 `/utf-8` 编译选项，
> 跳过它构建大概率失败。

## 提交规范

- 提交信息用英文祈使句，首行不超过 72 字符，正文说明**为什么**改。
- 提交前运行 `cargo fmt` 与 `cargo clippy --workspace`（rust-toolchain.toml
  已包含这两个组件）。
- 新功能请附带单元测试（core/render 的纯 Rust 逻辑必须有测试覆盖）；
  UI 交互问题尽量给出复现步骤或截图。

## 代码结构速览

- `crates/core` — 文档模型与编辑操作（撤销/重做核心）。所有会改动文档的
  逻辑都走 `Document::execute`；live 手势期间不得改动文档像素（预览由
  `crates/render` 的 `PreviewComposer` 负责）。
- `crates/ui/src/bridge.rs` — cxx-qt 桥。QML 只通过 Bridge 的
  `#[qinvokable]` 与 qproperty 访问文档；注意 qproperty 的 QML 名是
  snake_case。
- `crates/ui/qml/main.qml` — 全部界面。该文件较大且缩进层级复杂，
  插入 QML 块前务必确认目标容器。

## 问题报告

提 Issue 时请附上：Windows 版本、显卡/驱动（渲染类问题）、复现步骤、
预期与实际行为。崩溃类问题如能有 `--selftest` 或控制台输出更有帮助
（debug 构建会打印 JSON 事件流）。

## 行为准则

保持友善、专业。对事不对人。
