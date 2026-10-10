//! Tests added after the mutation pass (see `docs/remote-backends-testing.md`,
//! «kara-ops over Locations»): each one fails under a mutation that survived.

mod remote_common;

use std::sync::Arc;

use kara_core::FileEntry;
use kara_ops::runner::Op;
use kara_ops::{Action, ErrorDecision, UndoError, UndoStack};
use kara_vfs::memory::{Fault, FaultEffect, MemoryBackend, Op as MemOp};
use kara_vfs::{
    Backend, BackendError, BackendErrorKind, Cancel, Capabilities, Listing, RemotePath,
    WriteSession,
};
use remote_common::*;

/// Forwards everything to a memory drive, with two deliberate defects:
/// `list` hides `hidden` and reports an error for it instead (what a backend
/// does with an entry it cannot read), and, when `clobber` is set, `rename`
/// overwrites its target (a backend that breaks the contract).
struct Defective {
    inner: Arc<MemoryBackend>,
    hidden: Option<RemotePath>,
    clobber: bool,
}

impl Backend for Defective {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        let mut listing = self.inner.list(dir, cancel)?;
        if let Some(hidden) = &self.hidden
            && hidden.parent().as_ref() == Some(dir)
        {
            let name = hidden.file_name().unwrap_or_default().to_string();
            listing.entries.retain(|entry| entry.name.to_str() != Some(name.as_str()));
            listing
                .errors
                .push(BackendError::new(BackendErrorKind::PermissionDenied, Some(hidden.clone())));
        }
        Ok(listing)
    }
    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        self.inner.stat(path)
    }
    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn std::io::Read + Send>, BackendError> {
        self.inner.open_read(path, from)
    }
    fn begin_write(
        &self,
        path: &RemotePath,
        size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        self.inner.begin_write(path, size_hint, replace)
    }
    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.inner.create_dir(path)
    }
    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        if self.clobber && self.inner.stat(to).is_ok() {
            self.inner.remove_tree(to, &Cancel::new())?;
        }
        self.inner.rename(from, to)
    }
    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        self.inner.remove(path)
    }
    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        self.inner.remove_tree(path, cancel)
    }
    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        self.inner.copy_within(from, to)
    }
}

/// Survivor M23: a listing with per-entry errors was treated as complete, so
/// a move removed every source it did copy, and nobody was told that one
/// entry was never read.
#[test]
fn a_folder_listed_with_errors_is_reported_and_its_move_keeps_every_source() {
    let a = Arc::new(MemoryBackend::posix_like());
    let b = Arc::new(MemoryBackend::posix_like());
    put(a.as_ref(), "/d/readable", b"r");
    put(a.as_ref(), "/d/unreadable", b"u");
    let source = Defective {
        inner: a.clone(),
        hidden: Some(rpath("/d/unreadable")),
        clobber: false,
    };
    let resolver = resolver(vec![(drive("a"), Arc::new(source)), (drive("b"), b.clone())]);

    let (outcome, _) = run(
        request(Op::Move, vec![rloc(&drive("a"), "/d")], rloc(&drive("b"), "/")),
        &resolver,
        &Script::errors(ErrorDecision::Cancel),
    );

    assert_eq!(outcome.report.failures.len(), 1, "{:?}", outcome.report);
    assert_eq!(outcome.report.failures[0].path, uri(&drive("a"), "/d"));
    assert_eq!(content(&a, "/d/readable"), Some(b"r".to_vec()), "a partial move keeps it");
    assert_eq!(content(&a, "/d/unreadable"), Some(b"u".to_vec()));
    assert_eq!(content(&b, "/d/readable"), Some(b"r".to_vec()));
}

/// Survivor M25: undo relied on the backend's `rename` never overwriting. A
/// backend that breaks that contract must still not lose what took the old
/// place.
#[test]
fn undo_checks_the_old_place_itself_instead_of_trusting_the_backend() {
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/b", b"moved");
    put(memory.as_ref(), "/a", b"newcomer");
    let id = drive("nas");
    let clobbering = Defective {
        inner: memory.clone(),
        hidden: None,
        clobber: true,
    };
    let resolver = resolver(vec![(id.clone(), Arc::new(clobbering) as Arc<dyn Backend>)]);
    let mut stack = UndoStack::new();
    stack.push(Action::RemoteRenamed {
        from: rloc(&id, "/a"),
        to: rloc(&id, "/b"),
    });

    assert!(matches!(stack.undo_with(&resolver), Err(UndoError::Remote { .. })));
    assert_eq!(content(&memory, "/a"), Some(b"newcomer".to_vec()));
    assert_eq!(content(&memory, "/b"), Some(b"moved".to_vec()));
}

/// M24 (equivalent, see the table): a copied source that cannot be removed at
/// the end of a folder move is reported by its URI and stays, with its folder.
#[test]
fn a_source_that_cannot_be_removed_after_a_folder_move_is_reported_and_kept() {
    let a = Arc::new(MemoryBackend::posix_like());
    let b = Arc::new(MemoryBackend::posix_like());
    put(a.as_ref(), "/d/1", b"1");
    put(a.as_ref(), "/d/2", b"2");
    a.inject(Fault {
        op: MemOp::Remove,
        path: Some(rpath("/d/1")),
        after: 0,
        effect: FaultEffect::Fail(BackendErrorKind::PermissionDenied),
        times: None,
    })
    .unwrap();
    let resolver = resolver(vec![(drive("a"), a.clone()), (drive("b"), b.clone())]);

    let (outcome, transcript) = run(
        request(Op::Move, vec![rloc(&drive("a"), "/d")], rloc(&drive("b"), "/")),
        &resolver,
        &Script::errors(ErrorDecision::Skip),
    );

    assert_eq!(transcript.failures.len(), 1);
    assert_eq!(transcript.failures[0].path, uri(&drive("a"), "/d/1"));
    assert_eq!(outcome.report.skipped, vec![uri(&drive("a"), "/d/1")]);
    assert_eq!(content(&a, "/d/1"), Some(b"1".to_vec()));
    assert!(!exists(a.as_ref(), "/d/2"));
    assert!(exists(a.as_ref(), "/d"), "its folder stays while it is not empty");
    assert_eq!(content(&b, "/d/1"), Some(b"1".to_vec()));
    assert_eq!(content(&b, "/d/2"), Some(b"2".to_vec()));
}

/// Survivors M11/M11b: a local item inside a job (mixed or not) must take the
/// old local path, not the generic one through `LocalBackend`. The rename
/// fast path cannot tell them apart (both are rename(2)); a copy can: only
/// the old path keeps the mode and the modification time.
#[test]
fn local_copies_inside_any_location_job_keep_mode_and_mtime() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, SystemTime};

    let (_root, src, dst) = local_world();
    let file = src.join("keep.txt");
    write(&file, b"k");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options().write(true).open(&file).unwrap().set_modified(old).unwrap();
    let memory = Arc::new(MemoryBackend::posix_like());
    put(memory.as_ref(), "/r", b"r");
    let id = drive("nas");
    let resolver = resolver(vec![(id.clone(), memory.clone() as Arc<dyn Backend>)]);

    for (sources, target) in [
        (vec![kara_vfs::Location::Local(file.clone())], dst.join("a")),
        (vec![kara_vfs::Location::Local(file.clone()), rloc(&id, "/r")], dst.join("b")),
    ] {
        fs::create_dir(&target).unwrap();
        let (outcome, _) = run(
            request(Op::Copy, sources, kara_vfs::Location::Local(target.clone())),
            &resolver,
            &Script::errors(ErrorDecision::Cancel),
        );
        assert!(outcome.report.is_clean(), "{:?}", outcome.report);
        let copied = fs::metadata(target.join("keep.txt")).unwrap();
        assert_eq!(copied.permissions().mode() & 0o777, 0o640, "{target:?}");
        assert_eq!(copied.modified().unwrap(), old, "{target:?}");
    }
}
