//! A cursor over an SQL cursor (`DECLARE ... CURSOR`, `FETCH`, `CLOSE`),
//! used in manual-commit sessions. Closing it early is a plain `CLOSE`, so the
//! user's transaction survives; cancelling a running statement would abort it
//! and lose the pending changes.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use db_core::timeout::CANCEL_GRACE;
use db_core::{BusyGuard, Error, ErrorKind, Result, Row};
use futures_util::StreamExt;
use tokio_postgres::Client;
use tokio_postgres::types::{ToSql, Type};

use crate::conn::Inner;
use crate::cursor::{CursorCore, Lifecycle, stopped};
use crate::decode::convert_row;

/// Rows requested by one `FETCH` at most.
const FETCH_CHUNK: usize = 100_000;

struct State {
    buffer: VecDeque<Row>,
    /// An error to report once the buffered rows are delivered.
    error: Option<Error>,
    /// Reported on every call after the cursor failed or was stopped.
    sticky: Option<Error>,
    /// The server has no more rows.
    done: bool,
    /// The SQL cursor exists on the server and has not been closed.
    open: bool,
    /// A `FETCH` is running. Still set at the next call means the future of
    /// that call was dropped half-way and rows may have been lost.
    inflight: bool,
}

pub(crate) struct DeclaredCore {
    life: Lifecycle,
    client: Arc<Client>,
    name: String,
    max_value_bytes: usize,
    state: tokio::sync::Mutex<State>,
}

impl DeclaredCore {
    /// `name` is a cursor that was just declared on `client`.
    pub(crate) fn new(
        inner: Arc<Inner>,
        client: Arc<Client>,
        name: String,
        guard: BusyGuard,
        max_value_bytes: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            life: Lifecycle::new(inner, guard),
            client,
            name,
            max_value_bytes,
            state: tokio::sync::Mutex::new(State {
                buffer: VecDeque::new(),
                error: None,
                sticky: None,
                done: false,
                open: true,
                inflight: false,
            }),
        })
    }

    /// One `FETCH` of `count` rows into the buffer; returns how many arrived.
    async fn fetch_chunk(&self, st: &mut State, count: usize) -> Result<usize> {
        let sql = format!("FETCH FORWARD {count} FROM {}", self.name);
        let params = std::iter::empty::<(&(dyn ToSql + Sync), Type)>();
        let stream = self
            .client
            .query_typed_raw(&sql, params)
            .await
            .map_err(|e| self.life.map_error(&e))?;
        let mut stream = std::pin::pin!(stream);
        let mut got = 0;
        while let Some(item) = stream.next().await {
            let row = item.map_err(|e| self.life.map_error(&e))?;
            let values =
                convert_row(&row, self.max_value_bytes).map_err(|e| self.life.inner().map(&e))?;
            st.buffer.push_back(values);
            got += 1;
        }
        Ok(got)
    }

    /// Fetch until `want` rows are buffered, the cursor ends or fails.
    async fn fill(&self, st: &mut State, want: usize) {
        if st.inflight {
            st.inflight = false;
            st.error = Some(Error::new(
                ErrorKind::Cancelled,
                "a page request was dropped while it was running; the cursor lost its position",
            ));
            return;
        }
        while st.buffer.len() < want && !st.done && st.error.is_none() && st.sticky.is_none() {
            // One row more than needed: a shorter answer means the end, so
            // a result that ends exactly at a page boundary is known too.
            let count = (want - st.buffer.len()).min(FETCH_CHUNK) + 1;
            st.inflight = true;
            let result = self.fetch_chunk(st, count).await;
            st.inflight = false;
            match result {
                Ok(got) if got < count => {
                    st.done = true;
                    self.close_portal(st).await;
                    self.life.finish();
                }
                Ok(_) => {}
                Err(e) => st.error = Some(e),
            }
        }
    }

    /// Fetch the first rows, so that errors of the query surface in
    /// `execute`. Returns whether the cursor is already exhausted.
    pub(crate) async fn prefetch(&self, rows: usize) -> Result<bool> {
        let mut st = self.state.lock().await;
        self.fill(&mut st, rows).await;
        if st.buffer.is_empty()
            && let Some(e) = st.error.take()
        {
            self.fail(&mut st, e.clone());
            return Err(e);
        }
        Ok(st.done && st.buffer.is_empty())
    }

    async fn close_portal(&self, st: &mut State) {
        if !st.open {
            return;
        }
        st.open = false;
        let sql = format!("CLOSE {}", self.name);
        // In an aborted transaction this fails, and there is nothing to close.
        let _ = tokio::time::timeout(CANCEL_GRACE, self.client.batch_execute(&sql)).await;
    }

    /// The cursor failed: it is unusable, and so is the transaction.
    fn fail(&self, st: &mut State, error: Error) {
        st.sticky = Some(error);
        st.buffer.clear();
        st.open = false;
        self.life.finish();
    }
}

#[async_trait]
impl CursorCore for DeclaredCore {
    async fn fetch(&self, max_rows: usize) -> Result<(Vec<Row>, bool)> {
        let mut st = self.state.lock().await;
        self.fill(&mut st, max_rows).await;
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
        if self.state.try_lock().is_err() {
            // A FETCH is running: a cancel ends it (and the transaction).
            self.life.inner().cancel_quietly().await;
        }
        let mut st = self.state.lock().await;
        st.error = None;
        st.buffer.clear();
        if st.sticky.is_none() {
            st.sticky = Some(stopped());
        }
        self.close_portal(&mut st).await;
        drop(st);
        self.life.finish();
    }

    fn life(&self) -> &Lifecycle {
        &self.life
    }
}
