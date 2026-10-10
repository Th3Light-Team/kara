//! The SFTP adapter through the registry and kara-ops: lost and reconnected
//! drives, uploads and downloads, cancel and a connection lost mid-copy.

mod support;

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use kara_ops::runner::{Answer, Event, Handle, Op, Outcome, spawn_with};
use kara_ops::{ErrorDecision, FailureKind, LocationRequest};
use kara_remote::sftp::SftpFactory;
use kara_remote::{ConnectionState, DriveRegistry, MemorySecretStore, Remember, Secret};
use kara_vfs::{BackendErrorKind, Cancel, DriveId, Location, RemotePath};
use support::{PASSWORD, Scripted, ServerOptions, TestServer, drive_config, rp, trusting_known_hosts};

struct Setup {
    server: TestServer,
    _client: tempfile::TempDir,
    registry: Arc<DriveRegistry>,
    id: DriveId,
}

fn setup(options: ServerOptions, extra: &[(&str, &str)]) -> io::Result<Setup> {
    let server = TestServer::start(options)?;
    let client = tempfile::tempdir()?;
    let known = trusting_known_hosts(&server, client.path())?;
    let registry = DriveRegistry::new(Arc::new(MemorySecretStore::new()));
    registry.register_factory(SftpFactory::new());
    let config = drive_config(&server, "box", &known, extra)?;
    let id = config.id.clone();
    registry.add(config).map_err(io::Error::other)?;
    registry
        .remember_secret(&id, &Secret::new(PASSWORD))
        .map_err(io::Error::other)?;
    registry
        .connect(&id, &Scripted::default(), &Cancel::new())
        .map_err(io::Error::other)?;
    Ok(Setup {
        server,
        _client: client,
        registry,
        id,
    })
}

struct Job {
    handle: Handle,
    events: Receiver<Event>,
}

impl Job {
    fn start(request: LocationRequest, registry: &Arc<DriveRegistry>) -> io::Result<Job> {
        let (tx, events) = channel();
        let tx = Mutex::new(tx);
        let handle = spawn_with(request, registry.resolver(), move |event| {
            if let Ok(tx) = tx.lock() {
                let _ = tx.send(event);
            }
        })
        .map_err(|e| io::Error::other(format!("{e:?}")))?;
        Ok(Job { handle, events })
    }

    /// Runs to the end, answering failures with `decide`.
    fn finish(&self, mut decide: impl FnMut(&kara_ops::runner::FailurePrompt) -> ErrorDecision) -> io::Result<Box<Outcome>> {
        loop {
            match self.events.recv_timeout(Duration::from_secs(60)) {
                Ok(Event::Finished(outcome)) => return Ok(outcome),
                Ok(Event::Failure(prompt)) => self.handle.answer(Answer::Error(decide(&prompt))),
                Ok(_) => {}
                Err(_) => return Err(io::Error::other("the job did not finish")),
            }
        }
    }
}

fn remote(id: &DriveId, path: &str) -> io::Result<Location> {
    Ok(Location::Remote {
        drive: id.clone(),
        path: rp(path)?,
    })
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(131) >> 3) as u8).collect()
}

fn request(op: Op, sources: Vec<Location>, dest_dir: Location) -> LocationRequest {
    LocationRequest {
        op,
        sources,
        dest_dir,
        confirmed_permanent: false,
    }
}

fn tree_bytes(dir: &Path) -> io::Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d)?.flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(dir)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            if entry.file_type()?.is_dir() {
                stack.push(path);
                out.push((rel, Vec::new()));
            } else {
                out.push((rel, fs::read(&path)?));
            }
        }
    }
    out.sort();
    Ok(out)
}

#[test]
fn a_lost_drive_is_reported_and_reconnects_to_ready() -> io::Result<()> {
    let setup = setup(ServerOptions::default(), &[])?;
    let backend = setup.registry.backend(&setup.id).ok_or_else(|| io::Error::other("no backend"))?;
    setup.server.kill_connections();
    let started = Instant::now();
    let error = loop {
        match backend.stat(&RemotePath::root()) {
            Err(error) => break error,
            Ok(_) if started.elapsed() < Duration::from_secs(5) => thread::sleep(Duration::from_millis(20)),
            Ok(_) => return Err(io::Error::other("still answering")),
        }
    };
    assert_eq!(error.kind, BackendErrorKind::Unavailable);
    assert!(setup.registry.report_failure(&setup.id, &error));
    assert!(matches!(setup.registry.state(&setup.id), Some(ConnectionState::Lost { .. })));
    assert!((setup.registry.resolver())(&setup.id).is_none(), "a lost drive does not resolve");

    setup
        .registry
        .connect(&setup.id, &Scripted::default(), &Cancel::new())
        .map_err(io::Error::other)?;
    assert_eq!(setup.registry.state(&setup.id), Some(ConnectionState::Ready));
    let fresh = (setup.registry.resolver())(&setup.id).ok_or_else(|| io::Error::other("no backend"))?;
    fresh.stat(&RemotePath::root()).map_err(io::Error::other)?;
    Ok(())
}

#[test]
fn a_local_folder_is_copied_to_the_drive_and_back() -> io::Result<()> {
    let setup = setup(ServerOptions::default(), &[])?;
    let local = tempfile::tempdir()?;
    let src = local.path().join("project");
    fs::create_dir_all(src.join("docs/deep"))?;
    fs::write(src.join("big.bin"), pattern(3 * 1024 * 1024 + 11))?;
    fs::write(src.join("docs/a.txt"), b"alpha")?;
    fs::write(src.join("docs/deep/empty"), b"")?;

    let job = Job::start(
        request(Op::Copy, vec![Location::Local(src.clone())], remote(&setup.id, "/")?),
        &setup.registry,
    )?;
    let outcome = job.finish(|_| ErrorDecision::Cancel)?;
    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(tree_bytes(&src)?, tree_bytes(&setup.server.root().join("project"))?);
    assert!(setup.server.temporaries().is_empty());

    // And back down, into another local folder.
    let back = tempfile::tempdir()?;
    let job = Job::start(
        request(Op::Copy, vec![remote(&setup.id, "/project")?], Location::Local(back.path().to_path_buf())),
        &setup.registry,
    )?;
    let outcome = job.finish(|_| ErrorDecision::Cancel)?;
    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(tree_bytes(&src)?, tree_bytes(&back.path().join("project"))?);
    Ok(())
}

#[test]
fn a_move_to_the_drive_removes_the_source_only_after_the_copy() -> io::Result<()> {
    let setup = setup(ServerOptions::default(), &[])?;
    let local = tempfile::tempdir()?;
    let src = local.path().join("report.pdf");
    fs::write(&src, pattern(500_000))?;
    let job = Job::start(
        request(Op::Move, vec![Location::Local(src.clone())], remote(&setup.id, "/")?),
        &setup.registry,
    )?;
    let outcome = job.finish(|_| ErrorDecision::Cancel)?;
    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert!(!src.exists());
    assert_eq!(fs::read(setup.server.root().join("report.pdf"))?, pattern(500_000));
    Ok(())
}

#[test]
fn cancelling_an_upload_leaves_no_file_and_no_temporary() -> io::Result<()> {
    let setup = setup(ServerOptions::default(), &[])?;
    setup.server.faults.delay_ms.store(5, Ordering::SeqCst);
    let local = tempfile::tempdir()?;
    let src = local.path().join("slow.bin");
    fs::write(&src, pattern(8 * 1024 * 1024))?;
    let job = Job::start(
        request(Op::Copy, vec![Location::Local(src.clone())], remote(&setup.id, "/")?),
        &setup.registry,
    )?;
    let handle = job.handle.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(400));
        handle.cancel();
        Instant::now()
    });
    let outcome = job.finish(|_| ErrorDecision::Cancel)?;
    let finished = Instant::now();
    let cancelled_at = canceller.join().map_err(|_| io::Error::other("join"))?;
    assert!(outcome.cancelled || outcome.report.cancelled, "{outcome:?}");
    assert!(
        finished.saturating_duration_since(cancelled_at) < Duration::from_secs(3),
        "the job ran on for {:?} after the cancel",
        finished.saturating_duration_since(cancelled_at)
    );
    assert!(!setup.server.root().join("slow.bin").exists());
    assert!(setup.server.temporaries().is_empty(), "{:?}", setup.server.temporaries());
    assert!(src.exists());
    Ok(())
}

#[test]
fn a_connection_lost_mid_copy_is_media_gone_and_keeps_the_source_of_a_move() -> io::Result<()> {
    let setup = setup(ServerOptions::default(), &[])?;
    setup
        .server
        .faults
        .kill_after_written
        .store(512 * 1024, Ordering::SeqCst);
    let local = tempfile::tempdir()?;
    let src = local.path().join("video.mkv");
    fs::write(&src, pattern(4 * 1024 * 1024))?;
    let job = Job::start(
        request(Op::Move, vec![Location::Local(src.clone())], remote(&setup.id, "/")?),
        &setup.registry,
    )?;
    let mut kinds = Vec::new();
    let outcome = job.finish(|prompt| {
        kinds.push(prompt.kind);
        ErrorDecision::Skip
    })?;
    assert_eq!(kinds, vec![FailureKind::MediaGone]);
    assert_eq!(outcome.report.skipped.len(), 1, "{:?}", outcome.report);
    assert_eq!(fs::read(&src)?, pattern(4 * 1024 * 1024), "the source of a failed move stays");
    assert!(!setup.server.root().join("video.mkv").exists());
    Ok(())
}

#[test]
fn the_registry_keeps_the_password_for_the_group_when_asked() -> io::Result<()> {
    // A fleet sharing one password: the second drive connects without asking.
    let server = TestServer::start(ServerOptions::default())?;
    let client = tempfile::tempdir()?;
    let known = trusting_known_hosts(&server, client.path())?;
    let store = Arc::new(MemorySecretStore::new());
    let registry = DriveRegistry::new(store);
    registry.register_factory(SftpFactory::new());
    let mut ids = Vec::new();
    for name in ["ct101", "ct102"] {
        let config = drive_config(&server, name, &known, &[])?
            .with_group("proxmox")
            .map_err(io::Error::other)?;
        ids.push(config.id.clone());
        registry.add(config).map_err(io::Error::other)?;
    }
    let first = Scripted::new([kara_remote::PromptAnswer::Secret {
        secret: Secret::new(PASSWORD),
        remember: Remember::ForGroup,
    }]);
    registry.connect(&ids[0], &first, &Cancel::new()).map_err(io::Error::other)?;
    let second = Scripted::default();
    registry.connect(&ids[1], &second, &Cancel::new()).map_err(io::Error::other)?;
    assert!(second.asked().is_empty(), "{:?}", second.asked());
    assert_eq!(registry.state(&ids[1]), Some(ConnectionState::Ready));
    Ok(())
}
