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

    // El nombre y la carpeta son los de **antes de borrarlo**, que es lo que el
    // usuario reconoce. `display_path()` es la ruta dentro de la papelera: ahí
    // el fichero puede llevar un sufijo para no chocar con otro, y su carpeta es
    // `files/`, que no le dice a nadie de dónde salió. Solo una entrada sin su
    // `.trashinfo` no sabe su origen y cae al nombre que tiene en la papelera.
    let (original, location): (&std::path::Path, Option<std::path::PathBuf>) = match entry {
        kara_fs::trash::TrashEntry::Item(item) => (&item.original_path, item.original_path.parent().map(std::path::Path::to_path_buf)),
        kara_fs::trash::TrashEntry::MissingFile { original_path, .. } => {
            (original_path, original_path.parent().map(std::path::Path::to_path_buf))
        }
        kara_fs::trash::TrashEntry::MissingInfo { file_path, .. } => (file_path, None),
    };
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
        location,
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

/// Cómo se rotula una pestaña.
///
/// El nombre de la carpeta, no la ruta: en una barra de pestañas no cabe, y lo
/// que distingue una pestaña de otra casi siempre es el último tramo. La raíz
/// no tiene nombre, así que se la llama por lo que es.
#[must_use]
pub fn tab_title(path: &Path, in_trash: bool) -> String {
    if in_trash {
        return "Papelera".to_string();
    }
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => "Sistema de archivos".to_string(),
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

/// Extensiones de lo que se ejecuta o lanza. Con las extensiones ocultas
/// `factura.pdf.exe` se leería `factura.pdf`; estas no se ocultan nunca.
///
/// Se decide por el nombre porque mirar el bit de ejecución es un `stat` por
/// entrada, y en una carpeta de 100 000 ficheros eso es un segundo más en el
/// hilo de la ventana.
pub fn looks_executable(name: &str) -> bool {
    const LAUNCHERS: [&str; 17] = [
        "exe", "bat", "cmd", "com", "msi", "scr", "vbs", "ps1", "sh", "bash", "py", "pl", "rb",
        "run", "appimage", "desktop", "jar",
    ];
    kara_core::filter::base_and_extension(name)
        .is_some_and(|(_, ext)| LAUNCHERS.iter().any(|l| ext.eq_ignore_ascii_case(l)))
}

/// Una ruta lista para pegar en una terminal o en un campo de texto.
///
/// Sin comillas si no hacen falta; con comillas simples si hay espacios u otro
/// carácter que un shell interpretaría (`$`, `&`, `;`…). Las letras acentuadas
/// no cuentan: `Ñandú` se pega tal cual.
#[must_use]
pub fn path_as_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    let safe = |c: char| c.is_alphanumeric() || matches!(c, '_' | '@' | '%' | '+' | '=' | ':' | ',' | '.' | '/' | '-');
    if !text.is_empty() && text.chars().all(safe) {
        return text.into_owned();
    }
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Varias rutas, una por línea, que es lo que pide la spec.
#[must_use]
pub fn paths_as_text(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path_as_text(path))
        .collect::<Vec<_>>()
        .join("\n")
}

/// «12,3 KB (12 595 bytes)»: la cifra legible y la exacta, como Propiedades.
#[must_use]
pub fn size_with_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else {
        format!("{} ({bytes} bytes)", format_size(bytes))
    }
}

/// Suma dos avances de recorrido: lo que ya estaba más lo del tramo en curso.
#[must_use]
pub fn add_progress(
    before: kara_index::size::SizeProgress,
    now: kara_index::size::SizeProgress,
) -> kara_index::size::SizeProgress {
    kara_index::size::SizeProgress {
        files: before.files + now.files,
        directories: before.directories + now.directories,
        logical: before.logical + now.logical,
        on_disk: before.on_disk + now.on_disk,
    }
}

/// Lo que el cálculo de tamaño le cuenta al diálogo: los totales hasta ahora y
/// si ya acabó y si es parcial.
#[derive(Debug, Clone, Copy)]
pub struct SizeUpdate {
    /// Bytes de lo que no es carpeta, que no hace falta recorrer.
    files_bytes: u64,
    totals: kara_index::size::SizeProgress,
    done: bool,
    partial: bool,
}

impl SizeUpdate {
    #[must_use]
    pub fn new(
        files_bytes: u64,
        totals: kara_index::size::SizeProgress,
        done: bool,
        partial: bool,
    ) -> Self {
        Self {
            files_bytes,
            totals,
            done,
            partial,
        }
    }

    /// La fila «Tamaño». Mientras se calcula lleva la cuenta en vivo, como pide
    /// la spec; si acaba incompleto, lo dice en vez de presentarlo definitivo.
    #[must_use]
    pub fn size_text(&self) -> String {
        let logical = self.files_bytes + self.totals.logical;
        if !self.done {
            return format!("Calculando… {}", format_size(logical));
        }
        let mut text = size_with_bytes(logical);
        text.push_str(&format!(" · en disco: {}", format_size(self.totals.on_disk)));
        if self.partial {
            text.push_str(" (parcial: alguna carpeta no se pudo leer)");
        }
        text
    }

    /// La fila «Contiene».
    #[must_use]
    pub fn contents_text(&self) -> String {
        let text = format!(
            "{} archivos, {} carpetas",
            self.totals.files, self.totals.directories
        );
        if self.done {
            text
        } else {
            format!("Calculando… {text}")
        }
    }
}

/// Las filas del diálogo de Propiedades.
#[derive(Debug, Default)]
pub struct PropertyRows {
    pub rows: Vec<(String, String)>,
    /// Posición de «Tamaño» y de «Contiene», que el cálculo en segundo plano
    /// va actualizando.
    pub size_row: Option<usize>,
    pub contents_row: Option<usize>,
}

/// «ana (1000)», o solo el número si el sistema no tiene nombre.
fn owner_text(owner: &kara_fs::props::Owner) -> String {
    match &owner.name {
        Some(name) => format!("{name} ({})", owner.id),
        None => owner.id.to_string(),
    }
}

/// «rw-r--r-- (644)».
fn mode_text(mode: u32) -> String {
    format!("{} ({:o})", kara_fs::props::mode_string(mode), mode & 0o7777)
}

/// Construye las filas para uno o varios elementos.
///
/// `type_label` solo se usa con uno: la descripción del tipo la resuelve el
/// puente contra la base de MIME.
#[must_use]
pub fn properties_rows(infos: &[kara_fs::props::Properties], type_label: Option<&str>) -> PropertyRows {
    let mut out = PropertyRows::default();
    let mut push = |label: &str, value: String| -> usize {
        out.rows.push((label.to_string(), value));
        out.rows.len() - 1
    };

    if let [info] = infos {
        let name = info
            .path
            .file_name()
            .map_or_else(|| info.path.display().to_string(), |n| n.to_string_lossy().into_owned());
        push("Nombre", name);
        push("Tipo", type_label.unwrap_or_default().to_string());
        push(
            "Ubicación",
            info.path
                .parent()
                .map_or_else(|| "/".to_string(), |p| p.display().to_string()),
        );
        if let Some(target) = &info.link_target {
            push("Apunta a", target.display().to_string());
        }
        let folder = info.is_dir && !info.is_symlink;
        let size_row = push(
            "Tamaño",
            if folder {
                "Calculando…".to_string()
            } else {
                size_with_bytes(info.size)
            },
        );
        let contents_row = folder.then(|| push("Contiene", "Calculando…".to_string()));
        push("Modificado", modified_label(info.modified));
        push("Creado", modified_label(info.created));
        push("Último acceso", modified_label(info.accessed));
        push("Propietario", owner_text(&info.owner));
        push("Grupo", owner_text(&info.group));
        push("Permisos", mode_text(info.mode));
        out.size_row = Some(size_row);
        out.contents_row = contents_row;
        return out;
    }

    let folders = infos.iter().filter(|i| i.is_dir && !i.is_symlink).count();
    push(
        "Elementos",
        format!("{} ({} archivos, {folders} carpetas)", infos.len(), infos.len() - folders),
    );
    let parents: std::collections::BTreeSet<_> = infos.iter().map(|i| i.path.parent()).collect();
    if let (1, Some(Some(parent))) = (parents.len(), parents.iter().next()) {
        push("Ubicación", parent.display().to_string());
    }
    let files_bytes: u64 = infos
        .iter()
        .filter(|i| !i.is_dir || i.is_symlink)
        .map(|i| i.size)
        .sum();
    let size_row = push(
        "Tamaño",
        if folders > 0 {
            "Calculando…".to_string()
        } else {
            size_with_bytes(files_bytes)
        },
    );
    let contents_row = (folders > 0).then(|| push("Contiene", "Calculando…".to_string()));
    out.size_row = Some(size_row);
    out.contents_row = contents_row;
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

    #[test]
    fn un_lanzador_nunca_pierde_su_extension_aunque_se_oculten() {
        assert!(looks_executable("factura.pdf.exe"));
        assert!(looks_executable("INSTALAR.SH"));
        assert!(looks_executable("Kara.AppImage"));
        assert!(!looks_executable("factura.pdf"));
        assert!(!looks_executable("Makefile"));
        // Un dotfile no tiene extension: el punto marca «oculto».
        assert!(!looks_executable(".sh"));
    }

    #[test]
    fn una_ruta_sin_nada_raro_se_copia_sin_comillas() {
        assert_eq!(path_as_text(Path::new("/home/ana/Ñandú/notas.md")), "/home/ana/Ñandú/notas.md");
    }

    #[test]
    fn los_espacios_y_lo_que_un_shell_interpreta_llevan_comillas() {
        assert_eq!(path_as_text(Path::new("/tmp/mi carpeta")), "'/tmp/mi carpeta'");
        assert_eq!(path_as_text(Path::new("/tmp/a$b")), "'/tmp/a$b'");
        // Una comilla simple dentro se cierra, se escapa y se reabre.
        assert_eq!(path_as_text(Path::new("/tmp/it's")), "'/tmp/it'\\''s'");
    }

    #[test]
    fn varias_rutas_van_una_por_linea() {
        let paths = [PathBuf::from("/a"), PathBuf::from("/b c")];
        assert_eq!(paths_as_text(&paths), "/a\n'/b c'");
    }

    fn info(path: &str, is_dir: bool, size: u64) -> kara_fs::props::Properties {
        kara_fs::props::Properties {
            path: PathBuf::from(path),
            is_dir,
            is_symlink: false,
            link_target: None,
            size,
            modified: None,
            created: None,
            accessed: None,
            mode: 0o644,
            owner: kara_fs::props::Owner { id: 1000, name: Some("ana".into()) },
            group: kara_fs::props::Owner { id: 1000, name: None },
        }
    }

    #[test]
    fn un_fichero_enseña_su_tamano_exacto_y_sus_permisos() {
        let rows = properties_rows(&[info("/home/ana/notas.md", false, 12_595)], Some("Documento Markdown"));
        let get = |label: &str| rows.rows.iter().find(|(l, _)| l == label).map(|(_, v)| v.as_str());
        assert_eq!(get("Nombre"), Some("notas.md"));
        assert_eq!(get("Ubicación"), Some("/home/ana"));
        assert_eq!(get("Tamaño"), Some("12.3 KB (12595 bytes)"));
        assert_eq!(get("Propietario"), Some("ana (1000)"));
        assert_eq!(get("Grupo"), Some("1000"));
        assert_eq!(get("Permisos"), Some("rw-r--r-- (644)"));
        assert!(rows.contents_row.is_none());
    }

    #[test]
    fn una_carpeta_se_calcula_en_segundo_plano() {
        let rows = properties_rows(&[info("/home/ana/Docs", true, 4096)], Some("Carpeta de archivos"));
        let size = rows.size_row.map(|i| rows.rows[i].1.as_str());
        assert_eq!(size, Some("Calculando…"));
        assert!(rows.contents_row.is_some());
    }

    #[test]
    fn varios_elementos_cuentan_archivos_y_carpetas() {
        let rows = properties_rows(&[info("/a/x", false, 10), info("/a/d", true, 4096)], None);
        assert_eq!(rows.rows[0].1, "2 (1 archivos, 1 carpetas)");
        assert!(rows.rows.iter().any(|(l, v)| l == "Ubicación" && v == "/a"));
        assert!(rows.contents_row.is_some());
    }

    #[test]
    fn un_calculo_parcial_lo_dice() {
        let totals = kara_index::size::SizeProgress { files: 3, directories: 2, logical: 2048, on_disk: 4096 };
        let done = SizeUpdate::new(0, totals, true, true);
        assert!(done.size_text().contains("parcial"));
        let live = SizeUpdate::new(0, totals, false, false);
        assert!(live.size_text().starts_with("Calculando…"));
        assert_eq!(done.contents_text(), "3 archivos, 2 carpetas");
    }

    // ---- what the columns and labels say -----------------------------------

    fn entry(name: &str, kind: EntryKind) -> FileEntry {
        FileEntry {
            name: std::ffi::OsString::from(name),
            display: name.to_string(),
            kind,
            is_symlink: false,
            symlink_broken: false,
            is_hidden: false,
            size: None,
            modified: None,
            created: None,
            accessed: None,
            type_label: None,
            location: None,
            extra: kara_core::entry::MetadataBag::new(),
        }
    }

    fn column(id: &'static str) -> ColumnId {
        ColumnId(std::borrow::Cow::Borrowed(id))
    }

    #[test]
    fn una_carpeta_no_tiene_tamano_y_un_fichero_vacio_si() {
        let folder = entry("docs", EntryKind::Directory);
        assert_eq!(size_label(&folder), "", "vacío, no «0 B»: no se midió");

        let mut empty = entry("vacio.txt", EntryKind::File);
        empty.size = Some(0);
        assert_eq!(size_label(&empty), "0 B");
        empty.size = Some(2048);
        assert_eq!(size_label(&empty), "2.0 KB");
    }

    #[test]
    fn el_tamano_con_bytes_no_repite_los_bytes_de_lo_pequeno() {
        assert_eq!(size_with_bytes(0), "0 bytes");
        assert_eq!(size_with_bytes(1023), "1023 bytes");
        assert_eq!(size_with_bytes(1024), "1.0 KB (1024 bytes)");
    }

    #[test]
    fn la_fecha_ausente_se_deja_en_blanco_y_la_presente_lleva_dia_mes_ano_y_hora() {
        assert_eq!(modified_label(None), "");

        let text = modified_label(Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(86_400 * 366)));
        // dd/mm/aaaa hh:mm, sea cual sea la zona del equipo.
        let bytes = text.as_bytes();
        assert_eq!(text.len(), 16, "{text}");
        assert_eq!((bytes[2], bytes[5], bytes[10], bytes[13]), (b'/', b'/', b' ', b':'));
    }

    #[test]
    fn el_tipo_intrinseco_cubre_carpetas_y_enlaces_rotos_y_deja_el_resto_a_la_base_de_mime() {
        assert_eq!(
            intrinsic_type_label(&entry("docs", EntryKind::Directory)),
            Some("Carpeta de archivos")
        );
        assert_eq!(intrinsic_type_label(&entry("a.txt", EntryKind::File)), None);

        // Un enlace roto no se describe por su extensión: el destino no existe.
        let mut broken = entry("a.txt", EntryKind::File);
        broken.symlink_broken = true;
        assert_eq!(intrinsic_type_label(&broken), Some("Enlace roto"));
        let mut broken_dir = entry("d", EntryKind::Directory);
        broken_dir.symlink_broken = true;
        assert_eq!(intrinsic_type_label(&broken_dir), Some("Enlace roto"));
    }

    #[test]
    fn cada_columna_conocida_tiene_etiqueta_y_la_desconocida_se_enseña_tal_cual() {
        assert_eq!(column_label(&column("name")), "Nombre");
        assert_eq!(column_label(&column("modified")), "Fecha de modificación");
        assert_eq!(column_label(&column("meta/exposicion")), "meta/exposicion");
        for id in ["name", "extension", "size", "modified", "created", "accessed", "kind", "location"] {
            assert_ne!(column_label(&column(id)), id, "{id} debería estar traducida");
        }
    }

    #[test]
    fn las_celdas_salen_de_la_entrada_y_lo_que_no_aplica_queda_vacio() {
        let mut file = entry("informe.final.pdf", EntryKind::File);
        file.size = Some(1536);
        file.location = Some(PathBuf::from("/home/ana/Docs"));

        assert_eq!(cell_value(&file, &column("name")), "informe.final.pdf");
        assert_eq!(cell_value(&file, &column("size")), "1.5 KB");
        assert_eq!(cell_value(&file, &column("extension")), "pdf");
        assert_eq!(cell_value(&file, &column("location")), "/home/ana/Docs");
        // «Tipo» lo inyecta el puente; una columna de metadatos aún no tiene extractor.
        assert_eq!(cell_value(&file, &column("kind")), "");
        assert_eq!(cell_value(&file, &column("artist")), "");
        assert_eq!(cell_value(&entry("README", EntryKind::File), &column("extension")), "");
    }

    #[test]
    fn una_pestana_se_llama_como_su_carpeta_y_la_raiz_y_la_papelera_tienen_nombre_propio() {
        assert_eq!(tab_title(Path::new("/home/ana/Documentos"), false), "Documentos");
        assert_eq!(tab_title(Path::new("/"), false), "Sistema de archivos");
        assert_eq!(tab_title(Path::new("/home/ana"), true), "Papelera");
    }

    #[test]
    fn las_secciones_y_las_ubicaciones_se_leen_en_espanol() {
        use kara_core::tree::SectionId;
        assert_eq!(section_label(SectionId::QuickAccess), "Acceso rápido");
        assert_eq!(section_label(SectionId::ThisComputer), "Este equipo");

        let place = |kind, label: Option<&str>| kara_fs::places::Place {
            path: PathBuf::from("/mnt/usb"),
            kind,
            label: label.map(std::ffi::OsString::from),
        };
        assert_eq!(place_label(&place(PlaceKind::Home, None)), "Inicio");
        assert_eq!(place_label(&place(PlaceKind::Downloads, None)), "Descargas");
        assert_eq!(place_label(&place(PlaceKind::Volume, Some("PENDRIVE"))), "PENDRIVE");
        // Sin nombre legible no queda más que la ruta.
        assert_eq!(place_label(&place(PlaceKind::Volume, None)), "/mnt/usb");
    }

    #[test]
    fn los_iconos_de_cada_ubicacion_acaban_siempre_en_algo_que_todo_tema_trae() {
        for kind in [
            PlaceKind::Home,
            PlaceKind::Desktop,
            PlaceKind::Downloads,
            PlaceKind::Documents,
            PlaceKind::Pictures,
            PlaceKind::Music,
            PlaceKind::Videos,
            PlaceKind::Root,
            PlaceKind::Volume,
        ] {
            let icons = place_icons(kind);
            assert!(icons.len() >= 2, "{kind:?}: hace falta un respaldo");
            assert!(icons.contains(&"folder") || icons.contains(&"drive-harddisk"), "{kind:?}");
        }
    }

    fn date() -> kara_fs::trash::DeletionDate {
        kara_fs::trash::DeletionDate { year: 2026, month: 9, day: 30, hour: 12, minute: 0, second: 0 }
    }

    #[test]
    fn una_fila_de_papelera_lleva_el_nombre_y_la_carpeta_de_antes_de_borrar_no_los_de_dentro_de_la_papelera() {
        // En la papelera se llama `viejo.2.txt` y vive en `files/`; el usuario
        // lo borró de `/home/ana/Docs` y lo conoce como `viejo.txt`.
        let item = kara_fs::trash::TrashEntry::Item(kara_fs::trash::TrashedItem {
            original_path: PathBuf::from("/home/ana/Docs/viejo.txt"),
            trashed_path: PathBuf::from("/home/ana/.local/share/Trash/files/viejo.2.txt"),
            info_path: PathBuf::from("/home/ana/.local/share/Trash/info/viejo.2.txt.trashinfo"),
            deletion_date: date(),
            kind: kara_fs::trash::TrashKind::Home,
            top_dir: None,
            bytes_copied: None,
        });
        let row = trash_row(&item, 7);

        assert_eq!(row.display, "viejo.txt");
        assert_eq!(row.location.as_deref(), Some(Path::new("/home/ana/Docs")));
        // Ordenar reordena las filas: el índice viaja con la fila, no con su sitio.
        assert_eq!(trash_index_of(&row), Some(7));
        assert_eq!(trash_index_of(&entry("normal.txt", EntryKind::File)), None);
    }

    #[test]
    fn una_entrada_sin_su_fichero_sigue_sabiendo_de_donde_salio() {
        let item = kara_fs::trash::TrashEntry::MissingFile {
            info_path: PathBuf::from("/t/info/a.trashinfo"),
            original_path: PathBuf::from("/home/ana/Docs/viejo.txt"),
            deletion_date: date(),
            kind: kara_fs::trash::TrashKind::Home,
            top_dir: None,
        };
        let row = trash_row(&item, 0);
        assert_eq!(row.display, "viejo.txt");
        assert_eq!(row.location.as_deref(), Some(Path::new("/home/ana/Docs")));
    }

    #[test]
    fn una_entrada_sin_trashinfo_no_inventa_un_origen() {
        let item = kara_fs::trash::TrashEntry::MissingInfo {
            file_path: PathBuf::from("/t/files/huerfano.bin"),
            kind: kara_fs::trash::TrashKind::Home,
            top_dir: None,
        };
        let row = trash_row(&item, 3);
        assert_eq!(row.display, "huerfano.bin");
        assert_eq!(row.location, None, "su carpeta de origen se desconoce");
        assert_eq!(trash_index_of(&row), Some(3));
    }
}
