//! `kara_vfs::conformance` against `ObjectStoreBackend`: over a plain
//! `InMemory` (one `list_with_delimiter` call per folder), over the paged
//! fault-free wrapper (pages of 2 keys, so every listing crosses pages),
//! without conditional puts or copy-if-not-exists, and under a key prefix.

mod objstore_support;

use std::io;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use kara_remote::objstore::object_store::ObjectStore;
use kara_remote::objstore::object_store::memory::InMemory;
use kara_remote::objstore::{ObjectStoreBackend, ObjectStoreOptions};
use kara_vfs::Backend;
use kara_vfs::conformance::{self, CaseOutcome};
use kara_vfs::memory::MemoryBackend;
use objstore_support::{Faulty, backend_over, rp};

fn run_suite(backend: &dyn Backend) -> io::Result<()> {
    let scratch = rp("/scratch")?;
    backend.create_dir(&scratch).map_err(io::Error::other)?;
    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    for report in [
        conformance::run(backend, &scratch).map_err(|e| io::Error::other(e.to_string()))?,
        conformance::run_extra(backend, &scratch).map_err(|e| io::Error::other(e.to_string()))?,
    ] {
        for case in report.cases {
            match case.outcome {
                CaseOutcome::Failed { detail } => failures.push(format!("{}: {detail}", case.id)),
                CaseOutcome::Skipped { because } => skipped.push(format!("{}: {because}", case.id)),
                CaseOutcome::Passed => {}
            }
        }
    }
    assert!(failures.is_empty(), "conformance failures:\n{}", failures.join("\n"));
    // Only the case that needs real directories may be skipped.
    assert!(skipped.is_empty(), "skipped: {skipped:?}");
    Ok(())
}

#[test]
fn the_suite_passes_over_in_memory() -> io::Result<()> {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let backend = ObjectStoreBackend::new(store, ObjectStoreOptions::default()).map_err(io::Error::other)?;
    run_suite(&backend)
}

#[test]
fn the_suite_passes_listing_page_by_page() -> io::Result<()> {
    let store = Faulty::new();
    store.page_size.store(2, Ordering::SeqCst);
    let backend = backend_over(&store, ObjectStoreOptions::default())?;
    run_suite(&backend)?;
    assert!(store.counts.get(objstore_support::Op::List) > 0);
    assert_eq!(store.open_uploads(), 0, "no multipart upload may stay open");
    Ok(())
}

#[test]
fn the_suite_passes_without_conditional_put_or_copy() -> io::Result<()> {
    let store = Faulty::new();
    store.conditional_put.store(false, Ordering::SeqCst);
    store.copy_create.store(false, Ordering::SeqCst);
    let backend = backend_over(&store, ObjectStoreOptions::default())?;
    run_suite(&backend)
}

#[test]
fn the_suite_passes_under_a_prefix_and_nothing_escapes_it() -> io::Result<()> {
    let store = Faulty::new();
    store.put_raw("outside/keep", b"not ours")?;
    let backend = backend_over(
        &store,
        ObjectStoreOptions {
            prefix: String::from("/team//drive/"),
            ..ObjectStoreOptions::default()
        },
    )?;
    assert_eq!(backend.prefix(), "team/drive");
    run_suite(&backend)?;
    let keys = store.keys();
    assert!(
        keys.iter().all(|key| key == "outside/keep" || key.starts_with("team/drive/")),
        "{keys:?}"
    );
    assert_eq!(store.bytes("outside/keep"), Some(b"not ours".to_vec()));
    Ok(())
}

#[test]
fn the_suite_passes_with_small_multipart_parts() -> io::Result<()> {
    // Every file above 4 KiB goes through a multipart upload with 3 parts in flight.
    let store = Faulty::new();
    store.page_size.store(3, Ordering::SeqCst);
    let backend = objstore_support::backend_with_parts(&store, 4096, 3)?;
    run_suite(&backend)?;
    assert!(store.counts.get(objstore_support::Op::Complete) > 0, "multipart was used");
    assert_eq!(store.open_uploads(), 0, "every aborted or dropped upload was aborted");
    Ok(())
}

#[test]
fn capabilities_are_the_object_store_profile_and_never_change() -> io::Result<()> {
    let store = Faulty::new();
    let backend = backend_over(&store, ObjectStoreOptions::default())?;
    let model = MemoryBackend::object_store_like().capabilities();
    assert_eq!(backend.capabilities(), model);
    assert_eq!(ObjectStoreBackend::CAPABILITIES, model);
    store.unplugged.store(true, Ordering::SeqCst);
    let _ = backend.stat(&rp("/x")?);
    assert_eq!(backend.capabilities(), model, "a dead endpoint changes nothing");
    Ok(())
}
