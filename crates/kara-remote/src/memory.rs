//! A drive kind `mem` backed by [`MemoryBackend`], for tests and demos.
//!
//! Parameters:
//! - `profile`: `posix` (default) or `object`;
//! - `auth`: `required` to demand the secret [`MemoryFactory::ACCEPTED`];
//! - `unreachable`: `true` to fail with [`ConnectError::Unreachable`].
//!
//! Reconnecting a drive returns the same backend, so its data survives a
//! disconnect/connect cycle the way a real server's would.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use kara_vfs::memory::MemoryBackend;
use kara_vfs::{Backend, Cancel};

use crate::config::DriveConfig;
use crate::registry::{BackendFactory, ConnectError, PromptHandler};
use crate::secrets::Secret;

/// Serves `mem` drives.
#[derive(Default)]
pub struct MemoryFactory {
    backends: Mutex<BTreeMap<String, Arc<MemoryBackend>>>,
}

impl MemoryFactory {
    /// The only secret an `auth=required` drive accepts.
    pub const ACCEPTED: &'static str = "open-sesame";

    #[must_use]
    pub fn new() -> Arc<MemoryFactory> {
        Arc::new(MemoryFactory::default())
    }

    /// The backend behind a drive name, once it has connected, to poke at it in tests.
    #[must_use]
    pub fn backend_of(&self, name: &str) -> Option<Arc<MemoryBackend>> {
        match self.backends.lock() {
            Ok(map) => map.get(name).cloned(),
            Err(poisoned) => poisoned.into_inner().get(name).cloned(),
        }
    }
}

impl BackendFactory for MemoryFactory {
    fn scheme(&self) -> &str {
        "mem"
    }

    fn connect(
        &self,
        config: &DriveConfig,
        secret: Option<&Secret>,
        _prompts: &dyn PromptHandler,
        cancel: &Cancel,
    ) -> Result<Arc<dyn Backend>, ConnectError> {
        if cancel.is_cancelled() {
            return Err(ConnectError::Cancelled);
        }
        if config.param("unreachable") == Some("true") {
            return Err(ConnectError::Unreachable(String::from("connection refused")));
        }
        if config.param("auth") == Some("required") {
            match secret {
                None => return Err(ConnectError::AuthRequired),
                Some(given) if given.expose() != Self::ACCEPTED => {
                    return Err(ConnectError::AuthFailed);
                }
                Some(_) => {}
            }
        }
        let mut map = match self.backends.lock() {
            Ok(map) => map,
            Err(poisoned) => poisoned.into_inner(),
        };
        let backend = map
            .entry(config.id.name().to_owned())
            .or_insert_with(|| {
                Arc::new(match config.param("profile") {
                    Some("object") => MemoryBackend::object_store_like(),
                    _ => MemoryBackend::posix_like(),
                })
            })
            .clone();
        Ok(backend)
    }
}
