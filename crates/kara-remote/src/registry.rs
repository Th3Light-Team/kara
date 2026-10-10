//! The set of configured drives and what state each one is in.
//!
//! The registry never draws a dialog and never blocks the UI thread by itself:
//! [`DriveRegistry::connect`] is a blocking call meant for a worker thread, and
//! anything that needs the user (a password, an unknown host key) goes out as a
//! [`Prompt`] through a [`PromptHandler`] that the UI answers.
//!
//! `kara-ops` gets its backends from [`DriveRegistry::resolver`], which only
//! returns a backend while the drive is [`ConnectionState::Ready`]; a drive that
//! was lost resolves to `None`, so a job fails fast with "drive unavailable"
//! instead of hanging on a dead session.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use kara_vfs::{Backend, BackendError, BackendErrorKind, Cancel, DriveId};

use crate::config::DriveConfig;
use crate::secrets::{Secret, SecretStore};

/// Same shape as `kara_ops::BackendResolver`, so the two are interchangeable
/// without this crate depending on `kara-ops`.
pub type Resolver = Arc<dyn Fn(&DriveId) -> Option<Arc<dyn Backend>> + Send + Sync>;

type Listener = Arc<dyn Fn(&DriveId, &ConnectionState) + Send + Sync>;

/// Where a drive is in its life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Ready,
    /// Was connected, the session died. Reconnecting is up to the user.
    Lost { reason: String },
    /// The last attempt did not succeed.
    Failed { reason: String },
}

/// A question for the user, asked in the middle of connecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    /// The drive needs a password or passphrase.
    Password { drive: DriveId, label: String },
    /// First contact with a host: trust this key?
    TrustHostKey {
        drive: DriveId,
        host: String,
        fingerprint: String,
    },
    /// The host presented a different key than the one on record. Never trusted
    /// silently: the default answer must be a refusal.
    HostKeyChanged {
        drive: DriveId,
        host: String,
        known: String,
        presented: String,
    },
}

/// The user's reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptAnswer {
    /// A password, and whether to keep it in the secret store.
    Secret { secret: Secret, remember: bool },
    /// Accept the host key, and whether to remember it.
    Trust { remember: bool },
    Refuse,
}

/// Whoever can ask the user a question. Called on the connecting worker thread
/// and expected to block until the user answers.
pub trait PromptHandler: Send + Sync {
    fn ask(&self, prompt: &Prompt) -> PromptAnswer;
}

/// A handler for headless callers: refuses everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct RefuseAll;

impl PromptHandler for RefuseAll {
    fn ask(&self, _prompt: &Prompt) -> PromptAnswer {
        PromptAnswer::Refuse
    }
}

/// Why a connection attempt ended without a backend.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectError {
    /// A secret is needed and none was given.
    #[error("authentication is required")]
    AuthRequired,
    /// The server rejected the secret that was given.
    #[error("authentication failed")]
    AuthFailed,
    #[error("the host cannot be reached: {0}")]
    Unreachable(String),
    #[error("the host key was refused")]
    HostKeyRefused,
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

/// Opens drives of one kind. One factory per protocol.
pub trait BackendFactory: Send + Sync {
    /// The URI scheme this factory serves, e.g. `"sftp"`.
    fn scheme(&self) -> &str;

    /// Connects. Blocking. May ask through `prompts` (host keys); asking for the
    /// password is the registry's job: return [`ConnectError::AuthRequired`] and
    /// it will ask and call again with the secret.
    fn connect(
        &self,
        config: &DriveConfig,
        secret: Option<&Secret>,
        prompts: &dyn PromptHandler,
        cancel: &Cancel,
    ) -> Result<Arc<dyn Backend>, ConnectError>;
}

/// Why the registry refused an operation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("no factory is registered for the scheme {0:?}")]
    UnsupportedScheme(String),
    #[error("a drive with that identity already exists")]
    AlreadyExists,
    #[error("no such drive")]
    Unknown,
    #[error("the drive is already connecting")]
    Busy,
}

struct Entry {
    config: DriveConfig,
    state: ConnectionState,
    backend: Option<Arc<dyn Backend>>,
}

#[derive(Default)]
struct Inner {
    factories: BTreeMap<String, Arc<dyn BackendFactory>>,
    drives: BTreeMap<DriveId, Entry>,
    listeners: Vec<Listener>,
}

/// How many times a wrong or missing secret is asked for before giving up.
const SECRET_ATTEMPTS: usize = 3;

/// All configured drives. Cheap to share: clone the `Arc`.
pub struct DriveRegistry {
    secrets: Arc<dyn SecretStore>,
    inner: Mutex<Inner>,
}

impl DriveRegistry {
    #[must_use]
    pub fn new(secrets: Arc<dyn SecretStore>) -> Arc<DriveRegistry> {
        Arc::new(DriveRegistry {
            secrets,
            inner: Mutex::new(Inner::default()),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Whether secrets survive this run, for the UI to say so.
    #[must_use]
    pub fn secrets_are_persistent(&self) -> bool {
        self.secrets.is_persistent()
    }

    pub fn register_factory(&self, factory: Arc<dyn BackendFactory>) {
        self.lock()
            .factories
            .insert(factory.scheme().to_owned(), factory);
    }

    /// Calls `listener` after every state change, outside the registry's lock.
    pub fn subscribe(&self, listener: impl Fn(&DriveId, &ConnectionState) + Send + Sync + 'static) {
        self.lock().listeners.push(Arc::new(listener));
    }

    /// Adds a drive, disconnected. Persisting its config is the caller's job
    /// (`config::store`).
    pub fn add(&self, config: DriveConfig) -> Result<(), RegistryError> {
        let mut inner = self.lock();
        if !inner.factories.contains_key(config.id.scheme()) {
            return Err(RegistryError::UnsupportedScheme(
                config.id.scheme().to_owned(),
            ));
        }
        if inner.drives.contains_key(&config.id) {
            return Err(RegistryError::AlreadyExists);
        }
        inner.drives.insert(
            config.id.clone(),
            Entry {
                config,
                state: ConnectionState::Disconnected,
                backend: None,
            },
        );
        Ok(())
    }

    /// Adds every config whose scheme has a factory; returns the ones that could
    /// not be added with the reason.
    pub fn add_all(
        &self,
        configs: impl IntoIterator<Item = DriveConfig>,
    ) -> Vec<(DriveId, RegistryError)> {
        configs
            .into_iter()
            .filter_map(|config| {
                let id = config.id.clone();
                self.add(config).err().map(|error| (id, error))
            })
            .collect()
    }

    /// Forgets a drive: drops its session and deletes its stored secret.
    pub fn remove(&self, id: &DriveId) -> Result<(), RegistryError> {
        let removed = self.lock().drives.remove(id);
        let Some(_) = removed else {
            return Err(RegistryError::Unknown);
        };
        // Best effort: a keyring that is gone must not keep the drive in the list.
        let _ = self.secrets.delete(id);
        Ok(())
    }

    /// Stores a secret for a drive, e.g. when the user fills the "add drive" form.
    pub fn remember_secret(&self, id: &DriveId, secret: &Secret) -> Result<(), crate::SecretError> {
        self.secrets.set(id, secret)
    }

    #[must_use]
    pub fn configs(&self) -> Vec<DriveConfig> {
        self.lock()
            .drives
            .values()
            .map(|entry| entry.config.clone())
            .collect()
    }

    #[must_use]
    pub fn config(&self, id: &DriveId) -> Option<DriveConfig> {
        self.lock().drives.get(id).map(|entry| entry.config.clone())
    }

    #[must_use]
    pub fn state(&self, id: &DriveId) -> Option<ConnectionState> {
        self.lock().drives.get(id).map(|entry| entry.state.clone())
    }

    /// The live backend, or `None` unless the drive is `Ready`.
    #[must_use]
    pub fn backend(&self, id: &DriveId) -> Option<Arc<dyn Backend>> {
        let inner = self.lock();
        let entry = inner.drives.get(id)?;
        if entry.state == ConnectionState::Ready {
            entry.backend.clone()
        } else {
            None
        }
    }

    /// The resolver `kara-ops` takes. Holds only a weak reference's worth of
    /// state: it asks the registry each time, so a lost drive stops resolving.
    #[must_use]
    pub fn resolver(self: &Arc<Self>) -> Resolver {
        let registry = Arc::clone(self);
        Arc::new(move |id| registry.backend(id))
    }

    fn set_state(&self, id: &DriveId, state: ConnectionState, backend: Option<Arc<dyn Backend>>) {
        let listeners = {
            let mut inner = self.lock();
            let Some(entry) = inner.drives.get_mut(id) else {
                return;
            };
            entry.state = state.clone();
            entry.backend = backend;
            inner.listeners.clone()
        };
        for listener in listeners {
            listener(id, &state);
        }
    }

    /// Connects `id`. Blocking: call it from a worker thread.
    ///
    /// Asks for a password through `prompts` when the factory says one is needed
    /// (at most [`SECRET_ATTEMPTS`] times) and, if the user agrees, hands it to the
    /// secret store only after the connection worked.
    pub fn connect(
        &self,
        id: &DriveId,
        prompts: &dyn PromptHandler,
        cancel: &Cancel,
    ) -> Result<(), ConnectOrRegistryError> {
        let (config, factory) = {
            let inner = self.lock();
            let Some(entry) = inner.drives.get(id) else {
                return Err(RegistryError::Unknown.into());
            };
            if entry.state == ConnectionState::Connecting {
                return Err(RegistryError::Busy.into());
            }
            let config = entry.config.clone();
            let Some(factory) = inner.factories.get(id.scheme()).cloned() else {
                return Err(RegistryError::UnsupportedScheme(id.scheme().to_owned()).into());
            };
            (config, factory)
        };
        self.set_state(id, ConnectionState::Connecting, None);

        // A missing or unreadable keyring is not fatal: the secret is asked for.
        let mut secret = self.secrets.get(id).ok().flatten();
        let mut remember = false;
        for attempt in 0..SECRET_ATTEMPTS {
            if cancel.is_cancelled() {
                return Err(self.fail(id, ConnectError::Cancelled));
            }
            match factory.connect(&config, secret.as_ref(), prompts, cancel) {
                Ok(backend) => {
                    if remember && let Some(secret) = &secret {
                        // Only a secret that worked is worth keeping.
                        let _ = self.secrets.set(id, secret);
                    }
                    self.set_state(id, ConnectionState::Ready, Some(backend));
                    return Ok(());
                }
                Err(ConnectError::AuthRequired | ConnectError::AuthFailed)
                    if attempt + 1 < SECRET_ATTEMPTS =>
                {
                    let prompt = Prompt::Password {
                        drive: id.clone(),
                        label: config.label.clone(),
                    };
                    match prompts.ask(&prompt) {
                        PromptAnswer::Secret {
                            secret: given,
                            remember: keep,
                        } => {
                            secret = Some(given);
                            remember = keep;
                        }
                        PromptAnswer::Refuse | PromptAnswer::Trust { .. } => {
                            return Err(self.fail(id, ConnectError::AuthRequired));
                        }
                    }
                }
                Err(error) => return Err(self.fail(id, error)),
            }
        }
        Err(self.fail(id, ConnectError::AuthFailed))
    }

    fn fail(&self, id: &DriveId, error: ConnectError) -> ConnectOrRegistryError {
        self.set_state(
            id,
            ConnectionState::Failed {
                reason: error.to_string(),
            },
            None,
        );
        error.into()
    }

    /// Drops the session. The drive stays configured.
    pub fn disconnect(&self, id: &DriveId) -> Result<(), RegistryError> {
        if self.lock().drives.contains_key(id) {
            self.set_state(id, ConnectionState::Disconnected, None);
            Ok(())
        } else {
            Err(RegistryError::Unknown)
        }
    }

    /// Tells the registry an operation on `id` failed. A lost connection marks
    /// the drive `Lost` and stops it resolving; any other error changes nothing.
    /// Returns whether the drive was marked lost.
    pub fn report_failure(&self, id: &DriveId, error: &BackendError) -> bool {
        if error.kind != BackendErrorKind::Unavailable {
            return false;
        }
        if self.state(id) != Some(ConnectionState::Ready) {
            return false;
        }
        self.set_state(
            id,
            ConnectionState::Lost {
                reason: error.to_string(),
            },
            None,
        );
        true
    }
}

/// [`DriveRegistry::connect`] can fail before or during the attempt.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectOrRegistryError {
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Connect(#[from] ConnectError),
}
