//! Dominio puro de Kara: representación de entradas del sistema de ficheros,
//! ordenación, agrupación, filtrado, historial de navegación y breadcrumb.
//!
//! Esta capa es la base de la pila (`ui → ops → {fs, index} → core`) y **no hace
//! I/O ni conoce Qt**. Todo lo que vive aquí debe ser testeable sin tocar el disco.

#![forbid(unsafe_code)]

pub mod breadcrumb;
pub mod columns;
pub mod completion;
pub mod entry;
pub mod filter;
pub mod history;
pub mod naming;
pub mod selection;
pub mod sort;
pub mod tabs;
pub mod tree;

pub mod view;
pub mod typeahead;

pub use breadcrumb::{Collapsed, Segment, SegmentKind, collapse, segments};
pub use columns::{
    Column, ColumnLayout, ColumnMemory, ColumnPolicy, ColumnsError, clamp_width, is_removable,
    known_columns, policy_for,
};
pub use entry::{EntryKind, FileEntry, MetadataBag, MetadataKey, MetadataValue};
pub use completion::{Completer, PathInput, Source, Suggestion, Suggestions, split_input};
pub use filter::{NameDisplay, NameFilter, Visibility, base_and_extension};
pub use history::{History, HistoryEntry, ViewState};
pub use naming::{split_name, unique_name};
pub use selection::Selection;
pub use tabs::{CloseOutcome, OpenMode, REOPEN_CAPACITY, Tab, TabId, Tabs};
pub use tree::{Branch, Expandable, Row, RowKind, Section, SectionId, Tree};
pub use view::{FolderView, ViewMemory, ViewMode, ViewSettings};
pub use typeahead::TypeAhead;
pub use sort::{
    Collation, CollationKey, ColumnId, DirectoryGrouping, SortError, SortKey, SortOrder,
    SortOverrides, SortSpec,
    available_keys, collation_key, column_for_sort_key, compare_entries, compare_names,
    fold_for_match,
    insertion_index, invert_permutation, remap_selection, sort_entries, sort_key_for_column,
    sort_permutation,
};
