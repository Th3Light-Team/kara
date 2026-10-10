//! Registry additions for the drives panel: `schemes`, `update`, `test_connection`.

use std::sync::Arc;

use kara_remote::memory::MemoryFactory;
use kara_remote::{
    ConnectError, ConnectionState, DriveConfig, DriveRegistry, MemorySecretStore, RefuseAll,
    RegistryError, Secret,
};
use kara_vfs::Cancel;

fn drive(name: &str, params: &[(&str, &str)]) -> DriveConfig {
    DriveConfig::new(
        "mem",
        name,
        name,
        params.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    )
    .expect("valid config")
}

fn registry() -> Arc<DriveRegistry> {
    let registry = DriveRegistry::new(Arc::new(MemorySecretStore::new()));
    registry.register_factory(MemoryFactory::new());
    registry
}

#[test]
fn schemes_lists_the_registered_factories_only() {
    let empty = DriveRegistry::new(Arc::new(MemorySecretStore::new()));
    assert!(empty.schemes().is_empty());
    assert_eq!(registry().schemes(), ["mem"]);
}

#[test]
fn update_replaces_the_config_and_drops_the_session() {
    let registry = registry();
    let config = drive("a", &[]);
    let id = config.id.clone();
    registry.add(config).expect("add");
    registry.connect(&id, &RefuseAll, &Cancel::new()).expect("connect");
    assert_eq!(registry.state(&id), Some(ConnectionState::Ready));

    let changed = drive("a", &[("profile", "object")]);
    registry.update(changed).expect("update");
    assert_eq!(registry.state(&id), Some(ConnectionState::Disconnected));
    assert!(registry.backend(&id).is_none());
    assert_eq!(registry.config(&id).expect("config").param("profile"), Some("object"));
}

#[test]
fn update_of_an_unknown_drive_is_refused() {
    assert_eq!(registry().update(drive("ghost", &[])), Err(RegistryError::Unknown));
}

#[test]
fn test_connection_checks_without_registering() {
    let registry = registry();
    let config = drive("probe", &[]);
    registry
        .test_connection(&config, None, &RefuseAll, &Cancel::new())
        .expect("reachable");
    assert!(registry.config(&config.id).is_none(), "nothing was added");
}

#[test]
fn test_connection_reports_the_failure_kind() {
    let registry = registry();
    let down = drive("down", &[("unreachable", "true")]);
    assert!(matches!(
        registry.test_connection(&down, None, &RefuseAll, &Cancel::new()),
        Err(ConnectError::Unreachable(_))
    ));

    let locked = drive("locked", &[("auth", "required")]);
    assert_eq!(
        registry.test_connection(&locked, None, &RefuseAll, &Cancel::new()),
        Err(ConnectError::AuthRequired)
    );
    let wrong = Secret::new("nope");
    assert_eq!(
        registry.test_connection(&locked, Some(&wrong), &RefuseAll, &Cancel::new()),
        Err(ConnectError::AuthFailed)
    );
    let right = Secret::new(MemoryFactory::ACCEPTED);
    assert!(registry
        .test_connection(&locked, Some(&right), &RefuseAll, &Cancel::new())
        .is_ok());
}

#[test]
fn test_connection_with_no_factory_is_an_error() {
    let registry = DriveRegistry::new(Arc::new(MemorySecretStore::new()));
    let result = registry.test_connection(&drive("x", &[]), None, &RefuseAll, &Cancel::new());
    assert!(matches!(result, Err(ConnectError::Other(_))));
}

#[test]
fn test_connection_honours_cancel() {
    let registry = registry();
    let cancel = Cancel::new();
    cancel.cancel();
    assert_eq!(
        registry.test_connection(&drive("c", &[]), None, &RefuseAll, &cancel),
        Err(ConnectError::Cancelled)
    );
}

#[test]
fn stored_secret_prefers_the_drive_over_its_group() {
    let registry = registry();
    let config = drive("s", &[]).with_group("fleet").expect("group");
    assert!(registry.stored_secret(&config).is_none());
    registry
        .remember_group_secret("fleet", &Secret::new("group-pw"))
        .expect("group secret");
    assert_eq!(registry.stored_secret(&config).expect("group").expose(), "group-pw");
    registry.add(config.clone()).expect("add");
    registry.remember_secret(&config.id, &Secret::new("own")).expect("own");
    assert_eq!(registry.stored_secret(&config).expect("own").expose(), "own");
}
