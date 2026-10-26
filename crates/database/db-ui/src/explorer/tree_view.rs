//! The object tree of the tool window: the data sources opened in it (a
//! data source handle each, see [`crate::live`]), the expansion, the rows
//! and their drawing. Opening a data source shows its cached model at once
//! and refreshes only when there is none; opening any other node reads the
//! model only.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use db_core::DataSourceId;
use db_model::Model;
use db_session::{DataSourceState, Event, EventKind, EventSink, RefreshEvent};
use gpui::{
    AnyElement, App, ClickEvent, Context, Hsla, IntoElement, MouseButton, Window, div, prelude::*,
    px, uniform_list,
};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::icons::{Icon, IconName};
use ui::notification::{Notification, NotificationKind};
use ui::theme::ThemeExt;

use super::Explorer;
use crate::icons;
use crate::live::{self, Live};
use crate::store::DataSource;
use crate::tree::{self, NodeKey, NodeView, Row, RowIcon, RowKey, SourceTree, Status};

/// Metrics of the rows (logical px), the project tree's: the space left of
/// the first chevron, chevron to icon, icon to text.
const TREE_LEFT: f32 = 12.0;
const ICON_X: f32 = 21.0;
const TEXT_X: f32 = 20.0;
const TREE_TOP: f32 = 4.0;
const CHEVRON: f32 = 16.0;
/// Name to its grey detail or count.
const DETAIL_GAP: f32 = 6.0;

/// Texts of the status rows (our own wording).
pub const CONNECTING: &str = "Connecting\u{2026}";
pub const LOADING: &str = "Loading\u{2026}";
pub const EMPTY: &str = "No schemas in the introspection scope";
pub const RETRY: &str = "Retry";

/// The name of the database row of a data source: PostgreSQL data sources
/// show their database; Oracle and SQLite ones have one and show its
/// schemas directly.
pub(crate) fn database_name(profile: &db_core::Profile) -> Option<&str> {
    match profile.backend {
        db_core::Backend::Postgres => Some(profile.database.as_deref().unwrap_or("postgres")),
        _ => None,
    }
}

/// Two profiles connect alike: they differ at most in what is shown or
/// read (colour, comment, introspection level and scope).
fn same_connection(a: &db_core::Profile, b: &db_core::Profile) -> bool {
    let plain = |p: &db_core::Profile| {
        let mut p = p.clone();
        let mut gui = p.gui_options();
        gui.color = None;
        gui.comment = None;
        gui.level = None;
        gui.schema_levels.clear();
        gui.scope = None;
        gui.refresh_minutes = None;
        p.gui = (!gui.is_default()).then_some(gui);
        p
    };
    plain(a) == plain(b)
}

/// Where the name of a row at `depth` starts, from the rows' left edge.
pub(super) fn text_x(depth: usize, cx: &App) -> gpui::Pixels {
    px(TREE_LEFT + ICON_X + TEXT_X) + cx.theme().metric(Metric::TreeIndent) * depth as f32
}

fn icon_name(icon: RowIcon) -> IconName {
    match icon {
        RowIcon::Source => icons::DATA_SOURCE,
        RowIcon::Database => icons::DATABASE_NODE,
        RowIcon::Schema => icons::SCHEMA,
        RowIcon::Group => icons::FOLDER,
        RowIcon::Table => icons::TABLE,
        RowIcon::View => icons::VIEW,
        RowIcon::MaterializedView => icons::MATERIALIZED_VIEW,
        RowIcon::Sequence => icons::SEQUENCE,
        RowIcon::Routine => icons::ROUTINE,
        RowIcon::Procedure => icons::PROCEDURE,
        RowIcon::Package => icons::PACKAGE,
        RowIcon::Type => icons::OBJECT_TYPE,
        RowIcon::Synonym => icons::SYNONYM,
        RowIcon::Column => icons::COLUMN,
        RowIcon::ColumnKey => icons::COLUMN_KEY,
        RowIcon::ColumnNotNull => icons::COLUMN_NOT_NULL,
        RowIcon::Key => icons::KEY,
        RowIcon::ForeignKey => icons::FOREIGN_KEY,
        RowIcon::Index => icons::INDEX,
        RowIcon::UniqueIndex => icons::UNIQUE_INDEX,
        RowIcon::Check => icons::CHECK,
        RowIcon::Trigger => icons::TRIGGER,
        RowIcon::Status => icons::DATA_SOURCE,
        RowIcon::Error => icons::ERROR,
    }
}

/// What a row shows.
pub(super) struct RowText {
    icon: Option<IconName>,
    pub(super) view: NodeView,
    /// A failed status row: the text in the error colour, a Retry link.
    failed: bool,
}

/// The tree's part of the explorer.
#[derive(Default)]
pub(crate) struct TreeState {
    pub lives: HashMap<String, Live>,
    pub expanded: std::collections::HashSet<RowKey>,
    pub rows: Vec<Row>,
    pub selected: Option<RowKey>,
    cache_tasks: HashMap<String, gpui::Task<()>>,
    /// How many schemas and databases the servers offer, by data source
    /// (the counters after the names).
    pub totals: HashMap<String, Totals>,
    total_tasks: HashMap<String, gpui::Task<()>>,
    /// Statements of the context menus' actions that run.
    pub ddl: Vec<super::actions::DdlRun>,
    /// The Quick Documentation popup.
    pub doc: Option<super::actions::DocPopup>,
    /// The confirmation of Remove Data Source.
    pub confirm: Option<super::actions::Confirm>,
    /// Where the rows are drawn (window coordinates), for popups opened
    /// from the keyboard.
    pub rows_bounds: std::rc::Rc<std::cell::Cell<gpui::Bounds<gpui::Pixels>>>,
    pub search: super::speed_search::SpeedSearch,
}

/// What a server offers, as far as it is read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Totals {
    pub schemas: Option<usize>,
    pub databases: Option<usize>,
}

impl Explorer {
    /// The events of the data source handles, delivered on the UI thread.
    pub(super) fn event_sink(&mut self, cx: &mut Context<Self>) -> EventSink {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Event>();
        self.events_task = cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = rx.next().await {
                if this
                    .update(cx, |this, cx| this.on_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        EventSink::new(move |e| {
            let _ = tx.unbounded_send(e);
        })
    }

    fn sink(&mut self, cx: &mut Context<Self>) -> EventSink {
        if self.sink.is_none() {
            self.sink = Some(self.event_sink(cx));
        }
        self.sink.clone().expect("made above")
    }

    /// Recomputes the visible rows (after an expansion or a model change).
    pub(super) fn rebuild_rows(&mut self) {
        let sources = self.sources.as_deref().unwrap_or_default();
        let names: Vec<Arc<str>> = sources.iter().map(|s| Arc::from(s.name.as_str())).collect();
        let trees: Vec<SourceTree<'_>> = sources
            .iter()
            .zip(&names)
            .map(|(s, name)| {
                let live = self.tree.lives.get(&s.name);
                SourceTree {
                    name,
                    database: database_name(&s.profile),
                    model: live.and_then(Live::shown_model),
                    status: live.map(Live::status),
                }
            })
            .collect();
        let rows = tree::rows(&trees, &self.tree.expanded);
        drop(trees);
        self.tree.rows = if self.tree.search.filter && self.tree.search.active() {
            super::speed_search::filter_rows(rows, |r| {
                self.tree.search.find(&self.row_text(r).view.name).is_some()
            })
        } else {
            rows
        };
    }

    /// Keeps what still applies after the data sources were read again: a
    /// data source whose connection or session options changed gets a new
    /// handle; one whose display or introspection options changed (colour,
    /// comment, level, scope) keeps its handle with the new profile.
    pub(super) fn sources_changed(&mut self) {
        let sources = self.sources.clone().unwrap_or_default();
        self.tree
            .totals
            .retain(|name, _| sources.iter().any(|s| &s.name == name));
        self.tree.lives.retain(|name, live| {
            match sources
                .iter()
                .find(|s| &s.name == name && same_connection(&s.profile, &live.profile))
            {
                Some(s) => {
                    live.profile = s.profile.clone();
                    true
                }
                None => false,
            }
        });
        self.tree
            .expanded
            .retain(|k| sources.iter().any(|s| *s.name == *k.source));
        if self
            .tree
            .selected
            .as_ref()
            .is_some_and(|k| !sources.iter().any(|s| *s.name == *k.source))
        {
            self.tree.selected = None;
        }
        self.rebuild_rows();
    }

    /// Opens again the handles of the open data sources that lost theirs
    /// (renamed, or a connection option changed).
    pub(super) fn reopen_expanded(&mut self, cx: &mut Context<Self>) {
        let names: Vec<String> = self
            .tree
            .expanded
            .iter()
            .filter(|k| k.node == NodeKey::Source && !self.tree.lives.contains_key(&*k.source))
            .map(|k| k.source.to_string())
            .collect();
        for name in &names {
            self.open_live(name, cx);
        }
        if !names.is_empty() {
            self.rebuild_rows();
        }
    }

    /// Opens the data source's handle: its cached model first, a refresh
    /// when there is none.
    pub(super) fn open_live(&mut self, name: &str, cx: &mut Context<Self>) {
        let (Some(root), Some(source)) = (
            self.root.clone(),
            self.sources().iter().find(|s| s.name == name).cloned(),
        ) else {
            return;
        };
        let runtime = match live::runtime(cx) {
            Ok(r) => r,
            Err(e) => {
                self.notify(
                    Notification::new(NotificationKind::Error, name.to_owned(), e.to_string()),
                    cx,
                );
                return;
            }
        };
        let store = crate::store(cx);
        let sink = self.sink(cx);
        let handle = runtime.data_source(
            DataSourceId::from(name),
            live::options(&source.profile),
            store.connections(&root),
            sink,
        );
        let mut live = Live::new(source.profile.clone(), handle);
        match store.model_cache(&root) {
            Some(cache) => {
                live.cache_loading = true;
                let id = live.id().clone();
                let load = cx.background_spawn(async move { cache.load(&id).ok() });
                let key = name.to_owned();
                let task = cx.spawn(async move |this, cx| {
                    let model = load.await;
                    let _ = this.update(cx, |this, cx| this.cache_loaded(&key, model, cx));
                });
                self.tree.cache_tasks.insert(name.to_owned(), task);
            }
            None => live.refresh(),
        }
        self.tree.lives.insert(name.to_owned(), live);
        self.load_totals(&source, &root, cx);
    }

    /// Reads how many schemas and databases the server offers, in the
    /// background, for the counters.
    fn load_totals(&mut self, source: &DataSource, root: &Path, cx: &mut Context<Self>) {
        let store = crate::store(cx);
        let schemas = store.schemas(root, source.clone(), None);
        let databases = (source.profile.backend == db_core::Backend::Postgres)
            .then(|| store.databases(root, source.clone(), None));
        let run = cx.background_spawn(async move {
            let schemas = schemas.await.ok().map(|s| s.len());
            let databases = match databases {
                Some(d) => d.await.ok().map(|d| d.len()),
                None => None,
            };
            Totals { schemas, databases }
        });
        let name = source.name.clone();
        let key = name.clone();
        let task = cx.spawn(async move |this, cx| {
            let totals = run.await;
            let _ = this.update(cx, |this, cx| {
                this.tree.total_tasks.remove(&key);
                this.tree.totals.insert(key, totals);
                cx.notify();
            });
        });
        self.tree.total_tasks.insert(name, task);
    }

    fn cache_loaded(&mut self, name: &str, model: Option<Model>, cx: &mut Context<Self>) {
        self.tree.cache_tasks.remove(name);
        let Some(live) = self.tree.lives.get_mut(name) else {
            return;
        };
        live.cache_loading = false;
        match model {
            // A refresh that finished first wins.
            Some(model) if live.handle.model().generation() == 0 => {
                live.model = live.handle.model().set(model);
                live.has_model = true;
            }
            Some(_) => {}
            None => live.refresh(),
        }
        self.rebuild_rows();
        cx.notify();
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        let name = event.data_source.as_str().to_owned();
        let kind = match event.kind {
            EventKind::Job { job, event, .. } if self.tree.ddl.iter().any(|r| r.job == job) => {
                self.on_job_event(job, event, cx);
                return;
            }
            EventKind::Session { .. } | EventKind::Job { .. } => {
                crate::console::route(event, cx);
                return;
            }
            other => other,
        };
        let Some(live) = self.tree.lives.get_mut(&name) else {
            return;
        };
        let mut balloon = None;
        match kind {
            EventKind::State { state } => live.state = state,
            EventKind::Refresh { job, event } if live.refresh == Some(job) => match event {
                RefreshEvent::Started => {}
                RefreshEvent::Finished { .. } => {
                    live.refresh = None;
                    live.model = live.handle.model().get();
                    live.has_model = true;
                    live.error = None;
                    // The schemas with a level of their own, one by one.
                    live.refresh_next();
                    if let Some(root) = self.root.clone()
                        && let Some(cache) = crate::store(cx).model_cache(&root)
                    {
                        let model = live.model.clone();
                        cx.background_spawn(async move {
                            // A cache that cannot be written is rebuilt by
                            // the next refresh.
                            let _ = cache.store(&model);
                        })
                        .detach();
                    }
                }
                RefreshEvent::Failed { error } => {
                    live.refresh = None;
                    live.queued.clear();
                    if live.shown_model().is_some() {
                        // The tree keeps the cached model; the reason goes
                        // to a balloon.
                        balloon = Some(error.message);
                    } else {
                        live.error = Some(error.message);
                    }
                }
                RefreshEvent::Cancelled => {
                    live.refresh = None;
                    live.refresh_next();
                }
            },
            _ => return,
        }
        if let Some(text) = balloon {
            self.notify(Notification::new(NotificationKind::Warning, name, text), cx);
        }
        self.rebuild_rows();
        cx.notify();
    }

    /// Opens or closes a node.
    pub(super) fn toggle(&mut self, key: RowKey, cx: &mut Context<Self>) {
        if !self.tree.expanded.remove(&key) {
            if key.node == NodeKey::Source && !self.tree.lives.contains_key(&*key.source) {
                self.open_live(&key.source, cx);
            }
            self.tree.expanded.insert(key);
        }
        self.rebuild_rows();
        cx.notify();
    }

    /// Opens every node of the loaded data sources, level by level (the
    /// local model only), up to a bound for very large models.
    pub(super) fn expand_all(&mut self, cx: &mut Context<Self>) {
        const MAX_ROWS: usize = 20_000;
        loop {
            let closed: Vec<RowKey> = self
                .tree
                .rows
                .iter()
                .filter(|r| {
                    r.expandable
                        && !r.expanded
                        && (r.key.node != NodeKey::Source
                            || self.tree.lives.contains_key(&*r.key.source))
                })
                .map(|r| r.key.clone())
                .collect();
            if closed.is_empty() || self.tree.rows.len() >= MAX_ROWS {
                break;
            }
            self.tree.expanded.extend(closed);
            self.rebuild_rows();
        }
        cx.notify();
    }

    /// Closes every node, leaving the data sources.
    pub(super) fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.tree.expanded.clear();
        self.rebuild_rows();
        cx.notify();
    }

    /// Scrolls to the selected row (the selected data source's when no row
    /// is selected).
    pub(super) fn locate(&mut self, cx: &mut Context<Self>) {
        if self.tree.selected.is_none()
            && let Some(name) = self.selected_source().map(|s| s.name.clone())
        {
            self.select_row(RowKey::source(&name.as_str().into()), cx);
        }
        if let Some(ix) = self
            .tree
            .rows
            .iter()
            .position(|r| Some(&r.key) == self.tree.selected.as_ref())
        {
            self.scroll.scroll_to_item(ix, gpui::ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// Refreshes the selected data source (opening it when it is not); a
    /// selected schema, or a node or group in one, reads that schema only.
    pub(super) fn refresh_selected(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.selected_source().map(|s| s.name.clone()) else {
            return;
        };
        let schema = match self.tree.selected.as_ref().map(|k| k.node) {
            Some(NodeKey::Node(id) | NodeKey::Group(id, _)) => self
                .model_of(&name)
                .and_then(|m| m.node(id))
                .map(|n| n.schema().name().to_owned()),
            _ => None,
        };
        match self.tree.lives.get_mut(&name) {
            Some(live) => match schema {
                Some(schema) => live.refresh_schema(&schema),
                None => live.refresh(),
            },
            None => self.open_live(&name, cx),
        }
        self.rebuild_rows();
        cx.notify();
    }

    fn retry(&mut self, source: &str, cx: &mut Context<Self>) {
        if let Some(live) = self.tree.lives.get_mut(source) {
            live.refresh();
        }
        self.rebuild_rows();
        cx.notify();
    }

    /// The index of the selected row.
    pub(super) fn selected_ix(&self) -> Option<usize> {
        let key = self.tree.selected.as_ref()?;
        self.tree.rows.iter().position(|r| &r.key == key)
    }

    pub(super) fn select_ix(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.tree.rows.get(ix) else {
            return;
        };
        let key = row.key.clone();
        self.select_row(key, cx);
        // Scrolls as little as shows the row whole.
        let row_top = cx.theme().metric(Metric::RowHeight) * ix as f32;
        let top = -self.scroll.0.borrow().base_handle.offset().y;
        let strategy = if row_top < top {
            gpui::ScrollStrategy::Top
        } else {
            gpui::ScrollStrategy::Bottom
        };
        self.scroll.scroll_to_item(ix, strategy);
    }

    /// Up and Down: the row above or below (the first one when none is
    /// selected).
    pub(super) fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.search_step(delta, cx) {
            return;
        }
        let last = self.tree.rows.len().saturating_sub(1);
        let ix = match self.selected_ix() {
            Some(ix) => ix.saturating_add_signed(delta).min(last),
            None => 0,
        };
        self.select_ix(ix, cx);
    }

    /// Right: opens the selected node, or selects its first child when it
    /// is open.
    pub(super) fn expand_or_next(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.selected_ix() else {
            return;
        };
        let row = self.tree.rows[ix].clone();
        if !row.expandable {
            return;
        }
        if !row.expanded {
            self.toggle(row.key, cx);
        } else if self
            .tree
            .rows
            .get(ix + 1)
            .is_some_and(|n| n.depth > row.depth)
        {
            self.select_ix(ix + 1, cx);
        }
    }

    /// Left: closes the selected node, or selects its parent.
    pub(super) fn collapse_or_parent(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.selected_ix() else {
            return;
        };
        let row = self.tree.rows[ix].clone();
        if row.expandable && row.expanded {
            self.toggle(row.key, cx);
        } else if let Some(parent) = self.tree.rows[..ix]
            .iter()
            .rposition(|r| r.depth < row.depth)
        {
            self.select_ix(parent, cx);
        }
    }

    /// Enter and a double click: opens the data of a table, view or
    /// materialized view, else opens or closes the selected node.
    pub(super) fn toggle_selected(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.selected_ix() else {
            return;
        };
        let row = self.tree.rows[ix].clone();
        if self.open_row_table(&row.key, cx) {
            return;
        }
        if row.expandable {
            self.toggle(row.key, cx);
        }
    }

    pub(super) fn select_row(&mut self, key: RowKey, cx: &mut Context<Self>) {
        // Another row closes the documentation popup of the last one.
        if self.tree.selected.as_ref() != Some(&key) {
            self.tree.doc = None;
        }
        self.selected = Some(key.source.to_string());
        self.tree.selected = Some(key);
        cx.notify();
    }

    /// The model a row's node is in.
    pub(super) fn model_of(&self, source: &str) -> Option<&Model> {
        self.tree.lives.get(source).and_then(Live::shown_model)
    }

    pub(super) fn row_text(&self, row: &Row) -> RowText {
        let source = &*row.key.source;
        let profile = self
            .sources()
            .iter()
            .find(|s| s.name == source)
            .map(|s| &s.profile);
        let totals = self.tree.totals.get(source).copied();
        let shown_schemas = self.model_of(source).map(|m| m.schemas().len());
        let plain = |icon: Option<IconName>, name: String| RowText {
            icon,
            view: NodeView {
                icon: RowIcon::Status,
                name,
                detail: None,
                count: None,
            },
            failed: false,
        };
        match row.key.node {
            NodeKey::Source => {
                let mut text = plain(
                    Some(profile.map_or(icons::DATA_SOURCE, |p| icons::backend(p.backend))),
                    source.to_owned(),
                );
                // The databases shown of the server's (the current one),
                // or the schemas where the engine has one database.
                text.view.detail = match (profile.and_then(database_name), totals) {
                    (
                        Some(_),
                        Some(Totals {
                            databases: Some(n), ..
                        }),
                    ) => Some(format!("{} of {n}", 1.min(n))),
                    (
                        None,
                        Some(Totals {
                            schemas: Some(n), ..
                        }),
                    ) => shown_schemas.map(|k| format!("{k} of {}", n.max(k))),
                    _ => None,
                };
                text
            }
            NodeKey::Database => {
                let mut text = plain(
                    Some(icons::DATABASE_NODE),
                    profile
                        .and_then(database_name)
                        .unwrap_or_default()
                        .to_owned(),
                );
                text.view.detail = match (totals, shown_schemas) {
                    (
                        Some(Totals {
                            schemas: Some(n), ..
                        }),
                        Some(k),
                    ) => Some(format!("{k} of {}", n.max(k))),
                    _ => None,
                };
                text
            }
            NodeKey::Status => {
                let status = self
                    .tree
                    .lives
                    .get(source)
                    .map_or(Status::Connecting, Live::status);
                match status {
                    Status::Connecting => plain(None, CONNECTING.into()),
                    Status::Loading => plain(None, LOADING.into()),
                    Status::Empty => plain(None, EMPTY.into()),
                    Status::Failed(message) => RowText {
                        failed: true,
                        ..plain(Some(icons::ERROR), message)
                    },
                }
            }
            NodeKey::Node(id) => match self.model_of(source).and_then(|m| m.node(id)) {
                Some(node) => {
                    let view = tree::node_view(node);
                    RowText {
                        icon: Some(icon_name(view.icon)),
                        view,
                        failed: false,
                    }
                }
                None => plain(None, String::new()),
            },
            NodeKey::Group(parent, group) => {
                match self.model_of(source).and_then(|m| m.node(parent)) {
                    Some(node) => RowText {
                        icon: Some(icons::FOLDER),
                        view: tree::group_view(node, group),
                        failed: false,
                    },
                    None => plain(None, String::new()),
                }
            }
        }
    }

    /// The path of each row: the names from its data source down, joined
    /// by `/` (what `db.explorer.expand` takes).
    fn row_paths(&self) -> Vec<String> {
        let mut stack: Vec<String> = Vec::new();
        self.tree
            .rows
            .iter()
            .map(|row| {
                stack.truncate(row.depth);
                stack.push(self.row_text(row).view.name);
                stack.join("/")
            })
            .collect()
    }

    /// The row at `path` (see [`Self::row_paths`]).
    pub(super) fn row_at(&self, path: &str) -> Option<RowKey> {
        let paths = self.row_paths();
        let ix = paths.iter().position(|p| p == path)?;
        Some(self.tree.rows[ix].key.clone())
    }

    /// Opens every node along `path`, one level at a time (each level's
    /// rows exist once its parent is open).
    pub(super) fn expand_path(&mut self, path: &str, cx: &mut Context<Self>) {
        let parts: Vec<&str> = path.split('/').collect();
        for n in 1..=parts.len() {
            let prefix = parts[..n].join("/");
            let Some(key) = self.row_at(&prefix) else {
                return;
            };
            if !self.tree.expanded.contains(&key) {
                self.toggle(key.clone(), cx);
            }
            if n == parts.len() {
                self.select_row(key, cx);
            }
        }
    }

    pub(super) fn collapse_path(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(key) = self.row_at(path)
            && self.tree.expanded.contains(&key)
        {
            self.toggle(key, cx);
        }
    }

    /// The rows for `db.state`.
    pub(super) fn tree_json(&self) -> Json {
        let paths = self.row_paths();
        let rows: Vec<Json> = self
            .tree
            .rows
            .iter()
            .zip(paths)
            .map(|(row, path)| {
                let text = self.row_text(row);
                json!({
                    "path": path,
                    "depth": row.depth,
                    "text": text.view.name,
                    "detail": text.view.detail,
                    "count": text.view.count,
                    "expandable": row.expandable,
                    "expanded": row.expanded,
                    "failed": text.failed,
                    "selected": self.tree.selected.as_ref() == Some(&row.key),
                })
            })
            .collect();
        let lives: serde_json::Map<String, Json> = self
            .tree
            .lives
            .iter()
            .map(|(name, live)| {
                let state = match &live.state {
                    DataSourceState::Disconnected => "disconnected",
                    DataSourceState::Connecting => "connecting",
                    DataSourceState::Connected => "connected",
                    DataSourceState::Failed { .. } => "failed",
                };
                (
                    name.clone(),
                    json!({
                        "state": state,
                        "refreshing": live.refresh.is_some(),
                        "has_model": live.has_model,
                        "objects": live.model.len(),
                        "error": live.error,
                    }),
                )
            })
            .collect();
        json!({ "rows": rows, "data_sources": lives })
    }

    // ---- drawing ----------------------------------------------------------------

    fn guides(&self, cx: &App) -> ui::tree_guides::TreeGuides {
        ui::tree_guides::TreeGuides {
            first: px(TREE_LEFT),
            indent: cx.theme().metric(Metric::TreeIndent),
            control: px(CHEVRON),
        }
    }

    fn render_row(&self, ix: usize, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let row = self.tree.rows[ix].clone();
        let text = self.row_text(&row);
        let theme = cx.theme();
        let row_height = theme.metric(Metric::RowHeight);
        let indent = theme.metric(Metric::TreeIndent);
        let left = px(TREE_LEFT) + indent * row.depth as f32;
        let selected = self.tree.selected.as_ref() == Some(&row.key);
        let selection = theme.color(if focused {
            UiRole::Selection
        } else {
            UiRole::SelectionInactive
        });
        let hover = theme.color(UiRole::Hover);
        let name_color: Hsla = if text.failed {
            theme.color(UiRole::Danger)
        } else if row.key.node == NodeKey::Status {
            theme.color(UiRole::TextMuted)
        } else if selected && focused {
            theme.color(UiRole::SelectionText)
        } else {
            theme.color(UiRole::Text)
        };
        let muted = theme.color(UiRole::TextMuted);
        let small = theme.ui_size() - px(2.0);
        let guides = self.guides(cx).row(
            row.depth,
            row.expanded,
            row_height,
            theme.color(UiRole::TreeIndentGuide),
        );
        let key = row.key.clone();
        let chevron = row.expandable.then(|| {
            let key = key.clone();
            div()
                .id(("db-chevron", ix))
                .absolute()
                .left(left)
                .top((row_height - px(CHEVRON)) / 2.0)
                .child(ui::components::tree_arrow(row.expanded, cx))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.select_row(key.clone(), cx);
                    this.toggle(key.clone(), cx);
                }))
        });
        let icon = text.icon.map(|icon| {
            div()
                .absolute()
                .left(left + px(ICON_X))
                .top((row_height - px(16.0)) / 2.0)
                .child(Icon::new(icon).size(px(16.0)))
        });
        let extra = text
            .view
            .count
            .map(|c| c.to_string())
            .or(text.view.detail.clone());
        let retry = text.failed.then(|| {
            let source = key.source.clone();
            ui::link::link(("db-retry", ix), RETRY, cx).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.retry(&source, cx);
                },
            ))
        });
        let selector = format!("database-row:{}", text.view.name);
        let label = div()
            .absolute()
            .left(left + px(ICON_X + TEXT_X))
            .top_0()
            .h(row_height)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(DETAIL_GAP))
            .whitespace_nowrap()
            .child(
                div()
                    .text_color(name_color)
                    .child(self.row_label(&text.view.name, cx)),
            )
            .children(extra.map(|e| div().text_size(small).text_color(muted).child(e)))
            .children(retry);
        let click_key = key.clone();
        let menu_key = key.clone();
        div()
            .id(("db-row", ix))
            .debug_selector(move || selector.clone())
            .relative()
            .w_full()
            .h(row_height)
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(TREE_LEFT))
                    .right(px(TREE_LEFT))
                    .rounded(theme.metric(Metric::CornerRadius))
                    .when(selected, |d| d.bg(selection))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover))),
            )
            .children(guides)
            .children(chevron)
            .children(icon)
            .child(label)
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                window.focus(&this.focus);
                this.select_row(click_key.clone(), cx);
                if e.click_count() >= 2 {
                    this.toggle_selected(cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_context_menu(menu_key.clone(), e.position, window, cx);
                }),
            )
            .into_any_element()
    }

    pub(super) fn render_tree(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let focused = self.focus.contains_focused(window, cx);
        let count = self.tree.rows.len();
        let mut rows = uniform_list(
            "database-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|ix| this.render_row(ix, focused, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .debug_selector(|| "database-rows".into())
        .track_scroll(self.scroll.clone())
        .flex_1()
        .overflow_x_hidden();
        rows.style().restrict_scroll_to_axis = Some(true);
        div()
            .id("database-sources")
            .debug_selector(|| "database-sources".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .pt(px(TREE_TOP))
            .relative()
            .child({
                let bounds = self.tree.rows_bounds.clone();
                gpui::canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .top(px(TREE_TOP))
                    .left_0()
                    .size_full()
            })
            .child(rows)
            .children(self.error.clone().map(|e| {
                div()
                    .px(px(TREE_LEFT))
                    .text_color(cx.theme().color(UiRole::Danger))
                    .child(e)
            }))
            .into_any_element()
    }
}
