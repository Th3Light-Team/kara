//! Remote drives for Kara, below `kara-ops` and above `kara-vfs`.
//!
//! This crate knows nothing about any protocol. It holds what every remote
//! drive has in common:
//!
//! - [`config`]: the non-secret description of a drive and how it is stored in
//!   `settings.conf`;
//! - [`import`]: many drives at once, from a CSV list or an OpenSSH config;
//! - [`secrets`]: where passwords and passphrases live (the system keyring in
//!   production, never the settings file);
//! - [`registry`]: the set of configured drives, their connection state, and the
//!   resolver `kara-ops` uses to turn a drive id into a live backend.
//!
//! A protocol (SFTP, S3) is a [`registry::BackendFactory`] registered under its
//! scheme. Adding one touches no UI code. SFTP lives in `sftp` (feature `sftp`);
//! S3 and Google Cloud Storage in `objstore` (features `s3`, `gcs`).

pub mod config;
pub mod form;
pub mod import;
pub mod registry;
pub mod secrets;

#[cfg(feature = "keyring")]
pub mod keyring;
#[cfg(feature = "memory")]
pub mod memory;
#[cfg(feature = "objectstore")]
pub mod objstore;
#[cfg(feature = "sftp")]
pub mod sftp;

pub use config::{ConfigError, DriveConfig};
pub use registry::{
    BackendFactory, ConnectError, ConnectOrRegistryError, ConnectionState, DriveRegistry, Prompt,
    PromptAnswer, PromptHandler, RefuseAll, RegistryError, Remember, Resolver,
};
pub use secrets::{FallbackSecretStore, MemorySecretStore, Secret, SecretError, SecretKey, SecretStore};
