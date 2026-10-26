use async_trait::async_trait;
use db_core::worker::{OnAbandon, Worker, WorkerConfig};
use db_core::{Backend, Connection, ExecOptions, Outcome, Result, TxMode, Value};

/// How the connection thread of a session behaves. Client-side call
/// timeouts need an Oracle Client of 18c or later; with an older one the
/// worker's timer breaks the call itself.
pub(crate) fn worker_config(call_timeout_supported: bool) -> WorkerConfig {
    WorkerConfig {
        thread_name: format!("{}-odpi", app_info::slug()),
        // The outcome of an abandoned call is unknown: a fetch may have
        // consumed rows, a statement may have run. The session is closed
        // rather than continued half-way.
        on_abandon: OnAbandon::BreakSession,
        session_enforces_timeouts: call_timeout_supported,
    }
}

/// A session opened through the Oracle Client.
///
/// All calls run on the connection's own thread (see [`db_core::worker`]),
/// so waiting for the database never blocks an async worker, and `cancel`
/// breaks a statement or a fetch in progress. Query results are read page by
/// page: the statement stays open and fetches only what `next_page` asks for
/// (in round trips of `ExecOptions::fetch_size` rows); closing or dropping
/// the cursor before the end closes the statement.
pub struct OdpiConnection {
    worker: Worker,
}

impl OdpiConnection {
    pub(crate) fn new(worker: Worker) -> Self {
        OdpiConnection { worker }
    }
}

#[async_trait]
impl Connection for OdpiConnection {
    fn backend(&self) -> Backend {
        Backend::OracleOdpi
    }

    async fn execute(&self, sql: &str, params: &[Value], opts: &ExecOptions) -> Result<Outcome> {
        self.worker.execute(sql, params, opts).await
    }

    async fn cancel(&self) -> Result<()> {
        self.worker.cancel().await
    }

    async fn server_version(&self) -> Result<String> {
        self.worker.server_version().await
    }

    async fn ping(&self) -> Result<()> {
        self.worker.ping().await
    }

    async fn tx_mode(&self) -> TxMode {
        self.worker.tx_mode()
    }

    async fn set_tx_mode(&self, mode: TxMode) -> Result<()> {
        self.worker.set_tx_mode(mode).await
    }

    async fn commit(&self) -> Result<()> {
        self.worker.commit().await
    }

    async fn rollback(&self) -> Result<()> {
        self.worker.rollback().await
    }

    /// Closing a closed or broken connection is not an error.
    async fn close(&self) -> Result<()> {
        self.worker.close().await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use db_core::ErrorKind;

    use super::*;
    use crate::testing::mock_connection;

    async fn run(conn: &OdpiConnection, sql: &str) -> Result<Outcome> {
        conn.execute(sql, &[], &ExecOptions::default()).await
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

    #[test]
    fn worker_config_follows_client_call_timeouts() {
        let s = Duration::from_secs;
        assert_eq!(worker_config(false).watchdog(Some(s(10))), Some(s(10)));
        assert_eq!(worker_config(true).watchdog(Some(s(10))), Some(s(22)));
        assert_eq!(worker_config(true).on_abandon, OnAbandon::BreakSession);
    }

    #[tokio::test]
    async fn cancel_from_another_task_breaks_the_statement_and_keeps_the_connection() {
        let (conn, script) = mock_connection().await;
        let conn = Arc::new(conn);
        let runner = {
            let conn = conn.clone();
            tokio::spawn(async move { conn.execute("block", &[], &ExecOptions::default()).await })
        };
        wait_until(|| script.entries().contains(&"execute block".to_string())).await;
        conn.cancel().await.unwrap();
        let e = runner.await.unwrap().unwrap_err();
        assert_eq!(e.kind, ErrorKind::Cancelled);
        assert_eq!(e.code.as_deref(), Some("ORA-01013"));
        assert_eq!(script.breaks.load(Ordering::SeqCst), 1);
        assert!(run(&conn, "affected").await.is_ok());
    }

    #[tokio::test]
    async fn statement_timeout_reports_timeout() {
        let (conn, _) = mock_connection().await;
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
        let (conn, script) = mock_connection().await;
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
    async fn a_lost_session_breaks_the_connection() {
        let (conn, script) = mock_connection().await;
        let e = run(&conn, "lost").await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::ConnectionLost);
        assert_eq!(e.code.as_deref(), Some("ORA-03114"));
        wait_until(|| script.closed.load(Ordering::SeqCst)).await;
    }

    #[tokio::test]
    async fn close_is_idempotent_and_later_calls_fail() {
        let (conn, script) = mock_connection().await;
        conn.close().await.unwrap();
        conn.close().await.unwrap();
        assert!(script.closed.load(Ordering::SeqCst));
        assert_eq!(
            run(&conn, "affected").await.unwrap_err().kind,
            ErrorKind::ConnectionLost
        );
    }

    #[tokio::test]
    async fn tx_modes() {
        let (conn, script) = mock_connection().await;
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
        let (conn, _) = mock_connection().await;
        assert_eq!(conn.backend(), Backend::OracleOdpi);
        assert_eq!(conn.dialect(), db_core::Dialect::Oracle);
        assert_eq!(conn.server_version().await.unwrap(), "Oracle Mock 1.0");
    }
}
