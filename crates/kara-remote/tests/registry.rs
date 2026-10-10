//! DriveRegistry: lifecycle, prompts, resolver, lost drives.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use kara_remote::memory::MemoryFactory;
use kara_remote::{
    BackendFactory, ConnectError, ConnectOrRegistryError, ConnectionState, DriveConfig,
    DriveRegistry, MemorySecretStore, Prompt, PromptAnswer, PromptHandler, RefuseAll,
    RegistryError, Remember, Secret, SecretKey, SecretStore,
};
use kara_vfs::{Backend, BackendError, BackendErrorKind, Cancel, DriveId};

fn id(name: &str) -> DriveId {
    DriveId::new("mem", name).expect("valid id")
}

fn drive(name: &str, params: &[(&str, &str)]) -> DriveConfig {
    DriveConfig::new(
        "mem",
        name,
        name,
        params.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    )
    .expect("valid config")
}

fn registry() -> (Arc<DriveRegistry>, Arc<MemoryFactory>, Arc<MemorySecretStore>) {
    let secrets = Arc::new(MemorySecretStore::new());
    let registry = DriveRegistry::new(secrets.clone());
    let factory = MemoryFactory::new();
    registry.register_factory(factory.clone());
    (registry, factory, secrets)
}

/// Answers with a fixed list of secrets and counts the questions.
struct Scripted {
    secrets: Mutex<Vec<&'static str>>,
    remember: Remember,
    asked: AtomicUsize,
}

impl Scripted {
    fn new(secrets: &[&'static str], remember: Remember) -> Scripted {
        Scripted {
            secrets: Mutex::new(secrets.iter().rev().copied().collect()),
            remember,
            asked: AtomicUsize::new(0),
        }
    }
}

impl PromptHandler for Scripted {
    fn ask(&self, prompt: &Prompt) -> PromptAnswer {
        self.asked.fetch_add(1, Ordering::SeqCst);
        assert!(matches!(prompt, Prompt::Password { .. }), "{prompt:?}");
        match self.secrets.lock().expect("lock").pop() {
            Some(text) => PromptAnswer::Secret {
                secret: Secret::new(text),
                remember: self.remember,
            },
            None => PromptAnswer::Refuse,
        }
    }
}

#[test]
fn an_unknown_scheme_is_refused() {
    let (registry, _, _) = registry();
    let config = DriveConfig::new("sftp", "a", "A", []).expect("config");
    assert_eq!(
        registry.add(config),
        Err(RegistryError::UnsupportedScheme("sftp".into()))
    );
}

#[test]
fn a_drive_cannot_be_added_twice() {
    let (registry, _, _) = registry();
    registry.add(drive("a", &[])).expect("first");
    assert_eq!(registry.add(drive("a", &[])), Err(RegistryError::AlreadyExists));
    assert_eq!(registry.configs().len(), 1);
}

#[test]
fn connecting_makes_the_drive_resolvable_and_disconnecting_undoes_it() {
    let (registry, _, _) = registry();
    registry.add(drive("a", &[])).expect("add");
    let resolver = registry.resolver();
    assert_eq!(registry.state(&id("a")), Some(ConnectionState::Disconnected));
    assert!(resolver(&id("a")).is_none(), "not connected yet");

    registry.connect(&id("a"), &RefuseAll, &Cancel::new()).expect("connect");
    assert_eq!(registry.state(&id("a")), Some(ConnectionState::Ready));
    assert!(resolver(&id("a")).is_some());

    registry.disconnect(&id("a")).expect("disconnect");
    assert_eq!(registry.state(&id("a")), Some(ConnectionState::Disconnected));
    assert!(resolver(&id("a")).is_none(), "a disconnected drive must not resolve");
}

#[test]
fn data_survives_a_reconnect() {
    let (registry, factory, _) = registry();
    registry.add(drive("a", &[])).expect("add");
    registry.connect(&id("a"), &RefuseAll, &Cancel::new()).expect("connect");
    let backend = registry.backend(&id("a")).expect("backend");
    let path = kara_vfs::RemotePath::parse("/hello").expect("path");
    let mut session = backend.begin_write(&path, None, false).expect("begin");
    std::io::Write::write_all(&mut session, b"hi").expect("write");
    session.finish().expect("finish");

    registry.disconnect(&id("a")).expect("disconnect");
    registry.connect(&id("a"), &RefuseAll, &Cancel::new()).expect("reconnect");
    assert!(registry.backend(&id("a")).expect("backend").stat(&path).is_ok());
    assert!(factory.backend_of("a").is_some());
}

#[test]
fn a_secret_is_asked_for_and_remembered_after_it_worked() {
    let (registry, _, secrets) = registry();
    registry.add(drive("vault", &[("auth", "required")])).expect("add");
    let handler = Scripted::new(&[MemoryFactory::ACCEPTED], Remember::ForDrive);

    registry.connect(&id("vault"), &handler, &Cancel::new()).expect("connect");

    assert_eq!(handler.asked.load(Ordering::SeqCst), 1);
    assert_eq!(
        secrets.get(&SecretKey::Drive(id("vault"))).expect("get").map(|s| s.expose().to_owned()),
        Some(MemoryFactory::ACCEPTED.to_owned())
    );

    // The next connection uses the stored secret and does not ask.
    registry.disconnect(&id("vault")).expect("disconnect");
    let silent = Scripted::new(&[], Remember::No);
    registry.connect(&id("vault"), &silent, &Cancel::new()).expect("reconnect");
    assert_eq!(silent.asked.load(Ordering::SeqCst), 0);
}

#[test]
fn a_wrong_secret_is_not_remembered() {
    let (registry, _, secrets) = registry();
    registry.add(drive("vault", &[("auth", "required")])).expect("add");
    let handler = Scripted::new(&["wrong", "also wrong", "still wrong"], Remember::ForDrive);

    let error = registry
        .connect(&id("vault"), &handler, &Cancel::new())
        .expect_err("must fail");

    assert_eq!(error, ConnectOrRegistryError::Connect(ConnectError::AuthFailed));
    assert!(secrets.get(&SecretKey::Drive(id("vault"))).expect("get").is_none(), "nothing wrong may be kept");
    assert!(matches!(registry.state(&id("vault")), Some(ConnectionState::Failed { .. })));
    assert!(registry.backend(&id("vault")).is_none());
}

#[test]
fn a_secret_is_not_kept_unless_the_user_asked() {
    let (registry, _, secrets) = registry();
    registry.add(drive("vault", &[("auth", "required")])).expect("add");
    let handler = Scripted::new(&[MemoryFactory::ACCEPTED], Remember::No);
    registry.connect(&id("vault"), &handler, &Cancel::new()).expect("connect");
    assert!(secrets.get(&SecretKey::Drive(id("vault"))).expect("get").is_none());
}

#[test]
fn refusing_the_prompt_fails_with_auth_required() {
    let (registry, _, _) = registry();
    registry.add(drive("vault", &[("auth", "required")])).expect("add");
    let error = registry
        .connect(&id("vault"), &RefuseAll, &Cancel::new())
        .expect_err("must fail");
    assert_eq!(error, ConnectOrRegistryError::Connect(ConnectError::AuthRequired));
}

#[test]
fn an_unreachable_host_is_a_failure_with_a_reason() {
    let (registry, _, _) = registry();
    registry.add(drive("far", &[("unreachable", "true")])).expect("add");
    let error = registry.connect(&id("far"), &RefuseAll, &Cancel::new()).expect_err("fails");
    assert!(matches!(error, ConnectOrRegistryError::Connect(ConnectError::Unreachable(_))));
    match registry.state(&id("far")) {
        Some(ConnectionState::Failed { reason }) => assert!(reason.contains("refused"), "{reason}"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn a_cancelled_connection_does_not_touch_the_factory() {
    let (registry, factory, _) = registry();
    registry.add(drive("a", &[])).expect("add");
    let cancel = Cancel::new();
    cancel.cancel();
    let error = registry.connect(&id("a"), &RefuseAll, &cancel).expect_err("cancelled");
    assert_eq!(error, ConnectOrRegistryError::Connect(ConnectError::Cancelled));
    assert!(factory.backend_of("a").is_none());
}

#[test]
fn listeners_see_every_transition_in_order() {
    let (registry, _, _) = registry();
    registry.add(drive("a", &[])).expect("add");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    registry.subscribe(move |drive, state| {
        sink.lock().expect("lock").push((drive.name().to_owned(), state.clone()));
    });
    registry.connect(&id("a"), &RefuseAll, &Cancel::new()).expect("connect");
    registry.disconnect(&id("a")).expect("disconnect");
    let seen = seen.lock().expect("lock").clone();
    let states: Vec<_> = seen.iter().map(|(_, s)| s.clone()).collect();
    assert_eq!(
        states,
        vec![ConnectionState::Connecting, ConnectionState::Ready, ConnectionState::Disconnected]
    );
}

#[test]
fn a_lost_connection_stops_the_drive_resolving() {
    let (registry, _, _) = registry();
    registry.add(drive("a", &[])).expect("add");
    registry.connect(&id("a"), &RefuseAll, &Cancel::new()).expect("connect");
    let resolver = registry.resolver();

    let other = BackendError::new(BackendErrorKind::PermissionDenied, None);
    assert!(!registry.report_failure(&id("a"), &other), "only a lost connection counts");
    assert!(resolver(&id("a")).is_some());

    let lost = BackendError::new(BackendErrorKind::Unavailable, None);
    assert!(registry.report_failure(&id("a"), &lost));
    assert!(matches!(registry.state(&id("a")), Some(ConnectionState::Lost { .. })));
    assert!(resolver(&id("a")).is_none(), "ops must fail fast, not hang on a dead session");
    assert!(!registry.report_failure(&id("a"), &lost), "already lost");

    registry.connect(&id("a"), &RefuseAll, &Cancel::new()).expect("reconnect");
    assert!(resolver(&id("a")).is_some());
}

#[test]
fn removing_a_drive_deletes_its_secret() {
    let (registry, _, secrets) = registry();
    registry.add(drive("a", &[])).expect("add");
    registry.remember_secret(&id("a"), &Secret::new("pw")).expect("remember");
    registry.remove(&id("a")).expect("remove");
    assert!(secrets.get(&SecretKey::Drive(id("a"))).expect("get").is_none());
    assert!(registry.config(&id("a")).is_none());
    assert_eq!(registry.remove(&id("a")), Err(RegistryError::Unknown));
}

#[test]
fn unknown_drives_are_reported() {
    let (registry, _, _) = registry();
    assert_eq!(
        registry.connect(&id("nope"), &RefuseAll, &Cancel::new()),
        Err(ConnectOrRegistryError::Registry(RegistryError::Unknown))
    );
    assert_eq!(registry.disconnect(&id("nope")), Err(RegistryError::Unknown));
    assert!(registry.state(&id("nope")).is_none());
}

/// A factory that waits for a signal, to hold a connection open.
struct Gate {
    entered: Mutex<std::sync::mpsc::Sender<()>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    inner: Arc<MemoryFactory>,
}

impl BackendFactory for Gate {
    fn scheme(&self) -> &str {
        "mem"
    }
    fn connect(
        &self,
        config: &DriveConfig,
        secret: Option<&Secret>,
        prompts: &dyn PromptHandler,
        cancel: &Cancel,
    ) -> Result<Arc<dyn Backend>, ConnectError> {
        self.entered.lock().expect("lock").send(()).expect("send");
        self.release.lock().expect("lock").recv_timeout(Duration::from_secs(10)).expect("release");
        self.inner.connect(config, secret, prompts, cancel)
    }
}

#[test]
fn a_second_connect_while_connecting_is_refused() {
    let (entered_tx, entered_rx) = channel();
    let (release_tx, release_rx) = channel();
    let registry = DriveRegistry::new(Arc::new(MemorySecretStore::new()));
    registry.register_factory(Arc::new(Gate {
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
        inner: MemoryFactory::new(),
    }));
    registry.add(drive("a", &[])).expect("add");

    let worker = {
        let registry = Arc::clone(&registry);
        thread::spawn(move || registry.connect(&id("a"), &RefuseAll, &Cancel::new()))
    };
    entered_rx.recv_timeout(Duration::from_secs(10)).expect("worker is connecting");

    assert_eq!(registry.state(&id("a")), Some(ConnectionState::Connecting));
    assert_eq!(
        registry.connect(&id("a"), &RefuseAll, &Cancel::new()),
        Err(ConnectOrRegistryError::Registry(RegistryError::Busy))
    );
    assert!(registry.backend(&id("a")).is_none(), "a connecting drive does not resolve");

    release_tx.send(()).expect("release");
    worker.join().expect("join").expect("connected");
    assert_eq!(registry.state(&id("a")), Some(ConnectionState::Ready));
}

#[test]
fn secrets_never_show_in_debug_output() {
    let secret = Secret::new("hunter2");
    assert!(!format!("{secret:?}").contains("hunter2"));
    let prompt = PromptAnswer::Secret { secret, remember: Remember::ForDrive };
    assert!(!format!("{prompt:?}").contains("hunter2"));
}
