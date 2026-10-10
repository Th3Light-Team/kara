//! Operations whose sources or destination may live on a remote drive.
//!
//! Design: `docs/remote-backends.md`, «Order of work», step 3. Handoff for the
//! UI: `docs/remote-ops-integration.md`.
//!
//! The old path-based API ([`crate::runner::Request`], [`crate::runner::spawn`])
//! is untouched. [`LocationRequest`] is its [`Location`]-based twin, run by
//! [`crate::runner::spawn_with`] with a [`BackendResolver`] that maps a
//! [`DriveId`] to the backend currently serving it.
//!
//! # Routing
//!
//! - All-local request: converted back to a [`crate::runner::Request`] and run
//!   by the existing code, so local behaviour (rename(2) on one volume, the
//!   trash on Replace) is exactly what it was.
//! - Same remote drive with `server_side_copy`: `copy_within`.
//! - Same remote drive, a move and `atomic_rename`: `rename`.
//! - Anything else: streamed `open_read` → `begin_write` in chunks, then the
//!   size of the destination is checked before a move removes its source.
//!
//! # Paths in prompts and summaries
//!
//! [`crate::runner::Event`], [`crate::batch::Failure`] and the report carry a
//! `PathBuf`. A remote item is named there by its canonical URI
//! ([`display_path`]): `kara+sftp://nas/docs/a.txt`, never a local-looking path.
//! [`parse_display_path`] turns it back into a [`Location`].

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use kara_core::{EntryKind, FileEntry};
use kara_fs::LocalBackend;
use kara_fs::trash::ConflictPolicy;
use kara_vfs::{
    Backend, BackendError, BackendErrorKind, Cancel, DriveId, Location, RemotePath,
};

use crate::batch::FailureKind;
use crate::runner::{Op, Request};
use crate::undo::Action;

/// Finds the backend serving a drive, or `None` when it is not connected.
///
/// Called on the worker thread, so it must not touch the UI.
pub type BackendResolver = Arc<dyn Fn(&DriveId) -> Option<Arc<dyn Backend>> + Send + Sync>;

/// A resolver that knows no drive: every remote location is unavailable.
#[must_use]
pub fn no_drives() -> BackendResolver {
    Arc::new(|_| None)
}

/// A copy, move or permanent delete over [`Location`]s.
#[derive(Debug, Clone)]
pub struct LocationRequest {
    pub op: Op,
    pub sources: Vec<Location>,
    /// Ignored by [`Op::Delete`].
    pub dest_dir: Location,
    /// Must be `true` for [`Op::Delete`], which never goes to a trash: the
    /// caller asked the user first, with the focus on the safe button. Without
    /// it the request is refused before anything is touched.
    pub confirmed_permanent: bool,
}

impl LocationRequest {
    /// Whether every source and the destination are local.
    #[must_use]
    pub fn is_all_local(&self) -> bool {
        self.sources.iter().all(Location::is_local)
            && (self.op == Op::Delete || self.dest_dir.is_local())
    }

    /// The path-based request this is, when everything is local.
    #[must_use]
    pub fn to_local(&self) -> Option<Request> {
        if !self.is_all_local() {
            return None;
        }
        let sources = self
            .sources
            .iter()
            .filter_map(|location| match location {
                Location::Local(path) => Some(path.clone()),
                Location::Remote { .. } => None,
            })
            .collect();
        let dest_dir = match &self.dest_dir {
            Location::Local(path) => path.clone(),
            // A delete ignores its destination; the old API still wants one.
            Location::Remote { .. } => PathBuf::new(),
        };
        Some(Request {
            op: self.op,
            sources,
            dest_dir,
        })
    }
}

impl From<Request> for LocationRequest {
    /// The old API has no confirmation flag: its only `Op::Delete` caller is
    /// the permanent-delete dialog, so a converted delete counts as confirmed.
    fn from(request: Request) -> LocationRequest {
        LocationRequest {
            op: request.op,
            sources: request.sources.into_iter().map(Location::Local).collect(),
            dest_dir: Location::Local(request.dest_dir),
            confirmed_permanent: request.op == Op::Delete,
        }
    }
}

/// Why [`crate::runner::spawn_with`] refused a request. Nothing was touched.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    /// A delete is permanent and the request does not say the user confirmed it.
    #[error("permanent delete requested without confirmation")]
    PermanentDeleteNotConfirmed,
    /// A source or the destination is on a drive the resolver does not serve.
    #[error("drive {scheme}://{name} is not connected", scheme = .0.scheme(), name = .0.name())]
    DriveUnavailable(DriveId),
}

/// How a location is named in prompts, failures and the final report: the
/// local path as it is, a remote one as its canonical URI.
#[must_use]
pub fn display_path(location: &Location) -> PathBuf {
    match location {
        Location::Local(path) => path.clone(),
        Location::Remote { drive, path } => PathBuf::from(location.to_uri().unwrap_or_else(|_| {
            format!("kara+{}://{}{}", drive.scheme(), drive.name(), path.as_str())
        })),
    }
}

/// Inverse of [`display_path`]: a URI is parsed, anything else is a local path.
#[must_use]
pub fn parse_display_path(path: &Path) -> Location {
    path.to_str()
        .filter(|text| text.starts_with("kara+"))
        .and_then(|uri| Location::from_uri(uri).ok())
        .unwrap_or_else(|| Location::Local(path.to_path_buf()))
}

/// The failure category of a backend error.
///
/// `Unavailable` is a lost connection: like vanished media, retrying in a loop
/// cannot help, so it is [`FailureKind::MediaGone`]. `AuthRequired` needs the
/// user to act first, which is what [`FailureKind::PermissionDenied`] offers.
#[must_use]
pub fn failure_kind(kind: BackendErrorKind) -> FailureKind {
    match kind {
        BackendErrorKind::PermissionDenied | BackendErrorKind::AuthRequired => {
            FailureKind::PermissionDenied
        }
        BackendErrorKind::NoSpace => FailureKind::NoSpace,
        BackendErrorKind::Unavailable => FailureKind::MediaGone,
        BackendErrorKind::NotFound
        | BackendErrorKind::AlreadyExists
        | BackendErrorKind::Unsupported
        | BackendErrorKind::Cancelled
        | BackendErrorKind::Other => FailureKind::Other,
    }
}

/// A plain-language reason for a person. The path is not repeated: the prompt
/// already names the item, by its URI.
#[must_use]
pub fn reason(kind: BackendErrorKind) -> String {
    match kind {
        BackendErrorKind::NotFound => "no existe",
        BackendErrorKind::AlreadyExists => "ya existe",
        BackendErrorKind::PermissionDenied => "permiso denegado",
        BackendErrorKind::NoSpace => "no queda espacio en el destino",
        BackendErrorKind::Unavailable => "la unidad no está disponible",
        BackendErrorKind::AuthRequired => "la unidad pide autenticarse",
        BackendErrorKind::Unsupported => "la unidad no permite esta operación",
        BackendErrorKind::Cancelled => "cancelado",
        BackendErrorKind::Other => "error de la unidad",
    }
    .to_string()
}

/// A failed attempt, already classified for the batch policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fail {
    pub kind: FailureKind,
    pub reason: String,
}

impl Fail {
    pub(crate) fn other(reason: &str) -> Fail {
        Fail {
            kind: FailureKind::Other,
            reason: reason.to_string(),
        }
    }

    pub(crate) fn cancelled() -> Fail {
        Fail::from(BackendError::new(BackendErrorKind::Cancelled, None))
    }

    /// An error out of a backend's reader or session.
    pub(crate) fn from_io(error: io::Error) -> Fail {
        Fail::from(BackendError::from_io(error, None))
    }
}

impl From<BackendError> for Fail {
    fn from(error: BackendError) -> Fail {
        Fail {
            kind: failure_kind(error.kind),
            reason: reason(error.kind),
        }
    }
}

/// The one local backend, rooted at `/`.
pub(crate) fn local_backend() -> Arc<dyn Backend> {
    static LOCAL: OnceLock<Arc<dyn Backend>> = OnceLock::new();
    Arc::clone(LOCAL.get_or_init(|| Arc::new(LocalBackend::system())))
}

/// A location together with the backend that serves it and its path there.
#[derive(Clone)]
pub(crate) struct Endpoint {
    pub backend: Arc<dyn Backend>,
    pub path: RemotePath,
    pub location: Location,
}

/// Why a location could not be turned into an [`Endpoint`].
#[derive(Debug, Clone)]
pub(crate) enum EndpointError {
    Drive(DriveId),
    /// The local name cannot be addressed through the trait (not UTF-8, or
    /// not absolute).
    Name,
}

impl EndpointError {
    pub(crate) fn fail(&self) -> Fail {
        match self {
            EndpointError::Drive(_) => Fail::from(BackendError::new(
                BackendErrorKind::Unavailable,
                None,
            )),
            EndpointError::Name => Fail::other("el nombre no se puede usar en otra unidad"),
        }
    }
}

impl Endpoint {
    pub(crate) fn resolve(
        location: &Location,
        resolver: &BackendResolver,
    ) -> Result<Endpoint, EndpointError> {
        match location {
            Location::Local(local) => {
                let path = LocalBackend::system()
                    .to_remote(local)
                    .map_err(|_| EndpointError::Name)?;
                Ok(Endpoint {
                    backend: local_backend(),
                    path,
                    location: location.clone(),
                })
            }
            Location::Remote { drive, path } => {
                let backend = resolver(drive).ok_or_else(|| EndpointError::Drive(drive.clone()))?;
                Ok(Endpoint {
                    backend,
                    path: path.clone(),
                    location: location.clone(),
                })
            }
        }
    }

    pub(crate) fn is_local(&self) -> bool {
        self.location.is_local()
    }

    /// Both local, or both on the same remote drive.
    pub(crate) fn same_backend(&self, other: &Endpoint) -> bool {
        (self.is_local() && other.is_local()) || self.location.same_remote_drive(&other.location)
    }

    pub(crate) fn display(&self) -> PathBuf {
        display_path(&self.location)
    }

    pub(crate) fn name(&self) -> String {
        self.path.file_name().unwrap_or_default().to_string()
    }

    /// The child `name` of this directory.
    pub(crate) fn join(&self, name: &str) -> Option<Endpoint> {
        let path = self.path.join(name).ok()?;
        let location = match &self.location {
            Location::Local(local) => Location::Local(local.join(name)),
            Location::Remote { drive, .. } => Location::Remote {
                drive: drive.clone(),
                path: path.clone(),
            },
        };
        Some(Endpoint {
            backend: Arc::clone(&self.backend),
            path,
            location,
        })
    }

    pub(crate) fn parent(&self) -> Option<Endpoint> {
        let path = self.path.parent()?;
        let location = self.location.parent()?;
        Some(Endpoint {
            backend: Arc::clone(&self.backend),
            path,
            location,
        })
    }

    /// `name` next to this one.
    pub(crate) fn sibling(&self, name: &str) -> Option<Endpoint> {
        self.parent()?.join(name)
    }

    /// `dir/name` → `dir/name (2)`, the first free one, by the same rule as a
    /// local «Conservar ambos». A name the backend cannot answer for counts as
    /// free: the write that follows refuses to overwrite anyway.
    pub(crate) fn unique_sibling(&self) -> Endpoint {
        let Some(parent) = self.parent() else {
            return self.clone();
        };
        let free = kara_core::unique_name(&self.name(), |candidate| {
            parent
                .join(candidate)
                .is_some_and(|end| end.backend.stat(&end.path).is_ok())
        });
        parent.join(&free).unwrap_or_else(|| self.clone())
    }

    pub(crate) fn stat(&self) -> Result<FileEntry, BackendError> {
        self.backend.stat(&self.path)
    }
}

/// Whether an entry is walked into: a real directory, never a link to one.
pub(crate) fn is_walkable_dir(entry: &FileEntry) -> bool {
    entry.kind == EntryKind::Directory && !entry.is_symlink
}

/// `(nodes, bytes)` under `end`, links not followed. Cancellable.
pub(crate) fn measure(end: &Endpoint, cancel: &Cancel) -> Result<(u64, u64), BackendError> {
    let entry = end.stat()?;
    if !is_walkable_dir(&entry) {
        return Ok((1, entry.size.unwrap_or(0)));
    }
    measure_dir(end, cancel)
}

fn measure_dir(end: &Endpoint, cancel: &Cancel) -> Result<(u64, u64), BackendError> {
    let listing = end.backend.list(&end.path, cancel)?;
    let mut items = 1u64;
    let mut bytes = 0u64;
    for entry in listing.entries {
        if cancel.is_cancelled() {
            return Err(BackendError::new(BackendErrorKind::Cancelled, Some(end.path.clone())));
        }
        if !is_walkable_dir(&entry) {
            items = items.saturating_add(1);
            bytes = bytes.saturating_add(entry.size.unwrap_or(0));
            continue;
        }
        // One unreadable child must not hide the rest of the total.
        let child = entry.name.to_str().and_then(|name| end.join(name));
        match child.map(|child| measure_dir(&child, cancel)) {
            Some(Ok((i, b))) => {
                items = items.saturating_add(i);
                bytes = bytes.saturating_add(b);
            }
            Some(Err(error)) if error.kind == BackendErrorKind::Cancelled => return Err(error),
            _ => items = items.saturating_add(1),
        }
    }
    Ok((items, bytes))
}

/// Why [`rename_at`] or [`create_dir_at`] did nothing.
#[derive(Debug, thiserror::Error)]
pub enum LocationOpError {
    /// The drive is not connected.
    #[error("drive {scheme}://{name} is not connected", scheme = .0.scheme(), name = .0.name())]
    DriveUnavailable(DriveId),
    /// The new name is not a single valid name.
    #[error("invalid name: {0:?}")]
    InvalidName(String),
    /// The target name is taken. Nothing is ever overwritten.
    #[error("{} already exists", .0.display())]
    AlreadyExists(PathBuf),
    /// The backend refused; `path` is the item's display path (a URI when remote).
    #[error("{}: {reason}", path.display())]
    Failed {
        path: PathBuf,
        kind: FailureKind,
        reason: String,
    },
    /// The local operation failed, as the old code reports it.
    #[error(transparent)]
    Local(#[from] kara_fs::transfer::TransferError),
}

impl LocationOpError {
    fn backend(end: &Endpoint, error: &BackendError) -> LocationOpError {
        LocationOpError::Failed {
            path: end.display(),
            kind: failure_kind(error.kind),
            reason: reason(error.kind),
        }
    }
}

fn endpoint_for_op(
    location: &Location,
    resolver: &BackendResolver,
) -> Result<Endpoint, LocationOpError> {
    Endpoint::resolve(location, resolver).map_err(|error| match error {
        EndpointError::Drive(drive) => LocationOpError::DriveUnavailable(drive),
        EndpointError::Name => LocationOpError::InvalidName(
            display_path(location).to_string_lossy().into_owned(),
        ),
    })
}

/// Renames `location` to `new_name` in the same folder, never over an existing
/// name, and returns where it landed and the undo record.
///
/// Blocking: on a remote drive this is a network round trip, so the UI calls
/// it from a worker. A local location goes through `kara_fs::rename` exactly
/// as before. A remote rename is undoable only if the drive declares
/// `undo_rename`; otherwise the record says why not.
pub fn rename_at(
    location: &Location,
    new_name: &str,
    resolver: &BackendResolver,
) -> Result<(Location, Action), LocationOpError> {
    if let Location::Local(path) = location {
        let destination = kara_fs::rename(path, OsStr::new(new_name), ConflictPolicy::Fail)?;
        let action = Action::Renamed {
            from: path.clone(),
            to: destination.clone(),
        };
        return Ok((Location::Local(destination), action));
    }
    let from = endpoint_for_op(location, resolver)?;
    let to = from
        .sibling(new_name)
        .ok_or_else(|| LocationOpError::InvalidName(new_name.to_string()))?;
    if to.path == from.path {
        return Err(LocationOpError::InvalidName(new_name.to_string()));
    }
    match to.stat() {
        Ok(_) => return Err(LocationOpError::AlreadyExists(to.display())),
        Err(error) if error.kind == BackendErrorKind::NotFound => {}
        Err(error) => return Err(LocationOpError::backend(&to, &error)),
    }
    from.backend
        .rename(&from.path, &to.path)
        .map_err(|error| LocationOpError::backend(&from, &error))?;
    let action = if from.backend.capabilities().undo_rename {
        Action::RemoteRenamed {
            from: from.location.clone(),
            to: to.location.clone(),
        }
    } else {
        Action::NotUndoable {
            label: "renombrar",
            subject: to.location.clone(),
            reason: NO_UNDO_RENAME.to_string(),
        }
    };
    Ok((to.location, action))
}

/// Creates the folder `name` inside `parent`. A taken name becomes
/// «name (2)», like the local «Nueva carpeta». Blocking, like [`rename_at`].
pub fn create_dir_at(
    parent: &Location,
    name: &str,
    resolver: &BackendResolver,
) -> Result<(Location, Action), LocationOpError> {
    if let Location::Local(dir) = parent {
        let path = kara_fs::create_directory(dir, OsStr::new(name), ConflictPolicy::KeepBoth)?;
        return Ok((
            Location::Local(path.clone()),
            Action::DirectoryCreated { path },
        ));
    }
    let dir = endpoint_for_op(parent, resolver)?;
    let wanted = dir
        .join(name)
        .ok_or_else(|| LocationOpError::InvalidName(name.to_string()))?;
    let target = wanted.unique_sibling();
    target
        .backend
        .create_dir(&target.path)
        .map_err(|error| LocationOpError::backend(&target, &error))?;
    let action = Action::RemoteDirectoryCreated {
        path: target.location.clone(),
    };
    Ok((target.location, action))
}

/// Why a rename on a drive without `undo_rename` is not undoable.
pub(crate) const NO_UNDO_RENAME: &str = "la unidad no permite deshacer un renombrado";
/// Why a move on a drive without `undo_move` is not undoable.
pub(crate) const NO_UNDO_MOVE: &str = "la unidad no permite deshacer un movimiento";
/// Why a move between two different backends is not undoable.
pub(crate) const NO_UNDO_CROSS_MOVE: &str =
    "mover entre unidades distintas no se puede deshacer: habría que volver a transferirlo todo";
/// Why a copy that replaced a remote file is not undoable.
pub(crate) const NO_UNDO_REMOTE_REPLACE: &str =
    "se reemplazó un fichero en una unidad sin papelera: el anterior no se puede recuperar";
