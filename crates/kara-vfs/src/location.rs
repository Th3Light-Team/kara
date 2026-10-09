//! `Location`: where an entry lives, local or on a remote drive, and its
//! lossless string URI form (the only form QML ever sees).

use std::path::PathBuf;

use crate::path::{RemotePath, RemotePathError};

/// Identity of a configured drive: its scheme (`sftp`, `s3`, `mem`) and name.
///
/// The scheme is part of the identity (contract dec_03): a drive never changes
/// kind. Both parts are validated so the URI authority needs no escaping.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DriveId {
    scheme: String,
    name: String,
}

/// Why a scheme or name is not a valid [`DriveId`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DriveIdError {
    #[error("invalid drive scheme: {raw:?}")]
    InvalidScheme { raw: String },
    #[error("invalid drive name: {raw:?}")]
    InvalidName { raw: String },
}

impl DriveId {
    /// Validates `scheme` (`^[a-z][a-z0-9]{0,15}$`) and `name` (a DNS label).
    pub fn new(_scheme: &str, _name: &str) -> Result<DriveId, DriveIdError> {
        todo!("DriveId::new")
    }

    /// The scheme, e.g. `"sftp"`.
    #[must_use]
    pub fn scheme(&self) -> &str {
        todo!("DriveId::scheme")
    }

    /// The name, e.g. `"work-nas"`.
    #[must_use]
    pub fn name(&self) -> &str {
        todo!("DriveId::name")
    }
}

/// A place a folder view can show.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Location {
    Local(PathBuf),
    Remote { drive: DriveId, path: RemotePath },
}

/// Why a [`Location`] cannot be turned into a URI or parsed from one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocationError {
    #[error("local path is not absolute")]
    RelativeLocalPath,
    #[error("unsupported URI scheme: {raw:?}")]
    UnsupportedScheme { raw: String },
    #[error("file URI with a non-local host: {host:?}")]
    NonLocalFileHost { host: String },
    #[error("URI has a query or a fragment")]
    QueryOrFragment,
    #[error("invalid percent-encoding")]
    InvalidPercentEncoding,
    #[error("URI contains a NUL byte")]
    ContainsNul,
    #[error(transparent)]
    Drive(#[from] DriveIdError),
    #[error(transparent)]
    Path(#[from] RemotePathError),
}

impl Location {
    /// The canonical URI: `file://…` or `kara+<scheme>://<name>/…` (cb_06, cb_07).
    pub fn to_uri(&self) -> Result<String, LocationError> {
        todo!("Location::to_uri")
    }

    /// Strict inverse of [`Location::to_uri`] (cb_08, cb_09).
    pub fn from_uri(_uri: &str) -> Result<Location, LocationError> {
        todo!("Location::from_uri")
    }

    /// Whether this is a local path.
    #[must_use]
    pub fn is_local(&self) -> bool {
        todo!("Location::is_local")
    }

    /// The drive of a remote location.
    #[must_use]
    pub fn drive(&self) -> Option<&DriveId> {
        todo!("Location::drive")
    }

    /// True iff both are remote on the same drive (scheme and name).
    #[must_use]
    pub fn same_remote_drive(&self, _other: &Location) -> bool {
        todo!("Location::same_remote_drive")
    }

    /// The containing location; `None` for a root.
    #[must_use]
    pub fn parent(&self) -> Option<Location> {
        todo!("Location::parent")
    }
}
