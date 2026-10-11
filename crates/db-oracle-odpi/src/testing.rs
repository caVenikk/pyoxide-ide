//! A scripted session for testing how the connection is wired to the
//! `db_core::worker` thread, without a database. The page protocol itself is
//! tested in `db_core::worker`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use db_core::worker::{ExecArgs, Executed, Interrupt, Rows, Session, Worker};
use db_core::{Error, ErrorKind, Result, TxMode};

use crate::conn::{OdpiConnection, worker_config};

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

impl Interrupt for MockBreaker {
    fn interrupt(&self) -> Result<()> {
        self.0.breaks.fetch_add(1, Ordering::SeqCst);
        self.0.break_requested.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn may_block(&self) -> bool {
        true
    }
}

/// Statements understood by the script:
/// - `affected`: a DML statement that changed 3 rows;
/// - `block`: runs until the breaker is used, then fails like ORA-01013;
/// - `lost`: fails like a dead session (ORA-03114);
/// - anything else: a query error.
pub(crate) struct MockSession {
    script: Arc<Script>,
}

impl Session for MockSession {
    fn execute(&self, args: ExecArgs, _rows: Rows<'_, Self>) -> Result<Executed> {
        self.script.log(format!("execute {}", args.sql));
        match args.sql.as_str() {
            "affected" => Ok(Executed::affected(3)),
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

    fn ping(&self) -> Result<()> {
        self.script.log("ping");
        Ok(())
    }

    fn server_version(&self) -> Result<String> {
        Ok("Oracle Mock 1.0".into())
    }

    fn set_tx_mode(&self, mode: TxMode) -> Result<()> {
        self.script.log(format!("tx {mode:?}"));
        Ok(())
    }

    fn commit(&self) -> Result<()> {
        self.script.log("commit");
        Ok(())
    }

    fn rollback(&self) -> Result<()> {
        self.script.log("rollback");
        Ok(())
    }

    fn close(self) -> Result<()> {
        self.script.closed.store(true, Ordering::SeqCst);
        self.script.log("close");
        Ok(())
    }
}

/// A connection on a scripted session, configured as `connect` configures
/// a real one. Without client-side call timeouts, so the async timer is
/// exact.
pub(crate) async fn mock_connection() -> (OdpiConnection, Arc<Script>) {
    let script = Arc::new(Script::default());
    let session = MockSession {
        script: script.clone(),
    };
    let worker = Worker::start(
        worker_config(false),
        Arc::new(MockBreaker(script.clone())),
        move || Ok(session),
    )
    .await
    .unwrap();
    (OdpiConnection::new(worker), script)
}
