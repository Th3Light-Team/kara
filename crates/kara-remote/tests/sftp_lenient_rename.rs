//! Servers whose plain `SSH_FXP_RENAME` is rename(2) and silently replaces
//! (OpenSSH never does): the backend must still never overwrite.

mod support;

use std::fs;
use std::io::{self, Write};

use kara_vfs::conformance;
use kara_vfs::{Backend, BackendErrorKind};
use support::{Fixture, ServerOptions, rp};

fn lenient() -> ServerOptions {
    ServerOptions {
        rename_overwrites: true,
        posix_rename: false,
        ..ServerOptions::default()
    }
}

#[test]
fn rename_refuses_an_existing_target_even_if_the_server_would_replace_it() -> io::Result<()> {
    let fixture = Fixture::new(lenient(), &[])?;
    fs::write(fixture.server.root().join("a"), b"A")?;
    fs::write(fixture.server.root().join("b"), b"B")?;
    let error = fixture
        .backend
        .rename(&rp("/a")?, &rp("/b")?)
        .err()
        .ok_or_else(|| io::Error::other("rename succeeded"))?;
    assert_eq!(error.kind, BackendErrorKind::AlreadyExists);
    assert_eq!(error.path, Some(rp("/b")?));
    assert_eq!(fs::read(fixture.server.root().join("b"))?, b"B");
    assert_eq!(fs::read(fixture.server.root().join("a"))?, b"A");
    Ok(())
}

#[test]
fn finish_without_replace_refuses_a_target_that_appeared_meanwhile() -> io::Result<()> {
    let fixture = Fixture::new(lenient(), &[])?;
    let target = rp("/late")?;
    let mut session = fixture
        .backend
        .begin_write(&target, None, false)
        .map_err(io::Error::other)?;
    session.write_all(b"mine")?;
    fs::write(fixture.server.root().join("late"), b"theirs")?;
    let error = session.finish().err().ok_or_else(|| io::Error::other("finish succeeded"))?;
    assert_eq!(error.kind, BackendErrorKind::AlreadyExists);
    assert_eq!(fs::read(fixture.server.root().join("late"))?, b"theirs");
    assert!(fixture.server.temporaries().is_empty());
    Ok(())
}

#[test]
fn the_suite_passes_on_a_lenient_server() -> io::Result<()> {
    let fixture = Fixture::new(lenient(), &[])?;
    fs::create_dir(fixture.server.root().join("scratch"))?;
    let report = conformance::run(fixture.backend.as_ref(), &rp("/scratch")?)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let failures: Vec<String> = report
        .failures()
        .iter()
        .map(|case| format!("{}: {:?}", case.id, case.outcome))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}
