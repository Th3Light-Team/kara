//! The conformance suite against a real `sshd`. Ignored by default; run with
//!
//! ```text
//! KARA_TEST_SFTP="host:port,user,/path/to/private_key" \
//!     cargo test -p kara-remote --test sftp_real_server -- --ignored
//! ```
//!
//! - The key field may be empty to use a password from `KARA_TEST_SFTP_PASSWORD`
//!   (the key's passphrase, when the key is encrypted, goes there too).
//! - `KARA_TEST_SFTP_KNOWN_HOSTS` points at a known_hosts file (default
//!   `~/.ssh/known_hosts`). An unknown host key is trusted for this run only and
//!   never written down; a changed one is refused.
//! - The suite works in a fresh folder under the login's home directory
//!   (`kara-conformance-<pid>-<nanos>`) and removes it at the end.
//!
//! A disposable OpenSSH container is the intended target, e.g.
//! `docker run -p 2222:2222 -e USER_NAME=kara -e PUBLIC_KEY="$(cat id.pub)"
//! lscr.io/linuxserver/openssh-server` with `KARA_TEST_SFTP=127.0.0.1:2222,kara,id`.

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use kara_remote::sftp::SftpFactory;
use kara_remote::{DriveConfig, Prompt, PromptAnswer, PromptHandler, Secret};
use kara_vfs::conformance::{self, CaseOutcome};
use kara_vfs::{Backend, Cancel, RemotePath};

struct TrustNewOnly;

impl PromptHandler for TrustNewOnly {
    fn ask(&self, prompt: &Prompt) -> PromptAnswer {
        match prompt {
            Prompt::TrustHostKey { .. } => PromptAnswer::Trust { remember: false },
            _ => PromptAnswer::Refuse,
        }
    }
}

#[test]
#[ignore = "needs a real sshd: set KARA_TEST_SFTP=host:port,user,keyfile"]
fn conformance_against_a_real_sshd() -> io::Result<()> {
    let Ok(spec) = std::env::var("KARA_TEST_SFTP") else {
        eprintln!("KARA_TEST_SFTP is not set: nothing to test");
        return Ok(());
    };
    let mut fields = spec.split(',').map(str::trim);
    let (Some(address), Some(user)) = (fields.next(), fields.next()) else {
        return Err(io::Error::other("KARA_TEST_SFTP must be host:port,user[,keyfile]"));
    };
    let key_file = fields.next().filter(|k| !k.is_empty());
    let (host, port) = address.rsplit_once(':').unwrap_or((address, "22"));

    let mut params = vec![
        ("host".to_owned(), host.to_owned()),
        ("port".to_owned(), port.to_owned()),
        ("user".to_owned(), user.to_owned()),
        ("root".to_owned(), "~".to_owned()),
    ];
    if let Some(key) = key_file {
        params.push(("key_file".to_owned(), key.to_owned()));
    }
    if let Ok(known) = std::env::var("KARA_TEST_SFTP_KNOWN_HOSTS") {
        params.push(("known_hosts".to_owned(), known));
    }
    let config = DriveConfig::new("sftp", "real", "real", params).map_err(io::Error::other)?;
    let secret = std::env::var("KARA_TEST_SFTP_PASSWORD").ok().map(Secret::new);
    let backend = SftpFactory::new()
        .open(&config, secret.as_ref(), &TrustNewOnly, &Cancel::new())
        .map_err(|e| io::Error::other(format!("connect: {e}")))?;
    eprintln!("connected: {backend:?}");

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let scratch = RemotePath::root()
        .join(&format!("kara-conformance-{}-{nanos}", std::process::id()))
        .map_err(io::Error::other)?;
    backend.create_dir(&scratch).map_err(io::Error::other)?;

    let mut failures = Vec::new();
    for report in [conformance::run(backend.as_ref(), &scratch), conformance::run_extra(backend.as_ref(), &scratch)] {
        let report = report.map_err(|e| io::Error::other(e.to_string()))?;
        for case in report.cases {
            match case.outcome {
                CaseOutcome::Failed { detail } => failures.push(format!("{}: {detail}", case.id)),
                CaseOutcome::Skipped { because } => eprintln!("skipped {}: {because}", case.id),
                CaseOutcome::Passed => {}
            }
        }
    }
    let cleanup = backend.remove_tree(&scratch, &Cancel::new());
    assert!(failures.is_empty(), "conformance failures:\n{}", failures.join("\n"));
    cleanup.map_err(io::Error::other)?;
    Ok(())
}
