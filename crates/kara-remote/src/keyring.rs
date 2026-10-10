//! [`SecretStore`] on the system keyring, through `oo7`.
//!
//! `oo7` talks to the Secret Service over D-Bus (GNOME Keyring, KWallet) and
//! falls back to the portal-managed file keyring in a sandbox. Its API is async;
//! the store keeps a private current-thread runtime and blocks on it, which is
//! fine because every caller is already on a worker thread.
//!
//! **Not exercised by the test suite**: it needs a session bus and an unlocked
//! keyring, which CI and the cloud container do not have. It is checked by
//! `docs/remote-drives-handoff.md`, step «keyring smoke test».

use oo7::{Keyring, Secret as KeyringSecret};

use crate::secrets::{Secret, SecretError, SecretKey, SecretStore};

const APPLICATION: &str = "kara";

/// One keyring item per drive, found by its attributes.
pub struct KeyringSecretStore {
    runtime: tokio::runtime::Runtime,
}

impl KeyringSecretStore {
    /// Fails only if the private runtime cannot be built; whether a keyring is
    /// actually reachable shows on the first call.
    pub fn open() -> Result<KeyringSecretStore, SecretError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| SecretError::Unavailable(error.to_string()))?;
        Ok(KeyringSecretStore { runtime })
    }
}

fn attributes(key: &SecretKey) -> [(&'static str, String); 2] {
    [
        ("application", APPLICATION.to_owned()),
        ("kara.drive", key.label()),
    ]
}

fn unavailable(error: &oo7::Error) -> SecretError {
    SecretError::Unavailable(error.to_string())
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
        self.runtime.block_on(async {
            let keyring = Keyring::new().await.map_err(|e| unavailable(&e))?;
            let items = keyring
                .search_items(&attributes(key))
                .await
                .map_err(|e| unavailable(&e))?;
            let Some(item) = items.first() else {
                return Ok(None);
            };
            let secret = item.secret().await.map_err(|e| unavailable(&e))?;
            let text = String::from_utf8(secret.as_bytes().to_vec())
                .map_err(|_| SecretError::Denied(String::from("the stored secret is not text")))?;
            Ok(Some(Secret::new(text)))
        })
    }

    fn set(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretError> {
        self.runtime.block_on(async {
            let keyring = Keyring::new().await.map_err(|e| unavailable(&e))?;
            let label = format!("Kara {}", key.label());
            keyring
                .create_item(
                    &label,
                    &attributes(key),
                    KeyringSecret::text(secret.expose()),
                    true,
                )
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.runtime.block_on(async {
            let keyring = Keyring::new().await.map_err(|e| unavailable(&e))?;
            keyring
                .delete(&attributes(key))
                .await
                .map_err(|e| unavailable(&e))
        })
    }

    fn is_persistent(&self) -> bool {
        true
    }
}
