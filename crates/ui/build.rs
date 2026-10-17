use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("com.imageedit").qml_file("qml/main.qml"))
        .qt_module("Quick")
        .qt_module("Qml")
        .files(["src/bridge.rs"])
        .build();
}
