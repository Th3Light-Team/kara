//! The generic conformance suite every backend runs.
//!
//! It never panics: a misbehaving backend yields [`CaseOutcome::Failed`] with a
//! detail naming the path and what was expected versus observed. It refuses a
//! missing, non-directory or non-empty scratch, and never mutates anything that
//! is not strictly under it.

use crate::backend::Backend;
use crate::error::BackendError;
use crate::path::RemotePath;

/// The normative case list, in report order.
pub const CASE_IDS: &[&str] = &[
    "list_stat_roundtrip",
    "special_names",
    "hidden_entries_listed",
    "empty_dir_lists_ok",
    "list_missing_not_found",
    "list_file_is_error",
    "list_precancelled",
    "write_read_back",
    "empty_file",
    "multi_chunk_file",
    "size_hint_is_only_a_hint",
    "read_offsets",
    "read_missing_and_dir",
    "read_drop_midway",
    "replace_false_existing",
    "replace_false_race_at_finish",
    "replace_true",
    "invisible_before_finish",
    "abort_leaves_nothing",
    "drop_leaves_nothing",
    "write_onto_directory",
    "concurrent_sessions_same_target",
    "create_dir",
    "create_dir_existing",
    "missing_parent",
    "rename_file",
    "rename_dir_subtree",
    "rename_never_overwrites",
    "rename_same_is_noop",
    "rename_into_own_subtree",
    "remove_file_and_empty_dir",
    "remove_nonempty_dir",
    "remove_missing",
    "remove_tree",
    "remove_tree_precancelled",
    "error_names_path",
    "capabilities_stable",
    "copy_within",
    "implicit_directories",
];

/// One entry per [`CASE_IDS`] id, in that order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceReport {
    pub cases: Vec<CaseResult>,
}

impl ConformanceReport {
    /// No case failed (skipped cases are allowed).
    #[must_use]
    pub fn is_success(&self) -> bool {
        todo!("ConformanceReport::is_success")
    }

    /// The failed cases, in report order.
    #[must_use]
    pub fn failures(&self) -> Vec<&CaseResult> {
        todo!("ConformanceReport::failures")
    }
}

/// The outcome of one case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseResult {
    pub id: &'static str,
    pub outcome: CaseOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseOutcome {
    Passed,
    Failed {
        detail: String,
    },
    /// `because` is the exact capability condition, e.g. `"server_side_copy=false"`.
    Skipped {
        because: &'static str,
    },
}

/// Why the suite refused to run at all.
#[derive(Debug, thiserror::Error)]
pub enum ConformanceError {
    #[error("scratch directory is unreachable")]
    ScratchUnreachable(#[source] BackendError),
    #[error("scratch is not a directory")]
    ScratchNotDirectory,
    #[error("scratch directory is not empty ({entries} entries)")]
    ScratchNotEmpty { entries: usize },
}

/// Runs every case of [`CASE_IDS`] against `backend` inside `scratch`, which
/// must be an existing, empty directory.
pub fn run(
    _backend: &dyn Backend,
    _scratch: &RemotePath,
) -> Result<ConformanceReport, ConformanceError> {
    todo!("conformance::run")
}
