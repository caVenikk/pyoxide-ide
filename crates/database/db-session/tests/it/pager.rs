//! Table pages and counts of the data grid (`ResultPager`): SQLite in every
//! run, PostgreSQL on the demo schema of the container and Oracle
//! (ignored).

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use db_core::{ConnectParams, Dialect, ErrorKind, opt};
use db_oracle_thin::ThinDriver;
use db_postgres::PostgresDriver;
use db_session::{
    DataSourceOptions, JobId, Page, PageError, PageSize, PagerEvent, ResultPager, SessionOptions,
    TableQuery, TableRef,
};
use db_sqlite::SqliteDriver;

use crate::support::{Harness, WAIT, env, harness, int, password};

struct Pager {
    pager: ResultPager,
    rx: Receiver<PagerEvent>,
}

fn pager(h: &Harness) -> Pager {
    let (tx, rx) = std::sync::mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let pager = h.handle.pager(move |e| {
        let _ = tx.lock().unwrap().send(e);
    });
    Pager { pager, rx }
}

impl Pager {
    fn wait(&self, job: JobId) -> PagerEvent {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let e = self.rx.recv_timeout(left).expect("the pager's event");
            let j = match &e {
                PagerEvent::Page { job, .. } | PagerEvent::Count { job, .. } => *job,
            };
            if j == job {
                return e;
            }
        }
    }

    fn page(&self, q: &TableQuery, first: u64, size: PageSize) -> Result<Page, PageError> {
        match self.wait(self.pager.page(q, first, size)) {
            PagerEvent::Page { result, .. } => result,
            other => panic!("{other:?}"),
        }
    }

    fn count(&self, q: &TableQuery) -> Result<u64, PageError> {
        match self.wait(self.pager.count(q)) {
            PagerEvent::Count { result, .. } => result,
            other => panic!("{other:?}"),
        }
    }
}

fn sqlite(dir: &tempfile::TempDir) -> Harness {
    let mut params = ConnectParams {
        database: dir.path().join("pager.db").to_string_lossy().into_owned(),
        ..ConnectParams::default()
    };
    params
        .options
        .insert(opt::SQLITE_CREATE.into(), "true".into());
    harness(Arc::new(SqliteDriver), params, DataSourceOptions::default())
}

#[test]
fn sqlite_pages_count_and_keep_errors_apart() {
    let dir = tempfile::tempdir().unwrap();
    let h = sqlite(&dir);
    let session = h.handle.open_session(SessionOptions::default());
    h.run_ok(
        &session,
        "create table items (id integer primary key, name text);
         with recursive n(i) as (select 1 union all select i + 1 from n where i < 25)
         insert into items select i, 'item ' || i from n;",
    );
    let p = pager(&h);
    let q = TableQuery::new(TableRef::new(None, "items"));
    let first = p.page(&q, 0, PageSize::Rows(10)).unwrap();
    assert_eq!(first.rows.len(), 10);
    assert!(first.has_more);
    assert_eq!(first.columns[0].name, "id");
    assert_eq!(
        first.sql,
        q.page_sql(Dialect::Sqlite, 0, PageSize::Rows(10))
    );
    let last = p.page(&q, 20, PageSize::Rows(10)).unwrap();
    assert_eq!(last.rows.len(), 5);
    assert!(!last.has_more);
    assert_eq!(last.first_row, 20);
    assert_eq!(int(&last.rows[0][0]), 21);
    let all = p.page(&q, 0, PageSize::All).unwrap();
    assert_eq!(all.rows.len(), 25);
    assert!(!all.has_more);
    assert_eq!(p.count(&q).unwrap(), 25);

    let q = TableQuery {
        filter: "id > 20".into(),
        order: "id desc".into(),
        ..q
    };
    let page = p.page(&q, 0, PageSize::Rows(10)).unwrap();
    assert_eq!(page.rows.len(), 5);
    assert_eq!(int(&page.rows[0][0]), 25);
    assert_eq!(p.count(&q).unwrap(), 5);

    let wrong = TableQuery {
        filter: "nonexistent = 1".into(),
        ..q
    };
    let error = p.page(&wrong, 0, PageSize::Rows(10)).unwrap_err();
    assert_eq!(error.error.kind, ErrorKind::Query, "{error:?}");
    assert!(error.sql.contains("WHERE nonexistent = 1"), "{error:?}");
    // The data connection is still usable.
    assert_eq!(
        p.count(&TableQuery::new(TableRef::new(None, "items")))
            .unwrap(),
        25
    );
}

fn pg() -> Harness {
    let params = ConnectParams {
        host: env("PYOXIDE_TEST_PG_HOST", "localhost"),
        port: env("PYOXIDE_TEST_PG_PORT", "55432").parse().expect("port"),
        database: "pyoxide".into(),
        user: "pyoxide".into(),
        password: password(),
        ..ConnectParams::default()
    };
    harness(
        Arc::new(PostgresDriver),
        params,
        DataSourceOptions::default(),
    )
}

#[test]
#[ignore = "needs the postgres container with the demo schema"]
fn postgres_pages_the_demo_events_and_cancels_a_count() {
    let h = pg();
    let p = pager(&h);
    let q = TableQuery::new(TableRef::new(Some("demo"), "events"));
    let first = p.page(&q, 0, PageSize::Rows(500)).unwrap();
    assert_eq!(first.rows.len(), 500);
    assert!(first.has_more);
    assert_eq!(first.sql, "SELECT t.*\nFROM demo.events t\nLIMIT 501");
    let total = p.count(&q).unwrap();
    assert!(total > 500, "{total}");

    let wrong = TableQuery {
        filter: "nonexistent = 1".into(),
        ..q.clone()
    };
    let error = p.page(&wrong, 0, PageSize::Rows(500)).unwrap_err();
    assert_eq!(error.error.code.as_deref(), Some("42703"), "{error:?}");
    let position = error.error.position.expect("the server's position");
    assert_eq!(
        &error.sql[position..position + "nonexistent".len()],
        "nonexistent"
    );

    // A count that would run for hours stops on the server within a second.
    let slow = TableQuery {
        filter: "pg_sleep(1) is not null".into(),
        ..q
    };
    let job = p.pager.count(&slow);
    std::thread::sleep(Duration::from_millis(300));
    let asked = Instant::now();
    p.pager.cancel(job);
    let PagerEvent::Count { result, .. } = p.wait(job) else {
        panic!("a count")
    };
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "{:?}",
        asked.elapsed()
    );
    let error = result.unwrap_err();
    assert!(
        matches!(error.error.kind, ErrorKind::Cancelled | ErrorKind::Query),
        "{error:?}"
    );
}

#[test]
#[ignore = "needs the oracle container"]
fn oracle_pages_with_offset_and_fetch() {
    let params = ConnectParams {
        host: env("PYOXIDE_TEST_ORACLE_HOST", "localhost"),
        port: env("PYOXIDE_TEST_ORACLE_PORT", "1521")
            .parse()
            .unwrap_or(1521),
        database: env("PYOXIDE_TEST_ORACLE_SERVICE", "FREEPDB1"),
        user: "pyoxide".into(),
        password: password(),
        ..ConnectParams::default()
    };
    let h = harness(Arc::new(ThinDriver), params, DataSourceOptions::default());
    let p = pager(&h);
    let q = TableQuery {
        filter: "OBJECT_TYPE = 'TABLE'".into(),
        order: "OBJECT_NAME".into(),
        ..TableQuery::new(TableRef::new(Some("SYS"), "ALL_OBJECTS"))
    };
    let first = p.page(&q, 0, PageSize::Rows(2)).unwrap();
    assert_eq!(first.rows.len(), 2);
    assert!(first.has_more);
    assert!(
        first
            .sql
            .ends_with("ORDER BY OBJECT_NAME\nFETCH NEXT 3 ROWS ONLY"),
        "{}",
        first.sql
    );
    let second = p.page(&q, 2, PageSize::Rows(2)).unwrap();
    assert_eq!(second.first_row, 2);
    assert!(second.sql.contains("OFFSET 2 ROWS"));
    assert!(p.count(&q).unwrap() > 2);
}
