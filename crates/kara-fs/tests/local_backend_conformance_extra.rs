//! LocalBackend passes `conformance::run_extra` (drive root, paths under a
//! file, cancel racing a list, a transfer cancelled between chunks), rooted at
//! a tempdir, rooted at a symlink to one, and as the whole system.

mod local_backend_support;

use std::fs;
use std::io;
use std::path::Path;

use kara_fs::LocalBackend;
use kara_vfs::conformance::{self, EXTRA_CASE_IDS};
use kara_vfs::{Backend, RemotePath};
use local_backend_support::{disk_tempdir, rooted, tmp_tempdir};

fn run_and_check(backend: &dyn Backend, scratch: &RemotePath, local_scratch: &Path) {
    let report = match conformance::run_extra(backend, scratch) {
        Ok(report) => report,
        Err(error) => panic!("run_extra refused to run on {scratch}: {error:?}"),
    };
    let ids: Vec<&str> = report.cases.iter().map(|case| case.id).collect();
    assert_eq!(ids, EXTRA_CASE_IDS.to_vec());
    assert!(
        report.is_success(),
        "extra conformance failures on {scratch}: {:#?}",
        report.failures()
    );
    let left = fs::read_dir(local_scratch).map(Iterator::count).unwrap_or(usize::MAX);
    assert_eq!(left, 0, "the scratch is left empty");
}

#[test]
fn rooted_at_a_tempdir_on_tmp_and_on_disk() -> io::Result<()> {
    for tmp in [tmp_tempdir()?, disk_tempdir()?] {
        let backend = rooted(tmp.path())?;
        run_and_check(&backend, &RemotePath::root(), tmp.path());
    }
    Ok(())
}

#[test]
fn rooted_at_a_symlink_to_a_directory() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let real = tmp.path().join("real");
    fs::create_dir(&real)?;
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&real, &link)?;
    let backend = rooted(&link)?;
    run_and_check(&backend, &RemotePath::root(), &real);
    Ok(())
}

#[test]
fn as_the_whole_system() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    let backend = LocalBackend::system();
    let scratch = backend
        .to_remote(tmp.path())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    run_and_check(&backend, &scratch, tmp.path());
    Ok(())
}

/// A cancel that lands while `list` runs gives the complete listing or
/// `Cancelled` naming the directory, never a truncated `Ok`. (The kara-vfs
/// suite cannot start the second thread this needs.)
#[test]
fn a_cancel_racing_list_never_returns_a_truncated_listing() -> io::Result<()> {
    use kara_vfs::{BackendErrorKind, Cancel};

    const FILES: usize = 3000;
    let tmp = tmp_tempdir()?;
    let dir = tmp.path().join("many");
    fs::create_dir(&dir)?;
    for n in 0..FILES {
        fs::write(dir.join(format!("f{n:04}")), b"")?;
    }
    let backend = rooted(tmp.path())?;
    let remote = local_backend_support::rp("/many")?;
    for spin in [0_u32, 100, 10_000, 100_000, 1_000_000] {
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
        let result = backend.list(&remote, &token);
        let _ = canceller.join();
        match result {
            Ok(listing) => {
                assert_eq!(listing.entries.len(), FILES, "spin {spin}: truncated Ok");
                assert!(listing.errors.is_empty());
            }
            Err(error) => {
                assert_eq!(error.kind, BackendErrorKind::Cancelled, "spin {spin}");
                assert_eq!(error.path.as_ref(), Some(&remote), "spin {spin}");
            }
        }
    }
    Ok(())
}
