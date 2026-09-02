//! Ordenación basada en permutaciones, para preservar selección y foco.

use std::collections::BTreeSet;

use crate::entry::FileEntry;
use crate::sort::{SortError, SortKey, SortSpec};

/// Devuelve los índices originales en el nuevo orden, sin mover ni clonar entradas.
///
/// `perm[nuevo] == viejo`. Es la primitiva que permite a la capa superior conservar
/// la selección y dejar el elemento con foco visible al re-ordenar.
///
/// Es estable, igual que [`crate::sort::sort_entries`]: dos entradas que comparan
/// `Equal` conservan su orden relativo de entrada. Con
/// [`crate::sort::SortKey::Unsorted`] devuelve la identidad `0..len`.
#[must_use]
pub fn sort_permutation(entries: &[FileEntry], spec: &SortSpec) -> Vec<usize> {
    // `SortKey::Unsorted` is the identity by definition (`compare_decorated` already
    // returns `Equal` for it): skip building collation keys entirely rather than
    // decorating every entry just to discover that. On 100k entries this is the
    // difference between a no-op and the single most expensive allocation-heavy path
    // in the module.
    if spec.key == SortKey::Unsorted {
        return (0..entries.len()).collect();
    }
    let decorations = super::decorate(entries, spec);
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|&i, &j| super::compare_decorated(&decorations[i], &decorations[j], spec));
    order
}

/// Invierte una permutación: `perm[nuevo] == viejo` pasa a `salida[viejo] == nuevo`.
///
/// # Errores
///
/// [`SortError::InvalidPermutation`] si algún índice está fuera de rango o se repite,
/// es decir si la entrada no es una permutación de `0..perm.len()`.
pub fn invert_permutation(perm: &[usize]) -> Result<Vec<usize>, SortError> {
    let len = perm.len();
    let mut inverse: Vec<Option<usize>> = vec![None; len];
    for (new_index, &old_index) in perm.iter().enumerate() {
        if old_index >= len || inverse[old_index].is_some() {
            return Err(SortError::InvalidPermutation);
        }
        inverse[old_index] = Some(new_index);
    }
    let mut result = Vec::with_capacity(len);
    for slot in inverse {
        match slot {
            Some(new_index) => result.push(new_index),
            None => return Err(SortError::InvalidPermutation),
        }
    }
    Ok(result)
}

/// Traslada un conjunto de índices seleccionados al nuevo orden.
///
/// Los índices de entrada son posiciones en el listado **antes** de ordenar y los de
/// salida, posiciones **después**. Cumple «conservando la selección» al cambiar de
/// criterio y «manteniendo el criterio y la selección» al invertir el sentido: el
/// conjunto resultante tiene siempre la misma cardinalidad que el de entrada.
///
/// # Errores
///
/// - [`SortError::InvalidPermutation`] si `perm` no es una permutación válida. Se
///   comprueba antes que los índices seleccionados.
/// - [`SortError::SelectionOutOfRange`] si un índice seleccionado no existe en el
///   listado, con `len == perm.len()`.
pub fn remap_selection(
    selection: &BTreeSet<usize>,
    perm: &[usize],
) -> Result<BTreeSet<usize>, SortError> {
    let inverse = invert_permutation(perm)?;
    let len = perm.len();
    let mut result = BTreeSet::new();
    for &old_index in selection {
        if old_index >= len {
            return Err(SortError::SelectionOutOfRange { index: old_index, len });
        }
        result.insert(inverse[old_index]);
    }
    Ok(result)
}
