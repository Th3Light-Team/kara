//! The default agent is `$SSH_AUTH_SOCK`. This file changes the environment,
//! so it holds a single test (test threads share the environment).

mod support;

use std::io;

use kara_remote::sftp::SftpFactory;
use kara_remote::{ConnectError, Secret};
use kara_vfs::Cancel;
use support::{PASSWORD, Scripted, ServerOptions, TestServer, key_from_seed, trusting_known_hosts};

#[test]
fn ssh_auth_sock_is_used_when_set_and_ignored_when_dangling() -> io::Result<()> {
    let key = key_from_seed(31);
    let server = TestServer::start(ServerOptions {
        keys: vec![key.public_key().clone()],
        ..ServerOptions::default()
    })?;
    let client = tempfile::tempdir()?;
    let known = trusting_known_hosts(&server, client.path())?;
    let config = kara_remote::DriveConfig::new(
        "sftp",
        "env",
        "env",
        [
            ("host", "127.0.0.1".to_owned()),
            ("port", server.port.to_string()),
            ("user", support::USER.to_owned()),
            ("known_hosts", known.to_string_lossy().into_owned()),
        ]
        .map(|(k, v)| (k.to_owned(), v)),
    )
    .map_err(io::Error::other)?;
    let factory = SftpFactory::default();

    // Dangling socket: the agent is skipped, the password still works, and
    // without one the answer is «a secret is needed».
    // SAFETY: this is the only test in this binary; nothing else reads the
    // environment concurrently.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", client.path().join("gone.sock")) };
    let secret = Secret::new(PASSWORD);
    factory
        .open(&config, Some(&secret), &Scripted::default(), &Cancel::new())
        .map_err(io::Error::other)?;
    assert_eq!(
        factory
            .open(&config, None, &Scripted::default(), &Cancel::new())
            .err(),
        Some(ConnectError::AuthRequired)
    );

    // A live agent with the right key: no secret needed.
    let runtime = tokio::runtime::Runtime::new()?;
    let socket = support::start_agent(&runtime, client.path(), &[key])?;
    // SAFETY: as above.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", &socket) };
    let backend = factory
        .open(&config, None, &Scripted::default(), &Cancel::new())
        .map_err(io::Error::other)?;
    assert!(backend.is_connected());
    Ok(())
}
