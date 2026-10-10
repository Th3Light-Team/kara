//! Host keys: unknown → prompt → remembered; changed → refused by default and
//! the file never touched; hashed and `[host]:port` entries; `@revoked`.

mod support;

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use kara_remote::sftp::SftpFactory;
use kara_remote::sftp::known_hosts::{self, HostKeyStatus, KnownHosts};
use kara_remote::{
    ConnectError, ConnectionState, DriveRegistry, MemorySecretStore, Prompt, PromptAnswer,
    RefuseAll, Secret,
};
use kara_vfs::Cancel;
use support::{
    PASSWORD, Scripted, ServerOptions, TestServer, drive_config, hashed_host, key_from_seed,
    known_hosts_line,
};

fn open(
    factory: &SftpFactory,
    server: &TestServer,
    known: &Path,
    prompts: &dyn kara_remote::PromptHandler,
) -> Result<Arc<support::SftpBackend>, ConnectError> {
    let config = drive_config(server, "hk", known, &[]).map_err(|e| ConnectError::Other(e.to_string()))?;
    factory.open(&config, Some(&Secret::new(PASSWORD)), prompts, &Cancel::new())
}

fn fingerprint(server: &TestServer) -> String {
    known_hosts::fingerprint(server.host_key())
}

fn mode(path: &Path) -> io::Result<u32> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o777)
}

#[test]
fn an_unknown_key_is_asked_remembered_with_0600_and_then_silent() -> io::Result<()> {
    let server = TestServer::start(ServerOptions::default())?;
    let client = tempfile::tempdir()?;
    // Neither the folder nor the file exist yet.
    let known = client.path().join("dot-ssh").join("known_hosts");

    let prompts = Scripted::new([PromptAnswer::Trust { remember: true }]);
    let backend = open(&SftpFactory::default(), &server, &known, &prompts).map_err(io::Error::other)?;
    assert!(backend.is_connected());
    assert_eq!(
        prompts.asked(),
        vec![Prompt::TrustHostKey {
            drive: drive_config(&server, "hk", &known, &[])?.id,
            host: server.entry(),
            fingerprint: fingerprint(&server),
        }]
    );
    assert!(fingerprint(&server).starts_with("SHA256:"));
    assert_eq!(mode(&known)?, 0o600);
    assert_eq!(mode(known.parent().ok_or_else(|| io::Error::other("parent"))?)?, 0o700);
    let text = fs::read_to_string(&known)?;
    assert_eq!(text, known_hosts_line(&server.entry(), server.host_key())?);

    // A fresh factory (nothing trusted in memory): the file alone is enough.
    let silent = Scripted::default();
    open(&SftpFactory::default(), &server, &known, &silent).map_err(io::Error::other)?;
    assert!(silent.asked().is_empty(), "asked again: {:?}", silent.asked());
    Ok(())
}

#[test]
fn appending_keeps_existing_lines_and_adds_a_missing_newline() -> io::Result<()> {
    let server = TestServer::start(ServerOptions::default())?;
    let client = tempfile::tempdir()?;
    let known = client.path().join("known_hosts");
    let other = known_hosts_line("other.example", key_from_seed(40).public_key())?;
    let before = other.trim_end().to_owned(); // no trailing newline
    fs::write(&known, &before)?;
    fs::set_permissions(&known, fs::Permissions::from_mode(0o644))?;

    let prompts = Scripted::new([PromptAnswer::Trust { remember: true }]);
    open(&SftpFactory::default(), &server, &known, &prompts).map_err(io::Error::other)?;
    let text = fs::read_to_string(&known)?;
    assert_eq!(
        text,
        format!("{before}\n{}", known_hosts_line(&server.entry(), server.host_key())?)
    );
    assert_eq!(mode(&known)?, 0o644, "an existing file keeps its mode");
    Ok(())
}

#[test]
fn refusing_an_unknown_key_connects_nothing_and_writes_nothing() -> io::Result<()> {
    let server = TestServer::start(ServerOptions::default())?;
    let client = tempfile::tempdir()?;
    let known = client.path().join("known_hosts");
    let outcome = open(&SftpFactory::default(), &server, &known, &RefuseAll);
    assert_eq!(outcome.err(), Some(ConnectError::HostKeyRefused));
    assert!(!known.exists(), "a refusal must not create known_hosts");
    Ok(())
}

#[test]
fn trust_once_is_kept_for_the_run_but_not_written() -> io::Result<()> {
    let server = TestServer::start(ServerOptions::default())?;
    let client = tempfile::tempdir()?;
    let known = client.path().join("known_hosts");
    let factory = SftpFactory::default();
    let prompts = Scripted::new([PromptAnswer::Trust { remember: false }]);
    open(&factory, &server, &known, &prompts).map_err(io::Error::other)?;
    assert!(!known.exists());
    // Same factory: no second question.
    let again = Scripted::default();
    open(&factory, &server, &known, &again).map_err(io::Error::other)?;
    assert!(again.asked().is_empty());
    // Another run (factory): asked again.
    let fresh = Scripted::default();
    assert_eq!(
        open(&SftpFactory::default(), &server, &known, &fresh).err(),
        Some(ConnectError::HostKeyRefused)
    );
    assert_eq!(fresh.asked().len(), 1);
    Ok(())
}

#[test]
fn a_changed_key_is_refused_by_default_and_the_file_is_untouched() -> io::Result<()> {
    let server = TestServer::start(ServerOptions::default())?;
    let client = tempfile::tempdir()?;
    let known = client.path().join("known_hosts");
    let recorded = key_from_seed(99);
    let before = known_hosts_line(&server.entry(), recorded.public_key())?;
    fs::write(&known, &before)?;
    let modified = fs::metadata(&known)?.modified()?;

    let prompts = Scripted::default(); // answers Refuse
    let outcome = open(&SftpFactory::default(), &server, &known, &prompts);
    assert_eq!(outcome.err(), Some(ConnectError::HostKeyRefused));
    assert_eq!(
        prompts.asked(),
        vec![Prompt::HostKeyChanged {
            drive: drive_config(&server, "hk", &known, &[])?.id,
            host: server.entry(),
            known: known_hosts::fingerprint(recorded.public_key()),
            presented: fingerprint(&server),
        }]
    );
    assert_eq!(fs::read_to_string(&known)?, before);
    assert_eq!(fs::metadata(&known)?.modified()?, modified);

    // Even an explicit «trust and remember» never rewrites the file.
    let insist = Scripted::new([PromptAnswer::Trust { remember: true }]);
    open(&SftpFactory::default(), &server, &known, &insist).map_err(io::Error::other)?;
    assert_eq!(fs::read_to_string(&known)?, before);
    Ok(())
}

#[test]
fn a_hashed_entry_is_recognised_and_a_hashed_mismatch_is_a_change() -> io::Result<()> {
    let server = TestServer::start(ServerOptions::default())?;
    let client = tempfile::tempdir()?;
    let known = client.path().join("known_hosts");
    let hashed = hashed_host(&server.entry(), b"0123456789abcdefghij")?;
    fs::write(&known, known_hosts_line(&hashed, server.host_key())?)?;
    let silent = Scripted::default();
    open(&SftpFactory::default(), &server, &known, &silent).map_err(io::Error::other)?;
    assert!(silent.asked().is_empty());

    fs::write(&known, known_hosts_line(&hashed, key_from_seed(5).public_key())?)?;
    let prompts = Scripted::default();
    assert_eq!(
        open(&SftpFactory::default(), &server, &known, &prompts).err(),
        Some(ConnectError::HostKeyRefused)
    );
    assert!(matches!(prompts.asked().first(), Some(Prompt::HostKeyChanged { .. })));
    Ok(())
}

#[test]
fn a_port_22_entry_does_not_cover_another_port() -> io::Result<()> {
    let server = TestServer::start(ServerOptions::default())?;
    let client = tempfile::tempdir()?;
    let known = client.path().join("known_hosts");
    // `127.0.0.1` alone means port 22.
    fs::write(&known, known_hosts_line("127.0.0.1", key_from_seed(3).public_key())?)?;
    let prompts = Scripted::default();
    assert_eq!(
        open(&SftpFactory::default(), &server, &known, &prompts).err(),
        Some(ConnectError::HostKeyRefused)
    );
    assert!(
        matches!(prompts.asked().first(), Some(Prompt::TrustHostKey { .. })),
        "a key recorded for port 22 is not a change for this port: {:?}",
        prompts.asked()
    );
    Ok(())
}

#[test]
fn a_revoked_key_is_refused_without_asking() -> io::Result<()> {
    let server = TestServer::start(ServerOptions::default())?;
    let client = tempfile::tempdir()?;
    let known = client.path().join("known_hosts");
    let line = known_hosts_line(&server.entry(), server.host_key())?;
    fs::write(&known, format!("{line}@revoked * {}", line.split_once(' ').map(|(_, k)| k).unwrap_or("")))?;
    let prompts = Scripted::new([PromptAnswer::Trust { remember: true }]);
    assert_eq!(
        open(&SftpFactory::default(), &server, &known, &prompts).err(),
        Some(ConnectError::HostKeyRefused)
    );
    assert!(prompts.asked().is_empty());
    Ok(())
}

#[test]
fn the_registry_routes_host_key_prompts_through_its_handler() -> io::Result<()> {
    let server = TestServer::start(ServerOptions::default())?;
    let client = tempfile::tempdir()?;
    let known = client.path().join("known_hosts");
    let registry = DriveRegistry::new(Arc::new(MemorySecretStore::new()));
    registry.register_factory(SftpFactory::new());
    let config = drive_config(&server, "reg", &known, &[])?;
    let id = config.id.clone();
    registry.add(config).map_err(io::Error::other)?;
    let prompts = Scripted::new([
        PromptAnswer::Trust { remember: true },
        PromptAnswer::Secret {
            secret: Secret::new(PASSWORD),
            remember: kara_remote::Remember::No,
        },
    ]);
    registry.connect(&id, &prompts, &Cancel::new()).map_err(io::Error::other)?;
    assert_eq!(registry.state(&id), Some(ConnectionState::Ready));
    let asked = prompts.asked();
    assert!(matches!(asked.first(), Some(Prompt::TrustHostKey { .. })), "{asked:?}");
    assert!(matches!(asked.get(1), Some(Prompt::Password { .. })), "{asked:?}");
    assert_eq!(asked.len(), 2, "the host key is not asked twice: {asked:?}");
    Ok(())
}

// -- the parser on its own ---------------------------------------------------

#[test]
fn the_parser_skips_comments_bad_lines_and_honours_patterns() -> io::Result<()> {
    let key = key_from_seed(11);
    let other = key_from_seed(12);
    let line = known_hosts_line("x", key.public_key())?;
    let (_, rest) = line.split_once(' ').ok_or_else(|| io::Error::other("line"))?;
    let rest = rest.trim_end();
    let text = format!(
        "# a comment\n\
         garbage line that is not a key\n\
         \n\
         @cert-authority *.lan {rest}\n\
         web.lan,db.lan\t{rest} comment here\n\
         *.office,!printer.office {rest}\n\
         [nas.lan]:2222 {rest}\n"
    );
    let known = KnownHosts::parse(&text);
    let entry = |host: &str, port: u16| known_hosts::host_entry(host, port);
    assert_eq!(known.check(&entry("web.lan", 22), key.public_key()), HostKeyStatus::Trusted);
    assert_eq!(known.check(&entry("DB.lan", 22), key.public_key()), HostKeyStatus::Trusted);
    assert_eq!(known.check(&entry("pc.office", 22), key.public_key()), HostKeyStatus::Trusted);
    assert_eq!(known.check(&entry("printer.office", 22), key.public_key()), HostKeyStatus::Unknown);
    assert_eq!(known.check(&entry("nas.lan", 2222), key.public_key()), HostKeyStatus::Trusted);
    assert_eq!(known.check(&entry("nas.lan", 22), key.public_key()), HostKeyStatus::Unknown);
    // The CA line alone does not make a plain key trusted.
    assert_eq!(known.check(&entry("other.lan", 22), key.public_key()), HostKeyStatus::Unknown);
    assert_eq!(
        known.check(&entry("web.lan", 22), other.public_key()),
        HostKeyStatus::Changed {
            known: known_hosts::fingerprint(key.public_key())
        }
    );
    Ok(())
}
