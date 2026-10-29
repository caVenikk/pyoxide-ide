//! Export and import on real databases: SQLite in every run, PostgreSQL and
//! Oracle ignored (see the module docs of this test program). An export read
//! back by an import gives equal data, NULL, binary, unicode, quotes and line
//! breaks included; a failing row leaves the table as it was and is named;
//! a large export streams and stops on request.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use db_core::{ConnectParams, Dialect, Value, opt};
use db_oracle_thin::ThinDriver;
use db_postgres::PostgresDriver;
use db_session::export::writers;
use db_session::{
    ColumnMap, DataSourceOptions, EventKind, ExportEvent, ExportOptions, ExportOutcome,
    ExportRequest, ExportSource, ExportTarget, Format, ImportEvent, ImportFormat, ImportOutcome,
    ImportRequest, SessionOptions, TableQuery, TableRef,
};
use db_sqlite::SqliteDriver;

use crate::support::{Harness, WAIT, env, harness, password, unique};

fn sqlite(dir: &tempfile::TempDir) -> Harness {
    let mut params = ConnectParams {
        database: dir
            .path()
            .join("transfer.db")
            .to_string_lossy()
            .into_owned(),
        ..ConnectParams::default()
    };
    params
        .options
        .insert(opt::SQLITE_CREATE.into(), "true".into());
    harness(Arc::new(SqliteDriver), params, DataSourceOptions::default())
}

fn postgres() -> Harness {
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

fn oracle() -> Harness {
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
    harness(Arc::new(ThinDriver), params, DataSourceOptions::default())
}

/// One engine's table of the round trip.
struct Case {
    dialect: Dialect,
    /// `create table <name> (...)`, `{}` is the name.
    create: &'static str,
    /// The columns of the rows below (names only matter).
    columns: &'static [&'static str],
}

fn rows(dialect: Dialect) -> Vec<Vec<Value>> {
    let binary = |b: &[u8]| Value::Bytes(b.to_vec());
    vec![
        vec![
            Value::Int(1),
            Value::Text("Ada, L.".into()),
            Value::Null,
            Value::Decimal("10.50".into()),
            binary(&[0, 1, 0xfe, 0xff]),
        ],
        vec![
            Value::Int(2),
            Value::Text("Line\nbreak".into()),
            Value::Text("say \"hi\" now".into()),
            Value::Decimal("0".into()),
            Value::Null,
        ],
        vec![
            Value::Int(3),
            Value::Text("Grace Хоппер 日本 😀".into()),
            // An empty text is not NULL.
            if dialect == Dialect::Oracle {
                Value::Text("x".into())
            } else {
                Value::Text(String::new())
            },
            Value::Decimal("-99.99".into()),
            // Oracle has no empty binary value.
            if dialect == Dialect::Oracle {
                Value::Null
            } else {
                binary(&[])
            },
        ],
        vec![
            Value::Int(4),
            Value::Text("it's; tab\there | pipe".into()),
            Value::Text("\\N".into()),
            Value::Null,
            binary(&[0xde, 0xad]),
        ],
    ]
}

fn case_sqlite() -> Case {
    Case {
        dialect: Dialect::Sqlite,
        create: "create table {} (id integer primary key, name text, note text, price decimal(10,2), bin blob)",
        columns: &["id", "name", "note", "price", "bin"],
    }
}

fn case_postgres() -> Case {
    Case {
        dialect: Dialect::Postgres,
        create: "create table {} (id integer primary key, name text, note text, price numeric(10,2), bin bytea)",
        columns: &["id", "name", "note", "price", "bin"],
    }
}

fn case_oracle() -> Case {
    Case {
        dialect: Dialect::Oracle,
        create: "create table {} (id number(10) primary key, name varchar2(200), note clob, price number(10,2), bin raw(100))",
        columns: &["ID", "NAME", "NOTE", "PRICE", "BIN"],
    }
}

fn columns(case: &Case) -> Vec<db_core::Column> {
    case.columns
        .iter()
        .map(|n| db_core::Column {
            name: (*n).into(),
            type_name: "x".into(),
            kind: db_core::ValueKind::Text,
            nullable: true,
        })
        .collect()
}

/// A table of the case holding its rows.
struct Fixture<'a> {
    h: &'a Harness,
    case: &'a Case,
    tables: Vec<String>,
}

impl<'a> Fixture<'a> {
    fn new(h: &'a Harness, case: &'a Case) -> Self {
        Self {
            h,
            case,
            tables: Vec::new(),
        }
    }

    fn session(&self) -> db_session::Session {
        self.h.handle.open_session(SessionOptions::default())
    }

    fn table(&mut self, with_rows: bool) -> String {
        let mut name = unique("px_xfer");
        if self.case.dialect == Dialect::Oracle {
            // Unquoted names are upper case there.
            name = name.to_uppercase();
        }
        let session = self.session();
        self.h
            .run_ok(&session, &self.case.create.replace("{}", &name));
        self.tables.push(name.clone());
        if with_rows {
            let mut options = ExportOptions::new(Format::SqlInserts, self.case.dialect);
            options.table = Some(name.clone());
            let script =
                writers::to_string(&options, &columns(self.case), &rows(self.case.dialect));
            self.h.run_ok(&session, &script);
            self.h.commit(&session);
        }
        name
    }

    fn read(&self, table: &str) -> Vec<Vec<Value>> {
        let job = self
            .h
            .handle
            .read(format!("select * from {table} order by id"), 1000);
        self.h.wait_for(|k| match k {
            EventKind::DataRead { job: j, result } if *j == job => Some(
                result
                    .as_ref()
                    .unwrap_or_else(|e| panic!("{table}: {e:?}"))
                    .rows
                    .clone(),
            ),
            _ => None,
        })
    }
}

impl Drop for Fixture<'_> {
    fn drop(&mut self) {
        let session = self.session();
        for t in &self.tables {
            let _ = self.h.run(&session, &format!("drop table {t}"));
        }
    }
}

fn export(h: &Harness, request: ExportRequest) -> (db_session::JobId, Receiver<ExportEvent>) {
    let (tx, rx) = channel();
    let tx = std::sync::Mutex::new(tx);
    let job = h.handle.export(
        request,
        Arc::new(move |e| {
            let _ = tx.lock().unwrap().send(e);
        }),
    );
    (job, rx)
}

fn export_end(rx: &Receiver<ExportEvent>) -> ExportOutcome {
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if let ExportEvent::Finished { outcome, .. } = rx.recv_timeout(left).expect("an event") {
            return outcome;
        }
    }
}

fn import(h: &Harness, request: ImportRequest) -> ImportOutcome {
    let (tx, rx) = channel();
    let tx = std::sync::Mutex::new(tx);
    h.handle.import(
        request,
        Arc::new(move |e| {
            let _ = tx.lock().unwrap().send(e);
        }),
    );
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if let ImportEvent::Finished { outcome, .. } = rx.recv_timeout(left).expect("an event") {
            return outcome;
        }
    }
}

fn export_csv(h: &Harness, table: &str, file: &Path) -> u64 {
    let mut options = ExportOptions::new(Format::Csv, Dialect::Postgres);
    options.column_header = true;
    let (_, rx) = export(
        h,
        ExportRequest {
            source: ExportSource::Table(TableQuery::new(TableRef::new(None, table))),
            options,
            target: ExportTarget::File(file.to_path_buf()),
            limit: None,
        },
    );
    match export_end(&rx) {
        ExportOutcome::Done(done) => done.rows,
        other => panic!("{other:?}"),
    }
}

fn import_request(table: &str, file: &Path, case: &Case) -> ImportRequest {
    ImportRequest {
        table: TableRef::new(None, table),
        file: file.to_path_buf(),
        format: ImportFormat::default(),
        encoding: None,
        mapping: case
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| ColumnMap {
                column: (*c).into(),
                source: Some(i),
            })
            .collect(),
        inconvertible_as_null: false,
    }
}

fn round_trip(h: &Harness, case: &Case) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.csv");
    let mut fx = Fixture::new(h, case);
    let src = fx.table(true);
    let dst = fx.table(false);
    let before = fx.read(&src);
    assert_eq!(before.len(), 4);
    assert_eq!(export_csv(h, &src, &file), 4);
    let ImportOutcome::Done(done) = import(h, import_request(&dst, &file, case)) else {
        panic!("the import did not finish");
    };
    assert_eq!(done.rows, 4);
    assert_eq!(fx.read(&dst), before);
}

fn violation_in_the_last_row(h: &Harness, case: &Case) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("dup.csv");
    let mut fx = Fixture::new(h, case);
    let dst = fx.table(false);
    // One row that is there already; the file's last row repeats its key.
    let session = fx.session();
    h.run_ok(
        &session,
        &format!("insert into {dst} (id, name) values (1000, 'kept')"),
    );
    h.commit(&session);
    let mut text = String::from("id,name,note,price,bin\n");
    for i in (1..=1200).filter(|i| *i != 1000) {
        text.push_str(&format!("{i},row {i},,1.5,\n"));
    }
    text.push_str("1000,duplicate,,1.5,\n");
    std::fs::write(&file, text).unwrap();
    let outcome = import(h, import_request(&dst, &file, case));
    let ImportOutcome::Failed(failure) = outcome else {
        panic!("expected a failure: {outcome:?}");
    };
    assert_eq!(failure.row, Some(1200), "{failure:?}");
    assert_eq!(failure.line, Some(1201), "{failure:?}");
    let rows = fx.read(&dst);
    assert_eq!(rows.len(), 1, "the table is unchanged: {}", rows.len());
    assert_eq!(rows[0][1], Value::Text("kept".into()));
}

fn conversion_failure_names_the_row(h: &Harness, case: &Case) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bad.csv");
    let mut fx = Fixture::new(h, case);
    let dst = fx.table(false);
    std::fs::write(&file, "id,name,note,price,bin\n1,a,,1.5,\nx,b,,1.5,\n").unwrap();
    let outcome = import(h, import_request(&dst, &file, case));
    let ImportOutcome::Failed(failure) = outcome else {
        panic!("expected a failure: {outcome:?}");
    };
    assert_eq!(failure.row, Some(2));
    assert!(
        failure.reason.to_lowercase().contains("id"),
        "{}",
        failure.reason
    );
    assert!(fx.read(&dst).is_empty());
    // With the option the value becomes NULL (a primary key refuses it, so
    // a column that may be NULL is used).
    let file2 = dir.path().join("bad2.csv");
    std::fs::write(&file2, "id,name,note,price,bin\n1,a,,oops,\n").unwrap();
    let mut req = import_request(&dst, &file2, case);
    req.inconvertible_as_null = true;
    assert!(matches!(import(h, req), ImportOutcome::Done(_)));
    assert_eq!(fx.read(&dst)[0][3], Value::Null);
}

#[test]
fn sqlite_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    round_trip(&sqlite(&dir), &case_sqlite());
}

#[test]
fn sqlite_violation_in_the_last_row_leaves_the_table_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    violation_in_the_last_row(&sqlite(&dir), &case_sqlite());
}

#[test]
fn sqlite_conversion_failures() {
    let dir = tempfile::tempdir().unwrap();
    conversion_failure_names_the_row(&sqlite(&dir), &case_sqlite());
}

#[test]
fn sqlite_export_of_many_rows_streams_and_can_be_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let h = sqlite(&dir);
    let session = h.handle.open_session(SessionOptions::default());
    h.run_ok(
        &session,
        "create table big (id integer primary key, name text);\n\
         with recursive n(i) as (select 1 union all select i + 1 from n where i < 100000)\n\
         insert into big select i, 'name ' || i from n",
    );
    h.commit(&session);
    let file = dir.path().join("big.csv");
    assert_eq!(export_csv(&h, "big", &file), 100_000);
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text.lines().count(), 100_001);
    // A cancelled export leaves no file.
    let second = dir.path().join("cancelled.csv");
    let mut options = ExportOptions::new(Format::Csv, Dialect::Sqlite);
    options.column_header = true;
    let (job, rx) = export(
        &h,
        ExportRequest {
            // A result that takes a while to produce.
            source: ExportSource::Sql(
                "with recursive n(i) as (select 1 union all select i + 1 from n where i < 50000000) select i, i * 2 from n".into(),
            ),
            options,
            target: ExportTarget::File(second.clone()),
            limit: None,
        },
    );
    loop {
        match rx.recv_timeout(WAIT).expect("progress") {
            ExportEvent::Progress { rows, .. } if rows > 0 => break,
            ExportEvent::Finished { outcome, .. } => panic!("finished early: {outcome:?}"),
            ExportEvent::Progress { .. } => {}
        }
    }
    h.handle.cancel_job(job);
    assert_eq!(export_end(&rx), ExportOutcome::Cancelled);
    assert!(!second.exists());
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 2, "big.csv and the database only");
}

#[test]
#[ignore = "needs the postgres container"]
fn postgres_round_trip() {
    round_trip(&postgres(), &case_postgres());
}

#[test]
#[ignore = "needs the postgres container"]
fn postgres_violation_in_the_last_row_leaves_the_table_unchanged() {
    violation_in_the_last_row(&postgres(), &case_postgres());
}

#[test]
#[ignore = "needs the postgres container"]
fn postgres_conversion_failures() {
    conversion_failure_names_the_row(&postgres(), &case_postgres());
}

#[test]
#[ignore = "needs the postgres container"]
fn postgres_export_of_100000_rows_and_a_cancel() {
    let h = postgres();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("series.csv");
    let mut options = ExportOptions::new(Format::Csv, Dialect::Postgres);
    options.column_header = true;
    let request = |sql: &str, path: &Path| ExportRequest {
        source: ExportSource::Sql(sql.into()),
        options: options.clone(),
        target: ExportTarget::File(path.to_path_buf()),
        limit: None,
    };
    let (_, rx) = export(
        &h,
        request(
            "select g as id, md5(g::text) as name from generate_series(1, 100000) g",
            &file,
        ),
    );
    let ExportOutcome::Done(done) = export_end(&rx) else {
        panic!("not done");
    };
    assert_eq!(done.rows, 100_000);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap().lines().count(),
        100_001
    );
    // A long result is stopped on the server.
    let second = dir.path().join("long.csv");
    let (job, rx) = export(
        &h,
        request(
            "select g, md5(g::text) from generate_series(1, 2000000000) g",
            &second,
        ),
    );
    std::thread::sleep(Duration::from_millis(500));
    let asked = Instant::now();
    h.handle.cancel_job(job);
    assert_eq!(export_end(&rx), ExportOutcome::Cancelled);
    assert!(
        asked.elapsed() < Duration::from_secs(5),
        "{:?}",
        asked.elapsed()
    );
    assert!(!second.exists());
}

#[test]
#[ignore = "needs the oracle container"]
fn oracle_round_trip() {
    round_trip(&oracle(), &case_oracle());
}

#[test]
#[ignore = "needs the oracle container"]
fn oracle_violation_in_the_last_row_leaves_the_table_unchanged() {
    violation_in_the_last_row(&oracle(), &case_oracle());
}

#[test]
#[ignore = "needs the oracle container"]
fn oracle_conversion_failures() {
    conversion_failure_names_the_row(&oracle(), &case_oracle());
}

/// The resident memory of this process in KiB, from `ps` (the test has no
/// platform API to ask).
fn rss_kib() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("a number")
}

#[test]
#[ignore = "needs the postgres container"]
fn postgres_export_memory_stays_flat() {
    let h = postgres();
    let dir = tempfile::tempdir().unwrap();
    let mut options = ExportOptions::new(Format::Csv, Dialect::Postgres);
    options.column_header = true;
    let run = |rows: u64, name: &str| {
        let file = dir.path().join(name);
        let (_, rx) = export(
            &h,
            ExportRequest {
                source: ExportSource::Sql(format!(
                    "select g as id, md5(g::text) || md5((g + 1)::text) as name, now() as at from generate_series(1, {rows}) g"
                )),
                options: options.clone(),
                target: ExportTarget::File(file.clone()),
                limit: None,
            },
        );
        let mut peak = rss_kib();
        let outcome = loop {
            match rx.recv_timeout(WAIT).expect("an event") {
                ExportEvent::Progress { .. } => peak = peak.max(rss_kib()),
                ExportEvent::Finished { outcome, .. } => break outcome,
            }
        };
        let ExportOutcome::Done(done) = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(done.rows, rows);
        let size = std::fs::metadata(&file).unwrap().len();
        std::fs::remove_file(&file).unwrap();
        (peak.max(rss_kib()), size)
    };
    // Warm up: connection, buffers.
    let (small, small_size) = run(100_000, "a.csv");
    let (large, large_size) = run(2_000_000, "b.csv");
    eprintln!(
        "100 000 rows: {} MiB file, peak RSS {} MiB; 2 000 000 rows: {} MiB file, peak RSS {} MiB",
        small_size >> 20,
        small >> 10,
        large_size >> 20,
        large >> 10
    );
    // Twenty times the rows, the same memory (a few MiB of slack).
    assert!(
        large < small + 20 * 1024,
        "RSS grew from {small} KiB to {large} KiB"
    );
}
