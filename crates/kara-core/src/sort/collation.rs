//! Collation determinista e independiente del locale.
//!
//! El listado de Kara debe salir **byte a byte idéntico** bajo `LC_ALL=C` y bajo
//! `LC_ALL=es_ES.UTF-8`: queda prohibida cualquier función de comparación de cadenas
//! dependiente del locale del sistema (sea de la libc o de ICU) y cualquier
//! comparación que lea el entorno.

use core::cmp::Ordering;

use unicode_normalization::UnicodeNormalization;

/// Parámetros de comparación de texto.
///
/// Los valores por defecto son los que exige la spec («insensible a mayúsculas por
/// defecto en el nombre y con orden natural/numérico»); se exponen precisamente
/// porque la spec dice «por defecto».
///
/// **No existe plegado de acentos**: `accion` y `acción` son nombres distintos y se
/// ordenan por punto de código. Plegar acentos es una conveniencia de *búsqueda*
/// (`04-busqueda.md`), no de ordenación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Collation {
    /// Si es `true`, las mayúsculas y minúsculas son caracteres distintos y se
    /// comparan por punto de código.
    pub case_sensitive: bool,
    /// Si es `true`, los tramos de dígitos se comparan por valor numérico
    /// (`archivo2` antes que `archivo10`).
    pub natural_numeric: bool,
}

impl Default for Collation {
    fn default() -> Self {
        Self {
            case_sensitive: false,
            natural_numeric: true,
        }
    }
}

/// Clave de comparación precomputada una sola vez por entrada
/// (*decorate-sort-undecorate*).
///
/// Existe para que el comparador **no asigne memoria**: es lo que hace viable ordenar
/// 100 000 entradas sin bloquear la UI.
///
/// # Invariante normativa
///
/// Para cualesquiera `a`, `b` y una misma [`Collation`] `c`:
///
/// ```text
/// collation_key(a, &c).cmp(&collation_key(b, &c)) == compare_names(a, b, &c)
/// ```
///
/// Comparar claves construidas con [`Collation`] distintas no está definido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollationKey {
    /// Secuencia de átomos (caracteres plegados o tramos numéricos), en el orden en
    /// que aparecen en `original`. Determina la comparación primaria.
    atoms: Vec<CollationAtom>,
    /// Forma NFC del nombre, **sin plegar**: es el último desempate (puntos de código
    /// originales) y la fuente de los tramos numéricos de `atoms`.
    original: String,
}

// Internal representation of a single unit of comparison. Private on purpose: the
// implementation is free to replace this type entirely, as long as the `Ord`
// invariant documented on `CollationKey` holds.
//
// `Digits` stores byte offsets into the owning `CollationKey::original` instead of an
// owned string, so building a key allocates only twice (the `original` string and the
// `atoms` vector), regardless of how many numeric runs it contains.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CollationAtom {
    /// Carácter no numérico (o dígito cuando `natural_numeric` está desactivado), ya
    /// plegado según la collation.
    Char(char),
    /// Tramo maximal de dígitos ASCII: `start`/`len` delimitan el tramo completo en
    /// `original`; `zeros` es el número de ceros a la izquierda (acotado a `len - 1`
    /// para que siempre quede al menos un dígito significativo).
    Digits { start: usize, len: usize, zeros: usize },
}

/// Compara dos átomos, usando `a_src`/`b_src` (los `original` de cada clave) para
/// resolver los tramos numéricos. Devuelve la comparación primaria y, si ambos son
/// tramos numéricos de igual valor pero distinto número de ceros a la izquierda, el
/// desempate pendiente correspondiente (ver [`compare_names`]).
fn compare_atoms(
    a: &CollationAtom,
    a_src: &str,
    b: &CollationAtom,
    b_src: &str,
) -> (Ordering, Option<Ordering>) {
    match (a, b) {
        (
            CollationAtom::Digits {
                start: start_a,
                len: len_a,
                zeros: zeros_a,
            },
            CollationAtom::Digits {
                start: start_b,
                len: len_b,
                zeros: zeros_b,
            },
        ) => {
            let stripped_a = &a_src[start_a + zeros_a..start_a + len_a];
            let stripped_b = &b_src[start_b + zeros_b..start_b + len_b];
            let primary = stripped_a
                .len()
                .cmp(&stripped_b.len())
                .then_with(|| stripped_a.cmp(stripped_b));
            let zero_diff = if primary == Ordering::Equal && zeros_a != zeros_b {
                Some(zeros_a.cmp(zeros_b))
            } else {
                None
            };
            (primary, zero_diff)
        }
        (CollationAtom::Char(char_a), CollationAtom::Char(char_b)) => (char_a.cmp(char_b), None),
        (CollationAtom::Digits { start, zeros, .. }, CollationAtom::Char(char_b)) => {
            (first_char_at(a_src, start + zeros).cmp(char_b), None)
        }
        (CollationAtom::Char(char_a), CollationAtom::Digits { start, zeros, .. }) => {
            (char_a.cmp(&first_char_at(b_src, start + zeros)), None)
        }
    }
}

fn first_char_at(s: &str, byte_offset: usize) -> char {
    s[byte_offset..].chars().next().unwrap_or('0')
}

/// Unicode **simple** case folding of a single `char`: locale-independent, total and
/// always 1:1, so a `CollationAtom::Char` stays a single character.
///
/// Built on [`char::to_lowercase`], which implements Unicode's default case *mapping*
/// and reads no locale — the C library's `strcoll`/`tolower` are the locale-dependent
/// ones, not this. Two characters need their own arm because case folding and case
/// mapping genuinely diverge there, and `to_lowercase` leaves both untouched:
///
/// - Greek final sigma (U+03C2 `ς`) lowercases to itself but folds to U+03C3 `σ`, the
///   same target as capital sigma. Without this arm `"ΟΔΟΣ"` and `"οδός"` — the same
///   word with the same case-insensitive spelling — would sort apart.
/// - The micro sign (U+00B5 `µ`) folds to Greek small mu (U+03BC `μ`).
///
/// Characters whose lowercase mapping expands to several `char`s (U+0130 `İ` becomes
/// `i` + U+0307) are left unfolded: folding them would need a multi-character atom,
/// which would cost an allocation per name and break the 100 k-entry budget. That is
/// the only remaining scope gap, and it is two code points wide rather than the whole
/// of Latin Extended-A/B.
fn case_fold_char(c: char) -> char {
    match c {
        // Case folding, not case mapping: `to_lowercase` leaves these alone.
        '\u{00B5}' => '\u{03BC}',
        '\u{03C2}' => '\u{03C3}',
        _ => {
            let mut lowered = c.to_lowercase();
            match (lowered.next(), lowered.next()) {
                (Some(single), None) => single,
                // Multi-character expansion, or no mapping at all: leave as is.
                _ => c,
            }
        }
    }
}

impl Ord for CollationKey {
    fn cmp(&self, other: &Self) -> Ordering {
        let mut pending_zero_tiebreak: Option<Ordering> = None;
        for (atom_a, atom_b) in self.atoms.iter().zip(other.atoms.iter()) {
            let (primary, zero_diff) =
                compare_atoms(atom_a, &self.original, atom_b, &other.original);
            if primary != Ordering::Equal {
                return primary;
            }
            if pending_zero_tiebreak.is_none() {
                pending_zero_tiebreak = zero_diff;
            }
        }
        let length_cmp = self.atoms.len().cmp(&other.atoms.len());
        if length_cmp != Ordering::Equal {
            return length_cmp;
        }
        if let Some(zero_tiebreak) = pending_zero_tiebreak {
            return zero_tiebreak;
        }
        self.original.cmp(&other.original)
    }
}

impl PartialOrd for CollationKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Construye la clave de comparación de un nombre visible.
///
/// Los nombres con bytes inválidos ya llegan como `U+FFFD` en `display`; la colisión
/// que eso provoca la resuelve el desempate final por el `name` crudo de
/// [`crate::entry::FileEntry`].
#[must_use]
pub fn collation_key(display: &str, collation: &Collation) -> CollationKey {
    // `with_capacity` + `extend` (rather than `.collect()`) keeps this to a single
    // allocation: `nfc()`'s iterator can't report an exact size hint, so a bare
    // `collect()` would grow the buffer repeatedly for longer names.
    let mut original = String::with_capacity(display.len());
    original.extend(display.nfc());
    let mut atoms = Vec::with_capacity(original.len());

    let mut chars = original.char_indices().peekable();
    while let Some((start, first)) = chars.next() {
        if collation.natural_numeric && first.is_ascii_digit() {
            let mut end = start + first.len_utf8();
            while let Some(&(_, next)) = chars.peek() {
                if next.is_ascii_digit() {
                    end += next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let run = &original[start..end];
            let run_len = run.len(); // ASCII digits: 1 byte == 1 char.
            let leading_zeros = run.chars().take_while(|&c| c == '0').count();
            let zeros = leading_zeros.min(run_len - 1);
            atoms.push(CollationAtom::Digits {
                start,
                len: run_len,
                zeros,
            });
        } else {
            let folded = if collation.case_sensitive {
                first
            } else {
                // Unicode simple case folding: locale-independent, allocation-free
                // and always 1:1, so `CollationAtom::Char` stays a single character.
                case_fold_char(first)
            };
            atoms.push(CollationAtom::Char(folded));
        }
    }

    CollationKey { atoms, original }
}

/// Compara dos nombres visibles. Es total, determinista e independiente del locale.
///
/// # Algoritmo normativo
///
/// 1. Ambas cadenas se normalizan a **NFC**. Un mismo nombre visual llegado en NFC y
///    en NFD compara `Equal`.
/// 2. Se recorren en paralelo, carácter a carácter:
///    - Si `natural_numeric` está activo y **ambas** posiciones empiezan un dígito
///      ASCII (`0..=9`), se toma de cada cadena el tramo maximal de dígitos y se
///      comparan **por valor**, sin `parse` y sin desbordamiento posible: primero por
///      número de dígitos tras descartar los ceros a la izquierda y, a igual
///      longitud, lexicográficamente. Si el valor difiere, decide. Si empata y los
///      ceros a la izquierda difieren, se anota —solo la primera vez— un desempate
///      pendiente: **menos ceros a la izquierda va primero** (`a1` antes que `a01`).
///      En ambos casos se avanza más allá de los dos tramos.
///    - En cualquier otro caso se comparan los dos caracteres actuales; con
///      `case_sensitive == false` se aplica antes plegado de caja simple a cada uno.
///      La primera diferencia decide; si no la hay, se avanza un carácter en cada
///      cadena.
/// 3. Si una cadena se agota antes que la otra sin diferencias, la más corta es
///    `Less`.
/// 4. Si sigue habiendo empate, decide el desempate de ceros a la izquierda anotado
///    en el paso 2.
/// 5. Si sigue habiendo empate, deciden los puntos de código originales de las
///    cadenas NFC (`README` antes que `readme`).
///
/// Solo devuelve `Equal` si ambas cadenas normalizadas a NFC son idénticas.
#[must_use]
pub fn compare_names(a: &str, b: &str, collation: &Collation) -> Ordering {
    collation_key(a, collation).cmp(&collation_key(b, collation))
}
