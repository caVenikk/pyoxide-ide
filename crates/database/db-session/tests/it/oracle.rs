//! Oracle over the thin driver (ignored: needs `docker compose up -d
//! oracle`). The thin driver cannot cancel: a cancelled statement costs the
//! session its connection and an open transaction.

use std::sync::Arc;
use std::time::{Duration, Instant};

use db_core::{ConnectParams, TxMode};
use db_oracle_thin::ThinDriver;
use db_session::{
    DataSourceOptions, ErrorPolicy, EventKind, Job, JobEvent, JobOutcome, Session, SessionEvent,
    SessionOptions,
};

use crate::support::{Harness, env, failure, harness, int, outcome, password, unique};

fn params() -> ConnectParams {
    ConnectParams {
        host: env("PYOXIDE_TEST_ORACLE_HOST", "localhost"),
        port: env("PYOXIDE_TEST_ORACLE_PORT", "1521")
            .parse()
            .unwrap_or(1521),
        database: env("PYOXIDE_TEST_ORACLE_SERVICE", "FREEPDB1"),
        user: "pyoxide".into(),
        password: password(),
        ..ConnectParams::default()
    }
}

fn oracle() -> Harness {
    harness(Arc::new(ThinDriver), params(), DataSourceOptions::default())
}

fn manual() -> SessionOptions {
    SessionOptions {
        tx_mode: Some(TxMode::Manual),
        schema: None,
    }
}

struct Table<'a> {
    h: &'a Harness,
    session: Session,
    name: String,
}

impl<'a> Table<'a> {
    fn new(h: &'a Harness) -> Self {
        let session = h.handle.open_session(SessionOptions::default());
        let name = unique("px_sess").chars().take(30).collect::<String>();
        h.run_ok(
            &session,
            &format!("create table {name} (a number primary key)"),
        );
        Self { h, session, name }
    }
}

impl Drop for Table<'_> {
    fn drop(&mut self) {
        let _ = self
            .h
            .run(&self.session, &format!("drop table {}", self.name));
    }
}

#[test]
#[ignore = "needs the oracle container"]
fn cancel_reconnects_and_reports_the_lost_transaction() {
    let h = oracle();
    let t = Table::new(&h);
    let session = h.handle.open_session(manual());
    h.run_ok(&session, &format!("insert into {} values (1)", t.name));
    assert_eq!(session.status().tx.pending, 1);
    let opened = h.opened();

    let job = session.execute(Job::script("begin dbms_session.sleep(60); end;"));
    h.wait_started(job);
    std::thread::sleep(Duration::from_millis(300));
    let asked = Instant::now();
    session.cancel();
    let lost = h.wait_for(|k| match k {
        EventKind::Session {
            event: SessionEvent::TransactionLost { pending },
            ..
        } => Some(*pending),
        _ => None,
    });
    assert_eq!(lost, 1);
    let events = h.job(job);
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "{:?}",
        asked.elapsed()
    );
    assert_eq!(outcome(&events), JobOutcome::Cancelled);

    h.run_ok(&session, "select 1 from dual");
    assert_eq!(h.opened(), opened + 1, "a new connection");
    let count = format!("select count(*) from {}", t.name);
    assert_eq!(int(&h.read_value(&count)), 0, "the insert is gone");
}

#[test]
#[ignore = "needs the oracle container"]
fn manual_mode_keeps_the_transaction_after_a_failed_statement() {
    let h = oracle();
    let t = Table::new(&h);
    let session = h.handle.open_session(manual());
    let job = session.execute(
        Job::script(format!(
            "insert into {0} values (1); select 1/0 from dual; insert into {0} values (2)",
            t.name
        ))
        .on_error(ErrorPolicy::Continue),
    );
    let events = h.job(job);
    assert_eq!(outcome(&events), JobOutcome::Completed, "{events:?}");
    let error = failure(&events, 1);
    assert_eq!(error.error.code.as_deref(), Some("ORA-01476"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, JobEvent::Affected { index: 2, rows: 1 })),
        "{events:?}"
    );
    assert_eq!(session.status().tx.pending, 2);
    let count = format!("select count(*) from {}", t.name);
    assert_eq!(int(&h.read_value(&count)), 0);
    h.refresh_finished();
    assert_eq!(session.status().tx.pending, 2);
    h.commit(&session);
    assert_eq!(int(&h.read_value(&count)), 2);
}

#[test]
#[ignore = "needs the oracle container"]
fn an_error_carries_the_ora_code_and_the_statement_position() {
    let h = oracle();
    let session = h.handle.open_session(SessionOptions::default());
    let script = "select 1 from dual;\nselect nope from dual";
    let events = h.run(&session, script);
    let error = failure(&events, 1);
    assert_eq!(error.error.code.as_deref(), Some("ORA-00904"));
    assert_eq!(error.position, script.find("select nope").unwrap());
}
