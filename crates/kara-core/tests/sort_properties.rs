//! Propiedades del comparador y de las primitivas de permutación.
//!
//! Estas pruebas cubren los casos que un ejemplo concreto no puede cerrar: orden
//! total, validez de la permutación, idempotencia, composabilidad por grupos y
//! corrección de `insertion_index`.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use proptest::prelude::*;

use kara_core::{
    Collation, DirectoryGrouping, EntryKind, FileEntry, MetadataBag, MetadataKey, MetadataValue,
    SortKey, SortOrder, SortSpec, compare_entries, insertion_index, invert_permutation,
    remap_selection, sort_entries, sort_permutation,
};

// ------------------------------------------------------------ estrategias ---

/// Alfabeto pequeño a propósito: fuerza colisiones, empates y tramos numéricos.
fn name_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just("a".to_string()),
            Just("A".to_string()),
            Just("b".to_string()),
            Just(".".to_string()),
            Just("0".to_string()),
            Just("1".to_string()),
            Just("10".to_string()),
            Just("01".to_string()),
            Just("\u{f3}".to_string()),
            Just("o\u{301}".to_string()),
            Just("-".to_string()),
        ],
        0..6usize,
    )
    .prop_map(|parts| parts.concat())
}

fn time_strategy() -> impl Strategy<Value = SystemTime> {
    (-5i64..5i64).prop_map(|secs| {
        if secs >= 0 {
            UNIX_EPOCH + Duration::from_secs(secs.unsigned_abs())
        } else {
            UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
        }
    })
}

fn metadata_strategy() -> impl Strategy<Value = MetadataBag> {
    prop::collection::vec(
        (
            prop_oneof![
                Just(MetadataKey::Dimensions),
                Just(MetadataKey::Duration),
                Just(MetadataKey::Custom(Cow::Borrowed("bpm"))),
            ],
            prop_oneof![
                (0u32..4, 0u32..4).prop_map(|(w, h)| MetadataValue::Pair(w, h)),
                (0u64..4).prop_map(MetadataValue::Unsigned),
                (-2i64..2).prop_map(MetadataValue::Integer),
                name_strategy().prop_map(MetadataValue::Text),
            ],
        ),
        0..3usize,
    )
    .prop_map(|pairs| {
        let mut bag = MetadataBag::new();
        for (k, v) in pairs {
            bag.insert(k, v);
        }
        bag
    })
}

fn entry_strategy() -> impl Strategy<Value = FileEntry> {
    (
        name_strategy(),
        any::<bool>(),
        proptest::option::of(0u64..4),
        proptest::option::of(time_strategy()),
        proptest::option::of(time_strategy()),
        proptest::option::of(name_strategy()),
        proptest::option::of(name_strategy()),
        metadata_strategy(),
        any::<bool>(),
    )
        .prop_map(
            |(name, is_dir, size, modified, created, type_label, location, extra, invalid_byte)| {
                let raw = if invalid_byte {
                    let mut bytes = name.clone().into_bytes();
                    bytes.push(0xFF);
                    OsString::from_vec(bytes)
                } else {
                    OsString::from(name.clone())
                };
                let display = raw.to_string_lossy().into_owned();
                FileEntry {
                    name: raw,
                    display,
                    kind: if is_dir {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                    is_symlink: false,
                    symlink_broken: false,
                    is_hidden: name.starts_with('.'),
                    size,
                    modified,
                    created,
                    accessed: modified,
                    type_label,
                    location: location.map(PathBuf::from),
                    extra,
                }
            },
        )
}

fn key_strategy() -> impl Strategy<Value = SortKey> {
    prop_oneof![
        Just(SortKey::Name),
        Just(SortKey::Extension),
        Just(SortKey::Size),
        Just(SortKey::Modified),
        Just(SortKey::Created),
        Just(SortKey::Accessed),
        Just(SortKey::Kind),
        Just(SortKey::Location),
        Just(SortKey::Metadata(MetadataKey::Dimensions)),
        Just(SortKey::Metadata(MetadataKey::Duration)),
        Just(SortKey::Metadata(MetadataKey::Custom(Cow::Borrowed("bpm")))),
        Just(SortKey::Unsorted),
    ]
}

fn spec_strategy() -> impl Strategy<Value = SortSpec> {
    (
        key_strategy(),
        prop_oneof![Just(SortOrder::Ascending), Just(SortOrder::Descending)],
        prop_oneof![
            Just(DirectoryGrouping::First),
            Just(DirectoryGrouping::Last),
            Just(DirectoryGrouping::Mixed)
        ],
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(key, order, grouping, case_sensitive, natural_numeric)| SortSpec {
                key,
                order,
                grouping,
                collation: Collation {
                    case_sensitive,
                    natural_numeric,
                },
            },
        )
}

fn entries_strategy(max: usize) -> impl Strategy<Value = Vec<FileEntry>> {
    prop::collection::vec(entry_strategy(), 0..max)
}

fn is_sorted(entries: &[FileEntry], spec: &SortSpec) -> bool {
    entries
        .windows(2)
        .all(|w| compare_entries(&w[0], &w[1], spec) != core::cmp::Ordering::Greater)
}

// ------------------------------------------------------------ propiedades ---

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// `compare_entries` es antisimétrico para cualquier spec.
    #[test]
    fn compare_entries_is_antisymmetric(a in entry_strategy(), b in entry_strategy(), s in spec_strategy()) {
        prop_assert_eq!(
            compare_entries(&a, &b, &s),
            compare_entries(&b, &a, &s).reverse()
        );
        prop_assert_eq!(compare_entries(&a, &a, &s), core::cmp::Ordering::Equal);
    }

    /// `compare_entries` es transitivo para cualquier spec.
    #[test]
    fn compare_entries_is_transitive(
        a in entry_strategy(),
        b in entry_strategy(),
        c in entry_strategy(),
        s in spec_strategy(),
    ) {
        let ab = compare_entries(&a, &b, &s);
        let bc = compare_entries(&b, &c, &s);
        let ac = compare_entries(&a, &c, &s);
        if ab == bc {
            prop_assert_eq!(ac, ab, "transitividad rota");
        }
        if ab == core::cmp::Ordering::Equal {
            prop_assert_eq!(ac, bc, "la igualdad debe ser sustitutiva");
        }
    }

    /// Fuera de `Unsorted`, `Equal` implica el mismo basename crudo.
    #[test]
    fn equality_implies_same_raw_name(a in entry_strategy(), b in entry_strategy(), s in spec_strategy()) {
        if s.key != SortKey::Unsorted
            && compare_entries(&a, &b, &s) == core::cmp::Ordering::Equal
        {
            prop_assert_eq!(&a.name, &b.name);
        }
    }

    /// `sort_permutation` devuelve siempre una permutación válida de `0..len`.
    #[test]
    fn sort_permutation_is_a_valid_permutation(entries in entries_strategy(12), s in spec_strategy()) {
        let perm = sort_permutation(&entries, &s);
        prop_assert_eq!(perm.len(), entries.len());
        let unique: BTreeSet<usize> = perm.iter().copied().collect();
        prop_assert_eq!(unique.len(), entries.len());
        prop_assert!(perm.iter().all(|i| *i < entries.len()));

        let inverse = invert_permutation(&perm).map_err(|e| TestCaseError::fail(e.to_string()))?;
        for (new_index, old_index) in perm.iter().enumerate() {
            prop_assert_eq!(inverse[*old_index], new_index);
        }
    }

    /// `sort_permutation` y `sort_entries` describen exactamente la misma ordenación.
    #[test]
    fn sort_permutation_agrees_with_sort_entries(entries in entries_strategy(12), s in spec_strategy()) {
        let perm = sort_permutation(&entries, &s);
        let via_perm: Vec<FileEntry> = perm.iter().filter_map(|i| entries.get(*i).cloned()).collect();
        let mut via_sort = entries.clone();
        sort_entries(&mut via_sort, &s);
        prop_assert_eq!(via_perm, via_sort);
    }

    /// El resultado está realmente ordenado según el propio comparador.
    #[test]
    fn sort_entries_produces_a_sorted_slice(entries in entries_strategy(12), s in spec_strategy()) {
        let mut sorted = entries.clone();
        sort_entries(&mut sorted, &s);
        prop_assert_eq!(sorted.len(), entries.len());
        if s.key != SortKey::Unsorted {
            prop_assert!(is_sorted(&sorted, &s));
        } else {
            prop_assert_eq!(&sorted, &entries);
        }
    }

    /// Idempotencia: `sort(sort(x)) == sort(x)`.
    #[test]
    fn sorting_is_idempotent(entries in entries_strategy(12), s in spec_strategy()) {
        let mut once = entries;
        sort_entries(&mut once, &s);
        let mut twice = once.clone();
        sort_entries(&mut twice, &s);
        prop_assert_eq!(once, twice);
    }

    /// Un sub-slice contiguo del listado ordenado ya está ordenado, y ordenar una
    /// partición por grupos produce la subsecuencia correspondiente.
    #[test]
    fn sorting_composes_with_grouping(entries in entries_strategy(12), s in spec_strategy()) {
        let mut full = entries.clone();
        sort_entries(&mut full, &s);

        if !full.is_empty() {
            let mid = full.len() / 2;
            let mut slice = full[mid..].to_vec();
            sort_entries(&mut slice, &s);
            prop_assert_eq!(&slice, &full[mid..].to_vec());
        }

        if s.key != SortKey::Unsorted {
            for kind in [EntryKind::Directory, EntryKind::File] {
                let mut group: Vec<FileEntry> = entries
                    .iter()
                    .filter(|e| e.kind == kind)
                    .cloned()
                    .collect();
                sort_entries(&mut group, &s);
                let expected: Vec<FileEntry> = full
                    .iter()
                    .filter(|e| e.kind == kind)
                    .cloned()
                    .collect();
                prop_assert_eq!(group, expected);
            }
        }
    }

    /// Insertar en `insertion_index` deja el slice ordenado.
    #[test]
    fn insertion_index_keeps_the_slice_sorted(
        entries in entries_strategy(12),
        newcomer in entry_strategy(),
        s in spec_strategy(),
    ) {
        let mut sorted = entries;
        sort_entries(&mut sorted, &s);

        let idx = insertion_index(&sorted, &newcomer, &s);
        prop_assert!(idx <= sorted.len());

        let mut inserted = sorted.clone();
        inserted.insert(idx, newcomer.clone());

        if s.key == SortKey::Unsorted {
            prop_assert_eq!(idx, sorted.len());
        } else {
            prop_assert!(is_sorted(&inserted, &s), "insertar en {} rompio el orden", idx);
            // Limite superior: queda detras de todos los que comparan Equal.
            if idx > 0 {
                prop_assert_ne!(
                    compare_entries(&sorted[idx - 1], &newcomer, &s),
                    core::cmp::Ordering::Greater
                );
            }
            if idx < sorted.len() {
                prop_assert_eq!(
                    compare_entries(&sorted[idx], &newcomer, &s),
                    core::cmp::Ordering::Greater
                );
            }
        }
    }

    /// `remap_selection` conserva el conjunto seleccionado, no solo su tamaño.
    #[test]
    fn remap_selection_preserves_the_selected_entries(
        entries in entries_strategy(12),
        picks in prop::collection::vec(any::<bool>(), 0..12),
        s in spec_strategy(),
    ) {
        let selection: BTreeSet<usize> = picks
            .iter()
            .enumerate()
            .filter(|(i, keep)| **keep && *i < entries.len())
            .map(|(i, _)| i)
            .collect();

        let perm = sort_permutation(&entries, &s);
        let remapped = remap_selection(&selection, &perm)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(remapped.len(), selection.len());

        let mut sorted = entries.clone();
        sort_entries(&mut sorted, &s);
        let picked: Vec<&FileEntry> = remapped.iter().filter_map(|i| sorted.get(*i)).collect();
        let expected: Vec<&FileEntry> = {
            let mut v: Vec<&FileEntry> = selection.iter().filter_map(|i| entries.get(*i)).collect();
            v.sort_by(|a, b| compare_entries(a, b, &s));
            v
        };
        prop_assert_eq!(picked.len(), expected.len());
        for (a, b) in picked.iter().zip(expected.iter()) {
            prop_assert_eq!(compare_entries(a, b, &s), core::cmp::Ordering::Equal);
        }
    }

    /// Round-trip textual de `SortSpec` para cualquier combinación generada.
    #[test]
    fn sort_spec_text_round_trip(s in spec_strategy()) {
        let text = s.to_string();
        let parsed: Result<SortSpec, _> = text.parse();
        prop_assert_eq!(parsed, Ok(s));
    }
}
