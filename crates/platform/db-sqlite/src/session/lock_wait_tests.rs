//! Lock waits woken by releases ([`super::LockReleases`]). These tests set
//! the poll step of every lock wait in the process, so no other unit test of
//! this crate may wait for a lock.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use db_core::{ConnectParams, Connection, Driver, ExecOptions, opt};

use super::LOCK_RELEASES;
use crate::SqliteDriver;

/// Poll step of every lock wait while set, in milliseconds; 0 keeps the
/// normal steps.
static POLL_STEP_MS: AtomicU64 = AtomicU64::new(0);

pub(super) fn poll_step() -> Option<Duration> {
    match POLL_STEP_MS.load(Ordering::SeqCst) {
        0 => None,
        ms => Some(Duration::from_millis(ms)),
    }
}

/// Sets the poll step of every lock wait until dropped.
struct PollStep;

impl PollStep {
    fn set(step: Duration) -> PollStep {
        let ms = u64::try_from(step.as_millis()).unwrap_or(u64::MAX).max(1);
        POLL_STEP_MS.store(ms, Ordering::SeqCst);
        PollStep
    }
}

impl Drop for PollStep {
    fn drop(&mut self) {
        POLL_STEP_MS.store(0, Ordering::SeqCst);
    }
}

/// Far longer than any test runs: a lock wait with this poll step ends only
/// through a release.
const NEVER: Duration = Duration::from_secs(60);
/// Bound on waiting for something that normally takes milliseconds.
const LIMIT: Duration = Duration::from_secs(20);

async fn open(path: &std::path::Path, create: bool) -> Box<dyn Connection> {
    let mut params = ConnectParams {
        database: path.to_string_lossy().into_owned(),
        ..ConnectParams::default()
    };
    if create {
        params
            .options
            .insert(opt::SQLITE_CREATE.into(), "true".into());
    }
    SqliteDriver.connect(&params).await.expect("connect")
}

async fn run(conn: &dyn Connection, sql: &str) {
    conn.execute(sql, &[], &ExecOptions::default())
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn count(conn: &dyn Connection) -> i64 {
    let result = conn
        .query("SELECT count(*) FROM t", &[], usize::MAX)
        .await
        .expect("count");
    result.rows[0][0].as_i64().expect("an integer")
}

/// Lock waits in progress in this process.
fn waiting() -> usize {
    LOCK_RELEASES.state().waiting
}

#[tokio::test]
async fn a_waiting_reader_is_woken_by_the_release() {
    // A writer that commits back to back frees the file only for the round
    // trip to its next statement. A reader waiting for the lock gets into
    // that gap only when the release wakes it at once, not at its next poll.
    // Here the reader's poll would come after a minute, so it can only get
    // in through the release, and the writer commits only once the reader
    // waits: neither the scheduler nor the clock decides the outcome.
    let _polls = PollStep::set(NEVER);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("turns.db");
    let writer = open(&path, true).await;
    run(&*writer, "CREATE TABLE t(x)").await;
    let reader = open(&path, false).await;
    // The reader loads the schema now, so its next count needs one lock.
    assert_eq!(count(&*reader).await, 0);
    run(&*writer, "BEGIN EXCLUSIVE").await;
    assert_eq!(waiting(), 0);
    let (read, ()) = tokio::join!(tokio::time::timeout(LIMIT, count(&*reader)), async {
        let start = Instant::now();
        while waiting() == 0 {
            assert!(
                start.elapsed() < LIMIT,
                "the reader does not wait for the lock"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        run(&*writer, "COMMIT").await;
    });
    let rows = read.expect("the release did not wake the waiting reader");
    assert_eq!(rows, 0);
    assert_eq!(waiting(), 0);
}
