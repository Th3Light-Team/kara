//! The parameters of an `sftp` drive, read from its [`DriveConfig`].

use std::path::PathBuf;
use std::time::Duration;

use crate::config::DriveConfig;
use crate::registry::ConnectError;

use super::known_hosts;

/// Default per-request timeout, in seconds (`timeout_s`).
pub const DEFAULT_TIMEOUT_S: u64 = 30;
/// Default keepalive interval, in seconds (`keepalive_s`).
pub const DEFAULT_KEEPALIVE_S: u64 = 30;

/// Which SSH agent to ask, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentChoice {
    /// `$SSH_AUTH_SOCK`, read at connect time (the default).
    Environment,
    /// No agent (`agent = none`).
    Off,
    /// This socket (`agent = /path/to/socket`).
    Socket(PathBuf),
}

/// Everything needed to reach a drive. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub key_file: Option<PathBuf>,
    pub known_hosts: PathBuf,
    /// The server folder the drive starts at; `None` is the server's `/`.
    pub root: Option<String>,
    pub timeout: Duration,
    pub keepalive: Duration,
    pub agent: AgentChoice,
}

fn bad(reason: String) -> ConnectError {
    ConnectError::Other(reason)
}

/// `~` and `~/…` against `$HOME`.
fn expand_home(raw: &str) -> Result<PathBuf, ConnectError> {
    if raw == "~" || raw.starts_with("~/") {
        let home = std::env::home_dir()
            .ok_or_else(|| bad(String::from("no home directory to expand ~ against")))?;
        let rest = raw.trim_start_matches('~').trim_start_matches('/');
        return Ok(if rest.is_empty() { home } else { home.join(rest) });
    }
    Ok(PathBuf::from(raw))
}

fn seconds(config: &DriveConfig, key: &str, default: u64) -> Result<Duration, ConnectError> {
    match config.param(key) {
        None => Ok(Duration::from_secs(default)),
        Some(text) => match text.trim().parse::<u64>() {
            Ok(value) if (1..=3600).contains(&value) => Ok(Duration::from_secs(value)),
            _ => Err(bad(format!("{key} must be a number of seconds between 1 and 3600"))),
        },
    }
}

impl SftpParams {
    /// Reads and checks the parameters. `~/` in `key_file` and `known_hosts`
    /// is expanded here, at connect time.
    pub fn from_config(config: &DriveConfig) -> Result<SftpParams, ConnectError> {
        let host = config
            .param("host")
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .ok_or_else(|| bad(String::from("the drive has no host")))?
            .to_owned();
        let port = match config.param("port") {
            None => 22,
            Some(text) => match text.trim().parse::<u16>() {
                Ok(port) if port > 0 => port,
                _ => return Err(bad(format!("{text:?} is not a port number"))),
            },
        };
        let user = match config.param("user").map(str::trim).filter(|user| !user.is_empty()) {
            Some(user) => user.to_owned(),
            None => std::env::var("USER")
                .or_else(|_| std::env::var("LOGNAME"))
                .map_err(|_| bad(String::from("the drive has no user")))?,
        };
        let key_file = match config.param("key_file").map(str::trim).filter(|path| !path.is_empty()) {
            Some(raw) => Some(expand_home(raw)?),
            None => None,
        };
        let known_hosts = expand_home(
            config
                .param("known_hosts")
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .unwrap_or("~/.ssh/known_hosts"),
        )?;
        let root = config
            .param("root")
            .map(str::trim)
            .filter(|root| !root.is_empty())
            .map(str::to_owned);
        let agent = match config.param("agent").map(str::trim) {
            None | Some("") => AgentChoice::Environment,
            Some("none" | "no" | "off") => AgentChoice::Off,
            Some(path) => AgentChoice::Socket(expand_home(path)?),
        };
        Ok(SftpParams {
            host,
            port,
            user,
            key_file,
            known_hosts,
            root,
            timeout: seconds(config, "timeout_s", DEFAULT_TIMEOUT_S)?,
            keepalive: seconds(config, "keepalive_s", DEFAULT_KEEPALIVE_S)?,
            agent,
        })
    }

    /// How this host is recorded in `known_hosts`.
    #[must_use]
    pub fn host_entry(&self) -> String {
        known_hosts::host_entry(&self.host, self.port)
    }
}
