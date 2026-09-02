//! Breadcrumb: la ruta actual como secuencia de segmentos navegables.
//!
//! Conveniencia de referencia: `ground/spec/01-navegacion.md`, «Barra de
//! direcciones tipo breadcrumb». Lógica pura: parte una ruta y decide qué cabe.
//!
//! # Decisiones de diseño
//!
//! - **Aquí no se traducen etiquetas.** Cada segmento lleva su componente cruda y
//!   un [`SegmentKind`]; que la carpeta personal se muestre como «Inicio» o como
//!   `/home/oliverv` es decisión de la capa de presentación, no de `kara-core`.
//! - **`home` se recibe, no se lee.** Consultar `$HOME` aquí ataría el dominio al
//!   entorno del proceso y haría el resultado no reproducible en pruebas.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Qué representa un segmento, para que la vista elija icono y etiqueta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    /// Raíz del sistema de ficheros (`/`).
    Root,
    /// Carpeta personal del usuario.
    Home,
    /// Una carpeta cualquiera.
    Directory,
}

/// Un tramo navegable de la ruta.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Ruta absoluta a la que navega este segmento.
    pub path: PathBuf,
    /// Componente cruda. Para [`SegmentKind::Root`] es `/`.
    pub name: OsString,
    pub kind: SegmentKind,
}

/// Reparto de segmentos entre los que se ven y los que caen al desbordamiento.
#[derive(Debug, Clone, PartialEq)]
pub struct Collapsed {
    /// Ancestros ocultos, en orden. Van al menú del botón `«`.
    pub overflow: Vec<Segment>,
    /// Segmentos visibles, en orden. El último es siempre la carpeta actual.
    pub visible: Vec<Segment>,
}

/// Parte una ruta absoluta en segmentos navegables.
///
/// `home`, si se da y es prefijo de `path`, marca ese segmento como
/// [`SegmentKind::Home`]; los ancestros por encima siguen apareciendo, porque la
/// spec exige que la barra refleje la ubicación real y permita subir por encima
/// de la carpeta personal.
///
/// Una ruta relativa devuelve la lista vacía: no hay ancestros que ofrecer y
/// fingir una raíz sería inventar comportamiento.
#[must_use]
pub fn segments(path: &Path, home: Option<&Path>) -> Vec<Segment> {
    if !path.is_absolute() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut acc = PathBuf::new();

    for component in path.components() {
        match component {
            Component::RootDir => {
                acc.push("/");
                out.push(Segment {
                    path: acc.clone(),
                    name: OsString::from("/"),
                    kind: SegmentKind::Root,
                });
            }
            Component::Normal(name) => {
                acc.push(name);
                let kind = if home == Some(acc.as_path()) {
                    SegmentKind::Home
                } else {
                    SegmentKind::Directory
                };
                out.push(Segment {
                    path: acc.clone(),
                    name: name.to_os_string(),
                    kind,
                });
            }
            // `.`, `..` y prefijos de Windows no aparecen en una ruta absoluta ya
            // normalizada; ignorarlos evita fabricar segmentos que no navegan.
            _ => {}
        }
    }
    out
}

/// Decide qué segmentos caben en `max_visible` y cuáles van al desbordamiento.
///
/// Reglas de la spec, en este orden de fuerza:
///
/// 1. **Nunca se oculta el último segmento** — la carpeta actual siempre se ve.
/// 2. **Se mantienen visibles al menos la carpeta actual y su padre**, aunque
///    `max_visible` pida menos. Un breadcrumb que solo enseña dónde estás, sin
///    decir de dónde cuelga, no orienta.
/// 3. Lo que sobra se recorta **por el principio**: se ocultan los ancestros más
///    lejanos, que son los menos útiles.
#[must_use]
pub fn collapse(segments: &[Segment], max_visible: usize) -> Collapsed {
    // El mínimo de la regla 2, acotado por lo que realmente hay.
    let floor = segments.len().min(2);
    let allowed = max_visible.max(floor);

    if segments.len() <= allowed {
        return Collapsed {
            overflow: Vec::new(),
            visible: segments.to_vec(),
        };
    }

    let split = segments.len() - allowed;
    Collapsed {
        overflow: segments[..split].to_vec(),
        visible: segments[split..].to_vec(),
    }
}
