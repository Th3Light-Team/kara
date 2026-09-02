//! Dominio puro de Kara: representación de entradas del sistema de ficheros,
//! ordenación, agrupación, filtrado, historial de navegación y breadcrumb.
//!
//! Esta capa es la base de la pila (`ui → ops → {fs, index} → core`) y **no hace
//! I/O ni conoce Qt**. Todo lo que vive aquí debe ser testeable sin tocar el disco.

#![forbid(unsafe_code)]

pub mod breadcrumb;
pub mod entry;
pub mod history;
pub mod sort;

pub use breadcrumb::{Collapsed, Segment, SegmentKind, collapse, segments};
pub use entry::{EntryKind, FileEntry, MetadataBag, MetadataKey, MetadataValue};
pub use history::{History, HistoryEntry, ViewState};
pub use sort::{
    Collation, CollationKey, ColumnId, DirectoryGrouping, SortError, SortKey, SortOrder,
    SortOverrides, SortSpec,
    available_keys, collation_key, column_for_sort_key, compare_entries, compare_names,
    insertion_index, invert_permutation, remap_selection, sort_entries, sort_key_for_column,
    sort_permutation,
};
