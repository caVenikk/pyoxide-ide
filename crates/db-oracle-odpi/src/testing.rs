//! A scripted session for testing the connection protocol without a database.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use db_core::{Column, Error, ErrorKind, Result, Row, TxMode, Value, ValueKind};

use crate::conn::OdpiConnection;
use crate::link::{Breaker, ExecArgs, ExecReply, FetchArgs, Link, Page, Session};

/// What happened to the scripted session, observable from the test.
#[derive(Default)]
pub(crate) struct Script {
    log: Mutex<Vec<String>>,
    pub break_requested: AtomicBool,
    pub breaks: AtomicUsize,
    pub closed: AtomicBool,
}

impl Script {
    fn log(&self, s: impl Into<String>) {
        self.log.lock().unwrap().push(s.into());
    }

    pub fn entries(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

pub(crate) struct MockBreaker(pub Arc<Script>);

impl Breaker for MockBreaker {
    fn break_execution(&self) -> Result<()> {
        self.0.breaks.fetch_add(1, Ordering::SeqCst);
        self.0.break_requested.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct MockCursor {
    rows: VecDeque<i64>,
    next: i64,
    fail_at: Option<i64>,
}

/// Statements understood by the script:
/// - `rows N [fail_at K]`: a cursor of N rows `[Int(i)]`, failing at row K;
/// - `affected`: a DML statement that changed 3 rows;
/// - `block`: runs until the breaker is used, then fails like ORA-01013;
/// - `lost`: fails like a dead session (ORA-03114);
/// - anything else: a query error.
pub(crate) struct MockSession {
    script: Arc<Script>,
    cursor: Option<MockCursor>,
}

impl MockSession {
    pub fn new(script: &Arc<Script>) -> Self {
        MockSession {
            script: script.clone(),
            cursor: None,
        }
    }
}

impl Session for MockSession {
    fn execute(&mut self, args: ExecArgs) -> Result<ExecReply> {
        self.script.log(format!("execute {}", args.sql));
        if let Some(spec) = args.sql.strip_prefix("rows ") {
            let mut parts = spec.split_whitespace();
            let n: i64 = parts.next().unwrap().parse().unwrap();
            let fail_at = match (parts.next(), parts.next()) {
                (Some("fail_at"), Some(k)) => Some(k.parse().unwrap()),
                _ => None,
            };
            self.cursor = Some(MockCursor {
                rows: (0..n).collect(),
                next: 0,
                fail_at,
            });
            return Ok(ExecReply::Rows(vec![Column {
                name: "N".into(),
                type_name: "NUMBER".into(),
                kind: ValueKind::Int,
                nullable: false,
            }]));
        }
        match args.sql.as_str() {
            "affected" => Ok(ExecReply::Affected(3)),
            "block" => {
                while !self.script.break_requested.swap(false, Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(crate::errors::from_message(
                    "ORA-01013: user requested cancel of current operation",
                    ErrorKind::Query,
                ))
            }
            "lost" => Err(crate::errors::from_message(
                "ORA-03114: not connected to ORACLE",
                ErrorKind::Query,
            )),
            other => Err(Error::query(format!("bad statement {other}"))),
        }
    }

    fn fetch(&mut self, args: FetchArgs) -> Result<Page> {
        let Some(cursor) = self.cursor.as_mut() else {
            return Ok(Page {
                rows: vec![],
                exhausted: true,
            });
        };
        let mut rows: Vec<Row> = Vec::new();
        while rows.len() < args.max_rows {
            let Some(i) = cursor.rows.pop_front() else {
                break;
            };
            if cursor.fail_at == Some(i) {
                self.cursor = None;
                self.script.log("fetch failed");
                return Err(Error::query("boom"));
            }
            cursor.next = i + 1;
            rows.push(vec![Value::Int(i)]);
        }
        let exhausted = rows.len() < args.max_rows;
        if exhausted {
            self.cursor = None;
        }
        self.script.log(format!("fetch {}", rows.len()));
        Ok(Page { rows, exhausted })
    }

    fn close_cursor(&mut self) -> Result<()> {
        if self.cursor.take().is_some() {
            self.script.log("close_cursor");
        }
        Ok(())
    }

    fn ping(&mut self) -> Result<()> {
        self.script.log("ping");
        Ok(())
    }

    fn server_version(&mut self) -> Result<String> {
        Ok("Oracle Mock 1.0".into())
    }

    fn set_tx_mode(&mut self, mode: TxMode) -> Result<()> {
        self.script.log(format!("tx {mode:?}"));
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        self.script.log("commit");
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        self.script.log("rollback");
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.cursor = None;
        self.script.closed.store(true, Ordering::SeqCst);
        self.script.log("close");
        Ok(())
    }
}

pub(crate) fn mock_link(call_timeout_supported: bool) -> (Arc<Link>, Arc<Script>) {
    let script = Arc::new(Script::default());
    let link = Link::spawn(
        MockSession::new(&script),
        Arc::new(MockBreaker(script.clone())),
        call_timeout_supported,
    )
    .unwrap();
    (link, script)
}

/// A connection on a scripted session. Without client-side call timeouts, so
/// the async timer is exact.
pub(crate) fn mock_connection() -> (OdpiConnection, Arc<Script>) {
    let (link, script) = mock_link(false);
    (OdpiConnection::new(link), script)
}
