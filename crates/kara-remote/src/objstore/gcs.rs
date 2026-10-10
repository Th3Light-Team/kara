//! `gcs` drives: Google Cloud Storage through `object_store::gcp`.
//!
//! Credentials are a service-account JSON key **file** (`service_account_file`,
//! only its path is stored in the settings, never its content) or the
//! application default credentials (`credentials = adc`). There is no secret
//! to ask for: a refused key is reported as a plain error, not as a password
//! prompt.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use kara_vfs::{Backend, Cancel};
use object_store::ObjectStore;
use object_store::gcp::GoogleCloudStorageBuilder;
use object_store::list::PaginatedListStore;

use super::backend::{ObjectStoreBackend, ObjectStoreOptions};
use super::connect::{Tuning, bucket, endpoint, expand_home, param, verify};
use crate::config::DriveConfig;
use crate::registry::{BackendFactory, ConnectError, PromptHandler};
use crate::secrets::Secret;

/// Where the credentials come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcsCredentials {
    /// A service-account JSON key file (`service_account_file`, `~/` expanded).
    ServiceAccountFile(PathBuf),
    /// Application default credentials: `GOOGLE_APPLICATION_CREDENTIALS`,
    /// then `~/.config/gcloud/application_default_credentials.json`, then the
    /// GCE metadata server.
    ApplicationDefault,
}

/// Everything needed to reach a `gcs` drive. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcsParams {
    pub bucket: String,
    pub prefix: String,
    pub credentials: GcsCredentials,
    /// Another JSON API base URL (an emulator such as fake-gcs-server).
    pub endpoint: Option<String>,
    pub allow_http: bool,
    pub tuning: Tuning,
}

fn bad(reason: String) -> ConnectError {
    ConnectError::Other(reason)
}

impl GcsParams {
    /// Reads and checks the parameters. `~/` in the key file path is
    /// expanded here, at connect time.
    pub fn from_config(config: &DriveConfig) -> Result<GcsParams, ConnectError> {
        let (endpoint, allow_http) = endpoint(config)?;
        let bucket = bucket(config)?;
        let prefix = param(config, "prefix").unwrap_or("").to_owned();
        super::keys::Keys::new(&prefix).map_err(bad)?;
        let file = param(config, "service_account_file");
        let mode = param(config, "credentials").map(str::to_ascii_lowercase);
        let credentials = match (file, mode.as_deref()) {
            (Some(path), None | Some("file")) => GcsCredentials::ServiceAccountFile(expand_home(path)?),
            (None, Some("adc")) => GcsCredentials::ApplicationDefault,
            (Some(_), Some("adc")) => {
                return Err(bad(String::from(
                    "choose either service_account_file or credentials=adc, not both",
                )));
            }
            (None, None | Some("file")) => {
                return Err(bad(String::from(
                    "the drive needs service_account_file (a JSON key file) or credentials=adc",
                )));
            }
            (_, Some(other)) => {
                return Err(bad(format!("credentials must be file or adc, not {other:?}")));
            }
        };
        Ok(GcsParams {
            bucket,
            prefix,
            credentials,
            endpoint,
            allow_http,
            tuning: Tuning::from_config(config)?,
        })
    }

    /// `gs://bucket/prefix`.
    #[must_use]
    pub fn label(&self) -> String {
        if self.prefix.is_empty() {
            format!("gs://{}", self.bucket)
        } else {
            format!("gs://{}/{}", self.bucket, self.prefix.trim_matches('/'))
        }
    }
}

/// The real client: `object_store::gcp::GoogleCloudStorage`.
fn build_store(params: &GcsParams) -> Result<Arc<object_store::gcp::GoogleCloudStorage>, ConnectError> {
    let mut builder = match &params.credentials {
        GcsCredentials::ApplicationDefault => GoogleCloudStorageBuilder::from_env(),
        GcsCredentials::ServiceAccountFile(path) => {
            // Checked here so a typo is reported as such, before the library
            // says something vaguer.
            if let Err(error) = std::fs::metadata(path) {
                return Err(bad(format!(
                    "the service account file {} cannot be read: {error}",
                    path.display()
                )));
            }
            GoogleCloudStorageBuilder::new().with_service_account_path(path.to_string_lossy())
        }
    };
    builder = builder
        .with_bucket_name(&params.bucket)
        .with_client_options(params.tuning.client_options(params.allow_http))
        .with_retry(params.tuning.retry());
    if let Some(endpoint) = &params.endpoint {
        builder = builder.with_base_url(endpoint);
    }
    builder.build().map(Arc::new).map_err(|error| {
        // A malformed key file: say which file, not what is in it.
        let text = error.to_string();
        let reason = if text.contains("Unable to decode") || text.to_ascii_lowercase().contains("decod") {
            String::from("the service account file is not a valid JSON key")
        } else {
            text
        };
        bad(format!("the GCS drive cannot be set up: {reason}"))
    })
}

/// What builds the store from the parameters; replaceable in tests.
pub type GcsConnector = Arc<
    dyn Fn(&GcsParams) -> Result<(Arc<dyn ObjectStore>, Option<Arc<dyn PaginatedListStore>>), ConnectError>
        + Send
        + Sync,
>;

/// Opens `gcs` drives.
pub struct GcsFactory {
    connector: GcsConnector,
}

impl fmt::Debug for GcsFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcsFactory").finish_non_exhaustive()
    }
}

impl GcsFactory {
    /// The factory kara-ui registers.
    #[must_use]
    pub fn new() -> Arc<GcsFactory> {
        Arc::new(GcsFactory {
            connector: Arc::new(|params| {
                let store = build_store(params)?;
                let pager: Arc<dyn PaginatedListStore> = Arc::clone(&store) as _;
                Ok((store as Arc<dyn ObjectStore>, Some(pager)))
            }),
        })
    }

    /// A factory whose store comes from `connector`, for tests.
    #[doc(hidden)]
    #[must_use]
    pub fn with_connector(connector: GcsConnector) -> Arc<GcsFactory> {
        Arc::new(GcsFactory { connector })
    }

    /// Like [`BackendFactory::connect`], keeping the concrete type.
    pub fn open(&self, config: &DriveConfig, cancel: &Cancel) -> Result<Arc<ObjectStoreBackend>, ConnectError> {
        if cancel.is_cancelled() {
            return Err(ConnectError::Cancelled);
        }
        let params = GcsParams::from_config(config)?;
        let (store, pager) = (self.connector)(&params)?;
        let options = ObjectStoreOptions {
            prefix: params.prefix.clone(),
            pager,
            timeout: params.tuning.timeout,
            part_size: params.tuning.part_size,
            upload_concurrency: params.tuning.upload_concurrency,
            label: params.label(),
            ..ObjectStoreOptions::default()
        };
        let backend = Arc::new(ObjectStoreBackend::new(store, options).map_err(bad)?);
        // Nothing to prompt for: a refused key is the key file's problem.
        verify(&backend, cancel).map_err(|error| match error {
            ConnectError::AuthFailed | ConnectError::AuthRequired => bad(String::from(
                "Google Cloud Storage refused the credentials",
            )),
            other => other,
        })?;
        Ok(backend)
    }
}

impl BackendFactory for GcsFactory {
    fn scheme(&self) -> &str {
        "gcs"
    }

    /// The secret is ignored: GCS drives have none.
    fn connect(
        &self,
        config: &DriveConfig,
        _secret: Option<&Secret>,
        _prompts: &dyn PromptHandler,
        cancel: &Cancel,
    ) -> Result<Arc<dyn Backend>, ConnectError> {
        let backend: Arc<dyn Backend> = self.open(config, cancel)?;
        Ok(backend)
    }
}
