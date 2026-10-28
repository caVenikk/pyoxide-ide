//! What a metadata refresh reads and how it is merged into the model: the
//! whole scope, only what changed since the model was made, or some objects
//! by name (ADR 0021).

use std::collections::HashSet;

use db_core::schema::{Delta, Introspector, Level, ObjectRef, SchemaDelta, Scope, Snapshot};
use db_core::{ErrorKind, Result};
use db_model::{Cancelled, Model};
use serde::Serialize;

/// How a refresh reads the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshMode {
    /// Read every schema of the scope again.
    Full,
    /// Ask the database what changed since the model was made and read just
    /// that. Falls back to a full read when the model has no change token
    /// or the database cannot compare it (another version, another
    /// dialect).
    Incremental,
}

/// What a finished refresh did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RefreshSummary {
    /// The refresh read the whole scope: asked to, or the database could not
    /// tell what changed.
    pub full: bool,
    /// Schemas in which something was read.
    pub schemas_read: usize,
    /// Objects read (a table counts once, its indexes and triggers too).
    pub objects_read: usize,
    /// Objects removed from the model.
    pub objects_dropped: usize,
    /// Schemas removed from the model.
    pub schemas_dropped: usize,
}

/// What to read.
pub(crate) enum Request {
    Scope {
        scope: Scope,
        level: Level,
        mode: RefreshMode,
    },
    Objects {
        schema: String,
        objects: Vec<ObjectRef>,
        level: Level,
    },
}

/// What was read, before it is merged.
pub(crate) enum Read {
    Snapshot(Snapshot),
    Incremental {
        delta: Delta,
        /// Schemas read whole next to the delta: the ones the token knows
        /// nothing of and the ones the model holds at another level.
        whole: Option<Snapshot>,
    },
    Objects(SchemaDelta),
}

/// Reads on `intro`, which must have a connection of its own. `model` is the
/// model the result will be merged into.
pub(crate) async fn read(
    intro: &dyn Introspector,
    request: Request,
    model: &Model,
) -> Result<Read> {
    match request {
        Request::Scope {
            scope,
            level,
            mode: RefreshMode::Full,
        } => Ok(Read::Snapshot(intro.load(&scope, level).await?)),
        Request::Scope {
            scope,
            level,
            mode: RefreshMode::Incremental,
        } => read_incremental(intro, model, &scope, level).await,
        Request::Objects {
            schema,
            objects,
            level,
        } => Ok(Read::Objects(
            intro.load_objects(&schema, &objects, level).await?,
        )),
    }
}

async fn read_incremental(
    intro: &dyn Introspector,
    model: &Model,
    scope: &Scope,
    level: Level,
) -> Result<Read> {
    let full = async || Ok(Read::Snapshot(intro.load(scope, level).await?));
    let Some(token) = model.token().cloned() else {
        return full().await;
    };
    let mut delta = match intro.changes(&token, scope).await {
        Ok(delta) => delta,
        Err(e) if e.kind == ErrorKind::Unsupported => return full().await,
        Err(e) => return Err(e),
    };

    // Whole: what the token has no stamps of, what the model holds at
    // another level than asked for, and what the token knows and the model
    // does not (the two disagree: trust the database).
    let stamped: HashSet<String> = token.schemas().into_iter().map(|(name, _)| name).collect();
    let mut whole: Vec<String> = std::mem::take(&mut delta.unseen);
    for info in intro.schemas().await? {
        if !scope.selects(&info) || whole.contains(&info.name) {
            continue;
        }
        let stale = match model.schema_arc(&info.name) {
            Some(held) => held.level() != level,
            None => stamped.contains(&info.name),
        };
        if stale {
            whole.push(info.name);
        }
    }
    let whole = if whole.is_empty() {
        None
    } else {
        Some(intro.load(&Scope::list(whole), level).await?)
    };
    Ok(Read::Incremental { delta, whole })
}

/// `model` with `read` merged in; `cancel` is asked while the schemas are
/// rebuilt.
pub(crate) fn merge(
    model: &Model,
    read: Read,
    cancel: &dyn Fn() -> bool,
) -> std::result::Result<(Model, RefreshSummary), Cancelled> {
    match read {
        Read::Snapshot(snapshot) => {
            let summary = RefreshSummary {
                full: true,
                schemas_read: snapshot.schemas.len(),
                objects_read: snapshot.schemas.iter().map(|s| s.objects.len()).sum(),
                ..RefreshSummary::default()
            };
            Ok((model.with_snapshot_cancellable(snapshot, cancel)?, summary))
        }
        Read::Incremental { delta, whole } => {
            // A schema read whole counts once, not as well for the part of
            // the delta that it replaces.
            let whole_names: HashSet<&str> = whole
                .iter()
                .flat_map(|w| w.schemas.iter().map(|s| s.info.name.as_str()))
                .collect();
            let by_delta = || {
                delta
                    .schemas
                    .iter()
                    .filter(|s| !whole_names.contains(s.info.name.as_str()))
            };
            let mut summary = RefreshSummary {
                full: false,
                schemas_read: by_delta().count() + whole_names.len(),
                objects_read: by_delta().map(|s| s.objects.len()).sum::<usize>()
                    + whole
                        .iter()
                        .flat_map(|w| &w.schemas)
                        .map(|s| s.objects.len())
                        .sum::<usize>(),
                objects_dropped: delta.schemas.iter().map(|s| s.dropped.len()).sum(),
                schemas_dropped: delta.dropped_schemas.len(),
            };
            let mut next = model.with_delta_cancellable(delta, cancel)?;
            if let Some(snapshot) = whole {
                next = next.with_snapshot_cancellable(snapshot, cancel)?;
            }
            summary.full = false;
            Ok((next, summary))
        }
        Read::Objects(delta) => {
            let summary = RefreshSummary {
                full: false,
                schemas_read: 1,
                objects_read: delta.objects.len(),
                objects_dropped: delta.dropped.len(),
                schemas_dropped: 0,
            };
            Ok((model.with_schema_delta_cancellable(delta, cancel)?, summary))
        }
    }
}
