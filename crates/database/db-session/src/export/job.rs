use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use db_core::{Error, ErrorKind, ExecOptions, Outcome, Row};
use serde::Serialize;
use tokio::sync::watch;

use super::format::ExportOptions;
use super::writers::{self, RowWriter};
use crate::data_source::{DataSourceHandle, Shared, cancelled};
use crate::event::JobId;
use crate::pager::{PageSize, TableQuery};
use crate::session::{Guarded, guarded};

/// Rows a fetch asks for while exporting.
const FETCH: usize = 1000;
/// Values are read whole.
const MAX_VALUE: usize = 1 << 30;
/// How often progress is reported at most.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// What is exported.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportSource {
    /// A table or view with the grid's condition and order, every row.
    Table(TableQuery),
    /// A query result: the statement is run as it is.
    Sql(String),
}

/// Where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportTarget {
    /// A file, replaced when the export completes.
    File(PathBuf),
    /// The text of the result is reported with the end of the job (the
    /// clipboard, the preview).
    Memory,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportRequest {
    pub source: ExportSource,
    pub options: ExportOptions,
    pub target: ExportTarget,
    /// Stop after this many rows (the preview).
    pub limit: Option<u64>,
}

/// A finished export.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportDone {
    pub rows: u64,
    pub bytes: u64,
    /// The file written.
    pub path: Option<PathBuf>,
    /// The text, for [`ExportTarget::Memory`].
    pub text: Option<String>,
    #[serde(skip)]
    pub elapsed: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExportOutcome {
    Done(ExportDone),
    /// Stopped by [`DataSourceHandle::cancel_job`]; no file was left.
    Cancelled,
    Failed(Error),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExportEvent {
    Progress { job: JobId, rows: u64, bytes: u64 },
    Finished { job: JobId, outcome: ExportOutcome },
}

/// Receives an export's events on a thread of the database runtime; it must
/// return quickly.
pub type ExportSink = Arc<dyn Fn(ExportEvent) + Send + Sync>;

impl DataSourceHandle {
    /// Exports `request` on a data connection; progress and the end go to
    /// `sink`. [`DataSourceHandle::cancel_job`] stops it.
    pub fn export(&self, request: ExportRequest, sink: ExportSink) -> JobId {
        let (job, rx) = self.shared.new_job();
        let shared = self.shared.clone();
        self.shared.runtime.spawn(async move {
            let inner = shared.clone();
            let progress = sink.clone();
            let task =
                tokio::spawn(async move { inner.run_export(job, request, progress, rx).await });
            let outcome = match task.await {
                Ok(outcome) => outcome,
                Err(e) => {
                    ExportOutcome::Failed(Error::internal(format!("the export stopped: {e}")))
                }
            };
            shared.end_job(job);
            sink(ExportEvent::Finished { job, outcome });
        });
        job
    }
}

/// The destination while writing: a buffered file or memory, counting bytes.
enum Dest {
    File(BufWriter<File>),
    Memory(Vec<u8>),
}

struct Out {
    dest: Dest,
    bytes: u64,
}

impl Write for Out {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = match &mut self.dest {
            Dest::File(f) => f.write(buf)?,
            Dest::Memory(m) => m.write(buf)?,
        };
        self.bytes += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.dest {
            Dest::File(f) => f.flush(),
            Dest::Memory(_) => Ok(()),
        }
    }
}

/// The writer with its destination, handed to a blocking thread and back.
struct Sink {
    writer: Box<dyn RowWriter>,
    out: Out,
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.part", std::process::id()))
}

fn io_error(what: &str, e: io::Error) -> Error {
    Error::internal(format!("{what}: {e}"))
}

impl Shared {
    async fn run_export(
        self: &Arc<Self>,
        job: JobId,
        request: ExportRequest,
        progress: ExportSink,
        mut rx: watch::Receiver<bool>,
    ) -> ExportOutcome {
        let started = Instant::now();
        self.touch();
        let mut lease = tokio::select! {
            lease = self.lease_data() => match lease {
                Ok(l) => l,
                Err(e) => return ExportOutcome::Failed(e),
            },
            _ = cancelled(&mut rx) => return ExportOutcome::Cancelled,
        };
        let conn = lease.conn().clone();
        let dialect = conn.dialect();
        let mut options = request.options.clone();
        options.dialect = dialect;
        let sql = match &request.source {
            ExportSource::Table(query) => {
                if options.table.is_none() {
                    options.table = Some(query.table.qualified(dialect));
                }
                query.page_sql(dialect, 0, PageSize::All)
            }
            ExportSource::Sql(sql) => sql.clone(),
        };
        let exec = ExecOptions {
            fetch_size: FETCH,
            max_value_bytes: MAX_VALUE,
            ..self.exec_options()
        };
        let grace = self.options.cancel_grace;
        let run = conn.execute(&sql, &[], &exec);
        let mut cursor = match guarded(&conn, run, Some(&mut rx), grace).await {
            Guarded::Done(Ok(Outcome::Rows(cursor))) => cursor,
            Guarded::Done(Ok(Outcome::Affected(_))) => {
                return ExportOutcome::Failed(Error::query("the statement returns no rows"));
            }
            Guarded::Done(Err(e)) => {
                if e.kind == ErrorKind::ConnectionLost {
                    lease.broken();
                }
                return if e.kind == ErrorKind::Cancelled {
                    ExportOutcome::Cancelled
                } else {
                    ExportOutcome::Failed(e)
                };
            }
            Guarded::Abandoned => {
                lease.broken();
                return ExportOutcome::Cancelled;
            }
        };
        let columns = cursor.columns().to_vec();

        // The destination.
        let temp = match &request.target {
            ExportTarget::File(path) => Some((path.clone(), temp_path(path))),
            ExportTarget::Memory => None,
        };
        let dest = match &temp {
            Some((path, temp)) => match File::create(temp) {
                Ok(f) => Dest::File(BufWriter::with_capacity(1 << 16, f)),
                Err(e) => {
                    return ExportOutcome::Failed(io_error(
                        &format!("cannot write {}", path.display()),
                        e,
                    ));
                }
            },
            None => Dest::Memory(Vec::new()),
        };
        let remove_temp = || {
            if let Some((_, temp)) = &temp {
                let _ = std::fs::remove_file(temp);
            }
        };
        let mut writer = writers::writer(&options, &columns);
        let mut out = Out { dest, bytes: 0 };
        if let Err(e) = writer.begin(&mut out) {
            remove_temp();
            return ExportOutcome::Failed(io_error("cannot write", e));
        }
        let mut sink = Some(Sink { writer, out });

        let mut rows: u64 = 0;
        let mut last_progress = Instant::now();
        let outcome = loop {
            if *rx.borrow() {
                break ExportOutcome::Cancelled;
            }
            let want = match request.limit {
                Some(limit) => {
                    let left = limit.saturating_sub(rows);
                    if left == 0 {
                        break ExportOutcome::Done(ExportDone {
                            rows,
                            bytes: 0,
                            path: None,
                            text: None,
                            elapsed: Duration::ZERO,
                        });
                    }
                    FETCH.min(usize::try_from(left).unwrap_or(FETCH))
                }
                None => FETCH,
            };
            let page: Vec<Row> =
                match guarded(&conn, cursor.next_page(want), Some(&mut rx), grace).await {
                    Guarded::Done(Ok(page)) => page,
                    Guarded::Done(Err(e)) => {
                        if e.kind == ErrorKind::ConnectionLost {
                            lease.broken();
                        }
                        break if e.kind == ErrorKind::Cancelled {
                            ExportOutcome::Cancelled
                        } else {
                            ExportOutcome::Failed(e)
                        };
                    }
                    Guarded::Abandoned => {
                        lease.broken();
                        break ExportOutcome::Cancelled;
                    }
                };
            if page.is_empty() {
                break ExportOutcome::Done(ExportDone {
                    rows,
                    bytes: 0,
                    path: None,
                    text: None,
                    elapsed: Duration::ZERO,
                });
            }
            rows += page.len() as u64;
            let mut s = sink.take().expect("the sink is back after every page");
            let written = tokio::task::spawn_blocking(move || {
                for row in &page {
                    if let Err(e) = s.writer.row(&mut s.out, row) {
                        return (s, Err(e));
                    }
                }
                (s, Ok(()))
            })
            .await;
            match written {
                Ok((s, Ok(()))) => sink = Some(s),
                Ok((_, Err(e))) => {
                    break ExportOutcome::Failed(io_error("cannot write", e));
                }
                Err(e) => {
                    break ExportOutcome::Failed(Error::internal(format!(
                        "the writer stopped: {e}"
                    )));
                }
            }
            if last_progress.elapsed() >= PROGRESS_EVERY {
                last_progress = Instant::now();
                let bytes = sink.as_ref().map_or(0, |s| s.out.bytes);
                progress(ExportEvent::Progress { job, rows, bytes });
            }
        };
        // Closing a cursor that did not end stops the server's work.
        let _ = tokio::time::timeout(grace, cursor.close()).await;

        let ExportOutcome::Done(mut done) = outcome else {
            remove_temp();
            return outcome;
        };
        let Some(Sink {
            mut writer,
            mut out,
        }) = sink
        else {
            remove_temp();
            return ExportOutcome::Failed(Error::internal("the writer is gone"));
        };
        let finished = tokio::task::spawn_blocking(move || {
            let r = writer.finish(&mut out).and_then(|()| out.flush());
            (out, r)
        })
        .await;
        let (out, result) = match finished {
            Ok(v) => v,
            Err(e) => {
                remove_temp();
                return ExportOutcome::Failed(Error::internal(format!("the writer stopped: {e}")));
            }
        };
        if let Err(e) = result {
            remove_temp();
            return ExportOutcome::Failed(io_error("cannot write", e));
        }
        done.bytes = out.bytes;
        done.elapsed = started.elapsed();
        match (out.dest, temp) {
            (Dest::File(file), Some((path, temp))) => {
                drop(file);
                if let Err(e) = std::fs::rename(&temp, &path) {
                    let _ = std::fs::remove_file(&temp);
                    return ExportOutcome::Failed(io_error(
                        &format!("cannot write {}", path.display()),
                        e,
                    ));
                }
                done.path = Some(path);
            }
            (Dest::Memory(bytes), _) => {
                done.text = Some(String::from_utf8_lossy(&bytes).into_owned());
            }
            _ => {}
        }
        ExportOutcome::Done(done)
    }
}
