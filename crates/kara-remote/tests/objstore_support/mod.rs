//! Shared by the object-store tests: a fault-injecting `ObjectStore` over
//! `object_store::memory::InMemory`, with call counts, and small helpers.
//!
//! `Faulty` implements both `ObjectStore` and `PaginatedListStore`, so the
//! backend lists it page by page (pages of `page_size` keys) exactly as it
//! lists S3 and GCS. Faults target one operation, let `skip` calls through,
//! then fire `times` times:
//!
//! - fail with an `object_store` error shaped like the real client's
//!   (connection refused, 503 after retries, 403, 401, 507, a lost body);
//! - stall for a while (a hung endpoint);
//! - for multipart: fail a part, the completion or the abort.
//!
//! It also counts every call and every multipart upload still open, can
//! pretend not to support conditional puts or copy-if-not-exists, and can be
//! "unplugged": every call fails as a dead endpoint does.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use kara_remote::objstore::object_store::client::{HttpError, HttpErrorKind};
use kara_remote::objstore::object_store::list::{
    PaginatedListOptions, PaginatedListResult, PaginatedListStore,
};
use kara_remote::objstore::object_store::memory::InMemory;
use kara_remote::objstore::object_store::path::Path;
use kara_remote::objstore::object_store::{
    self, CopyMode, CopyOptions, GetOptions, GetResult, GetResultPayload, ListResult,
    MultipartUpload, ObjectMeta, ObjectStore, ObjectStoreExt, PutMode, PutMultipartOptions,
    PutOptions, PutPayload, PutResult, Result as OsResult, UploadPart,
};
use kara_remote::objstore::{ObjectStoreBackend, ObjectStoreOptions};
use kara_vfs::RemotePath;

pub fn rp(path: &str) -> io::Result<RemotePath> {
    RemotePath::parse(path).map_err(|e| io::Error::other(e.to_string()))
}

/// The operations a fault can target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    Head,
    Get,
    /// A piece of a `GET` body after the first.
    Body,
    Put,
    StartUpload,
    Part,
    Complete,
    Abort,
    List,
    Copy,
    Delete,
}

/// What a fault does.
#[derive(Debug, Clone)]
pub enum Effect {
    /// Connection refused, as the HTTP client reports it after its retries.
    Refused,
    /// `503 SlowDown` after the client's retries.
    ServerError,
    /// 403.
    Denied,
    /// 401.
    Unauthenticated,
    /// 507, MinIO's «storage full».
    Full,
    /// 404 `NoSuchBucket`.
    NoSuchBucket,
    /// Waits this long, then goes on normally.
    Stall(Duration),
    /// Waits this long, then fails as a timeout.
    Hang(Duration),
}

#[derive(Debug, Clone)]
pub struct Fault {
    pub op: Op,
    /// Calls let through before the fault fires.
    pub skip: usize,
    pub times: usize,
    pub effect: Effect,
    /// Only for keys that start with this.
    pub key_prefix: Option<String>,
}

impl Fault {
    pub fn on(op: Op, effect: Effect) -> Fault {
        Fault {
            op,
            skip: 0,
            times: 1,
            effect,
            key_prefix: None,
        }
    }

    pub fn after(mut self, skip: usize) -> Fault {
        self.skip = skip;
        self
    }

    pub fn times(mut self, times: usize) -> Fault {
        self.times = times;
        self
    }

    pub fn always(mut self) -> Fault {
        self.times = usize::MAX;
        self
    }

    pub fn under(mut self, prefix: &str) -> Fault {
        self.key_prefix = Some(prefix.to_owned());
        self
    }
}

#[derive(Default)]
pub struct Counts {
    pub head: AtomicUsize,
    pub get: AtomicUsize,
    pub body_bytes: AtomicUsize,
    pub put: AtomicUsize,
    pub start_upload: AtomicUsize,
    pub part: AtomicUsize,
    pub complete: AtomicUsize,
    pub abort: AtomicUsize,
    pub list: AtomicUsize,
    pub copy: AtomicUsize,
    pub delete: AtomicUsize,
}

impl Counts {
    fn of(&self, op: Op) -> &AtomicUsize {
        match op {
            Op::Head => &self.head,
            Op::Get => &self.get,
            Op::Body => &self.body_bytes,
            Op::Put => &self.put,
            Op::StartUpload => &self.start_upload,
            Op::Part => &self.part,
            Op::Complete => &self.complete,
            Op::Abort => &self.abort,
            Op::List => &self.list,
            Op::Copy => &self.copy,
            Op::Delete => &self.delete,
        }
    }

    pub fn get(&self, op: Op) -> usize {
        self.of(op).load(Ordering::SeqCst)
    }
}

/// `InMemory` with faults, counts and paged listing.
pub struct Faulty {
    me: std::sync::Weak<Faulty>,
    pub inner: InMemory,
    pub counts: Counts,
    faults: Mutex<Vec<Fault>>,
    /// `PutMode::Create` works (otherwise `NotImplemented`).
    pub conditional_put: AtomicBool,
    /// `CopyMode::Create` works (otherwise `NotSupported`).
    pub copy_create: AtomicBool,
    /// Every call fails as a dead endpoint.
    pub unplugged: AtomicBool,
    /// Multipart uploads started and neither completed nor aborted.
    pub open_uploads: Arc<AtomicIsize>,
    /// Keys per listing page.
    pub page_size: AtomicUsize,
    /// Every key ever written with `put` or a completed upload, in order.
    pub written: Mutex<Vec<String>>,
    /// Delay before each page of a listing.
    pub list_delay_ms: AtomicUsize,
    /// Delay before each delete.
    pub delete_delay_ms: AtomicUsize,
    /// A `GET` body ends cleanly after this many 64 KiB pieces (a server
    /// that closes the connection early).
    pub truncate_body_after: AtomicUsize,
}

impl fmt::Debug for Faulty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Faulty")
    }
}

impl fmt::Display for Faulty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Faulty")
    }
}

pub fn effect_error(effect: &Effect, key: &str) -> object_store::Error {
    match effect {
        Effect::Refused => object_store::Error::Generic {
            store: "Faulty",
            source: Box::new(HttpError::new(
                HttpErrorKind::Connect,
                io::Error::new(io::ErrorKind::ConnectionRefused, "tcp connect error: Connection refused"),
            )),
        },
        Effect::ServerError => object_store::Error::Generic {
            store: "Faulty",
            source: format!(
                "Error performing PUT http://127.0.0.1:1/bucket/{key} in 1.2s, after 3 retries - Server returned non-2xx status code: 503 Service Unavailable: <Error><Code>SlowDown</Code></Error>"
            )
            .into(),
        },
        Effect::Denied => object_store::Error::PermissionDenied {
            path: key.to_owned(),
            source: "AccessDenied".into(),
        },
        Effect::Unauthenticated => object_store::Error::Unauthenticated {
            path: key.to_owned(),
            source: "InvalidAccessKeyId".into(),
        },
        Effect::Full => object_store::Error::Generic {
            store: "Faulty",
            source: "Server returned non-2xx status code: 507 Insufficient Storage: <Error><Code>XMinioStorageFull</Code></Error>"
                .into(),
        },
        Effect::NoSuchBucket => object_store::Error::NotFound {
            path: key.to_owned(),
            source: "NoSuchBucket: The specified bucket does not exist".into(),
        },
        Effect::Stall(_) | Effect::Hang(_) => object_store::Error::Generic {
            store: "Faulty",
            source: Box::new(HttpError::new(
                HttpErrorKind::Timeout,
                io::Error::new(io::ErrorKind::TimedOut, "operation timed out"),
            )),
        },
    }
}

impl Faulty {
    pub fn new() -> Arc<Faulty> {
        Arc::new_cyclic(|me| Faulty {
            me: me.clone(),
            inner: InMemory::new(),
            counts: Counts::default(),
            faults: Mutex::new(Vec::new()),
            conditional_put: AtomicBool::new(true),
            copy_create: AtomicBool::new(true),
            unplugged: AtomicBool::new(false),
            open_uploads: Arc::new(AtomicIsize::new(0)),
            page_size: AtomicUsize::new(1000),
            written: Mutex::new(Vec::new()),
            list_delay_ms: AtomicUsize::new(0),
            delete_delay_ms: AtomicUsize::new(0),
            truncate_body_after: AtomicUsize::new(usize::MAX),
        })
    }

    pub fn inject(&self, fault: Fault) {
        if let Ok(mut faults) = self.faults.lock() {
            faults.push(fault);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut faults) = self.faults.lock() {
            faults.clear();
        }
    }

    /// Counts the call and returns the fault that fires on it, if any.
    fn take(&self, op: Op, key: &str) -> Option<Effect> {
        self.counts.of(op).fetch_add(1, Ordering::SeqCst);
        if self.unplugged.load(Ordering::SeqCst) {
            return Some(Effect::Refused);
        }
        let mut faults = self.faults.lock().ok()?;
        let index = faults.iter().position(|fault| {
            fault.op == op
                && fault
                    .key_prefix
                    .as_deref()
                    .is_none_or(|prefix| key.starts_with(prefix))
        })?;
        let fault = faults.get_mut(index)?;
        if fault.skip > 0 {
            fault.skip -= 1;
            return None;
        }
        let effect = fault.effect.clone();
        fault.times = fault.times.saturating_sub(1);
        if fault.times == 0 {
            faults.remove(index);
        }
        Some(effect)
    }

    /// Applies the fault of this call: `Err` to fail it, `Ok` to go on.
    async fn gate(&self, op: Op, key: &str) -> OsResult<()> {
        match self.take(op, key) {
            None => Ok(()),
            Some(Effect::Stall(delay)) => {
                tokio::time::sleep(delay).await;
                Ok(())
            }
            Some(Effect::Hang(delay)) => {
                tokio::time::sleep(delay).await;
                Err(effect_error(&Effect::Hang(delay), key))
            }
            Some(effect) => Err(effect_error(&effect, key)),
        }
    }

    /// Every key stored, sorted.
    pub fn keys(&self) -> Vec<String> {
        let rt = tokio::runtime::Builder::new_current_thread().build();
        let Ok(rt) = rt else {
            return Vec::new();
        };
        rt.block_on(async {
            let mut keys: Vec<String> = self
                .inner
                .list(None)
                .filter_map(|meta| async move { meta.ok().map(|m| m.location.to_string()) })
                .collect()
                .await;
            keys.sort();
            keys
        })
    }

    /// The bytes of a key, if it exists.
    pub fn bytes(&self, key: &str) -> Option<Vec<u8>> {
        let rt = tokio::runtime::Builder::new_current_thread().build().ok()?;
        let path = Path::parse(key).ok()?;
        rt.block_on(async {
            let got = self.inner.get(&path).await.ok()?;
            got.bytes().await.ok().map(|b| b.to_vec())
        })
    }

    /// Writes a key directly (as another tool would).
    pub fn put_raw(&self, key: &str, data: &[u8]) -> io::Result<()> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(io::Error::other)?;
        let path = Path::parse(key).map_err(|e| io::Error::other(e.to_string()))?;
        rt.block_on(self.inner.put(&path, PutPayload::from(data.to_vec())))
            .map(|_| ())
            .map_err(|e| io::Error::other(e.to_string()))
    }

    pub fn open_uploads(&self) -> isize {
        self.open_uploads.load(Ordering::SeqCst)
    }
}

/// A backend over `store`, listing page by page through it.
pub fn backend_over(store: &Arc<Faulty>, options: ObjectStoreOptions) -> io::Result<ObjectStoreBackend> {
    let pager: Arc<dyn PaginatedListStore> = Arc::clone(store) as _;
    let os: Arc<dyn ObjectStore> = Arc::clone(store) as _;
    ObjectStoreBackend::new(
        os,
        ObjectStoreOptions {
            pager: Some(pager),
            ..options
        },
    )
    .map_err(io::Error::other)
}

/// Same, with parts as small as `part_size` (below S3's minimum), for tests.
pub fn backend_with_parts(
    store: &Arc<Faulty>,
    part_size: usize,
    concurrency: usize,
) -> io::Result<ObjectStoreBackend> {
    let pager: Arc<dyn PaginatedListStore> = Arc::clone(store) as _;
    let os: Arc<dyn ObjectStore> = Arc::clone(store) as _;
    ObjectStoreBackend::new_with_small_parts(
        os,
        ObjectStoreOptions {
            pager: Some(pager),
            part_size,
            upload_concurrency: concurrency,
            ..ObjectStoreOptions::default()
        },
    )
    .map_err(io::Error::other)
}

/// A backend over `store` with default options and paged listing.
pub fn faulty_backend(store: &Arc<Faulty>) -> io::Result<ObjectStoreBackend> {
    backend_over(store, ObjectStoreOptions::default())
}

#[derive(Debug)]
struct FaultyUpload {
    store: Arc<Faulty>,
    key: String,
    inner: Box<dyn MultipartUpload>,
    open: bool,
}

#[async_trait]
impl MultipartUpload for FaultyUpload {
    fn put_part(&mut self, data: PutPayload) -> UploadPart {
        let fault = self.store.take(Op::Part, &self.key);
        let key = self.key.clone();
        let next = self.inner.put_part(data);
        Box::pin(async move {
            match fault {
                None => next.await,
                Some(Effect::Stall(delay)) => {
                    tokio::time::sleep(delay).await;
                    next.await
                }
                Some(effect) => {
                    if let Effect::Hang(delay) = effect {
                        tokio::time::sleep(delay).await;
                    }
                    drop(next);
                    Err(effect_error(&effect, &key))
                }
            }
        })
    }

    async fn complete(&mut self) -> OsResult<PutResult> {
        self.store.gate(Op::Complete, &self.key).await?;
        let result = self.inner.complete().await?;
        if self.open {
            self.open = false;
            self.store.open_uploads.fetch_sub(1, Ordering::SeqCst);
        }
        if let Ok(mut written) = self.store.written.lock() {
            written.push(self.key.clone());
        }
        Ok(result)
    }

    async fn abort(&mut self) -> OsResult<()> {
        self.store.gate(Op::Abort, &self.key).await?;
        self.inner.abort().await?;
        if self.open {
            self.open = false;
            self.store.open_uploads.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// Wraps a body so it can fail after some bytes.
fn faulty_body(
    store: Arc<Faulty>,
    key: String,
    body: BoxStream<'static, OsResult<bytes::Bytes>>,
) -> BoxStream<'static, OsResult<bytes::Bytes>> {
    // Re-chunk in 64 KiB pieces so a fault can land mid-body.
    let keep = store.truncate_body_after.load(Ordering::SeqCst);
    body.flat_map(|piece| {
        let pieces: Vec<OsResult<bytes::Bytes>> = match piece {
            Ok(bytes) => {
                let mut out = Vec::new();
                let mut rest = bytes;
                while rest.len() > 65_536 {
                    out.push(Ok(rest.split_to(65_536)));
                }
                if !rest.is_empty() {
                    out.push(Ok(rest));
                }
                out
            }
            Err(e) => vec![Err(e)],
        };
        futures::stream::iter(pieces)
    })
    .take(keep)
    .enumerate()
    .then(move |(index, piece)| {
        let store = Arc::clone(&store);
        let key = key.clone();
        async move {
            if index > 0 {
                store.gate(Op::Body, &key).await?;
            }
            piece
        }
    })
    .boxed()
}

#[async_trait]
impl ObjectStore for Faulty {
    async fn put_opts(&self, location: &Path, payload: PutPayload, opts: PutOptions) -> OsResult<PutResult> {
        self.gate(Op::Put, location.as_ref()).await?;
        if opts.mode == PutMode::Create && !self.conditional_put.load(Ordering::SeqCst) {
            return Err(object_store::Error::NotImplemented {
                operation: String::from("put with PutMode::Create"),
                implementer: String::from("Faulty"),
            });
        }
        let result = self.inner.put_opts(location, payload, opts).await?;
        if let Ok(mut written) = self.written.lock() {
            written.push(location.to_string());
        }
        Ok(result)
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> OsResult<Box<dyn MultipartUpload>> {
        self.gate(Op::StartUpload, location.as_ref()).await?;
        let inner = self.inner.put_multipart_opts(location, opts).await?;
        self.open_uploads.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FaultyUpload {
            store: self.self_arc(),
            key: location.to_string(),
            inner,
            open: true,
        }))
    }

    async fn get_opts(&self, location: &Path, options: GetOptions) -> OsResult<GetResult> {
        let op = if options.head { Op::Head } else { Op::Get };
        self.gate(op, location.as_ref()).await?;
        let mut result = self.inner.get_opts(location, options).await?;
        if op == Op::Get {
            let payload = std::mem::replace(
                &mut result.payload,
                GetResultPayload::Stream(futures::stream::empty().boxed()),
            );
            #[allow(irrefutable_let_patterns)]
            if let GetResultPayload::Stream(body) = payload {
                result.payload =
                    GetResultPayload::Stream(faulty_body(self.self_arc(), location.to_string(), body));
            }
        }
        Ok(result)
    }

    async fn get_ranges(&self, location: &Path, ranges: &[Range<u64>]) -> OsResult<Vec<bytes::Bytes>> {
        self.gate(Op::Get, location.as_ref()).await?;
        self.inner.get_ranges(location, ranges).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, OsResult<Path>>,
    ) -> BoxStream<'static, OsResult<Path>> {
        let store = self.self_arc();
        locations
            .then(move |location| {
                let store = Arc::clone(&store);
                async move {
                    let location = location?;
                    let delay = store.delete_delay_ms.load(Ordering::SeqCst);
                    if delay > 0 {
                        tokio::time::sleep(Duration::from_millis(delay as u64)).await;
                    }
                    store.gate(Op::Delete, location.as_ref()).await?;
                    store.inner.delete(&location).await?;
                    Ok(location)
                }
            })
            .boxed()
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, OsResult<ObjectMeta>> {
        let key = prefix.map(|p| p.to_string()).unwrap_or_default();
        if let Some(effect) = self.take(Op::List, &key) {
            let error = effect_error(&effect, &key);
            return futures::stream::once(async move { Err(error) }).boxed();
        }
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> OsResult<ListResult> {
        let key = prefix.map(|p| p.to_string()).unwrap_or_default();
        self.gate(Op::List, &key).await?;
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(&self, from: &Path, to: &Path, options: CopyOptions) -> OsResult<()> {
        self.gate(Op::Copy, from.as_ref()).await?;
        if options.mode == CopyMode::Create && !self.copy_create.load(Ordering::SeqCst) {
            return Err(object_store::Error::NotSupported {
                source: "copy-if-not-exists is not supported".into(),
            });
        }
        self.inner.copy_opts(from, to, options).await
    }
}

impl Faulty {
    /// `self` as the `Arc` it lives in (`Faulty::new` builds it cyclically).
    fn self_arc(&self) -> Arc<Faulty> {
        self.me.upgrade().unwrap_or_else(Faulty::new)
    }
}

#[async_trait]
impl PaginatedListStore for Faulty {
    async fn list_paginated(
        &self,
        prefix: Option<&str>,
        opts: PaginatedListOptions,
    ) -> OsResult<PaginatedListResult> {
        let raw = prefix.unwrap_or("");
        self.gate(Op::List, raw).await?;
        let delay = self.list_delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay as u64)).await;
        }
        let dir = raw.trim_end_matches('/');
        let path = if dir.is_empty() {
            None
        } else {
            Some(Path::parse(dir).map_err(|e| object_store::Error::Generic {
                store: "Faulty",
                source: Box::new(e),
            })?)
        };
        // Every item, objects and common prefixes, in key order.
        enum Item {
            Object(ObjectMeta),
            Prefix(Path),
        }
        let mut items: Vec<(String, Item)> = Vec::new();
        if opts.delimiter.is_some() {
            let all = self.inner.list_with_delimiter(path.as_ref()).await?;
            for meta in all.objects {
                items.push((meta.location.to_string(), Item::Object(meta)));
            }
            for prefix in all.common_prefixes {
                items.push((format!("{prefix}/"), Item::Prefix(prefix)));
            }
        } else {
            let all: Vec<ObjectMeta> = self.inner.list(path.as_ref()).try_collect_vec().await?;
            for meta in all {
                items.push((meta.location.to_string(), Item::Object(meta)));
            }
        }
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let after = opts.page_token.clone().unwrap_or_default();
        let max = opts
            .max_keys
            .unwrap_or(1000)
            .min(self.page_size.load(Ordering::SeqCst))
            .max(1);
        let mut page: VecDeque<(String, Item)> = items
            .into_iter()
            .filter(|(key, _)| after.is_empty() || key.as_str() > after.as_str())
            .collect();
        let more = page.len() > max;
        page.truncate(max);
        let next = if more { page.back().map(|(key, _)| key.clone()) } else { None };
        let mut result = ListResult {
            common_prefixes: Vec::new(),
            objects: Vec::new(),
            extensions: Default::default(),
        };
        for (_, item) in page {
            match item {
                Item::Object(meta) => result.objects.push(meta),
                Item::Prefix(prefix) => result.common_prefixes.push(prefix),
            }
        }
        Ok(PaginatedListResult {
            result,
            page_token: next,
        })
    }
}

trait TryCollectVec<T> {
    async fn try_collect_vec(self) -> OsResult<Vec<T>>;
}

impl<T: Send, S: futures::Stream<Item = OsResult<T>> + Send> TryCollectVec<T> for S {
    async fn try_collect_vec(self) -> OsResult<Vec<T>> {
        let mut out = Vec::new();
        let mut stream = std::pin::pin!(self);
        while let Some(item) = stream.next().await {
            out.push(item?);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Prompts, drive configs, fake factories and kara-ops jobs.

use kara_remote::objstore::object_store::list::PaginatedListStore as Pager;
use kara_remote::objstore::{GcsFactory, S3Factory};
use kara_remote::{DriveConfig, Prompt, PromptAnswer, PromptHandler, Secret};

/// Answers prompts from a script and records what was asked.
#[derive(Default)]
pub struct Scripted {
    pub answers: Mutex<VecDeque<PromptAnswer>>,
    pub asked: Mutex<Vec<Prompt>>,
}

impl Scripted {
    pub fn new(answers: impl IntoIterator<Item = PromptAnswer>) -> Scripted {
        Scripted {
            answers: Mutex::new(answers.into_iter().collect()),
            asked: Mutex::new(Vec::new()),
        }
    }

    pub fn asked(&self) -> Vec<Prompt> {
        self.asked.lock().map(|a| a.clone()).unwrap_or_default()
    }
}

impl PromptHandler for Scripted {
    fn ask(&self, prompt: &Prompt) -> PromptAnswer {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(prompt.clone());
        }
        self.answers
            .lock()
            .ok()
            .and_then(|mut answers| answers.pop_front())
            .unwrap_or(PromptAnswer::Refuse)
    }
}

/// The only secret access key the fake S3 accepts.
pub const RIGHT_SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

pub fn s3_config(name: &str, params: &[(&str, &str)]) -> io::Result<DriveConfig> {
    DriveConfig::new(
        "s3",
        name,
        name,
        params.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    )
    .map_err(|e| io::Error::other(e.to_string()))
}

pub fn gcs_config(name: &str, params: &[(&str, &str)]) -> io::Result<DriveConfig> {
    DriveConfig::new(
        "gcs",
        name,
        name,
        params.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    )
    .map_err(|e| io::Error::other(e.to_string()))
}

/// An S3 factory whose «service» is `store`: a wrong secret makes every
/// request answer 401, the right one ([`RIGHT_SECRET`]) lets them through.
pub fn fake_s3_factory(store: &Arc<Faulty>) -> Arc<S3Factory> {
    let store = Arc::clone(store);
    S3Factory::with_connector(Arc::new(move |_params, secret: Option<&Secret>| {
        let ok = secret.is_some_and(|s| s.expose() == RIGHT_SECRET);
        let wrapped = if ok {
            Arc::clone(&store)
        } else {
            let denied = Faulty::new();
            denied.inject(Fault::on(Op::List, Effect::Unauthenticated).always());
            denied
        };
        let pager: Arc<dyn Pager> = Arc::clone(&wrapped) as _;
        let os: Arc<dyn ObjectStore> = wrapped as _;
        Ok((os, Some(pager)))
    }))
}

/// A GCS factory over `store`.
pub fn fake_gcs_factory(store: &Arc<Faulty>) -> Arc<GcsFactory> {
    let store = Arc::clone(store);
    GcsFactory::with_connector(Arc::new(move |_params| {
        let pager: Arc<dyn Pager> = Arc::clone(&store) as _;
        let os: Arc<dyn ObjectStore> = Arc::clone(&store) as _;
        Ok((os, Some(pager)))
    }))
}
