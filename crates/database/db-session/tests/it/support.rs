use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use db_core::{
    ConnectParams, Connection, ConnectionSource, DataSourceId, Driver, Purpose, Result, Secret,
    Value,
};
use db_session::{
    DataSourceHandle, DataSourceOptions, DbRuntime, Event, EventKind, EventSink, Job, JobEvent,
    JobId, JobOutcome, RefreshEvent, Session, SessionEvent,
};

/// Generous: a loaded CI machine or a cold Oracle container.
pub const WAIT: Duration = Duration::from_secs(60);

pub fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

pub fn password() -> Secret {
    Secret::new(env("PYOXIDE_DB_PASSWORD", "pyoxide"))
}

/// Opens connections with one driver and fixed parameters, and counts them.
pub struct DriverSource {
    driver: Arc<dyn Driver>,
    params: ConnectParams,
    opened: Arc<AtomicUsize>,
}

#[async_trait]
impl ConnectionSource for DriverSource {
    async fn open(&self, _: &DataSourceId, _: Purpose) -> Result<Arc<dyn Connection>> {
        let conn = self.driver.connect(&self.params).await?;
        self.opened.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::from(conn))
    }
}

pub struct Harness {
    _runtime: DbRuntime,
    pub handle: DataSourceHandle,
    rx: Receiver<Event>,
    opened: Arc<AtomicUsize>,
}

pub fn harness(
    driver: Arc<dyn Driver>,
    params: ConnectParams,
    options: DataSourceOptions,
) -> Harness {
    let runtime = DbRuntime::new().unwrap();
    let opened = Arc::new(AtomicUsize::new(0));
    let source = DriverSource {
        driver,
        params,
        opened: opened.clone(),
    };
    let (sink, rx) = EventSink::channel();
    let handle = runtime.data_source("it".into(), options, Arc::new(source), sink);
    Harness {
        _runtime: runtime,
        handle,
        rx,
        opened,
    }
}

impl Harness {
    /// Connections opened so far (any purpose).
    pub fn opened(&self) -> usize {
        self.opened.load(Ordering::SeqCst)
    }

    pub fn wait_for<T>(&self, mut pick: impl FnMut(&EventKind) -> Option<T>) -> T {
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

    pub fn job(&self, job: JobId) -> Vec<JobEvent> {
        let mut out = Vec::new();
        self.wait_for(|k| match k {
            EventKind::Job { job: j, event, .. } if *j == job => {
                out.push(event.clone());
                matches!(event, JobEvent::Done { .. }).then_some(())
            }
            _ => None,
        });
        out
    }

    pub fn run(&self, session: &Session, script: &str) -> Vec<JobEvent> {
        self.job(session.execute(Job::script(script)))
    }

    /// Runs and asserts that every statement succeeded.
    pub fn run_ok(&self, session: &Session, script: &str) -> Vec<JobEvent> {
        let events = self.run(session, script);
        assert_eq!(
            outcome(&events),
            JobOutcome::Completed,
            "{script}: {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, JobEvent::Failed { .. })),
            "{script}: {events:?}"
        );
        events
    }

    pub fn wait_started(&self, job: JobId) {
        self.wait_for(|k| match k {
            EventKind::Job {
                job: j,
                event: JobEvent::Started { .. },
                ..
            } if *j == job => Some(()),
            _ => None,
        });
    }

    /// The first value of the first row, read on a data connection.
    pub fn read_value(&self, sql: &str) -> Value {
        let job = self.handle.read(sql, 1);
        self.wait_for(|k| match k {
            EventKind::DataRead { job: j, result } if *j == job => {
                let rs = result.as_ref().unwrap_or_else(|e| panic!("{sql}: {e:?}"));
                Some(rs.rows[0][0].clone())
            }
            _ => None,
        })
    }

    pub fn commit(&self, session: &Session) {
        session.commit();
        self.wait_for(|k| match k {
            EventKind::Session {
                event: SessionEvent::CommandDone { .. },
                ..
            } => Some(()),
            EventKind::Session {
                event: SessionEvent::CommandFailed { error, .. },
                ..
            } => panic!("commit failed: {error:?}"),
            _ => None,
        });
    }

    pub fn refresh_finished(&self) {
        let job = self.handle.refresh(
            db_core::schema::Scope::current(),
            db_core::schema::Level::Names,
        );
        self.wait_for(|k| match k {
            EventKind::Refresh { job: j, event } if *j == job => match event {
                RefreshEvent::Finished { .. } => Some(()),
                RefreshEvent::Started => None,
                other => panic!("refresh: {other:?}"),
            },
            _ => None,
        });
    }
}

pub fn outcome(events: &[JobEvent]) -> JobOutcome {
    match events.last() {
        Some(JobEvent::Done { outcome }) => *outcome,
        other => panic!("no Done at the end: {other:?}"),
    }
}

pub fn failure(events: &[JobEvent], index: usize) -> &db_session::StatementError {
    events
        .iter()
        .find_map(|e| match e {
            JobEvent::Failed {
                index: i, error, ..
            } if *i == index => Some(error),
            _ => None,
        })
        .unwrap_or_else(|| panic!("statement {index} did not fail: {events:?}"))
}

/// A name no other test run uses at the same time.
pub fn unique(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("{prefix}_{}_{nanos}", std::process::id())
}

pub fn int(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.to_text().and_then(|t| t.parse().ok()))
        .unwrap_or_else(|| panic!("not a number: {v:?}"))
}
