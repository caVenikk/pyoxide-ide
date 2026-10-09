//! Cursors. Two implementations share the public handle and the lifecycle:
//!
//! - [`crate::stream_cursor::StreamCore`] reads a streaming result and stops it
//!   with a cancel request (auto-commit sessions).
//! - [`crate::declared_cursor::DeclaredCore`] pages through an SQL cursor
//!   (`DECLARE` / `FETCH` / `CLOSE`), which can be closed without cancelling and
//!   so without aborting the user's transaction (manual-commit sessions).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use db_core::timeout::with_timeout;
use db_core::{BusyGuard, Column, Cursor, Error, ErrorKind, Result, Row};

use crate::conn::Inner;

/// What the public cursor needs from an implementation.
#[async_trait]
pub(crate) trait CursorCore: Send + Sync {
    /// Up to `max_rows` rows; the flag says the cursor is now exhausted.
    async fn fetch(&self, max_rows: usize) -> Result<(Vec<Row>, bool)>;

    /// Stop server work now and leave the connection clean.
    async fn terminate(&self);

    fn life(&self) -> &Lifecycle;
}

/// Ownership of the busy flag and the "a time limit fired" marker.
pub(crate) struct Lifecycle {
    inner: Arc<Inner>,
    guard: Mutex<Option<BusyGuard>>,
    /// `false` once the server side is finished and the connection is free.
    live: AtomicBool,
    /// A per-page time limit fired and a cancel was sent. The cancel error
    /// may be read by a later page than the one that timed out, so the flag
    /// stays set: that error is a timeout, not a user cancel.
    timed_out: AtomicBool,
}

impl Lifecycle {
    pub(crate) fn new(inner: Arc<Inner>, guard: BusyGuard) -> Self {
        Self {
            inner,
            guard: Mutex::new(Some(guard)),
            live: AtomicBool::new(true),
            timed_out: AtomicBool::new(false),
        }
    }

    pub(crate) fn inner(&self) -> &Arc<Inner> {
        &self.inner
    }

    pub(crate) fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }

    /// The connection is free again: release the busy flag.
    pub(crate) fn finish(&self) {
        self.live.store(false, Ordering::Release);
        if let Ok(mut guard) = self.guard.lock() {
            guard.take();
        }
        self.inner.released.notify_waiters();
    }

    pub(crate) fn mark_timed_out(&self) {
        self.timed_out.store(true, Ordering::Release);
    }

    /// Map an error of the running statement. After a time limit fired, the
    /// server's "cancelled" is reported as a timeout.
    pub(crate) fn map_error(&self, e: &tokio_postgres::Error) -> Error {
        let mapped = self.inner.map(e);
        if mapped.kind == ErrorKind::Cancelled && self.timed_out.load(Ordering::Acquire) {
            Error::timeout()
        } else {
            mapped
        }
    }
}

/// Run `terminate` in the background, for `Drop`. New statements on the
/// connection wait for it (see `Inner::acquire_guard`).
pub(crate) fn spawn_terminate(core: Arc<dyn CursorCore>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        // No runtime to run the cleanup on. tokio-postgres still discards
        // the rest of a response, so the connection stays consistent.
        core.life().finish();
        return;
    };
    let inner = Arc::clone(core.life().inner());
    inner.cleanups.fetch_add(1, Ordering::AcqRel);
    handle.spawn(async move {
        core.terminate().await;
        inner.cleanups.fetch_sub(1, Ordering::AcqRel);
        inner.released.notify_waiters();
    });
}

pub(crate) fn stopped() -> Error {
    Error::new(
        ErrorKind::Cancelled,
        "the cursor was closed (by close, commit, rollback or an earlier error)",
    )
}

/// The public cursor handle.
pub(crate) struct PgCursor {
    core: Arc<dyn CursorCore>,
    columns: Vec<Column>,
    timeout: Option<Duration>,
    exhausted: bool,
}

impl PgCursor {
    pub(crate) fn new(
        core: Arc<dyn CursorCore>,
        columns: Vec<Column>,
        timeout: Option<Duration>,
        exhausted: bool,
    ) -> Self {
        Self {
            core,
            columns,
            timeout,
            exhausted,
        }
    }
}

#[async_trait]
impl Cursor for PgCursor {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>> {
        if self.exhausted {
            return Ok(Vec::new());
        }
        let core = &self.core;
        let fut = core.fetch(max_rows.max(1));
        let on_timeout = async move {
            core.life().mark_timed_out();
            core.life().inner().cancel_quietly().await;
        };
        match with_timeout(self.timeout, fut, on_timeout).await {
            Ok((rows, exhausted)) => {
                self.exhausted = exhausted;
                Ok(rows)
            }
            Err(e) => {
                if e.kind == ErrorKind::Timeout && core.life().is_live() {
                    // The statement may still be running: stop it in the
                    // background and keep the connection busy until it ends.
                    spawn_terminate(Arc::clone(core));
                }
                Err(e)
            }
        }
    }

    fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    async fn close(self: Box<Self>) -> Result<()> {
        self.core.terminate().await;
        Ok(())
    }
}

impl Drop for PgCursor {
    fn drop(&mut self) {
        if self.core.life().is_live() {
            spawn_terminate(Arc::clone(&self.core));
        }
    }
}
