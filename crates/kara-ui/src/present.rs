//! Presentación: cómo se enseña al usuario lo que el dominio ya decidió.
//!
//! Esta capa no lista, no ordena y no navega; traduce valores a texto y texto a
//! rutas. Vive aquí y no en QML porque son decisiones que hay que poder probar,
//! y vive aquí y no en `kara-core` porque son decisiones de idioma y de vista.

use std::path::{Path, PathBuf};

use kara_core::FileEntry;
use kara_core::breadcrumb::{Segment, SegmentKind};
use kara_core::entry::EntryKind;
use kara_core::tree::SectionId;
use kara_fs::places::{Place, PlaceKind};

/// Tamaño legible con la escala binaria que usan Windows y Dolphin.
///
/// Los bytes se enseñan enteros: «1,0 B» no aporta nada y «1023 B» sí.
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// El tamaño de una carpeta se deja vacío, no a cero: calcularlo es recursivo y
/// enseñar «0 B» mentiría sobre una carpeta llena.
#[must_use]
pub fn size_label(entry: &FileEntry) -> String {
    entry.size.map(format_size).unwrap_or_default()
}

/// Etiqueta de tipo. Provisional: la spec pide el tipo real por MIME.
#[must_use]
pub fn kind_label(entry: &FileEntry) -> &'static str {
    match entry.kind {
        EntryKind::Directory => "Carpeta",
        EntryKind::File => "Archivo",
    }
}

/// Cómo se lee un tramo de la barra de direcciones.
///
/// `kara-core` deja esto a la vista a propósito: el segmento sabe que es la
/// carpeta personal, pero no que aquí se llama «Inicio».
#[must_use]
pub fn crumb_label(segment: &Segment) -> String {
    match segment.kind {
        // El mismo nombre que en el panel: «Este equipo» es la sección que
        // agrupa la raíz y los volúmenes, no la raíz.
        SegmentKind::Root => "Sistema de archivos".to_string(),
        SegmentKind::Home => "Inicio".to_string(),
        SegmentKind::Directory => segment.name.to_string_lossy().into_owned(),
    }
}

/// Cómo se lee una sección del panel de navegación.
#[must_use]
pub fn section_label(id: SectionId) -> &'static str {
    match id {
        SectionId::QuickAccess => "Acceso rápido",
        SectionId::ThisComputer => "Este equipo",
    }
}

/// Cómo se lee una ubicación del panel.
///
/// Los volúmenes no tienen un nombre nuestro: se llaman como el sistema los
/// montó, y sin punto de montaje legible no queda más que la ruta.
#[must_use]
pub fn place_label(place: &Place) -> String {
    match place.kind {
        PlaceKind::Home => "Inicio".to_string(),
        PlaceKind::Desktop => "Escritorio".to_string(),
        PlaceKind::Downloads => "Descargas".to_string(),
        PlaceKind::Documents => "Documentos".to_string(),
        PlaceKind::Pictures => "Imágenes".to_string(),
        PlaceKind::Music => "Música".to_string(),
        PlaceKind::Videos => "Vídeos".to_string(),
        PlaceKind::Root => "Sistema de archivos".to_string(),
        PlaceKind::Volume => place
            .label
            .as_ref()
            .map_or_else(
                || place.path.to_string_lossy().into_owned(),
                |label| label.to_string_lossy().into_owned(),
            ),
    }
}

/// Convierte lo tecleado en la barra de direcciones en una ruta absoluta.
///
/// Admite lo que pide la spec: `~`, `~/algo`, `$VAR`, `${VAR}` y rutas relativas
/// a la carpeta actual. **No comprueba que exista**: eso es I/O y lo decide quien
/// llame. Devuelve `None` solo si no queda nada que interpretar.
#[must_use]
pub fn expand_path(text: &str, home: Option<&Path>, current: &Path) -> Option<PathBuf> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    let expanded = expand_vars(text);

    // `~` solo cuenta al principio y como segmento completo: un fichero que se
    // llame «~raro» es un nombre válido y no debe convertirse en la carpeta
    // personal.
    let tilde = expanded == "~" || expanded.starts_with("~/");
    let resolved = match (tilde, home) {
        (true, Some(home)) => {
            let rest = expanded.trim_start_matches('~').trim_start_matches('/');
            if rest.is_empty() {
                home.to_path_buf()
            } else {
                home.join(rest)
            }
        }
        // Sin `$HOME` no hay nada a lo que expandir; se deja tal cual en vez de
        // inventar una raíz.
        _ => PathBuf::from(&expanded),
    };

    if resolved.is_absolute() {
        Some(normalize(&resolved))
    } else {
        Some(normalize(&current.join(resolved)))
    }
}

/// Resuelve `.` y `..` sin tocar el disco.
///
/// No se usa `canonicalize`: resolvería los enlaces simbólicos y sacaría al
/// usuario de la ruta que él ve, que es justo lo que la barra de direcciones no
/// debe hacer.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // En la raíz, `..` es la propia raíz.
                if !out.pop() {
                    out.push(Component::RootDir.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Sustituye `$VAR` y `${VAR}`. Una variable que no existe se deja escrita:
/// borrarla convertiría un error de tecleo en una ruta distinta y plausible.
fn expand_vars(text: &str) -> String {
    if !text.contains('$') {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }

        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next();
        }

        let mut name = String::new();
        while let Some(&next) = chars.peek() {
            let part_of_name = if braced {
                next != '}'
            } else {
                next.is_ascii_alphanumeric() || next == '_'
            };
            if !part_of_name {
                break;
            }
            name.push(next);
            chars.next();
        }
        if braced {
            // Se consume la llave de cierre si está; si falta, el texto estaba
            // mal formado y se deja como se escribió.
            if chars.peek() == Some(&'}') {
                chars.next();
            }
        }

        match std::env::var(&name) {
            Ok(value) if !name.is_empty() => out.push_str(&value),
            _ => {
                out.push('$');
                if braced {
                    out.push('{');
                }
                out.push_str(&name);
                if braced {
                    out.push('}');
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/ana")
    }

    #[test]
    fn los_bytes_se_enseñan_enteros_y_el_resto_con_un_decimal() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn la_escala_se_para_en_la_ultima_unidad_en_vez_de_desbordar() {
        assert!(format_size(u64::MAX).ends_with(" TB"));
    }

    #[test]
    fn la_virgulilla_sola_es_la_carpeta_personal() {
        let resolved = expand_path("~", Some(&home()), Path::new("/tmp"));
        assert_eq!(resolved, Some(home()));
    }

    #[test]
    fn la_virgulilla_con_cola_cuelga_de_la_carpeta_personal() {
        let resolved = expand_path("~/Documentos", Some(&home()), Path::new("/tmp"));
        assert_eq!(resolved, Some(PathBuf::from("/home/ana/Documentos")));
    }

    #[test]
    fn una_virgulilla_pegada_a_un_nombre_no_es_la_carpeta_personal() {
        // `~raro` es un nombre de fichero perfectamente válido, y en Linux la
        // expansión `~usuario` del shell no es cosa del explorador.
        let resolved = expand_path("~raro", Some(&home()), Path::new("/tmp"));
        assert_eq!(resolved, Some(PathBuf::from("/tmp/~raro")));
    }

    #[test]
    fn sin_carpeta_personal_la_virgulilla_se_queda_como_esta() {
        let resolved = expand_path("~/x", None, Path::new("/tmp"));
        assert_eq!(resolved, Some(PathBuf::from("/tmp/~/x")));
    }

    #[test]
    fn lo_relativo_cuelga_de_la_carpeta_actual() {
        let resolved = expand_path("sub/otra", Some(&home()), Path::new("/var/log"));
        assert_eq!(resolved, Some(PathBuf::from("/var/log/sub/otra")));
    }

    #[test]
    fn los_puntos_se_resuelven_sin_tocar_el_disco() {
        let resolved = expand_path("../otra", Some(&home()), Path::new("/var/log"));
        assert_eq!(resolved, Some(PathBuf::from("/var/otra")));
        let aqui = expand_path("./x", Some(&home()), Path::new("/var/log"));
        assert_eq!(aqui, Some(PathBuf::from("/var/log/x")));
    }

    #[test]
    fn subir_desde_la_raiz_se_queda_en_la_raiz() {
        let resolved = expand_path("../../..", Some(&home()), Path::new("/"));
        assert_eq!(resolved, Some(PathBuf::from("/")));
    }

    #[test]
    fn las_variables_de_entorno_se_sustituyen() {
        let real = std::env::var("HOME").expect("la prueba necesita $HOME");
        let resolved = expand_path("$HOME/x", None, Path::new("/tmp"));
        assert_eq!(resolved, Some(PathBuf::from(format!("{real}/x"))));

        let llaves = expand_path("${HOME}/x", None, Path::new("/tmp"));
        assert_eq!(llaves, Some(PathBuf::from(format!("{real}/x"))));
    }

    #[test]
    fn una_variable_que_no_existe_se_deja_escrita() {
        // Borrarla convertiría `/$TYPO/etc` en `/etc`, que existe: el usuario
        // acabaría en una carpeta real que no es la que pidió.
        let resolved = expand_path("/$KARA_VARIABLE_QUE_NO_EXISTE/etc", None, Path::new("/"));
        assert_eq!(
            resolved,
            Some(PathBuf::from("/$KARA_VARIABLE_QUE_NO_EXISTE/etc"))
        );
    }

    #[test]
    fn el_texto_vacio_no_es_ninguna_ruta() {
        assert_eq!(expand_path("   ", Some(&home()), Path::new("/tmp")), None);
    }

    #[test]
    fn las_migas_traducen_la_raiz_y_la_carpeta_personal() {
        let segments = kara_core::breadcrumb::segments(
            Path::new("/home/ana/Documentos"),
            Some(Path::new("/home/ana")),
        );
        let labels: Vec<String> = segments.iter().map(crumb_label).collect();
        assert_eq!(labels, ["Sistema de archivos", "home", "Inicio", "Documentos"]);
    }
}
