//! Reading and writing object contents.
//!
//! **Reads** are one ranged `GET` from the requested offset, streamed: the
//! blocking `Read` waits for the next piece the HTTP body yields. A body that
//! ends before the object's size is `Unavailable`, never a silent short file.
//!
//! **Writes** are buffered. A file smaller than one part is a single `PUT` at
//! [`WriteSession::finish`]; a bigger one becomes a multipart upload, started
//! when the first part is full, with up to `concurrency` parts on their way
//! while the caller keeps writing. `finish` sends the tail, waits for **every**
//! part and only then completes the upload: nothing appears under the final
//! name before that. `abort`, `Drop`, and any failure abort the multipart
//! upload so no parts are left billed on the service.
//!
//! `replace = false` never overwrites:
//!
//! - single `PUT`: a check (`HEAD`) and then `PutMode::Create` (S3
//!   `If-None-Match: *`, GCS `ifGenerationMatch=0`), so the loser of a race
//!   gets `AlreadyExists`. A store that does not support the condition gets
//!   the check alone, which leaves a small race window;
//! - multipart: `object_store` completes uploads unconditionally, so the
//!   target is checked right before `complete`; **racy** in the window between
//!   that check and the completion.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use kara_vfs::{BackendError, BackendErrorKind, RemotePath, WriteSession};
use object_store::path::Path;
use object_store::{
    GetOptions, GetRange, MultipartUpload, ObjectStoreExt, PutPayload,
};
use tokio::task::JoinHandle;

use super::backend::Inner;
use super::error::{Fail, error};
use super::runtime::Rt;

/// How a write session cuts and sends its parts.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WriteTuning {
    pub part_size: usize,
    pub concurrency: usize,
}

/// Wraps an error so that `BackendError::from_io` gets it back unchanged.
fn io_error(error: BackendError) -> io::Error {
    io::Error::from(error)
}

// ---------------------------------------------------------------------------
// Reading.

/// An object opened for reading from some offset.
pub(crate) struct ObjectReader {
    rt: Arc<Rt>,
    path: RemotePath,
    key: Path,
    stream: Option<BoxStream<'static, object_store::Result<Bytes>>>,
    current: Bytes,
    /// Bytes still expected from the stream.
    remaining: u64,
}

impl ObjectReader {
    /// Opens `key` (shown to the caller as `path`) at `from`; `size` is the
    /// object's size as `HEAD` said it. `from == size` reads nothing.
    pub(crate) fn open(
        inner: &Arc<Inner>,
        rt: &Arc<Rt>,
        path: &RemotePath,
        key: &Path,
        from: u64,
        size: u64,
    ) -> Result<ObjectReader, BackendError> {
        let remaining = size.saturating_sub(from);
        let stream = if remaining == 0 {
            None
        } else {
            let options = GetOptions {
                range: Some(GetRange::Offset(from)),
                ..GetOptions::default()
            };
            let store = Arc::clone(&inner.store);
            let result = rt
                .block_on(async move { store.get_opts(key, options).await })
                .map_err(|fail| fail.at(path, Some(key)))?
                .map_err(|e| Fail::Store(e).at(path, Some(key)))?;
            Some(result.into_stream())
        };
        Ok(ObjectReader {
            rt: Arc::clone(rt),
            path: path.clone(),
            key: key.clone(),
            stream,
            current: Bytes::new(),
            remaining,
        })
    }
}

impl Read for ObjectReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if !self.current.is_empty() {
                let n = self.current.len().min(buf.len());
                let piece = self.current.split_to(n);
                if let Some(dst) = buf.get_mut(..n) {
                    dst.copy_from_slice(&piece);
                }
                return Ok(n);
            }
            if buf.is_empty() {
                return Ok(0);
            }
            let Some(stream) = self.stream.as_mut() else {
                return Ok(0);
            };
            let next = self
                .rt
                .block_on(stream.next())
                .map_err(|fail| io_error(fail.at(&self.path, Some(&self.key))))?;
            match next {
                Some(Ok(bytes)) => {
                    let got = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                    self.remaining = self.remaining.saturating_sub(got);
                    self.current = bytes;
                }
                Some(Err(e)) => {
                    self.stream = None;
                    return Err(io_error(Fail::Store(e).at(&self.path, Some(&self.key))));
                }
                None => {
                    self.stream = None;
                    if self.remaining > 0 {
                        // The body ended early: the connection went away.
                        return Err(io_error(error(
                            BackendErrorKind::Unavailable,
                            &self.path,
                            io::ErrorKind::UnexpectedEof,
                        )));
                    }
                    return Ok(0);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Writing.

type PartTask = JoinHandle<object_store::Result<()>>;

/// An upload in progress.
pub(crate) struct ObjectWriteSession {
    inner: Arc<Inner>,
    rt: Arc<Rt>,
    /// The caller's path: every error names it.
    path: RemotePath,
    key: Path,
    replace: bool,
    tuning: WriteTuning,
    buffer: Vec<u8>,
    upload: Option<Box<dyn MultipartUpload>>,
    in_flight: VecDeque<PartTask>,
    /// Kind of the first failure; the session is dead from then on.
    poison: Option<BackendErrorKind>,
    /// Finished, aborted or failed: nothing left to clean up.
    closed: bool,
}

impl ObjectWriteSession {
    pub(crate) fn new(
        inner: Arc<Inner>,
        rt: Arc<Rt>,
        path: RemotePath,
        key: Path,
        replace: bool,
        tuning: WriteTuning,
    ) -> ObjectWriteSession {
        ObjectWriteSession {
            inner,
            rt,
            path,
            key,
            replace,
            tuning,
            buffer: Vec::new(),
            upload: None,
            in_flight: VecDeque::new(),
            poison: None,
            closed: false,
        }
    }

    fn fail(&self, fail: &Fail) -> BackendError {
        fail.at(&self.path, Some(&self.key))
    }

    /// Records the failure, aborts the upload and hands the error back.
    fn poisoned(&mut self, error: BackendError) -> BackendError {
        self.poison = Some(error.kind);
        self.buffer = Vec::new();
        let _ = self.abort_upload();
        error
    }

    /// Starts the multipart upload if it is not running yet.
    fn ensure_upload(&mut self) -> Result<(), BackendError> {
        if self.upload.is_some() {
            return Ok(());
        }
        let store = Arc::clone(&self.inner.store);
        let key = self.key.clone();
        let started = self
            .rt
            .block_on(async move { store.put_multipart(&key).await })
            .map_err(|fail| self.fail(&fail))?
            .map_err(|e| self.fail(&Fail::Store(e)))?;
        self.upload = Some(started);
        Ok(())
    }

    /// Waits for the oldest part on its way.
    fn wait_oldest(&mut self) -> Result<(), BackendError> {
        let Some(task) = self.in_flight.pop_front() else {
            return Ok(());
        };
        match self.rt.block_on(task) {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(e))) => Err(self.fail(&Fail::Store(e))),
            Ok(Err(join)) => Err(self.fail(&Fail::Other(join.to_string()))),
            Err(fail) => Err(self.fail(&fail)),
        }
    }

    /// Sends `part` as the next part, keeping at most `concurrency` on their way.
    fn send_part(&mut self, part: Vec<u8>) -> Result<(), BackendError> {
        self.ensure_upload()?;
        while self.in_flight.len() >= self.tuning.concurrency {
            self.wait_oldest()?;
        }
        let Some(upload) = self.upload.as_mut() else {
            return Err(self.fail(&Fail::Other(String::from("no upload to add a part to"))));
        };
        let future = upload.put_part(PutPayload::from(part));
        let Some(task) = self.rt.spawn(future) else {
            return Err(self.fail(&Fail::Other(String::from("the drive is closed"))));
        };
        self.in_flight.push_back(task);
        Ok(())
    }

    /// Ships every full part in the buffer.
    fn ship_full_parts(&mut self) -> Result<(), BackendError> {
        while self.buffer.len() >= self.tuning.part_size {
            let rest = self.buffer.split_off(self.tuning.part_size);
            let part = std::mem::replace(&mut self.buffer, rest);
            self.send_part(part)?;
        }
        Ok(())
    }

    pub(crate) fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Some(kind) = self.poison {
            return Err(io_error(error(kind, &self.path, io::ErrorKind::Other)));
        }
        if self.closed {
            return Err(io_error(error(
                BackendErrorKind::Other,
                &self.path,
                io::ErrorKind::BrokenPipe,
            )));
        }
        if data.is_empty() {
            return Ok(0);
        }
        self.buffer.extend_from_slice(data);
        if let Err(error) = self.ship_full_parts() {
            return Err(io_error(self.poisoned(error)));
        }
        Ok(data.len())
    }

    /// Aborts the multipart upload, if one was started; parts still on their
    /// way are stopped first. Returns the service's answer to the abort.
    fn abort_upload(&mut self) -> Result<(), BackendError> {
        for task in self.in_flight.drain(..) {
            task.abort();
        }
        let Some(mut upload) = self.upload.take() else {
            return Ok(());
        };
        let limit = self.rt.limit().min(Duration::from_secs(30));
        match self.rt.block_on_for(async move { upload.abort().await }, limit) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(self.fail(&Fail::Store(e))),
            Err(fail) => Err(self.fail(&fail)),
        }
    }

    /// Whether something is already stored under the key.
    fn target_taken(&self) -> Result<bool, BackendError> {
        let inner = Arc::clone(&self.inner);
        let key = self.key.clone();
        self.rt
            .block_on(async move { inner.head_exists(&key).await })
            .map_err(|fail| self.fail(&fail))?
            .map_err(|fail| self.fail(&fail))
    }

    fn commit(&mut self) -> Result<(), BackendError> {
        if let Some(kind) = self.poison {
            return Err(error(kind, &self.path, io::ErrorKind::Other));
        }
        if self.upload.is_none() {
            // Small file: one PUT.
            let payload = PutPayload::from(std::mem::take(&mut self.buffer));
            let inner = Arc::clone(&self.inner);
            let key = self.key.clone();
            let replace = self.replace;
            let outcome = self.rt.block_on(async move {
                if replace {
                    inner.store.put(&key, payload).await.map(|_| ()).map_err(Fail::Store)
                } else {
                    inner.put_new(&key, payload).await
                }
            });
            return match outcome {
                Ok(Ok(())) => Ok(()),
                Ok(Err(fail)) | Err(fail) => Err(self.fail(&fail)),
            };
        }
        // Multipart: the tail, then every part, then the check, then complete.
        if !self.buffer.is_empty() {
            let tail = std::mem::take(&mut self.buffer);
            self.send_part(tail)?;
        }
        while !self.in_flight.is_empty() {
            self.wait_oldest()?;
        }
        if !self.replace && self.target_taken()? {
            return Err(error(
                BackendErrorKind::AlreadyExists,
                &self.path,
                io::ErrorKind::AlreadyExists,
            ));
        }
        let Some(mut upload) = self.upload.take() else {
            return Err(self.fail(&Fail::Other(String::from("the upload vanished"))));
        };
        match self.rt.block_on(async move {
            let result = upload.complete().await;
            (result, upload)
        }) {
            Ok((Ok(_), _)) => Ok(()),
            Ok((Err(e), upload)) => {
                // Put it back so the failure path aborts it.
                self.upload = Some(upload);
                Err(self.fail(&Fail::Store(e)))
            }
            Err(fail) => Err(self.fail(&fail)),
        }
    }
}

impl WriteSession for ObjectWriteSession {
    fn finish(mut self: Box<Self>) -> Result<(), BackendError> {
        let outcome = self.commit();
        if outcome.is_err() {
            let _ = self.abort_upload();
        }
        self.closed = true;
        self.buffer = Vec::new();
        outcome
    }

    fn abort(mut self: Box<Self>) -> Result<(), BackendError> {
        self.closed = true;
        self.buffer = Vec::new();
        self.abort_upload()
    }
}

impl Drop for ObjectWriteSession {
    fn drop(&mut self) {
        if !self.closed {
            self.closed = true;
            let _ = self.abort_upload();
        }
    }
}
