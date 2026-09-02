//! Representación de una entrada de listado, sin I/O.
//!
//! `kara-core` nunca toca el disco: `kara-fs` construye estos valores a partir de
//! `scandir`/`stat` y esta capa se limita a ordenarlos, agruparlos y filtrarlos.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::SystemTime;

/// Único eje de agrupación de «Carpetas primero».
///
/// Lo calcula `kara-fs` resolviendo el enlace simbólico; `kara-core` no hace I/O y
/// nunca lo deduce del nombre. Un enlace simbólico a directorio llega como
/// [`EntryKind::Directory`]; uno roto, como [`EntryKind::File`].
///
/// El orden de declaración es normativo: `Directory < File`, que es exactamente
/// [`crate::sort::DirectoryGrouping::First`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryKind {
    /// Directorio (o enlace simbólico que resuelve a directorio).
    Directory,
    /// Fichero regular, dispositivo, socket o enlace simbólico roto.
    File,
}

/// Claves de los criterios de ordenación que solo existen si el contenido los aporta.
///
/// El orden de declaración es normativo: fija el orden de salida de
/// [`crate::sort::available_keys`]. Las claves [`MetadataKey::Custom`] se ordenan
/// lexicográficamente entre sí y van después de las intrínsecas.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataKey {
    /// Dimensiones en píxeles de una imagen o un vídeo.
    Dimensions,
    /// Duración de un audio o un vídeo.
    Duration,
    /// Álbum de una pista de audio.
    Album,
    /// Artista de una pista de audio.
    Artist,
    /// Etiquetas del usuario.
    Tags,
    /// Valoración del usuario.
    Rating,
    /// Metadato arbitrario aportado por un extractor externo.
    Custom(Cow<'static, str>),
}

/// Valor tipado de un metadato.
///
/// Reglas de comparación, normativas:
///
/// - [`MetadataValue::Text`] se compara con la misma collation que el nombre.
/// - [`MetadataValue::Unsigned`] e [`MetadataValue::Integer`] se comparan por valor
///   numérico.
/// - [`MetadataValue::Time`] compara `SystemTime` directamente (nunca vía
///   `duration_since`, que falla antes de la época UNIX).
/// - [`MetadataValue::Pair`] cubre las dimensiones `(ancho, alto)` y ordena por área
///   (`ancho * alto`, calculado en `u64` para no desbordar) y luego por ancho.
/// - Comparar dos variantes distintas (el dato es inconsistente: la misma clave de
///   metadato aportando tipos distintos entre entradas) nunca entra en pánico y
///   decide por un rango fijo de variante (el de declaración de este enum), en vez de
///   `Ordering::Equal`: un empate en la etapa de valor que dependiera de *con qué
///   otra entrada* se compara rompería la transitividad del orden total exigida por
///   `crate::sort::compare_entries` (dos comparaciones "empatadas" por separado con
///   una tercera entrada no implican que esas dos comparen igual entre sí cuando una
///   de ellas en realidad sí tiene un valor numérico comparable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataValue {
    /// Texto libre, comparado con la collation activa.
    Text(String),
    /// Magnitud sin signo (bytes, milisegundos, valoración…).
    Unsigned(u64),
    /// Magnitud con signo.
    Integer(i64),
    /// Instante.
    Time(SystemTime),
    /// Par ordenado `(ancho, alto)`.
    Pair(u32, u32),
}

/// Bolsa de metadatos opcionales de una entrada.
///
/// Determina qué criterios ofrece el menú «Ordenar por»: un
/// [`crate::sort::SortKey::Metadata`] solo se ofrece si al menos una entrada del
/// listado aporta esa clave. La iteración es determinista y sigue el orden de
/// [`MetadataKey`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetadataBag(BTreeMap<MetadataKey, MetadataValue>);

impl MetadataBag {
    /// Crea una bolsa vacía.
    #[must_use]
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    /// Inserta un metadato y devuelve el valor anterior, si lo había.
    pub fn insert(&mut self, key: MetadataKey, value: MetadataValue) -> Option<MetadataValue> {
        self.0.insert(key, value)
    }

    /// Devuelve el valor de una clave, o `None` si la entrada no lo aporta.
    #[must_use]
    pub fn get(&self, key: &MetadataKey) -> Option<&MetadataValue> {
        self.0.get(key)
    }

    /// Itera las claves presentes en orden determinista ([`MetadataKey`]).
    pub fn keys(&self) -> impl Iterator<Item = &MetadataKey> {
        self.0.keys()
    }
}

/// Una entrada de listado.
///
/// `name` guarda el basename crudo (los nombres POSIX son bytes, no UTF-8) y es el
/// desempate final que garantiza un orden total; `display` es la conversión lossy que
/// se compara con la collation.
///
/// `size`, `modified`, `created`, `accessed`, `type_label` y `location` son `Option`
/// porque un `stat` puede fallar, un sistema de ficheros puede no tener fecha de
/// creación, el tamaño de una carpeta se calcula en segundo plano y la ubicación solo
/// existe en resultados de búsqueda recursiva. Un valor ausente va **siempre al
/// final** al ordenar, en ambos sentidos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Basename crudo, tal cual lo devuelve el sistema de ficheros.
    pub name: OsString,
    /// Conversión lossy de `name` a `String`, que es lo que se muestra y se compara.
    pub display: String,
    /// Directorio o fichero, ya resuelto por `kara-fs`.
    pub kind: EntryKind,
    /// La entrada es un enlace simbólico.
    pub is_symlink: bool,
    /// El enlace simbólico apunta a un destino inexistente.
    pub symlink_broken: bool,
    /// La entrada está oculta (nombre con punto inicial). No participa en la
    /// ordenación: ocultar es filtrar, no ordenar.
    pub is_hidden: bool,
    /// Tamaño en bytes; `None` mientras no se conozca.
    pub size: Option<u64>,
    /// Fecha de modificación.
    pub modified: Option<SystemTime>,
    /// Fecha de creación, si el sistema de ficheros la expone.
    pub created: Option<SystemTime>,
    /// Fecha de último acceso.
    pub accessed: Option<SystemTime>,
    /// Etiqueta de tipo legible («Documento PDF»), ya resuelta por `kara-fs`.
    pub type_label: Option<String>,
    /// Carpeta contenedora; alimenta la columna «Ubicación» de los resultados de
    /// búsqueda.
    pub location: Option<PathBuf>,
    /// Metadatos dependientes del contenido.
    pub extra: MetadataBag,
}
