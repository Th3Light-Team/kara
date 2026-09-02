use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(
        QmlModule::new("com.kara.ui").qml_files(["../../qml/Main.qml"]),
    )
    .qt_module("Quick")
    .file("src/bridge.rs")
    .build();
}
