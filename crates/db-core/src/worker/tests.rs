//! The worker protocol against a scripted session.

use std::sync::atomic::AtomicUsize;
use std::time::Instant;

use super::*;
use crate::{ValueKind, collect};

/// What happened to the scripted session, observable from the test.
#[derive(Default)]
struct Script {
    log: Mutex<Vec<String>>,
    break_requested: AtomicBool,
    breaks: AtomicUsize,
    closed: AtomicBool,
}

impl Script {
    fn log(&self, s: impl Into<String>) {
        self.log.lock().unwrap().push(s.into());
    }

    fn entries(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn saw(&self, entry: &str) -> bool {
        self.entries().iter().any(|e| e == entry)
    }

    /// Wait until the interrupt is used; `false` after `limit`.
    fn wait_for_break(&self, limit: Duration) -> bool {
        let started = Instant::now();
        while !self.break_requested.swap(false, Ordering::SeqCst) {
            if started.elapsed() > limit {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        true
    }
}

struct MockInterrupt(Arc<Script>);

impl Interrupt for MockInterrupt {
    fn interrupt(&self) -> Result<()> {
        self.0.breaks.fetch_add(1, Ordering::SeqCst);
        self.0.break_requested.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// An open cursor of the scripted session; logs `close_cursor` when it is
/// dropped before its end.
struct MockCursor {
    script: Arc<Script>,
    next: i64,
    end: i64,
    fail_at: Option<i64>,
    block_at: Option<i64>,
    slow: bool,
}

impl MockCursor {
    fn fetch(&mut self, args: &FetchArgs) -> Result<Page> {
        if self.slow {
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut rows = Vec::new();
        while rows.len() < args.max_rows && self.next < self.end {
            if self.fail_at == Some(self.next) {
                self.script.log("fetch failed");
                return Err(Error::query("boom"));
            }
            if self.block_at == Some(self.next) {
                self.block_at = None;
                if self.script.wait_for_break(Duration::from_secs(10)) {
                    return Err(Error::cancelled());
                }
            }
            rows.push(vec![Value::Int(self.next)]);
            self.next += 1;
        }
        self.script.log(format!("fetch {}", rows.len()));
        Ok(Page {
            rows,
            exhausted: self.next >= self.end,
        })
    }
}

impl Drop for MockCursor {
    fn drop(&mut self) {
        if self.next < self.end {
            self.script.log("close_cursor");
        }
    }
}

/// Statements understood by the script:
/// - `rows N [fail_at K] [block_at K] [slow] [slow_open]`: a cursor of N rows
///   `[Int(i)]`; it fails at row K, or waits for an interrupt at row K;
///   `slow` pages take 100 ms, `slow_open` the statement itself;
/// - `affected`: a statement that changed 3 rows;
/// - `block`: runs until the interrupt is used, then is cancelled;
/// - `lost`: fails like a dead session;
/// - anything else: a query error.
struct MockSession {
    script: Arc<Script>,
}

impl Session for MockSession {
    fn execute(&self, args: ExecArgs, rows: Rows<'_, Self>) -> Result<Executed> {
        self.script.log(format!("execute {}", args.sql));
        if let Some(spec) = args.sql.strip_prefix("rows ") {
            let words: Vec<&str> = spec.split_whitespace().collect();
            let after = |key: &str| {
                words
                    .iter()
                    .position(|w| *w == key)
                    .map(|i| words[i + 1].parse().unwrap())
            };
            if words.contains(&"slow_open") {
                std::thread::sleep(Duration::from_millis(100));
            }
            let mut cursor = MockCursor {
                script: self.script.clone(),
                next: 0,
                end: words[0].parse().unwrap(),
                fail_at: after("fail_at"),
                block_at: after("block_at"),
                slow: words.contains(&"slow"),
            };
            let column = Column {
                name: "N".into(),
                type_name: "NUMBER".into(),
                kind: ValueKind::Int,
                nullable: false,
            };
            let empty = cursor.end == 0;
            return Ok(rows.serve(vec![column], empty, |f| cursor.fetch(f)));
        }
        match args.sql.as_str() {
            "affected" => Ok(Executed::affected(3)),
            "block" => {
                self.script.wait_for_break(Duration::from_secs(10));
                Err(Error::cancelled())
            }
            "lost" => Err(Error::connection_lost("not connected")),
            other => Err(Error::query(format!("bad statement {other}"))),
        }
    }

    fn ping(&self) -> Result<()> {
        self.script.log("ping");
        Ok(())
    }

    fn server_version(&self) -> Result<String> {
        Ok("Mock 1.0".into())
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

fn config(on_abandon: OnAbandon) -> WorkerConfig {
    WorkerConfig {
        thread_name: "db-core-test".into(),
        on_abandon,
        session_enforces_timeouts: false,
    }
}

async fn worker_with(on_abandon: OnAbandon) -> (Worker, Arc<Script>) {
    let script = Arc::new(Script::default());
    let session = MockSession {
        script: script.clone(),
    };
    let worker = Worker::start(
        config(on_abandon),
        Arc::new(MockInterrupt(script.clone())),
        move || Ok(session),
    )
    .await
    .unwrap();
    (worker, script)
}

async fn worker() -> (Worker, Arc<Script>) {
    worker_with(OnAbandon::BreakSession).await
}

fn opts() -> ExecOptions {
    ExecOptions::default()
}

async fn run(w: &Worker, sql: &str) -> Result<Outcome> {
    w.execute(sql, &[], &opts()).await
}

async fn cursor(w: &Worker, sql: &str) -> Box<dyn Cursor> {
    match run(w, sql).await.unwrap() {
        Outcome::Rows(c) => c,
        other => panic!("rows expected, got {other:?}"),
    }
}

fn ints(rows: &[Row]) -> Vec<i64> {
    rows.iter().map(|r| r[0].as_i64().unwrap()).collect()
}

async fn wait_until(mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "timed out waiting"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn gate() -> (Gate, Arc<Script>) {
    let script = Arc::new(Script::default());
    (Gate::new(Arc::new(MockInterrupt(script.clone()))), script)
}

#[test]
fn gate_cancel_with_nothing_running_does_nothing() {
    let (gate, script) = gate();
    gate.cancel().unwrap();
    assert_eq!(script.breaks.load(Ordering::SeqCst), 0);
    // Nothing was outstanding, so the next request runs.
    let id = gate.next_id();
    assert!(matches!(gate.start(Some(id)), Start::Run));
    gate.end();
}

#[test]
fn gate_interrupts_only_inside_a_call() {
    let (gate, script) = gate();
    let id = gate.next_id();
    assert!(matches!(gate.start(Some(id)), Start::Run));
    gate.cancel().unwrap();
    assert_eq!(script.breaks.load(Ordering::SeqCst), 1);
    gate.end();
    // The call is over: another cancel interrupts nothing.
    gate.cancel().unwrap();
    assert_eq!(script.breaks.load(Ordering::SeqCst), 1);
}

#[test]
fn gate_cancels_the_requests_sent_before_it() {
    let (gate, script) = gate();
    let first = gate.next_id();
    let second = gate.next_id();
    gate.cancel().unwrap();
    assert_eq!(script.breaks.load(Ordering::SeqCst), 0, "nothing runs yet");
    assert!(matches!(gate.start(Some(first)), Start::Cancelled));
    assert!(matches!(gate.start(Some(second)), Start::Cancelled));
    // Calls without a number are never cancelled before they start.
    assert!(matches!(gate.start(None), Start::Run));
    gate.end();
    // Requests sent after the cancel are not affected.
    let third = gate.next_id();
    assert!(matches!(gate.start(Some(third)), Start::Run));
    gate.end();
}

#[test]
fn gate_usability() {
    let (gate, _) = gate();
    assert!(gate.check_usable().is_ok());
    gate.mark_broken();
    assert_eq!(
        gate.check_usable().unwrap_err().kind,
        ErrorKind::ConnectionLost
    );
    assert!(matches!(gate.start(None), Start::Broken));
    let (gate, _) = self::gate();
    gate.mark_closed();
    assert_eq!(
        gate.check_usable().unwrap_err().kind,
        ErrorKind::ConnectionLost
    );
    assert!(gate.is_unusable());
}

#[test]
fn watchdog_depends_on_session_timeouts() {
    let s = Duration::from_secs;
    let mut config = config(OnAbandon::Cancel);
    assert_eq!(config.watchdog(None), None);
    assert_eq!(config.watchdog(Some(s(10))), Some(s(10)));
    config.session_enforces_timeouts = true;
    assert_eq!(config.watchdog(None), None);
    assert_eq!(config.watchdog(Some(s(10))), Some(s(22)));
}

#[tokio::test]
async fn a_session_that_cannot_open_reports_why() {
    let script = Arc::new(Script::default());
    let r = Worker::start::<MockSession, _>(
        config(OnAbandon::Cancel),
        Arc::new(MockInterrupt(script)),
        || Err(Error::config("no such database")),
    )
    .await;
    assert_eq!(r.err().unwrap().kind, ErrorKind::Config);
}

#[tokio::test]
async fn requests_run_in_order_and_answer() {
    let (w, script) = worker().await;
    assert!(matches!(
        run(&w, "affected").await.unwrap(),
        Outcome::Affected(3)
    ));
    w.ping().await.unwrap();
    w.commit().await.unwrap();
    w.rollback().await.unwrap();
    assert_eq!(w.server_version().await.unwrap(), "Mock 1.0");
    assert_eq!(
        script.entries(),
        ["execute affected", "ping", "commit", "rollback"]
    );
}

#[tokio::test]
async fn pages_continue_where_the_last_page_ended() {
    let (w, script) = worker().await;
    let mut c = cursor(&w, "rows 1000").await;
    assert_eq!(c.columns()[0].name, "N");
    let mut seen = Vec::new();
    for want in [1usize, 7, 300, 300, 300, 300] {
        let page = c.next_page(want).await.unwrap();
        assert!(page.len() <= want);
        seen.extend(ints(&page));
        if c.is_exhausted() {
            break;
        }
    }
    assert_eq!(seen, (0..1000).collect::<Vec<_>>());
    assert!(c.is_exhausted());
    assert!(c.next_page(10).await.unwrap().is_empty());
    // The connection is free once the end was read, before the drop.
    assert!(run(&w, "affected").await.is_ok());
    drop(c);
    // The thread saw the end and needed no close.
    assert!(!script.saw("close_cursor"));
}

#[tokio::test]
async fn an_empty_result_is_exhausted_at_once() {
    let (w, _) = worker().await;
    let mut c = cursor(&w, "rows 0").await;
    assert!(c.is_exhausted());
    assert!(c.next_page(10).await.unwrap().is_empty());
    // The cursor is still alive, but holds nothing.
    assert!(run(&w, "affected").await.is_ok());
    drop(c);
}

#[tokio::test]
async fn one_cursor_per_connection() {
    let (w, script) = worker().await;
    let mut c = cursor(&w, "rows 100").await;
    assert_eq!(c.next_page(10).await.unwrap().len(), 10);
    for sql in ["affected", "rows 5"] {
        assert_eq!(run(&w, sql).await.unwrap_err().kind, ErrorKind::Busy);
    }
    // Calls and cancel still work while a cursor is open...
    w.ping().await.unwrap();
    w.commit().await.unwrap();
    w.rollback().await.unwrap();
    w.cancel().await.unwrap();
    assert_eq!(script.breaks.load(Ordering::SeqCst), 0, "nothing ran");
    // ...and the cursor goes on where it was.
    assert_eq!(ints(&c.next_page(3).await.unwrap()), [10, 11, 12]);
    c.close().await.unwrap();
    assert!(script.saw("close_cursor"));
    assert!(run(&w, "affected").await.is_ok());
}

#[tokio::test]
async fn early_close_stops_reading() {
    let (w, script) = worker().await;
    let mut c = cursor(&w, "rows 1000").await;
    assert_eq!(c.next_page(100).await.unwrap().len(), 100);
    assert!(!c.is_exhausted());
    c.close().await.unwrap();
    assert!(script.saw("close_cursor"));
    assert!(run(&w, "affected").await.is_ok());
    assert!(!script.saw("fetch 900"), "nothing read the rest");
}

#[tokio::test]
async fn a_dropped_cursor_is_closed_before_the_next_statement() {
    let (w, script) = worker().await;
    drop(cursor(&w, "rows 1000").await);
    run(&w, "affected").await.unwrap();
    let log = script.entries();
    let close = log.iter().position(|e| e == "close_cursor").unwrap();
    let next = log.iter().position(|e| e == "execute affected").unwrap();
    assert!(close < next, "{log:?}");
}

#[tokio::test]
async fn collect_reads_up_to_the_cap() {
    let (w, script) = worker().await;
    let rs = collect(cursor(&w, "rows 1000").await, 250, 100)
        .await
        .unwrap();
    assert_eq!(rs.rows.len(), 250);
    assert!(rs.truncated);
    assert!(script.saw("close_cursor"));
    let rs = collect(cursor(&w, "rows 5").await, 100, 100).await.unwrap();
    assert_eq!(rs.rows.len(), 5);
    assert!(!rs.truncated);
}

#[tokio::test]
async fn statement_errors_leave_the_connection_usable() {
    let (w, _) = worker().await;
    let e = run(&w, "nonsense").await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert!(run(&w, "affected").await.is_ok());
}

#[tokio::test]
async fn a_failed_page_ends_only_the_cursor() {
    let (w, _) = worker().await;
    let mut c = cursor(&w, "rows 10 fail_at 3").await;
    let e = c.next_page(100).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert!(c.is_exhausted());
    assert_eq!(
        c.next_page(100).await.unwrap_err(),
        e,
        "the same error again"
    );
    drop(c);
    assert!(run(&w, "affected").await.is_ok());
}

#[tokio::test]
async fn cancel_interrupts_a_running_statement() {
    let (w, script) = worker().await;
    let w = Arc::new(w);
    let runner = {
        let w = w.clone();
        tokio::spawn(async move { w.execute("block", &[], &opts()).await.map(|_| ()) })
    };
    wait_until(|| script.saw("execute block")).await;
    w.cancel().await.unwrap();
    assert_eq!(
        runner.await.unwrap().unwrap_err().kind,
        ErrorKind::Cancelled
    );
    assert_eq!(script.breaks.load(Ordering::SeqCst), 1);
    assert!(run(&w, "affected").await.is_ok());
}

#[tokio::test]
async fn cancel_covers_a_statement_that_has_not_started() {
    let (w, _) = worker().await;
    // The statement is sent when the first future is polled, the cancel
    // right after: whether or not the thread has picked it up, it ends.
    let (r, ()) = tokio::join!(run(&w, "block"), async { w.cancel().await.unwrap() });
    assert_eq!(r.unwrap_err().kind, ErrorKind::Cancelled);
    assert!(run(&w, "affected").await.is_ok());
}

#[tokio::test]
async fn cancel_while_idle_is_a_no_op() {
    let (w, script) = worker().await;
    for _ in 0..3 {
        w.cancel().await.unwrap();
    }
    assert_eq!(script.breaks.load(Ordering::SeqCst), 0);
    assert!(run(&w, "affected").await.is_ok(), "not hit by the cancel");
    // An idle open cursor is not affected either.
    let mut c = cursor(&w, "rows 10").await;
    w.cancel().await.unwrap();
    assert_eq!(c.next_page(5).await.unwrap().len(), 5);
}

#[tokio::test]
async fn cancel_ends_a_slow_page_and_the_cursor() {
    let (w, script) = worker().await;
    let w = Arc::new(w);
    let mut c = cursor(&w, "rows 10 block_at 5").await;
    let canceller = {
        let w = w.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            w.cancel().await.unwrap();
        })
    };
    let e = c.next_page(10).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled);
    canceller.await.unwrap();
    assert_eq!(script.breaks.load(Ordering::SeqCst), 1);
    assert!(c.is_exhausted());
    assert!(c.next_page(10).await.is_err());
    drop(c);
    assert!(run(&w, "affected").await.is_ok());
}

#[tokio::test]
async fn timeout_cancels_and_reports_timeout() {
    let (w, script) = worker().await;
    let opts = ExecOptions {
        timeout: Some(Duration::from_millis(100)),
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let e = w.execute("block", &[], &opts).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Timeout);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(script.breaks.load(Ordering::SeqCst), 1);
    // The statement ended with the interrupt; the connection works.
    assert!(run(&w, "affected").await.is_ok());
}

#[tokio::test]
async fn page_timeout_reports_timeout() {
    let (w, _) = worker().await;
    let opts = ExecOptions {
        timeout: Some(Duration::from_millis(100)),
        ..ExecOptions::default()
    };
    let Outcome::Rows(mut c) = w.execute("rows 10 block_at 5", &[], &opts).await.unwrap() else {
        panic!("rows expected");
    };
    assert_eq!(c.next_page(10).await.unwrap_err().kind, ErrorKind::Timeout);
    drop(c);
    assert!(run(&w, "affected").await.is_ok());
}

#[tokio::test]
async fn break_session_a_dropped_call_marks_the_connection_broken() {
    let (w, script) = worker_with(OnAbandon::BreakSession).await;
    let r = tokio::time::timeout(Duration::from_millis(100), run(&w, "block")).await;
    assert!(r.is_err(), "the future is dropped");
    // The running statement is interrupted and the session closed.
    wait_until(|| script.closed.load(Ordering::SeqCst)).await;
    assert!(script.breaks.load(Ordering::SeqCst) >= 1);
    assert!(w.is_unusable());
    assert_eq!(
        run(&w, "affected").await.unwrap_err().kind,
        ErrorKind::ConnectionLost
    );
    assert_eq!(w.ping().await.unwrap_err().kind, ErrorKind::ConnectionLost);
    // Cancel and close on a broken connection are harmless.
    w.cancel().await.unwrap();
    w.close().await.unwrap();
}

#[tokio::test]
async fn cancel_policy_a_dropped_call_is_cancelled() {
    let (w, script) = worker_with(OnAbandon::Cancel).await;
    let r = tokio::time::timeout(Duration::from_millis(100), run(&w, "block")).await;
    assert!(r.is_err(), "the future is dropped");
    assert!(run(&w, "affected").await.is_ok());
    assert_eq!(script.breaks.load(Ordering::SeqCst), 1);
    assert!(!script.closed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancel_policy_an_unanswered_cursor_is_released() {
    let (w, script) = worker_with(OnAbandon::Cancel).await;
    // Given up while the statement opens its cursor: nobody receives it.
    let r = tokio::time::timeout(Duration::from_millis(20), run(&w, "rows 10 slow_open")).await;
    assert!(r.is_err());
    assert!(run(&w, "affected").await.is_ok());
    let log = script.entries();
    let close = log.iter().position(|e| e == "close_cursor").unwrap();
    let next = log.iter().position(|e| e == "execute affected").unwrap();
    assert!(close < next, "{log:?}");
}

#[tokio::test]
async fn cancel_policy_rows_of_an_abandoned_page_come_with_the_next() {
    let (w, _) = worker_with(OnAbandon::Cancel).await;
    let mut c = cursor(&w, "rows 10 slow").await;
    let r = tokio::time::timeout(Duration::from_millis(20), c.next_page(3)).await;
    assert!(r.is_err(), "the page request is dropped");
    let mut seen = ints(&c.next_page(5).await.unwrap());
    while !c.is_exhausted() {
        seen.extend(ints(&c.next_page(5).await.unwrap()));
    }
    assert_eq!(seen, (0..10).collect::<Vec<_>>());
}

#[tokio::test]
async fn a_lost_session_marks_the_connection_broken() {
    let (w, script) = worker().await;
    assert_eq!(
        run(&w, "lost").await.unwrap_err().kind,
        ErrorKind::ConnectionLost
    );
    wait_until(|| script.closed.load(Ordering::SeqCst)).await;
    assert_eq!(
        run(&w, "affected").await.unwrap_err().kind,
        ErrorKind::ConnectionLost
    );
}

#[tokio::test]
async fn close_ends_the_session_and_later_calls_fail() {
    let (w, script) = worker().await;
    w.close().await.unwrap();
    assert!(script.closed.load(Ordering::SeqCst));
    w.close().await.unwrap();
    assert_eq!(
        run(&w, "affected").await.unwrap_err().kind,
        ErrorKind::ConnectionLost
    );
    w.cancel().await.unwrap();
}

#[tokio::test]
async fn closing_the_connection_closes_an_open_cursor_first() {
    let (w, script) = worker().await;
    let mut c = cursor(&w, "rows 10").await;
    w.close().await.unwrap();
    let log = script.entries();
    assert_eq!(log[log.len() - 2..], ["close_cursor", "close"]);
    assert_eq!(
        c.next_page(1).await.unwrap_err().kind,
        ErrorKind::ConnectionLost
    );
    c.close().await.unwrap();
}

#[tokio::test]
async fn dropping_the_worker_closes_the_session() {
    let (w, script) = worker().await;
    let c = cursor(&w, "rows 10").await;
    drop(c);
    drop(w);
    wait_until(|| script.closed.load(Ordering::SeqCst)).await;
}

#[tokio::test]
async fn tx_mode_is_tracked() {
    let (w, script) = worker().await;
    assert_eq!(w.tx_mode(), TxMode::Auto);
    w.set_tx_mode(TxMode::Manual).await.unwrap();
    assert_eq!(w.tx_mode(), TxMode::Manual);
    w.set_tx_mode(TxMode::Auto).await.unwrap();
    assert_eq!(w.tx_mode(), TxMode::Auto);
    assert_eq!(script.entries(), ["tx Manual", "tx Auto"]);
}
