//! A cancelled connect must not reach the factory at all, even when the factory
//! itself would not notice the cancel (the mutation pass found the older test
//! passed only because MemoryFactory checks the token on its own).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kara_remote::{
    BackendFactory, ConnectError, ConnectOrRegistryError, ConnectionState, DriveConfig,
    DriveRegistry, MemorySecretStore, PromptHandler, RefuseAll, Secret,
};
use kara_vfs::memory::MemoryBackend;
use kara_vfs::{Backend, Cancel, DriveId};

struct Counting(AtomicUsize);

impl BackendFactory for Counting {
    fn scheme(&self) -> &str {
        "mem"
    }
    fn connect(
        &self,
        _config: &DriveConfig,
        _secret: Option<&Secret>,
        _prompts: &dyn PromptHandler,
        _cancel: &Cancel,
    ) -> Result<Arc<dyn Backend>, ConnectError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(MemoryBackend::posix_like()))
    }
}

#[test]
fn a_cancelled_connect_never_calls_the_factory() {
    let registry = DriveRegistry::new(Arc::new(MemorySecretStore::new()));
    let factory = Arc::new(Counting(AtomicUsize::new(0)));
    registry.register_factory(factory.clone());
    let config = DriveConfig::new("mem", "a", "A", []).expect("config");
    let id: DriveId = config.id.clone();
    registry.add(config).expect("add");

    let cancel = Cancel::new();
    cancel.cancel();
    let error = registry.connect(&id, &RefuseAll, &cancel).expect_err("cancelled");

    assert_eq!(error, ConnectOrRegistryError::Connect(ConnectError::Cancelled));
    assert_eq!(factory.0.load(Ordering::SeqCst), 0);
    assert!(matches!(registry.state(&id), Some(ConnectionState::Failed { .. })));
}
