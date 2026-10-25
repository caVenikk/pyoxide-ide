//! SQLite, in every run: a file database shared by the connections of a data
//! source.

use std::sync::Arc;
use std::time::{Duration, Instant};

use db_core::{ConnectParams, ErrorKind, TxMode, opt};
use db_session::{
    DataSourceOptions, ErrorPolicy, Job, JobEvent, JobOutcome, SessionOptions, StatementError,
};
use db_sqlite::SqliteDriver;

use crate::support::{Harness, failure, harness, int, outcome};

fn sqlite(dir: &tempfile::TempDir, options: DataSourceOptions) -> Harness {
    let mut params = ConnectParams {
        database: dir.path().join("it.db").to_string_lossy().into_owned(),
        ..ConnectParams::default()
    };
    params
        .options
        .insert(opt::SQLITE_CREATE.into(), "true".into());
    harness(Arc::new(SqliteDriver), params, options)
}

fn manual() -> SessionOptions {
    SessionOptions {
        tx_mode: Some(TxMode::Manual),
        schema: None,
    }
}

#[test]
fn a_job_reports_statements_in_order_with_the_error_position() {
    let dir = tempfile::tempdir().unwrap();
    let h = sqlite(&dir, DataSourceOptions::default());
    let session = h.handle.open_session(SessionOptions::default());
    let script = "create table t (a integer);\ninsert into t values (1), (2);\nselect a frm t";
    let events = h.run(&session, script);
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| match e {
            JobEvent::Started { .. } => "started",
            JobEvent::ResultOpened { .. } => "result",
            JobEvent::Affected { .. } => "affected",
            JobEvent::Finished { .. } => "finished",
            JobEvent::Failed { .. } => "failed",
            JobEvent::Done { .. } => "done",
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "started", "affected", "finished", "started", "affected", "finished", "started",
            "failed", "done"
        ]
    );
    assert!(matches!(events[4], JobEvent::Affected { rows: 2, .. }));
    let StatementError { error, position } = failure(&events, 2);
    assert_eq!(error.kind, ErrorKind::Query);
    assert!(error.code.is_some(), "{error:?}");
    // SQLite points at the token after which the statement stopped making
    // sense: inside the failing statement, past its start.
    let start = script.find("select a frm").unwrap();
    assert!(*position > start && *position < script.len(), "{position}");
    assert_eq!(outcome(&events), JobOutcome::Failed);

    let events = h.run_ok(&session, "select a from t order by a");
    let JobEvent::ResultOpened { rows, has_more, .. } = &events[1] else {
        panic!("{events:?}")
    };
    assert_eq!(rows.len(), 2);
    assert!(!has_more);
}

#[test]
fn cancel_interrupts_a_running_query_and_the_session_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let h = sqlite(&dir, DataSourceOptions::default());
    let session = h.handle.open_session(SessionOptions::default());
    h.run_ok(&session, "select 1");
    let job = session.execute(Job::script(
        "with recursive c(x) as (select 1 union all select x + 1 from c) select count(*) from c",
    ));
    h.wait_started(job);
    // Let the query reach the database.
    std::thread::sleep(Duration::from_millis(100));
    let asked = Instant::now();
    session.cancel();
    let events = h.job(job);
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "{:?}",
        asked.elapsed()
    );
    assert_eq!(outcome(&events), JobOutcome::Cancelled);
    assert_eq!(failure(&events, 0).error.kind, ErrorKind::Cancelled);
    h.run_ok(&session, "select 2");
    assert_eq!(h.opened(), 1, "the same connection");
}

#[test]
fn manual_mode_keeps_the_transaction_after_a_failed_statement() {
    let dir = tempfile::tempdir().unwrap();
    let h = sqlite(&dir, DataSourceOptions::default());
    let setup = h.handle.open_session(SessionOptions::default());
    h.run_ok(&setup, "create table t (a integer primary key)");

    let session = h.handle.open_session(manual());
    let job = session.execute(
        Job::script("insert into t values (1); insert into t values (1); insert into t values (2)")
            .on_error(ErrorPolicy::Continue),
    );
    let events = h.job(job);
    assert_eq!(outcome(&events), JobOutcome::Completed);
    let error = &failure(&events, 1).error;
    assert!(error.message.to_lowercase().contains("unique"), "{error:?}");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, JobEvent::Affected { index: 2, rows: 1 })),
        "the third statement ran in a live transaction: {events:?}"
    );
    let tx = session.status().tx;
    assert!(tx.open);
    assert_eq!(tx.pending, 2);

    // A data connection does not see the uncommitted rows, and a refresh
    // on its own connection does not end the transaction.
    assert_eq!(int(&h.read_value("select count(*) from t")), 0);
    h.refresh_finished();
    assert!(!h.handle.model().get().is_empty());
    assert_eq!(session.status().tx.pending, 2);

    h.commit(&session);
    assert_eq!(session.status().tx.pending, 0);
    assert_eq!(int(&h.read_value("select count(*) from t")), 2);
}

#[test]
fn read_only_mode_is_enforced_by_the_server() {
    let dir = tempfile::tempdir().unwrap();
    {
        let h = sqlite(&dir, DataSourceOptions::default());
        let s = h.handle.open_session(SessionOptions::default());
        h.run_ok(&s, "create table t (a integer)");
    }
    let h = sqlite(
        &dir,
        DataSourceOptions {
            read_only: true,
            startup_script: Some("pragma cache_size = 100".into()),
            ..DataSourceOptions::default()
        },
    );
    let session = h.handle.open_session(SessionOptions::default());
    let events = h.run(&session, "insert into t values (1)");
    assert_eq!(outcome(&events), JobOutcome::Failed);
    let error = &failure(&events, 0).error;
    assert!(
        error.code.as_deref().unwrap_or("").contains("READONLY"),
        "{error:?}"
    );
    let events = h.run_ok(&session, "pragma cache_size");
    let JobEvent::ResultOpened { rows, .. } = &events[1] else {
        panic!("{events:?}")
    };
    assert_eq!(int(&rows[0][0]), 100, "the startup script ran");
}

#[test]
fn results_page_from_the_open_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let h = sqlite(
        &dir,
        DataSourceOptions {
            page_size: 4,
            ..DataSourceOptions::default()
        },
    );
    let session = h.handle.open_session(SessionOptions::default());
    let events = h.run_ok(
        &session,
        "with recursive c(x) as (select 1 union all select x + 1 from c where x < 10) select x from c",
    );
    let JobEvent::ResultOpened {
        result,
        rows,
        has_more,
        ..
    } = &events[1]
    else {
        panic!("{events:?}")
    };
    assert_eq!((rows.len(), *has_more), (4, true));
    session.fetch(*result, Some(100));
    let (first, n, more) = h.wait_for(|k| match k {
        db_session::EventKind::Session {
            event:
                db_session::SessionEvent::Page {
                    first_row,
                    rows,
                    has_more,
                    ..
                },
            ..
        } => Some((*first_row, rows.len(), *has_more)),
        _ => None,
    });
    assert_eq!((first, n, more), (4, 6, false));
}
