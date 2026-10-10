//! SFTP drives (feature `sftp`): [`SftpFactory`] serves the scheme `sftp`,
//! [`SftpBackend`] implements [`kara_vfs::Backend`] over `russh` +
//! `russh-sftp` (pure Rust).
//!
//! # Registering it (kara-ui)
//!
//! ```ignore
//! let registry = DriveRegistry::new(secret_store);
//! registry.register_factory(kara_remote::sftp::SftpFactory::new());
//! registry.add_all(drives_from_settings);
//! // On a worker thread, with the bridge's PromptHandler:
//! registry.connect(&id, &prompts, &cancel)?;
//! ```
//!
//! Build `kara-ui` with `kara-remote/sftp` enabled. Nothing else is needed: the
//! registry asks for the password itself, and host-key questions arrive at the
//! same `PromptHandler` as [`Prompt::TrustHostKey`] and [`Prompt::HostKeyChanged`].
//! On any `Unavailable` from a listing or a job, call
//! `DriveRegistry::report_failure`; the backend never reconnects by itself.
//!
//! # Parameters (`DriveConfig::params`)
//!
//! | Key | Default | Meaning |
//! |---|---|---|
//! | `host` | required | Name or address |
//! | `port` | `22` | |
//! | `user` | `$USER` | |
//! | `key_file` | none | Private key (OpenSSH, PEM or PuTTY); `~/` is expanded |
//! | `known_hosts` | `~/.ssh/known_hosts` | Read, and appended to when a key is remembered |
//! | `root` | `/` | Server folder the drive starts at; `~` and relative paths are resolved by the server |
//! | `agent` | `$SSH_AUTH_SOCK` | `none` to skip the agent, or a socket path |
//! | `timeout_s` | `30` | Per request, and twice that for connecting |
//! | `keepalive_s` | `30` | Interval; three unanswered keepalives close the session |
//!
//! The secret ([`Secret`]) is the password, or the key's passphrase when
//! `key_file` is set. It is handed to the protocol and dropped: it is never
//! logged, never part of an error, and no type here prints it.
//!
//! # Authentication
//!
//! Agent (only the identity matching `key_file` when one is set) → `key_file`
//! → password. A secret that is needed and missing is
//! [`ConnectError::AuthRequired`] (the registry then asks for it); a rejected
//! one is [`ConnectError::AuthFailed`]. With `key_file` there is no password
//! fallback. A network failure is [`ConnectError::Unreachable`].
//!
//! # Host keys
//!
//! Checked against `known_hosts` (plain, hashed `|1|` and `[host]:port`
//! entries; see [`known_hosts`]):
//!
//! - known and equal: connect;
//! - unknown: [`Prompt::TrustHostKey`] with the `SHA256:` fingerprint.
//!   `Trust { remember: true }` appends one line (file 0600, folder 0700 if
//!   they have to be created); `Trust { remember: false }` trusts the key for
//!   the life of this factory only; anything else is
//!   [`ConnectError::HostKeyRefused`];
//! - changed: [`Prompt::HostKeyChanged`]. Only an explicit `Trust` proceeds,
//!   for this run only: the file is **never** rewritten, the user removes the
//!   old line;
//! - `@revoked`: refused without asking.
//!
//! # Capabilities
//!
//! `atomic_rename` (and with it `undo_rename`/`undo_move`) is true only when
//! the server offers `posix-rename@openssh.com`, probed once at connect; the
//! flags never change afterwards. No trash, no server-side copy, no watching;
//! real directories, POSIX permissions and symlinks.
//!
//! # Session health
//!
//! Keepalive every `keepalive_s`. Each request waits at most `timeout_s`. Once
//! the connection or the channel is gone, every call answers `Unavailable`
//! immediately. There is no reconnect inside: the caller reports the failure.

mod backend;
mod connect;
mod error;
mod io;
pub mod known_hosts;
mod params;
mod session;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use kara_vfs::{Backend, Cancel};
use russh::keys::PublicKey;

pub use backend::SftpBackend;
pub use params::{AgentChoice, DEFAULT_KEEPALIVE_S, DEFAULT_TIMEOUT_S, SftpParams};

use crate::config::DriveConfig;
use crate::registry::{BackendFactory, ConnectError, Prompt, PromptAnswer, PromptHandler};
use crate::secrets::Secret;
use connect::{Attempt, bounded, establish};
use known_hosts::{HostKeyStatus, KnownHosts, fingerprint};
use session::Session;

/// Opens `sftp` drives.
#[derive(Default)]
pub struct SftpFactory {
    /// Host keys the user trusted for this run without writing them down,
    /// by `known_hosts` entry name.
    trusted_for_run: Mutex<HashMap<String, Vec<PublicKey>>>,
}

impl std::fmt::Debug for SftpFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SftpFactory").finish_non_exhaustive()
    }
}

impl SftpFactory {
    #[must_use]
    pub fn new() -> Arc<SftpFactory> {
        Arc::new(SftpFactory::default())
    }

    fn trusted(&self, entry: &str) -> Vec<PublicKey> {
        match self.trusted_for_run.lock() {
            Ok(map) => map.get(entry).cloned().unwrap_or_default(),
            Err(poisoned) => poisoned.into_inner().get(entry).cloned().unwrap_or_default(),
        }
    }

    fn trust_for_run(&self, entry: &str, key: PublicKey) {
        let mut map = match self.trusted_for_run.lock() {
            Ok(map) => map,
            Err(poisoned) => poisoned.into_inner(),
        };
        map.entry(entry.to_owned()).or_default().push(key);
    }

    /// Like [`BackendFactory::connect`], keeping the concrete type.
    pub fn open(
        &self,
        config: &DriveConfig,
        secret: Option<&Secret>,
        prompts: &dyn PromptHandler,
        cancel: &Cancel,
    ) -> Result<Arc<SftpBackend>, ConnectError> {
        if cancel.is_cancelled() {
            return Err(ConnectError::Cancelled);
        }
        let params = SftpParams::from_config(config)?;
        let known = Arc::new(KnownHosts::load(&params.known_hosts).map_err(|error| {
            ConnectError::Other(format!(
                "{} cannot be read: {error}",
                params.known_hosts.display()
            ))
        })?);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("kara-sftp")
            .enable_all()
            .build()
            .map_err(|error| ConnectError::Other(format!("no runtime for SFTP: {error}")))?;
        let entry = params.host_entry();
        let limit = params.timeout.saturating_mul(2);
        // At most: one attempt that meets an untrusted key, one with it trusted.
        for _ in 0..2 {
            let accept = self.trusted(&entry);
            let attempt = runtime.block_on(bounded(
                establish(&params, secret, Arc::clone(&known), accept),
                cancel,
                limit,
            ));
            let presented = match attempt {
                Ok(established) => {
                    let session = Session::new(
                        runtime,
                        established.sftp,
                        established.ssh,
                        params.timeout,
                        established.posix_rename,
                        established.fsync,
                    );
                    let label = format!("{}@{}", params.user, entry);
                    return Ok(Arc::new(SftpBackend::new(session, established.root, label)));
                }
                Err(Attempt::Failed(error)) => {
                    runtime.shutdown_background();
                    return Err(error);
                }
                Err(Attempt::HostKey(presented)) => presented,
            };
            if !self.ask_about(config, &params, &entry, *presented, prompts) {
                runtime.shutdown_background();
                return Err(ConnectError::HostKeyRefused);
            }
            if cancel.is_cancelled() {
                runtime.shutdown_background();
                return Err(ConnectError::Cancelled);
            }
        }
        runtime.shutdown_background();
        // The host showed yet another key on the second attempt.
        Err(ConnectError::HostKeyRefused)
    }

    /// Asks the user about an untrusted host key. `true` to go on with it.
    fn ask_about(
        &self,
        config: &DriveConfig,
        params: &SftpParams,
        entry: &str,
        presented: connect::Presented,
        prompts: &dyn PromptHandler,
    ) -> bool {
        let presented_fingerprint = fingerprint(&presented.key);
        let prompt = match presented.status {
            HostKeyStatus::Trusted => return true,
            HostKeyStatus::Revoked => return false,
            HostKeyStatus::Unknown => Prompt::TrustHostKey {
                drive: config.id.clone(),
                host: entry.to_owned(),
                fingerprint: presented_fingerprint,
            },
            HostKeyStatus::Changed { known } => Prompt::HostKeyChanged {
                drive: config.id.clone(),
                host: entry.to_owned(),
                known,
                presented: presented_fingerprint,
            },
        };
        let changed = matches!(prompt, Prompt::HostKeyChanged { .. });
        match prompts.ask(&prompt) {
            PromptAnswer::Trust { remember } => {
                // A changed key is never written down, whatever the answer says.
                if remember && !changed {
                    // Not fatal: the user trusted the key, so this connection
                    // goes ahead; the next one asks again.
                    let _ = known_hosts::append(&params.known_hosts, entry, &presented.key);
                }
                self.trust_for_run(entry, presented.key);
                true
            }
            PromptAnswer::Refuse | PromptAnswer::Secret { .. } => false,
        }
    }
}

impl BackendFactory for SftpFactory {
    fn scheme(&self) -> &str {
        "sftp"
    }

    fn connect(
        &self,
        config: &DriveConfig,
        secret: Option<&Secret>,
        prompts: &dyn PromptHandler,
        cancel: &Cancel,
    ) -> Result<Arc<dyn Backend>, ConnectError> {
        let backend: Arc<dyn Backend> = self.open(config, secret, prompts, cancel)?;
        Ok(backend)
    }
}
