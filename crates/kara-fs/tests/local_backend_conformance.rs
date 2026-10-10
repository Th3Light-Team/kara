//! cb_01: LocalBackend passes the generic conformance suite of kara-vfs, rooted
//! at a tempdir and as the whole system, on /tmp and on the build's disk.

mod local_backend_support;

use std::fs;
use std::io;
use std::path::Path;

use kara_fs::LocalBackend;
use kara_vfs::conformance::{self, CASE_IDS, CaseOutcome};
use kara_vfs::{Backend, RemotePath};
use local_backend_support::{disk_tempdir, rooted, tmp_tempdir};

fn run_and_check(backend: &dyn Backend, scratch: &RemotePath, local_scratch: &Path) {
    let report = match conformance::run(backend, scratch) {
        Ok(report) => report,
        Err(error) => panic!("the suite refused to run on {scratch}: {error:?}"),
    };

    let failures: Vec<String> = report
        .failures()
        .iter()
        .map(|case| format!("{}: {:?}", case.id, case.outcome))
        .collect();
    assert!(
        report.is_success(),
        "conformance failures on {scratch}:\n{}",
        failures.join("\n")
    );

    let ids: Vec<&str> = report.cases.iter().map(|case| case.id).collect();
    assert_eq!(ids, CASE_IDS.to_vec(), "case ids must follow CASE_IDS");

    let mut passed = 0;
    let mut skipped = Vec::new();
    for case in &report.cases {
        match &case.outcome {
            CaseOutcome::Passed => passed += 1,
            CaseOutcome::Skipped { because } => skipped.push((case.id, *because)),
            CaseOutcome::Failed { detail } => panic!("{} failed: {detail}", case.id),
        }
    }
    assert_eq!(
        skipped,
        vec![
            ("copy_within", "server_side_copy=false"),
            ("implicit_directories", "real_directories=true"),
        ],
        "exactly the two capability-driven cases are skipped"
    );
    assert_eq!(passed, 37, "37 cases pass");
    assert_eq!(CASE_IDS.len(), 39);

    let left: Vec<_> = match fs::read_dir(local_scratch) {
        Ok(items) => items
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect(),
        Err(error) => panic!("scratch {} unreadable: {error}", local_scratch.display()),
    };
    assert!(
        left.is_empty(),
        "the suite must leave the scratch empty, found {left:?}"
    );
}

fn rooted_run(dir: &Path) -> io::Result<()> {
    let backend = rooted(dir)?;
    run_and_check(&backend, &RemotePath::root(), dir);
    Ok(())
}

fn system_run(dir: &Path) -> io::Result<()> {
    let backend = LocalBackend::system();
    let scratch = match backend.to_remote(dir) {
        Ok(path) => path,
        Err(error) => panic!("to_remote({}) failed: {error}", dir.display()),
    };
    run_and_check(&backend, &scratch, dir);
    Ok(())
}

#[test]
fn cb_01_conformance_passes_rooted_at_a_tempdir_on_tmp() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    rooted_run(tmp.path())
}

#[test]
fn cb_01_conformance_passes_rooted_at_a_tempdir_on_disk() -> io::Result<()> {
    let tmp = disk_tempdir()?;
    rooted_run(tmp.path())
}

#[test]
fn cb_01_conformance_passes_on_the_system_backend_on_tmp() -> io::Result<()> {
    let tmp = tmp_tempdir()?;
    system_run(tmp.path())
}

#[test]
fn cb_01_conformance_passes_on_the_system_backend_on_disk() -> io::Result<()> {
    let tmp = disk_tempdir()?;
    system_run(tmp.path())
}
