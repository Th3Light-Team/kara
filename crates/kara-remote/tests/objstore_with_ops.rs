//! Object-store drives through the registry and kara-ops: lost and
//! reconnected drives, uploads and downloads, copies inside the drive done by
//! the service (no byte through the client), a cancelled upload and a
//! connection lost mid-move.

mod objstore_support;

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use kara_ops::runner::{Answer, Event, FailurePrompt, Handle, Op as JobOp, Outcome, spawn_with};
use kara_ops::{ErrorDecision, FailureKind, LocationRequest};
use kara_remote::{ConnectionState, DriveRegistry, MemorySecretStore, Secret};
use kara_vfs::{BackendErrorKind, Cancel, DriveId, Location, RemotePath};
use objstore_support::{Effect, Fault, Faulty, Op, RIGHT_SECRET, Scripted, fake_s3_factory, rp, s3_config};

struct Setup {
    service: Arc<Faulty>,
    registry: Arc<DriveRegistry>,
    id: DriveId,
}

fn setup(extra: &[(&str, &str)]) -> io::Result<Setup> {
    let service = Faulty::new();
    let registry = DriveRegistry::new(Arc::new(MemorySecretStore::new()));
    registry.register_factory(fake_s3_factory(&service));
    let mut params = vec![("bucket", "b"), ("access_key_id", "AKIA1")];
    params.extend_from_slice(extra);
    let config = s3_config("cloud", &params)?;
    let id = config.id.clone();
    registry.add(config).map_err(io::Error::other)?;
    registry
        .remember_secret(&id, &Secret::new(RIGHT_SECRET))
        .map_err(io::Error::other)?;
    registry
        .connect(&id, &Scripted::default(), &Cancel::new())
        .map_err(io::Error::other)?;
    Ok(Setup { service, registry, id })
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

    fn finish(&self, mut decide: impl FnMut(&FailurePrompt) -> ErrorDecision) -> io::Result<Box<Outcome>> {
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

fn request(op: JobOp, sources: Vec<Location>, dest_dir: Location) -> LocationRequest {
    LocationRequest {
        op,
        sources,
        dest_dir,
        confirmed_permanent: false,
    }
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(131) >> 3) as u8).collect()
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
fn a_lost_drive_is_reported_and_reconnects_to_ready_with_its_data() -> io::Result<()> {
    let setup = setup(&[])?;
    let backend = setup.registry.backend(&setup.id).ok_or_else(|| io::Error::other("no backend"))?;
    let path = rp("/kept.txt")?;
    let mut session = backend.begin_write(&path, None, false).map_err(io::Error::other)?;
    io::Write::write_all(&mut session, b"still here")?;
    session.finish().map_err(io::Error::other)?;

    setup.service.unplugged.store(true, Ordering::SeqCst);
    let error = backend.stat(&path).err().ok_or_else(|| io::Error::other("still answering"))?;
    assert_eq!(error.kind, BackendErrorKind::Unavailable);
    assert!(setup.registry.report_failure(&setup.id, &error));
    assert!(matches!(setup.registry.state(&setup.id), Some(ConnectionState::Lost { .. })));
    assert!((setup.registry.resolver())(&setup.id).is_none(), "a lost drive does not resolve");

    // While the service is down, reconnecting fails at connect time.
    let down = setup.registry.connect(&setup.id, &Scripted::default(), &Cancel::new());
    assert!(down.is_err());
    assert!(matches!(setup.registry.state(&setup.id), Some(ConnectionState::Failed { .. })));

    setup.service.unplugged.store(false, Ordering::SeqCst);
    setup
        .registry
        .connect(&setup.id, &Scripted::default(), &Cancel::new())
        .map_err(io::Error::other)?;
    assert_eq!(setup.registry.state(&setup.id), Some(ConnectionState::Ready));
    let fresh = (setup.registry.resolver())(&setup.id).ok_or_else(|| io::Error::other("no backend"))?;
    assert_eq!(fresh.stat(&path).map_err(io::Error::other)?.size, Some(10));
    Ok(())
}

#[test]
fn a_local_folder_is_uploaded_and_downloaded_unchanged() -> io::Result<()> {
    let setup = setup(&[("part_size_mb", "5")])?;
    let local = tempfile::tempdir()?;
    let src = local.path().join("project");
    fs::create_dir_all(src.join("docs/deep"))?;
    fs::create_dir_all(src.join("empty"))?;
    fs::write(src.join("big.bin"), pattern(11 * 1024 * 1024 + 7))?;
    fs::write(src.join("docs/a.txt"), b"alpha")?;
    fs::write(src.join("docs/deep/zero"), b"")?;

    let job = Job::start(
        request(JobOp::Copy, vec![Location::Local(src.clone())], remote(&setup.id, "/")?),
        &setup.registry,
    )?;
    let outcome = job.finish(|_| ErrorDecision::Cancel)?;
    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert!(setup.service.counts.get(Op::Complete) >= 1, "the big file went up in parts");
    assert_eq!(setup.service.open_uploads(), 0);
    assert_eq!(setup.service.bytes("project/big.bin"), Some(pattern(11 * 1024 * 1024 + 7)));

    let back = tempfile::tempdir()?;
    let job = Job::start(
        request(
            JobOp::Copy,
            vec![remote(&setup.id, "/project")?],
            Location::Local(back.path().to_path_buf()),
        ),
        &setup.registry,
    )?;
    let outcome = job.finish(|_| ErrorDecision::Cancel)?;
    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(tree_bytes(&src)?, tree_bytes(&back.path().join("project"))?);
    Ok(())
}

#[test]
fn copies_and_moves_inside_the_drive_are_done_by_the_service() -> io::Result<()> {
    let setup = setup(&[])?;
    let backend = setup.registry.backend(&setup.id).ok_or_else(|| io::Error::other("no backend"))?;
    backend.create_dir(&rp("/archive")?).map_err(io::Error::other)?;
    for (name, size) in [("/photos/a.jpg", 70_000usize), ("/photos/b.jpg", 1), ("/photos/raw/c.cr2", 300_000)] {
        let mut session = backend.begin_write(&rp(name)?, None, false).map_err(io::Error::other)?;
        io::Write::write_all(&mut session, &pattern(size))?;
        session.finish().map_err(io::Error::other)?;
    }
    let gets = setup.service.counts.get(Op::Get);
    let job = Job::start(
        request(JobOp::Copy, vec![remote(&setup.id, "/photos")?], remote(&setup.id, "/archive")?),
        &setup.registry,
    )?;
    let outcome = job.finish(|_| ErrorDecision::Cancel)?;
    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(setup.service.counts.get(Op::Get), gets, "no byte was downloaded");
    assert!(setup.service.counts.get(Op::Copy) >= 3);
    assert_eq!(setup.service.bytes("archive/photos/raw/c.cr2"), Some(pattern(300_000)));

    // A move: copy, verify, delete; still no download.
    let job = Job::start(
        request(JobOp::Move, vec![remote(&setup.id, "/photos/a.jpg")?], remote(&setup.id, "/archive")?),
        &setup.registry,
    )?;
    let outcome = job.finish(|_| ErrorDecision::Cancel)?;
    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert_eq!(setup.service.counts.get(Op::Get), gets);
    assert!(setup.service.bytes("photos/a.jpg").is_none());
    assert_eq!(setup.service.bytes("archive/a.jpg"), Some(pattern(70_000)));
    Ok(())
}

#[test]
fn cancelling_an_upload_leaves_no_object_and_no_open_upload() -> io::Result<()> {
    let setup = setup(&[("part_size_mb", "5"), ("upload_concurrency", "1")])?;
    setup
        .service
        .inject(Fault::on(Op::Part, Effect::Stall(Duration::from_millis(250))).always());
    let local = tempfile::tempdir()?;
    let src = local.path().join("slow.bin");
    fs::write(&src, pattern(40 * 1024 * 1024))?;
    let job = Job::start(
        request(JobOp::Copy, vec![Location::Local(src.clone())], remote(&setup.id, "/")?),
        &setup.registry,
    )?;
    let handle = job.handle.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(600));
        handle.cancel();
        Instant::now()
    });
    let outcome = job.finish(|_| ErrorDecision::Cancel)?;
    let finished = Instant::now();
    let cancelled_at = canceller.join().map_err(|_| io::Error::other("join"))?;
    assert!(outcome.cancelled || outcome.report.cancelled, "{outcome:?}");
    assert!(
        finished.saturating_duration_since(cancelled_at) < Duration::from_secs(3),
        "the job ran on for {:?}",
        finished.saturating_duration_since(cancelled_at)
    );
    assert!(setup.service.bytes("slow.bin").is_none());
    assert_eq!(setup.service.open_uploads(), 0, "the multipart upload was aborted");
    assert!(src.exists());
    Ok(())
}

#[test]
fn a_connection_lost_mid_move_is_media_gone_and_keeps_the_source() -> io::Result<()> {
    let setup = setup(&[("part_size_mb", "5")])?;
    setup.service.inject(Fault::on(Op::Part, Effect::Refused).after(1).always());
    let local = tempfile::tempdir()?;
    let src = local.path().join("video.mkv");
    fs::write(&src, pattern(16 * 1024 * 1024))?;
    let job = Job::start(
        request(JobOp::Move, vec![Location::Local(src.clone())], remote(&setup.id, "/")?),
        &setup.registry,
    )?;
    let mut kinds = Vec::new();
    let outcome = job.finish(|prompt| {
        kinds.push(prompt.kind);
        ErrorDecision::Skip
    })?;
    assert_eq!(kinds, vec![FailureKind::MediaGone]);
    assert_eq!(outcome.report.skipped.len(), 1, "{:?}", outcome.report);
    assert_eq!(fs::read(&src)?, pattern(16 * 1024 * 1024), "the source of a failed move stays");
    assert!(setup.service.bytes("video.mkv").is_none());
    assert_eq!(setup.service.open_uploads(), 0);
    Ok(())
}

#[test]
fn a_remote_folder_is_deleted_permanently_only_when_confirmed() -> io::Result<()> {
    let setup = setup(&[])?;
    let backend = setup.registry.backend(&setup.id).ok_or_else(|| io::Error::other("no backend"))?;
    backend.create_dir(&rp("/old")?).map_err(io::Error::other)?;
    backend.create_dir(&rp("/old/sub")?).map_err(io::Error::other)?;
    let mut session = backend.begin_write(&rp("/old/sub/f")?, None, false).map_err(io::Error::other)?;
    io::Write::write_all(&mut session, b"f")?;
    session.finish().map_err(io::Error::other)?;

    let unconfirmed = spawn_with(
        request(JobOp::Delete, vec![remote(&setup.id, "/old")?], remote(&setup.id, "/")?),
        setup.registry.resolver(),
        |_| {},
    );
    assert!(unconfirmed.is_err(), "a drive without trash needs a confirmed delete");
    let mut confirmed = request(JobOp::Delete, vec![remote(&setup.id, "/old")?], remote(&setup.id, "/")?);
    confirmed.confirmed_permanent = true;
    let job = Job::start(confirmed, &setup.registry)?;
    let outcome = job.finish(|_| ErrorDecision::Cancel)?;
    assert!(outcome.report.is_clean(), "{:?}", outcome.report);
    assert!(setup.service.keys().is_empty(), "{:?}", setup.service.keys());
    let gone = backend.stat(&RemotePath::root().join("old").map_err(io::Error::other)?);
    assert_eq!(gone.err().map(|e| e.kind), Some(BackendErrorKind::NotFound));
    Ok(())
}
