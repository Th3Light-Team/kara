//! Where a drive's (or a group's) password or passphrase lives.
//!
//! In production that is the system keyring (Secret Service), behind
//! [`SecretStore`]. If no keyring is available the drive still connects: the
//! secret is asked each time and [`SecretStore::is_persistent`] tells the UI to
//! say so. Nothing in this crate ever writes a secret to `settings.conf`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use kara_vfs::DriveId;

/// A password or passphrase. `Debug` never prints it, and there is no `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// The secret itself. Call it only where it is handed to the protocol.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Why the store could not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// No keyring service to talk to (no session bus, locked and not unlockable).
    #[error("no secret service is available: {0}")]
    Unavailable(String),
    #[error("the secret service refused the operation: {0}")]
    Denied(String),
}

/// What a secret belongs to: one drive, or a whole group of them (a fleet that
/// shares one password or key passphrase).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SecretKey {
    Drive(DriveId),
    Group(String),
}

impl SecretKey {
    /// Stable text form, used for keyring attributes: `sftp://nas` or `group:fleet`.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            SecretKey::Drive(id) => format!("{}://{}", id.scheme(), id.name()),
            SecretKey::Group(name) => format!("group:{name}"),
        }
    }
}

/// A place to keep secrets, one per [`SecretKey`].
pub trait SecretStore: Send + Sync {
    /// `Ok(None)` when nothing is stored for the key.
    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError>;
    /// Stores or replaces the secret.
    fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretError>;
    /// Removes it. Removing what is not there is not an error.
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError>;
    /// `false` when secrets only last for this run (nothing is saved to disk).
    fn is_persistent(&self) -> bool;
}

/// Keeps secrets in memory for the life of the process. The fallback when there
/// is no keyring, and the store used by tests.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    secrets: Mutex<HashMap<SecretKey, Secret>>,
}

impl MemorySecretStore {
    #[must_use]
    pub fn new() -> MemorySecretStore {
        MemorySecretStore::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SecretKey, Secret>> {
        match self.secrets.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
        Ok(self.lock().get(key).cloned())
    }

    fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretError> {
        self.lock().insert(key.clone(), secret.clone());
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.lock().remove(key);
        Ok(())
    }

    fn is_persistent(&self) -> bool {
        false
    }
}
