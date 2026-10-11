use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use db_core::{
    Backend, BusyFlag, BusyGuard, Column, Connection, Cursor, Error, ErrorKind, ExecOptions,
    Outcome, Result, Row, TxMode, Value,
};

use crate::link::{ExecArgs, ExecReply, FetchArgs, Link, Request};

/// A session opened through the Oracle Client.
///
/// All calls run on the connection's own thread (see [`crate::link`]), so
/// waiting for the database never blocks an async worker, and `cancel` works
/// while a statement or a fetch is in progress.
pub struct OdpiConnection {
    link: Arc<Link>,
    busy: BusyFlag,
}

impl OdpiConnection {
    pub(crate) fn new(link: Arc<Link>) -> Self {
        OdpiConnection {
            link,
            busy: BusyFlag::default(),
        }
    }
}

#[async_trait]
impl Connection for OdpiConnection {
    fn backend(&self) -> Backend {
        Backend::OracleOdpi
    }

    async fn execute(&self, sql: &str, params: &[Value], opts: &ExecOptions) -> Result<Outcome> {
        // Held from here on: a second statement while this one runs or while
        // its cursor is open is a `Busy` error.
        let guard = self.busy.acquire()?;
        let args = ExecArgs {
            sql: sql.to_owned(),
            params: params.to_vec(),
            timeout: opts.timeout,
            fetch_size: opts.fetch_size,
        };
        match self
            .link
            .timed(opts.timeout, |reply| Request::Execute(args, reply))
            .await?
        {
            ExecReply::Affected(n) => Ok(Outcome::Affected(n)),
            ExecReply::Rows(columns) => Ok(Outcome::Rows(Box::new(OdpiCursor {
                link: self.link.clone(),
                columns,
                exhausted: false,
                failed: None,
                released: false,
                timeout: opts.timeout,
                max_value_bytes: opts.max_value_bytes,
                _guard: guard,
            }))),
        }
    }

    async fn cancel(&self) -> Result<()> {
        self.link.cancel().await
    }

    async fn server_version(&self) -> Result<String> {
        self.link.call(Request::ServerVersion).await
    }

    async fn ping(&self) -> Result<()> {
        self.link.call(Request::Ping).await
    }

    async fn tx_mode(&self) -> TxMode {
        self.link.tx_mode()
    }

    async fn set_tx_mode(&self, mode: TxMode) -> Result<()> {
        self.link
            .call(|reply| Request::SetTxMode(mode, reply))
            .await?;
        self.link.note_tx_mode(mode);
        Ok(())
    }

    async fn commit(&self) -> Result<()> {
        self.link.call(Request::Commit).await
    }

    async fn rollback(&self) -> Result<()> {
        self.link.call(Request::Rollback).await
    }

    async fn close(&self) -> Result<()> {
        if self.link.gate().is_unusable() {
            // Already closed, or broken and being closed by its thread.
            return Ok(());
        }
        self.link.call(Request::Close).await
    }
}

/// A query result read page by page.
///
/// The session behind it keeps the statement open and fetches only what
/// `next_page` asks for (in round trips of `ExecOptions::fetch_size` rows).
/// Closing or dropping the cursor before the end closes the statement.
struct OdpiCursor {
    link: Arc<Link>,
    columns: Vec<Column>,
    exhausted: bool,
    /// A fetch failed or was abandoned: the cursor is unusable.
    failed: Option<Error>,
    /// The session no longer holds this cursor.
    released: bool,
    timeout: Option<Duration>,
    max_value_bytes: usize,
    _guard: BusyGuard,
}

#[async_trait]
impl Cursor for OdpiCursor {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        if self.exhausted {
            return Ok(Vec::new());
        }
        let args = FetchArgs {
            max_rows: max_rows.max(1),
            timeout: self.timeout,
            max_value_bytes: self.max_value_bytes,
        };
        match self
            .link
            .timed(self.timeout, |reply| Request::Fetch(args, reply))
            .await
        {
            Ok(page) => {
                if page.exhausted {
                    // The session released the server cursor at the end.
                    self.exhausted = true;
                    self.released = true;
                }
                Ok(page.rows)
            }
            Err(e) => {
                // A failed fetch closes the statement in the session.
                self.released = true;
                self.failed = Some(e.clone());
                Err(e)
            }
        }
    }

    fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    async fn close(mut self: Box<Self>) -> Result<()> {
        if self.released || self.link.gate().is_unusable() {
            self.released = true;
            return Ok(());
        }
        self.released = true;
        match self
            .link
            .call(|reply| Request::CloseCursor(Some(reply)))
            .await
        {
            // A cancel that raced with the close: the cursor is gone anyway.
            Err(e) if e.kind == ErrorKind::Cancelled => Ok(()),
            other => other,
        }
    }
}

impl Drop for OdpiCursor {
    fn drop(&mut self) {
        if !self.released {
            self.link.release_cursor();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    use db_core::{ErrorKind, Value, collect};

    use super::*;
    use crate::testing::mock_connection;

    fn opts() -> ExecOptions {
        ExecOptions::default()
    }

    async fn run(conn: &OdpiConnection, sql: &str) -> Result<Outcome> {
        conn.execute(sql, &[], &opts()).await
    }

    async fn rows(conn: &OdpiConnection, sql: &str) -> Box<dyn Cursor> {
        match run(conn, sql).await.unwrap() {
            Outcome::Rows(c) => c,
            other => panic!("rows expected, got {other:?}"),
        }
    }

    async fn wait_until(mut f: impl FnMut() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "timed out waiting"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn dml_reports_affected_rows_and_frees_the_connection() {
        let (conn, _) = mock_connection();
        assert!(matches!(
            run(&conn, "affected").await.unwrap(),
            Outcome::Affected(3)
        ));
        assert!(matches!(
            run(&conn, "affected").await.unwrap(),
            Outcome::Affected(3)
        ));
    }

    #[tokio::test]
    async fn pages_of_a_large_result_are_continuous() {
        let (conn, script) = mock_connection();
        let mut cursor = rows(&conn, "rows 100000").await;
        assert_eq!(cursor.columns()[0].name, "N");
        let mut next = 0i64;
        let mut pages = 0;
        loop {
            let page = cursor.next_page(500).await.unwrap();
            if page.is_empty() {
                break;
            }
            for row in &page {
                assert_eq!(row[0], Value::Int(next));
                next += 1;
            }
            pages += 1;
            if cursor.is_exhausted() {
                break;
            }
        }
        assert_eq!(next, 100_000);
        assert_eq!(pages, 200);
        assert!(cursor.is_exhausted());
        assert!(cursor.next_page(500).await.unwrap().is_empty());
        drop(cursor);
        // The session saw the end and never needed a close.
        assert!(!script.entries().contains(&"close_cursor".to_string()));
    }

    #[tokio::test]
    async fn only_one_cursor_per_connection() {
        let (conn, _) = mock_connection();
        let cursor = rows(&conn, "rows 10").await;
        assert_eq!(
            run(&conn, "affected").await.unwrap_err().kind,
            ErrorKind::Busy
        );
        assert_eq!(
            run(&conn, "rows 5").await.unwrap_err().kind,
            ErrorKind::Busy
        );
        // Transactions and cancel still work while a cursor is open.
        conn.commit().await.unwrap();
        conn.rollback().await.unwrap();
        conn.ping().await.unwrap();
        conn.cancel().await.unwrap();
        drop(cursor);
        assert!(run(&conn, "affected").await.is_ok());
    }

    #[tokio::test]
    async fn early_close_releases_the_server_cursor() {
        let (conn, script) = mock_connection();
        let mut cursor = rows(&conn, "rows 1000").await;
        assert_eq!(cursor.next_page(100).await.unwrap().len(), 100);
        assert!(!cursor.is_exhausted());
        cursor.close().await.unwrap();
        assert!(script.entries().contains(&"close_cursor".to_string()));
        // The next statement runs at once; nothing drained the rest.
        assert!(run(&conn, "affected").await.is_ok());
        assert!(!script.entries().iter().any(|e| e == "fetch 900"));
    }

    #[tokio::test]
    async fn dropping_a_cursor_closes_it_before_the_next_statement() {
        let (conn, script) = mock_connection();
        drop(rows(&conn, "rows 1000").await);
        run(&conn, "affected").await.unwrap();
        let log = script.entries();
        let close = log.iter().position(|e| e == "close_cursor").unwrap();
        let next = log.iter().position(|e| e == "execute affected").unwrap();
        assert!(close < next, "{log:?}");
    }

    #[tokio::test]
    async fn collect_reads_up_to_the_cap_and_flags_truncation() {
        let (conn, script) = mock_connection();
        let cursor = rows(&conn, "rows 1000").await;
        let rs = collect(cursor, 250, 100).await.unwrap();
        assert_eq!(rs.rows.len(), 250);
        assert!(rs.truncated);
        assert!(script.entries().contains(&"close_cursor".to_string()));
        // And the whole result when it is small.
        let rs = conn.query("rows 5", &[], 100).await.unwrap();
        assert_eq!(rs.rows.len(), 5);
        assert!(!rs.truncated);
    }

    #[tokio::test]
    async fn cancel_from_another_task_ends_the_statement_and_keeps_the_connection() {
        let (conn, script) = mock_connection();
        let conn = Arc::new(conn);
        let runner = {
            let conn = conn.clone();
            tokio::spawn(async move { conn.execute("block", &[], &ExecOptions::default()).await })
        };
        wait_until(|| script.entries().contains(&"execute block".to_string())).await;
        conn.cancel().await.unwrap();
        let e = runner.await.unwrap().unwrap_err();
        assert_eq!(e.kind, ErrorKind::Cancelled);
        // Not busy any more, and still usable.
        assert!(run(&conn, "affected").await.is_ok());
    }

    #[tokio::test]
    async fn statement_timeout_reports_timeout() {
        let (conn, _) = mock_connection();
        let opts = ExecOptions {
            timeout: Some(Duration::from_millis(100)),
            ..ExecOptions::default()
        };
        let started = Instant::now();
        let e = conn.execute("block", &[], &opts).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Timeout);
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(run(&conn, "affected").await.is_ok());
    }

    #[tokio::test]
    async fn dropped_execute_leaves_the_connection_broken_not_half_way() {
        let (conn, script) = mock_connection();
        let r = tokio::time::timeout(Duration::from_millis(80), run(&conn, "block")).await;
        assert!(r.is_err());
        wait_until(|| script.closed.load(Ordering::SeqCst)).await;
        assert_eq!(
            run(&conn, "affected").await.unwrap_err().kind,
            ErrorKind::ConnectionLost
        );
        assert_eq!(
            conn.ping().await.unwrap_err().kind,
            ErrorKind::ConnectionLost
        );
        // Closing a broken connection is not an error.
        conn.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_failed_fetch_poisons_only_the_cursor() {
        let (conn, _) = mock_connection();
        let mut cursor = rows(&conn, "rows 10 fail_at 3").await;
        let e = cursor.next_page(100).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Query);
        let again = cursor.next_page(100).await.unwrap_err();
        assert_eq!(again, e, "the same error, not an empty page");
        drop(cursor);
        assert!(run(&conn, "affected").await.is_ok());
    }

    #[tokio::test]
    async fn close_is_idempotent_and_later_calls_fail() {
        let (conn, script) = mock_connection();
        conn.close().await.unwrap();
        conn.close().await.unwrap();
        assert!(script.closed.load(Ordering::SeqCst));
        assert_eq!(
            run(&conn, "affected").await.unwrap_err().kind,
            ErrorKind::ConnectionLost
        );
    }

    #[tokio::test]
    async fn closing_the_connection_closes_an_open_cursor_first() {
        let (conn, script) = mock_connection();
        let cursor = rows(&conn, "rows 10").await;
        conn.close().await.unwrap();
        let log = script.entries();
        assert_eq!(log.last().map(String::as_str), Some("close"));
        drop(cursor);
    }

    #[tokio::test]
    async fn tx_modes() {
        let (conn, script) = mock_connection();
        assert_eq!(conn.tx_mode().await, TxMode::Auto);
        conn.set_tx_mode(TxMode::Manual).await.unwrap();
        assert_eq!(conn.tx_mode().await, TxMode::Manual);
        conn.commit().await.unwrap();
        conn.set_tx_mode(TxMode::Auto).await.unwrap();
        assert_eq!(conn.tx_mode().await, TxMode::Auto);
        assert_eq!(script.entries(), ["tx Manual", "commit", "tx Auto"]);
    }

    #[tokio::test]
    async fn server_version_and_backend() {
        let (conn, _) = mock_connection();
        assert_eq!(conn.backend(), Backend::OracleOdpi);
        assert_eq!(conn.dialect(), db_core::Dialect::Oracle);
        assert_eq!(conn.server_version().await.unwrap(), "Oracle Mock 1.0");
    }
}
