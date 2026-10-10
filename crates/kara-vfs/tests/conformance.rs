//! The conformance suite: scratch safety, capability gating, broken backends,
//! both MemoryBackend profiles. Edge cases cb_17 (after a run), cb_43, cb_44,
//! cb_45, cb_46.

mod common;

use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

use common::{mkdir, p, snap, write};
use kara_core::FileEntry;
use kara_vfs::conformance::{
    self, CASE_IDS, CaseOutcome, CaseResult, ConformanceError, ConformanceReport,
};
use kara_vfs::memory::MemoryBackend;
use kara_vfs::{
    Backend, BackendError, BackendErrorKind as K, Cancel, Capabilities, Listing, RemotePath,
    WriteSession,
};

const EXPECTED_IDS: [&str; 39] = [
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

fn outcome<'a>(report: &'a ConformanceReport, id: &str) -> &'a CaseOutcome {
    &report
        .cases
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("case {id} missing from the report"))
        .outcome
}

fn ids(report: &ConformanceReport) -> Vec<&'static str> {
    report.cases.iter().map(|c| c.id).collect()
}

fn run_ok(backend: &dyn Backend, scratch: &RemotePath, what: &str) -> ConformanceReport {
    match conformance::run(backend, scratch) {
        Ok(report) => report,
        Err(e) => panic!("{what}: run() refused to run: {e:?}"),
    }
}

// --------------------------------------------------------------------------
// A spy that records every mutating call.

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    CreateDir(RemotePath),
    BeginWrite(RemotePath),
    Rename(RemotePath, RemotePath),
    Remove(RemotePath),
    RemoveTree(RemotePath),
    CopyWithin(RemotePath, RemotePath),
}

impl Call {
    fn paths(&self) -> Vec<&RemotePath> {
        match self {
            Call::CreateDir(a) | Call::BeginWrite(a) | Call::Remove(a) | Call::RemoveTree(a) => {
                vec![a]
            }
            Call::Rename(a, b) | Call::CopyWithin(a, b) => vec![a, b],
        }
    }
}

struct Spy {
    inner: MemoryBackend,
    calls: Mutex<Vec<Call>>,
}

impl Spy {
    fn new(inner: MemoryBackend) -> Spy {
        Spy {
            inner,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, call: Call) {
        self.calls.lock().expect("spy lock").push(call);
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("spy lock").clone()
    }

    fn reset(&self) {
        self.calls.lock().expect("spy lock").clear();
    }
}

impl Backend for Spy {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        self.inner.list(dir, cancel)
    }
    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        self.inner.stat(path)
    }
    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn Read + Send>, BackendError> {
        self.inner.open_read(path, from)
    }
    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        self.record(Call::BeginWrite(path.clone()));
        self.inner.begin_write(path, size_hint, replace)
    }
    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.record(Call::CreateDir(path.clone()));
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.record(Call::Rename(from.clone(), to.clone()));
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.record(Call::Remove(path.clone()));
        self.inner.remove(path)
    }
    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        self.record(Call::RemoveTree(path.clone()));
        self.inner.remove_tree(path, cancel)
    }
    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.record(Call::CopyWithin(from.clone(), to.clone()));
        self.inner.copy_within(from, to)
    }
}

// --------------------------------------------------------------------------
// Deliberately broken backends (cb_45).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Breakage {
    /// (a) rename removes `to` first, then renames.
    RenameOverwrites,
    /// (b) begin_write ignores replace = false.
    IgnoresReplaceFalse,
    /// (c) abort and drop commit the session.
    AbortCommits,
    /// (d) list ignores the Cancel token.
    ListIgnoresCancel,
    /// (e) every write is committed at once under the final name.
    CommitsOnWrite,
    /// (f) remove deletes non-empty directories.
    RemoveIsRecursive,
    /// (g) every error loses its path.
    ErrorsWithoutPath,
    /// Everything but the scratch checks fails: the suite must still report.
    MutationsUnavailable,
}

struct Broken {
    inner: Arc<MemoryBackend>,
    breakage: Breakage,
}

impl Broken {
    fn new(inner: MemoryBackend, breakage: Breakage) -> Broken {
        Broken {
            inner: Arc::new(inner),
            breakage,
        }
    }

    fn strip<T>(&self, r: Result<T, BackendError>) -> Result<T, BackendError> {
        if self.breakage == Breakage::ErrorsWithoutPath {
            r.map_err(|e| BackendError::new(e.kind, None))
        } else {
            r
        }
    }

    fn mutation(&self, path: &RemotePath) -> Result<(), BackendError> {
        if self.breakage == Breakage::MutationsUnavailable {
            Err(BackendError::new(K::Unavailable, Some(path.clone())))
        } else {
            Ok(())
        }
    }
}

/// (c) A session that commits when it should discard.
struct CommitOnAbort {
    inner: Option<Box<dyn WriteSession>>,
}

impl Write for CommitOnAbort {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.inner.as_mut() {
            Some(s) => s.write(buf),
            None => Err(io::Error::other("closed")),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self.inner.as_mut() {
            Some(s) => s.flush(),
            None => Ok(()),
        }
    }
}

impl WriteSession for CommitOnAbort {
    fn finish(mut self: Box<Self>) -> Result<(), BackendError> {
        match self.inner.take() {
            Some(s) => s.finish(),
            None => Ok(()),
        }
    }
    fn abort(mut self: Box<Self>) -> Result<(), BackendError> {
        match self.inner.take() {
            Some(s) => s.finish(),
            None => Ok(()),
        }
    }
}

impl Drop for CommitOnAbort {
    fn drop(&mut self) {
        if let Some(s) = self.inner.take() {
            let _ = s.finish();
        }
    }
}

/// (e) A session that makes every write visible under the final name.
struct CommitOnWrite {
    backend: Arc<MemoryBackend>,
    path: RemotePath,
    buf: Vec<u8>,
}

impl CommitOnWrite {
    fn commit(&self) -> Result<(), BackendError> {
        let mut s = self.backend.begin_write(&self.path, None, true)?;
        s.write_all(&self.buf)
            .map_err(|e| BackendError::from_io(e, Some(&self.path)))?;
        s.finish()
    }
}

impl Write for CommitOnWrite {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        self.commit().map_err(io::Error::from)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl WriteSession for CommitOnWrite {
    fn finish(self: Box<Self>) -> Result<(), BackendError> {
        self.commit()
    }
    fn abort(self: Box<Self>) -> Result<(), BackendError> {
        Ok(())
    }
}

impl Backend for Broken {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        if self.breakage == Breakage::ListIgnoresCancel {
            return self.inner.list(dir, &Cancel::new());
        }
        self.strip(self.inner.list(dir, cancel))
    }
    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        self.strip(self.inner.stat(path))
    }
    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn Read + Send>, BackendError> {
        self.strip(self.inner.open_read(path, from))
    }
    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        self.mutation(path)?;
        match self.breakage {
            Breakage::IgnoresReplaceFalse => self.inner.begin_write(path, size_hint, true),
            Breakage::AbortCommits => {
                let s = self.inner.begin_write(path, size_hint, replace)?;
                Ok(Box::new(CommitOnAbort { inner: Some(s) }))
            }
            Breakage::CommitsOnWrite => {
                // Validate like the real thing, then discard the real session.
                let s = self.inner.begin_write(path, size_hint, replace)?;
                let _ = s.abort();
                Ok(Box::new(CommitOnWrite {
                    backend: Arc::clone(&self.inner),
                    path: path.clone(),
                    buf: Vec::new(),
                }))
            }
            _ => self.strip(self.inner.begin_write(path, size_hint, replace)),
        }
    }
    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.mutation(path)?;
        self.strip(self.inner.create_dir(path))
    }
    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.mutation(from)?;
        if self.breakage == Breakage::RenameOverwrites && from != to {
            let _ = self.inner.remove_tree(to, &Cancel::new());
        }
        self.strip(self.inner.rename(from, to))
    }
    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.mutation(path)?;
        if self.breakage == Breakage::RemoveIsRecursive {
            return self.inner.remove_tree(path, &Cancel::new());
        }
        self.strip(self.inner.remove(path))
    }
    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        self.mutation(path)?;
        self.strip(self.inner.remove_tree(path, cancel))
    }
    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.mutation(from)?;
        self.strip(self.inner.copy_within(from, to))
    }
}

fn run_broken(base: fn() -> MemoryBackend, breakage: Breakage) -> ConformanceReport {
    let inner = base();
    mkdir(&inner, "/scratch");
    let broken = Broken::new(inner, breakage);
    let report = run_ok(&broken, &p("/scratch"), &format!("{breakage:?}"));
    assert_eq!(
        ids(&report),
        EXPECTED_IDS,
        "{breakage:?}: one result per case, in order"
    );
    assert!(
        !report.is_success(),
        "{breakage:?}: a broken backend must not pass"
    );
    report
}

fn assert_failed(report: &ConformanceReport, id: &str, what: &str) {
    match outcome(report, id) {
        CaseOutcome::Failed { detail } => {
            assert!(
                !detail.trim().is_empty(),
                "{what}: {id} failed without a detail"
            );
        }
        other => panic!("{what}: case {id} should be Failed, got {other:?}"),
    }
    assert!(
        report.failures().iter().any(|c| c.id == id),
        "{what}: failures() must list {id}"
    );
}

// cb_43 ---------------------------------------------------------------------

#[test]
fn cb_43_missing_scratch_is_unreachable_and_untouched() {
    let spy = Spy::new(MemoryBackend::posix_like());
    write(&spy.inner, "/keep", b"keep");
    let before = snap(&spy.inner);
    spy.reset();
    match conformance::run(&spy, &p("/scratch")) {
        Err(ConformanceError::ScratchUnreachable(e)) => assert_eq!(e.kind, K::NotFound),
        other => panic!("expected ScratchUnreachable(NotFound), got {other:?}"),
    }
    assert_eq!(spy.calls(), vec![], "no mutating call on a refused scratch");
    assert_eq!(snap(&spy.inner), before);
}

#[test]
fn cb_43_scratch_that_is_a_file_is_refused() {
    let spy = Spy::new(MemoryBackend::posix_like());
    write(&spy.inner, "/scratch", b"precious");
    let before = snap(&spy.inner);
    spy.reset();
    assert!(matches!(
        conformance::run(&spy, &p("/scratch")),
        Err(ConformanceError::ScratchNotDirectory)
    ));
    assert_eq!(spy.calls(), vec![]);
    assert_eq!(snap(&spy.inner), before);
}

#[test]
fn cb_43_non_empty_scratch_is_refused_and_survives() {
    for base in [
        MemoryBackend::posix_like as fn() -> MemoryBackend,
        MemoryBackend::object_store_like,
    ] {
        let spy = Spy::new(base());
        mkdir(&spy.inner, "/scratch");
        write(&spy.inner, "/scratch/user-file.txt", b"do not delete");
        mkdir(&spy.inner, "/scratch/user-dir");
        let before = snap(&spy.inner);
        spy.reset();
        match conformance::run(&spy, &p("/scratch")) {
            Err(ConformanceError::ScratchNotEmpty { entries }) => assert_eq!(entries, 2),
            other => panic!("expected ScratchNotEmpty {{ entries: 2 }}, got {other:?}"),
        }
        assert_eq!(spy.calls(), vec![], "zero mutating calls");
        assert_eq!(snap(&spy.inner), before);
    }
}

#[test]
fn cb_43_a_hidden_entry_also_makes_scratch_dirty() {
    let spy = Spy::new(MemoryBackend::posix_like());
    mkdir(&spy.inner, "/scratch");
    write(&spy.inner, "/scratch/.dotfile", b"x");
    spy.reset();
    assert!(matches!(
        conformance::run(&spy, &p("/scratch")),
        Err(ConformanceError::ScratchNotEmpty { entries: 1 })
    ));
    assert_eq!(spy.calls(), vec![]);
}

#[test]
fn cb_43_a_run_mutates_only_strictly_inside_scratch_and_cleans_up() {
    for base in [
        MemoryBackend::posix_like as fn() -> MemoryBackend,
        MemoryBackend::object_store_like,
    ] {
        let spy = Spy::new(base());
        let label = format!("{:?}", spy.inner.capabilities());
        if spy.inner.capabilities().real_directories {
            mkdir(&spy.inner, "/work");
            mkdir(&spy.inner, "/work/scratch2");
        }
        write(&spy.inner, "/outside.txt", b"outside");
        write(
            &spy.inner,
            "/work/scratch2/keep",
            b"sibling sharing a prefix",
        );
        write(&spy.inner, "/work/scratch.txt", b"another sibling");
        mkdir(&spy.inner, "/work/scratch");
        let before = snap(&spy.inner);
        spy.reset();

        let scratch = p("/work/scratch");
        let report = run_ok(&spy, &scratch, &label);
        assert!(report.is_success(), "{label}: {:?}", report.failures());

        let calls = spy.calls();
        assert!(
            !calls.is_empty(),
            "{label}: the suite did exercise the backend"
        );
        for call in &calls {
            for path in call.paths() {
                assert!(
                    path.starts_with(&scratch) && path != &scratch,
                    "{label}: {call:?} touches {path}, outside or equal to scratch"
                );
            }
        }
        let left = spy
            .inner
            .list(&scratch, &Cancel::new())
            .expect("scratch still lists");
        assert!(
            left.entries.is_empty(),
            "{label}: scratch is empty again: {:?}",
            left.entries
        );
        assert_eq!(
            snap(&spy.inner),
            before,
            "{label}: nothing outside scratch changed"
        );
    }
}

// cb_44 ---------------------------------------------------------------------

#[test]
fn cb_44_posix_profile_skips_exactly_the_undeclared_capabilities() {
    let spy = Spy::new(MemoryBackend::posix_like());
    mkdir(&spy.inner, "/scratch");
    let report = run_ok(&spy, &p("/scratch"), "posix");
    assert_eq!(
        outcome(&report, "copy_within"),
        &CaseOutcome::Skipped {
            because: "server_side_copy=false"
        }
    );
    assert_eq!(
        outcome(&report, "implicit_directories"),
        &CaseOutcome::Skipped {
            because: "real_directories=true"
        }
    );
    let copies = spy
        .calls()
        .iter()
        .filter(|c| matches!(c, Call::CopyWithin(..)))
        .count();
    assert_eq!(
        copies, 0,
        "copy_within is never called without server_side_copy"
    );
    let skipped = report
        .cases
        .iter()
        .filter(|c| matches!(c.outcome, CaseOutcome::Skipped { .. }))
        .count();
    assert_eq!(
        skipped, 2,
        "only the two capability-gated cases may be skipped"
    );
}

#[test]
fn cb_44_object_profile_skips_nothing_and_exercises_copy_within() {
    let spy = Spy::new(MemoryBackend::object_store_like());
    mkdir(&spy.inner, "/scratch");
    let report = run_ok(&spy, &p("/scratch"), "object");
    for case in &report.cases {
        assert!(
            !matches!(case.outcome, CaseOutcome::Skipped { .. }),
            "object profile: {} skipped",
            case.id
        );
    }
    assert_eq!(outcome(&report, "copy_within"), &CaseOutcome::Passed);
    assert_eq!(
        outcome(&report, "implicit_directories"),
        &CaseOutcome::Passed
    );
    assert!(
        spy.calls()
            .iter()
            .any(|c| matches!(c, Call::CopyWithin(..)))
    );
}

// cb_45 ---------------------------------------------------------------------

#[test]
fn cb_45_a_rename_that_overwrites_is_caught() {
    for base in [
        MemoryBackend::posix_like as fn() -> MemoryBackend,
        MemoryBackend::object_store_like,
    ] {
        let r = run_broken(base, Breakage::RenameOverwrites);
        assert_failed(&r, "rename_never_overwrites", "RenameOverwrites");
    }
}

#[test]
fn cb_45_ignoring_replace_false_is_caught() {
    for base in [
        MemoryBackend::posix_like as fn() -> MemoryBackend,
        MemoryBackend::object_store_like,
    ] {
        let r = run_broken(base, Breakage::IgnoresReplaceFalse);
        assert_failed(&r, "replace_false_existing", "IgnoresReplaceFalse");
    }
}

#[test]
fn cb_45_abort_or_drop_that_commits_is_caught() {
    for base in [
        MemoryBackend::posix_like as fn() -> MemoryBackend,
        MemoryBackend::object_store_like,
    ] {
        let r = run_broken(base, Breakage::AbortCommits);
        assert_failed(&r, "abort_leaves_nothing", "AbortCommits");
        assert_failed(&r, "drop_leaves_nothing", "AbortCommits");
    }
}

#[test]
fn cb_45_list_ignoring_cancel_is_caught() {
    for base in [
        MemoryBackend::posix_like as fn() -> MemoryBackend,
        MemoryBackend::object_store_like,
    ] {
        let r = run_broken(base, Breakage::ListIgnoresCancel);
        assert_failed(&r, "list_precancelled", "ListIgnoresCancel");
    }
}

#[test]
fn cb_45_half_written_file_visible_is_caught() {
    for base in [
        MemoryBackend::posix_like as fn() -> MemoryBackend,
        MemoryBackend::object_store_like,
    ] {
        let r = run_broken(base, Breakage::CommitsOnWrite);
        assert_failed(&r, "invisible_before_finish", "CommitsOnWrite");
    }
}

#[test]
fn cb_45_recursive_remove_is_caught() {
    for base in [
        MemoryBackend::posix_like as fn() -> MemoryBackend,
        MemoryBackend::object_store_like,
    ] {
        let r = run_broken(base, Breakage::RemoveIsRecursive);
        assert_failed(&r, "remove_nonempty_dir", "RemoveIsRecursive");
    }
}

#[test]
fn cb_45_errors_without_path_are_caught() {
    for base in [
        MemoryBackend::posix_like as fn() -> MemoryBackend,
        MemoryBackend::object_store_like,
    ] {
        let r = run_broken(base, Breakage::ErrorsWithoutPath);
        assert_failed(&r, "error_names_path", "ErrorsWithoutPath");
    }
}

#[test]
fn cb_45_a_backend_that_fails_everything_still_gets_a_report() {
    let r = run_broken(MemoryBackend::posix_like, Breakage::MutationsUnavailable);
    for id in [
        "write_read_back",
        "create_dir",
        "rename_file",
        "remove_tree",
    ] {
        assert_failed(&r, id, "MutationsUnavailable");
    }
}

// cb_46 ---------------------------------------------------------------------

#[test]
fn cb_46_case_ids_are_the_normative_list() {
    assert_eq!(CASE_IDS, EXPECTED_IDS);
}

#[test]
fn cb_46_both_profiles_pass_the_whole_suite() {
    for (label, m) in common::profiles() {
        mkdir(&m, "/scratch");
        let report = run_ok(&m, &p("/scratch"), label);
        assert_eq!(ids(&report), EXPECTED_IDS, "{label}");
        assert!(report.is_success(), "{label}: {:#?}", report.failures());
        assert!(report.failures().is_empty(), "{label}");
    }
}

#[test]
fn cb_46_suite_passes_in_a_nested_scratch_and_twice_in_a_row() {
    for (label, m) in common::profiles() {
        if m.capabilities().real_directories {
            mkdir(&m, "/a");
            mkdir(&m, "/a/b");
        }
        mkdir(&m, "/a/b/scratch dir");
        let scratch = p("/a/b/scratch dir");
        let first = run_ok(&m, &scratch, label);
        assert!(first.is_success(), "{label}: {:#?}", first.failures());
        let second = run_ok(&m, &scratch, label);
        assert!(
            second.is_success(),
            "{label}: the suite leaves scratch reusable"
        );
    }
}

#[test]
fn cb_46_report_helpers() {
    let passed = CaseResult {
        id: "a",
        outcome: CaseOutcome::Passed,
    };
    let skipped = CaseResult {
        id: "b",
        outcome: CaseOutcome::Skipped { because: "x=false" },
    };
    let failed = CaseResult {
        id: "c",
        outcome: CaseOutcome::Failed {
            detail: "boom".into(),
        },
    };
    let failed2 = CaseResult {
        id: "d",
        outcome: CaseOutcome::Failed {
            detail: "bang".into(),
        },
    };

    let ok = ConformanceReport {
        cases: vec![passed.clone(), skipped.clone()],
    };
    assert!(ok.is_success(), "skipped is allowed");
    assert!(ok.failures().is_empty());

    let bad = ConformanceReport {
        cases: vec![failed.clone(), passed.clone(), skipped, failed2.clone()],
    };
    assert!(!bad.is_success());
    assert_eq!(
        bad.failures(),
        vec![&failed, &failed2],
        "failures in report order"
    );

    assert!(ConformanceReport { cases: vec![] }.is_success());
}

// cb_17 ---------------------------------------------------------------------

#[test]
fn cb_17_capabilities_unchanged_by_a_full_run() {
    for (label, m) in common::profiles() {
        let before = m.capabilities();
        mkdir(&m, "/scratch");
        let report = run_ok(&m, &p("/scratch"), label);
        assert_eq!(
            outcome(&report, "capabilities_stable"),
            &CaseOutcome::Passed,
            "{label}"
        );
        assert_eq!(m.capabilities(), before, "{label}");
    }
}
