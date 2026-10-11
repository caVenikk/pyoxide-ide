//! A cursor over a streaming result (`RowStream`), read page by page. Closing
//! it before the end sends a cancel request, so it is used in auto-commit
//! sessions where nothing is lost by aborting the statement.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use db_core::timeout::CANCEL_GRACE;
use db_core::{BusyGuard, Error, Result, Row};
use futures_util::{FutureExt, StreamExt};
use tokio_postgres::RowStream;

use crate::conn::Inner;
use crate::cursor::{CursorCore, Lifecycle, stopped};
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
pub(crate) struct StreamCore {
    life: Lifecycle,
    state: tokio::sync::Mutex<State>,
    /// A data-modifying statement: stopping it before it completes could roll
    /// the change back, so it is read to the end instead of being cancelled.
    modifies: bool,
    max_value_bytes: usize,
}

impl StreamCore {
    pub(crate) fn new(
        inner: Arc<Inner>,
        stream: RowStream,
        guard: BusyGuard,
        modifies: bool,
        max_value_bytes: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            life: Lifecycle::new(inner, guard),
            state: tokio::sync::Mutex::new(State {
                stream: Some(Box::pin(stream)),
                buffer: VecDeque::new(),
                error: None,
                sticky: None,
                done: false,
            }),
            modifies,
            max_value_bytes,
        })
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
                Err(e) => st.error = Some(self.life.inner().map(&e)),
            },
            Some(Err(e)) => st.error = Some(self.life.map_error(&e)),
            None => {
                st.done = true;
                st.stream = None;
                self.life.finish();
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

    /// The statement failed: nothing is left to stop on the server.
    fn fail(&self, st: &mut State, error: Error) {
        st.sticky = Some(error);
        st.stream = None;
        st.buffer.clear();
        self.life.finish();
    }
}

#[async_trait]
impl CursorCore for StreamCore {
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

    async fn terminate(&self) {
        if !self.life.is_live() {
            return;
        }
        let inner = self.life.inner();
        if !self.modifies {
            // A fetch blocked on a slow statement is woken by the cancel and
            // releases the state lock.
            inner.cancel_quietly().await;
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
                    inner.cancel_quietly().await;
                    let _ = tokio::time::timeout(CANCEL_GRACE, drain(&mut stream)).await;
                }
            } else {
                let _ = tokio::time::timeout(CANCEL_GRACE, drain(&mut stream)).await;
            }
        }
        drop(st);
        self.life.finish();
    }

    fn life(&self) -> &Lifecycle {
        &self.life
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
