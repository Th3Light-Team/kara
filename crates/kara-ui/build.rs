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
            QmlFile::from("qml/EntryMouse.qml"),
            QmlFile::from("qml/EntryIcon.qml"),
            QmlFile::from("qml/FileDetails.qml"),
            QmlFile::from("qml/FileGrid.qml"),
            QmlFile::from("qml/ViewModeButton.qml"),
            QmlFile::from("qml/ColumnHeader.qml"),
            QmlFile::from("qml/TabStrip.qml"),
            QmlFile::from("qml/TabModeButton.qml"),
            QmlFile::from("qml/RubberBand.qml"),
            // Prueba de extremo a extremo. Va en el módulo siempre, y solo se
            // instancia con `--e2e`: separarla por feature obligaría a que
            // `Main.qml` la cargara por URL y perdería la comprobación de
            // tipos, que es justo lo que sujeta un arnés de prueba.
            QmlFile::from("qml/E2E.qml"),
        ]),
    )
    .qt_module("Quick")
    .include_dir("cpp")
    // El portapapeles es de Qt y `cxx-qt-lib` no envuelve `QClipboard` ni
    // `QMimeData`: este trozo de C++ es el mínimo puente para llegar a ellos.
    .cpp_file("cpp/clipboard.cpp")
    .file("src/bridge.rs")
    .build();
}
