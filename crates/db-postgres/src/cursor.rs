//! The row cursor: a streaming `RowStream` that is read page by page and
//! stopped on the server when it is closed early.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use db_core::timeout::{CANCEL_GRACE, with_timeout};
use db_core::{BusyGuard, Column, Cursor, Error, ErrorKind, Result, Row};
use futures_util::{FutureExt, StreamExt};
use tokio_postgres::RowStream;

use crate::conn::Inner;
use crate::decode::convert_row;

/// How long the rest of a data-modifying statement may be read before it is
/// cancelled after all.
const MODIFY_DRAIN: Duration = Duration::from_secs(30);

struct State {
    stream: Option<Pin<Box<RowStream>>>,
    /// Converted rows not yet handed to the caller.
    buffer: VecDeque<Row>,
    /// An error to report once the buffered rows are delivered.
    error: Option<Error>,
    /// Reported on every call after the cursor failed or was stopped.
    sticky: Option<Error>,
    /// The server sent the last row.
    done: bool,
}

/// Cursor state shared with the connection so that `commit`, `rollback` and
/// `close` can stop it.
pub(crate) struct CursorCore {
    inner: Arc<Inner>,
    state: tokio::sync::Mutex<State>,
    guard: Mutex<Option<BusyGuard>>,
    /// `false` once the server side is finished and the connection is free.
    live: AtomicBool,
    /// A per-page time limit fired and a cancel was sent. The cancel error
    /// may be read by a later page than the one that timed out, so the flag
    /// stays set: that error is a timeout, not a user cancel.
    timed_out: AtomicBool,
    modifies: bool,
    max_value_bytes: usize,
}

impl CursorCore {
    pub(crate) fn new(
        inner: Arc<Inner>,
        stream: RowStream,
        guard: BusyGuard,
        modifies: bool,
        max_value_bytes: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner,
            state: tokio::sync::Mutex::new(State {
                stream: Some(Box::pin(stream)),
                buffer: VecDeque::new(),
                error: None,
                sticky: None,
                done: false,
            }),
            guard: Mutex::new(Some(guard)),
            live: AtomicBool::new(true),
            timed_out: AtomicBool::new(false),
            modifies,
            max_value_bytes,
        })
    }

    fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }

    /// The connection is free again: release the busy flag.
    fn finish(&self) {
        self.live.store(false, Ordering::Release);
        if let Ok(mut guard) = self.guard.lock() {
            guard.take();
        }
        self.inner.released.notify_waiters();
    }

    /// Take one item from the stream into the state.
    fn absorb(
        &self,
        st: &mut State,
        item: Option<std::result::Result<tokio_postgres::Row, tokio_postgres::Error>>,
    ) {
        match item {
            Some(Ok(row)) => match convert_row(&row, self.max_value_bytes) {
                Ok(values) => st.buffer.push_back(values),
                Err(e) => st.error = Some(self.inner.map(&e)),
            },
            Some(Err(e)) => {
                let mapped = self.inner.map(&e);
                st.error = Some(
                    if mapped.kind == ErrorKind::Cancelled && self.timed_out.load(Ordering::Acquire)
                    {
                        Error::timeout()
                    } else {
                        mapped
                    },
                );
            }
            None => {
                st.done = true;
                st.stream = None;
                self.finish();
            }
        }
    }

    /// Read from the stream until `want` rows are buffered, the stream ends
    /// or fails.
    async fn fill(&self, st: &mut State, want: usize) {
        while st.buffer.len() < want && !st.done && st.error.is_none() {
            let Some(stream) = st.stream.as_mut() else {
                st.error = st.sticky.clone().or_else(|| Some(stopped()));
                break;
            };
            let item = stream.next().await;
            self.absorb(st, item);
        }
    }

    /// Without waiting: take what the server already sent after a full page,
    /// so that a result that ends exactly there is known to be exhausted.
    fn peek(&self, st: &mut State) {
        if st.done || st.error.is_some() {
            return;
        }
        let Some(stream) = st.stream.as_mut() else {
            return;
        };
        if let Some(item) = stream.next().now_or_never() {
            self.absorb(st, item);
        }
    }

    /// Wait for the first row, so that errors of the statement surface in
    /// `execute`. Returns whether the cursor is already exhausted.
    pub(crate) async fn prefetch(&self) -> Result<bool> {
        let mut st = self.state.lock().await;
        self.fill(&mut st, 1).await;
        if st.buffer.is_empty()
            && let Some(e) = st.error.take()
        {
            self.fail(&mut st, e.clone());
            return Err(e);
        }
        Ok(st.done && st.buffer.is_empty())
    }

    /// Up to `max_rows` rows; the flag says the cursor is now exhausted.
    async fn fetch(&self, max_rows: usize) -> Result<(Vec<Row>, bool)> {
        let mut st = self.state.lock().await;
        self.fill(&mut st, max_rows).await;
        if st.buffer.len() >= max_rows {
            self.peek(&mut st);
        }
        let n = st.buffer.len().min(max_rows);
        if n > 0 {
            let rows: Vec<Row> = st.buffer.drain(..n).collect();
            let exhausted = st.done && st.buffer.is_empty();
            return Ok((rows, exhausted));
        }
        if let Some(e) = st.error.take() {
            self.fail(&mut st, e.clone());
            return Err(e);
        }
        if let Some(e) = st.sticky.clone() {
            return Err(e);
        }
        Ok((Vec::new(), true))
    }

    /// The statement failed: nothing is left to stop on the server.
    fn fail(&self, st: &mut State, error: Error) {
        st.sticky = Some(error);
        st.stream = None;
        st.buffer.clear();
        self.finish();
    }

    /// Stop server work now and leave the connection clean.
    pub(crate) async fn terminate(&self) {
        if !self.is_live() {
            return;
        }
        if !self.modifies {
            // A fetch blocked on a slow statement is woken by the cancel and
            // releases the state lock.
            self.inner.cancel_quietly().await;
        }
        let mut st = self.state.lock().await;
        st.error = None;
        st.buffer.clear();
        if st.sticky.is_none() {
            st.sticky = Some(stopped());
        }
        if let Some(mut stream) = st.stream.take() {
            if self.modifies {
                // The change commits only when the statement completes.
                let finished = tokio::time::timeout(MODIFY_DRAIN, drain(&mut stream)).await;
                if finished.is_err() {
                    self.inner.cancel_quietly().await;
                    let _ = tokio::time::timeout(CANCEL_GRACE, drain(&mut stream)).await;
                }
            } else {
                let _ = tokio::time::timeout(CANCEL_GRACE, drain(&mut stream)).await;
            }
        }
        drop(st);
        self.finish();
    }

    /// `terminate` in the background, for `Drop`. New statements on the
    /// connection wait for it (see `Inner::acquire_guard`).
    pub(crate) fn spawn_terminate(self: Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            // No runtime to run the cleanup on. tokio-postgres still discards
            // the rest of the response, so the connection stays consistent.
            self.finish();
            return;
        };
        self.inner.cleanups.fetch_add(1, Ordering::AcqRel);
        handle.spawn(async move {
            self.terminate().await;
            self.inner.cleanups.fetch_sub(1, Ordering::AcqRel);
            self.inner.released.notify_waiters();
        });
    }
}

/// Read the stream to its end, ignoring the rows.
async fn drain(stream: &mut Pin<Box<RowStream>>) {
    while let Some(item) = stream.next().await {
        if let Err(e) = item
            && e.as_db_error().is_none()
        {
            break;
        }
    }
}

fn stopped() -> Error {
    Error::new(
        ErrorKind::Cancelled,
        "the cursor was closed (by close, commit, rollback or an earlier error)",
    )
}

/// The public cursor handle.
pub(crate) struct PgCursor {
    core: Arc<CursorCore>,
    columns: Vec<Column>,
    timeout: Option<Duration>,
    exhausted: bool,
}

impl PgCursor {
    pub(crate) fn new(
        core: Arc<CursorCore>,
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
        let fut = self.core.fetch(max_rows.max(1));
        let core = &self.core;
        let on_timeout = async move {
            core.timed_out.store(true, Ordering::Release);
            core.inner.cancel_quietly().await;
        };
        let result = with_timeout(self.timeout, fut, on_timeout).await;
        match result {
            Ok((rows, exhausted)) => {
                self.exhausted = exhausted;
                Ok(rows)
            }
            Err(e) => {
                if e.kind == ErrorKind::Timeout && self.core.is_live() {
                    // The statement may still be running: stop it in the
                    // background and keep the connection busy until it ends.
                    Arc::clone(&self.core).spawn_terminate();
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
        if self.core.is_live() {
            Arc::clone(&self.core).spawn_terminate();
        }
    }
}
