//! `kara_vfs::conformance` against `SftpBackend` over the in-process server,
//! with and without `posix-rename@openssh.com`, and with `root` set.

mod support;

use std::fs;
use std::io;

use kara_vfs::Backend;
use kara_vfs::conformance::{self, CaseOutcome};
use support::{Fixture, ServerOptions, rp};

fn run_suite(options: ServerOptions, extra: &[(&str, &str)]) -> io::Result<()> {
    let fixture = Fixture::new(options, extra)?;
    let scratch_local = fixture.server.root().join("scratch");
    fs::create_dir(&scratch_local)?;
    let scratch = rp("/scratch")?;
    let backend: &dyn Backend = fixture.backend.as_ref();

    let mut failures = Vec::new();
    for report in [
        conformance::run(backend, &scratch).map_err(|e| io::Error::other(e.to_string()))?,
        conformance::run_extra(backend, &scratch).map_err(|e| io::Error::other(e.to_string()))?,
    ] {
        for case in report.cases {
            if let CaseOutcome::Failed { detail } = case.outcome {
                failures.push(format!("{}: {detail}", case.id));
            }
        }
    }
    assert!(failures.is_empty(), "conformance failures:\n{}", failures.join("\n"));
    assert!(
        fixture.server.temporaries().is_empty(),
        "temporaries left on the server: {:?}",
        fixture.server.temporaries()
    );
    Ok(())
}

#[test]
fn the_suite_passes_with_posix_rename() -> io::Result<()> {
    run_suite(ServerOptions::default(), &[])
}

#[test]
fn the_suite_passes_without_posix_rename_or_fsync() -> io::Result<()> {
    run_suite(
        ServerOptions {
            posix_rename: false,
            fsync: false,
            ..ServerOptions::default()
        },
        &[],
    )
}

#[test]
fn the_suite_passes_with_strerror_messages() -> io::Result<()> {
    run_suite(
        ServerOptions {
            strerror_messages: true,
            ..ServerOptions::default()
        },
        &[],
    )
}

#[test]
fn the_suite_passes_under_a_start_folder() -> io::Result<()> {
    let options = ServerOptions::default();
    let server = support::TestServer::start(options)?;
    fs::create_dir_all(server.root().join("home/kara/scratch"))?;
    let client = tempfile::tempdir()?;
    let known = support::trusting_known_hosts(&server, client.path())?;
    let backend = support::connect(&server, &known, &[("root", "/home/kara")])?;
    let scratch = rp("/scratch")?;
    let report = conformance::run(backend.as_ref(), &scratch).map_err(|e| io::Error::other(e.to_string()))?;
    let failures: Vec<String> = report
        .failures()
        .iter()
        .map(|case| format!("{}: {:?}", case.id, case.outcome))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // Nothing escaped the start folder.
    let top: Vec<String> = fs::read_dir(server.root())?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(top, vec![String::from("home")]);
    Ok(())
}

#[test]
fn capabilities_follow_the_extension_probe() -> io::Result<()> {
    let with = Fixture::standard()?;
    let caps = with.backend.capabilities();
    assert!(caps.atomic_rename && caps.undo_rename && caps.undo_move);
    assert!(caps.real_directories && caps.posix_permissions && caps.symlinks);
    assert!(!caps.trash && !caps.server_side_copy && !caps.watch);

    let without = Fixture::new(
        ServerOptions {
            posix_rename: false,
            ..ServerOptions::default()
        },
        &[],
    )?;
    let caps = without.backend.capabilities();
    assert!(!caps.atomic_rename && !caps.undo_rename && !caps.undo_move);
    assert!(caps.real_directories && caps.posix_permissions && caps.symlinks);
    Ok(())
}
