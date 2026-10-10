//! The registry's resolver drives a real kara-ops job.

use std::fs;
use std::sync::mpsc::channel;
use std::time::Duration;

use kara_ops::runner::{Event, Op, spawn_with};
use kara_ops::{LocationRequest, RequestError};
use kara_remote::memory::MemoryFactory;
use kara_remote::{ConnectionState, DriveConfig, DriveRegistry, MemorySecretStore, RefuseAll};
use kara_vfs::{BackendError, BackendErrorKind, Cancel, DriveId, Location, RemotePath};
use std::sync::Arc;

fn setup() -> (Arc<DriveRegistry>, DriveId) {
    let registry = DriveRegistry::new(Arc::new(MemorySecretStore::new()));
    registry.register_factory(MemoryFactory::new());
    let config = DriveConfig::new("mem", "nas", "NAS", []).expect("config");
    let id = config.id.clone();
    registry.add(config).expect("add");
    (registry, id)
}

fn run(request: LocationRequest, registry: &Arc<DriveRegistry>) -> Result<(), RequestError> {
    let (tx, rx) = channel();
    let tx = std::sync::Mutex::new(tx);
    spawn_with(request, registry.resolver(), move |event| {
        if matches!(event, Event::Finished(_)) {
            let _ = tx.lock().expect("lock").send(());
        }
    })?;
    rx.recv_timeout(Duration::from_secs(20)).expect("the job finishes");
    Ok(())
}

#[test]
fn copying_a_local_file_to_a_registered_drive() {
    let (registry, id) = setup();
    registry.connect(&id, &RefuseAll, &Cancel::new()).expect("connect");
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("report.txt");
    fs::write(&source, b"quarterly numbers").expect("write");

    let request = LocationRequest {
        op: Op::Copy,
        sources: vec![Location::Local(source)],
        dest_dir: Location::Remote { drive: id.clone(), path: RemotePath::root() },
        confirmed_permanent: false,
    };
    run(request, &registry).expect("accepted");

    let backend = registry.backend(&id).expect("backend");
    let path = RemotePath::parse("/report.txt").expect("path");
    assert_eq!(backend.stat(&path).expect("stat").size, Some(17));
}

#[test]
fn a_lost_drive_is_refused_before_anything_is_touched() {
    let (registry, id) = setup();
    registry.connect(&id, &RefuseAll, &Cancel::new()).expect("connect");
    registry.report_failure(&id, &BackendError::new(BackendErrorKind::Unavailable, None));
    assert!(matches!(registry.state(&id), Some(ConnectionState::Lost { .. })));

    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("keep.txt");
    fs::write(&source, b"x").expect("write");
    let request = LocationRequest {
        op: Op::Move,
        sources: vec![Location::Local(source.clone())],
        dest_dir: Location::Remote { drive: id, path: RemotePath::root() },
        confirmed_permanent: false,
    };
    let outcome = run(request, &registry);

    assert!(outcome.is_err(), "a lost drive must refuse the job");
    assert!(source.exists(), "and the local file must still be there");
}
