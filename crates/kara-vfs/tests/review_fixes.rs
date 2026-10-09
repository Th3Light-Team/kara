//! Regressions for the supervisor's findings on the first MemoryBackend. Each one
//! was a case where the implementation matched what the tests checked and not
//! what the contract says (cb_12, cb_18, cb_19, cb_36).

mod common;

use common::{err, mkdir, names, p, read, write};
use kara_core::EntryKind;
use kara_vfs::memory::{Fault, FaultEffect, MemoryBackend, Op};
use kara_vfs::{Backend, BackendErrorKind as K, Cancel};

fn rename_fault(after: u64) -> Fault {
    Fault {
        op: Op::Rename,
        path: None,
        after,
        effect: FaultEffect::Fail(K::PermissionDenied),
        times: Some(1),
    }
}

// cb_36: with atomic_rename the fault fires whatever its `after`, and the
// tree stays entirely under `from`.
#[test]
fn cb_36_atomic_rename_fault_with_any_after_leaves_the_tree_under_from() {
    for after in 0..10 {
        let m = MemoryBackend::posix_like();
        mkdir(&m, "/src");
        for i in 0..10 {
            write(&m, &format!("/src/f{i}"), b"x");
        }
        m.inject(rename_fault(after)).expect("inject");
        let e = err(m.rename(&p("/src"), &p("/dst")), "rename with a fault");
        assert_eq!(e.kind, K::PermissionDenied, "after = {after}");
        assert_eq!(names(&m, "/src").len(), 10, "after = {after}: all under /src");
        assert!(m.stat(&p("/dst")).is_err(), "after = {after}: nothing under /dst");
    }
}

// cb_18: a link entry carries the size of the link itself, broken or not.
#[test]
fn cb_18_symlink_entries_carry_the_size_of_the_link() {
    let m = MemoryBackend::posix_like();
    write(&m, "/target", b"0123456789");
    m.create_symlink(&p("/to-file"), "/target").expect("link");
    m.create_symlink(&p("/broken"), "/missing-target").expect("link");

    let to_file = m.stat(&p("/to-file")).expect("stat");
    assert_eq!(to_file.kind, EntryKind::File);
    assert_eq!(to_file.size, Some("/target".len() as u64));

    let broken = m.stat(&p("/broken")).expect("stat");
    assert!(broken.symlink_broken);
    assert_eq!(broken.kind, EntryKind::File);
    assert_eq!(broken.size, Some("/missing-target".len() as u64));
}

// A relative link with `..` is the common case on POSIX servers.
#[test]
fn relative_links_with_dot_dot_resolve() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/t");
    mkdir(&m, "/l");
    write(&m, "/t/x.txt", b"data");
    m.create_symlink(&p("/l/up"), "../t/x.txt").expect("link");

    assert!(!m.stat(&p("/l/up")).expect("stat").symlink_broken);
    let mut got = Vec::new();
    std::io::Read::read_to_end(&mut m.open_read(&p("/l/up"), 0).expect("open"), &mut got)
        .expect("read");
    assert_eq!(got, b"data");
}

// cb_19: what `list` shows under a linked directory can be used.
#[test]
fn cb_19_children_of_a_linked_directory_are_usable() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/t");
    write(&m, "/t/x.txt", b"data");
    m.create_symlink(&p("/link"), "/t").expect("link");

    assert_eq!(names(&m, "/link"), names(&m, "/t"));
    let listed = m.stat(&p("/link/x.txt")).expect("stat under the link");
    assert_eq!(listed.size, Some(4));
    assert_eq!(read(&m, "/link/x.txt"), b"data");

    write(&m, "/link/new.txt", b"new");
    assert_eq!(read(&m, "/t/new.txt"), b"new", "a write lands in the target");

    m.rename(&p("/link/new.txt"), &p("/link/moved.txt")).expect("rename");
    assert_eq!(read(&m, "/t/moved.txt"), b"new");

    m.create_dir(&p("/link/sub")).expect("create_dir");
    assert!(m.stat(&p("/t/sub")).is_ok());

    m.remove(&p("/link/moved.txt")).expect("remove");
    assert!(m.stat(&p("/t/moved.txt")).is_err());
}

// Destructive operations on the link itself act on the link (cb_39).
#[test]
fn removing_the_link_keeps_the_target() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/t");
    write(&m, "/t/x.txt", b"data");
    m.create_symlink(&p("/link"), "/t").expect("link");
    m.remove(&p("/link")).expect("remove the link");
    assert_eq!(read(&m, "/t/x.txt"), b"data");
}

// Errors under a link still name the path the caller used (cb_12).
#[test]
fn errors_under_a_link_name_the_used_path() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/t");
    m.create_symlink(&p("/link"), "/t").expect("link");
    let e = err(m.stat(&p("/link/missing")), "stat");
    assert_eq!(e.kind, K::NotFound);
    assert_eq!(e.path, Some(p("/link/missing")));
}

// A fault on `list` fires on an empty directory too.
#[test]
fn list_fault_fires_on_an_empty_directory() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/empty");
    m.inject(Fault {
        op: Op::List,
        path: None,
        after: 0,
        effect: FaultEffect::Fail(K::PermissionDenied),
        times: Some(1),
    })
    .expect("inject");
    let e = err(m.list(&p("/empty"), &Cancel::new()), "list");
    assert_eq!(e.kind, K::PermissionDenied);
}

// copy_within counts against the capacity like any other write.
#[test]
fn copy_within_respects_the_capacity() {
    let m = MemoryBackend::object_store_like();
    write(&m, "/big", &[7u8; 80]);
    m.set_capacity(Some(100)).expect("capacity");
    let e = err(m.copy_within(&p("/big"), &p("/copy")), "copy_within");
    assert_eq!(e.kind, K::NoSpace);
    assert_eq!(e.path, Some(p("/copy")));
    assert!(m.stat(&p("/copy")).is_err(), "nothing was stored");
}

// cb_12 / cb_45: a backend whose errors never name a path must not pass the
// suite. Before, only a subset of the cases looked at the path.
mod nameless_errors {
    use std::io::Read;

    use kara_core::FileEntry;
    use kara_vfs::conformance;
    use kara_vfs::{
        Backend, BackendError, Cancel, Capabilities, Listing, RemotePath, WriteSession,
    };

    use super::{MemoryBackend, mkdir, p};

    /// Forwards everything and drops the path from every error.
    struct Nameless(MemoryBackend);

    fn strip<T>(r: Result<T, BackendError>) -> Result<T, BackendError> {
        r.map_err(|mut e| {
            e.path = None;
            e
        })
    }

    impl Backend for Nameless {
        fn capabilities(&self) -> Capabilities {
            self.0.capabilities()
        }
        fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
            strip(self.0.list(dir, cancel))
        }
        fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
            strip(self.0.stat(path))
        }
        fn open_read(
            &self,
            path: &RemotePath,
            from: u64,
        ) -> Result<Box<dyn Read + Send>, BackendError> {
            strip(self.0.open_read(path, from))
        }
        fn begin_write(
            &self,
            path: &RemotePath,
            size_hint: Option<u64>,
            replace: bool,
        ) -> Result<Box<dyn WriteSession>, BackendError> {
            strip(self.0.begin_write(path, size_hint, replace))
        }
        fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
            strip(self.0.create_dir(path))
        }
        fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
            strip(self.0.rename(from, to))
        }
        fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
            strip(self.0.remove(path))
        }
        fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
            strip(self.0.remove_tree(path, cancel))
        }
        fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
            strip(self.0.copy_within(from, to))
        }
    }

    #[test]
    fn a_backend_that_names_no_path_fails_the_suite() {
        for backend in [MemoryBackend::posix_like(), MemoryBackend::object_store_like()] {
            mkdir(&backend, "/scratch");
            let wrapped = Nameless(backend);
            let report = conformance::run(&wrapped, &p("/scratch")).expect("suite runs");
            let failed: Vec<_> = report.failures().iter().map(|r| r.id).collect();
            for id in [
                "list_file_is_error",
                "list_precancelled",
                "read_missing_and_dir",
                "rename_into_own_subtree",
                "remove_nonempty_dir",
                "remove_tree_precancelled",
            ] {
                assert!(failed.contains(&id), "{id} should fail, failures: {failed:?}");
            }
        }
    }
}

// Two different strings must not name the same place: bookmarks and history
// deduplicate by URI.
mod uri_aliases {
    use kara_vfs::location::Location;

    #[test]
    fn non_canonical_forms_are_refused() {
        for uri in [
            "kara+sftp://w/a%2Fb",
            "kara+sftp://w//x//",
            "kara+sftp://w/a%c3%a9",
            "file:///a b",
        ] {
            assert!(Location::from_uri(uri).is_err(), "{uri} should be refused");
        }
    }

    #[test]
    fn canonical_forms_round_trip() {
        for uri in ["kara+sftp://w/a/b", "kara+sftp://w/a%C3%A9", "kara+sftp://w", "file:///a%20b"] {
            let location = Location::from_uri(uri).expect(uri);
            assert_eq!(Location::from_uri(&location.to_uri().expect("uri")), Ok(location));
        }
    }
}

mod second_review {
    use std::io::{Read, Write};

    use kara_vfs::location::{Location, LocationError};
    use kara_vfs::memory::MemoryBackend;
    use kara_vfs::{Backend, BackendErrorKind as K};

    use super::{err, mkdir, p, read, write};

    #[test]
    fn local_uris_must_be_canonical() {
        for uri in ["file:///a//b", "file:///a/", "file:///a/./b"] {
            assert_eq!(
                Location::from_uri(uri),
                Err(LocationError::InvalidPercentEncoding),
                "{uri}"
            );
        }
        assert!(Location::from_uri("file:///").is_ok());
        assert!(Location::from_uri("file:///a/b").is_ok());
    }

    // A link whose target goes through another linked directory.
    #[test]
    fn a_link_through_a_linked_directory_resolves() {
        let m = MemoryBackend::posix_like();
        mkdir(&m, "/t");
        write(&m, "/t/f", b"deep");
        m.create_symlink(&p("/l1"), "/t").expect("link");
        m.create_symlink(&p("/l2"), "/l1/f").expect("link");
        assert!(!m.stat(&p("/l2")).expect("stat").symlink_broken);
        assert_eq!(read(&m, "/l2"), b"deep");
    }

    #[test]
    fn a_cycle_of_links_ends() {
        let m = MemoryBackend::posix_like();
        m.create_symlink(&p("/a"), "/b").expect("link");
        m.create_symlink(&p("/b"), "/a").expect("link");
        assert!(m.stat(&p("/a")).expect("stat").symlink_broken);
        assert!(m.open_read(&p("/a"), 0).is_err());
    }

    // create_symlink follows links on the way, like every other mutator.
    #[test]
    fn create_symlink_below_a_linked_directory() {
        let m = MemoryBackend::posix_like();
        mkdir(&m, "/t");
        m.create_symlink(&p("/d"), "/t").expect("link");
        m.create_symlink(&p("/d/ln"), "/elsewhere").expect("link below a link");
        assert!(m.stat(&p("/t/ln")).is_ok());
    }

    // A session or reader opened before a disconnect is dead after reconnect.
    #[test]
    fn disconnect_kills_open_sessions_and_readers() {
        let m = MemoryBackend::posix_like();
        write(&m, "/r", b"0123456789");
        let mut reader = m.open_read(&p("/r"), 0).expect("open");
        let mut session = m.begin_write(&p("/w"), None, false).expect("begin");
        session.write_all(b"half").expect("write");

        m.disconnect().expect("disconnect");
        m.reconnect().expect("reconnect");

        let e = err(session.finish(), "finish after a reconnect");
        assert_eq!(e.kind, K::Unavailable);
        assert!(m.stat(&p("/w")).is_err(), "nothing was committed");
        let mut buf = [0u8; 4];
        assert!(reader.read(&mut buf).is_err(), "the old reader is dead");
        // New work after the reconnect is fine.
        write(&m, "/after", b"ok");
        assert_eq!(read(&m, "/after"), b"ok");
    }
}
