//! Headless commands of the session layer (ADR 0001): the same jobs the
//! console runs, for the CLI and the agent control channel.

use std::sync::Arc;
use std::time::Duration;

use commands::{CommandError, CommandMeta, Registry, typed};
use db_core::{ConnectionSource, DataSourceId, TxMode};
use serde::{Deserialize, Serialize};

use crate::{
    DataSourceOptions, DbRuntime, ErrorPolicy, EventKind, EventSink, Job, JobEvent, SessionEvent,
    SessionOptions, TxPolicy, TxStatus,
};

/// The options of a data source by id (its profile's GUI options).
pub type OptionsLookup = Arc<dyn Fn(&DataSourceId) -> DataSourceOptions + Send + Sync>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    data_source: String,
    sql: String,
    #[serde(default)]
    tx_mode: Option<TxMode>,
    /// Manual mode: commit at the end instead of rolling back.
    #[serde(default)]
    commit: bool,
    #[serde(default)]
    continue_on_error: bool,
    #[serde(default)]
    page_size: Option<usize>,
}

#[derive(Serialize)]
struct RunOutput {
    events: Vec<JobEvent>,
    transaction: TxStatus,
}

/// Longest a headless job may take before the command gives up on it.
const RUN_LIMIT: Duration = Duration::from_secs(3600);

/// Register `db.session.run`: run a script in a new session of a data source
/// and return the job's events (the first page of every result).
pub fn register(
    registry: &mut Registry,
    runtime: DbRuntime,
    source: Arc<dyn ConnectionSource>,
    options: OptionsLookup,
) {
    registry.register(typed(
        CommandMeta {
            id: "db.session.run",
            title: "Run SQL Script in a Session",
            summary: "Runs a script statement by statement in a new console session and returns the job's events; manual mode rolls back unless commit is set.",
            args_example: r#"{"data_source": "local-pg", "sql": "select 1; select 2", "tx_mode": "auto"}"#,
        },
        move |args: RunArgs| {
            let runtime = runtime.clone();
            let source = source.clone();
            let options = options.clone();
            async move { run(&runtime, source, &options, args).await }
        },
    ));
}

async fn run(
    runtime: &DbRuntime,
    source: Arc<dyn ConnectionSource>,
    options: &OptionsLookup,
    args: RunArgs,
) -> Result<RunOutput, CommandError> {
    let id = DataSourceId::new(args.data_source);
    let mut opts = options(&id);
    if let Some(page) = args.page_size {
        opts.page_size = page.max(1);
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = EventSink::new(move |e| {
        let _ = tx.send(e);
    });
    let handle = runtime.data_source(id, opts, source, sink);
    let session = handle.open_session(SessionOptions {
        tx_mode: args.tx_mode,
        schema: None,
    });
    let policy = if args.continue_on_error {
        ErrorPolicy::Continue
    } else {
        ErrorPolicy::Stop
    };
    let job = session.execute(Job::script(args.sql).on_error(policy));
    let mut events = Vec::new();
    let collect = async {
        while let Some(e) = rx.recv().await {
            if let EventKind::Job { job: j, event, .. } = e.kind
                && j == job
            {
                let done = matches!(event, JobEvent::Done { .. });
                events.push(event);
                if done {
                    break;
                }
            }
        }
    };
    if tokio::time::timeout(RUN_LIMIT, collect).await.is_err() {
        session.cancel();
        return Err(CommandError::new("db.timeout", "the job did not finish"));
    }
    let transaction = session.status().tx;
    session.close(if args.commit {
        TxPolicy::Commit
    } else {
        TxPolicy::Rollback
    });
    let closed = async {
        while let Some(e) = rx.recv().await {
            if let EventKind::Session {
                event: SessionEvent::Closed,
                ..
            } = e.kind
            {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(30), closed).await;
    Ok(RunOutput {
        events,
        transaction,
    })
}
