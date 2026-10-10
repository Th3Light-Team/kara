//! Links whose target runs through the link itself (`/l -> l/x`) or through a
//! cycle of links must resolve to "broken" in bounded work. Found by the
//! differential test against LocalBackend: the resolution recursed through
//! the link's own path before spending any of its budget, and overflowed the
//! stack.

mod common;

use common::{mkdir, p, write};
use kara_core::EntryKind;
use kara_vfs::memory::MemoryBackend;
use kara_vfs::{Backend, BackendErrorKind as K, Cancel};

fn every_operation_through(m: &MemoryBackend, base: &str) {
    let below = format!("{base}/y");
    let deeper = format!("{base}/y/z");
    let c = Cancel::new();
    for path in [base, below.as_str(), deeper.as_str()] {
        let _ = m.stat(&p(path));
        let _ = m.list(&p(path), &c);
        let _ = m.open_read(&p(path), 0);
        let _ = m.create_dir(&p(&format!("{path}/new")));
        let _ = m.begin_write(&p(&format!("{path}/f")), None, false);
        let _ = m.rename(&p(path), &p("/elsewhere"));
        let _ = m.rename(&p("/plain"), &p(&format!("{path}/moved")));
        let _ = m.create_symlink(&p(&format!("{path}/l2")), "x");
    }
}

#[test]
fn a_link_through_itself_is_broken_and_nothing_recurses_forever() {
    let m = MemoryBackend::posix_like();
    write(&m, "/plain", b"p");
    m.create_symlink(&p("/l"), "l/x").expect("link");
    m.create_symlink(&p("/m"), "m/m/m/a").expect("link");

    let entry = m.stat(&p("/l")).expect("stat of the link itself");
    assert!(entry.is_symlink && entry.symlink_broken);
    assert_eq!(entry.kind, EntryKind::File);
    assert!(m.stat(&p("/m")).expect("stat").symlink_broken);

    assert_eq!(
        m.stat(&p("/l/y")).map_err(|e| e.kind).err(),
        Some(K::NotFound)
    );
    assert_eq!(
        m.create_dir(&p("/l/y")).map_err(|e| e.kind).err(),
        Some(K::NotFound),
        "the parent resolves nowhere"
    );
    every_operation_through(&m, "/l");
    every_operation_through(&m, "/m");
    assert_eq!(common::read(&m, "/plain"), b"p", "nothing else was touched");
}

#[test]
fn a_cycle_of_links_through_their_parents_ends() {
    let m = MemoryBackend::posix_like();
    mkdir(&m, "/d");
    write(&m, "/plain", b"p");
    m.create_symlink(&p("/a"), "b/x").expect("link");
    m.create_symlink(&p("/b"), "a/x").expect("link");
    m.create_symlink(&p("/d/up"), "../d/up/up").expect("link");
    for path in ["/a", "/b", "/d/up"] {
        assert!(m.stat(&p(path)).expect("stat").symlink_broken, "{path}");
        every_operation_through(&m, path);
    }
    assert_eq!(common::read(&m, "/plain"), b"p");
}
