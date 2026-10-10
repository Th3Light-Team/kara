//! SFTP authentication: password, key file, encrypted key, agent, and the
//! registry asking for the secret. Also: unreachable hosts, connect timeout
//! and cancel, and that no secret ever shows in an error or a `Debug`.

mod support;

use std::io;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use kara_remote::sftp::SftpFactory;
use kara_remote::{
    ConnectError, ConnectOrRegistryError, ConnectionState, DriveRegistry, MemorySecretStore,
    PromptAnswer, Remember, Secret, SecretKey, SecretStore,
};
use kara_vfs::{Backend, Cancel};
use support::{
    ENCRYPTED_KEY, ENCRYPTED_KEY_PASSPHRASE, PASSWORD, Scripted, ServerOptions, TestServer,
    drive_config, encrypted_key_public, key_from_seed, rp, trusting_known_hosts, write_key_file,
};

struct Setup {
    server: TestServer,
    client: tempfile::TempDir,
    known_hosts: std::path::PathBuf,
}

fn setup(options: ServerOptions) -> io::Result<Setup> {
    let server = TestServer::start(options)?;
    let client = tempfile::tempdir()?;
    let known_hosts = trusting_known_hosts(&server, client.path())?;
    Ok(Setup {
        server,
        client,
        known_hosts,
    })
}

fn open(
    setup: &Setup,
    extra: &[(&str, &str)],
    secret: Option<&str>,
) -> Result<Arc<support::SftpBackend>, ConnectError> {
    let config = drive_config(&setup.server, "auth", &setup.known_hosts, extra)
        .map_err(|e| ConnectError::Other(e.to_string()))?;
    let secret = secret.map(Secret::new);
    SftpFactory::new().open(&config, secret.as_ref(), &Scripted::default(), &Cancel::new())
}

fn works(backend: &support::SftpBackend) -> io::Result<()> {
    let root = backend.stat(&rp("/")?).map_err(io::Error::other)?;
    assert_eq!(root.display, "/");
    Ok(())
}

#[test]
fn the_right_password_connects() -> io::Result<()> {
    let setup = setup(ServerOptions::default())?;
    let backend = open(&setup, &[], Some(PASSWORD)).map_err(io::Error::other)?;
    works(&backend)
}

#[test]
fn a_wrong_password_is_auth_failed_and_none_is_auth_required() -> io::Result<()> {
    let setup = setup(ServerOptions::default())?;
    assert_eq!(open(&setup, &[], Some("nope")).err(), Some(ConnectError::AuthFailed));
    assert_eq!(open(&setup, &[], None).err(), Some(ConnectError::AuthRequired));
    Ok(())
}

#[test]
fn a_key_file_connects_without_a_secret() -> io::Result<()> {
    let key = key_from_seed(7);
    let setup = setup(ServerOptions {
        keys: vec![key.public_key().clone()],
        password: None,
        ..ServerOptions::default()
    })?;
    let path = write_key_file(setup.client.path(), "id_ed25519", &key)?;
    let backend = open(&setup, &[("key_file", &path.to_string_lossy())], None).map_err(io::Error::other)?;
    works(&backend)
}

#[test]
fn a_key_the_server_does_not_know_is_auth_failed_with_no_password_fallback() -> io::Result<()> {
    let setup = setup(ServerOptions {
        keys: vec![key_from_seed(8).public_key().clone()],
        ..ServerOptions::default()
    })?;
    let path = write_key_file(setup.client.path(), "id_other", &key_from_seed(9))?;
    // The secret is a passphrase here, never tried as the password.
    let outcome = open(&setup, &[("key_file", &path.to_string_lossy())], Some(PASSWORD));
    assert_eq!(outcome.err(), Some(ConnectError::AuthFailed));
    Ok(())
}

#[test]
fn an_encrypted_key_needs_its_passphrase() -> io::Result<()> {
    let setup = setup(ServerOptions {
        keys: vec![encrypted_key_public()?],
        password: None,
        ..ServerOptions::default()
    })?;
    let path = setup.client.path().join("id_encrypted");
    std::fs::write(&path, ENCRYPTED_KEY)?;
    let key_file = path.to_string_lossy().into_owned();
    let extra = [("key_file", key_file.as_str())];

    assert_eq!(open(&setup, &extra, None).err(), Some(ConnectError::AuthRequired));
    assert_eq!(
        open(&setup, &extra, Some("not the passphrase")).err(),
        Some(ConnectError::AuthFailed)
    );
    let backend = open(&setup, &extra, Some(ENCRYPTED_KEY_PASSPHRASE)).map_err(io::Error::other)?;
    works(&backend)
}

#[test]
fn key_file_with_tilde_is_expanded_at_connect_time() -> io::Result<()> {
    // `~/` is expanded against $HOME; a missing file says which path it tried.
    let setup = setup(ServerOptions::default())?;
    let outcome = open(&setup, &[("key_file", "~/.kara-test-no-such-key")], Some(PASSWORD));
    match outcome {
        Err(ConnectError::Other(reason)) => {
            assert!(!reason.contains('~'), "not expanded: {reason}");
            assert!(reason.contains(".kara-test-no-such-key"), "{reason}");
        }
        other => panic!("expected Other, got {:?}", other.map(|_| ())),
    }
    Ok(())
}

#[test]
fn an_absent_agent_falls_through_to_the_password() -> io::Result<()> {
    let setup = setup(ServerOptions::default())?;
    let missing = setup.client.path().join("no-agent.sock");
    let backend = open(&setup, &[("agent", &missing.to_string_lossy())], Some(PASSWORD))
        .map_err(io::Error::other)?;
    works(&backend)
}

#[test]
fn an_agent_key_connects_without_a_secret() -> io::Result<()> {
    let key = key_from_seed(21);
    let setup = setup(ServerOptions {
        keys: vec![key.public_key().clone()],
        password: None,
        ..ServerOptions::default()
    })?;
    let runtime = tokio::runtime::Runtime::new()?;
    let socket = support::start_agent(&runtime, setup.client.path(), &[key])?;
    let backend = open(&setup, &[("agent", &socket.to_string_lossy())], None).map_err(io::Error::other)?;
    works(&backend)
}

#[test]
fn with_a_key_file_the_agent_only_offers_that_key() -> io::Result<()> {
    let in_agent = key_from_seed(22);
    let setup = setup(ServerOptions {
        keys: vec![in_agent.public_key().clone()],
        password: None,
        ..ServerOptions::default()
    })?;
    let runtime = tokio::runtime::Runtime::new()?;
    let socket = support::start_agent(&runtime, setup.client.path(), &[in_agent])?;
    let other = write_key_file(setup.client.path(), "id_other", &key_from_seed(23))?;
    let outcome = open(
        &setup,
        &[
            ("agent", &socket.to_string_lossy()),
            ("key_file", &other.to_string_lossy()),
        ],
        None,
    );
    assert_eq!(outcome.err(), Some(ConnectError::AuthFailed));
    Ok(())
}

#[test]
fn the_registry_asks_for_the_password_and_keeps_it_only_if_asked() -> io::Result<()> {
    let setup = setup(ServerOptions::default())?;
    let store = Arc::new(MemorySecretStore::new());
    let registry = DriveRegistry::new(store.clone());
    registry.register_factory(SftpFactory::new());
    let config = drive_config(&setup.server, "nas", &setup.known_hosts, &[])?;
    let id = config.id.clone();
    registry.add(config).map_err(io::Error::other)?;

    let prompts = Scripted::new([
        PromptAnswer::Secret {
            secret: Secret::new("wrong"),
            remember: Remember::ForDrive,
        },
        PromptAnswer::Secret {
            secret: Secret::new(PASSWORD),
            remember: Remember::ForDrive,
        },
    ]);
    registry
        .connect(&id, &prompts, &Cancel::new())
        .map_err(io::Error::other)?;
    assert_eq!(registry.state(&id), Some(ConnectionState::Ready));
    assert_eq!(prompts.asked().len(), 2, "asked once, then again after the wrong one");
    let kept = store.get(&SecretKey::Drive(id.clone())).map_err(io::Error::other)?;
    assert_eq!(kept, Some(Secret::new(PASSWORD)), "only the secret that worked is kept");

    let backend = registry.backend(&id).ok_or_else(|| io::Error::other("no backend"))?;
    assert!(backend.list(&rp("/")?, &Cancel::new()).is_ok());
    Ok(())
}

#[test]
fn a_closed_port_is_unreachable() -> io::Result<()> {
    let setup = setup(ServerOptions::default())?;
    // Bind and drop: the port is free and nothing listens on it.
    let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let port = port.to_string();
    match open(&setup, &[("port", &port)], Some(PASSWORD)) {
        Err(ConnectError::Unreachable(_)) => Ok(()),
        other => panic!("expected Unreachable, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn a_silent_server_times_out_instead_of_hanging() -> io::Result<()> {
    let setup = setup(ServerOptions::default())?;
    setup.server.freeze();
    let started = Instant::now();
    let outcome = open(&setup, &[("timeout_s", "1")], Some(PASSWORD));
    assert!(matches!(outcome, Err(ConnectError::Unreachable(_))), "{:?}", outcome.map(|_| ()));
    assert!(started.elapsed() < Duration::from_secs(6), "took {:?}", started.elapsed());
    Ok(())
}

#[test]
fn cancel_stops_a_connect_promptly() -> io::Result<()> {
    let setup = setup(ServerOptions::default())?;
    setup.server.freeze();
    let config = drive_config(&setup.server, "auth", &setup.known_hosts, &[("timeout_s", "60")])?;
    let cancel = Cancel::new();
    let trigger = cancel.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        trigger.cancel();
    });
    let started = Instant::now();
    let secret = Secret::new(PASSWORD);
    let outcome = SftpFactory::new().open(&config, Some(&secret), &Scripted::default(), &cancel);
    let _ = canceller.join();
    assert_eq!(outcome.err(), Some(ConnectError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(3), "took {:?}", started.elapsed());
    Ok(())
}

#[test]
fn a_cancelled_registry_connect_reports_cancelled() -> io::Result<()> {
    let setup = setup(ServerOptions::default())?;
    let registry = DriveRegistry::new(Arc::new(MemorySecretStore::new()));
    registry.register_factory(SftpFactory::new());
    let config = drive_config(&setup.server, "c", &setup.known_hosts, &[])?;
    let id = config.id.clone();
    registry.add(config).map_err(io::Error::other)?;
    let cancel = Cancel::new();
    cancel.cancel();
    let outcome = registry.connect(&id, &Scripted::default(), &cancel);
    assert_eq!(
        outcome.err(),
        Some(ConnectOrRegistryError::Connect(ConnectError::Cancelled))
    );
    Ok(())
}

#[test]
fn no_secret_shows_in_errors_or_debug_output() -> io::Result<()> {
    let setup = setup(ServerOptions::default())?;
    let config = drive_config(&setup.server, "dbg", &setup.known_hosts, &[])?;
    let factory = SftpFactory::new();
    let secret = Secret::new(PASSWORD);
    let backend = factory
        .open(&config, Some(&secret), &Scripted::default(), &Cancel::new())
        .map_err(io::Error::other)?;
    let wrong = Secret::new("hunter2-wrong-password");
    let failure = factory
        .open(&config, Some(&wrong), &Scripted::default(), &Cancel::new())
        .err();
    let missing = backend.stat(&rp("/no/such/file")?).err();
    let texts = [
        format!("{backend:?}"),
        format!("{factory:?}"),
        format!("{config:?}"),
        format!("{secret:?}"),
        format!("{failure:?} {}", failure.as_ref().map(ToString::to_string).unwrap_or_default()),
        format!("{missing:?} {}", missing.as_ref().map(ToString::to_string).unwrap_or_default()),
        format!("{:?}", kara_remote::sftp::SftpParams::from_config(&config)),
    ];
    for text in &texts {
        assert!(!text.contains(PASSWORD), "the password leaked: {text}");
        assert!(!text.contains("hunter2"), "the wrong password leaked: {text}");
    }
    Ok(())
}
