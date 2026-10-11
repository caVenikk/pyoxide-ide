//! Incremental row source over a server cursor.
//!
//! Rows are fetched with an execute-with-fetch request on the open cursor
//! (oracle-rs `fetch_cursor_with_size` / `fetch_implicit_result_with_size`;
//! its `fetch_more` is not used: it frames requests for small-SDU sessions
//! only and breaks the session on 12.2+ servers). The end of the rows is a
//! round trip that returns fewer rows than requested.
//!
//! oracle-rs reads one network packet per call, so each round trip asks for
//! as many rows as fit the session's fetch budget, estimated from the
//! declared column sizes and the sizes seen so far.
//!
//! oracle-rs cannot close a server cursor. Closing or dropping the cursor
//! stops fetching (Oracle computes rows on fetch, so no further server work
//! happens), and the server cursor stays allocated until the session ends;
//! the connection renews its session before `OPEN_CURSORS` is reached.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use db_core::timeout::with_timeout;
use db_core::{BusyGuard, Column, Cursor, ExecOptions, Result, Row, Value};
use oracle_rs::{ColumnInfo, ImplicitResult, LobData, LobLocator, OracleType, RefCursor};

use crate::session::Session;
use crate::types::{self, Cell, ColPlan};

/// Rows oracle-rs prefetches with `Connection::query` (fixed in 0.1.7).
pub const NATIVE_PREFETCH: usize = 100;

pub enum Source {
    /// A REF CURSOR returned by the wrapper block.
    Ref(RefCursor),
    /// A cursor opened by oracle-rs `query` (statements with LOB columns).
    Native {
        cursor_id: u16,
        columns: Vec<ColumnInfo>,
    },
}

pub struct ThinCursor {
    session: Arc<Session>,
    _guard: BusyGuard,
    columns: Vec<Column>,
    plans: Vec<ColPlan>,
    source: Source,
    buffered: VecDeque<oracle_rs::Row>,
    /// The server has no more rows.
    done: bool,
    closed: bool,
    opts: ExecOptions,
    /// Largest encoded size seen per column.
    observed: Vec<usize>,
}

impl ThinCursor {
    pub fn new(
        session: Arc<Session>,
        guard: BusyGuard,
        described: &[ColumnInfo],
        source: Source,
        first_rows: Vec<oracle_rs::Row>,
        done: bool,
        opts: &ExecOptions,
    ) -> Self {
        let (columns, plans): (Vec<_>, Vec<_>) = described.iter().map(types::column).unzip();
        let n = columns.len();
        Self {
            session,
            _guard: guard,
            columns,
            plans,
            source,
            buffered: first_rows.into(),
            done,
            closed: false,
            opts: opts.clone(),
            observed: vec![0; n],
        }
    }

    /// Rows to ask for in one round trip.
    fn trip_rows(&self, want: usize) -> usize {
        let per_row: usize = 8 + self
            .plans
            .iter()
            .zip(&self.observed)
            .map(|(p, &seen)| estimate(p.declared_bytes, seen))
            .sum::<usize>();
        let fit = (self.session.fetch_budget / per_row.max(1)).max(1);
        fit.min(want).min(self.opts.fetch_size.max(1)).max(1)
    }

    async fn fetch_trip(&mut self, n: usize) -> Result<Vec<oracle_rs::Row>> {
        let size = u32::try_from(n).unwrap_or(u32::MAX);
        let session = self.session.clone();
        let res = match &self.source {
            Source::Ref(rc) => {
                session
                    .call(session.conn.fetch_cursor_with_size(rc, size))
                    .await?
            }
            Source::Native { cursor_id, columns } => {
                let ir = ImplicitResult::new(*cursor_id, columns.clone(), Vec::new());
                session
                    .call(session.conn.fetch_implicit_result_with_size(&ir, size))
                    .await?
            }
        };
        if res.rows.len() < n {
            self.done = true;
        }
        Ok(res.rows)
    }

    async fn read_lob(&self, loc: &LobLocator) -> Result<Value> {
        let max = self.opts.max_value_bytes;
        let size = loc.size();
        let amount = size.min(max as u64);
        let is_blob = loc.oracle_type() == OracleType::Blob;
        let data = if amount == 0 {
            None
        } else {
            Some(
                self.session
                    .call(self.session.conn.read_lob_range(loc, 1, amount))
                    .await?,
            )
        };
        Ok(if is_blob {
            let bytes = match data {
                Some(LobData::Bytes(b)) => b.to_vec(),
                Some(LobData::String(s)) => s.into_bytes(),
                None => Vec::new(),
            };
            types::truncate_bytes(bytes, max, Some(size))
        } else {
            let text = match data {
                Some(LobData::String(s)) => s,
                Some(LobData::Bytes(b)) => String::from_utf8_lossy(&b).into_owned(),
                None => String::new(),
            };
            // Oracle reports CLOB lengths in characters.
            types::truncate_text(text, max, Some(size))
        })
    }

    async fn convert(&mut self, rows: Vec<oracle_rs::Row>) -> Result<Vec<Row>> {
        let max = self.opts.max_value_bytes;
        let mut out = Vec::with_capacity(rows.len());
        for raw in rows {
            let mut row = Vec::with_capacity(self.plans.len());
            for (i, v) in raw.into_values().into_iter().enumerate() {
                let Some(plan) = self.plans.get(i) else {
                    break;
                };
                let value = match types::cell(plan, v, max) {
                    Cell::Ready(v) => v,
                    Cell::Lob(loc) => self.read_lob(&loc).await?,
                };
                let size = encoded_size(&value, plan.declared_bytes);
                if let Some(o) = self.observed.get_mut(i) {
                    *o = (*o).max(size);
                }
                row.push(value);
            }
            out.push(row);
        }
        Ok(out)
    }

    async fn page(&mut self, max_rows: usize) -> Result<Vec<Row>> {
        let mut out: Vec<Row> = Vec::new();
        while out.len() < max_rows {
            if !self.buffered.is_empty() {
                let take = (max_rows - out.len()).min(self.buffered.len());
                let rows: Vec<_> = self.buffered.drain(..take).collect();
                let rows = self.convert(rows).await?;
                out.extend(rows);
                continue;
            }
            if self.done {
                break;
            }
            let n = self.trip_rows(max_rows - out.len());
            let rows = self.fetch_trip(n).await?;
            if rows.is_empty() {
                break;
            }
            let rows = self.convert(rows).await?;
            out.extend(rows);
        }
        Ok(out)
    }
}

/// Bytes to plan for one column per row: the declared size for short
/// columns; for long ones twice the largest value seen, at least a quarter
/// of the declared size (the declared size until a value was seen).
fn estimate(declared: usize, seen: usize) -> usize {
    if declared <= 256 || seen == 0 {
        declared
    } else {
        declared.min((seen * 2).max(declared / 4).max(64))
    }
}

fn encoded_size(v: &Value, declared: usize) -> usize {
    match v {
        Value::Null => 1,
        Value::Text(s) | Value::Decimal(s) | Value::Interval(s) | Value::Json(s) => s.len() + 3,
        Value::Other { text, .. } => text.len() + 3,
        Value::Bytes(b) => b.len() + 3,
        Value::Truncated { .. } => declared,
        _ => declared.min(24),
    }
}

#[async_trait]
impl Cursor for ThinCursor {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>> {
        if self.closed || max_rows == 0 {
            return Ok(Vec::new());
        }
        self.session.check()?;
        let timeout = self.opts.timeout;
        with_timeout(timeout, self.page(max_rows), async {}).await
    }

    fn is_exhausted(&self) -> bool {
        self.closed || (self.done && self.buffered.is_empty())
    }

    async fn close(mut self: Box<Self>) -> Result<()> {
        self.closed = true;
        self.buffered.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_column_sizes() {
        assert_eq!(estimate(22, 0), 22);
        assert_eq!(estimate(4003, 0), 4003);
        assert_eq!(estimate(4003, 10), 1000);
        assert_eq!(estimate(4003, 3000), 4003);
        assert_eq!(estimate(300, 5), 75);
    }

    #[test]
    fn encoded_sizes() {
        assert_eq!(encoded_size(&Value::Text("abc".into()), 100), 6);
        assert_eq!(encoded_size(&Value::Int(1), 22), 22);
        assert_eq!(encoded_size(&Value::Null, 22), 1);
    }
}
