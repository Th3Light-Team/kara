//! `Location`: where an entry lives, local or on a remote drive, and its
//! lossless string URI form (the only form QML ever sees).

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use percent_encoding::{AsciiSet, CONTROLS, percent_encode};

use crate::path::{RemotePath, RemotePathError};

/// Bytes escaped on top of the ASCII controls (and every byte >= 0x80, which
/// `percent_encode` always escapes). This is the same set the local `file://`
/// URI of the desktop uses (GLib, Qt): the thumbnail cache is keyed by that
/// exact string, so it must not drift. Duplicated here because this crate sits
/// below the filesystem crate in the layering.
const URI_PATH_ESCAPE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

const FILE_PREFIX: &str = "file";
const REMOTE_PREFIX: &str = "kara+";

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
    pub fn new(scheme: &str, name: &str) -> Result<DriveId, DriveIdError> {
        if !valid_scheme(scheme) {
            return Err(DriveIdError::InvalidScheme {
                raw: scheme.to_owned(),
            });
        }
        if !valid_name(name) {
            return Err(DriveIdError::InvalidName {
                raw: name.to_owned(),
            });
        }
        Ok(DriveId {
            scheme: scheme.to_owned(),
            name: name.to_owned(),
        })
    }

    /// The scheme, e.g. `"sftp"`.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The name, e.g. `"work-nas"`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

fn valid_scheme(scheme: &str) -> bool {
    let bytes = scheme.as_bytes();
    let first_ok = bytes.first().is_some_and(u8::is_ascii_lowercase);
    first_ok
        && bytes.len() <= 16
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let alnum = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let edges_ok = bytes.first().is_some_and(alnum) && bytes.last().is_some_and(alnum);
    edges_ok && bytes.len() <= 63 && bytes.iter().all(|b| alnum(b) || *b == b'-')
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
        match self {
            Location::Local(path) => {
                if !path.is_absolute() {
                    return Err(LocationError::RelativeLocalPath);
                }
                let bytes = path.as_os_str().as_bytes();
                if bytes.contains(&0) {
                    return Err(LocationError::ContainsNul);
                }
                let mut uri = String::from("file://");
                uri.extend(percent_encode(bytes, URI_PATH_ESCAPE_SET));
                Ok(uri)
            }
            Location::Remote { drive, path } => {
                let mut uri = String::with_capacity(16 + path.as_str().len());
                uri.push_str(REMOTE_PREFIX);
                uri.push_str(drive.scheme());
                uri.push_str("://");
                uri.push_str(drive.name());
                uri.extend(percent_encode(
                    path.as_str().as_bytes(),
                    URI_PATH_ESCAPE_SET,
                ));
                Ok(uri)
            }
        }
    }

    /// Strict inverse of [`Location::to_uri`] (cb_08, cb_09).
    pub fn from_uri(uri: &str) -> Result<Location, LocationError> {
        let Some((scheme, rest)) = uri.split_once(':') else {
            return Err(LocationError::UnsupportedScheme {
                raw: uri.to_owned(),
            });
        };
        let remote_scheme = scheme.strip_prefix(REMOTE_PREFIX);
        if scheme != FILE_PREFIX && remote_scheme.is_none_or(str::is_empty) {
            return Err(LocationError::UnsupportedScheme {
                raw: scheme.to_owned(),
            });
        }
        if rest.contains(['?', '#']) {
            return Err(LocationError::QueryOrFragment);
        }
        let Some(after_slashes) = rest.strip_prefix("//") else {
            return if remote_scheme.is_none() {
                Err(LocationError::RelativeLocalPath)
            } else {
                Err(LocationError::UnsupportedScheme {
                    raw: scheme.to_owned(),
                })
            };
        };
        let (authority, path) = match after_slashes.find('/') {
            Some(cut) => after_slashes.split_at(cut),
            None => (after_slashes, ""),
        };
        let bytes = decode_path(path)?;

        match remote_scheme {
            None => {
                if !authority.is_empty() {
                    return Err(LocationError::NonLocalFileHost {
                        host: authority.to_owned(),
                    });
                }
                if bytes.first() != Some(&b'/') {
                    return Err(LocationError::RelativeLocalPath);
                }
                Ok(Location::Local(PathBuf::from(OsStr::from_bytes(&bytes))))
            }
            Some(drive_scheme) => {
                let drive = DriveId::new(drive_scheme, authority)?;
                let path = if bytes.is_empty() {
                    RemotePath::root()
                } else {
                    RemotePath::from_bytes(&bytes)?
                };
                Ok(Location::Remote { drive, path })
            }
        }
    }

    /// Whether this is a local path.
    #[must_use]
    pub fn is_local(&self) -> bool {
        matches!(self, Location::Local(_))
    }

    /// The drive of a remote location.
    #[must_use]
    pub fn drive(&self) -> Option<&DriveId> {
        match self {
            Location::Local(_) => None,
            Location::Remote { drive, .. } => Some(drive),
        }
    }

    /// True iff both are remote on the same drive (scheme and name).
    #[must_use]
    pub fn same_remote_drive(&self, other: &Location) -> bool {
        match (self.drive(), other.drive()) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// The containing location; `None` for a root.
    #[must_use]
    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Local(path) => Path::parent(path).map(|p| Location::Local(p.to_path_buf())),
            Location::Remote { drive, path } => path.parent().map(|parent| Location::Remote {
                drive: drive.clone(),
                path: parent,
            }),
        }
    }
}

/// Decodes `%XX` escapes strictly: a `%` not followed by two hex digits is an
/// error (the `percent_encoding` decoder would pass it through), and a decoded
/// NUL is refused.
fn decode_path(raw: &str) -> Result<Vec<u8>, LocationError> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hi = bytes.get(i + 1).and_then(|c| hex_value(*c));
            let lo = bytes.get(i + 2).and_then(|c| hex_value(*c));
            match (hi, lo) {
                (Some(hi), Some(lo)) => out.push(hi << 4 | lo),
                _ => return Err(LocationError::InvalidPercentEncoding),
            }
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    if out.contains(&0) {
        return Err(LocationError::ContainsNul);
    }
    Ok(out)
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}
