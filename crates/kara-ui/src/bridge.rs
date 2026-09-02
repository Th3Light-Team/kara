//! Puente cxx-qt: expone los modelos y acciones de Rust al motor QML.
//!
//! El QML **no** contiene lógica de negocio; todo lo que la UI necesita entra por
//! aquí desde `kara-ops` / `kara-index` / `kara-fs`.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, version)]
        type App = super::AppRust;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }
}

pub struct AppRust {
    version: cxx_qt_lib::QString,
}

impl Default for AppRust {
    fn default() -> Self {
        Self {
            version: cxx_qt_lib::QString::from(env!("CARGO_PKG_VERSION")),
        }
    }
}
