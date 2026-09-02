use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(
        QmlModule::new("com.kara.ui").qml_files([
            QmlFile::from("qml/Main.qml"),
            // Singleton: la paleta es una sola para toda la ventana. Pasarla de
            // padre a hijo obligaria a cada componente a recibirla y a cada uso
            // a reenviarla, y el primero que se olvide se queda con los colores
            // del tema contrario.
            QmlFile::from("qml/Theme.qml").singleton(true),
            QmlFile::from("qml/TitleButton.qml"),
            QmlFile::from("qml/NavButton.qml"),
            QmlFile::from("qml/AddressBar.qml"),
            QmlFile::from("qml/Sidebar.qml"),
            QmlFile::from("qml/ResizeHandle.qml"),
        ]),
    )
    .qt_module("Quick")
    .file("src/bridge.rs")
    .build();
}
