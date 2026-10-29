//! Sessions, jobs and data source handles over the scripted fake source.
//! Every test waits on a standard channel from a plain thread, as the UI
//! would: nothing here runs inside the database runtime.

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use db_core::{Backend, ErrorKind, Purpose, TxMode, Value};

use crate::fake::{FakeSource, OpenBehavior, Reaction};
use crate::*;

/// Long enough for a loaded CI machine; a passing test never waits this long.
const WAIT: Duration = Duration::from_secs(20);

struct Harness {
    _runtime: DbRuntime,
    source: FakeSource,
    handle: DataSourceHandle,
    rx: Receiver<Event>,
}

fn harness(backend: Backend, options: DataSourceOptions) -> Harness {
    let runtime = DbRuntime::new().unwrap();
    let source = FakeSource::new(backend);
    let (sink, rx) = EventSink::channel();
    let handle = runtime.data_source(
        "fake".into(),
        options,
        Arc::new(source.clone()) as Arc<dyn db_core::ConnectionSource>,
        sink,
    );
    Harness {
        _runtime: runtime,
        source,
        handle,
        rx,
    }
}

fn manual() -> SessionOptions {
    SessionOptions {
        tx_mode: Some(TxMode::Manual),
        schema: None,
        isolation: None,
    }
}

impl Harness {
    fn next(&self) -> Event {
        self.rx
            .recv_timeout(WAIT)
            .expect("an event within the wait time")
    }

    /// Skips events until `pick` returns a value.
    fn wait_for<T>(&self, mut pick: impl FnMut(&EventKind) -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let e = self
                .rx
                .recv_timeout(left)
                .expect("the expected event within the wait time");
            if let Some(v) = pick(&e.kind) {
                return v;
            }
        }
    }

    /// The events of `job` up to and including `Done`.
    fn job(&self, job: JobId) -> Vec<JobEvent> {
        let mut out = Vec::new();
        loop {
            if let EventKind::Job { job: j, event, .. } = self.next().kind
                && j == job
            {
                let done = matches!(event, JobEvent::Done { .. });
                out.push(event);
                if done {
                    return out;
                }
            }
        }
    }

    fn run(&self, session: &Session, script: &str) -> Vec<JobEvent> {
        let job = session.execute(Job::script(script));
        self.job(job)
    }

    fn console_sql(&self) -> Vec<String> {
        self.source.statements(Purpose::Console)
    }
}

fn outcome(events: &[JobEvent]) -> JobOutcome {
    match events.last() {
        Some(JobEvent::Done { outcome }) => *outcome,
        other => panic!("no Done at the end: {other:?}"),
    }
}

/// `Started 0`, `ResultOpened 0` ... as short names, to compare orders.
fn names(events: &[JobEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| match e {
            JobEvent::Started { index, .. } => format!("started {index}"),
            JobEvent::ResultOpened { index, .. } => format!("result {index}"),
            JobEvent::Affected { index, .. } => format!("affected {index}"),
            JobEvent::Finished { index, .. } => format!("finished {index}"),
            JobEvent::Failed { index, .. } => format!("failed {index}"),
            JobEvent::Done { .. } => "done".into(),
        })
        .collect()
}

#[test]
fn three_statements_report_in_order_with_timing_and_error_position() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    h.source.on(
        "select bad",
        Reaction::query_error("42703", "column \"bad\" does not exist"),
    );
    h.source.on(
        "slow",
        Reaction::Delay(Duration::from_millis(30), Box::new(Reaction::Affected(2))),
    );
    let session = h.handle.open_session(SessionOptions::default());
    let script = "select 1;\nupdate slow set a = 1;\nselect bad";
    let events = h.run(&session, script);
    assert_eq!(
        names(&events),
        [
            "started 0",
            "result 0",
            "finished 0",
            "started 1",
            "affected 1",
            "finished 1",
            "started 2",
            "failed 2",
            "done"
        ]
    );
    let JobEvent::ResultOpened { rows, has_more, .. } = &events[1] else {
        unreachable!()
    };
    assert_eq!(rows, &vec![vec![Value::Int(1)]]);
    assert!(!has_more);
    let JobEvent::Affected { rows, .. } = &events[4] else {
        unreachable!()
    };
    assert_eq!(*rows, 2);
    let JobEvent::Finished { timing, .. } = &events[5] else {
        unreachable!()
    };
    assert!(timing.execute >= Duration::from_millis(30), "{timing:?}");
    let JobEvent::Failed { error, .. } = &events[7] else {
        unreachable!()
    };
    assert_eq!(error.error.code.as_deref(), Some("42703"));
    assert_eq!(error.position, script.find("select bad").unwrap());
    assert_eq!(outcome(&events), JobOutcome::Failed);
}

#[test]
fn continue_policy_runs_past_a_failure() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    h.source.on("boom", Reaction::query_error("XX000", "boom"));
    let session = h.handle.open_session(SessionOptions::default());
    let job = session.execute(Job::script("select boom; select 2").on_error(ErrorPolicy::Continue));
    let events = h.job(job);
    assert_eq!(
        names(&events),
        [
            "started 0",
            "failed 0",
            "started 1",
            "result 1",
            "finished 1",
            "done"
        ]
    );
    assert_eq!(outcome(&events), JobOutcome::Completed);
}

#[test]
fn cancel_stops_a_hanging_statement_and_the_session_goes_on() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    h.source.on("pg_sleep", Reaction::Hang);
    let session = h.handle.open_session(SessionOptions::default());
    let job = session.execute(Job::script("select pg_sleep(60)"));
    h.wait_for(|k| match k {
        EventKind::Job {
            event: JobEvent::Started { .. },
            ..
        } => Some(()),
        _ => None,
    });
    let asked = Instant::now();
    session.cancel();
    let events = h.job(job);
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "{:?}",
        asked.elapsed()
    );
    let JobEvent::Failed { error, .. } = &events[0] else {
        panic!("{events:?}")
    };
    assert_eq!(error.error.kind, ErrorKind::Cancelled);
    assert_eq!(outcome(&events), JobOutcome::Cancelled);
    assert!(h.console_sql().contains(&"<cancel>".to_owned()));

    let events = h.run(&session, "select 1");
    assert_eq!(outcome(&events), JobOutcome::Completed);
    assert_eq!(h.source.opened(Purpose::Console), 1, "same connection");
}

#[test]
fn a_driver_without_cancel_reconnects_and_reports_the_lost_transaction() {
    let h = harness(Backend::OracleThin, DataSourceOptions::default());
    h.source.set_cancel_supported(false);
    h.source.on("dbms_session.sleep", Reaction::Hang);
    let session = h.handle.open_session(manual());
    let events = h.run(&session, "insert into t values (1)");
    assert_eq!(outcome(&events), JobOutcome::Completed);
    assert_eq!(session.status().tx.pending, 1);

    let job = session.execute(Job::script("begin dbms_session.sleep(60); end;"));
    h.wait_for(|k| match k {
        EventKind::Job {
            event: JobEvent::Started { .. },
            ..
        } => Some(()),
        _ => None,
    });
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
    assert_eq!(session.status().tx.pending, 0);

    let events = h.run(&session, "select 1 from dual");
    assert_eq!(outcome(&events), JobOutcome::Completed);
    assert_eq!(h.source.opened(Purpose::Console), 2, "a new connection");
}

#[test]
fn a_statement_still_running_after_the_grace_period_is_abandoned() {
    let h = harness(
        Backend::Postgres,
        DataSourceOptions {
            cancel_grace: Duration::from_millis(200),
            ..DataSourceOptions::default()
        },
    );
    h.source.on("stuck", Reaction::HangIgnoringCancel);
    let session = h.handle.open_session(SessionOptions::default());
    let job = session.execute(Job::script("select stuck"));
    h.wait_for(|k| match k {
        EventKind::Job {
            event: JobEvent::Started { .. },
            ..
        } => Some(()),
        _ => None,
    });
    session.cancel();
    assert_eq!(outcome(&h.job(job)), JobOutcome::Cancelled);
    assert_eq!(outcome(&h.run(&session, "select 1")), JobOutcome::Completed);
    assert_eq!(h.source.opened(Purpose::Console), 2);
}

#[test]
fn manual_mode_keeps_the_transaction_after_a_failed_statement() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    h.source
        .on("1/0", Reaction::query_error("22012", "division by zero"));
    let session = h.handle.open_session(manual());
    let job = session.execute(
        Job::script("insert into t values (1); select 1/0; insert into t values (2)")
            .on_error(ErrorPolicy::Continue),
    );
    let events = h.job(job);
    assert_eq!(
        names(&events),
        [
            "started 0",
            "affected 0",
            "finished 0",
            "started 1",
            "failed 1",
            "started 2",
            "affected 2",
            "finished 2",
            "done"
        ],
        "the second insert runs in a live transaction"
    );
    let status = session.status().tx;
    assert!(status.open);
    assert_eq!(status.pending, 2);
    let sql: Vec<String> = h
        .console_sql()
        .into_iter()
        .filter(|s| !s.starts_with("SELECT current_schema"))
        .collect();
    assert_eq!(
        sql,
        [
            "<tx manual>",
            "BEGIN",
            "SAVEPOINT px_stmt",
            "insert into t values (1)",
            "RELEASE SAVEPOINT px_stmt",
            "SAVEPOINT px_stmt",
            "select 1/0",
            "ROLLBACK TO SAVEPOINT px_stmt",
            "RELEASE SAVEPOINT px_stmt",
            "SAVEPOINT px_stmt",
            "insert into t values (2)",
            "RELEASE SAVEPOINT px_stmt",
        ]
    );

    // A refresh uses a connection of its own and never touches the console
    // (the fake knows no catalog: the refresh fails, which is fine here).
    h.source.on(
        "pg_",
        Reaction::query_error("42P01", "no catalog in the fake"),
    );
    let console_before = h.console_sql().len();
    let job = h.handle.refresh(
        db_core::schema::Scope::current(),
        db_core::schema::Level::Names,
    );
    h.wait_for(|k| match k {
        EventKind::Refresh {
            job: j,
            event: RefreshEvent::Finished { .. } | RefreshEvent::Failed { .. },
        } if *j == job => Some(()),
        _ => None,
    });
    assert_eq!(h.source.opened(Purpose::Meta), 1);
    assert_eq!(h.console_sql().len(), console_before);
    assert_eq!(session.status().tx.pending, 2);

    session.commit();
    h.wait_for(|k| match k {
        EventKind::Session {
            event:
                SessionEvent::CommandDone {
                    command: Command::Commit,
                },
            ..
        } => Some(()),
        _ => None,
    });
    assert_eq!(session.status().tx.pending, 0);
    assert!(h.console_sql().ends_with(&["<commit>".to_owned()]));
}

#[test]
fn oracle_moves_one_savepoint_and_ddl_ends_the_transaction() {
    let h = harness(Backend::OracleThin, DataSourceOptions::default());
    let session = h.handle.open_session(manual());
    h.run(&session, "insert into t values (1)");
    h.run(&session, "update t set a = 2");
    assert_eq!(session.status().tx.pending, 2);
    let sql = h.console_sql();
    assert!(!sql.iter().any(|s| s.starts_with("RELEASE") || s == "BEGIN"));
    assert_eq!(sql.iter().filter(|s| *s == "SAVEPOINT px_stmt").count(), 2);
    h.run(&session, "create table x (a number)");
    assert_eq!(session.status().tx.pending, 0, "DDL commits on Oracle");
}

#[test]
fn rows_are_paged_from_the_open_cursor() {
    let h = harness(
        Backend::Postgres,
        DataSourceOptions {
            page_size: 3,
            ..DataSourceOptions::default()
        },
    );
    let values: Vec<String> = (1..=7).map(|i| i.to_string()).collect();
    let refs: Vec<&str> = values.iter().map(String::as_str).collect();
    h.source.on("from big", Reaction::column("n", &refs));
    let session = h.handle.open_session(SessionOptions::default());
    let events = h.run(&session, "select n from big");
    let JobEvent::ResultOpened {
        result,
        rows,
        has_more,
        ..
    } = &events[1]
    else {
        panic!("{events:?}")
    };
    assert_eq!((rows.len(), *has_more), (3, true));
    let result = *result;
    let page = |h: &Harness| {
        session.fetch(result, None);
        h.wait_for(|k| match k {
            EventKind::Session {
                event:
                    SessionEvent::Page {
                        first_row,
                        rows,
                        has_more,
                        ..
                    },
                ..
            } => Some((*first_row, rows.len(), *has_more)),
            _ => None,
        })
    };
    assert_eq!(page(&h), (3, 3, true));
    assert_eq!(page(&h), (6, 1, false));
    h.wait_for(|k| match k {
        EventKind::Session {
            event: SessionEvent::ResultClosed { result: r },
            ..
        } if *r == result => Some(()),
        _ => None,
    });
    session.fetch(result, None);
    h.wait_for(|k| match k {
        EventKind::Session {
            event:
                SessionEvent::CommandFailed {
                    command: Command::Fetch,
                    ..
                },
            ..
        } => Some(()),
        _ => None,
    });
}

#[test]
fn running_another_statement_closes_the_open_result() {
    let h = harness(
        Backend::Postgres,
        DataSourceOptions {
            page_size: 1,
            ..DataSourceOptions::default()
        },
    );
    h.source.on("from two", Reaction::column("n", &["1", "2"]));
    let session = h.handle.open_session(SessionOptions::default());
    let events = h.run(&session, "select n from two");
    let JobEvent::ResultOpened { result, .. } = &events[1] else {
        panic!("{events:?}")
    };
    let result = *result;
    session.execute(Job::script("select 1"));
    h.wait_for(|k| match k {
        EventKind::Session {
            event: SessionEvent::ResultClosed { result: r },
            ..
        } if *r == result => Some(()),
        _ => None,
    });
}

#[test]
fn idle_disconnect_asks_when_changes_are_pending() {
    let h = harness(
        Backend::Postgres,
        DataSourceOptions {
            auto_disconnect: Some(Duration::from_millis(150)),
            ..DataSourceOptions::default()
        },
    );
    let session = h.handle.open_session(manual());
    h.run(&session, "insert into t values (1)");
    let (reason, sessions) = h.wait_for(|k| match k {
        EventKind::DecisionNeeded { reason, sessions } => Some((*reason, sessions.clone())),
        _ => None,
    });
    assert_eq!(reason, DecisionReason::IdleDisconnect);
    assert_eq!(sessions, vec![session.id()]);
    assert_eq!(h.source.open_now(), 1, "nothing was closed");
    assert!(!h.console_sql().contains(&"<close>".to_owned()));
    assert_eq!(session.status().tx.pending, 1);

    // Without pending changes the next idle period disconnects.
    session.rollback();
    h.wait_for(|k| match k {
        EventKind::State {
            state: DataSourceState::Disconnected,
        } => Some(()),
        _ => None,
    });
    assert_eq!(h.source.open_now(), 0);
    // The session reconnects with its next statement.
    assert_eq!(outcome(&h.run(&session, "select 1")), JobOutcome::Completed);
    assert_eq!(h.source.opened(Purpose::Console), 2);
}

#[test]
fn disconnect_and_close_ask_first_with_pending_changes() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    let session = h.handle.open_session(manual());
    h.run(&session, "insert into t values (1)");

    h.handle.disconnect(TxPolicy::Ask);
    let reason = h.wait_for(|k| match k {
        EventKind::DecisionNeeded { reason, .. } => Some(*reason),
        _ => None,
    });
    assert_eq!(reason, DecisionReason::Disconnect);
    session.close(TxPolicy::Ask);
    let reason = h.wait_for(|k| match k {
        EventKind::DecisionNeeded { reason, .. } => Some(*reason),
        _ => None,
    });
    assert_eq!(reason, DecisionReason::Close);
    session.set_tx_mode(TxMode::Auto, TxPolicy::Ask);
    let reason = h.wait_for(|k| match k {
        EventKind::DecisionNeeded { reason, .. } => Some(*reason),
        _ => None,
    });
    assert_eq!(reason, DecisionReason::SwitchToAuto);
    assert_eq!(session.status().tx.pending, 1);

    session.close(TxPolicy::Commit);
    h.wait_for(|k| match k {
        EventKind::Session {
            event: SessionEvent::Closed,
            ..
        } => Some(()),
        _ => None,
    });
    let sql = h.console_sql();
    let n = sql.len();
    assert_eq!(sql[n - 2..], ["<commit>".to_owned(), "<close>".to_owned()]);
    assert!(session.status().closed);
}

#[test]
fn disconnect_with_a_policy_closes_everything_and_sessions_reconnect() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    let session = h.handle.open_session(manual());
    h.run(&session, "insert into t values (1)");
    h.handle.connect();
    let deadline = Instant::now() + WAIT;
    while h.source.open_now() < 2 {
        assert!(Instant::now() < deadline, "no data connection");
        std::thread::sleep(Duration::from_millis(10));
    }
    h.handle.disconnect(TxPolicy::Rollback);
    h.wait_for(|k| match k {
        EventKind::State {
            state: DataSourceState::Disconnected,
        } => Some(()),
        _ => None,
    });
    assert!(h.console_sql().contains(&"<rollback>".to_owned()));
    assert_eq!(session.status().tx.pending, 0);
    assert!(!session.status().connected);
    assert_eq!(outcome(&h.run(&session, "select 1")), JobOutcome::Completed);
    assert_eq!(h.source.opened(Purpose::Console), 2);
}

#[test]
fn state_machine_goes_through_connecting_to_failed_or_connected() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    h.source.set_open(OpenBehavior::Fail(
        db_core::Error::new(ErrorKind::Auth, "password authentication failed").with_code("28P01"),
    ));
    let session = h.handle.open_session(SessionOptions::default());
    let events = h.run(&session, "select 1");
    assert_eq!(outcome(&events), JobOutcome::Failed);
    let JobEvent::Failed { error, .. } = &events[1] else {
        panic!("{events:?}")
    };
    assert_eq!(error.error.code.as_deref(), Some("28P01"));
    assert!(matches!(h.handle.state(), DataSourceState::Failed { .. }));

    h.source.set_open(OpenBehavior::Connect);
    h.handle.connect();
    h.wait_for(|k| match k {
        EventKind::State {
            state: DataSourceState::Connecting,
        } => Some(()),
        _ => None,
    });
    h.wait_for(|k| match k {
        EventKind::State {
            state: DataSourceState::Connected,
        } => Some(()),
        _ => None,
    });
}

#[test]
fn no_call_blocks_the_caller_while_connections_hang() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    h.source.set_open(OpenBehavior::Hang);
    let started = Instant::now();
    let session = h.handle.open_session(manual());
    let job = session.execute(Job::script("select 1"));
    session.commit();
    session.rollback();
    session.set_schema("x");
    session.set_tx_mode(TxMode::Auto, TxPolicy::Rollback);
    session.fetch(ResultId::next(), None);
    let refresh = h.handle.refresh(
        db_core::schema::Scope::current(),
        db_core::schema::Level::Names,
    );
    let read = h.handle.read("select 1", 10);
    h.handle.connect();
    h.handle.disconnect(TxPolicy::Rollback);
    let _ = h.handle.state();
    let _ = session.status();
    session.cancel();
    h.handle.cancel_job(refresh);
    h.handle.cancel_job(read);
    session.close(TxPolicy::Rollback);
    // Generous: a blocking call would hang for good.
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
    // The hanging connect gave way to the cancel.
    assert_eq!(outcome(&h.job(job)), JobOutcome::Cancelled);
}

#[test]
fn a_cancelled_incremental_refresh_keeps_the_model() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    h.source.set_open(OpenBehavior::Hang);
    let generation = h.handle.model().generation();
    let before = h.handle.model().get();
    let cancel = |job: JobId| {
        h.wait_for(|k| match k {
            EventKind::Refresh {
                job: j,
                event: RefreshEvent::Started,
            } if *j == job => Some(()),
            _ => None,
        });
        h.handle.cancel_job(job);
        h.wait_for(|k| match k {
            EventKind::Refresh {
                job: j,
                event: RefreshEvent::Cancelled,
            } if *j == job => Some(()),
            _ => None,
        });
    };
    cancel(h.handle.refresh_with(
        db_core::schema::Scope::current(),
        db_core::schema::Level::Structure,
        RefreshMode::Incremental,
    ));
    cancel(h.handle.refresh_objects(
        "public",
        vec![db_core::schema::ObjectRef {
            schema: "public".into(),
            name: "t".into(),
            kind: db_core::schema::ObjectKind::Table,
        }],
        db_core::schema::Level::Structure,
    ));
    assert_eq!(h.handle.model().generation(), generation);
    assert_eq!(h.handle.model().get(), before);
}

#[test]
fn every_new_connection_gets_time_zone_startup_script_and_read_only_mode() {
    let h = harness(
        Backend::Postgres,
        DataSourceOptions {
            startup_script: Some("set work_mem = '64MB';\nset statement_timeout = 0".into()),
            time_zone: Some("UTC".into()),
            read_only: true,
            ..DataSourceOptions::default()
        },
    );
    let session = h.handle.open_session(SessionOptions::default());
    h.run(&session, "select 1");
    let read = h.handle.read("select 2", 10);
    h.wait_for(|k| match k {
        EventKind::DataRead { job, result } if *job == read => {
            assert!(result.is_ok(), "{result:?}");
            Some(())
        }
        _ => None,
    });
    let setup = [
        "SET TIME ZONE 'UTC'",
        "set work_mem = '64MB'",
        "set statement_timeout = 0",
        "SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY",
    ];
    for purpose in [Purpose::Console, Purpose::Data] {
        let sql = h.source.statements(purpose);
        assert_eq!(sql[..4], setup, "{purpose}");
    }
}

#[test]
fn reads_use_data_connections_and_never_the_console() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    let session = h.handle.open_session(manual());
    h.run(&session, "insert into t values (1)");
    let before = h.console_sql().len();
    for _ in 0..3 {
        let job = h.handle.read("select count(*) from t", 1);
        h.wait_for(|k| match k {
            EventKind::DataRead { job: j, result } if *j == job => {
                assert_eq!(result.as_ref().unwrap().rows.len(), 1);
                Some(())
            }
            _ => None,
        });
    }
    assert_eq!(h.console_sql().len(), before);
    assert_eq!(
        h.source.opened(Purpose::Data),
        1,
        "the connection is reused"
    );
}

#[test]
fn keep_alive_pings_idle_sessions() {
    let h = harness(
        Backend::Postgres,
        DataSourceOptions {
            keep_alive: Some(Duration::from_millis(50)),
            ..DataSourceOptions::default()
        },
    );
    let session = h.handle.open_session(SessionOptions::default());
    h.run(&session, "select 1");
    let deadline = Instant::now() + WAIT;
    while !h.console_sql().contains(&"<ping>".to_owned()) {
        assert!(Instant::now() < deadline, "no ping");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_lost_connection_reports_the_lost_transaction_and_reconnects() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    h.source.on("kill", Reaction::LoseConnection);
    let session = h.handle.open_session(manual());
    h.run(&session, "insert into t values (1)");
    let job = session.execute(Job::script("select kill"));
    let lost = h.wait_for(|k| match k {
        EventKind::Session {
            event: SessionEvent::TransactionLost { pending },
            ..
        } => Some(*pending),
        _ => None,
    });
    assert_eq!(lost, 1);
    assert_eq!(outcome(&h.job(job)), JobOutcome::Failed);
    assert_eq!(outcome(&h.run(&session, "select 1")), JobOutcome::Completed);
    assert_eq!(h.source.opened(Purpose::Console), 2);
}

#[test]
fn single_session_mode_shares_one_session() {
    let h = harness(
        Backend::Postgres,
        DataSourceOptions {
            single_session: true,
            ..DataSourceOptions::default()
        },
    );
    let a = h.handle.open_session(SessionOptions::default());
    let b = h.handle.open_session(SessionOptions::default());
    assert_eq!(a.id(), b.id());
}

#[test]
fn schema_is_read_after_connecting_and_after_set_schema() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    h.source.on(
        "current_schema",
        Reaction::column("current_schema", &["public"]),
    );
    let session = h.handle.open_session(SessionOptions::default());
    h.run(&session, "select 1");
    assert_eq!(session.status().schema.as_deref(), Some("public"));
    session.set_schema("Sales");
    h.wait_for(|k| match k {
        EventKind::Session {
            event:
                SessionEvent::CommandDone {
                    command: Command::SetSchema,
                },
            ..
        } => Some(()),
        _ => None,
    });
    assert!(
        h.console_sql()
            .contains(&"SET search_path TO \"Sales\"".to_owned())
    );
}

#[test]
fn connect_reads_the_schema_before_any_statement() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    h.source.on(
        "current_schema",
        Reaction::column("current_schema", &["public"]),
    );
    let session = h.handle.open_session(SessionOptions::default());
    assert_eq!(session.status().schema, None);
    session.connect();
    h.wait_for(|k| match k {
        EventKind::Session {
            event: SessionEvent::Schema { schema },
            ..
        } => Some(schema.clone()),
        _ => None,
    });
    assert_eq!(session.status().schema.as_deref(), Some("public"));
    assert_eq!(h.source.opened(Purpose::Console), 1);
}

#[test]
fn set_schema_before_the_first_statement_connects_and_switches() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    h.source.on(
        "current_schema",
        Reaction::column("current_schema", &["public"]),
    );
    let session = h.handle.open_session(SessionOptions::default());
    session.set_schema("Sales");
    h.wait_for(|k| match k {
        EventKind::Session {
            event:
                SessionEvent::CommandDone {
                    command: Command::SetSchema,
                },
            ..
        } => Some(()),
        _ => None,
    });
    assert!(
        h.console_sql()
            .contains(&"SET search_path TO \"Sales\"".to_owned())
    );
    assert_eq!(h.source.opened(Purpose::Console), 1);
}

#[test]
fn isolation_is_set_now_and_after_a_reconnect_and_refused_where_unsupported() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    let session = h.handle.open_session(SessionOptions::default());
    h.run(&session, "select 1");
    session.set_isolation(Isolation::Serializable);
    h.wait_for(|k| match k {
        EventKind::Session {
            event:
                SessionEvent::CommandDone {
                    command: Command::SetIsolation,
                },
            ..
        } => Some(()),
        _ => None,
    });
    assert!(h.console_sql().contains(
        &"SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL SERIALIZABLE".to_owned()
    ));

    let h = harness(Backend::OracleThin, DataSourceOptions::default());
    let session = h.handle.open_session(SessionOptions::default());
    h.run(&session, "select 1 from dual");
    session.set_isolation(Isolation::RepeatableRead);
    let error = h.wait_for(|k| match k {
        EventKind::Session {
            event:
                SessionEvent::CommandFailed {
                    command: Command::SetIsolation,
                    error,
                },
            ..
        } => Some(error.clone()),
        _ => None,
    });
    assert_eq!(error.kind, ErrorKind::Unsupported);
}

#[test]
fn headless_command_runs_a_script() {
    let runtime = DbRuntime::new().unwrap();
    let source = FakeSource::new(Backend::Postgres);
    let mut registry = ::commands::Registry::new();
    commands::register(
        &mut registry,
        runtime.clone(),
        Arc::new(source.clone()),
        Arc::new(|_| DataSourceOptions::default()),
    );
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let out = rt
        .block_on(registry.invoke(
            "db.session.run",
            serde_json::json!({"data_source": "fake", "sql": "select 1; insert into t values (1)", "tx_mode": "manual"}),
        ))
        .unwrap();
    let types: Vec<&str> = out["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        [
            "started",
            "result_opened",
            "finished",
            "started",
            "affected",
            "finished",
            "done"
        ]
    );
    assert_eq!(out["transaction"]["pending"], 1);
    // Manual mode without `commit` rolls back.
    let deadline = Instant::now() + WAIT;
    while !source
        .statements(Purpose::Console)
        .contains(&"<rollback>".to_owned())
    {
        assert!(Instant::now() < deadline, "{:?}", source.log());
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_server_position_is_mapped_into_the_script() {
    let h = harness(Backend::Postgres, DataSourceOptions::default());
    // Character 9 of the statement, after a two-byte character.
    h.source.on(
        "ünd",
        Reaction::Fail(
            db_core::Error::query("syntax error")
                .with_code("42601")
                .with_position(9),
        ),
    );
    let session = h.handle.open_session(SessionOptions::default());
    let script = "select 1;\nselect ünd x";
    let events = h.run(&session, script);
    let JobEvent::Failed { error, .. } = &events[4] else {
        panic!("{events:?}")
    };
    assert_eq!(&script[error.position..], "d x");
}
