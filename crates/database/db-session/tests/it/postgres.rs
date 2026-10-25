//! PostgreSQL (ignored: needs `docker compose up -d postgres`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use db_core::{ConnectParams, ErrorKind, TxMode};
use db_postgres::PostgresDriver;
use db_session::{
    DataSourceOptions, ErrorPolicy, Job, JobEvent, JobOutcome, Session, SessionOptions,
};

use crate::support::{Harness, env, failure, harness, int, outcome, password, unique};

fn params() -> ConnectParams {
    ConnectParams {
        host: env("PYOXIDE_TEST_PG_HOST", "localhost"),
        port: env("PYOXIDE_TEST_PG_PORT", "55432").parse().expect("port"),
        database: "pyoxide".into(),
        user: "pyoxide".into(),
        password: password(),
        ..ConnectParams::default()
    }
}

fn pg(options: DataSourceOptions) -> Harness {
    harness(Arc::new(PostgresDriver), params(), options)
}

/// A table dropped when the test ends.
struct Table<'a> {
    h: &'a Harness,
    session: Session,
    name: String,
}

impl<'a> Table<'a> {
    fn new(h: &'a Harness) -> Self {
        let session = h.handle.open_session(SessionOptions::default());
        let name = unique("px_session");
        h.run_ok(
            &session,
            &format!("create table {name} (a integer primary key)"),
        );
        Self { h, session, name }
    }
}

impl Drop for Table<'_> {
    fn drop(&mut self) {
        let _ = self.h.run(
            &self.session,
            &format!("drop table if exists {}", self.name),
        );
    }
}

#[test]
#[ignore = "needs the postgres container"]
fn a_job_reports_sqlstate_and_position() {
    let h = pg(DataSourceOptions::default());
    let t = Table::new(&h);
    let session = h.handle.open_session(SessionOptions::default());
    let script = format!(
        "select 1;\ninsert into {0} values (1);\nselect nope from {0}",
        t.name
    );
    let events = h.run(&session, &script);
    assert_eq!(outcome(&events), JobOutcome::Failed);
    assert!(matches!(events[1], JobEvent::ResultOpened { .. }));
    assert!(matches!(events[4], JobEvent::Affected { rows: 1, .. }));
    let JobEvent::Finished { timing, .. } = events[5] else {
        panic!("{events:?}")
    };
    assert!(timing.execute > Duration::ZERO);
    let error = failure(&events, 2);
    assert_eq!(error.error.code.as_deref(), Some("42703"));
    assert_eq!(error.position, script.find("nope").unwrap());
}

#[test]
#[ignore = "needs the postgres container"]
fn cancel_stops_pg_sleep_within_a_second_and_the_session_goes_on() {
    let h = pg(DataSourceOptions::default());
    let session = h.handle.open_session(SessionOptions::default());
    h.run_ok(&session, "select 1");
    let job = session.execute(Job::script("select pg_sleep(60)"));
    h.wait_started(job);
    std::thread::sleep(Duration::from_millis(200));
    let asked = Instant::now();
    session.cancel();
    let events = h.job(job);
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "{:?}",
        asked.elapsed()
    );
    assert_eq!(outcome(&events), JobOutcome::Cancelled);
    let error = &failure(&events, 0).error;
    assert_eq!(error.code.as_deref(), Some("57014"), "{error:?}");
    h.run_ok(&session, "select 2");
    assert_eq!(h.opened(), 1, "the same connection");
}

#[test]
#[ignore = "needs the postgres container"]
fn manual_mode_survives_a_failed_statement_and_a_refresh() {
    let h = pg(DataSourceOptions::default());
    let t = Table::new(&h);
    let session = h.handle.open_session(SessionOptions {
        tx_mode: Some(TxMode::Manual),
        schema: None,
    });
    let job = session.execute(
        Job::script(format!(
            "insert into {0} values (1); select 1/0; insert into {0} values (2)",
            t.name
        ))
        .on_error(ErrorPolicy::Continue),
    );
    let events = h.job(job);
    assert_eq!(outcome(&events), JobOutcome::Completed, "{events:?}");
    assert_eq!(failure(&events, 1).error.code.as_deref(), Some("22012"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, JobEvent::Affected { index: 2, rows: 1 })),
        "{events:?}"
    );
    assert_eq!(session.status().tx.pending, 2);

    let count = format!("select count(*) from {}", t.name);
    assert_eq!(int(&h.read_value(&count)), 0, "not visible outside");
    h.refresh_finished();
    assert_eq!(session.status().tx.pending, 2);

    h.commit(&session);
    assert_eq!(int(&h.read_value(&count)), 2);
}

#[test]
#[ignore = "needs the postgres container"]
fn read_only_mode_is_enforced_by_the_server() {
    let h = pg(DataSourceOptions {
        read_only: true,
        time_zone: Some("Asia/Tokyo".into()),
        ..DataSourceOptions::default()
    });
    let session = h.handle.open_session(SessionOptions::default());
    let events = h.run(&session, "create table px_never (a int)");
    assert_eq!(failure(&events, 0).error.code.as_deref(), Some("25006"));
    let events = h.run_ok(&session, "show timezone");
    let JobEvent::ResultOpened { rows, .. } = &events[1] else {
        panic!("{events:?}")
    };
    assert_eq!(rows[0][0].as_str(), Some("Asia/Tokyo"));
    assert_eq!(
        failure(&h.run(&session, "select 1/0"), 0).error.kind,
        ErrorKind::Query
    );
}
