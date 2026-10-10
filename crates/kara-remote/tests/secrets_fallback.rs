//! `FallbackSecretStore`: a missing keyring must not stop a drive from connecting.

use kara_remote::{FallbackSecretStore, MemorySecretStore, Secret, SecretError, SecretKey, SecretStore};
use kara_vfs::DriveId;
use std::sync::Arc;

fn key(name: &str) -> SecretKey {
    SecretKey::Drive(DriveId::new("sftp", name).expect("id"))
}

/// A keyring that is always unreachable.
struct Broken;

impl SecretStore for Broken {
    fn get(&self, _: &SecretKey) -> Result<Option<Secret>, SecretError> {
        Err(SecretError::Unavailable(String::from("no bus")))
    }
    fn set(&self, _: &SecretKey, _: &Secret) -> Result<(), SecretError> {
        Err(SecretError::Unavailable(String::from("no bus")))
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Err(SecretError::Unavailable(String::from("no bus")))
    }
    fn is_persistent(&self) -> bool {
        true
    }
}

/// Forwards to a shared memory store, claiming to be persistent.
struct Shared(Arc<MemorySecretStore>);

impl SecretStore for Shared {
    fn get(&self, k: &SecretKey) -> Result<Option<Secret>, SecretError> {
        self.0.get(k)
    }
    fn set(&self, k: &SecretKey, s: &Secret) -> Result<(), SecretError> {
        self.0.set(k, s)
    }
    fn delete(&self, k: &SecretKey) -> Result<(), SecretError> {
        self.0.delete(k)
    }
    fn is_persistent(&self) -> bool {
        true
    }
}

#[test]
fn a_working_keyring_is_used_and_stays_persistent() {
    let inner = Arc::new(MemorySecretStore::new());
    let store = FallbackSecretStore::new(Box::new(Shared(inner.clone())));
    store.set(&key("a"), &Secret::new("pw")).expect("set");
    assert_eq!(inner.get(&key("a")).expect("get").expect("stored").expose(), "pw");
    assert!(store.is_persistent());
    assert_eq!(store.get(&key("a")).expect("get").expect("found").expose(), "pw");
}

#[test]
fn a_broken_keyring_falls_back_to_memory_and_says_so() {
    let store = FallbackSecretStore::new(Box::new(Broken));
    assert!(store.is_persistent(), "nothing has failed yet");
    store.set(&key("a"), &Secret::new("pw")).expect("falls back, no error");
    assert!(!store.is_persistent());
    assert_eq!(store.get(&key("a")).expect("get").expect("found").expose(), "pw");
    store.delete(&key("a")).expect("delete");
    assert!(store.get(&key("a")).expect("get").is_none());
}

#[test]
fn a_read_failure_degrades_too() {
    let store = FallbackSecretStore::new(Box::new(Broken));
    assert!(store.get(&key("a")).expect("get").is_none());
    assert!(!store.is_persistent());
}

#[test]
fn a_secret_stored_in_the_keyring_replaces_a_session_copy() {
    let inner = Arc::new(MemorySecretStore::new());
    let store = FallbackSecretStore::new(Box::new(Shared(inner.clone())));
    store.set(&key("a"), &Secret::new("one")).expect("set");
    store.delete(&key("a")).expect("delete");
    assert!(inner.get(&key("a")).expect("get").is_none());
    assert!(store.get(&key("a")).expect("get").is_none());
}
