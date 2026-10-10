//! The generic conformance suite every backend runs.
//!
//! It never panics: a misbehaving backend yields [`CaseOutcome::Failed`] with a
//! detail naming the path and what was expected versus observed. It refuses a
//! missing, non-directory or non-empty scratch, and never mutates anything that
//! is not strictly under it.

use std::collections::BTreeSet;
use std::io::{Read, Write};

use kara_core::{EntryKind, FileEntry};

use crate::backend::{Backend, Cancel, TRANSFER_CHUNK};
use crate::capabilities::Capabilities;
use crate::error::{BackendError, BackendErrorKind};
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

/// Cases that hold for every backend but are not part of the normative list
/// [`CASE_IDS`] (whose order and length other suites pin): run them with
/// [`run_extra`]. Same rules: own subdirectory, cleaned up, never panics.
///
/// A cancel landing in the middle of `list` needs a second thread, which this
/// crate's sources may not start (source rule cb_48); each backend's own tests
/// race it instead.
pub const EXTRA_CASE_IDS: &[&str] = &[
    "root_is_a_directory",
    "path_under_a_file_is_not_found",
    "transfer_cancel_midway",
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
        self.failures().is_empty()
    }

    /// The failed cases, in report order.
    #[must_use]
    pub fn failures(&self) -> Vec<&CaseResult> {
        self.cases
            .iter()
            .filter(|case| matches!(case.outcome, CaseOutcome::Failed { .. }))
            .collect()
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
///
/// Each case works in its own subdirectory of `scratch` and removes it at the
/// end. Nothing outside those subdirectories is ever mutated, and nothing is
/// mutated at all before `scratch` has been checked.
pub fn run(
    backend: &dyn Backend,
    scratch: &RemotePath,
) -> Result<ConformanceReport, ConformanceError> {
    check_scratch(backend, scratch)?;
    let caps = backend.capabilities();
    let mut cases: Vec<CaseResult> = CASE_IDS
        .iter()
        .map(|id| CaseResult {
            id,
            outcome: run_case(backend, scratch, caps, id),
        })
        .collect();

    // `capabilities_stable` compares against the start of the run, so a change
    // made by a case that ran after it is still caught.
    if backend.capabilities() != caps {
        for case in &mut cases {
            if case.id == "capabilities_stable" && case.outcome == CaseOutcome::Passed {
                case.outcome = CaseOutcome::Failed {
                    detail: String::from(
                        "capabilities() changed while the later cases were running",
                    ),
                };
            }
        }
    }
    Ok(ConformanceReport { cases })
}

/// Runs every case of [`EXTRA_CASE_IDS`], with the same scratch rules as [`run`].
pub fn run_extra(
    backend: &dyn Backend,
    scratch: &RemotePath,
) -> Result<ConformanceReport, ConformanceError> {
    check_scratch(backend, scratch)?;
    let caps = backend.capabilities();
    let cases = EXTRA_CASE_IDS
        .iter()
        .map(|id| CaseResult {
            id,
            outcome: run_case(backend, scratch, caps, id),
        })
        .collect();
    Ok(ConformanceReport { cases })
}

/// Refuses a missing, non-directory or non-empty scratch before any mutation.
fn check_scratch(backend: &dyn Backend, scratch: &RemotePath) -> Result<(), ConformanceError> {
    let found = backend
        .stat(scratch)
        .map_err(ConformanceError::ScratchUnreachable)?;
    if found.kind != EntryKind::Directory {
        return Err(ConformanceError::ScratchNotDirectory);
    }
    let listing = backend
        .list(scratch, &Cancel::new())
        .map_err(ConformanceError::ScratchUnreachable)?;
    let entries = listing.entries.len().saturating_add(listing.errors.len());
    if entries > 0 {
        return Err(ConformanceError::ScratchNotEmpty { entries });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Case plumbing.

/// A case either holds, or says in one sentence what it saw instead.
type Check = Result<(), String>;

type CaseFn = fn(&Ctx<'_>) -> Check;

/// What a case works with: the backend and its own fresh directory.
struct Ctx<'a> {
    backend: &'a dyn Backend,
    dir: RemotePath,
    caps: Capabilities,
}

fn case_for(id: &str) -> Option<CaseFn> {
    let case: CaseFn = match id {
        "list_stat_roundtrip" => list_stat_roundtrip,
        "special_names" => special_names,
        "hidden_entries_listed" => hidden_entries_listed,
        "empty_dir_lists_ok" => empty_dir_lists_ok,
        "list_missing_not_found" => list_missing_not_found,
        "list_file_is_error" => list_file_is_error,
        "list_precancelled" => list_precancelled,
        "write_read_back" => write_read_back,
        "empty_file" => empty_file,
        "multi_chunk_file" => multi_chunk_file,
        "size_hint_is_only_a_hint" => size_hint_is_only_a_hint,
        "read_offsets" => read_offsets,
        "read_missing_and_dir" => read_missing_and_dir,
        "read_drop_midway" => read_drop_midway,
        "replace_false_existing" => replace_false_existing,
        "replace_false_race_at_finish" => replace_false_race_at_finish,
        "replace_true" => replace_true,
        "invisible_before_finish" => invisible_before_finish,
        "abort_leaves_nothing" => abort_leaves_nothing,
        "drop_leaves_nothing" => drop_leaves_nothing,
        "write_onto_directory" => write_onto_directory,
        "concurrent_sessions_same_target" => concurrent_sessions_same_target,
        "create_dir" => create_dir,
        "create_dir_existing" => create_dir_existing,
        "missing_parent" => missing_parent,
        "rename_file" => rename_file,
        "rename_dir_subtree" => rename_dir_subtree,
        "rename_never_overwrites" => rename_never_overwrites,
        "rename_same_is_noop" => rename_same_is_noop,
        "rename_into_own_subtree" => rename_into_own_subtree,
        "remove_file_and_empty_dir" => remove_file_and_empty_dir,
        "remove_nonempty_dir" => remove_nonempty_dir,
        "remove_missing" => remove_missing,
        "remove_tree" => remove_tree,
        "remove_tree_precancelled" => remove_tree_precancelled,
        "error_names_path" => error_names_path,
        "capabilities_stable" => capabilities_stable,
        "copy_within" => copy_within,
        "implicit_directories" => implicit_directories,
        "root_is_a_directory" => root_is_a_directory,
        "path_under_a_file_is_not_found" => path_under_a_file_is_not_found,
        "transfer_cancel_midway" => transfer_cancel_midway,
        _ => return None,
    };
    Some(case)
}

fn run_case(
    backend: &dyn Backend,
    scratch: &RemotePath,
    caps: Capabilities,
    id: &str,
) -> CaseOutcome {
    if id == "copy_within" && !caps.server_side_copy {
        return CaseOutcome::Skipped {
            because: "server_side_copy=false",
        };
    }
    if id == "implicit_directories" && caps.real_directories {
        return CaseOutcome::Skipped {
            because: "real_directories=true",
        };
    }
    let Some(case) = case_for(id) else {
        return failed(format!("case {id} has no implementation"));
    };
    let dir = match scratch.join(id) {
        Ok(dir) => dir,
        Err(e) => return failed(format!("{id} is not a usable directory name: {e}")),
    };
    if let Err(e) = backend.create_dir(&dir) {
        return failed(format!("setup: create_dir({dir}) failed: {e}"));
    }
    let ctx = Ctx {
        backend,
        dir: dir.clone(),
        caps,
    };
    let result = case(&ctx);
    // Best effort and also after a failure: scratch must be left empty.
    let cleanup = backend.remove_tree(&dir, &Cancel::new());
    match (result, cleanup) {
        (Ok(()), Ok(())) => match backend.stat(&dir) {
            Err(e) if e.kind == BackendErrorKind::NotFound => CaseOutcome::Passed,
            Err(e) => failed(format!("checking that {dir} is gone failed: {e}")),
            Ok(_) => failed(format!("remove_tree({dir}) returned Ok but left {dir} behind")),
        },
        (Ok(()), Err(e)) => failed(format!("the case held but cleaning up {dir} failed: {e}")),
        (Err(detail), Ok(())) => failed(detail),
        (Err(detail), Err(e)) => failed(format!("{detail} (cleaning up {dir} also failed: {e})")),
    }
}

fn failed(detail: String) -> CaseOutcome {
    CaseOutcome::Failed { detail }
}

/// A deterministic, non-repeating byte pattern.
fn pattern(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x9E37_79B9;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect()
}

/// `Ok` only if `result` is an error of `kind` that, when `path` is given, names it.
fn expect_err<T>(
    what: &str,
    result: Result<T, BackendError>,
    kind: BackendErrorKind,
    path: Option<&RemotePath>,
) -> Check {
    let error = match result {
        Ok(_) => return Err(format!("{what}: expected {kind}, got Ok")),
        Err(error) => error,
    };
    if error.kind != kind {
        return Err(format!("{what}: expected {kind}, got {error}"));
    }
    if let Some(expected) = path
        && error.path.as_ref() != Some(expected)
    {
        return Err(format!(
            "{what}: the error should name {expected}, but names {:?}",
            error.path
        ));
    }
    Ok(())
}

impl Ctx<'_> {
    fn at(&self, name: &str) -> Result<RemotePath, String> {
        self.dir
            .join(name)
            .map_err(|e| format!("{name:?} is not a usable name: {e}"))
    }

    fn under(&self, dir: &RemotePath, name: &str) -> Result<RemotePath, String> {
        dir.join(name)
            .map_err(|e| format!("{name:?} is not a usable name: {e}"))
    }

    fn put(&self, path: &RemotePath, content: &[u8]) -> Check {
        self.put_with(path, content, false)
    }

    fn put_with(&self, path: &RemotePath, content: &[u8], replace: bool) -> Check {
        let hint = u64::try_from(content.len()).ok();
        let mut session = self
            .backend
            .begin_write(path, hint, replace)
            .map_err(|e| format!("begin_write({path}, replace={replace}) failed: {e}"))?;
        session
            .write_all(content)
            .map_err(|e| format!("writing {} bytes to {path} failed: {e}", content.len()))?;
        session
            .finish()
            .map_err(|e| format!("finish({path}) failed: {e}"))
    }

    /// Writes `content` in pieces of the given sizes, the rest in a last piece.
    fn put_in_pieces(&self, path: &RemotePath, content: &[u8], sizes: &[usize]) -> Check {
        let mut session = self
            .backend
            .begin_write(path, None, false)
            .map_err(|e| format!("begin_write({path}) failed: {e}"))?;
        let mut offset = 0usize;
        for size in sizes.iter().copied().chain(std::iter::once(usize::MAX)) {
            let end = offset.saturating_add(size).min(content.len());
            let piece = content
                .get(offset..end)
                .ok_or_else(|| format!("internal: bad piece {offset}..{end}"))?;
            session
                .write_all(piece)
                .map_err(|e| format!("writing bytes {offset}..{end} to {path} failed: {e}"))?;
            offset = end;
        }
        session
            .finish()
            .map_err(|e| format!("finish({path}) failed: {e}"))
    }

    fn get(&self, path: &RemotePath) -> Result<Vec<u8>, String> {
        let mut reader = self
            .backend
            .open_read(path, 0)
            .map_err(|e| format!("open_read({path}) failed: {e}"))?;
        let mut out = Vec::new();
        reader
            .read_to_end(&mut out)
            .map_err(|e| format!("reading {path} failed: {e}"))?;
        Ok(out)
    }

    fn mkdir(&self, path: &RemotePath) -> Check {
        self.backend
            .create_dir(path)
            .map_err(|e| format!("create_dir({path}) failed: {e}"))
    }

    fn content_is(&self, path: &RemotePath, expected: &[u8]) -> Check {
        let got = self.get(path)?;
        if got == expected {
            Ok(())
        } else {
            Err(format!(
                "{path}: expected {} bytes, read {} bytes that differ",
                expected.len(),
                got.len()
            ))
        }
    }

    fn stat(&self, path: &RemotePath) -> Result<FileEntry, String> {
        self.backend
            .stat(path)
            .map_err(|e| format!("stat({path}) failed: {e}"))
    }

    fn is_missing(&self, path: &RemotePath) -> Check {
        match self.backend.stat(path) {
            Ok(found) => Err(format!(
                "{path} should not exist, but stat found a {:?}",
                found.kind
            )),
            Err(e) if e.kind == BackendErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("stat({path}) should be NotFound, got {e}")),
        }
    }

    fn is_directory(&self, path: &RemotePath) -> Check {
        let found = self.stat(path)?;
        if found.kind == EntryKind::Directory {
            Ok(())
        } else {
            Err(format!(
                "{path} should be a directory, stat says {:?}",
                found.kind
            ))
        }
    }

    fn list(&self, dir: &RemotePath) -> Result<Vec<FileEntry>, String> {
        let listing = self
            .backend
            .list(dir, &Cancel::new())
            .map_err(|e| format!("list({dir}) failed: {e}"))?;
        if !listing.errors.is_empty() {
            return Err(format!(
                "list({dir}) reported {} per-entry errors, the first: {}",
                listing.errors.len(),
                listing
                    .errors
                    .first()
                    .map(ToString::to_string)
                    .unwrap_or_default()
            ));
        }
        Ok(listing.entries)
    }

    fn names(&self, dir: &RemotePath) -> Result<BTreeSet<String>, String> {
        Ok(self.list(dir)?.into_iter().map(|e| e.display).collect())
    }

    fn lists(&self, dir: &RemotePath, name: &str) -> Result<bool, String> {
        Ok(self.names(dir)?.contains(name))
    }
}

// ---------------------------------------------------------------------------
// The cases, in CASE_IDS order.

fn differing_field(a: &FileEntry, b: &FileEntry) -> Option<&'static str> {
    if a.name != b.name {
        Some("name")
    } else if a.display != b.display {
        Some("display")
    } else if a.kind != b.kind {
        Some("kind")
    } else if a.is_symlink != b.is_symlink {
        Some("is_symlink")
    } else if a.is_hidden != b.is_hidden {
        Some("is_hidden")
    } else if a.size != b.size {
        Some("size")
    } else if a.modified != b.modified {
        Some("modified")
    } else {
        None
    }
}

fn list_stat_roundtrip(c: &Ctx<'_>) -> Check {
    c.put(&c.at("file.txt")?, b"12345")?;
    c.mkdir(&c.at("sub")?)?;
    let entries = c.list(&c.dir)?;
    if entries.len() != 2 {
        return Err(format!(
            "{} should list exactly file.txt and sub, it lists {} entries",
            c.dir,
            entries.len()
        ));
    }
    for listed in &entries {
        let path = c.at(&listed.display)?;
        let statted = c.stat(&path)?;
        if let Some(field) = differing_field(listed, &statted) {
            return Err(format!(
                "{path}: list and stat disagree on {field}: {listed:?} against {statted:?}"
            ));
        }
        let expected = if listed.display == "sub" {
            EntryKind::Directory
        } else {
            EntryKind::File
        };
        if listed.kind != expected {
            return Err(format!(
                "{path}: expected {expected:?}, listed as {:?}",
                listed.kind
            ));
        }
    }
    Ok(())
}

fn special_names(c: &Ctx<'_>) -> Check {
    const NAMES: [&str; 6] = [
        "a b.txt",
        "\u{fc}n\u{ef} c\u{f8}d\u{e9}.txt",
        "#hash.txt",
        "100%.txt",
        "why?.txt",
        "[x].txt",
    ];
    for name in NAMES {
        let path = c.at(name)?;
        let content = name.as_bytes();
        c.put(&path, content)?;
        let entries = c.list(&c.dir)?;
        let listed = entries
            .iter()
            .find(|e| e.display == name)
            .ok_or_else(|| format!("{name:?} was written but is not listed in {}", c.dir))?;
        if listed.name != name {
            return Err(format!(
                "{name:?} is listed as {:?}, not byte-identical",
                listed.name
            ));
        }
        c.stat(&path)?;
        c.content_is(&path, content)?;
        c.backend
            .remove(&path)
            .map_err(|e| format!("remove({path}) failed: {e}"))?;
        c.is_missing(&path)?;
    }
    Ok(())
}

fn hidden_entries_listed(c: &Ctx<'_>) -> Check {
    c.put(&c.at(".dot")?, b"x")?;
    let entries = c.list(&c.dir)?;
    match entries.iter().find(|e| e.display == ".dot") {
        None => Err(format!(
            "{} does not list .dot: hiding is the UI's job",
            c.dir
        )),
        Some(e) if !e.is_hidden => Err(String::from(".dot is listed but is_hidden is false")),
        Some(_) => Ok(()),
    }
}

fn empty_dir_lists_ok(c: &Ctx<'_>) -> Check {
    let empty = c.at("empty")?;
    c.mkdir(&empty)?;
    let listing = c
        .backend
        .list(&empty, &Cancel::new())
        .map_err(|e| format!("list({empty}) of an empty directory failed: {e}"))?;
    if listing.entries.is_empty() && listing.errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{empty} should list 0 entries and 0 errors, got {} and {}",
            listing.entries.len(),
            listing.errors.len()
        ))
    }
}

fn list_missing_not_found(c: &Ctx<'_>) -> Check {
    let missing = c.at("missing")?;
    expect_err(
        "list of a missing path",
        c.backend.list(&missing, &Cancel::new()),
        BackendErrorKind::NotFound,
        Some(&missing),
    )
}

fn list_file_is_error(c: &Ctx<'_>) -> Check {
    let file = c.at("file")?;
    c.put(&file, b"x")?;
    expect_err(
        "list of a file",
        c.backend.list(&file, &Cancel::new()),
        BackendErrorKind::Other,
        Some(&file),
    )
}

fn list_precancelled(c: &Ctx<'_>) -> Check {
    for name in ["a", "b", "c"] {
        c.put(&c.at(name)?, name.as_bytes())?;
    }
    let token = Cancel::new();
    token.cancel();
    expect_err(
        "list with an already cancelled token",
        c.backend.list(&c.dir, &token),
        BackendErrorKind::Cancelled,
        Some(&c.dir),
    )
}

fn write_read_back(c: &Ctx<'_>) -> Check {
    let path = c.at("f")?;
    let content = b"hello, remote world";
    c.put(&path, content)?;
    c.content_is(&path, content)
}

fn empty_file(c: &Ctx<'_>) -> Check {
    let path = c.at("empty")?;
    let session = c
        .backend
        .begin_write(&path, Some(0), false)
        .map_err(|e| format!("begin_write({path}) failed: {e}"))?;
    session
        .finish()
        .map_err(|e| format!("finish of an empty session on {path} failed: {e}"))?;
    let found = c.stat(&path)?;
    if found.size != Some(0) {
        return Err(format!(
            "{path} should have size Some(0), has {:?}",
            found.size
        ));
    }
    c.content_is(&path, b"")
}

fn multi_chunk_file(c: &Ctx<'_>) -> Check {
    let path = c.at("big")?;
    let content = pattern(2 * TRANSFER_CHUNK + 17);
    c.put_in_pieces(&path, &content, &[1, 4095, TRANSFER_CHUNK + 3, 7])?;
    let found = c.stat(&path)?;
    if found.size != u64::try_from(content.len()).ok() {
        return Err(format!(
            "{path}: stat says {:?}, {} bytes were written",
            found.size,
            content.len()
        ));
    }
    c.content_is(&path, &content)
}

fn size_hint_is_only_a_hint(c: &Ctx<'_>) -> Check {
    for (name, hint, content) in [("short", 10u64, &b"abc"[..]), ("long", 0u64, &b"12345"[..])] {
        let path = c.at(name)?;
        let mut session = c
            .backend
            .begin_write(&path, Some(hint), false)
            .map_err(|e| format!("begin_write({path}, hint {hint}) failed: {e}"))?;
        session
            .write_all(content)
            .map_err(|e| format!("write to {path} failed: {e}"))?;
        session.finish().map_err(|e| {
            format!(
                "finish({path}) with hint {hint} and {} bytes failed: {e}",
                content.len()
            )
        })?;
        c.content_is(&path, content)?;
    }
    Ok(())
}

/// Reads to the end, treating an error at open or at read as one outcome.
fn read_from(c: &Ctx<'_>, path: &RemotePath, from: u64) -> Result<Vec<u8>, BackendError> {
    let mut reader = c.backend.open_read(path, from)?;
    let mut out = Vec::new();
    reader
        .read_to_end(&mut out)
        .map_err(|e| BackendError::from_io(e, Some(path)))?;
    Ok(out)
}

fn read_offsets(c: &Ctx<'_>) -> Check {
    let path = c.at("ten")?;
    let content = b"0123456789";
    c.put(&path, content)?;
    for (from, expected) in [
        (0u64, &content[..]),
        (4, &content[4..]),
        (10, &content[10..]),
    ] {
        let got = read_from(c, &path, from)
            .map_err(|e| format!("open_read({path}, {from}) failed: {e}"))?;
        if got != expected {
            return Err(format!(
                "open_read({path}, {from}) returned {} bytes, expected {}",
                got.len(),
                expected.len()
            ));
        }
    }
    // Past the end the error comes at open time, not as a short read, and it
    // must not overflow on the largest offset.
    for from in [11, u64::MAX] {
        expect_err(
            &format!("open_read({path}, {from}) past the end of a 10 byte file"),
            c.backend.open_read(&path, from),
            BackendErrorKind::Other,
            Some(&path),
        )?;
    }
    Ok(())
}

fn read_missing_and_dir(c: &Ctx<'_>) -> Check {
    let missing = c.at("missing")?;
    expect_err(
        "open_read of a missing path",
        c.backend.open_read(&missing, 0),
        BackendErrorKind::NotFound,
        Some(&missing),
    )?;
    let dir = c.at("dir")?;
    c.mkdir(&dir)?;
    match read_from(c, &dir, 0) {
        Err(e) if e.kind == BackendErrorKind::Other && e.path.as_ref() == Some(&dir) => Ok(()),
        Err(e) => Err(format!(
            "reading the directory {dir} should fail with Other naming it, got {e} ({:?})",
            e.path
        )),
        Ok(got) => Err(format!(
            "reading the directory {dir} succeeded with {} bytes",
            got.len()
        )),
    }
}

fn read_drop_midway(c: &Ctx<'_>) -> Check {
    let path = c.at("f")?;
    let content = pattern(10_000);
    c.put(&path, &content)?;
    {
        let mut reader = c
            .backend
            .open_read(&path, 0)
            .map_err(|e| format!("open_read({path}) failed: {e}"))?;
        let mut half = vec![0u8; content.len() / 2];
        reader
            .read_exact(&mut half)
            .map_err(|e| format!("reading half of {path} failed: {e}"))?;
    }
    let found = c.stat(&path)?;
    if found.size != u64::try_from(content.len()).ok() {
        return Err(format!(
            "{path} has size {:?} after a dropped reader",
            found.size
        ));
    }
    c.content_is(&path, &content)
}

fn replace_false_existing(c: &Ctx<'_>) -> Check {
    let path = c.at("x")?;
    c.put(&path, b"original")?;
    expect_err(
        "begin_write onto an existing file with replace = false",
        c.backend.begin_write(&path, Some(3), false),
        BackendErrorKind::AlreadyExists,
        Some(&path),
    )?;
    c.content_is(&path, b"original")
}

fn replace_false_race_at_finish(c: &Ctx<'_>) -> Check {
    let path = c.at("race")?;
    let mut first = c
        .backend
        .begin_write(&path, None, false)
        .map_err(|e| format!("first begin_write({path}) failed: {e}"))?;
    first
        .write_all(b"first")
        .map_err(|e| format!("first write failed: {e}"))?;
    let mut second = c
        .backend
        .begin_write(&path, None, false)
        .map_err(|e| format!("second begin_write({path}) failed: {e}"))?;
    second
        .write_all(b"second")
        .map_err(|e| format!("second write failed: {e}"))?;
    second
        .finish()
        .map_err(|e| format!("the second session should win, finish failed: {e}"))?;
    expect_err(
        "finish of the session that lost the race",
        first.finish(),
        BackendErrorKind::AlreadyExists,
        Some(&path),
    )?;
    c.content_is(&path, b"second")
}

fn replace_true(c: &Ctx<'_>) -> Check {
    let path = c.at("doc")?;
    c.put(&path, b"old content")?;
    let mut session = c
        .backend
        .begin_write(&path, None, true)
        .map_err(|e| format!("begin_write({path}, replace) failed: {e}"))?;
    session
        .write_all(b"NEW")
        .map_err(|e| format!("write failed: {e}"))?;
    c.content_is(&path, b"old content")
        .map_err(|e| format!("before finish the old content must show: {e}"))?;
    if c.stat(&path)?.size != Some(11) {
        return Err(format!(
            "before finish {path} must still have the old size 11"
        ));
    }
    session
        .finish()
        .map_err(|e| format!("finish({path}) failed: {e}"))?;
    c.content_is(&path, b"NEW")?;
    if c.stat(&path)?.size != Some(3) {
        return Err(format!("after finish {path} must have the new size 3"));
    }
    Ok(())
}

fn invisible_before_finish(c: &Ctx<'_>) -> Check {
    let path = c.at("new")?;
    let mut session = c
        .backend
        .begin_write(&path, None, false)
        .map_err(|e| format!("begin_write({path}) failed: {e}"))?;
    session
        .write_all(b"partial")
        .map_err(|e| format!("write failed: {e}"))?;
    c.is_missing(&path)
        .map_err(|e| format!("a half-written file is visible: {e}"))?;
    if c.lists(&c.dir, "new")? {
        return Err(format!(
            "{} lists new while its session is still open",
            c.dir
        ));
    }
    session
        .finish()
        .map_err(|e| format!("finish({path}) failed: {e}"))?;
    if !c.lists(&c.dir, "new")? {
        return Err(format!("{} does not list new after finish", c.dir));
    }
    Ok(())
}

/// Shared by the abort and drop cases: `end` closes the session one way or the other.
fn nothing_left_behind(c: &Ctx<'_>, end: fn(Box<dyn crate::WriteSession>) -> Check) -> Check {
    let keep = c.at("keep")?;
    c.put(&keep, b"keep")?;
    let target = c.at("target")?;
    c.put(&target, b"old")?;
    let before = c.names(&c.dir)?;

    let new = c.at("new")?;
    let mut session = c
        .backend
        .begin_write(&new, None, false)
        .map_err(|e| format!("begin_write({new}) failed: {e}"))?;
    session
        .write_all(&pattern(5000))
        .map_err(|e| format!("write failed: {e}"))?;
    end(session)?;
    let after = c.names(&c.dir)?;
    if after != before {
        return Err(format!(
            "the listing of {} changed: {before:?} became {after:?}",
            c.dir
        ));
    }
    c.is_missing(&new)?;

    let mut replacing = c
        .backend
        .begin_write(&target, None, true)
        .map_err(|e| format!("begin_write({target}, replace) failed: {e}"))?;
    replacing
        .write_all(b"replacement")
        .map_err(|e| format!("write failed: {e}"))?;
    end(replacing)?;
    c.content_is(&target, b"old")
}

fn abort_leaves_nothing(c: &Ctx<'_>) -> Check {
    nothing_left_behind(c, |session| {
        session.abort().map_err(|e| format!("abort failed: {e}"))
    })
}

fn drop_leaves_nothing(c: &Ctx<'_>) -> Check {
    nothing_left_behind(c, |session| {
        drop(session);
        Ok(())
    })
}

fn write_onto_directory(c: &Ctx<'_>) -> Check {
    let dir = c.at("d")?;
    c.mkdir(&dir)?;
    let child = c.under(&dir, "child")?;
    c.put(&child, b"c")?;
    for replace in [false, true] {
        expect_err(
            &format!("begin_write onto a directory, replace = {replace}"),
            c.backend.begin_write(&dir, None, replace),
            BackendErrorKind::AlreadyExists,
            Some(&dir),
        )?;
    }
    c.is_directory(&dir)?;
    c.content_is(&child, b"c")
}

fn concurrent_sessions_same_target(c: &Ctx<'_>) -> Check {
    let path = c.at("t")?;
    c.put(&path, b"original")?;
    let a_bytes = vec![b'A'; 3000];
    let b_bytes = vec![b'B'; 2000];
    let mut a = c
        .backend
        .begin_write(&path, None, true)
        .map_err(|e| format!("session a on {path} failed: {e}"))?;
    let mut b = c
        .backend
        .begin_write(&path, None, true)
        .map_err(|e| format!("session b on {path} failed: {e}"))?;
    for (piece_a, piece_b) in a_bytes.chunks(300).zip(b_bytes.chunks(200)) {
        a.write_all(piece_a)
            .map_err(|e| format!("write a failed: {e}"))?;
        b.write_all(piece_b)
            .map_err(|e| format!("write b failed: {e}"))?;
    }
    b.finish().map_err(|e| format!("finish b failed: {e}"))?;
    a.finish().map_err(|e| format!("finish a failed: {e}"))?;
    let got = c.get(&path)?;
    if got == a_bytes || got == b_bytes {
        Ok(())
    } else {
        Err(format!(
            "{path} holds {} bytes that are neither session's content",
            got.len()
        ))
    }
}

fn create_dir(c: &Ctx<'_>) -> Check {
    let dir = c.at("new")?;
    c.mkdir(&dir)?;
    c.is_directory(&dir)?;
    if !c.lists(&c.dir, "new")? {
        return Err(format!("{} does not list the new directory", c.dir));
    }
    let inside = c.list(&dir)?;
    if inside.is_empty() {
        Ok(())
    } else {
        Err(format!("a new directory lists {} entries", inside.len()))
    }
}

fn create_dir_existing(c: &Ctx<'_>) -> Check {
    let dir = c.at("dir")?;
    c.mkdir(&dir)?;
    let file = c.at("file")?;
    c.put(&file, b"f")?;
    let before = c.names(&c.dir)?;
    for path in [&dir, &file] {
        expect_err(
            "create_dir onto something that exists",
            c.backend.create_dir(path),
            BackendErrorKind::AlreadyExists,
            Some(path),
        )?;
    }
    if c.names(&c.dir)? != before {
        return Err(format!("{} changed after refused create_dir calls", c.dir));
    }
    c.content_is(&file, b"f")
}

fn missing_parent(c: &Ctx<'_>) -> Check {
    let parent = c.at("missing")?;
    let nested_dir = c.under(&parent, "x")?;
    let nested_file = c.under(&parent, "f")?;
    if c.caps.real_directories {
        expect_err(
            "create_dir under a missing parent",
            c.backend.create_dir(&nested_dir),
            BackendErrorKind::NotFound,
            Some(&nested_dir),
        )?;
        expect_err(
            "begin_write under a missing parent",
            c.backend.begin_write(&nested_file, None, false),
            BackendErrorKind::NotFound,
            Some(&nested_file),
        )?;
        return c.is_missing(&parent);
    }
    c.put(&nested_file, b"f")?;
    c.is_directory(&parent)
        .map_err(|e| format!("the prefix of {nested_file} should list as a directory: {e}"))?;
    if c.lists(&c.dir, "missing")? {
        Ok(())
    } else {
        Err(format!(
            "{} does not list the prefix of {nested_file}",
            c.dir
        ))
    }
}

fn rename_file(c: &Ctx<'_>) -> Check {
    let old = c.at("old")?;
    let new = c.at("new")?;
    c.put(&old, b"payload")?;
    c.backend
        .rename(&old, &new)
        .map_err(|e| format!("rename({old}, {new}) failed: {e}"))?;
    c.is_missing(&old)?;
    c.content_is(&new, b"payload")
}

fn rename_dir_subtree(c: &Ctx<'_>) -> Check {
    let src = c.at("src")?;
    let dst = c.at("dst")?;
    let sub = c.under(&src, "sub")?;
    c.mkdir(&src)?;
    c.mkdir(&sub)?;
    let files = [
        ("f1", b"one".to_vec()),
        ("sub/f2", b"two".to_vec()),
        ("sub/f3", pattern(777)),
    ];
    for (rel, content) in &files {
        let dir = if rel.starts_with("sub/") { &sub } else { &src };
        let name = rel.rsplit('/').next().unwrap_or(rel);
        c.put(&c.under(dir, name)?, content)?;
    }
    // A sibling whose name merely starts with "src" is not inside it.
    let sibling = c.at("src2")?;
    c.mkdir(&sibling)?;
    let sibling_file = c.under(&sibling, "keep")?;
    c.put(&sibling_file, b"keep")?;
    c.backend
        .rename(&src, &dst)
        .map_err(|e| format!("rename({src}, {dst}) failed: {e}"))?;
    c.content_is(&sibling_file, b"keep")?;
    for (rel, content) in &files {
        let mut path = dst.clone();
        for part in rel.split('/') {
            path = c.under(&path, part)?;
        }
        c.content_is(&path, content)?;
    }
    c.is_missing(&src)?;
    // Renaming to a name that merely extends the old one is not "into itself".
    let dst2 = c.at("dst2")?;
    c.backend
        .rename(&dst, &dst2)
        .map_err(|e| format!("rename({dst}, {dst2}) failed: {e}"))?;
    c.content_is(&c.under(&c.under(&dst2, "sub")?, "f2")?, b"two")?;
    c.is_missing(&dst)
}

fn rename_never_overwrites(c: &Ctx<'_>) -> Check {
    let a = c.at("a")?;
    let b = c.at("b")?;
    c.put(&a, b"A")?;
    c.put(&b, b"B")?;
    expect_err(
        "rename onto an existing file",
        c.backend.rename(&a, &b),
        BackendErrorKind::AlreadyExists,
        Some(&b),
    )?;
    c.content_is(&a, b"A")?;
    c.content_is(&b, b"B")?;

    // Nothing is replaced silently: not an empty directory, not a directory
    // onto a directory (POSIX rename(2) would do both).
    let empty = c.at("empty")?;
    c.mkdir(&empty)?;
    expect_err(
        "rename of a file onto an existing empty directory",
        c.backend.rename(&a, &empty),
        BackendErrorKind::AlreadyExists,
        Some(&empty),
    )?;
    let d1 = c.at("d1")?;
    c.mkdir(&d1)?;
    expect_err(
        "rename of a directory onto an existing directory",
        c.backend.rename(&d1, &empty),
        BackendErrorKind::AlreadyExists,
        Some(&empty),
    )?;
    c.content_is(&a, b"A")?;
    if c.caps.real_directories {
        let orphan = c.under(&c.at("no-such-dir")?, "x")?;
        expect_err(
            "rename into a missing parent",
            c.backend.rename(&a, &orphan),
            BackendErrorKind::NotFound,
            Some(&orphan),
        )?;
        c.content_is(&a, b"A")?;
    }
    Ok(())
}

fn rename_same_is_noop(c: &Ctx<'_>) -> Check {
    let x = c.at("x")?;
    c.put(&x, b"same")?;
    c.backend
        .rename(&x, &x)
        .map_err(|e| format!("rename({x}, {x}) should be a no-op: {e}"))?;
    c.content_is(&x, b"same")
}

fn rename_into_own_subtree(c: &Ctx<'_>) -> Check {
    let d = c.at("d")?;
    let sub = c.under(&d, "sub")?;
    c.mkdir(&d)?;
    c.mkdir(&sub)?;
    let f = c.under(&sub, "f")?;
    c.put(&f, b"f")?;
    let into = c.under(&sub, "d2")?;
    expect_err(
        "rename of a directory into itself",
        c.backend.rename(&d, &into),
        BackendErrorKind::Other,
        Some(&into),
    )?;
    c.content_is(&f, b"f")?;
    c.is_missing(&into)
}

fn remove_file_and_empty_dir(c: &Ctx<'_>) -> Check {
    let file = c.at("f")?;
    let dir = c.at("empty")?;
    c.put(&file, b"f")?;
    c.mkdir(&dir)?;
    for path in [&file, &dir] {
        c.backend
            .remove(path)
            .map_err(|e| format!("remove({path}) failed: {e}"))?;
        c.is_missing(path)?;
    }
    Ok(())
}

fn remove_nonempty_dir(c: &Ctx<'_>) -> Check {
    let dir = c.at("d")?;
    c.mkdir(&dir)?;
    let child = c.under(&dir, "f")?;
    c.put(&child, b"f")?;
    expect_err(
        "remove of a non-empty directory",
        c.backend.remove(&dir),
        BackendErrorKind::Other,
        Some(&dir),
    )?;
    c.content_is(&child, b"f")
}

fn remove_missing(c: &Ctx<'_>) -> Check {
    let missing = c.at("missing")?;
    expect_err(
        "remove of a missing path",
        c.backend.remove(&missing),
        BackendErrorKind::NotFound,
        Some(&missing),
    )
}

/// `<dir>` with files at three levels: returns every file path.
fn three_level_tree(c: &Ctx<'_>, root: &RemotePath) -> Result<Vec<RemotePath>, String> {
    let mid = c.under(root, "mid")?;
    let deep = c.under(&mid, "deep")?;
    c.mkdir(root)?;
    c.mkdir(&mid)?;
    c.mkdir(&deep)?;
    let mut files = Vec::new();
    for dir in [root, &mid, &deep] {
        for name in ["one", "two"] {
            let path = c.under(dir, name)?;
            c.put(&path, name.as_bytes())?;
            files.push(path);
        }
    }
    Ok(files)
}

fn remove_tree(c: &Ctx<'_>) -> Check {
    let root = c.at("tree")?;
    let outside = c.at("outside")?;
    c.put(&outside, b"stays")?;
    // Shares a string prefix with "tree" without being inside it.
    let sibling = c.under(&c.at("tree2")?, "f")?;
    c.mkdir(&c.at("tree2")?)?;
    c.put(&sibling, b"survives")?;
    three_level_tree(c, &root)?;
    c.backend
        .remove_tree(&root, &Cancel::new())
        .map_err(|e| format!("remove_tree({root}) failed: {e}"))?;
    c.is_missing(&root)?;
    c.content_is(&outside, b"stays")?;
    c.content_is(&sibling, b"survives")?;

    let single = c.at("single")?;
    c.put(&single, b"s")?;
    c.backend
        .remove_tree(&single, &Cancel::new())
        .map_err(|e| format!("remove_tree of the single file {single} failed: {e}"))?;
    c.is_missing(&single)
}

fn remove_tree_precancelled(c: &Ctx<'_>) -> Check {
    let root = c.at("tree")?;
    let files = three_level_tree(c, &root)?;
    let token = Cancel::new();
    token.cancel();
    expect_err(
        "remove_tree with a cancelled token",
        c.backend.remove_tree(&root, &token),
        BackendErrorKind::Cancelled,
        Some(&root),
    )?;
    for file in &files {
        c.stat(file)
            .map_err(|e| format!("a cancelled remove_tree removed something: {e}"))?;
    }
    Ok(())
}

fn error_names_path(c: &Ctx<'_>) -> Check {
    use BackendErrorKind::{AlreadyExists, Cancelled, NotFound, Other};

    let missing = c.at("missing")?;
    let other_missing = c.at("other-missing")?;
    let dir = c.at("dir")?;
    let file = c.at("file")?;
    let plain = c.at("plain")?;
    c.mkdir(&dir)?;
    c.put(&c.under(&dir, "child")?, b"c")?;
    c.put(&file, b"f")?;
    c.put(&plain, b"p")?;

    let token = Cancel::new();
    token.cancel();
    let live = Cancel::new();
    let b = c.backend;

    expect_err(
        "stat of a missing path",
        b.stat(&missing),
        NotFound,
        Some(&missing),
    )?;
    expect_err(
        "list of a missing path",
        b.list(&missing, &live),
        NotFound,
        Some(&missing),
    )?;
    expect_err(
        "open_read of a missing path",
        b.open_read(&missing, 0),
        NotFound,
        Some(&missing),
    )?;
    expect_err(
        "remove of a missing path",
        b.remove(&missing),
        NotFound,
        Some(&missing),
    )?;
    expect_err(
        "remove_tree of a missing path",
        b.remove_tree(&missing, &live),
        NotFound,
        Some(&missing),
    )?;
    expect_err(
        "rename of a missing source",
        b.rename(&missing, &other_missing),
        NotFound,
        Some(&missing),
    )?;
    expect_err(
        "create_dir onto a directory",
        b.create_dir(&dir),
        AlreadyExists,
        Some(&dir),
    )?;
    expect_err(
        "begin_write onto an existing file",
        b.begin_write(&file, None, false),
        AlreadyExists,
        Some(&file),
    )?;
    expect_err(
        "rename onto an existing file",
        b.rename(&plain, &file),
        AlreadyExists,
        Some(&file),
    )?;
    expect_err(
        "list with a cancelled token",
        b.list(&dir, &token),
        Cancelled,
        Some(&dir),
    )?;
    expect_err(
        "remove_tree with a cancelled token",
        b.remove_tree(&dir, &token),
        Cancelled,
        Some(&dir),
    )?;
    expect_err(
        "remove of a non-empty directory",
        b.remove(&dir),
        Other,
        Some(&dir),
    )
}

fn capabilities_stable(c: &Ctx<'_>) -> Check {
    let now = c.backend.capabilities();
    if now == c.caps {
        Ok(())
    } else {
        Err(format!(
            "capabilities() was {:?} when the run started and is {now:?} now",
            c.caps
        ))
    }
}

fn copy_within(c: &Ctx<'_>) -> Check {
    let a = c.at("a")?;
    let b = c.at("b")?;
    let content = pattern(4096);
    c.put(&a, &content)?;
    c.backend
        .copy_within(&a, &b)
        .map_err(|e| format!("copy_within({a}, {b}) failed: {e}"))?;
    c.content_is(&a, &content)?;
    c.content_is(&b, &content)?;

    expect_err(
        "copy_within onto an existing target",
        c.backend.copy_within(&a, &b),
        BackendErrorKind::AlreadyExists,
        Some(&b),
    )?;
    c.content_is(&b, &content)?;

    let dir = c.at("dir")?;
    c.mkdir(&dir)?;
    c.put(&c.under(&dir, "f")?, b"f")?;
    expect_err(
        "copy_within of a directory",
        c.backend.copy_within(&dir, &c.at("dir2")?),
        BackendErrorKind::Unsupported,
        Some(&dir),
    )
}

fn implicit_directories(c: &Ctx<'_>) -> Check {
    let prefix = c.at("prefix")?;
    let object = c.under(&prefix, "f")?;
    c.put(&object, b"f")?;
    c.is_directory(&prefix)
        .map_err(|e| format!("a prefix with an object should be a directory: {e}"))?;
    expect_err(
        "begin_write onto an implicit prefix",
        c.backend.begin_write(&prefix, None, true),
        BackendErrorKind::AlreadyExists,
        Some(&prefix),
    )?;
    c.backend
        .remove(&object)
        .map_err(|e| format!("remove({object}) failed: {e}"))?;
    c.is_missing(&prefix).map_err(|e| {
        format!("a prefix without a marker should vanish with its last object: {e}")
    })?;
    if c.lists(&c.dir, "prefix")? {
        Err(format!("{} still lists the vanished prefix", c.dir))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The extra cases, in EXTRA_CASE_IDS order.

fn root_is_a_directory(c: &Ctx<'_>) -> Check {
    let root = RemotePath::root();
    let found = c.stat(&root)?;
    if found.kind != EntryKind::Directory {
        return Err(format!("stat(/) says {:?}, not a directory", found.kind));
    }
    if found.display != "/" || found.name != "/" {
        return Err(format!(
            "stat(/) is named {:?} / {:?}, not \"/\"",
            found.name, found.display
        ));
    }
    if found.is_symlink || found.symlink_broken {
        return Err(String::from("stat(/) describes a link, not the drive"));
    }
    if found.is_hidden {
        return Err(String::from("the drive root is hidden"));
    }
    if found.size.is_some() {
        return Err(format!("stat(/) has size {:?}, a directory has None", found.size));
    }
    Ok(())
}

fn path_under_a_file_is_not_found(c: &Ctx<'_>) -> Check {
    let file = c.at("file")?;
    c.put(&file, b"f")?;
    let under = c.under(&file, "x")?;
    let deeper = c.under(&under, "y")?;
    let b = c.backend;
    for path in [&under, &deeper] {
        expect_err(
            "stat under a file",
            b.stat(path),
            BackendErrorKind::NotFound,
            Some(path),
        )?;
        expect_err(
            "open_read under a file",
            b.open_read(path, 0),
            BackendErrorKind::NotFound,
            Some(path),
        )?;
        expect_err(
            "remove under a file",
            b.remove(path),
            BackendErrorKind::NotFound,
            Some(path),
        )?;
        expect_err(
            "remove_tree under a file",
            b.remove_tree(path, &Cancel::new()),
            BackendErrorKind::NotFound,
            Some(path),
        )?;
        expect_err(
            "list under a file",
            b.list(path, &Cancel::new()),
            BackendErrorKind::NotFound,
            Some(path),
        )?;
    }
    c.content_is(&file, b"f")
}

/// What `kara-ops` does when a transfer is cancelled between two chunks: the
/// reader is dropped and the write session aborted (or dropped).
fn transfer_cancel_midway(c: &Ctx<'_>) -> Check {
    let source = c.at("source")?;
    let content = pattern(3 * TRANSFER_CHUNK + 5);
    c.put(&source, &content)?;
    let existing = c.at("existing")?;
    c.put(&existing, b"old bytes")?;
    let before = c.names(&c.dir)?;
    for (target, replace) in [(c.at("new")?, false), (existing.clone(), true)] {
        for by_drop in [false, true] {
            let mut reader = c
                .backend
                .open_read(&source, 0)
                .map_err(|e| format!("open_read({source}) failed: {e}"))?;
            let mut session = c
                .backend
                .begin_write(&target, Some(to_u64(content.len())), replace)
                .map_err(|e| format!("begin_write({target}, replace={replace}) failed: {e}"))?;
            let mut chunk = vec![0u8; TRANSFER_CHUNK];
            for _ in 0..2 {
                reader
                    .read_exact(&mut chunk)
                    .map_err(|e| format!("reading a chunk of {source} failed: {e}"))?;
                session
                    .write_all(&chunk)
                    .map_err(|e| format!("writing a chunk to {target} failed: {e}"))?;
            }
            drop(reader);
            if by_drop {
                drop(session);
            } else {
                session
                    .abort()
                    .map_err(|e| format!("abort of the transfer to {target} failed: {e}"))?;
            }
            let after = c.names(&c.dir)?;
            if after != before {
                return Err(format!(
                    "a transfer cancelled midway changed {}: {before:?} became {after:?}",
                    c.dir
                ));
            }
            if replace {
                c.content_is(&target, b"old bytes")?;
            } else {
                c.is_missing(&target)?;
            }
            c.content_is(&source, &content)?;
        }
    }
    Ok(())
}

fn to_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}
