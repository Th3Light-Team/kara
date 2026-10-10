//! What the S3 and GCS factories share: parameter parsing, client tuning and
//! the check that runs at connect time.
//!
//! A factory builds the store, wraps it in an [`ObjectStoreBackend`] and makes
//! one cheap request (the first key under the drive's prefix). Wrong
//! credentials, a missing bucket or a dead endpoint come out of `connect`, not
//! out of the user's first click.

#[cfg(feature = "gcs")]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use kara_vfs::{BackendErrorKind, Cancel};

use super::backend::{DEFAULT_PART_SIZE, DEFAULT_TIMEOUT_S, DEFAULT_UPLOAD_CONCURRENCY, ObjectStoreBackend};
use super::error::Fail;
use crate::config::DriveConfig;
use crate::registry::ConnectError;

fn bad(reason: String) -> ConnectError {
    ConnectError::Other(reason)
}

/// A trimmed, non-empty parameter.
pub(crate) fn param<'a>(config: &'a DriveConfig, key: &str) -> Option<&'a str> {
    config.param(key).map(str::trim).filter(|value| !value.is_empty())
}

/// `true`/`false` (also `yes`/`no`, `1`/`0`); `None` when absent.
pub(crate) fn flag(config: &DriveConfig, key: &str) -> Result<Option<bool>, ConnectError> {
    match param(config, key).map(str::to_ascii_lowercase).as_deref() {
        None => Ok(None),
        Some("true" | "yes" | "1" | "on") => Ok(Some(true)),
        Some("false" | "no" | "0" | "off") => Ok(Some(false)),
        Some(other) => Err(bad(format!("{key} must be true or false, not {other:?}"))),
    }
}

/// A whole number in `range`, or `default` when absent.
fn number(
    config: &DriveConfig,
    key: &str,
    default: u64,
    range: std::ops::RangeInclusive<u64>,
) -> Result<u64, ConnectError> {
    match param(config, key) {
        None => Ok(default),
        Some(text) => match text.parse::<u64>() {
            Ok(value) if range.contains(&value) => Ok(value),
            _ => Err(bad(format!(
                "{key} must be a whole number between {} and {}",
                range.start(),
                range.end()
            ))),
        },
    }
}

/// `~` and `~/…` against `$HOME`.
#[cfg(feature = "gcs")]
pub(crate) fn expand_home(raw: &str) -> Result<PathBuf, ConnectError> {
    if raw == "~" || raw.starts_with("~/") {
        let home = std::env::home_dir()
            .ok_or_else(|| bad(String::from("no home directory to expand ~ against")))?;
        let rest = raw.trim_start_matches('~').trim_start_matches('/');
        return Ok(if rest.is_empty() { home } else { home.join(rest) });
    }
    Ok(PathBuf::from(raw))
}

/// The tuning every object-store drive accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tuning {
    /// `timeout_s`: per request (default 30).
    pub timeout: Duration,
    /// `part_size_mb`: upload part size in MiB (5..=512, default 8).
    pub part_size: usize,
    /// `upload_concurrency`: parts in flight per upload (1..=32, default 4).
    pub upload_concurrency: usize,
}

impl Tuning {
    pub(crate) fn from_config(config: &DriveConfig) -> Result<Tuning, ConnectError> {
        let timeout = number(config, "timeout_s", DEFAULT_TIMEOUT_S, 1..=3600)?;
        let default_mb = u64::try_from(DEFAULT_PART_SIZE / (1024 * 1024)).unwrap_or(8);
        let part_mb = number(config, "part_size_mb", default_mb, 5..=512)?;
        let concurrency = number(
            config,
            "upload_concurrency",
            u64::try_from(DEFAULT_UPLOAD_CONCURRENCY).unwrap_or(4),
            1..=32,
        )?;
        Ok(Tuning {
            timeout: Duration::from_secs(timeout),
            part_size: usize::try_from(part_mb).unwrap_or(8).saturating_mul(1024 * 1024),
            upload_concurrency: usize::try_from(concurrency).unwrap_or(4),
        })
    }

    /// HTTP client options: the whole request may take four times the
    /// timeout (a part upload on a slow link), a silent connection only the
    /// timeout, connecting at most 10 s.
    #[cfg(any(feature = "s3", feature = "gcs"))]
    pub(crate) fn client_options(&self, allow_http: bool) -> object_store::ClientOptions {
        object_store::ClientOptions::new()
            .with_timeout(self.timeout.saturating_mul(4))
            .with_read_timeout(self.timeout)
            .with_connect_timeout(self.timeout.min(Duration::from_secs(10)))
            .with_allow_http(allow_http)
    }

    /// Three retries with a short backoff, all within the timeout: a dead
    /// endpoint answers `Unavailable` in about a second, not minutes.
    #[cfg(any(feature = "s3", feature = "gcs"))]
    pub(crate) fn retry(&self) -> object_store::RetryConfig {
        object_store::RetryConfig {
            backoff: object_store::BackoffConfig {
                init_backoff: Duration::from_millis(100),
                max_backoff: Duration::from_secs(2),
                base: 2.0,
            },
            max_retries: 3,
            retry_timeout: self.timeout,
        }
    }
}

/// Checks an `endpoint` and the `allow_http` rule: plain `http://` only when
/// the user said so, and `allow_http` only makes sense for such an endpoint.
pub(crate) fn endpoint(config: &DriveConfig) -> Result<(Option<String>, bool), ConnectError> {
    let allow_http = flag(config, "allow_http")?.unwrap_or(false);
    let Some(endpoint) = param(config, "endpoint") else {
        if allow_http {
            return Err(bad(String::from(
                "allow_http only applies to an endpoint that starts with http://",
            )));
        }
        return Ok((None, false));
    };
    let lower = endpoint.to_ascii_lowercase();
    if lower.starts_with("http://") {
        if !allow_http {
            return Err(bad(format!(
                "the endpoint {endpoint} is plain http: set allow_http=true to accept an unencrypted connection"
            )));
        }
    } else if lower.starts_with("https://") {
        if allow_http {
            return Err(bad(String::from(
                "allow_http only applies to an endpoint that starts with http://",
            )));
        }
    } else {
        return Err(bad(format!(
            "the endpoint {endpoint} must start with https:// (or http:// with allow_http=true)"
        )));
    }
    Ok((Some(endpoint.trim_end_matches('/').to_owned()), allow_http))
}

/// The bucket parameter: required, one segment.
pub(crate) fn bucket(config: &DriveConfig) -> Result<String, ConnectError> {
    let bucket = param(config, "bucket").ok_or_else(|| bad(String::from("the drive has no bucket")))?;
    if bucket.contains('/') || bucket.chars().any(char::is_whitespace) {
        return Err(bad(format!("{bucket:?} is not a bucket name")));
    }
    Ok(bucket.to_owned())
}

/// Turns a failed first request into the reason the connection failed.
pub(crate) fn connect_error(fail: &Fail) -> ConnectError {
    let text = fail.to_string();
    match fail {
        Fail::Cancelled => return ConnectError::Cancelled,
        Fail::TimedOut => return ConnectError::Unreachable(text),
        Fail::Store(object_store::Error::Unauthenticated { .. }) => return ConnectError::AuthFailed,
        Fail::Store(object_store::Error::PermissionDenied { .. }) => {
            // 403 is both «wrong key» and «right key, no permission».
            let lower = text.to_ascii_lowercase();
            return if lower.contains("accessdenied") && !lower.contains("signature") {
                ConnectError::Other(String::from(
                    "the credentials were accepted but may not list this bucket or prefix",
                ))
            } else {
                ConnectError::AuthFailed
            };
        }
        Fail::Store(object_store::Error::NotFound { .. }) => {
            return ConnectError::Other(String::from("the bucket does not exist"));
        }
        _ => {}
    }
    match fail.kind() {
        BackendErrorKind::Unavailable => ConnectError::Unreachable(text),
        BackendErrorKind::Cancelled => ConnectError::Cancelled,
        _ => ConnectError::Other(text),
    }
}

/// The one request that proves the drive works: the first key under its
/// prefix. Honours `cancel`.
pub(crate) fn verify(backend: &Arc<ObjectStoreBackend>, cancel: &Cancel) -> Result<(), ConnectError> {
    if cancel.is_cancelled() {
        return Err(ConnectError::Cancelled);
    }
    match backend.probe(cancel) {
        Ok(()) => Ok(()),
        Err(fail) => Err(connect_error(&fail)),
    }
}
