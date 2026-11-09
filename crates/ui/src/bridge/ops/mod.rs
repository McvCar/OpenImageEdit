//! `impl qobject::Bridge` 的方法按功能拆到各子模块。
//!
//! 可见性事实：私有助手方法留在 `bridge/mod.rs`（父模块），本目录所有
//! 后代模块可直接访问（Rust 后代可见祖先私有项），故 qinvokable 方法调用
//! 助手方法无需改可见性。`#[qproperty]/#[qinvokable]` 声明全在 mod.rs 的
//! `qobject` 块里，这里只是实现体。

pub(super) mod script;
pub(super) mod shell;
pub(super) mod settings;
pub(super) mod document;
pub(super) mod view;
pub(super) mod canvas;
pub(super) mod layers;
pub(super) mod layer_cmds;
pub(super) mod adjust;
pub(super) mod picker;
pub(super) mod clipboard;
pub(super) mod cursor;
pub(super) mod marks;
pub(super) mod selection;
pub(super) mod paint;
pub(super) mod transform;
