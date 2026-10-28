//! The data sources opened in the object tree: a data source handle each
//! (`db-session`), its model (loaded from the cache first, then from
//! refreshes) and the refresh that runs. The tree reads the current model
//! of a handle; it never asks the database when a node opens.

use std::sync::Arc;
use std::time::Duration;

use db_core::schema::{Level, Scope};
use db_core::{DataSourceId, Profile};
use db_model::Model;
use db_session::{DataSourceHandle, DataSourceOptions, DataSourceState, DbRuntime, JobId};
use gpui::{App, Global};

use crate::tree::Status;

/// The runtime of the module's data source handles, started with the
/// first one.
struct Runtime(DbRuntime);

impl Global for Runtime {}

pub(crate) fn runtime(cx: &mut App) -> std::io::Result<DbRuntime> {
    if let Some(r) = cx.try_global::<Runtime>() {
        return Ok(r.0.clone());
    }
    let runtime = DbRuntime::new()?;
    cx.set_global(Runtime(runtime.clone()));
    Ok(runtime)
}

/// The session options of a data source's GUI settings.
pub(crate) fn options(profile: &Profile) -> DataSourceOptions {
    let gui = profile.gui_options();
    DataSourceOptions {
        startup_script: gui.startup_script.clone(),
        time_zone: gui.time_zone.clone(),
        default_tx_mode: gui.tx_mode.unwrap_or_default(),
        keep_alive: gui.keep_alive_secs.map(Duration::from_secs),
        auto_disconnect: gui.auto_disconnect_secs.map(Duration::from_secs),
        single_session: gui.single_session,
        ..DataSourceOptions::default()
    }
}

/// What a refresh of the data source reads: its introspection scope (the
/// current schema when none is set) at its level.
pub(crate) fn refresh_scope(profile: &Profile) -> (Scope, Level) {
    let gui = profile.gui_options();
    (
        gui.scope.clone().unwrap_or_else(Scope::current),
        gui.level.unwrap_or(Level::Structure),
    )
}

/// One data source opened in the tree.
pub(crate) struct Live {
    /// The profile the handle was made with (a changed profile gets a new
    /// handle).
    pub profile: Profile,
    pub handle: DataSourceHandle,
    /// The handle's model as last read.
    pub model: Arc<Model>,
    /// The model came from the cache or a refresh (an empty model before
    /// either is not shown).
    pub has_model: bool,
    pub cache_loading: bool,
    pub refresh: Option<JobId>,
    /// Refreshes that run after the current one: the schemas read at a
    /// level of their own (`GuiOptions::schema_levels`).
    pub queued: Vec<(Scope, Level)>,
    pub state: DataSourceState,
    /// Why the last refresh failed.
    pub error: Option<String>,
}

impl Live {
    pub fn new(profile: Profile, handle: DataSourceHandle) -> Self {
        let model = handle.model().get();
        Self {
            profile,
            handle,
            model,
            has_model: false,
            cache_loading: false,
            refresh: None,
            queued: Vec::new(),
            state: DataSourceState::Disconnected,
            error: None,
        }
    }

    pub fn id(&self) -> &DataSourceId {
        self.handle.id()
    }

    /// Starts a refresh of the data source's scope unless one runs; the
    /// schemas with a level of their own are read again at it afterwards.
    pub fn refresh(&mut self) {
        if self.refresh.is_some() {
            return;
        }
        let (scope, level) = refresh_scope(&self.profile);
        self.queued = self
            .profile
            .gui_options()
            .schema_levels
            .into_iter()
            .filter(|(_, l)| *l != level)
            .map(|(schema, l)| (Scope::list([schema]), l))
            .collect();
        self.error = None;
        self.refresh = Some(self.handle.refresh(scope, level));
    }

    /// Reads one schema again at its level, after the refresh that runs.
    pub fn refresh_schema(&mut self, schema: &str) {
        let gui = self.profile.gui_options();
        let level = gui
            .schema_levels
            .get(schema)
            .copied()
            .unwrap_or_else(|| refresh_scope(&self.profile).1);
        let scope = Scope::list([schema.to_owned()]);
        if self.refresh.is_some() {
            if !self.queued.iter().any(|(s, _)| *s == scope) {
                self.queued.push((scope, level));
            }
            return;
        }
        self.error = None;
        self.refresh = Some(self.handle.refresh(scope, level));
    }

    /// Starts the next queued refresh when one is queued; `false` when
    /// none is.
    pub fn refresh_next(&mut self) -> bool {
        if self.queued.is_empty() {
            return false;
        }
        let (scope, level) = self.queued.remove(0);
        self.refresh = Some(self.handle.refresh(scope, level));
        true
    }

    /// The model to show: `None` while there is none or it holds no schema.
    pub fn shown_model(&self) -> Option<&Model> {
        (self.has_model && !self.model.is_empty()).then_some(&*self.model)
    }

    /// The status row shown without a model.
    pub fn status(&self) -> Status {
        if let Some(e) = &self.error {
            return Status::Failed(e.clone());
        }
        if self.refresh.is_some() {
            return match self.state {
                DataSourceState::Connected => Status::Loading,
                _ => Status::Connecting,
            };
        }
        if self.cache_loading {
            return Status::Loading;
        }
        Status::Empty
    }
}
