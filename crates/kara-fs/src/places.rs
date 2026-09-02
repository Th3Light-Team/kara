//! Las ubicaciones que encabezan el panel de navegación.
//!
//! Conveniencia de referencia: `ground/spec/01-navegacion.md`, «Panel de
//! navegación en árbol» — el panel «se organiza en secciones (Acceso
//! rápido/Favoritos, Este equipo/Unidades, Dispositivos extraíbles, Red)».
//!
//! # Decisiones de diseño
//!
//! - **Aquí no se traduce nada.** Cada ubicación sale con un [`PlaceKind`]; que
//!   `$HOME` se lea «Inicio» y `XDG_DOWNLOAD_DIR` «Descargas» lo decide la
//!   vista, igual que con las migas de pan.
//! - **Lo que no existe no aparece.** Una entrada automática que apunte a una
//!   carpeta borrada no aporta nada: para las que el usuario fije a mano sí hará
//!   falta enseñarlas como no disponibles, que es lo que pide la spec, pero eso
//!   es otra conveniencia.
//! - **El troceo de los ficheros va aparte de leerlos.** [`parse_user_dirs`] y
//!   [`parse_mounts`] son funciones puras sobre texto, y son donde vive todo lo
//!   que puede salir mal: comillas, comentarios, `$HOME`, y los escapes octales
//!   de `/proc/mounts`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Qué es una ubicación, para que la vista elija rótulo e icono.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlaceKind {
    /// La carpeta personal.
    Home,
    Desktop,
    Downloads,
    Documents,
    Pictures,
    Music,
    Videos,
    /// La raíz del sistema de ficheros.
    Root,
    /// Un volumen montado: disco externo, memoria USB, partición.
    Volume,
}

/// Una ubicación del panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub path: PathBuf,
    pub kind: PlaceKind,
    /// Cómo lo llama el sistema. Solo lo traen los volúmenes, que no tienen un
    /// nombre fijo: sale del punto de montaje.
    pub label: Option<OsString>,
}

/// Las carpetas de uso frecuente: la personal y las de usuario de XDG.
///
/// El orden es el del Explorador de Windows, no el alfabético.
#[must_use]
pub fn quick_access(home: Option<&Path>) -> Vec<Place> {
    let Some(home) = home else {
        return Vec::new();
    };

    let mut places = vec![Place {
        path: home.to_path_buf(),
        kind: PlaceKind::Home,
        label: None,
    }];

    let configured = read_user_dirs(home);
    for (key, kind, fallback) in USER_DIRS {
        let path = configured
            .get(*key)
            .cloned()
            // Sin `user-dirs.dirs` —o sin esa entrada— se prueba el nombre
            // convencional. Es lo que hace `xdg-user-dir` cuando no hay
            // configuración, y en la mayoría de instalaciones acierta.
            .unwrap_or_else(|| home.join(fallback));

        // La carpeta personal ya está la primera; que `XDG_DESKTOP_DIR` apunte a
        // `$HOME` es una configuración real (escritorio sin carpeta propia) y
        // duplicaría la entrada.
        if path == home {
            continue;
        }
        if path.is_dir() {
            places.push(Place {
                path,
                kind: *kind,
                label: None,
            });
        }
    }

    places
}

/// La raíz y los volúmenes montados.
#[must_use]
pub fn this_computer() -> Vec<Place> {
    let mut places = vec![Place {
        path: PathBuf::from("/"),
        kind: PlaceKind::Root,
        label: None,
    }];

    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    for mount in parse_mounts(&mounts) {
        let label = mount.file_name().map(std::ffi::OsStr::to_os_string);
        places.push(Place {
            path: mount,
            kind: PlaceKind::Volume,
            label,
        });
    }

    places
}

/// Claves de XDG, en el orden en que se enseñan, con su nombre convencional.
const USER_DIRS: &[(&str, PlaceKind, &str)] = &[
    ("XDG_DESKTOP_DIR", PlaceKind::Desktop, "Desktop"),
    ("XDG_DOWNLOAD_DIR", PlaceKind::Downloads, "Downloads"),
    ("XDG_DOCUMENTS_DIR", PlaceKind::Documents, "Documents"),
    ("XDG_PICTURES_DIR", PlaceKind::Pictures, "Pictures"),
    ("XDG_MUSIC_DIR", PlaceKind::Music, "Music"),
    ("XDG_VIDEOS_DIR", PlaceKind::Videos, "Videos"),
];

/// Dónde se montan los volúmenes del usuario en las distribuciones actuales.
///
/// Es una lista de prefijos, no una consulta a udisks: el panel enseña
/// ubicaciones, y equivocarse aquí sobra una entrada o falta una, no rompe nada.
/// Todo lo demás que hay en `/proc/mounts` —`/proc`, `/sys`, `/run/...`, las
/// capas de los snaps— es fontanería que el usuario no navega.
const MOUNT_PREFIXES: &[&str] = &["/media/", "/run/media/", "/mnt/"];

/// Lee `user-dirs.dirs`. Devuelve vacío si no existe, que es lo normal en una
/// sesión recién creada.
fn read_user_dirs(home: &Path) -> BTreeMap<String, PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));

    let Ok(text) = std::fs::read_to_string(config.join("user-dirs.dirs")) else {
        return BTreeMap::new();
    };
    parse_user_dirs(&text, home)
}

/// Trocea `user-dirs.dirs`.
///
/// El formato es un fragmento de shell: `XDG_DOWNLOAD_DIR="$HOME/Descargas"`.
/// Solo se admite `$HOME` al principio, que es lo único que `xdg-user-dirs`
/// escribe; una ruta absoluta también vale. Cualquier otra cosa se descarta en
/// vez de intentar interpretar shell de verdad.
#[must_use]
pub fn parse_user_dirs(text: &str, home: &Path) -> BTreeMap<String, PathBuf> {
    let mut dirs = BTreeMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !key.starts_with("XDG_") {
            continue;
        }

        let value = value.trim().trim_matches('"');
        let path = if let Some(rest) = value.strip_prefix("$HOME/") {
            home.join(rest)
        } else if value == "$HOME" {
            home.to_path_buf()
        } else if value.starts_with('/') {
            PathBuf::from(value)
        } else {
            continue;
        };

        dirs.insert(key.to_string(), path);
    }

    dirs
}

/// Saca de `/proc/mounts` los puntos de montaje que el usuario navega.
///
/// Se ordenan y se quitan los repetidos: un mismo punto de montaje puede
/// aparecer dos veces si algo se montó encima, y en el panel es una entrada.
#[must_use]
pub fn parse_mounts(text: &str) -> Vec<PathBuf> {
    let mut mounts: Vec<PathBuf> = text
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(unescape_mount)
        .filter(|mount| {
            MOUNT_PREFIXES
                .iter()
                .any(|prefix| mount.starts_with(prefix) && mount.len() > prefix.len())
        })
        .map(PathBuf::from)
        .collect();

    mounts.sort();
    mounts.dedup();
    mounts
}

/// Deshace los escapes octales de `/proc/mounts`.
///
/// El núcleo escapa espacio, tabulador, salto de línea y la propia barra
/// invertida; sin deshacerlo, un disco llamado «Mis cosas» se monta en una ruta
/// que no existe.
fn unescape_mount(raw: &str) -> String {
    if !raw.contains('\\') {
        return raw.to_string();
    }

    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }

        // Se recorre por caracteres y no por bytes: un punto de montaje puede
        // llevar acentos, y trocear su UTF-8 byte a byte los destroza.
        let digits: String = chars.clone().take(3).collect();
        let octal = digits.len() == 3 && digits.bytes().all(|b| b.is_ascii_digit() && b < b'8');
        // Solo se deshacen los escapes ASCII, que son los unicos que el nucleo
        // escribe: espacio, tabulador, salto de linea y la propia barra.
        if octal && let Ok(byte) = u8::from_str_radix(&digits, 8) && byte.is_ascii() {
            out.push(byte as char);
            for _ in 0..3 {
                chars.next();
            }
            continue;
        }

        out.push('\\');
    }
    out
}
