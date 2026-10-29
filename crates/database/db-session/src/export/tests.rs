//! The export job over the scripted fake source.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use db_core::{Backend, Value};

use super::*;
use crate::fake::{FakeSource, Reaction};
use crate::pager::{TableQuery, TableRef};
use crate::{DataSourceOptions, DbRuntime, EventSink};

const WAIT: Duration = Duration::from_secs(20);

struct Harness {
    _runtime: DbRuntime,
    source: FakeSource,
    handle: crate::DataSourceHandle,
}

fn harness() -> Harness {
    let runtime = DbRuntime::new().unwrap();
    let source = FakeSource::new(Backend::Postgres);
    let (sink, _rx) = EventSink::channel();
    let handle = runtime.data_source(
        "fake".into(),
        DataSourceOptions::default(),
        Arc::new(source.clone()) as Arc<dyn db_core::ConnectionSource>,
        sink,
    );
    Harness {
        _runtime: runtime,
        source,
        handle,
    }
}

fn start(h: &Harness, request: ExportRequest) -> (crate::JobId, Receiver<ExportEvent>) {
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

fn finish(rx: &Receiver<ExportEvent>) -> ExportOutcome {
    loop {
        match rx.recv_timeout(WAIT).expect("an export event") {
            ExportEvent::Finished { outcome, .. } => return outcome,
            ExportEvent::Progress { .. } => {}
        }
    }
}

fn numbers(n: usize) -> Reaction {
    Reaction::Rows {
        columns: vec!["id".into(), "name".into()],
        rows: (0..n)
            .map(|i| vec![Value::Int(i as i64), Value::Text(format!("name, {i}"))])
            .collect(),
    }
}

fn request(target: ExportTarget) -> ExportRequest {
    let mut options = ExportOptions::new(Format::Csv, db_core::Dialect::Postgres);
    options.column_header = true;
    ExportRequest {
        source: ExportSource::Table(TableQuery::new(TableRef::new(Some("demo"), "events"))),
        options,
        target,
        limit: None,
    }
}

#[test]
fn a_table_is_written_to_a_file_in_pages() {
    let h = harness();
    h.source.on("from demo.events", numbers(2500));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.csv");
    let (_, rx) = start(&h, request(ExportTarget::File(path.clone())));
    let ExportOutcome::Done(done) = finish(&rx) else {
        panic!("not done");
    };
    assert_eq!(done.rows, 2500);
    assert_eq!(done.path.as_deref(), Some(path.as_path()));
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 2501);
    assert!(text.starts_with("id,name\n0,\"name, 0\"\n"));
    assert_eq!(done.bytes, text.len() as u64);
    // The statement had no page limit; nothing is left beside the file.
    let sent = h.source.statements(db_core::Purpose::Data);
    assert_eq!(sent, ["SELECT t.*\nFROM demo.events t"]);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn memory_target_with_a_limit_is_the_preview() {
    let h = harness();
    h.source.on("select", numbers(50));
    let mut r = request(ExportTarget::Memory);
    r.source = ExportSource::Sql("select * from t".into());
    r.limit = Some(10);
    let (_, rx) = start(&h, r);
    let ExportOutcome::Done(done) = finish(&rx) else {
        panic!("not done");
    };
    assert_eq!(done.rows, 10);
    assert_eq!(done.text.unwrap().lines().count(), 11);
}

#[test]
fn sql_formats_know_the_table_and_the_dialect() {
    let h = harness();
    h.source.on("from demo.events", numbers(2));
    let mut r = request(ExportTarget::Memory);
    r.options.format = Format::SqlInserts;
    let (_, rx) = start(&h, r);
    let ExportOutcome::Done(done) = finish(&rx) else {
        panic!("not done");
    };
    assert_eq!(
        done.text.unwrap(),
        "INSERT INTO demo.events (id, name) VALUES (0, 'name, 0');\n\
         INSERT INTO demo.events (id, name) VALUES (1, 'name, 1');\n"
    );
}

#[test]
fn cancelled_export_leaves_no_file() {
    let h = harness();
    h.source.on("from demo.events", Reaction::Hang);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.csv");
    let (job, rx) = start(&h, request(ExportTarget::File(path.clone())));
    std::thread::sleep(Duration::from_millis(200));
    h.handle.cancel_job(job);
    assert_eq!(finish(&rx), ExportOutcome::Cancelled);
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_failing_statement_is_reported() {
    let h = harness();
    h.source.on(
        "from demo.events",
        Reaction::query_error("42P01", "relation \"demo.events\" does not exist"),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.csv");
    let (_, rx) = start(&h, request(ExportTarget::File(path.clone())));
    let ExportOutcome::Failed(e) = finish(&rx) else {
        panic!("not failed");
    };
    assert_eq!(e.code.as_deref(), Some("42P01"));
    assert!(!path.exists());
}

#[test]
fn an_existing_file_is_replaced_only_when_complete() {
    let h = harness();
    h.source.on("from demo.events", Reaction::Hang);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.csv");
    std::fs::write(&path, "old").unwrap();
    let (job, rx) = start(&h, request(ExportTarget::File(path.clone())));
    std::thread::sleep(Duration::from_millis(200));
    h.handle.cancel_job(job);
    assert_eq!(finish(&rx), ExportOutcome::Cancelled);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
}
