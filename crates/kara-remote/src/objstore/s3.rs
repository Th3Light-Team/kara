//! `s3` drives: Amazon S3 and S3-compatible services (MinIO, Cloudflare R2,
//! Backblaze B2, Ceph RGW) through `object_store::aws`.

use std::fmt;
use std::sync::Arc;

use kara_vfs::{Backend, Cancel};
use object_store::ObjectStore;
use object_store::aws::{AmazonS3Builder, AmazonS3ConfigKey};
use object_store::list::PaginatedListStore;

use super::backend::{ObjectStoreBackend, ObjectStoreOptions};
use super::connect::{Tuning, bucket, endpoint, flag, param, verify};
use crate::config::DriveConfig;
use crate::registry::{BackendFactory, ConnectError, PromptHandler};
use crate::secrets::Secret;

/// S3's `CopyObject` copies at most 5 GiB; bigger objects are streamed.
pub const S3_MAX_SERVER_COPY: u64 = 5 * 1024 * 1024 * 1024;

/// Where the credentials come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S3Credentials {
    /// `access_key_id` plus the secret access key from the secret store
    /// (the default).
    Key,
    /// The ambient AWS chain as `object_store` reads it: `AWS_*` environment
    /// variables, web identity, ECS task role, EC2 instance metadata.
    /// SSO and named profiles (`~/.aws/config`) are **not** supported.
    Env,
    /// No signature at all: public buckets.
    Anonymous,
}

/// Everything needed to reach an `s3` drive. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Params {
    /// `None`: AWS itself.
    pub endpoint: Option<String>,
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    pub access_key_id: Option<String>,
    pub credentials: S3Credentials,
    /// Path-style URLs (`endpoint/bucket/key`) instead of virtual-hosted ones.
    pub path_style: bool,
    pub allow_http: bool,
    pub tuning: Tuning,
    /// Passed to `object_store` as `aws_conditional_put` (`etag`, `disabled`).
    pub conditional_put: Option<String>,
    /// Passed as `aws_copy_if_not_exists` (`multipart`, `header: <k>: <v>`, ...).
    pub copy_if_not_exists: Option<String>,
}

fn bad(reason: String) -> ConnectError {
    ConnectError::Other(reason)
}

impl S3Params {
    /// Reads and checks the parameters.
    pub fn from_config(config: &DriveConfig) -> Result<S3Params, ConnectError> {
        let (endpoint, allow_http) = endpoint(config)?;
        let bucket = bucket(config)?;
        let prefix = param(config, "prefix").unwrap_or("").to_owned();
        super::keys::Keys::new(&prefix).map_err(bad)?;
        let region = param(config, "region").unwrap_or("us-east-1").to_owned();
        let credentials = match param(config, "credentials").map(str::to_ascii_lowercase).as_deref() {
            None | Some("key") => S3Credentials::Key,
            Some("env") => S3Credentials::Env,
            Some("anonymous") => S3Credentials::Anonymous,
            Some(other) => {
                return Err(bad(format!(
                    "credentials must be key, env or anonymous, not {other:?}"
                )));
            }
        };
        let access_key_id = param(config, "access_key_id").map(str::to_owned);
        if credentials == S3Credentials::Key && access_key_id.is_none() {
            return Err(bad(String::from(
                "the drive has no access_key_id (set credentials=env to use the AWS environment)",
            )));
        }
        let path_style = flag(config, "path_style")?.unwrap_or(endpoint.is_some());
        Ok(S3Params {
            endpoint,
            bucket,
            prefix,
            region,
            access_key_id,
            credentials,
            path_style,
            allow_http,
            tuning: Tuning::from_config(config)?,
            conditional_put: param(config, "conditional_put").map(str::to_owned),
            copy_if_not_exists: param(config, "copy_if_not_exists").map(str::to_owned),
        })
    }

    /// `s3://bucket/prefix` (with the endpoint when it is not AWS).
    #[must_use]
    pub fn label(&self) -> String {
        let place = if self.prefix.is_empty() {
            self.bucket.clone()
        } else {
            format!("{}/{}", self.bucket, self.prefix.trim_matches('/'))
        };
        match &self.endpoint {
            Some(endpoint) => format!("s3://{place} at {endpoint}"),
            None => format!("s3://{place}"),
        }
    }
}

/// The secret of an `s3` drive is the secret access key, optionally followed
/// by a line break and a session token (temporary STS credentials).
fn split_secret(secret: &Secret) -> (&str, Option<&str>) {
    let text = secret.expose();
    match text.split_once('\n') {
        Some((key, token)) => {
            let token = token.trim();
            (key.trim_end_matches('\r'), (!token.is_empty()).then_some(token))
        }
        None => (text, None),
    }
}

/// The real client: `object_store::aws::AmazonS3`.
fn build_store(
    params: &S3Params,
    secret: Option<&Secret>,
) -> Result<Arc<object_store::aws::AmazonS3>, ConnectError> {
    let mut builder = match params.credentials {
        S3Credentials::Env => AmazonS3Builder::from_env(),
        S3Credentials::Key | S3Credentials::Anonymous => AmazonS3Builder::new(),
    };
    builder = builder
        .with_bucket_name(&params.bucket)
        .with_region(&params.region)
        .with_virtual_hosted_style_request(!params.path_style)
        .with_client_options(params.tuning.client_options(params.allow_http))
        .with_retry(params.tuning.retry());
    if let Some(endpoint) = &params.endpoint {
        builder = builder.with_endpoint(endpoint);
    }
    match params.credentials {
        S3Credentials::Key => {
            let Some(secret) = secret else {
                return Err(ConnectError::AuthRequired);
            };
            let (secret_key, token) = split_secret(secret);
            if secret_key.is_empty() {
                return Err(ConnectError::AuthRequired);
            }
            builder = builder
                .with_access_key_id(params.access_key_id.clone().unwrap_or_default())
                .with_secret_access_key(secret_key);
            if let Some(token) = token {
                builder = builder.with_token(token);
            }
        }
        S3Credentials::Anonymous => builder = builder.with_skip_signature(true),
        S3Credentials::Env => {}
    }
    if let Some(value) = &params.conditional_put {
        builder = builder.with_config(AmazonS3ConfigKey::ConditionalPut, value);
    }
    if let Some(value) = &params.copy_if_not_exists {
        builder = builder.with_config(AmazonS3ConfigKey::CopyIfNotExists, value);
    }
    // The builder's own errors never carry the secret, but say nothing of
    // theirs anyway: the parameters are what the user can fix.
    builder
        .build()
        .map(Arc::new)
        .map_err(|error| bad(format!("the S3 drive cannot be set up: {}", redact(&error.to_string(), secret))))
}

/// Removes the secret (and its parts) from a text, in case a library echoes it.
fn redact(text: &str, secret: Option<&Secret>) -> String {
    let mut out = text.to_owned();
    if let Some(secret) = secret {
        let (key, token) = split_secret(secret);
        for part in [Some(secret.expose()), Some(key), token].into_iter().flatten() {
            if !part.is_empty() {
                out = out.replace(part, "<redacted>");
            }
        }
    }
    out
}

/// What builds the store from the parameters; replaceable in tests.
pub type S3Connector = Arc<
    dyn Fn(&S3Params, Option<&Secret>) -> Result<(Arc<dyn ObjectStore>, Option<Arc<dyn PaginatedListStore>>), ConnectError>
        + Send
        + Sync,
>;

/// Opens `s3` drives.
pub struct S3Factory {
    connector: S3Connector,
}

impl fmt::Debug for S3Factory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Factory").finish_non_exhaustive()
    }
}

impl S3Factory {
    /// The factory kara-ui registers.
    #[must_use]
    pub fn new() -> Arc<S3Factory> {
        Arc::new(S3Factory {
            connector: Arc::new(|params, secret| {
                let store = build_store(params, secret)?;
                let pager: Arc<dyn PaginatedListStore> = Arc::clone(&store) as _;
                Ok((store as Arc<dyn ObjectStore>, Some(pager)))
            }),
        })
    }

    /// A factory whose store comes from `connector`: for tests that put a
    /// fake store behind the real parameter, secret and connect handling.
    #[doc(hidden)]
    #[must_use]
    pub fn with_connector(connector: S3Connector) -> Arc<S3Factory> {
        Arc::new(S3Factory { connector })
    }

    /// Like [`BackendFactory::connect`], keeping the concrete type.
    pub fn open(
        &self,
        config: &DriveConfig,
        secret: Option<&Secret>,
        cancel: &Cancel,
    ) -> Result<Arc<ObjectStoreBackend>, ConnectError> {
        if cancel.is_cancelled() {
            return Err(ConnectError::Cancelled);
        }
        let params = S3Params::from_config(config)?;
        if params.credentials == S3Credentials::Key && secret.is_none() {
            return Err(ConnectError::AuthRequired);
        }
        let (store, pager) = (self.connector)(&params, secret)?;
        let options = ObjectStoreOptions {
            prefix: params.prefix.clone(),
            pager,
            timeout: params.tuning.timeout,
            part_size: params.tuning.part_size,
            upload_concurrency: params.tuning.upload_concurrency,
            max_server_copy: Some(S3_MAX_SERVER_COPY),
            label: params.label(),
            ..ObjectStoreOptions::default()
        };
        let backend = Arc::new(ObjectStoreBackend::new(store, options).map_err(bad)?);
        verify(&backend, cancel).map_err(|error| match (params.credentials, error) {
            // Only a key from the secret store is worth asking for again.
            (S3Credentials::Env | S3Credentials::Anonymous, ConnectError::AuthFailed) => bad(
                String::from("the service refused the credentials of the environment"),
            ),
            (_, error) => error,
        })?;
        Ok(backend)
    }
}

impl BackendFactory for S3Factory {
    fn scheme(&self) -> &str {
        "s3"
    }

    fn connect(
        &self,
        config: &DriveConfig,
        secret: Option<&Secret>,
        _prompts: &dyn PromptHandler,
        cancel: &Cancel,
    ) -> Result<Arc<dyn Backend>, ConnectError> {
        let backend: Arc<dyn Backend> = self.open(config, secret, cancel)?;
        Ok(backend)
    }
}
