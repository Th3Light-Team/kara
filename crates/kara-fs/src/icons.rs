//! Iconos del tema del escritorio, según la spec de temas de iconos de
//! FreeDesktop.
//!
//! # Por qué el tema del sistema y no un juego propio
//!
//! Un juego propio cubriría una docena de tipos; el tema instalado cubre
//! cientos y lo amplía cualquier paquete que se instale. Además es lo que el
//! usuario ya eligió: si tiene el escritorio en un tema violeta, Kara no puede
//! ser la única ventana con iconos azules.
//!
//! # Decisiones de diseño
//!
//! - **La búsqueda se resuelve una vez por nombre de icono, no por fichero.**
//!   Una carpeta con 100 000 entradas tiene un puñado de tipos distintos; sin
//!   memoria, listar sería cientos de miles de `stat` buscando ficheros que ni
//!   existen.
//! - **El troceo de `index.theme` va aparte de recorrer el disco**, que es lo
//!   que permite probar el algoritmo de tallas sin instalar temas.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use kara_core::entry::EntryKind;

use crate::mime::MimeDatabase;

/// Cómo se interpreta la talla de un subdirectorio del tema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirKind {
    /// Solo sirve para su talla exacta.
    Fixed,
    /// Sirve para cualquier talla entre `min` y `max`.
    Scalable,
    /// Sirve para tallas dentro de `threshold` de la suya.
    Threshold,
}

/// Un subdirectorio del tema, tal y como lo declara `index.theme`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirSpec {
    pub name: String,
    pub size: u32,
    pub scale: u32,
    pub kind: DirKind,
    pub min: u32,
    pub max: u32,
    pub threshold: u32,
}

impl DirSpec {
    /// Si este subdirectorio sirve exactamente para la talla pedida.
    #[must_use]
    pub fn matches(&self, size: u32) -> bool {
        match self.kind {
            DirKind::Fixed => self.size == size,
            DirKind::Scalable => self.min <= size && size <= self.max,
            DirKind::Threshold => {
                self.size.saturating_sub(self.threshold) <= size
                    && size <= self.size.saturating_add(self.threshold)
            }
        }
    }

    /// Cuánto se desvía de la talla pedida, para cuando ninguno encaja.
    #[must_use]
    pub fn distance(&self, size: u32) -> u32 {
        match self.kind {
            DirKind::Fixed => self.size.abs_diff(size),
            DirKind::Scalable => {
                if size < self.min {
                    self.min - size
                } else {
                    size.saturating_sub(self.max)
                }
            }
            DirKind::Threshold => {
                let low = self.size.saturating_sub(self.threshold);
                let high = self.size.saturating_add(self.threshold);
                if size < low {
                    low - size
                } else {
                    size.saturating_sub(high)
                }
            }
        }
    }
}

/// Lo que un `index.theme` declara.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeIndex {
    /// Temas de los que hereda, en orden.
    pub inherits: Vec<String>,
    pub dirs: Vec<DirSpec>,
}

/// Trocea un `index.theme`.
///
/// Solo se leen las claves que deciden qué fichero se coge; los nombres
/// traducidos, los comentarios y el resto se ignoran. Un subdirectorio listado
/// en `Directories` pero sin sección propia se descarta: sin talla no se puede
/// decidir nada de él.
#[must_use]
pub fn parse_index_theme(text: &str) -> ThemeIndex {
    let mut inherits = Vec::new();
    let mut listed: Vec<String> = Vec::new();
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current = String::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());

        if current == "Icon Theme" {
            match key {
                "Inherits" => {
                    inherits = value
                        .split(',')
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(String::from)
                        .collect();
                }
                "Directories" | "ScaledDirectories" => {
                    listed.extend(
                        value
                            .split(',')
                            .map(str::trim)
                            .filter(|d| !d.is_empty())
                            .map(String::from),
                    );
                }
                _ => {}
            }
        } else {
            sections
                .entry(current.clone())
                .or_default()
                .insert(key.to_string(), value.to_string());
        }
    }

    let dirs = listed
        .into_iter()
        .filter_map(|name| {
            let section = sections.get(&name)?;
            let number = |key: &str| section.get(key).and_then(|v| v.parse::<u32>().ok());
            let size = number("Size")?;
            let kind = match section.get("Type").map(String::as_str) {
                Some("Fixed") => DirKind::Fixed,
                Some("Scalable") => DirKind::Scalable,
                // El valor por defecto de la spec es `Threshold`.
                _ => DirKind::Threshold,
            };
            Some(DirSpec {
                min: number("MinSize").unwrap_or(size),
                max: number("MaxSize").unwrap_or(size),
                threshold: number("Threshold").unwrap_or(2),
                scale: number("Scale").unwrap_or(1),
                size,
                kind,
                name,
            })
        })
        .collect();

    ThemeIndex { inherits, dirs }
}

/// El nombre del tema de iconos que el escritorio tiene puesto.
///
/// Se mira primero la configuración de KDE y luego la de GTK, que es el orden
/// en que un escritorio Plasma las escribe. Sin ninguna de las dos queda
/// `hicolor`, que la spec obliga a que exista siempre.
#[must_use]
pub fn current_theme_name() -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.as_ref().map(|h| h.join(".config")));

    let Some(config) = config else {
        return "hicolor".to_string();
    };

    if let Some(theme) = ini_value(&config.join("kdeglobals"), "Icons", "Theme") {
        return theme;
    }
    if let Some(theme) = ini_value(
        &config.join("gtk-3.0/settings.ini"),
        "Settings",
        "gtk-icon-theme-name",
    ) {
        return theme;
    }
    // GNOME no escribe ninguno de los dos ficheros: guarda el tema en gsettings.
    if let Some(theme) = gsettings_icon_theme() {
        return theme;
    }
    // Sin nada configurado, un tema de verdad antes que `hicolor`, que apenas
    // trae iconos: una máquina sin configuración de escritorio (un contenedor,
    // un gestor de ventanas suelto) vería una lista sin iconos de carpeta.
    let bases = base_directories();
    FALLBACK_THEMES
        .iter()
        .find(|name| read_index(&bases, name).is_some())
        .map_or_else(|| "hicolor".to_string(), |name| (*name).to_string())
}

/// Temas que se prueban, por orden, cuando el escritorio no dice cuál usa.
const FALLBACK_THEMES: [&str; 3] = ["Adwaita", "breeze", "Humanity"];

/// El tema que GNOME tiene puesto, según `gsettings`. `None` si no hay
/// `gsettings`, si falla o si no devuelve nada utilizable.
fn gsettings_icon_theme() -> Option<String> {
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_gsettings_string(&String::from_utf8_lossy(&output.stdout))
}

/// `'Adwaita'\n` → `Adwaita`. Lo que no venga entre comillas simples no es una
/// cadena de gsettings y se descarta.
fn parse_gsettings_string(text: &str) -> Option<String> {
    let inner = text.trim().strip_prefix('\'')?.strip_suffix('\'')?;
    (!inner.is_empty()).then(|| inner.to_string())
}

/// Saca una clave de una sección de un fichero tipo `.ini`.
fn ini_value(path: &Path, section: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            inside = name == section;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((found, value)) = line.split_once('=')
            && found.trim() == key
        {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Dónde se buscan los temas, en orden de preferencia.
fn base_directories() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"));
        bases.push(data.join("icons"));
        // `~/.icons` es la ubicación antigua; sigue habiendo temas ahí.
        bases.push(home.join(".icons"));
    }

    let dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    bases.extend(
        dirs.split(':')
            .filter(|d| !d.is_empty())
            .map(|d| PathBuf::from(d).join("icons")),
    );
    bases
}

/// Un subdirectorio concreto, ya resuelto a una ruta que existe.
#[derive(Debug, Clone)]
struct Candidate {
    path: PathBuf,
    spec: DirSpec,
}

/// El tema activo con toda su cadena de herencia, listo para buscar.
#[derive(Debug, Clone, Default)]
pub struct IconTheme {
    candidates: Vec<Candidate>,
}

impl IconTheme {
    /// Carga el tema activo y todo lo que hereda.
    #[must_use]
    pub fn load() -> Self {
        Self::named(&current_theme_name())
    }

    /// Carga un tema por nombre.
    #[must_use]
    pub fn named(name: &str) -> Self {
        let bases = base_directories();
        let mut candidates = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        // En anchura: un tema se agota entero antes de pasar a lo que hereda,
        // que es lo que hace que el tema elegido gane siempre.
        let mut pending = vec![name.to_string()];

        while let Some(theme) = pending.first().cloned() {
            pending.remove(0);
            if !seen.insert(theme.clone()) {
                continue;
            }

            let Some(index) = read_index(&bases, &theme) else {
                continue;
            };
            for spec in &index.dirs {
                for base in &bases {
                    let path = base.join(&theme).join(&spec.name);
                    if path.is_dir() {
                        candidates.push(Candidate {
                            path,
                            spec: spec.clone(),
                        });
                    }
                }
            }
            pending.extend(index.inherits);
        }

        // `hicolor` es el respaldo que la spec obliga a que exista; si nadie lo
        // ha nombrado en la cadena, se añade al final.
        if !seen.contains("hicolor")
            && let Some(index) = read_index(&bases, "hicolor")
        {
            for spec in &index.dirs {
                for base in &bases {
                    let path = base.join("hicolor").join(&spec.name);
                    if path.is_dir() {
                        candidates.push(Candidate {
                            path,
                            spec: spec.clone(),
                        });
                    }
                }
            }
        }

        Self { candidates }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// Busca un icono por nombre para una talla.
    ///
    /// Primero los subdirectorios que sirven exactamente para esa talla, en el
    /// orden del tema; si ninguno tiene el icono, el más cercano de los que sí
    /// lo tienen. Es el algoritmo de la spec, y el orden importa: sin él, un
    /// icono de 512 píxeles de `hicolor` puede ganarle al de 16 del tema.
    #[must_use]
    pub fn find(&self, name: &str, size: u32) -> Option<PathBuf> {
        let exact = self
            .candidates
            .iter()
            .filter(|c| c.spec.matches(size))
            .find_map(|c| file_in(&c.path, name));
        if exact.is_some() {
            return exact;
        }

        self.candidates
            .iter()
            .filter_map(|c| file_in(&c.path, name).map(|path| (c.spec.distance(size), path)))
            .min_by_key(|(distance, _)| *distance)
            .map(|(_, path)| path)
    }
}

fn read_index(bases: &[PathBuf], theme: &str) -> Option<ThemeIndex> {
    bases
        .iter()
        .map(|base| base.join(theme).join("index.theme"))
        .find_map(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse_index_theme(&text))
}

/// SVG primero: escala sin pixelarse a cualquier zoom de icono.
const EXTENSIONS: [&str; 3] = ["svg", "png", "xpm"];

fn file_in(dir: &Path, name: &str) -> Option<PathBuf> {
    EXTENSIONS.iter().find_map(|extension| {
        let path = dir.join(format!("{name}.{extension}"));
        path.is_file().then_some(path)
    })
}

/// Resuelve el icono de cada entrada, recordando lo ya buscado.
#[derive(Debug)]
pub struct Icons {
    theme: IconTheme,
    mime: MimeDatabase,
    size: u32,
    /// Nombre de icono → fichero. Guarda también los fallos: buscar dos veces
    /// un icono que no existe cuesta lo mismo que buscarlo una.
    resolved: HashMap<String, Option<PathBuf>>,
}

impl Icons {
    /// Carga el tema del escritorio y la base de tipos para una talla.
    #[must_use]
    pub fn load(size: u32) -> Self {
        Self {
            theme: IconTheme::load(),
            mime: MimeDatabase::load(),
            size,
            resolved: HashMap::new(),
        }
    }

    /// Like [`Icons::load`], with the theme named by the caller instead of the
    /// one the configuration files name. On GNOME the theme is published by
    /// the Settings portal, which `kara-desktop` reads; a `kdeglobals` left
    /// behind by an earlier Plasma session would otherwise win.
    #[must_use]
    pub fn with_theme(theme: &str, size: u32) -> Self {
        Self {
            theme: IconTheme::named(theme),
            mime: MimeDatabase::load(),
            size,
            resolved: HashMap::new(),
        }
    }

    /// The size icons are looked up at.
    #[must_use]
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Looks icons up at another size from now on, forgetting what was found
    /// at the old one. Themes that ship bitmaps (Yaru, Humanity) have a
    /// different file per size, and a 16-pixel PNG scaled up to a 96-pixel
    /// grid cell is a blur.
    pub fn set_size(&mut self, size: u32) {
        if size != self.size {
            self.size = size;
            self.resolved.clear();
        }
    }

    /// El tipo MIME de un nombre, según la base que este resolutor ya tiene
    /// cargada. Se ofrece aquí para que quien pinta la columna «Tipo» no tenga
    /// que cargar una segunda copia de la base entera.
    #[must_use]
    pub fn mime_of(&self, name: &str) -> Option<&str> {
        self.mime.of(name)
    }

    /// El icono de una entrada de listado.
    pub fn of(&mut self, name: &str, kind: EntryKind) -> Option<PathBuf> {
        for candidate in self.names_for(name, kind) {
            if let Some(path) = self.resolve(&candidate) {
                return Some(path);
            }
        }
        None
    }

    /// El icono de una ubicación con nombre propio: `user-home`, `folder-music`.
    ///
    /// Se prueban en orden y se cae a la carpeta genérica, porque no todos los
    /// temas traen los especiales.
    pub fn any_of(&mut self, names: &[&str]) -> Option<PathBuf> {
        names.iter().find_map(|name| self.resolve(name))
    }

    fn resolve(&mut self, name: &str) -> Option<PathBuf> {
        if let Some(found) = self.resolved.get(name) {
            return found.clone();
        }
        let found = self.theme.find(name, self.size);
        self.resolved.insert(name.to_string(), found.clone());
        found
    }

    /// Los nombres de icono a probar, del más específico al más genérico.
    fn names_for(&self, name: &str, kind: EntryKind) -> Vec<String> {
        if kind == EntryKind::Directory {
            return vec!["folder".to_string(), "inode-directory".to_string()];
        }

        let mut names = Vec::new();
        if let Some(mime) = self.mime.of(name) {
            names.push(mime.replace('/', "-"));
            if let Some(generic) = self.mime.generic_icon(mime) {
                names.push(generic.to_string());
            }
            if let Some((media, _)) = mime.split_once('/') {
                names.push(format!("{media}-x-generic"));
            }
        }
        // Sin tipo reconocido, el icono de «un fichero cualquiera».
        names.push("application-octet-stream".to_string());
        names.push("unknown".to_string());
        names.push("text-x-generic".to_string());
        names
    }
}

#[cfg(test)]
mod gsettings_tests {
    use super::parse_gsettings_string;

    #[test]
    fn a_quoted_value_is_unwrapped() {
        assert_eq!(parse_gsettings_string("'Adwaita'\n").as_deref(), Some("Adwaita"));
        assert_eq!(parse_gsettings_string("  'Yaru-dark' ").as_deref(), Some("Yaru-dark"));
    }

    #[test]
    fn anything_else_is_not_a_theme_name() {
        assert_eq!(parse_gsettings_string(""), None);
        assert_eq!(parse_gsettings_string("''"), None);
        assert_eq!(parse_gsettings_string("Adwaita"), None);
        assert_eq!(parse_gsettings_string("No such key"), None);
    }
}
