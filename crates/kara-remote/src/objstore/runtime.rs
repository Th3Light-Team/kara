//! The private runtime behind the blocking calls.
//!
//! One multi-thread tokio runtime (two workers) per connected drive: parts of
//! an upload keep moving while the caller is between two `write` calls.
//! Nothing async leaves this module except through [`Rt::spawn`].

use std::future::Future;
use std::time::Duration;

use kara_vfs::Cancel;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

use super::error::Fail;

/// How often a cancellable wait looks at the token.
const CANCEL_POLL: Duration = Duration::from_millis(20);

/// The runtime and the outer bound every blocking wait gets.
pub(crate) struct Rt {
    runtime: Option<Runtime>,
    /// The longest a single blocking call may take: the client's own timeouts
    /// and retries normally answer long before.
    limit: Duration,
}

impl std::fmt::Debug for Rt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rt").field("limit", &self.limit).finish_non_exhaustive()
    }
}

impl Rt {
    /// A fresh runtime. `limit` bounds every blocking call.
    pub(crate) fn new(limit: Duration) -> Result<Rt, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("kara-objstore")
            .enable_all()
            .build()
            .map_err(|error| format!("no runtime for the object store: {error}"))?;
        Ok(Rt {
            runtime: Some(runtime),
            limit,
        })
    }

    pub(crate) fn limit(&self) -> Duration {
        self.limit
    }

    /// Runs `future` to completion, bounded by the limit.
    pub(crate) fn block_on<F: Future>(&self, future: F) -> Result<F::Output, Fail> {
        self.block_on_for(future, self.limit)
    }

    /// Like [`Rt::block_on`] with an explicit bound.
    pub(crate) fn block_on_for<F: Future>(&self, future: F, limit: Duration) -> Result<F::Output, Fail> {
        // Blocking inside an async runtime would panic in tokio: refuse instead.
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(Fail::Other(String::from(
                "an object-store drive was called from inside an async runtime",
            )));
        }
        let Some(runtime) = self.runtime.as_ref() else {
            return Err(Fail::Other(String::from("the drive is closed")));
        };
        // The timer is created inside the runtime: tokio needs its context for it.
        runtime
            .block_on(async move { tokio::time::timeout(limit, future).await })
            .map_err(|_| Fail::TimedOut)
    }

    /// Like [`Rt::block_on`], but gives up as soon as `cancel` is set.
    pub(crate) fn block_on_cancellable<F: Future>(
        &self,
        future: F,
        cancel: &Cancel,
    ) -> Result<F::Output, Fail> {
        let watch = async {
            while !cancel.is_cancelled() {
                tokio::time::sleep(CANCEL_POLL).await;
            }
        };
        self.block_on(async {
            tokio::select! {
                out = future => Ok(out),
                () = watch => Err(Fail::Cancelled),
            }
        })?
    }

    /// Starts `future` without waiting for it. `None` once the drive is closed.
    pub(crate) fn spawn<F>(&self, future: F) -> Option<JoinHandle<F::Output>>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        Some(self.runtime.as_ref()?.spawn(future))
    }
}

impl Drop for Rt {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
