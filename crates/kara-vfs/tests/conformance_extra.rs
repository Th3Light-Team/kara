//! `conformance::run_extra`: the cases outside the pinned normative list
//! (drive root, paths under a file, a transfer cancelled between chunks). Both
//! MemoryBackend profiles pass; each case catches the backend that breaks
//! exactly its rule. A cancel landing mid-list needs a second thread, which
//! kara-vfs sources may not start, so that race lives here as a test.

mod common;

use std::io::{self, Read, Write};
use std::time::Duration;

use common::{mkdir, p, profiles, write};
use kara_core::{EntryKind, FileEntry};
use kara_vfs::conformance::{self, CaseOutcome, ConformanceError, EXTRA_CASE_IDS};
use kara_vfs::memory::MemoryBackend;
use kara_vfs::{
    Backend, BackendError, BackendErrorKind, Cancel, Capabilities, Listing, RemotePath,
    WriteSession,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Twist {
    /// `stat("/")` describes a symlink.
    RootIsALink,
    /// A path under a file reports the file, not the path asked for.
    UnderFileNamesTheParent,
    /// A cancel that lands mid-list returns the entries read so far as `Ok`.
    TruncatesOnCancel,
    /// Abort and drop commit whatever was written.
    AbortCommits,
}

struct Twisted {
    inner: MemoryBackend,
    twist: Twist,
}

/// A session whose abort and drop commit (a multipart upload completed on cancel).
struct CommitAnyway(Option<Box<dyn WriteSession>>);

impl Write for CommitAnyway {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.as_mut() {
            Some(s) => s.write(buf),
            None => Err(io::Error::other("closed")),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl WriteSession for CommitAnyway {
    fn finish(mut self: Box<Self>) -> Result<(), BackendError> {
        self.0.take().map_or(Ok(()), |s| s.finish())
    }
    fn abort(mut self: Box<Self>) -> Result<(), BackendError> {
        self.0.take().map_or(Ok(()), |s| s.finish())
    }
}

impl Drop for CommitAnyway {
    fn drop(&mut self) {
        if let Some(s) = self.0.take() {
            let _ = s.finish();
        }
    }
}

impl Twisted {
    fn rename_parent<T>(&self, r: Result<T, BackendError>) -> Result<T, BackendError> {
        if self.twist != Twist::UnderFileNamesTheParent {
            return r;
        }
        r.map_err(|mut e| {
            if let Some(path) = e.path.as_ref()
                && path.segments().any(|s| s == "x")
            {
                e.path = path.parent();
            }
            e
        })
    }
}

impl Backend for Twisted {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        if self.twist == Twist::TruncatesOnCancel {
            let mut listing = self.inner.list(dir, &Cancel::new())?;
            std::thread::sleep(Duration::from_millis(3));
            if cancel.is_cancelled() {
                listing.entries.truncate(listing.entries.len() / 2);
            }
            return Ok(listing);
        }
        self.rename_parent(self.inner.list(dir, cancel))
    }
    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        let mut entry = self.rename_parent(self.inner.stat(path))?;
        if self.twist == Twist::RootIsALink && path.is_root() {
            entry.is_symlink = true;
        }
        Ok(entry)
    }
    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn Read + Send>, BackendError> {
        self.rename_parent(self.inner.open_read(path, from))
    }
    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        let session = self.inner.begin_write(path, size_hint, replace)?;
        if self.twist == Twist::AbortCommits {
            return Ok(Box::new(CommitAnyway(Some(session))));
        }
        Ok(session)
    }
    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.rename_parent(self.inner.remove(path))
    }
    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        self.rename_parent(self.inner.remove_tree(path, cancel))
    }
    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.inner.copy_within(from, to)
    }
}

fn outcome_of(backend: &dyn Backend, id: &str) -> CaseOutcome {
    let report = match conformance::run_extra(backend, &p("/scratch")) {
        Ok(report) => report,
        Err(e) => panic!("run_extra refused to run: {e:?}"),
    };
    let ids: Vec<&str> = report.cases.iter().map(|c| c.id).collect();
    assert_eq!(ids, EXTRA_CASE_IDS.to_vec(), "one result per case, in order");
    match report.cases.iter().find(|c| c.id == id) {
        Some(case) => case.outcome.clone(),
        None => panic!("{id} missing"),
    }
}

fn twisted(base: fn() -> MemoryBackend, twist: Twist) -> Twisted {
    let inner = base();
    mkdir(&inner, "/scratch");
    Twisted { inner, twist }
}

const BASES: [fn() -> MemoryBackend; 2] =
    [MemoryBackend::posix_like, MemoryBackend::object_store_like];

#[test]
fn both_memory_profiles_pass_every_extra_case() {
    for (label, m) in profiles() {
        mkdir(&m, "/scratch");
        let report = conformance::run_extra(&m, &p("/scratch"))
            .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        assert!(report.is_success(), "{label}: {:#?}", report.failures());
        assert!(
            common::names(&m, "/scratch").is_empty(),
            "{label}: scratch is left empty"
        );
    }
}

#[test]
fn run_extra_keeps_the_scratch_rules() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/scratch");
    write(&m, "/scratch/dirty", b"x");
    match conformance::run_extra(&m, &p("/scratch")) {
        Err(ConformanceError::ScratchNotEmpty { entries: 1 }) => {}
        other => panic!("a dirty scratch must be refused, got {other:?}"),
    }
    assert!(matches!(
        conformance::run_extra(&m, &p("/missing")),
        Err(ConformanceError::ScratchUnreachable(_))
    ));
}

fn assert_case_fails(twist: Twist, id: &str) {
    for base in BASES {
        let backend = twisted(base, twist);
        match outcome_of(&backend, id) {
            CaseOutcome::Failed { detail } => assert!(!detail.is_empty()),
            other => panic!("{twist:?}: {id} should fail, got {other:?}"),
        }
    }
}

#[test]
fn a_root_described_as_a_link_is_caught() {
    assert_case_fails(Twist::RootIsALink, "root_is_a_directory");
}

#[test]
fn errors_naming_the_parent_instead_of_the_path_are_caught() {
    assert_case_fails(Twist::UnderFileNamesTheParent, "path_under_a_file_is_not_found");
}

/// A cancel that lands while `list` runs: the answer is the complete listing or
/// `Cancelled` naming the directory, never a truncated `Ok`.
fn list_cancel_race(b: &dyn Backend, dir: &RemotePath, expected: usize) -> Result<(), String> {
    for spin in [0_u32, 10, 100, 1_000, 10_000, 100_000] {
        let token = Cancel::new();
        let canceller = {
            let token = token.clone();
            std::thread::spawn(move || {
                for _ in 0..spin {
                    std::hint::spin_loop();
                }
                token.cancel();
            })
        };
        let result = b.list(dir, &token);
        let _ = canceller.join();
        match result {
            Ok(listing) if listing.entries.len() == expected && listing.errors.is_empty() => {}
            Ok(listing) => {
                return Err(format!(
                    "spin {spin}: Ok with {} of {expected} entries",
                    listing.entries.len()
                ));
            }
            Err(e) if e.kind == BackendErrorKind::Cancelled && e.path.as_ref() == Some(dir) => {}
            Err(e) => return Err(format!("spin {spin}: {e:?}")),
        }
    }
    Ok(())
}

fn many_files(b: &dyn Backend, dir: &str, count: usize) {
    if b.capabilities().real_directories {
        mkdir(b, dir);
    }
    for n in 0..count {
        write(b, &format!("{dir}/f{n:03}"), b"");
    }
}

#[test]
fn a_cancel_racing_a_memory_list_never_truncates() {
    for (label, m) in profiles() {
        many_files(&m, "/many", 300);
        list_cancel_race(&m, &p("/many"), 300).unwrap_or_else(|e| panic!("{label}: {e}"));
    }
}

#[test]
fn the_race_check_catches_a_truncated_ok_after_a_cancel() {
    for base in BASES {
        let backend = Twisted {
            inner: base(),
            twist: Twist::TruncatesOnCancel,
        };
        many_files(&backend.inner, "/many", 300);
        assert!(
            list_cancel_race(&backend, &p("/many"), 300).is_err(),
            "a truncated Ok must be caught"
        );
    }
}

#[test]
fn a_transfer_that_commits_on_cancel_is_caught() {
    assert_case_fails(Twist::AbortCommits, "transfer_cancel_midway");
}

#[test]
fn the_drive_root_of_memory_is_a_plain_directory() {
    for (label, m) in profiles() {
        let root = m.stat(&RemotePath::root()).unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(root.kind, EntryKind::Directory, "{label}");
        assert_eq!(root.display, "/", "{label}");
        assert_eq!(
            m.stat(&p("/a/b")).map_err(|e| e.kind).err(),
            Some(BackendErrorKind::NotFound),
            "{label}"
        );
    }
}
