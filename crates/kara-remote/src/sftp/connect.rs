//! Opening a session: TCP, SSH handshake with the host-key check,
//! authentication, the SFTP subsystem and the extension probe.
//!
//! All of it is async and runs inside [`super::SftpFactory`]'s private runtime.
//! The host-key check never blocks inside the runtime: when the key is not
//! trusted, the handshake is refused and the key is handed back
//! ([`Attempt::HostKey`]); the factory asks the user on its own thread and, if
//! the user trusts the key, connects again accepting exactly that key.

use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kara_vfs::Cancel;
use russh::client::{self, AuthResult, Handle};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh_sftp::client::RawSftpSession;

use super::known_hosts::{HostKeyStatus, KnownHosts};
use super::params::{AgentChoice, SftpParams};
use crate::registry::ConnectError;
use crate::secrets::Secret;

/// The key a host presented, and what `known_hosts` said about it.
#[derive(Debug, Clone)]
pub(crate) struct Presented {
    pub key: PublicKey,
    pub status: HostKeyStatus,
}

/// The SSH client side: only checks the host key.
pub(crate) struct Client {
    known: Arc<KnownHosts>,
    entry: String,
    /// Keys the user trusted without (or before) writing them down.
    accept: Vec<PublicKey>,
    presented: Arc<Mutex<Option<Presented>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(certificate) => {
                PublicKey::from(certificate.public_key().clone())
            }
        };
        let status = if self
            .accept
            .iter()
            .any(|accepted| accepted.key_data() == key.key_data())
        {
            HostKeyStatus::Trusted
        } else {
            self.known.check(&self.entry, &key)
        };
        let trusted = status == HostKeyStatus::Trusted;
        if let Ok(mut slot) = self.presented.lock() {
            *slot = Some(Presented { key, status });
        }
        Ok(trusted)
    }
}

/// Why an attempt ended without a session.
pub(crate) enum Attempt {
    /// The host key is not trusted (yet): ask the user.
    HostKey(Box<Presented>),
    Failed(ConnectError),
}

/// What a successful attempt hands over.
pub(crate) struct Established {
    pub ssh: Handle<Client>,
    pub sftp: RawSftpSession,
    pub root: String,
    pub posix_rename: bool,
    pub fsync: bool,
}

/// Runs `future`, giving up when `cancel` is set or `limit` passes.
pub(crate) async fn bounded<T>(
    future: impl Future<Output = Result<T, Attempt>>,
    cancel: &Cancel,
    limit: Duration,
) -> Result<T, Attempt> {
    let watch = async {
        while !cancel.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    tokio::select! {
        outcome = tokio::time::timeout(limit, future) => match outcome {
            Ok(result) => result,
            Err(_) => Err(Attempt::Failed(ConnectError::Unreachable(String::from(
                "the server did not answer in time",
            )))),
        },
        () = watch => Err(Attempt::Failed(ConnectError::Cancelled)),
    }
}

fn unreachable(error: &impl std::fmt::Display) -> Attempt {
    Attempt::Failed(ConnectError::Unreachable(error.to_string()))
}

/// Whether a russh error means the network or the peer went away.
fn is_network(error: &russh::Error) -> bool {
    matches!(
        error,
        russh::Error::IO(_)
            | russh::Error::Disconnect
            | russh::Error::HUP
            | russh::Error::ConnectionTimeout
            | russh::Error::KeepaliveTimeout
            | russh::Error::InactivityTimeout
            | russh::Error::SendError
    )
}

fn russh_failure(error: &russh::Error) -> Attempt {
    if is_network(error) {
        unreachable(error)
    } else {
        Attempt::Failed(ConnectError::Other(error.to_string()))
    }
}

/// One full attempt. `accept` are keys trusted by the user for this run.
pub(crate) async fn establish(
    params: &SftpParams,
    secret: Option<&Secret>,
    known: Arc<KnownHosts>,
    accept: Vec<PublicKey>,
) -> Result<Established, Attempt> {
    let stream = tokio::net::TcpStream::connect((params.host.as_str(), params.port))
        .await
        .map_err(|error| unreachable(&error))?;
    let _ = stream.set_nodelay(true);

    let config = client::Config {
        keepalive_interval: Some(params.keepalive),
        keepalive_max: 3,
        inactivity_timeout: None,
        nodelay: true,
        ..client::Config::default()
    };
    let presented = Arc::new(Mutex::new(None));
    let handler = Client {
        known,
        entry: params.host_entry(),
        accept,
        presented: Arc::clone(&presented),
    };
    let mut ssh = match client::connect_stream(Arc::new(config), stream, handler).await {
        Ok(ssh) => ssh,
        Err(error) => {
            let seen = presented.lock().ok().and_then(|slot| slot.clone());
            return Err(match seen {
                Some(presented) if presented.status != HostKeyStatus::Trusted => {
                    Attempt::HostKey(Box::new(presented))
                }
                _ => russh_failure(&error),
            });
        }
    };

    authenticate(&mut ssh, params, secret).await?;

    let channel = ssh
        .channel_open_session()
        .await
        .map_err(|error| russh_failure(&error))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| russh_failure(&error))?;
    let config = russh_sftp::client::Config {
        request_timeout_secs: params.timeout.as_secs().max(1),
        ..russh_sftp::client::Config::default()
    };
    let sftp = RawSftpSession::new_with_config(channel.into_stream(), config);
    let version = sftp.init().await.map_err(|error| {
        Attempt::Failed(ConnectError::Other(format!(
            "the server did not start SFTP: {error}"
        )))
    })?;
    let offered = |name: &str| version.extensions.contains_key(name);
    let posix_rename = offered("posix-rename@openssh.com");
    let fsync = offered("fsync@openssh.com");

    let root = resolve_root(&sftp, params.root.as_deref()).await?;
    Ok(Established {
        ssh,
        sftp,
        root,
        posix_rename,
        fsync,
    })
}

/// The server directory `RemotePath` "/" stands for. Absolute paths are kept
/// as written (lexically normalised); `~`, `~/x` and relative paths are
/// resolved by the server. The result must be a directory.
async fn resolve_root(sftp: &RawSftpSession, root: Option<&str>) -> Result<String, Attempt> {
    let failed = |reason: String| Attempt::Failed(ConnectError::Other(reason));
    let wanted = root.unwrap_or("/");
    let path = if wanted.starts_with('/') {
        normalise(wanted)
    } else {
        let relative = wanted.strip_prefix("~/").unwrap_or(if wanted == "~" { "." } else { wanted });
        let name = sftp
            .realpath(relative)
            .await
            .map_err(|error| failed(format!("the start folder {wanted:?} cannot be resolved: {error}")))?;
        let resolved = name
            .files
            .into_iter()
            .next()
            .map(|file| file.filename)
            .ok_or_else(|| failed(format!("the start folder {wanted:?} cannot be resolved")))?;
        normalise(&resolved)
    };
    let attrs = sftp
        .stat(path.as_str())
        .await
        .map_err(|error| failed(format!("the start folder {wanted:?} is not reachable: {error}")))?;
    if !attrs.attrs.is_dir() {
        return Err(failed(format!("the start folder {wanted:?} is not a folder")));
    }
    Ok(path)
}

/// `/a//b/./c/` → `/a/b/c`; `..` is kept out (refused by stopping at the root).
fn normalise(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        String::from("/")
    } else {
        format!("/{}", parts.join("/"))
    }
}

/// Agent, then key file, then password. The secret is the key's passphrase
/// when a key file is configured, the password otherwise.
async fn authenticate(
    ssh: &mut Handle<Client>,
    params: &SftpParams,
    secret: Option<&Secret>,
) -> Result<(), Attempt> {
    let user = params.user.as_str();
    // The key file, if any, is read first: with a key file only the agent
    // identity of that same key is offered, so a crowded agent cannot use up
    // the server's MaxAuthTries before the key and the password get a turn.
    let key_file = match &params.key_file {
        Some(path) => Some(load_key(path, secret)?),
        None => None,
    };
    let only = key_file.as_ref().map(|key| key.public_key().clone());
    if try_agent(ssh, params, only.as_ref()).await? {
        return Ok(());
    }

    if let Some(key) = key_file {
        let hash = rsa_hash(ssh, &key).await;
        let result = ssh
            .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
            .await
            .map_err(|error| russh_failure(&error))?;
        return match result {
            AuthResult::Success => Ok(()),
            AuthResult::Failure { .. } => Err(Attempt::Failed(ConnectError::AuthFailed)),
        };
    }

    let Some(password) = secret else {
        return Err(Attempt::Failed(ConnectError::AuthRequired));
    };
    let result = ssh
        .authenticate_password(user, password.expose())
        .await
        .map_err(|error| russh_failure(&error))?;
    match result {
        AuthResult::Success => Ok(()),
        AuthResult::Failure { .. } => Err(Attempt::Failed(ConnectError::AuthFailed)),
    }
}

/// Reads the private key; the secret is its passphrase.
fn load_key(path: &Path, secret: Option<&Secret>) -> Result<PrivateKey, Attempt> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        Attempt::Failed(ConnectError::Other(format!(
            "the key file {} cannot be read: {error}",
            path.display()
        )))
    })?;
    match russh::keys::decode_secret_key(&text, None) {
        Ok(key) => Ok(key),
        Err(russh::keys::Error::KeyIsEncrypted) => match secret {
            None => Err(Attempt::Failed(ConnectError::AuthRequired)),
            Some(passphrase) => russh::keys::decode_secret_key(&text, Some(passphrase.expose()))
                .map_err(|_| Attempt::Failed(ConnectError::AuthFailed)),
        },
        Err(error) => Err(Attempt::Failed(ConnectError::Other(format!(
            "the key file {} is not a usable private key: {error}",
            path.display()
        )))),
    }
}

async fn rsa_hash(ssh: &Handle<Client>, key: &PrivateKey) -> Option<HashAlg> {
    if !key.algorithm().is_rsa() {
        return None;
    }
    ssh.best_supported_rsa_hash().await.ok().flatten().flatten()
}

/// Tries the agent's identities (only `only` when given). `Ok(true)` when one
/// was accepted; an absent or broken agent is not an error.
async fn try_agent(
    ssh: &mut Handle<Client>,
    params: &SftpParams,
    only: Option<&PublicKey>,
) -> Result<bool, Attempt> {
    let socket = match &params.agent {
        AgentChoice::Off => return Ok(false),
        AgentChoice::Socket(path) => path.clone(),
        AgentChoice::Environment => match std::env::var_os("SSH_AUTH_SOCK") {
            Some(path) if !path.is_empty() => path.into(),
            _ => return Ok(false),
        },
    };
    let Ok(mut agent) = AgentClient::connect_uds(&socket).await else {
        return Ok(false);
    };
    let Ok(identities) = agent.request_identities().await else {
        return Ok(false);
    };
    for identity in identities {
        let AgentIdentity::PublicKey { key, .. } = identity else {
            continue;
        };
        if only.is_some_and(|wanted| wanted.key_data() != key.key_data()) {
            continue;
        }
        let hash = if key.algorithm().is_rsa() {
            ssh.best_supported_rsa_hash().await.ok().flatten().flatten()
        } else {
            None
        };
        match ssh
            .authenticate_publickey_with(params.user.as_str(), key, hash, &mut agent)
            .await
        {
            Ok(AuthResult::Success) => return Ok(true),
            Ok(AuthResult::Failure { .. }) => {}
            // The agent stopped answering: go on without it.
            Err(_) => {
                if ssh.is_closed() {
                    return Err(Attempt::Failed(ConnectError::Unreachable(String::from(
                        "the connection closed during authentication",
                    ))));
                }
                return Ok(false);
            }
        }
    }
    Ok(false)
}
