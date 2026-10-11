//! The real session: ODPI-C calls through the `oracle` crate.

use std::sync::Arc;
use std::time::Duration;

use db_core::{Error, Result, TxMode};
use oracle::sql_type::ToSql;

use crate::binds::binds_from_values;
use crate::errors::map_error;
use crate::link::{Breaker, ExecArgs, ExecReply, FetchArgs, Page, Session};
use crate::sqltext::prepare_sql;
use crate::tx::TxState;
use crate::values::{ConvertedRow, column_from_info, set_max_value_bytes};

/// Rows per round trip are bounded so a large `fetch_size` cannot ask the
/// client library for gigabytes of define buffers.
const MAX_FETCH_ARRAY: usize = 20_000;

pub(crate) struct OracleSession {
    conn: Arc<oracle::Connection>,
    cursor: Option<oracle::ResultSet<'static, ConvertedRow>>,
    tx: TxState,
    call_timeout_supported: bool,
    /// The call timeout currently set on the connection.
    applied_timeout: Option<Duration>,
}

impl OracleSession {
    pub(crate) fn new(
        conn: Arc<oracle::Connection>,
        read_only: bool,
        call_timeout_supported: bool,
    ) -> Self {
        OracleSession {
            conn,
            cursor: None,
            tx: TxState::new(read_only),
            call_timeout_supported,
            applied_timeout: None,
        }
    }

    /// Set the client's call timeout (round trips taking longer fail with
    /// `DPI-1067`). Statements pass their own limit, everything else none.
    fn apply_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        if !self.call_timeout_supported {
            return Ok(());
        }
        let wanted = timeout.map(|d| {
            d.clamp(
                Duration::from_millis(1),
                Duration::from_millis(u64::from(u32::MAX)),
            )
        });
        if wanted == self.applied_timeout {
            return Ok(());
        }
        self.conn
            .set_call_timeout(wanted)
            .map_err(|e| map_error(&e))?;
        self.applied_timeout = wanted;
        Ok(())
    }

    fn start_read_only_transaction(&mut self) -> Result<()> {
        let mut stmt = self
            .conn
            .statement("SET TRANSACTION READ ONLY")
            .build()
            .map_err(|e| map_error(&e))?;
        stmt.execute(&[]).map_err(|e| map_error(&e))?;
        self.tx.read_only_started();
        Ok(())
    }

    fn commit_now(&mut self) -> Result<()> {
        self.apply_timeout(None)?;
        self.conn.commit().map_err(|e| map_error(&e))
    }
}

impl Session for OracleSession {
    fn execute(&mut self, a: ExecArgs) -> Result<ExecReply> {
        // A statement that arrives while an old cursor is still open means the
        // caller dropped it; the one-cursor rule is enforced before this point.
        self.cursor = None;
        let sql = prepare_sql(&a.sql)?;
        let binds = binds_from_values(&a.params)?;
        self.apply_timeout(a.timeout)?;

        let fetch = a.fetch_size.clamp(1, MAX_FETCH_ARRAY) as u32;
        let mut builder = self.conn.statement(sql);
        // Prefetching the page size returns the first page with the execute
        // round trip. LOBs come as locators and are read up to the size limit.
        builder
            .fetch_array_size(fetch)
            .prefetch_rows(fetch)
            .lob_locator();
        let mut stmt = builder.build().map_err(|e| map_error(&e))?;
        let kind = stmt.statement_type();

        let start_read_only = self.tx.admit(kind)?;
        if binds.len() > stmt.bind_count() {
            return Err(Error::query(format!(
                "{} parameters were given but the statement has {} bind variables",
                binds.len(),
                stmt.bind_count()
            )));
        }
        if start_read_only {
            self.start_read_only_transaction()?;
        }
        let params: Vec<&dyn ToSql> = binds.iter().map(|b| b as &dyn ToSql).collect();

        if stmt.is_query() {
            let rows = stmt
                .into_result_set::<ConvertedRow>(&params)
                .map_err(|e| map_error(&e))?;
            let columns = rows.column_info().iter().map(column_from_info).collect();
            self.cursor = Some(rows);
            return Ok(ExecReply::Rows(columns));
        }

        stmt.execute(&params).map_err(|e| map_error(&e))?;
        let affected = stmt.row_count().unwrap_or(0);
        drop(stmt);
        if self.tx.statement_done(kind) {
            self.commit_now()?;
        }
        Ok(ExecReply::Affected(affected))
    }

    fn fetch(&mut self, a: FetchArgs) -> Result<Page> {
        if self.cursor.is_none() {
            return Ok(Page {
                rows: Vec::new(),
                exhausted: true,
            });
        }
        // Before the cursor is taken: a failure here must not lose it.
        self.apply_timeout(a.timeout)?;
        let Some(mut rows) = self.cursor.take() else {
            return Ok(Page {
                rows: Vec::new(),
                exhausted: true,
            });
        };
        set_max_value_bytes(a.max_value_bytes);
        let mut page = Vec::with_capacity(a.max_rows.min(4096));
        let mut exhausted = false;
        while page.len() < a.max_rows {
            match rows.next() {
                None => {
                    exhausted = true;
                    break;
                }
                // The statement is dropped (closed) with `rows` on an error.
                Some(Err(e)) => return Err(map_error(&e)),
                Some(Ok(row)) => {
                    if let Some(e) = row.error {
                        return Err(e);
                    }
                    page.push(row.values);
                }
            }
        }
        if !exhausted {
            self.cursor = Some(rows);
        }
        Ok(Page {
            rows: page,
            exhausted,
        })
    }

    fn close_cursor(&mut self) -> Result<()> {
        // Dropping the result set closes the statement and with it the cursor.
        self.cursor = None;
        Ok(())
    }

    fn ping(&mut self) -> Result<()> {
        self.apply_timeout(None)?;
        self.conn.ping().map_err(|e| map_error(&e))
    }

    fn server_version(&mut self) -> Result<String> {
        let (version, banner) = self.conn.server_version().map_err(|e| map_error(&e))?;
        Ok(format_server_version(&banner, &version.to_string()))
    }

    fn set_tx_mode(&mut self, mode: TxMode) -> Result<()> {
        if mode == self.tx.mode() {
            return Ok(());
        }
        if self.tx.switch_needs_commit(mode) {
            // Leaving manual mode ends the transaction, as autocommit does.
            self.commit_now()?;
        }
        self.tx.set_mode(mode);
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        self.commit_now()?;
        self.tx.transaction_ended();
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        self.apply_timeout(None)?;
        self.conn.rollback().map_err(|e| map_error(&e))?;
        self.tx.transaction_ended();
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.cursor = None;
        self.conn.close().map_err(|e| map_error(&e))
    }
}

/// Interrupts the running call through the client library.
pub(crate) struct OracleBreaker(pub(crate) Arc<oracle::Connection>);

impl Breaker for OracleBreaker {
    fn break_execution(&self) -> Result<()> {
        self.0.break_execution().map_err(|e| map_error(&e))
    }
}

/// `Oracle Database 19c Enterprise Edition Release 19.0.0.0.0 - Production
/// Version 19.21.0.0.0` on one line. The client's banner spans two lines;
/// when it carries no version number the numeric version is appended.
pub(crate) fn format_server_version(banner: &str, version: &str) -> String {
    let one_line = banner.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return format!("Oracle Database {version}");
    }
    if one_line.contains(version) || one_line.contains("Version ") {
        one_line
    } else {
        format!("{one_line} {version}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_on_one_line() {
        assert_eq!(
            format_server_version(
                "Oracle Database 19c Enterprise Edition Release 19.0.0.0.0 - Production\nVersion 19.21.0.0.0",
                "19.21.0.0.0"
            ),
            "Oracle Database 19c Enterprise Edition Release 19.0.0.0.0 - Production Version 19.21.0.0.0"
        );
        assert_eq!(
            format_server_version(
                "Oracle Database 12c Release 12.2.0.1.0 - 64bit Production",
                "12.2.0.1.0"
            ),
            "Oracle Database 12c Release 12.2.0.1.0 - 64bit Production"
        );
        assert_eq!(
            format_server_version("Oracle Database 11g", "11.2.0.4.0"),
            "Oracle Database 11g 11.2.0.4.0"
        );
        assert_eq!(
            format_server_version("", "19.3.0.0.0"),
            "Oracle Database 19.3.0.0.0"
        );
    }
}
