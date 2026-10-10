//! `ObjectStoreBackend`: any `object_store::ObjectStore` behind
//! [`kara_vfs::Backend`].
//!
//! The semantics are `MemoryBackend::object_store_like()`'s, checked by the
//! same conformance suite and by a differential test against it:
//!
//! - folders are key prefixes; an empty folder is kept alive by a hidden
//!   placeholder object (see [`super::keys`]); a folder without one vanishes
//!   with its last object;
//! - when an object and a folder share a name (another tool wrote both
//!   `photos` and `photos/x.jpg`), **the object wins**: `stat` describes the
//!   file, `list` of it is `Other` (not a folder), and the listing of the
//!   parent reports the hidden folder as a per-entry error;
//! - nothing is created under an object (`Other`, not a directory), and
//!   nothing is ever overwritten unless the caller asked to replace a file;
//! - errors name the caller's [`RemotePath`], never an object key.
//!
//! `rename` is copy then delete (`atomic_rename = false`): every object is
//! copied first (no-clobber), and the sources are deleted only once **all**
//! copies exist. A copy that fails rolls the copies made so far back, so a
//! failure in that phase leaves every object under its old name only. A
//! failure while deleting the sources leaves the rest under **both** names
//! (never under neither) and is reported.

use std::borrow::Cow;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::join_all;
use futures_util::StreamExt;
use kara_core::{EntryKind, FileEntry, MetadataBag, MetadataKey, MetadataValue};
use kara_vfs::{
    Backend, BackendError, BackendErrorKind, Cancel, Capabilities, Listing, RemotePath,
    WriteSession,
};
use object_store::list::{PaginatedListOptions, PaginatedListStore};
use object_store::path::Path;
use object_store::{
    CopyMode, CopyOptions, ObjectMeta, ObjectStore, ObjectStoreExt, PutMode, PutOptions,
    PutPayload,
};

use super::error::{Fail, error, other};
use super::io::{ObjectReader, ObjectWriteSession, WriteTuning};
use super::keys::{Keys, PLACEHOLDER, is_placeholder, join};
use super::runtime::Rt;

/// Smallest part S3 accepts (except for the last one).
pub const MIN_PART_SIZE: usize = 5 * 1024 * 1024;
/// Default size of an upload part.
pub const DEFAULT_PART_SIZE: usize = 8 * 1024 * 1024;
/// Default number of parts uploaded at once by one write session.
pub const DEFAULT_UPLOAD_CONCURRENCY: usize = 4;
/// Default number of keys asked for per listing page.
pub const DEFAULT_PAGE_SIZE: usize = 1000;
/// Default per-request timeout, in seconds.
pub const DEFAULT_TIMEOUT_S: u64 = 30;
/// Objects copied at once by a folder `rename`.
const COPY_BATCH: usize = 32;
/// Keys deleted per batch by `remove_tree` (S3 `DeleteObjects` takes 1000).
const DELETE_BATCH: usize = 1000;

/// How an [`ObjectStoreBackend`] is set up.
#[derive(Clone)]
pub struct ObjectStoreOptions {
    /// Key prefix the drive's root stands for; empty for the whole bucket.
    pub prefix: String,
    /// Real page-by-page listing (S3, GCS). Without it, a folder is listed in
    /// one call to `list_with_delimiter` (the library pages internally) and
    /// the cancel token is only watched while that call runs.
    pub pager: Option<Arc<dyn PaginatedListStore>>,
    /// The client's per-request timeout. Every blocking call is also bounded
    /// by four times this (plus the client's retries) so nothing hangs.
    pub timeout: Duration,
    /// Size of upload parts; at least [`MIN_PART_SIZE`]. Smaller files are
    /// one `PUT`.
    pub part_size: usize,
    /// Parts in flight per write session.
    pub upload_concurrency: usize,
    /// Keys per listing page.
    pub page_size: usize,
    /// Objects larger than this are copied by streaming them through the
    /// client instead of a server-side copy (S3's `CopyObject` stops at 5 GiB).
    pub max_server_copy: Option<u64>,
    /// What `Debug` shows: `s3://bucket/prefix`, never a credential.
    pub label: String,
}

impl Default for ObjectStoreOptions {
    fn default() -> ObjectStoreOptions {
        ObjectStoreOptions {
            prefix: String::new(),
            pager: None,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_S),
            part_size: DEFAULT_PART_SIZE,
            upload_concurrency: DEFAULT_UPLOAD_CONCURRENCY,
            page_size: DEFAULT_PAGE_SIZE,
            max_server_copy: None,
            label: String::from("object store"),
        }
    }
}

impl fmt::Debug for ObjectStoreOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObjectStoreOptions")
            .field("prefix", &self.prefix)
            .field("paged_listing", &self.pager.is_some())
            .field("timeout", &self.timeout)
            .field("part_size", &self.part_size)
            .field("upload_concurrency", &self.upload_concurrency)
            .field("page_size", &self.page_size)
            .field("max_server_copy", &self.max_server_copy)
            .field("label", &self.label)
            .finish()
    }
}

/// The store and what every async helper needs; shared with readers and
/// write sessions.
pub(crate) struct Inner {
    pub(crate) store: Arc<dyn ObjectStore>,
    pager: Option<Arc<dyn PaginatedListStore>>,
    pub(crate) keys: Keys,
    page_size: usize,
    /// The store refused `PutMode::Create` once: check-then-put from now on.
    put_create_unsupported: AtomicBool,
    /// The store refused `CopyMode::Create` once: check-then-copy from now on.
    copy_create_unsupported: AtomicBool,
}

/// One page of a listing.
struct Page {
    objects: Vec<ObjectMeta>,
    prefixes: Vec<Path>,
    next: Option<String>,
}

fn not_found(path: &RemotePath) -> BackendError {
    BackendError::new(BackendErrorKind::NotFound, Some(path.clone()))
}

fn already_exists(path: &RemotePath) -> BackendError {
    BackendError::new(BackendErrorKind::AlreadyExists, Some(path.clone()))
}

fn cancelled(path: &RemotePath) -> BackendError {
    BackendError::new(BackendErrorKind::Cancelled, Some(path.clone()))
}

/// The error for a path that cannot be created (an unrepresentable or
/// reserved name).
fn no_key_for_create(path: &RemotePath) -> BackendError {
    other(path, io::ErrorKind::InvalidInput)
}

/// Whole seconds: S3's `HEAD` says `Last-Modified` to the second, its
/// listing to the millisecond; `list` and `stat` must agree.
fn time_of(meta: &ObjectMeta) -> Option<SystemTime> {
    let seconds = meta.last_modified.timestamp();
    if seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(seconds.unsigned_abs()))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()))
    }
}

pub(crate) fn file_entry(name: &str, meta: &ObjectMeta) -> FileEntry {
    let mut extra = MetadataBag::new();
    if let Some(tag) = &meta.e_tag {
        extra.insert(
            MetadataKey::Custom("object.etag".into()),
            MetadataValue::Text(tag.clone()),
        );
    }
    if let Some(version) = &meta.version {
        extra.insert(
            MetadataKey::Custom("object.version".into()),
            MetadataValue::Text(version.clone()),
        );
    }
    FileEntry {
        name: OsString::from(name),
        display: name.to_owned(),
        kind: EntryKind::File,
        is_symlink: false,
        symlink_broken: false,
        is_hidden: name.starts_with('.'),
        size: Some(meta.size),
        modified: time_of(meta),
        created: None,
        accessed: None,
        type_label: None,
        location: None,
        extra,
    }
}

fn dir_entry(name: &str) -> FileEntry {
    FileEntry {
        name: OsString::from(name),
        display: name.to_owned(),
        kind: EntryKind::Directory,
        is_symlink: false,
        symlink_broken: false,
        is_hidden: name.starts_with('.') && name != "/",
        size: None,
        modified: None,
        created: None,
        accessed: None,
        type_label: None,
        location: None,
        extra: MetadataBag::new(),
    }
}

impl Inner {
    /// The object at `key`, `None` when there is none.
    async fn head(&self, key: &Path) -> Result<Option<ObjectMeta>, Fail> {
        if key.as_ref().is_empty() {
            return Ok(None);
        }
        match self.store.head(key).await {
            Ok(meta) => Ok(Some(meta)),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(Fail::Store(error)),
        }
    }

    /// Whether an object is stored at `key`.
    pub(crate) async fn head_exists(&self, key: &Path) -> Result<bool, Fail> {
        Ok(self.head(key).await?.is_some())
    }

    /// One page of the keys under `key/` (the whole bucket or prefix for the
    /// root), with or without the `/` delimiter.
    async fn page(
        &self,
        key: &Path,
        delimiter: bool,
        token: Option<String>,
        max: Option<usize>,
    ) -> Result<Page, Fail> {
        if let Some(pager) = &self.pager {
            let dir = Keys::dir_prefix(key);
            let prefix = (!dir.is_empty()).then_some(dir.as_str());
            let options = PaginatedListOptions {
                delimiter: delimiter.then_some(Cow::Borrowed("/")),
                max_keys: Some(max.unwrap_or(self.page_size)),
                page_token: token,
                ..PaginatedListOptions::default()
            };
            let page = pager.list_paginated(prefix, options).await?;
            return Ok(Page {
                objects: page.result.objects,
                prefixes: page.result.common_prefixes,
                next: page.page_token,
            });
        }
        let prefix = (!key.as_ref().is_empty()).then_some(key);
        if delimiter {
            let all = self.store.list_with_delimiter(prefix).await?;
            return Ok(Page {
                objects: all.objects,
                prefixes: all.common_prefixes,
                next: None,
            });
        }
        let mut stream = self.store.list(prefix);
        let mut objects = Vec::new();
        while let Some(item) = stream.next().await {
            objects.push(item?);
            if max.is_some_and(|max| objects.len() >= max) {
                break;
            }
        }
        Ok(Page {
            objects,
            prefixes: Vec::new(),
            next: None,
        })
    }

    /// Up to `n` keys anywhere under `key/`.
    async fn first_under(&self, key: &Path, n: usize) -> Result<Vec<ObjectMeta>, Fail> {
        let page = self.page(key, false, None, Some(n)).await?;
        Ok(page.objects.into_iter().take(n).collect())
    }

    /// Whether anything is stored under `key/` (a folder, explicit or not).
    async fn has_children(&self, key: &Path) -> Result<bool, Fail> {
        Ok(!self.first_under(key, 1).await?.is_empty())
    }

    /// Every key under `key/`, all pages.
    async fn all_under(&self, key: &Path) -> Result<Vec<ObjectMeta>, Fail> {
        let mut out = Vec::new();
        let mut token = None;
        loop {
            let page = self.page(key, false, token, None).await?;
            out.extend(page.objects);
            match page.next {
                Some(next) => token = Some(next),
                None => return Ok(out),
            }
        }
    }

    /// Whether some ancestor of `path` (below the root) is an object.
    async fn object_ancestor(&self, path: &RemotePath) -> Result<bool, Fail> {
        let mut keys = Vec::new();
        let mut ancestor = path.parent();
        while let Some(dir) = ancestor {
            if dir.is_root() {
                break;
            }
            if let Ok(key) = self.keys.raw(&dir) {
                keys.push(key);
            }
            ancestor = dir.parent();
        }
        for found in join_all(keys.iter().map(|key| self.head(key))).await {
            if found?.is_some() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Stores `payload` under `key` only if nothing is there. Conditional put
    /// where the store has it; otherwise (and always before it, for servers
    /// that ignore the condition) a check first, which leaves a small race.
    pub(crate) async fn put_new(&self, key: &Path, payload: PutPayload) -> Result<(), Fail> {
        if self.head(key).await?.is_some() {
            return Err(Fail::Store(object_store::Error::AlreadyExists {
                path: key.to_string(),
                source: "the object exists".into(),
            }));
        }
        if !self.put_create_unsupported.load(Ordering::Relaxed) {
            let options = PutOptions {
                mode: PutMode::Create,
                ..PutOptions::default()
            };
            match self.store.put_opts(key, payload.clone(), options).await {
                Ok(_) => return Ok(()),
                Err(
                    object_store::Error::NotSupported { .. }
                    | object_store::Error::NotImplemented { .. },
                ) => self.put_create_unsupported.store(true, Ordering::Relaxed),
                Err(error) => return Err(Fail::Store(error)),
            }
        }
        self.store.put(key, payload).await?;
        Ok(())
    }

    /// Copies `from` to `to` only if nothing is at `to`: `CopyMode::Create`
    /// where the store has it, otherwise check-then-copy (a small race).
    async fn copy_new(&self, from: &Path, to: &Path) -> Result<(), Fail> {
        if !self.copy_create_unsupported.load(Ordering::Relaxed) {
            let options = CopyOptions::new().with_mode(CopyMode::Create);
            match self.store.copy_opts(from, to, options).await {
                Ok(()) => return Ok(()),
                Err(
                    object_store::Error::NotSupported { .. }
                    | object_store::Error::NotImplemented { .. },
                ) => self.copy_create_unsupported.store(true, Ordering::Relaxed),
                Err(error) => return Err(Fail::Store(error)),
            }
        }
        if self.head(to).await?.is_some() {
            return Err(Fail::Store(object_store::Error::AlreadyExists {
                path: to.to_string(),
                source: "the object exists".into(),
            }));
        }
        self.store.copy(from, to).await?;
        Ok(())
    }

    /// Deletes `keys`; a key that is already gone counts as deleted.
    async fn delete_all(&self, keys: Vec<Path>) -> Result<(), Fail> {
        if keys.is_empty() {
            return Ok(());
        }
        let input = futures_util::stream::iter(keys.into_iter().map(Ok)).boxed();
        let results: Vec<_> = self.store.delete_stream(input).collect().await;
        for result in results {
            match result {
                Ok(_) | Err(object_store::Error::NotFound { .. }) => {}
                Err(error) => return Err(Fail::Store(error)),
            }
        }
        Ok(())
    }
}

/// The key named in an `object_store` error, when it has one.
fn key_in(fail: &Fail) -> Option<Path> {
    use object_store::Error as E;
    let Fail::Store(error) = fail else {
        return None;
    };
    let path = match error {
        E::NotFound { path, .. }
        | E::AlreadyExists { path, .. }
        | E::Precondition { path, .. }
        | E::NotModified { path, .. }
        | E::PermissionDenied { path, .. }
        | E::Unauthenticated { path, .. } => path,
        _ => return None,
    };
    Path::parse(path).ok()
}

/// An object-store drive. Cheap to share; every method blocks the calling thread.
pub struct ObjectStoreBackend {
    pub(crate) inner: Arc<Inner>,
    pub(crate) rt: Arc<Rt>,
    tuning: WriteTuning,
    max_server_copy: Option<u64>,
    label: String,
}

impl fmt::Debug for ObjectStoreBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObjectStoreBackend")
            .field("drive", &self.label)
            .field("prefix", &self.inner.keys.prefix())
            .field("capabilities", &Self::CAPABILITIES)
            .finish_non_exhaustive()
    }
}

impl ObjectStoreBackend {
    /// What every object-store drive has: the S3 column of the design's table,
    /// `MemoryBackend::object_store_like()`'s. They never change.
    pub const CAPABILITIES: Capabilities = Capabilities {
        trash: false,
        atomic_rename: false,
        server_side_copy: true,
        real_directories: false,
        posix_permissions: false,
        symlinks: false,
        watch: false,
        undo_rename: false,
        undo_move: false,
    };

    /// A drive over `store`. Starts its own runtime; no request is made.
    pub fn new(
        store: Arc<dyn ObjectStore>,
        options: ObjectStoreOptions,
    ) -> Result<ObjectStoreBackend, String> {
        let keys = Keys::new(&options.prefix)?;
        let timeout = options.timeout.max(Duration::from_millis(100));
        let rt = Rt::new(timeout.saturating_mul(4).saturating_add(Duration::from_secs(5)))?;
        Ok(ObjectStoreBackend {
            inner: Arc::new(Inner {
                store,
                pager: options.pager,
                keys,
                page_size: options.page_size.clamp(1, 1000),
                put_create_unsupported: AtomicBool::new(false),
                copy_create_unsupported: AtomicBool::new(false),
            }),
            rt: Arc::new(rt),
            tuning: WriteTuning {
                part_size: options.part_size.max(MIN_PART_SIZE),
                concurrency: options.upload_concurrency.max(1),
            },
            max_server_copy: options.max_server_copy,
            label: options.label,
        })
    }

    /// Like [`ObjectStoreBackend::new`], with parts smaller than S3's minimum
    /// allowed: for tests that need many parts without many megabytes.
    #[doc(hidden)]
    pub fn new_with_small_parts(
        store: Arc<dyn ObjectStore>,
        options: ObjectStoreOptions,
    ) -> Result<ObjectStoreBackend, String> {
        let part_size = options.part_size.max(1);
        let mut backend = ObjectStoreBackend::new(store, options)?;
        backend.tuning.part_size = part_size;
        Ok(backend)
    }

    /// The drive's key prefix (empty for a whole bucket).
    #[must_use]
    pub fn prefix(&self) -> &str {
        self.inner.keys.prefix()
    }

    /// The object key a path is stored under, for tools and tests.
    #[must_use]
    pub fn key_of(&self, path: &RemotePath) -> Option<Path> {
        self.inner.keys.raw(path).ok()
    }

    /// The cheap request a factory makes at connect time: the first key
    /// under the drive's prefix.
    #[cfg(any(feature = "s3", feature = "gcs"))]
    pub(crate) fn probe(&self, cancel: &Cancel) -> Result<(), Fail> {
        let root = self
            .inner
            .keys
            .raw(&RemotePath::root())
            .map_err(|_| Fail::Other(String::from("the prefix is not a valid key")))?;
        self.rt
            .block_on_cancellable(self.inner.first_under(&root, 1), cancel)?
            .map(|_| ())
    }

    fn block<T>(&self, future: impl Future<Output = Result<T, Fail>>) -> Result<T, Fail> {
        self.rt.block_on(future)?
    }

    /// The caller-visible path of an object key, or `fallback`.
    fn path_or(&self, key: Option<&Path>, fallback: &RemotePath) -> RemotePath {
        key.and_then(|key| self.inner.keys.path_of(key))
            .unwrap_or_else(|| fallback.clone())
    }

    /// The error of a failed operation on an object of a tree rooted at
    /// `path`: names the object the store complained about when it says
    /// which, and `path` otherwise.
    fn tree_error(&self, fail: &Fail, key: Option<&Path>, path: &RemotePath) -> BackendError {
        let named = key_in(fail);
        let at = self.path_or(named.as_ref().or(key), path);
        let key = self.inner.keys.raw(&at).ok();
        fail.at(&at, key.as_ref())
    }

    /// Copies one object by streaming it through the client (for objects the
    /// service will not copy by itself). Never overwrites.
    fn stream_copy(&self, from: &Path, to: &Path, size: u64, at: &RemotePath) -> Result<(), BackendError> {
        let mut reader = ObjectReader::open(&self.inner, &self.rt, at, from, 0, size)?;
        let mut session = ObjectWriteSession::new(
            Arc::clone(&self.inner),
            Arc::clone(&self.rt),
            at.clone(),
            to.clone(),
            false,
            self.tuning,
        );
        let copied = io::copy(&mut reader, &mut session);
        match copied {
            Ok(_) => Box::new(session).finish(),
            Err(error) => {
                let _ = Box::new(session).abort();
                Err(BackendError::from_io(error, Some(at)))
            }
        }
    }

    /// Whether this object needs a streamed copy.
    fn too_big_to_copy(&self, size: u64) -> bool {
        self.max_server_copy.is_some_and(|limit| size > limit)
    }

    /// What is at `path`: `Some(meta)` for an object, `None` for a folder.
    /// `Err(NotFound)` when neither.
    fn describe(&self, path: &RemotePath, key: &Path) -> Result<Option<ObjectMeta>, BackendError> {
        let inner = &self.inner;
        let head = self
            .block(inner.head(key))
            .map_err(|fail| fail.at(path, Some(key)))?;
        if head.is_some() {
            return Ok(head);
        }
        let children = self
            .block(inner.has_children(key))
            .map_err(|fail| fail.at(path, Some(key)))?;
        if children {
            Ok(None)
        } else {
            Err(not_found(path))
        }
    }

    /// The checks before anything is created at `path` (key `key`): nothing
    /// may be there (`replace` allows a file), and no ancestor may be an object.
    fn check_free(&self, path: &RemotePath, key: &Path, replace: bool) -> Result<(), BackendError> {
        let inner = &self.inner;
        let (head, children, ancestor) = self
            .rt
            .block_on(async {
                futures_util::join!(
                    inner.head(key),
                    inner.has_children(key),
                    inner.object_ancestor(path)
                )
            })
            .map_err(|fail| fail.at(path, Some(key)))?;
        let head = head.map_err(|fail| fail.at(path, Some(key)))?;
        let children = children.map_err(|fail| fail.at(path, Some(key)))?;
        let ancestor = ancestor.map_err(|fail| fail.at(path, Some(key)))?;
        if children || (head.is_some() && !replace) {
            return Err(already_exists(path));
        }
        if ancestor {
            return Err(other(path, io::ErrorKind::NotADirectory));
        }
        Ok(())
    }
}

/// Collects one folder's entries across pages, and decides who wins when an
/// object and a folder share a name.
struct Collector<'a> {
    dir: &'a RemotePath,
    dir_key: &'a Path,
    dir_prefix: String,
    files: std::collections::BTreeMap<String, ObjectMeta>,
    folders: std::collections::BTreeSet<String>,
    /// Anything at all was under the prefix (placeholders included).
    seen: bool,
}

impl<'a> Collector<'a> {
    fn new(dir: &'a RemotePath, dir_key: &'a Path) -> Collector<'a> {
        Collector {
            dir,
            dir_key,
            dir_prefix: Keys::dir_prefix(dir_key),
            files: std::collections::BTreeMap::new(),
            folders: std::collections::BTreeSet::new(),
            seen: false,
        }
    }

    fn name_of(&self, key: &Path) -> Option<String> {
        key.as_ref()
            .strip_prefix(self.dir_prefix.as_str())
            .map(|rest| rest.trim_end_matches('/').to_owned())
    }

    fn add(&mut self, page: Page) {
        for meta in page.objects {
            self.seen = true;
            // A `dir/` marker written by another tool is the folder itself.
            if meta.location == *self.dir_key {
                continue;
            }
            match self.name_of(&meta.location) {
                Some(name) if name == PLACEHOLDER || name.is_empty() => {}
                // Without a delimiter-aware store a deeper key would show up
                // here; it only proves its first segment is a folder.
                Some(name) => match name.split_once('/') {
                    Some((first, _)) => {
                        self.folders.insert(first.to_owned());
                    }
                    None => {
                        self.files.insert(name, meta);
                    }
                },
                None => {}
            }
        }
        for prefix in page.prefixes {
            self.seen = true;
            match self.name_of(&prefix) {
                Some(name) if name == PLACEHOLDER || name.is_empty() => {}
                Some(name) => {
                    self.folders.insert(name);
                }
                None => {}
            }
        }
    }

    fn finish(self) -> Listing {
        let mut listing = Listing::default();
        for (name, meta) in &self.files {
            match self.dir.join(name) {
                Ok(_) => listing.entries.push(file_entry(name, meta)),
                Err(_) => listing
                    .errors
                    .push(other(self.dir, io::ErrorKind::InvalidData)),
            }
        }
        for name in &self.folders {
            match self.dir.join(name) {
                Ok(child) if self.files.contains_key(name) => {
                    // Another tool stored an object and a folder under one
                    // name: the object is listed, the folder reported.
                    listing.errors.push(
                        BackendError::new(BackendErrorKind::Other, Some(child)).with_source(
                            io::Error::new(
                                io::ErrorKind::AlreadyExists,
                                "a folder with the same name as this object is hidden by it",
                            ),
                        ),
                    );
                }
                Ok(_) => listing.entries.push(dir_entry(name)),
                Err(_) => listing
                    .errors
                    .push(other(self.dir, io::ErrorKind::InvalidData)),
            }
        }
        listing
    }
}

impl Backend for ObjectStoreBackend {
    fn capabilities(&self) -> Capabilities {
        Self::CAPABILITIES
    }

    fn list(&self, dir: &RemotePath, cancel: &Cancel) -> Result<Listing, BackendError> {
        if cancel.is_cancelled() {
            return Err(cancelled(dir));
        }
        let Ok(key) = self.inner.keys.key(dir) else {
            return Err(not_found(dir));
        };
        let inner = &self.inner;
        let fail_at = |fail: Fail| match fail {
            Fail::Cancelled => cancelled(dir),
            fail => fail.at(dir, Some(&key)),
        };
        // The first page and the object check travel together.
        let (head, first) = self
            .rt
            .block_on_cancellable(
                async { futures_util::join!(inner.head(&key), inner.page(&key, true, None, None)) },
                cancel,
            )
            .map_err(fail_at)?;
        if head.map_err(fail_at)?.is_some() {
            return Err(other(dir, io::ErrorKind::NotADirectory));
        }
        let mut page = first.map_err(fail_at)?;
        let mut collector = Collector::new(dir, &key);
        loop {
            let next = page.next.take();
            collector.add(page);
            let Some(token) = next else {
                break;
            };
            if cancel.is_cancelled() {
                return Err(cancelled(dir));
            }
            page = self
                .rt
                .block_on_cancellable(inner.page(&key, true, Some(token), None), cancel)
                .map_err(fail_at)?
                .map_err(fail_at)?;
        }
        if cancel.is_cancelled() {
            return Err(cancelled(dir));
        }
        if !dir.is_root() && !collector.seen {
            return Err(not_found(dir));
        }
        Ok(collector.finish())
    }

    fn stat(&self, path: &RemotePath) -> Result<FileEntry, BackendError> {
        if path.is_root() {
            return Ok(dir_entry("/"));
        }
        let Ok(key) = self.inner.keys.key(path) else {
            return Err(not_found(path));
        };
        let name = path.file_name().unwrap_or("/");
        match self.describe(path, &key)? {
            Some(meta) => Ok(file_entry(name, &meta)),
            None => Ok(dir_entry(name)),
        }
    }

    fn open_read(
        &self,
        path: &RemotePath,
        from: u64,
    ) -> Result<Box<dyn Read + Send>, BackendError> {
        if path.is_root() {
            return Err(other(path, io::ErrorKind::IsADirectory));
        }
        let Ok(key) = self.inner.keys.key(path) else {
            return Err(not_found(path));
        };
        let Some(meta) = self.describe(path, &key)? else {
            return Err(other(path, io::ErrorKind::IsADirectory));
        };
        if from > meta.size {
            return Err(other(path, io::ErrorKind::InvalidInput));
        }
        let reader = ObjectReader::open(&self.inner, &self.rt, path, &key, from, meta.size)?;
        Ok(Box::new(reader))
    }

    fn begin_write(
        &self,
        path: &RemotePath,
        _size_hint: Option<u64>,
        replace: bool,
    ) -> Result<Box<dyn WriteSession>, BackendError> {
        if path.is_root() {
            return Err(already_exists(path));
        }
        let key = self.inner.keys.key(path).map_err(|_| no_key_for_create(path))?;
        self.check_free(path, &key, replace)?;
        Ok(Box::new(ObjectWriteSession::new(
            Arc::clone(&self.inner),
            Arc::clone(&self.rt),
            path.clone(),
            key,
            replace,
            self.tuning,
        )))
    }

    fn create_dir(&self, path: &RemotePath) -> Result<(), BackendError> {
        if path.is_root() {
            return Err(already_exists(path));
        }
        let key = self.inner.keys.key(path).map_err(|_| no_key_for_create(path))?;
        self.check_free(path, &key, false)?;
        let marker = join(&key, PLACEHOLDER);
        self.block(self.inner.put_new(&marker, PutPayload::new()))
            .map_err(|fail| fail.at(path, Some(&marker)))
    }

    fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        if from.is_root() {
            return Err(other(from, io::ErrorKind::InvalidInput));
        }
        if to.is_root() {
            return Err(other(to, io::ErrorKind::InvalidInput));
        }
        let Ok(src) = self.inner.keys.key(from) else {
            return Err(not_found(from));
        };
        let inner = &self.inner;
        let (head, children) = self
            .rt
            .block_on(async { futures_util::join!(inner.head(&src), inner.all_under(&src)) })
            .map_err(|fail| fail.at(from, Some(&src)))?;
        let head = head.map_err(|fail| fail.at(from, Some(&src)))?;
        let children = children.map_err(|fail| fail.at(from, Some(&src)))?;
        if head.is_none() && children.is_empty() {
            return Err(not_found(from));
        }
        if from == to {
            return Ok(());
        }
        let is_dir = head.is_none();
        if is_dir && to.starts_with(from) {
            return Err(other(to, io::ErrorKind::InvalidInput));
        }
        let dst = self.inner.keys.key(to).map_err(|_| no_key_for_create(to))?;
        self.check_free(to, &dst, false)?;

        // Every object of the source, and where it goes. A `dir/` marker
        // written by another tool cannot be copied through object_store and
        // stays behind.
        let src_len = src.as_ref().len();
        let moves: Vec<(Path, Path, u64)> = head
            .into_iter()
            .chain(children.into_iter().filter(|meta| meta.location != src))
            .filter_map(|meta| {
                let suffix = meta.location.as_ref().get(src_len..)?;
                let target = Path::parse(format!("{}{suffix}", dst.as_ref())).ok()?;
                Some((meta.location, target, meta.size))
            })
            .collect();

        // Phase 1: copy everything; on a failure, take the copies back.
        let mut copied: Vec<Path> = Vec::new();
        let mut failure: Option<(Fail, Path)> = None;
        let (small, big): (Vec<_>, Vec<_>) = moves
            .iter()
            .partition(|(_, _, size)| !self.too_big_to_copy(*size));
        for batch in small.chunks(COPY_BATCH) {
            let results = self.rt.block_on(join_all(
                batch.iter().map(|(from_key, to_key, _)| inner.copy_new(from_key, to_key)),
            ));
            match results {
                Ok(results) => {
                    for ((from_key, to_key, _), result) in batch.iter().zip(results) {
                        match result {
                            Ok(()) => copied.push(to_key.clone()),
                            Err(fail) if failure.is_none() => failure = Some((fail, from_key.clone())),
                            Err(_) => {}
                        }
                    }
                }
                Err(fail) => failure = Some((fail, batch.first().map(|m| m.0.clone()).unwrap_or_else(|| src.clone()))),
            }
            if failure.is_some() {
                break;
            }
        }
        let mut streamed_failure = None;
        if failure.is_none() {
            for (from_key, to_key, size) in &big {
                let at = self.path_or(Some(from_key), from);
                match self.stream_copy(from_key, to_key, *size, &at) {
                    Ok(()) => copied.push(to_key.clone()),
                    Err(error) => {
                        streamed_failure = Some(error);
                        break;
                    }
                }
            }
        }
        if failure.is_some() || streamed_failure.is_some() {
            // Best effort: a copy that cannot be taken back leaves the object
            // under both names, never under neither.
            for batch in copied.chunks(DELETE_BATCH) {
                let _ = self.block(inner.delete_all(batch.to_vec()));
            }
            if let Some(error) = streamed_failure {
                return Err(error);
            }
            if let Some((fail, key)) = failure {
                return Err(self.tree_error(&fail, Some(&key), from));
            }
        }

        // Phase 2: every copy exists; delete the sources, files before the
        // placeholders that keep their folders alive.
        let (mut placeholders, files): (Vec<Path>, Vec<Path>) = moves
            .into_iter()
            .map(|(from_key, _, _)| from_key)
            .partition(is_placeholder);
        placeholders.sort_by_key(|key| std::cmp::Reverse(key.as_ref().len()));
        for batch in files.chunks(DELETE_BATCH).chain(placeholders.chunks(DELETE_BATCH)) {
            self.block(inner.delete_all(batch.to_vec()))
                .map_err(|fail| self.tree_error(&fail, batch.first(), from))?;
        }
        Ok(())
    }

    fn remove(&self, path: &RemotePath) -> Result<(), BackendError> {
        if path.is_root() {
            return Err(other(path, io::ErrorKind::InvalidInput));
        }
        let Ok(key) = self.inner.keys.key(path) else {
            return Err(not_found(path));
        };
        let inner = &self.inner;
        if self
            .block(inner.head(&key))
            .map_err(|fail| fail.at(path, Some(&key)))?
            .is_some()
        {
            return self
                .block(inner.delete_all(vec![key.clone()]))
                .map_err(|fail| fail.at(path, Some(&key)));
        }
        let marker = join(&key, PLACEHOLDER);
        let under = self
            .block(inner.first_under(&key, 3))
            .map_err(|fail| fail.at(path, Some(&key)))?;
        if under.is_empty() {
            return Err(not_found(path));
        }
        if under
            .iter()
            .any(|meta| meta.location != marker && meta.location != key)
        {
            return Err(other(path, io::ErrorKind::DirectoryNotEmpty));
        }
        if !under.iter().any(|meta| meta.location == marker) {
            // Only a `dir/` marker of another tool: object_store cannot name it.
            return Err(error(BackendErrorKind::Unsupported, path, io::ErrorKind::Unsupported));
        }
        self.block(inner.delete_all(vec![marker.clone()]))
            .map_err(|fail| fail.at(path, Some(&marker)))
    }

    fn remove_tree(&self, path: &RemotePath, cancel: &Cancel) -> Result<(), BackendError> {
        if cancel.is_cancelled() {
            return Err(cancelled(path));
        }
        if path.is_root() {
            return Err(other(path, io::ErrorKind::InvalidInput));
        }
        let Ok(key) = self.inner.keys.key(path) else {
            return Err(not_found(path));
        };
        let inner = &self.inner;
        let fail_at = |fail: Fail| match fail {
            Fail::Cancelled => cancelled(path),
            fail => fail.at(path, Some(&key)),
        };
        let head = self
            .rt
            .block_on_cancellable(inner.head(&key), cancel)
            .map_err(fail_at)?
            .map_err(fail_at)?;
        let mut found = head.is_some();
        let mut placeholders = Vec::new();
        let mut token = None;
        // Page by page: delete what the page holds before asking for the
        // next. Placeholders wait until every child is gone.
        loop {
            if cancel.is_cancelled() {
                return Err(cancelled(path));
            }
            let page = self
                .rt
                .block_on_cancellable(inner.page(&key, false, token, None), cancel)
                .map_err(fail_at)?
                .map_err(fail_at)?;
            found |= !page.objects.is_empty();
            let (marks, files): (Vec<Path>, Vec<Path>) = page
                .objects
                .into_iter()
                .map(|meta| meta.location)
                .filter(|location| *location != key)
                .partition(is_placeholder);
            placeholders.extend(marks);
            if cancel.is_cancelled() {
                return Err(cancelled(path));
            }
            for batch in files.chunks(DELETE_BATCH) {
                self.block(inner.delete_all(batch.to_vec()))
                    .map_err(|fail| self.tree_error(&fail, batch.first(), path))?;
            }
            match page.next {
                Some(next) => token = Some(next),
                None => break,
            }
        }
        if !found {
            return Err(not_found(path));
        }
        if cancel.is_cancelled() {
            return Err(cancelled(path));
        }
        // Deepest folders first, then the object at the path itself.
        placeholders.sort_by_key(|key| std::cmp::Reverse(key.as_ref().len()));
        for batch in placeholders.chunks(DELETE_BATCH) {
            self.block(inner.delete_all(batch.to_vec()))
                .map_err(|fail| self.tree_error(&fail, batch.first(), path))?;
        }
        if head.is_some() {
            self.block(inner.delete_all(vec![key.clone()]))
                .map_err(|fail| fail.at(path, Some(&key)))?;
        }
        Ok(())
    }

    fn copy_within(&self, from: &RemotePath, to: &RemotePath) -> Result<(), BackendError> {
        let Ok(src) = self.inner.keys.key(from) else {
            return Err(not_found(from));
        };
        if from.is_root() {
            return Err(error(BackendErrorKind::Unsupported, from, io::ErrorKind::IsADirectory));
        }
        let Some(meta) = self.describe(from, &src)? else {
            return Err(error(BackendErrorKind::Unsupported, from, io::ErrorKind::IsADirectory));
        };
        if to.is_root() {
            return Err(already_exists(to));
        }
        let dst = self.inner.keys.key(to).map_err(|_| no_key_for_create(to))?;
        self.check_free(to, &dst, false)?;
        if self.too_big_to_copy(meta.size) {
            return self.stream_copy(&src, &dst, meta.size, to);
        }
        self.block(self.inner.copy_new(&src, &dst))
            .map_err(|fail| match fail.kind() {
                BackendErrorKind::NotFound if fail.is_not_found() && key_in(&fail).as_ref() == Some(&src) => {
                    fail.at(from, Some(&src))
                }
                _ => fail.at(to, Some(&dst)),
            })
    }
}

impl Write for ObjectWriteSession {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.write_bytes(data)
    }

    /// Flushing never commits.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
