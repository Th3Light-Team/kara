//! Protocol-free foundation every remote drive plugs into.
//!
//! Contract slug: `remote-backend-contract-kara-vfs-foundation-location-remotep`.
//! Design: [`docs/remote-backends.md`](https://github.com/oliverv/kara/blob/main/docs/remote-backends.md),
//! «Order of work», step 1.
//!
//! This crate fixes the vocabulary ([`RemotePath`], [`DriveId`], [`Location`]
//! with a lossless string URI), the blocking [`Backend`] trait with a
//! commit/abort [`WriteSession`], the closed [`BackendError`] kind set, the
//! [`Capabilities`] flags the UI reads instead of branching on protocol, an
//! in-memory backend (feature `memory`) and the generic conformance suite
//! (feature `conformance`). It contains no protocol code.

pub mod backend;
pub mod capabilities;
pub mod error;
pub mod location;
pub mod path;

#[cfg(feature = "memory")]
pub mod memory;

#[cfg(feature = "conformance")]
pub mod conformance;

pub use backend::{Backend, Cancel, Listing, TRANSFER_CHUNK, WriteSession};
pub use capabilities::Capabilities;
pub use error::{BackendError, BackendErrorKind};
pub use location::{DriveId, DriveIdError, Location, LocationError};
pub use path::{RemotePath, RemotePathError};
