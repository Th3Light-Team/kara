//! Criterio, sentido, agrupación y su serialización estable.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use crate::entry::MetadataKey;
use crate::sort::{Collation, SortError};

/// Criterio de ordenación.
///
/// El orden de declaración es normativo: fija el orden de salida de
/// [`crate::sort::available_keys`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SortKey {
    /// Nombre visible, con la collation activa.
    Name,
    /// Extensión (`Ctrl+F4` de Total Commander).
    Extension,
    /// Tamaño en bytes.
    Size,
    /// Fecha de modificación.
    Modified,
    /// Fecha de creación.
    Created,
    /// Fecha de último acceso.
    Accessed,
    /// Etiqueta de tipo legible (`type_label`), **no** [`crate::entry::EntryKind`].
    Kind,
    /// Carpeta contenedora; columna «Ubicación» de los resultados de búsqueda.
    Location,
    /// Criterio dependiente del contenido.
    Metadata(MetadataKey),
    /// Sin ordenar (`Ctrl+F7` de Total Commander): se conserva el orden de llegada
    /// del `scandir`.
    Unsorted,
}

/// Sentido de la ordenación, independiente del criterio y persistido junto a él.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortOrder {
    /// De menor a mayor.
    Ascending,
    /// De mayor a menor.
    Descending,
}

impl SortOrder {
    /// Devuelve el sentido contrario.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// Toggle «Carpetas primero».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DirectoryGrouping {
    /// Carpetas antes que archivos. Es el valor por defecto.
    #[default]
    First,
    /// Carpetas después de los archivos.
    Last,
    /// Sin agrupar: carpetas y archivos mezclados, útil al ordenar por fecha.
    Mixed,
}

/// Estado de ordenación completo de una carpeta.
///
/// Es la unidad que «Recordar la vista por carpeta» persiste y restaura; por eso
/// implementa [`fmt::Display`] y [`FromStr`] con round-trip exacto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortSpec {
    /// Criterio activo.
    pub key: SortKey,
    /// Sentido activo.
    pub order: SortOrder,
    /// Agrupación de carpetas.
    pub grouping: DirectoryGrouping,
    /// Parámetros de comparación de texto.
    pub collation: Collation,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            order: SortOrder::Ascending,
            grouping: DirectoryGrouping::First,
            collation: Collation::default(),
        }
    }
}

impl SortSpec {
    /// Semántica exacta del clic en cabecera de columna (vista Detalles).
    ///
    /// Si la columna ya es el criterio activo, invierte el sentido; si es otra, fija
    /// el criterio y vuelve a [`SortOrder::Ascending`], aunque el sentido anterior
    /// fuese descendente. `grouping` y `collation` se conservan intactos.
    ///
    /// Devuelve un `SortSpec` nuevo, sin mutar, para que la capa superior compare
    /// estados y decida si hace falta re-ordenar.
    ///
    /// # Errores
    ///
    /// - [`SortError::UnknownColumn`] si el identificador no mapea a ningún criterio.
    /// - [`SortError::UnsortableColumn`] si la columna es explícitamente no ordenable.
    pub fn on_header_click(&self, column: &ColumnId) -> Result<SortSpec, SortError> {
        let key = sort_key_for_column(column)?;
        if key == self.key {
            Ok(self.toggled_order())
        } else {
            Ok(self.with_key(key))
        }
    }

    /// Devuelve una copia con otro sentido, conservando el criterio.
    #[must_use]
    pub fn with_order(&self, order: SortOrder) -> SortSpec {
        SortSpec {
            key: self.key.clone(),
            order,
            grouping: self.grouping,
            collation: self.collation,
        }
    }

    /// Devuelve una copia con el sentido invertido, conservando el criterio.
    ///
    /// Es la transición de «Alternar el sentido reordena manteniendo el criterio y la
    /// selección».
    #[must_use]
    pub fn toggled_order(&self) -> SortSpec {
        self.with_order(self.order.toggled())
    }

    /// Devuelve una copia con otro criterio.
    ///
    /// Si el criterio es **distinto** del activo, el sentido se resetea a
    /// [`SortOrder::Ascending`]; si es el mismo, el sentido se conserva.
    #[must_use]
    pub fn with_key(&self, key: SortKey) -> SortSpec {
        let order = if key == self.key {
            self.order
        } else {
            SortOrder::Ascending
        };
        SortSpec {
            key,
            order,
            grouping: self.grouping,
            collation: self.collation,
        }
    }
}

/// Identificador estable de una columna de la vista Detalles.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColumnId(pub Cow<'static, str>);

/// Mapea una columna visible a su criterio de ordenación.
///
/// # Identificadores normativos
///
/// | Columna | Criterio |
/// |---|---|
/// | `name` | [`SortKey::Name`] |
/// | `extension` | [`SortKey::Extension`] |
/// | `size` | [`SortKey::Size`] |
/// | `modified` | [`SortKey::Modified`] |
/// | `created` | [`SortKey::Created`] |
/// | `accessed` | [`SortKey::Accessed`] |
/// | `kind` | [`SortKey::Kind`] |
/// | `location` | [`SortKey::Location`] |
/// | `dimensions` | `Metadata(Dimensions)` |
/// | `duration` | `Metadata(Duration)` |
/// | `album` | `Metadata(Album)` |
/// | `artist` | `Metadata(Artist)` |
/// | `tags` | `Metadata(Tags)` |
/// | `rating` | `Metadata(Rating)` |
/// | `meta/<nombre>` | `Metadata(Custom("<nombre>"))` — columnas añadidas por el usuario |
/// | `thumbnail`, `preview`, `icon` | no ordenables |
///
/// Este vocabulario de columna es deliberadamente distinto del de
/// [`SortSpec::to_string`]/[`SortSpec::from_str`] (que persiste los seis metadatos
/// intrínsecos como `meta/dimensions`, etc. y los de usuario como
/// `meta/custom/<nombre>` escapado): son identificadores de columnas visibles, no
/// texto persistido, y una columna de usuario legítimamente llamada
/// `"dimensions"` produce el id `"meta/dimensions"`, que aquí debe resolver a
/// `Custom("dimensions")` — no colarse en secreto como el criterio intrínseco de
/// mismo nombre. `meta/` con el resto vacío no es ninguna columna.
///
/// # Errores
///
/// - [`SortError::UnknownColumn`] si el identificador no corresponde a ninguna
///   columna conocida ni a un metadato registrado (incluido `meta/` sin nombre).
/// - [`SortError::UnsortableColumn`] para las columnas explícitamente no ordenables.
pub fn sort_key_for_column(column: &ColumnId) -> Result<SortKey, SortError> {
    let name: &str = column.0.as_ref();
    match name {
        "name" => Ok(SortKey::Name),
        "extension" => Ok(SortKey::Extension),
        "size" => Ok(SortKey::Size),
        "modified" => Ok(SortKey::Modified),
        "created" => Ok(SortKey::Created),
        "accessed" => Ok(SortKey::Accessed),
        "kind" => Ok(SortKey::Kind),
        "location" => Ok(SortKey::Location),
        "dimensions" => Ok(SortKey::Metadata(MetadataKey::Dimensions)),
        "duration" => Ok(SortKey::Metadata(MetadataKey::Duration)),
        "album" => Ok(SortKey::Metadata(MetadataKey::Album)),
        "artist" => Ok(SortKey::Metadata(MetadataKey::Artist)),
        "tags" => Ok(SortKey::Metadata(MetadataKey::Tags)),
        "rating" => Ok(SortKey::Metadata(MetadataKey::Rating)),
        "thumbnail" | "preview" | "icon" => Err(SortError::UnsortableColumn(name.to_string())),
        other => match other.strip_prefix("meta/") {
            Some("") => Err(SortError::UnknownColumn(name.to_string())),
            Some(custom) => Ok(SortKey::Metadata(MetadataKey::Custom(Cow::Owned(
                custom.to_string(),
            )))),
            None => Err(SortError::UnknownColumn(name.to_string())),
        },
    }
}

/// Maps a sort key back to the column that represents it.
///
/// Inverse of [`sort_key_for_column`]. The Details view needs it to draw the
/// active-header arrow (spec 03-vistas.md:68 "la flecha (▲/▼) lo indica" and
/// :92 "Se muestra una flecha ▲/▼ en la columna activa") without walking every
/// visible column asking which one matches: that walk would put sorting logic
/// in the presentation layer, which the project rules forbid.
///
/// Returns `None` when no column can represent the key:
///
/// - [`SortKey::Unsorted`] is a state, not a column: nothing is highlighted and
///   no arrow is drawn.
/// - `Metadata(Custom(""))` would render as the column id `meta/`, which
///   [`sort_key_for_column`] rejects as unknown. Returning `None` keeps the
///   round-trip total: every id this function yields maps back to its own key.
pub fn column_for_sort_key(key: &SortKey) -> Option<ColumnId> {
    let id: Cow<'static, str> = match key {
        SortKey::Name => Cow::Borrowed("name"),
        SortKey::Extension => Cow::Borrowed("extension"),
        SortKey::Size => Cow::Borrowed("size"),
        SortKey::Modified => Cow::Borrowed("modified"),
        SortKey::Created => Cow::Borrowed("created"),
        SortKey::Accessed => Cow::Borrowed("accessed"),
        SortKey::Kind => Cow::Borrowed("kind"),
        SortKey::Location => Cow::Borrowed("location"),
        SortKey::Metadata(MetadataKey::Dimensions) => Cow::Borrowed("dimensions"),
        SortKey::Metadata(MetadataKey::Duration) => Cow::Borrowed("duration"),
        SortKey::Metadata(MetadataKey::Album) => Cow::Borrowed("album"),
        SortKey::Metadata(MetadataKey::Artist) => Cow::Borrowed("artist"),
        SortKey::Metadata(MetadataKey::Tags) => Cow::Borrowed("tags"),
        SortKey::Metadata(MetadataKey::Rating) => Cow::Borrowed("rating"),
        SortKey::Metadata(MetadataKey::Custom(custom)) if custom.is_empty() => return None,
        SortKey::Metadata(MetadataKey::Custom(custom)) => Cow::Owned(format!("meta/{custom}")),
        SortKey::Unsorted => return None,
    };
    Some(ColumnId(id))
}

/// Serialización textual estable de un [`SortSpec`].
///
/// Formato: `criterio:sentido:agrupacion:banderas`, por ejemplo
/// `modified:desc:dirs-first:ci,natural`.
///
/// - criterio: `name`, `extension`, `size`, `modified`, `created`, `accessed`,
///   `kind`, `location`, `unsorted`, `meta/dimensions`, `meta/duration`,
///   `meta/album`, `meta/artist`, `meta/tags`, `meta/rating`,
///   `meta/custom/<nombre>`.
/// - sentido: `asc` o `desc`.
/// - agrupación: `dirs-first`, `dirs-last` o `mixed`.
/// - banderas: exactamente dos, separadas por coma y en este orden: `cs` o `ci`
///   (sensible / insensible a mayúsculas) y `natural` o `plain`.
impl fmt::Display for SortSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = key_to_text(&self.key);
        let order = match self.order {
            SortOrder::Ascending => "asc",
            SortOrder::Descending => "desc",
        };
        let grouping = match self.grouping {
            DirectoryGrouping::First => "dirs-first",
            DirectoryGrouping::Last => "dirs-last",
            DirectoryGrouping::Mixed => "mixed",
        };
        let case = if self.collation.case_sensitive {
            "cs"
        } else {
            "ci"
        };
        let natural = if self.collation.natural_numeric {
            "natural"
        } else {
            "plain"
        };
        write!(f, "{key}:{order}:{grouping}:{case},{natural}")
    }
}

/// Serializa un [`SortKey`] al identificador normativo usado por [`SortSpec::to_string`]
/// y [`SortSpec::from_str`].
fn key_to_text(key: &SortKey) -> String {
    match key {
        SortKey::Name => "name".to_string(),
        SortKey::Extension => "extension".to_string(),
        SortKey::Size => "size".to_string(),
        SortKey::Modified => "modified".to_string(),
        SortKey::Created => "created".to_string(),
        SortKey::Accessed => "accessed".to_string(),
        SortKey::Kind => "kind".to_string(),
        SortKey::Location => "location".to_string(),
        SortKey::Unsorted => "unsorted".to_string(),
        SortKey::Metadata(meta) => match meta {
            MetadataKey::Dimensions => "meta/dimensions".to_string(),
            MetadataKey::Duration => "meta/duration".to_string(),
            MetadataKey::Album => "meta/album".to_string(),
            MetadataKey::Artist => "meta/artist".to_string(),
            MetadataKey::Tags => "meta/tags".to_string(),
            MetadataKey::Rating => "meta/rating".to_string(),
            MetadataKey::Custom(name) => format!("meta/custom/{}", escape_custom_key(name)),
        },
    }
}

/// Escapa un nombre de [`MetadataKey::Custom`] para que quepa como el primer campo
/// (`criterio`) de la cadena de [`SortSpec::to_string`], que se corta en exactamente
/// cuatro trozos separados por `:`. Solo `:` rompe ese recuento de campos — `,` y `/`
/// no participan en ningún separador del campo de criterio — así que basta con
/// escapar `:` y, para que el escape sea inambiguo, el propio carácter de escape
/// (`%`). Percent-encoding minimalista, sin dependencias.
fn escape_custom_key(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            '%' => out.push_str("%25"),
            ':' => out.push_str("%3A"),
            _ => out.push(ch),
        }
    }
    out
}

/// Inverso de [`escape_custom_key`].
///
/// # Errores
///
/// `None` si `text` contiene una secuencia de escape inválida (un `%` no seguido de
/// `25` o `3A`).
fn unescape_custom_key(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        match (chars.next(), chars.next()) {
            (Some('2'), Some('5')) => out.push('%'),
            (Some('3'), Some('A')) => out.push(':'),
            _ => return None,
        }
    }
    Some(out)
}

/// Inverso de [`key_to_text`].
///
/// # Errores
///
/// [`SortError::UnknownSortKey`] si el texto no corresponde a ningún criterio.
fn key_from_text(text: &str) -> Result<SortKey, SortError> {
    match text {
        "name" => Ok(SortKey::Name),
        "extension" => Ok(SortKey::Extension),
        "size" => Ok(SortKey::Size),
        "modified" => Ok(SortKey::Modified),
        "created" => Ok(SortKey::Created),
        "accessed" => Ok(SortKey::Accessed),
        "kind" => Ok(SortKey::Kind),
        "location" => Ok(SortKey::Location),
        "unsorted" => Ok(SortKey::Unsorted),
        "meta/dimensions" => Ok(SortKey::Metadata(MetadataKey::Dimensions)),
        "meta/duration" => Ok(SortKey::Metadata(MetadataKey::Duration)),
        "meta/album" => Ok(SortKey::Metadata(MetadataKey::Album)),
        "meta/artist" => Ok(SortKey::Metadata(MetadataKey::Artist)),
        "meta/tags" => Ok(SortKey::Metadata(MetadataKey::Tags)),
        "meta/rating" => Ok(SortKey::Metadata(MetadataKey::Rating)),
        other => match other.strip_prefix("meta/custom/") {
            Some(escaped) => match unescape_custom_key(escaped) {
                Some(name) => Ok(SortKey::Metadata(MetadataKey::Custom(Cow::Owned(name)))),
                None => Err(SortError::UnknownSortKey(other.to_string())),
            },
            None => Err(SortError::UnknownSortKey(other.to_string())),
        },
    }
}

/// Reconstruye un [`SortSpec`] persistido.
///
/// El llamante debe caer a [`SortSpec::default`] ante cualquier error: una vista
/// guardada corrupta nunca puede impedir abrir la carpeta.
///
/// # Errores
///
/// - [`SortError::UnknownSortKey`] si el criterio persistido ya no existe pero la
///   cadena tiene la forma correcta.
/// - [`SortError::MalformedSpec`] en cualquier otro fallo de forma: número de campos
///   distinto de cuatro, sentido, agrupación o banderas desconocidos.
impl FromStr for SortSpec {
    type Err = SortError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let malformed = || SortError::MalformedSpec(s.to_string());

        let fields: Vec<&str> = s.split(':').collect();
        let [key_text, order_text, grouping_text, flags_text] =
            <[&str; 4]>::try_from(fields.as_slice()).map_err(|_| malformed())?;

        let key = key_from_text(key_text)?;

        let order = match order_text {
            "asc" => SortOrder::Ascending,
            "desc" => SortOrder::Descending,
            _ => return Err(malformed()),
        };

        let grouping = match grouping_text {
            "dirs-first" => DirectoryGrouping::First,
            "dirs-last" => DirectoryGrouping::Last,
            "mixed" => DirectoryGrouping::Mixed,
            _ => return Err(malformed()),
        };

        let flags: Vec<&str> = flags_text.split(',').collect();
        let [case_text, natural_text] =
            <[&str; 2]>::try_from(flags.as_slice()).map_err(|_| malformed())?;
        let case_sensitive = match case_text {
            "cs" => true,
            "ci" => false,
            _ => return Err(malformed()),
        };
        let natural_numeric = match natural_text {
            "natural" => true,
            "plain" => false,
            _ => return Err(malformed()),
        };

        Ok(SortSpec {
            key,
            order,
            grouping,
            collation: Collation {
                case_sensitive,
                natural_numeric,
            },
        })
    }
}
