//! Each divergence the differential test (`differential_memory_local.rs`)
//! found between MemoryBackend and LocalBackend, pinned as a plain scenario run
//! on both, with the answer both now give.

mod local_backend_support;

use std::io::{self, Read, Write};

use kara_fs::LocalBackend;
use kara_vfs::memory::MemoryBackend;
use kara_vfs::{Backend, BackendError, BackendErrorKind as K, Cancel, RemotePath};
use local_backend_support::rp;

/// A drive of each kind plus a way to make links on it.
struct Pair {
    _tmp: tempfile::TempDir,
    local: LocalBackend,
    memory: MemoryBackend,
}

impl Pair {
    fn new() -> io::Result<Pair> {
        let tmp = tempfile::tempdir()?;
        let local = LocalBackend::with_root(tmp.path()).map_err(io::Error::from)?;
        Ok(Pair {
            _tmp: tmp,
            local,
            memory: MemoryBackend::posix_like(),
        })
    }

    fn both(&self) -> [(&'static str, &dyn Backend); 2] {
        [("memory", &self.memory), ("local", &self.local)]
    }

    fn mkdir(&self, path: &str) -> io::Result<()> {
        for (_, b) in self.both() {
            b.create_dir(&rp(path)?)?;
        }
        Ok(())
    }

    fn file(&self, path: &str, content: &[u8]) -> io::Result<()> {
        for (_, b) in self.both() {
            put(b, &rp(path)?, content, false)?;
        }
        Ok(())
    }

    fn link(&self, path: &str, target: &str) -> io::Result<()> {
        self.memory.create_symlink(&rp(path)?, target)?;
        std::os::unix::fs::symlink(target, self.local.to_local(&rp(path)?))
    }

    /// Runs `op` on both and checks both fail with `kind` naming `path`.
    fn both_fail<T>(
        &self,
        what: &str,
        op: impl Fn(&dyn Backend) -> Result<T, BackendError>,
        kind: K,
        path: &str,
    ) -> io::Result<()> {
        let expected = rp(path)?;
        for (label, b) in self.both() {
            match op(b) {
                Ok(_) => panic!("{label}: {what}: expected {kind:?}, got Ok"),
                Err(e) => {
                    assert_eq!(e.kind, kind, "{label}: {what}: {e:?}");
                    assert_eq!(e.path.as_ref(), Some(&expected), "{label}: {what}");
                }
            }
        }
        Ok(())
    }
}

/// A test path; the literals here always parse.
fn q(text: &str) -> RemotePath {
    match RemotePath::parse(text) {
        Ok(path) => path,
        Err(e) => panic!("{text:?}: {e}"),
    }
}

fn put(b: &dyn Backend, path: &RemotePath, content: &[u8], replace: bool) -> io::Result<()> {
    let mut session = b.begin_write(path, None, replace)?;
    session.write_all(content)?;
    session.finish()?;
    Ok(())
}

fn get(b: &dyn Backend, path: &str) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    b.open_read(&rp(path)?, 0)?.read_to_end(&mut out)?;
    Ok(out)
}

#[test]
fn a_file_above_the_parent_makes_the_parent_missing() -> io::Result<()> {
    let pair = Pair::new()?;
    pair.file("/f", b"f")?;
    pair.file("/src", b"s")?;
    pair.both_fail("create_dir", |b| b.create_dir(&q("/f/x/y")), K::NotFound, "/f/x/y")?;
    pair.both_fail(
        "begin_write",
        |b| b.begin_write(&q("/f/x/y"), None, true).map(drop),
        K::NotFound,
        "/f/x/y",
    )?;
    pair.both_fail("rename", |b| b.rename(&q("/src"), &q("/f/x/y")), K::NotFound, "/f/x/y")?;
    // Directly under the file it stays "not a directory".
    pair.both_fail("create_dir", |b| b.create_dir(&q("/f/x")), K::Other, "/f/x")?;
    Ok(())
}

#[test]
fn a_link_resolving_under_a_file_lists_as_missing() -> io::Result<()> {
    let pair = Pair::new()?;
    pair.file("/f", b"f")?;
    pair.link("/l", "f/x")?;
    pair.both_fail("list", |b| b.list(&q("/l"), &Cancel::new()), K::NotFound, "/l")?;
    for (label, b) in pair.both() {
        let entry = b.stat(&q("/l"))?;
        assert!(entry.symlink_broken, "{label}");
    }
    Ok(())
}

#[test]
fn a_path_through_a_link_cycle_does_not_exist() -> io::Result<()> {
    let pair = Pair::new()?;
    pair.link("/a", "b")?;
    pair.link("/b", "a")?;
    let none = || Cancel::new();
    pair.both_fail("list", |b| b.list(&q("/a"), &none()), K::NotFound, "/a")?;
    pair.both_fail("open_read", |b| b.open_read(&q("/a"), 0).map(drop), K::NotFound, "/a")?;
    pair.both_fail("stat below", |b| b.stat(&q("/a/x")), K::NotFound, "/a/x")?;
    pair.both_fail("remove below", |b| b.remove(&q("/a/x")), K::NotFound, "/a/x")?;
    pair.both_fail("create below", |b| b.create_dir(&q("/a/x")), K::NotFound, "/a/x")?;
    Ok(())
}

#[test]
fn a_dangling_link_as_the_parent_is_a_missing_parent() -> io::Result<()> {
    let pair = Pair::new()?;
    pair.link("/d", "nowhere")?;
    pair.both_fail("create_dir", |b| b.create_dir(&q("/d/x")), K::NotFound, "/d/x")?;
    pair.both_fail(
        "begin_write",
        |b| b.begin_write(&q("/d/x"), None, false).map(drop),
        K::NotFound,
        "/d/x",
    )?;
    Ok(())
}

#[test]
fn only_a_directory_can_be_moved_into_itself() -> io::Result<()> {
    let pair = Pair::new()?;
    pair.file("/f", b"f")?;
    pair.mkdir("/d")?;
    // A file "into itself" has no parent: not a directory / missing parent.
    pair.both_fail("file into itself", |b| b.rename(&q("/f"), &q("/f/x")), K::Other, "/f/x")?;
    pair.both_fail(
        "file deeper into itself",
        |b| b.rename(&q("/f"), &q("/f/x/y")),
        K::NotFound,
        "/f/x/y",
    )?;
    // A directory into a missing parent below itself: the parent is missing.
    pair.both_fail(
        "dir into a missing parent below itself",
        |b| b.rename(&q("/d"), &q("/d/missing/x")),
        K::NotFound,
        "/d/missing/x",
    )?;
    // Into itself through a link, checked before "to exists".
    pair.mkdir("/d/sub")?;
    pair.file("/d/sub/taken", b"t")?;
    pair.link("/via", "d/sub")?;
    pair.both_fail(
        "dir into itself through a link",
        |b| b.rename(&q("/d"), &q("/via/taken")),
        K::Other,
        "/via/taken",
    )?;
    pair.both_fail(
        "dir into itself through a link, new name",
        |b| b.rename(&q("/d"), &q("/via/new")),
        K::Other,
        "/via/new",
    )?;
    Ok(())
}

#[test]
fn two_spellings_of_one_entry_rename_as_a_no_op() -> io::Result<()> {
    let pair = Pair::new()?;
    pair.mkdir("/d")?;
    pair.file("/d/f", b"keep")?;
    pair.link("/d/self", ".")?;
    for (label, b) in pair.both() {
        b.rename(&q("/d/self/f"), &q("/d/f"))
            .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        assert_eq!(get(b, "/d/f")?, b"keep", "{label}");
    }
    Ok(())
}

#[test]
fn a_write_that_replaces_a_link_on_its_own_path_commits_and_says_so() -> io::Result<()> {
    let pair = Pair::new()?;
    pair.link("/l", ".")?;
    for (label, b) in pair.both() {
        // `/l/l/l` is the link `/l` itself (in the root, through `/l -> .`
        // twice): replacing it turns `/l` into a file, after which the path of
        // its directory, `/l/l`, no longer resolves. The commit must still
        // report success.
        put(b, &q("/l/l/l"), b"new", true).unwrap_or_else(|e| panic!("{label}: {e:?}"));
        let entry = b.stat(&q("/l"))?;
        assert!(!entry.is_symlink, "{label}: the link became the file");
        assert_eq!(get(b, "/l")?, b"new", "{label}");
    }
    let leftovers = local_backend_support::temp_names(pair._tmp.path())?;
    assert!(leftovers.is_empty(), "{leftovers:?}");
    Ok(())
}
