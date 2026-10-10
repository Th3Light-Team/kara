//! Where a drive's password or passphrase lives.
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

/// A place to keep one secret per drive.
pub trait SecretStore: Send + Sync {
    /// `Ok(None)` when nothing is stored for the drive.
    fn get(&self, id: &DriveId) -> Result<Option<Secret>, SecretError>;
    /// Stores or replaces the drive's secret.
    fn set(&self, id: &DriveId, secret: &Secret) -> Result<(), SecretError>;
    /// Removes it. Removing what is not there is not an error.
    fn delete(&self, id: &DriveId) -> Result<(), SecretError>;
    /// `false` when secrets only last for this run (nothing is saved to disk).
    fn is_persistent(&self) -> bool;
}

/// Keeps secrets in memory for the life of the process. The fallback when there
/// is no keyring, and the store used by tests.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    secrets: Mutex<HashMap<DriveId, Secret>>,
}

impl MemorySecretStore {
    #[must_use]
    pub fn new() -> MemorySecretStore {
        MemorySecretStore::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<DriveId, Secret>> {
        match self.secrets.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, id: &DriveId) -> Result<Option<Secret>, SecretError> {
        Ok(self.lock().get(id).cloned())
    }

    fn set(&self, id: &DriveId, secret: &Secret) -> Result<(), SecretError> {
        self.lock().insert(id.clone(), secret.clone());
        Ok(())
    }

    fn delete(&self, id: &DriveId) -> Result<(), SecretError> {
        self.lock().remove(id);
        Ok(())
    }

    fn is_persistent(&self) -> bool {
        false
    }
}
