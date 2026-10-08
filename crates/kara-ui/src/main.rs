//! Binario principal de Kara: arranca Qt y carga el módulo QML.

mod args;
mod bridge;
mod prefs;
mod present;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

fn main() -> std::process::ExitCode {
    // Before the window exists: the Wayland app_id is fixed when it is created,
    // and it has to name `kara.desktop` for the desktop to show Kara's icon.
    bridge::qobject::set_desktop_file_name("kara");
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/com/kara/ui/qml/Main.qml"));
    }

    // Se devuelve lo que devuelve Qt en vez de descartarlo: `Qt.exit(1)` desde
    // QML es como la prueba de extremo a extremo dice que algo falló, y sin
    // esto el proceso salía siempre con 0.
    let code = match app.as_mut() {
        Some(app) => app.exec(),
        None => 1,
    };
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(1))
}
