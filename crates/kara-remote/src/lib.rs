//! Remote drives for Kara, below `kara-ops` and above `kara-vfs`.
//!
//! This crate knows nothing about any protocol. It holds what every remote
//! drive has in common:
//!
//! - [`config`]: the non-secret description of a drive and how it is stored in
//!   `settings.conf`;
//! - [`secrets`]: where passwords and passphrases live (the system keyring in
//!   production, never the settings file);
//! - [`registry`]: the set of configured drives, their connection state, and the
//!   resolver `kara-ops` uses to turn a drive id into a live backend.
//!
//! A protocol (SFTP, S3) is a [`registry::BackendFactory`] registered under its
//! scheme. Adding one touches no UI code.

pub mod config;
pub mod registry;
pub mod secrets;

#[cfg(feature = "keyring")]
pub mod keyring;
#[cfg(feature = "memory")]
pub mod memory;

pub use config::{ConfigError, DriveConfig};
pub use registry::{
    BackendFactory, ConnectError, ConnectOrRegistryError, ConnectionState, DriveRegistry, Prompt,
    PromptAnswer, PromptHandler, RefuseAll, RegistryError, Resolver,
};
pub use secrets::{MemorySecretStore, Secret, SecretError, SecretStore};
