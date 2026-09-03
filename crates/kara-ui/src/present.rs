//! Presentación: cómo se enseña al usuario lo que el dominio ya decidió.
//!
//! Esta capa no lista, no ordena y no navega; traduce valores a texto y texto a
//! rutas. Vive aquí y no en QML porque son decisiones que hay que poder probar,
//! y vive aquí y no en `kara-core` porque son decisiones de idioma y de vista.

use std::path::{Path, PathBuf};

use kara_core::FileEntry;
use kara_core::breadcrumb::{Segment, SegmentKind};
use kara_core::entry::EntryKind;
use kara_core::sort::ColumnId;
use kara_core::filter::base_and_extension;
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

/// Pone en mayúscula la inicial de una descripción del sistema.
///
/// Hace falta porque la columna mezcla dos orígenes: «Carpeta de archivos» sale
/// de aquí y «documento JSON» de la traducción de FreeDesktop, que va en
/// minúscula. Verlas juntas parece un error.
///
/// No se toca lo que ya empieza por mayúscula ni los nombres cuya segunda letra
/// lo es: «eDonkey» y «iPod» se escriben así a propósito y volverlos «EDonkey»
/// sería peor que la inconsistencia que se quiere arreglar.
#[must_use]
pub fn capitalize_type(description: &str) -> String {
    let mut chars = description.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    if !first.is_lowercase() || chars.next().is_some_and(char::is_uppercase) {
        return description.to_string();
    }
    first.to_uppercase().collect::<String>() + &description[first.len_utf8()..]
}

/// Etiqueta de tipo cuando el sistema no describe el fichero.
///
/// Es lo que hace Windows con lo que no conoce: «Archivo JSON», con la
/// extensión en mayúsculas. Sin extensión no queda nada que decir salvo que es
/// un archivo.
#[must_use]
pub fn fallback_type_label(name: &str) -> String {
    match base_and_extension(name) {
        Some((_, extension)) => format!("Archivo {}", extension.to_uppercase()),
        None => "Archivo".to_string(),
    }
}

/// Etiqueta de tipo de una entrada que no necesita consultar la base de MIME.
///
/// Devuelve `None` para los ficheros corrientes, que sí la necesitan.
#[must_use]
pub fn intrinsic_type_label(entry: &FileEntry) -> Option<&'static str> {
    // Un enlace roto no tiene tipo: el destino no existe, así que preguntarle a
    // la base por su extensión describiría algo que no está.
    if entry.symlink_broken {
        return Some("Enlace roto");
    }
    match entry.kind {
        // Como el Explorador. «Carpeta» a secas se confunde con la columna
        // Nombre cuando la carpeta se llama, precisamente, «Carpeta».
        EntryKind::Directory => Some("Carpeta de archivos"),
        EntryKind::File => None,
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

/// Fecha de modificación, como la enseña el Explorador: día, mes, año y hora.
///
/// Se escribe en hora local, que es lo que el usuario reconoce; el sistema la
/// guarda en UTC. Sin segundos: en una columna estrecha no aportan y hacen la
/// lista más difícil de barrer con la vista.
#[must_use]
pub fn modified_label(modified: Option<std::time::SystemTime>) -> String {
    let Some(modified) = modified else {
        return String::new();
    };
    let local: chrono::DateTime<chrono::Local> = modified.into();
    local.format("%d/%m/%Y %H:%M").to_string()
}

/// Clave con la que una entrada de la papelera lleva su sitio en el listado.
///
/// La vista ordena y filtra `FileEntry`, no entradas de papelera, así que hace
/// falta poder volver de la fila que el usuario señaló a la entrada real. La
/// bolsa de metadatos existe justo para que una capa cuelgue de la entrada algo
/// que el dominio no conoce.
pub const TRASH_INDEX: &str = "kara/trash-index";

/// Construye la fila que representa a una entrada de la papelera.
///
/// `index` es su posición en el listado que la produjo, y viaja en la bolsa de
/// metadatos porque ordenar reordena las filas y el índice de pantalla deja de
/// coincidir con el del listado.
#[must_use]
pub fn trash_row(entry: &kara_fs::trash::TrashEntry, index: usize) -> FileEntry {
    use kara_core::entry::{MetadataKey, MetadataValue};

    let original = entry.display_path().to_path_buf();
    let name = original
        .file_name()
        .map_or_else(|| std::ffi::OsString::from("?"), |name| name.to_os_string());

    let mut row = FileEntry {
        display: name.to_string_lossy().into_owned(),
        name,
        // Restaurar una carpeta y restaurar un fichero es lo mismo, y mirar el
        // disco por cada entrada solo para el icono no compensa aquí.
        kind: EntryKind::File,
        is_symlink: false,
        symlink_broken: false,
        is_hidden: false,
        size: None,
        modified: None,
        created: None,
        accessed: None,
        type_label: None,
        // La carpeta de la que salió, que es lo que hace útil la papelera:
        // sin ella no se sabe qué se está restaurando.
        location: original.parent().map(std::path::Path::to_path_buf),
        extra: kara_core::entry::MetadataBag::new(),
    };
    row.extra.insert(
        MetadataKey::Custom(std::borrow::Cow::Borrowed(TRASH_INDEX)),
        MetadataValue::Unsigned(index as u64),
    );
    row
}

/// Recupera de una fila la entrada de papelera de la que salió.
#[must_use]
pub fn trash_index_of(entry: &FileEntry) -> Option<usize> {
    use kara_core::entry::{MetadataKey, MetadataValue};

    match entry.extra.get(&MetadataKey::Custom(std::borrow::Cow::Borrowed(
        TRASH_INDEX,
    ))) {
        Some(MetadataValue::Unsigned(index)) => usize::try_from(*index).ok(),
        _ => None,
    }
}

/// Cómo se lee la cabecera de una columna.
///
/// Un identificador que esta versión no conoce —una columna de metadatos que
/// aporte un extractor -— se enseña tal cual en vez de esconderse: es más útil
/// ver «meta/exposicion» que una columna sin nombre.
#[must_use]
pub fn column_label(id: &ColumnId) -> String {
    match id.0.as_ref() {
        "name" => "Nombre".to_string(),
        "extension" => "Extensión".to_string(),
        "size" => "Tamaño".to_string(),
        "modified" => "Fecha de modificación".to_string(),
        "created" => "Fecha de creación".to_string(),
        "accessed" => "Último acceso".to_string(),
        "kind" => "Tipo".to_string(),
        "location" => "Ubicación".to_string(),
        "dimensions" => "Dimensiones".to_string(),
        "duration" => "Duración".to_string(),
        "album" => "Álbum".to_string(),
        "artist" => "Artista".to_string(),
        "tags" => "Etiquetas".to_string(),
        "rating" => "Valoración".to_string(),
        "thumbnail" => "Miniatura".to_string(),
        "preview" => "Vista previa".to_string(),
        "icon" => "Icono".to_string(),
        other => other.to_string(),
    }
}

/// El valor de una entrada en una columna.
///
/// Vacío cuando esa entrada no tiene nada que decir ahí: una carpeta no tiene
/// tamaño y un texto no tiene duración. Vacío es la respuesta honesta; un cero
/// diría que se midió y dio cero.
#[must_use]
pub fn cell_value(entry: &FileEntry, id: &ColumnId) -> String {
    match id.0.as_ref() {
        "name" => entry.display.clone(),
        "size" => size_label(entry),
        "modified" => modified_label(entry.modified),
        "created" => modified_label(entry.created),
        "accessed" => modified_label(entry.accessed),
        // «Tipo» no sale de aquí: la descripción la resuelve el puente contra
        // la base de MIME, que tiene memoria y no cabe en una función pura.
        // `Snapshot` la inyecta ya resuelta.
        "kind" => String::new(),
        "extension" => kara_core::filter::base_and_extension(&entry.display)
            .map(|(_, extension)| extension.to_string())
            .unwrap_or_default(),
        "location" => entry
            .location
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        // Las columnas de metadatos las alimenta un extractor que todavía no
        // existe; salen vacías en vez de inventarse un valor.
        _ => String::new(),
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

/// Los nombres de icono de una ubicación, del más específico al genérico.
///
/// No todos los temas traen los especiales —una carpeta de descargas con su
/// flecha, un disco— y por eso la lista acaba siempre en algo que sí existe.
#[must_use]
pub fn place_icons(kind: PlaceKind) -> &'static [&'static str] {
    match kind {
        PlaceKind::Home => &["user-home", "folder-home", "folder"],
        PlaceKind::Desktop => &["user-desktop", "folder-desktop", "folder"],
        PlaceKind::Downloads => &["folder-download", "folder-downloads", "folder"],
        PlaceKind::Documents => &["folder-documents", "folder"],
        PlaceKind::Pictures => &["folder-pictures", "folder-images", "folder"],
        PlaceKind::Music => &["folder-music", "folder"],
        PlaceKind::Videos => &["folder-videos", "folder-video", "folder"],
        PlaceKind::Root => &["drive-harddisk", "computer", "folder"],
        PlaceKind::Volume => &["drive-removable-media", "drive-harddisk", "folder"],
    }
}

/// Convierte una ruta del disco en la URL que QML necesita para cargarla.
///
/// Hay que escapar: un tema de iconos puede vivir en una carpeta con espacios,
/// y `file:///.../Tela ubuntu/folder.svg` sin escapar no carga y no dice por
/// qué. Se conservan los caracteres que la RFC 3986 llama no reservados más la
/// barra, que aquí es separador y no dato.
#[must_use]
pub fn file_url(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut url = String::with_capacity(raw.len() + 8);
    url.push_str("file://");

    for byte in raw.as_bytes() {
        let safe = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/');
        if safe {
            url.push(*byte as char);
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
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
    fn la_descripcion_del_sistema_arranca_en_mayuscula() {
        assert_eq!(capitalize_type("documento JSON"), "Documento JSON");
        assert_eq!(capitalize_type("Documento PDF"), "Documento PDF");
    }

    #[test]
    fn un_nombre_que_empieza_en_minuscula_a_proposito_se_respeta() {
        assert_eq!(capitalize_type("eDonkey link"), "eDonkey link");
        assert_eq!(capitalize_type("iPod"), "iPod");
    }

    #[test]
    fn una_descripcion_vacia_no_rompe_nada() {
        assert_eq!(capitalize_type(""), "");
    }

    #[test]
    fn una_inicial_acentuada_tambien_sube() {
        assert_eq!(capitalize_type("índice de archivos"), "Índice de archivos");
    }

    #[test]
    fn sin_tipo_conocido_la_extension_hace_de_etiqueta() {
        assert_eq!(fallback_type_label("apuntes.qwerty"), "Archivo QWERTY");
    }

    #[test]
    fn sin_extension_no_se_inventa_una_etiqueta() {
        assert_eq!(fallback_type_label("LEEME"), "Archivo");
        // Un fichero oculto no tiene extensión: `.bashrc` se llama así entero.
        assert_eq!(fallback_type_label(".bashrc"), "Archivo");
    }

    #[test]
    fn una_ruta_corriente_da_una_url_corriente() {
        assert_eq!(
            file_url(Path::new("/usr/share/icons/breeze/places/16/folder.svg")),
            "file:///usr/share/icons/breeze/places/16/folder.svg"
        );
    }

    #[test]
    fn los_espacios_y_los_acentos_se_escapan() {
        // Sin escapar, QML no carga el fichero y no dice por que.
        assert_eq!(
            file_url(Path::new("/home/ana/Mis cosas/á.svg")),
            "file:///home/ana/Mis%20cosas/%C3%A1.svg"
        );
    }

    #[test]
    fn la_almohadilla_no_parte_la_url() {
        // `#` abre el fragmento de una URL: sin escapar, todo lo que va detras
        // se pierde.
        assert_eq!(file_url(Path::new("/a/b#c.png")), "file:///a/b%23c.png");
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
