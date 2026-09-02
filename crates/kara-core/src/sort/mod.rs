//! Ordenación de listados: criterio, sentido, agrupación y clic en cabecera.
//!
//! Cubre las conveniencias «Ordenar por» (`ground/spec/03-vistas.md`), «Sentido de
//! ordenación ascendente/descendente», «Carpetas primero» y «Ordenar con clic en
//! cabecera de columna». Todo aquí es dominio puro: sin I/O, sin Qt y sin leer el
//! entorno, de modo que `kara-ops` pueda ejecutarlo fuera del hilo de UI.
//!
//! # Decisiones que la spec no fija
//!
//! La spec no cierra estos cuatro puntos; se resuelven así y se documentan para que
//! nadie los reinvente:
//!
//! 1. **Extensión** = el segmento posterior al último `.` que no esté en la posición
//!    inicial. Por tanto `Makefile`, `file.` y `.bashrc` tienen extensión vacía, y
//!    `factura.pdf.exe` tiene `exe`. El punto inicial marca «oculto», no extensión, y
//!    tomar el último segmento es lo que evita enmascarar dobles extensiones
//!    engañosas. La cadena vacía es un **valor**, no un ausente.
//! 2. **Enlaces simbólicos**: los resuelve `kara-fs`, que entrega el
//!    [`crate::entry::EntryKind`] ya decidido. `kara-core` no hace I/O, así que no
//!    puede saber si un enlace apunta a un directorio.
//! 3. **Desempate final por el `name` crudo, siempre ascendente**, aunque el sentido
//!    sea descendente. Es lo que garantiza que el orden sea total y reproducible
//!    incluso cuando dos nombres POSIX distintos producen el mismo `display` lossy;
//!    invertirlo con el sentido no aportaría nada y haría el resultado dependiente
//!    del orden de llegada del `scandir`.
//! 4. **Sin plegado de acentos** al ordenar: `accion` y `acción` son entradas
//!    distintas. Plegar acentos es una conveniencia de búsqueda
//!    (`ground/spec/04-busqueda.md`), y aplicarla aquí haría que dos nombres
//!    visualmente distintos cayeran juntos sin explicación.

mod collation;
mod permutation;
mod spec;

pub use collation::{Collation, CollationKey, collation_key, compare_names};
pub use permutation::{invert_permutation, remap_selection, sort_permutation};
pub use spec::{
    ColumnId, DirectoryGrouping, SortKey, SortOrder, SortSpec, column_for_sort_key,
    sort_key_for_column,
};

use core::cmp::Ordering;
use std::time::SystemTime;

use crate::entry::{EntryKind, FileEntry, MetadataKey, MetadataValue};

/// Único tipo de error del módulo.
///
/// Todos los fallos son de contrato del llamante: columna inexistente, permutación
/// corrupta o estado persistido corrupto. Ordenar datos válidos **nunca** falla.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SortError {
    /// La columna no corresponde a ningún criterio conocido.
    #[error("column `{0}` maps to no sort key")]
    UnknownColumn(String),
    /// La columna existe pero no es ordenable.
    #[error("column `{0}` is not sortable")]
    UnsortableColumn(String),
    /// El criterio persistido no existe en esta versión.
    #[error("unknown sort key `{0}`")]
    UnknownSortKey(String),
    /// La cadena persistida no tiene la forma esperada.
    #[error("malformed sort spec `{0}`")]
    MalformedSpec(String),
    /// La secuencia recibida no es una permutación de `0..len`.
    #[error("not a valid permutation")]
    InvalidPermutation,
    /// Un índice seleccionado no existe en el listado.
    #[error("selection index {index} out of range for {len} entries")]
    SelectionOutOfRange {
        /// Índice ofensivo.
        index: usize,
        /// Longitud del listado.
        len: usize,
    },
}

/// Cadena de comparación normativa entre dos entradas.
///
/// Se aplica exactamente en este orden:
///
/// 1. **Grupo** por [`crate::entry::EntryKind`] según `spec.grouping`. El grupo
///    **nunca** se invierte con [`SortOrder::Descending`]: con
///    [`DirectoryGrouping::First`] las carpetas siguen arriba también en
///    descendente.
/// 2. **Disponibilidad** del criterio: una entrada con el valor ausente (`None`, o
///    una clave que la bolsa de metadatos no aporta) va **siempre al final**, tanto
///    en ascendente como en descendente. Nunca sube a lo alto al invertir el sentido.
/// 3. **Valor** del criterio, con el sentido aplicado.
/// 4. **Desempate por nombre** con la collation activa y el sentido aplicado.
/// 5. **Desempate final por el `name` crudo** (bytes), siempre ascendente.
///
/// Devuelve `Equal` solo si ambas entradas tienen el mismo basename crudo — con una
/// única excepción: con [`SortKey::Unsorted`] devuelve siempre `Equal`, de modo que
/// una ordenación estable deja el listado intacto y ni el sentido ni la agrupación
/// se aplican.
///
/// Es total (antisimétrica, transitiva y reflexiva) y no falla con ningún dato,
/// incluidos instantes anteriores a `UNIX_EPOCH` y nombres que no son UTF-8 válido.
///
/// # Rendimiento
///
/// Cada llamada decora **ambas** entradas de cero (normaliza a NFC y construye su
/// [`CollationKey`]), así que sirve para comparar dos entradas sueltas o en tests,
/// pero **no** para ordenar un listado: usarlo como comparador de un `sort_by` externo
/// deshace el ahorro de *decorate-sort-undecorate* y vuelve a asignar memoria en cada
/// comparación. Para ordenar, usar [`sort_entries`] o [`sort_permutation`], que
/// precomputan la decoración una sola vez por entrada.
#[must_use]
pub fn compare_entries(a: &FileEntry, b: &FileEntry, spec: &SortSpec) -> Ordering {
    let da = decorate_one(a, spec);
    let db = decorate_one(b, spec);
    compare_decorated(&da, &db, spec)
}

/// Representación reducida y comparable del valor de un criterio, calculada una sola
/// vez por entrada. Comparar dos variantes distintas (dato inconsistente en
/// [`crate::entry::MetadataValue`]: la misma [`crate::entry::MetadataKey`] aportando
/// tipos distintos entre entradas) ordena por un **rango fijo de variante** — nunca
/// `Equal` — para no romper la transitividad del orden total; ver
/// [`value_slot_rank`].
enum ValueSlot {
    /// Texto, comparado con la collation activa.
    Text(CollationKey),
    /// Magnitud sin signo.
    Number(u64),
    /// Magnitud con signo.
    SignedNumber(i64),
    /// Instante.
    Time(SystemTime),
    /// Dimensiones `(ancho, alto)`: ordena por área y luego por ancho.
    Area { area: u64, width: u32 },
}

/// Rango fijo de variante, usado solo cuando dos entradas aportan tipos distintos
/// para la misma clave de metadato (dato inconsistente). Ver la nota de
/// [`crate::entry::MetadataValue`]: no puede ser `Ordering::Equal` sin arriesgar la
/// transitividad del orden total, porque el "empate" dependería de con qué otra
/// entrada se compara.
fn value_slot_rank(value: &ValueSlot) -> u8 {
    match value {
        ValueSlot::Text(_) => 0,
        ValueSlot::Number(_) => 1,
        ValueSlot::SignedNumber(_) => 2,
        ValueSlot::Time(_) => 3,
        ValueSlot::Area { .. } => 4,
    }
}

fn compare_value_slots(a: &ValueSlot, b: &ValueSlot) -> Ordering {
    match (a, b) {
        (ValueSlot::Text(x), ValueSlot::Text(y)) => x.cmp(y),
        (ValueSlot::Number(x), ValueSlot::Number(y)) => x.cmp(y),
        (ValueSlot::SignedNumber(x), ValueSlot::SignedNumber(y)) => x.cmp(y),
        (ValueSlot::Time(x), ValueSlot::Time(y)) => x.cmp(y),
        (
            ValueSlot::Area {
                area: area_a,
                width: width_a,
            },
            ValueSlot::Area {
                area: area_b,
                width: width_b,
            },
        ) => area_a.cmp(area_b).then_with(|| width_a.cmp(width_b)),
        // Mismatched variants: inconsistent data, ordered by a fixed variant rank
        // (never `Equal`, see `value_slot_rank`).
        _ => value_slot_rank(a).cmp(&value_slot_rank(b)),
    }
}

/// Extensión normativa: el segmento posterior al último `.` que no esté en la
/// posición inicial. Ver la decisión documentada en el módulo.
fn extension_of(display: &str) -> &str {
    match display.rfind('.') {
        None | Some(0) => "",
        Some(idx) => &display[idx + 1..],
    }
}

fn metadata_to_value(value: &MetadataValue, collation: &Collation) -> ValueSlot {
    match value {
        MetadataValue::Text(text) => ValueSlot::Text(collation_key(text, collation)),
        MetadataValue::Unsigned(n) => ValueSlot::Number(*n),
        MetadataValue::Integer(n) => ValueSlot::SignedNumber(*n),
        MetadataValue::Time(t) => ValueSlot::Time(*t),
        MetadataValue::Pair(width, height) => ValueSlot::Area {
            area: u64::from(*width) * u64::from(*height),
            width: *width,
        },
    }
}

/// Valor del criterio para una entrada, o `None` si está ausente. No se llama para
/// [`SortKey::Name`] ni [`SortKey::Unsorted`], que no tienen etapa de valor.
fn extract_value(entry: &FileEntry, key: &SortKey, collation: &Collation) -> Option<ValueSlot> {
    match key {
        SortKey::Name | SortKey::Unsorted => None,
        SortKey::Extension => Some(ValueSlot::Text(collation_key(
            // Directories have no extension: `My.folder` is not a `.folder`
            // file. They get the empty string, which is a *value* and not an
            // absent one, so they group with the extensionless files instead of
            // falling to the end under the availability rule.
            if entry.kind == EntryKind::Directory {
                ""
            } else {
                extension_of(&entry.display)
            },
            collation,
        ))),
        SortKey::Size => entry.size.map(ValueSlot::Number),
        SortKey::Modified => entry.modified.map(ValueSlot::Time),
        SortKey::Created => entry.created.map(ValueSlot::Time),
        SortKey::Accessed => entry.accessed.map(ValueSlot::Time),
        SortKey::Kind => entry
            .type_label
            .as_deref()
            .map(|label| ValueSlot::Text(collation_key(label, collation))),
        SortKey::Location => entry
            .location
            .as_ref()
            .map(|path| ValueSlot::Text(collation_key(&path.to_string_lossy(), collation))),
        SortKey::Metadata(key) => entry
            .extra
            .get(key)
            .map(|value| metadata_to_value(value, collation)),
    }
}

/// Rango de agrupación por [`EntryKind`] según `grouping`. Nunca se invierte con el
/// sentido: es plano y ascendente siempre.
fn group_rank(kind: EntryKind, grouping: DirectoryGrouping) -> u8 {
    match grouping {
        DirectoryGrouping::Mixed => 0,
        DirectoryGrouping::First => {
            if kind == EntryKind::Directory {
                0
            } else {
                1
            }
        }
        DirectoryGrouping::Last => {
            if kind == EntryKind::Directory {
                1
            } else {
                0
            }
        }
    }
}

/// Datos de una entrada precomputados una sola vez, para que comparar muchas veces
/// (ordenar, buscar el punto de inserción) no asigne memoria.
struct Decoration<'a> {
    group: u8,
    value: Option<ValueSlot>,
    name_key: CollationKey,
    raw_name: &'a std::ffi::OsStr,
}

fn decorate_one<'a>(entry: &'a FileEntry, spec: &SortSpec) -> Decoration<'a> {
    let value = if spec.key == SortKey::Name || spec.key == SortKey::Unsorted {
        None
    } else {
        extract_value(entry, &spec.key, &spec.collation)
    };
    Decoration {
        group: group_rank(entry.kind, spec.grouping),
        value,
        name_key: collation_key(&entry.display, &spec.collation),
        raw_name: entry.name.as_os_str(),
    }
}

fn decorate<'a>(entries: &'a [FileEntry], spec: &SortSpec) -> Vec<Decoration<'a>> {
    entries
        .iter()
        .map(|entry| decorate_one(entry, spec))
        .collect()
}

/// La cadena de comparación normativa, operando sobre datos ya precomputados.
fn compare_decorated(a: &Decoration<'_>, b: &Decoration<'_>, spec: &SortSpec) -> Ordering {
    if spec.key == SortKey::Unsorted {
        return Ordering::Equal;
    }
    if a.group != b.group {
        return a.group.cmp(&b.group);
    }
    if spec.key != SortKey::Name {
        match (&a.value, &b.value) {
            (None, None) => {}
            (None, Some(_)) => return Ordering::Greater,
            (Some(_), None) => return Ordering::Less,
            (Some(x), Some(y)) => {
                let cmp = compare_value_slots(x, y);
                let cmp = if spec.order == SortOrder::Descending {
                    cmp.reverse()
                } else {
                    cmp
                };
                if cmp != Ordering::Equal {
                    return cmp;
                }
            }
        }
    }
    let name_cmp = a.name_key.cmp(&b.name_key);
    let name_cmp = if spec.order == SortOrder::Descending {
        name_cmp.reverse()
    } else {
        name_cmp
    };
    if name_cmp != Ordering::Equal {
        return name_cmp;
    }
    a.raw_name.cmp(b.raw_name)
}

/// Ordena el listado in situ, de forma **estable**.
///
/// Dos entradas que comparan `Equal` conservan su orden de entrada. Con
/// [`SortKey::Unsorted`] no toca nada.
///
/// El comparador no debe asignar memoria: las claves de collation se precomputan una
/// sola vez por entrada.
pub fn sort_entries(entries: &mut [FileEntry], spec: &SortSpec) {
    if entries.len() < 2 {
        return;
    }
    let perm = sort_permutation(entries, spec);
    apply_permutation(entries, &perm);
}

/// Reordena `entries` in situ según `perm` (`perm[nuevo] == viejo`), usando solo
/// intercambios: nunca clona una [`FileEntry`].
fn apply_permutation(entries: &mut [FileEntry], perm: &[usize]) {
    let mut visited = vec![false; perm.len()];
    for start in 0..perm.len() {
        if visited[start] {
            continue;
        }
        let mut current = start;
        loop {
            visited[current] = true;
            let next = perm[current];
            if next == start {
                break;
            }
            entries.swap(current, next);
            current = next;
        }
    }
}

/// Punto de inserción de una entrada nueva en un listado ya ordenado.
///
/// Búsqueda binaria con **límite superior**: el índice devuelto queda detrás de todas
/// las entradas que comparan `Equal` con `entry`. Insertar ahí deja el slice ordenado
/// según `spec`.
///
/// Existe para que un evento de `inotify` o un tamaño de carpeta recién calculado no
/// obliguen a reordenar 100 000 entradas. Con [`SortKey::Unsorted`] devuelve
/// `sorted.len()`.
#[must_use]
pub fn insertion_index(sorted: &[FileEntry], entry: &FileEntry, spec: &SortSpec) -> usize {
    let newcomer = decorate_one(entry, spec);
    let mut lo = 0usize;
    let mut hi = sorted.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let mid_decoration = decorate_one(&sorted[mid], spec);
        if compare_decorated(&mid_decoration, &newcomer, spec) == Ordering::Greater {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// Criterios ofrecibles para este listado.
///
/// Devuelve todos los criterios intrínsecos más un [`SortKey::Metadata`] por cada
/// clave que aporte al menos una entrada. Evita ofrecer «Dimensiones» en una carpeta
/// sin imágenes.
///
/// El orden de salida es determinista y es exactamente el de declaración de
/// [`SortKey`], con las variantes [`SortKey::Metadata`] expandidas en su posición y
/// ordenadas entre sí por [`crate::entry::MetadataKey`]:
/// `Name`, `Extension`, `Size`, `Modified`, `Created`, `Accessed`, `Kind`,
/// `Location`, los metadatos presentes, y `Unsorted`.
///
/// Con un listado vacío devuelve solo los criterios intrínsecos.
#[must_use]
pub fn available_keys(entries: &[FileEntry]) -> Vec<SortKey> {
    // Collect borrowed keys first: with many entries sharing few distinct metadata
    // keys, this clones at most once per *distinct* key instead of once per entry.
    let mut present: std::collections::BTreeSet<&MetadataKey> = std::collections::BTreeSet::new();
    for entry in entries {
        for key in entry.extra.keys() {
            present.insert(key);
        }
    }

    let mut keys = Vec::with_capacity(9 + present.len());
    keys.push(SortKey::Name);
    keys.push(SortKey::Extension);
    keys.push(SortKey::Size);
    keys.push(SortKey::Modified);
    keys.push(SortKey::Created);
    keys.push(SortKey::Accessed);
    keys.push(SortKey::Kind);
    keys.push(SortKey::Location);
    for key in present {
        keys.push(SortKey::Metadata(key.clone()));
    }
    keys.push(SortKey::Unsorted);
    keys
}
