//! Compile-time check of the public API (acceptance criterion ac_04): every
//! function is coerced to a fn pointer of the contract's type, every trait is
//! implemented with the contract's exact signatures, every enum is matched
//! exhaustively and every struct is built literally (so a missing or extra
//! public field, or a `#[non_exhaustive]`, fails to compile).

use std::collections::BTreeMap;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use kara_core::FileEntry;
use kara_vfs::conformance::{
    self, CASE_IDS, CaseOutcome, CaseResult, ConformanceError, ConformanceReport,
};
use kara_vfs::memory::{
    Fault, FaultEffect, MemoryBackend, MemoryNode, MemorySnapshot, MemoryStats, Op,
};
use kara_vfs::{
    Backend, BackendError, BackendErrorKind, Cancel, Capabilities, DriveId, DriveIdError, Listing,
    Location, LocationError, RemotePath, RemotePathError, TRANSFER_CHUNK, WriteSession,
};

fn debug<T: Debug + ?Sized>() {}
fn value<T: Debug + Clone + PartialEq + Eq + Hash + PartialOrd + Ord>() {}
fn error<T: std::error::Error + Send + Sync + 'static>() {}

#[test]
fn ac_04_remote_path() {
    let _: fn() -> RemotePath = RemotePath::root;
    let _: fn(&str) -> Result<RemotePath, RemotePathError> = RemotePath::parse;
    let _: fn(&[u8]) -> Result<RemotePath, RemotePathError> = RemotePath::from_bytes;
    let _: for<'a> fn(&'a RemotePath) -> &'a str = RemotePath::as_str;
    let _: fn(&RemotePath) -> bool = RemotePath::is_root;
    let _: fn(&RemotePath) -> Option<RemotePath> = RemotePath::parent;
    let _: for<'a> fn(&'a RemotePath) -> Option<&'a str> = RemotePath::file_name;
    let _: fn(&RemotePath, &str) -> Result<RemotePath, RemotePathError> = RemotePath::join;
    let _: fn(&RemotePath, &RemotePath) -> bool = RemotePath::starts_with;
    fn segments(x: &RemotePath) -> Vec<&str> {
        x.segments().collect()
    }
    let _ = segments;
    fn parse_and_show<T: FromStr<Err = RemotePathError> + Display>() {}
    parse_and_show::<RemotePath>();
    value::<RemotePath>();

    let _ = |e: RemotePathError| match e {
        RemotePathError::Empty => 0,
        RemotePathError::NotAbsolute { raw } => raw.len(),
        RemotePathError::DotSegment { raw } => raw.len(),
        RemotePathError::ContainsNul => 1,
        RemotePathError::NotUtf8 => 2,
        RemotePathError::InvalidSegment { segment } => segment.len(),
    };
    fn clone_eq<T: Clone + PartialEq + Eq>() {}
    clone_eq::<RemotePathError>();
    error::<RemotePathError>();
}

#[test]
fn ac_04_drive_id_and_location() {
    let _: fn(&str, &str) -> Result<DriveId, DriveIdError> = DriveId::new;
    let _: for<'a> fn(&'a DriveId) -> &'a str = DriveId::scheme;
    let _: for<'a> fn(&'a DriveId) -> &'a str = DriveId::name;
    value::<DriveId>();
    let _ = |e: DriveIdError| match e {
        DriveIdError::InvalidScheme { raw } | DriveIdError::InvalidName { raw } => raw,
    };
    error::<DriveIdError>();

    let _: fn(&Location) -> Result<String, LocationError> = Location::to_uri;
    let _: fn(&str) -> Result<Location, LocationError> = Location::from_uri;
    let _: fn(&Location) -> bool = Location::is_local;
    let _: for<'a> fn(&'a Location) -> Option<&'a DriveId> = Location::drive;
    let _: fn(&Location, &Location) -> bool = Location::same_remote_drive;
    let _: fn(&Location) -> Option<Location> = Location::parent;
    fn loc<T: Debug + Clone + PartialEq + Eq + Hash>() {}
    loc::<Location>();
    let _ = |l: Location| match l {
        Location::Local(path) => path,
        Location::Remote { drive, path } => PathBuf::from(format!("{drive:?}{path:?}")),
    };
    let _ = |e: LocationError| match e {
        LocationError::RelativeLocalPath
        | LocationError::QueryOrFragment
        | LocationError::InvalidPercentEncoding
        | LocationError::ContainsNul => String::new(),
        LocationError::UnsupportedScheme { raw } => raw,
        LocationError::NonLocalFileHost { host } => host,
        LocationError::Drive(d) => d.to_string(),
        LocationError::Path(p) => p.to_string(),
    };
    let _: fn(DriveIdError) -> LocationError = LocationError::from;
    let _: fn(RemotePathError) -> LocationError = LocationError::from;
    error::<LocationError>();
}

#[test]
fn ac_04_errors() {
    let _: fn(BackendErrorKind, Option<RemotePath>) -> BackendError = BackendError::new;
    let _: fn(&BackendError) -> BackendErrorKind = BackendError::kind;
    let _: for<'a> fn(io::Error, Option<&'a RemotePath>) -> BackendError = BackendError::from_io;
    let _: fn(BackendError) -> io::Error = io::Error::from;
    fn with_sources(e: BackendError) -> BackendError {
        e.with_source(io::Error::other("io")).with_source("text")
    }
    let _ = with_sources;
    let _ = |kind: BackendErrorKind| BackendError {
        kind,
        path: None::<RemotePath>,
        source: None::<Box<dyn std::error::Error + Send + Sync + 'static>>,
    };
    let _ = |k: BackendErrorKind| match k {
        BackendErrorKind::NotFound
        | BackendErrorKind::AlreadyExists
        | BackendErrorKind::PermissionDenied
        | BackendErrorKind::NoSpace
        | BackendErrorKind::Unavailable
        | BackendErrorKind::AuthRequired
        | BackendErrorKind::Unsupported
        | BackendErrorKind::Cancelled
        | BackendErrorKind::Other => k.to_string(),
    };
    fn kind<T: Debug + Clone + Copy + PartialEq + Eq + Hash + Display>() {}
    kind::<BackendErrorKind>();
    error::<BackendError>();
}

#[test]
fn ac_04_capabilities_cancel_listing_chunk() {
    let all = Capabilities {
        trash: true,
        atomic_rename: true,
        server_side_copy: true,
        real_directories: true,
        posix_permissions: true,
        symlinks: true,
        watch: true,
        undo_rename: true,
        undo_move: true,
    };
    fn caps<T: Debug + Clone + Copy + PartialEq + Eq + Default>() {}
    caps::<Capabilities>();
    let _ = all;

    let _: fn() -> Cancel = Cancel::new;
    let _: fn(&Cancel) = Cancel::cancel;
    let _: fn(&Cancel) -> bool = Cancel::is_cancelled;
    fn cancel<T: Debug + Clone + Default + Send + Sync + 'static>() {}
    cancel::<Cancel>();

    let _ = Listing {
        entries: Vec::<FileEntry>::new(),
        errors: Vec::<BackendError>::new(),
    };
    fn listing<T: Debug + Default>() {}
    listing::<Listing>();

    const CHUNK: usize = TRANSFER_CHUNK;
    assert_eq!(CHUNK, 1 << 20);
}

/// Implementing the traits with the contract's exact signatures: any change in
/// the trait (a parameter, a return type, a new required method) breaks this.
struct Dummy;
struct DummySession;

impl Write for DummySession {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl WriteSession for DummySession {
    fn finish(self: Box<Self>) -> Result<(), BackendError> {
        Ok(())
    }
    fn abort(self: Box<Self>) -> Result<(), BackendError> {
        Ok(())
    }
}

fn nope<T>() -> Result<T, BackendError> {
    Err(BackendError::new(BackendErrorKind::Unsupported, None))
}

impl Backend for Dummy {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
    fn list(&self, _dir: &RemotePath, _cancel: &Cancel) -> Result<Listing, BackendError> {
        nope()
    }
    fn stat(&self, _path: &RemotePath) -> Result<FileEntry, BackendError> {
        nope()
    }
    fn open_read(
        &self,
        _path: &RemotePath,
        _from: u64,
    ) -> Result<Box<dyn Read + Send>, BackendError> {
        nope()
    }
    fn begin_write(
        &self,
        _path: &RemotePath,
        _size_hint: Option<u64>,
        _replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        Ok(Box::new(DummySession))
    }
    fn create_dir(&self, _path: &RemotePath) -> Result<(), BackendError> {
        nope()
    }
    fn rename(&self, _from: &RemotePath, _to: &RemotePath) -> Result<(), BackendError> {
        nope()
    }
    fn remove(&self, _path: &RemotePath) -> Result<(), BackendError> {
        nope()
    }
    fn remove_tree(&self, _path: &RemotePath, _cancel: &Cancel) -> Result<(), BackendError> {
        nope()
    }
    fn copy_within(&self, _from: &RemotePath, _to: &RemotePath) -> Result<(), BackendError> {
        nope()
    }
}

#[test]
fn ac_04_traits_are_object_safe_with_exact_signatures() {
    let shared: Arc<dyn Backend> = Arc::new(Dummy);
    let boxed: Box<dyn Backend> = Box::new(Dummy);
    let session: Box<dyn WriteSession> = Box::new(DummySession);
    fn send_sync<T: Send + Sync + ?Sized>(_: &T) {}
    fn send<T: Send + ?Sized>(_: &T) {}
    send_sync(&shared);
    send_sync(&boxed);
    send(&session);
    fn coerce(m: MemoryBackend) -> Arc<dyn Backend> {
        Arc::new(m)
    }
    let _ = coerce;
}

#[test]
fn ac_04_memory_backend() {
    let _: fn() -> MemoryBackend = MemoryBackend::posix_like;
    let _: fn() -> MemoryBackend = MemoryBackend::object_store_like;
    let _: fn(Capabilities) -> MemoryBackend = MemoryBackend::with_capabilities;
    let _: fn(&MemoryBackend, Fault) -> Result<(), BackendError> = MemoryBackend::inject;
    let _: fn(&MemoryBackend) -> Result<(), BackendError> = MemoryBackend::clear_faults;
    let _: fn(&MemoryBackend, Option<u64>) -> Result<(), BackendError> =
        MemoryBackend::set_capacity;
    let _: fn(&MemoryBackend) -> Result<(), BackendError> = MemoryBackend::disconnect;
    let _: fn(&MemoryBackend) -> Result<(), BackendError> = MemoryBackend::reconnect;
    let _: fn(&MemoryBackend, &RemotePath, &str) -> Result<(), BackendError> =
        MemoryBackend::create_symlink;
    let _: fn(&MemoryBackend) -> Result<MemorySnapshot, BackendError> = MemoryBackend::snapshot;
    let _: fn(&MemoryBackend) -> Result<MemoryStats, BackendError> = MemoryBackend::stats;
    fn backend<T: Backend + Debug + Send + Sync + 'static>() {}
    backend::<MemoryBackend>();

    let _ = |op: Op| match op {
        Op::List
        | Op::Stat
        | Op::OpenRead
        | Op::Read
        | Op::BeginWrite
        | Op::Write
        | Op::Finish
        | Op::CreateDir
        | Op::Rename
        | Op::Remove
        | Op::RemoveTree
        | Op::CopyWithin => op,
    };
    fn op<T: Debug + Clone + Copy + PartialEq + Eq + Hash>() {}
    op::<Op>();

    let _ = |e: FaultEffect| match e {
        FaultEffect::Fail(kind) => kind.to_string(),
        FaultEffect::Cancel(token) => format!("{token:?}"),
    };
    let _ = |path: Option<RemotePath>| Fault {
        op: Op::Stat,
        path,
        after: 0u64,
        effect: FaultEffect::Fail(BackendErrorKind::Other),
        times: None::<u32>,
    };
    fn clone_debug<T: Debug + Clone>() {}
    clone_debug::<Fault>();
    clone_debug::<FaultEffect>();

    let _ = |n: MemoryNode| match n {
        MemoryNode::File { content } => content.len(),
        MemoryNode::Dir | MemoryNode::DirMarker => 0,
        MemoryNode::Symlink { target } => target.len(),
    };
    fn node<T: Debug + Clone + PartialEq + Eq>() {}
    node::<MemoryNode>();
    node::<MemorySnapshot>();
    let _ = MemorySnapshot {
        nodes: BTreeMap::<RemotePath, MemoryNode>::new(),
    };
    let _ = MemoryStats {
        bytes_read: 0u64,
        bytes_written: 0u64,
        open_sessions: 0usize,
    };
    fn stats<T: Debug + Clone + Copy + PartialEq + Eq + Default>() {}
    stats::<MemoryStats>();
}

#[test]
fn ac_04_conformance() {
    let _: fn(&dyn Backend, &RemotePath) -> Result<ConformanceReport, ConformanceError> =
        conformance::run;
    let _: &[&str] = CASE_IDS;
    let _: fn(&ConformanceReport) -> bool = ConformanceReport::is_success;
    let _: for<'a> fn(&'a ConformanceReport) -> Vec<&'a CaseResult> = ConformanceReport::failures;
    let sample = ConformanceReport {
        cases: vec![CaseResult {
            id: "x",
            outcome: CaseOutcome::Passed,
        }],
    };
    let _ = sample;
    let _ = |o: CaseOutcome| match o {
        CaseOutcome::Passed => String::new(),
        CaseOutcome::Failed { detail } => detail,
        CaseOutcome::Skipped { because } => because.to_owned(),
    };
    let _ = |e: ConformanceError| match e {
        ConformanceError::ScratchUnreachable(source) => source.kind,
        ConformanceError::ScratchNotDirectory => BackendErrorKind::Other,
        ConformanceError::ScratchNotEmpty { entries } => {
            let _: usize = entries;
            BackendErrorKind::Other
        }
    };
    fn report<T: Debug + Clone + PartialEq + Eq>() {}
    report::<ConformanceReport>();
    report::<CaseResult>();
    report::<CaseOutcome>();
    error::<ConformanceError>();
    debug::<ConformanceError>();
}
