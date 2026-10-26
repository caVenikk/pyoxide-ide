//! The commands of the object tree's context menus (see [`context_menu`]),
//! run on the selected row: the data source's own (Duplicate, Rename,
//! Move, Set Color, Deactivate, Remove), what reads or forgets the model
//! (Refresh of a schema, Force Refresh, Forget All Cached Schemas,
//! Introspection Level), what copies text (Copy Reference, Generate DDL to
//! Clipboard) and what changes the database through a statement shown
//! first (Drop, Truncate, Rename of an object), run on a session of the
//! data source in auto-commit, after which the schema is read again.
//!
//! [`context_menu`]: super::context_menu

use std::path::PathBuf;
use std::sync::Arc;

use db_core::schema::{Level, SchemaFilter, Scope};
use db_model::{Model, NodeKind, NodeRef};
use db_session::{
    DataSourceState, Job, JobEvent, JobId, JobOutcome, Session, SessionOptions, TxPolicy,
};
use gpui::{
    App, AppContext as _, ClipboardItem, Context, DismissEvent, Pixels, Point, Window, point,
};
use serde_json::{Value as Json, json};
use themes::Metric;
use ui::icons::IconName;
use ui::menu::ContextMenu;
use ui::notification::{Notification, NotificationKind};
use ui::theme::ThemeExt;
use workbench::dialog::{Dialog, DialogButton, DialogEvent};

use super::context_menu::{self, MenuInput, Target};
use super::sql_dialog::{self, Ask};
use super::tree_view::database_name;
use super::{Explorer, OpenMenu, with_target};
use crate::ddl::{self, DropOptions};
use crate::store::{DataSource, SaveRequest};
use crate::tree::{NodeKey, RowKey};

ui::ui_commands! {
    ACTION_COMMANDS, scope = "Database";
    /// Show the selected row's context menu.
    ShowContextMenu => "db.explorer.context-menu", "Show Context Menu",
        "Shows the context menu of the selected row of the Database tool window's tree, below the row.";
    /// Copy the selected node's name.
    CopyReference => "db.explorer.copy-reference", "Copy Reference",
        "Copies the qualified name of the selected node of the Database tool window (`schema.table`, `schema.table.column`; a data source's name) to the clipboard.";
    /// Duplicate the selected data source.
    Duplicate => "db.explorer.duplicate", "Duplicate",
        "Adds a copy of the selected data source after it, named `<name> (2)`; its password is not copied.";
    /// Close the data source's connections.
    Deactivate => "db.explorer.deactivate", "Deactivate",
        "Closes every connection of the selected data source (an open transaction is rolled back); the tree keeps its model.";
    /// Rename the selected node.
    Rename => "db.explorer.rename", "Rename\u{2026}",
        "Asks for a new name of the selected data source, or of the selected database object, whose rename statement shows before it runs.";
    /// Move the data source up.
    MoveUp => "db.explorer.move-up", "Move Up",
        "Moves the selected data source one place up in the Database tool window.";
    /// Move the data source down.
    MoveDown => "db.explorer.move-down", "Move Down",
        "Moves the selected data source one place down in the Database tool window.";
    /// Set the data source's colour.
    SetColor { color: String } => "db.explorer.set-color", "Set Color",
        "Sets the colour of the selected data source: `none`, `blue`, `teal`, `green`, `yellow`, `orange`, `red`, `purple` or `gray`.",
        r#"{"color": "blue"}"#;
    /// Show the selected node's documentation.
    QuickDocumentation => "db.explorer.quick-doc", "Quick Documentation",
        "Shows a popup with what the local model knows of the selected node of the Database tool window: its kind, name, comment and DDL.";
    /// Set how much a refresh reads.
    SetIntrospectionLevel { level: String } => "db.explorer.introspection-level", "Introspection Level",
        "Sets the introspection level of the selected data source (data source and database rows) or schema: `auto`, `names`, `structure` or `full`, and reads the metadata again at it.",
        r#"{"level": "full"}"#;
    /// Copy the selected node's DDL.
    GenerateDdl => "db.explorer.generate-ddl", "Generate DDL to Clipboard",
        "Copies the DDL of the selected node of the Database tool window, made from the local model (a data source: every schema), to the clipboard.";
    /// Read the whole scope again from nothing.
    ForceRefresh => "db.explorer.force-refresh", "Force Refresh",
        "Forgets the model of the selected data source and reads its introspection scope again.";
    /// Forget the model.
    ForgetCachedSchemas => "db.explorer.forget-cached-schemas", "Forget All Cached Schemas",
        "Forgets the model of the selected data source and its copy on disk; the tree shows it again after the next refresh.";
    /// Edit the introspection scope.
    ManageShownSchemas => "db.explorer.manage-shown-schemas", "Manage Shown Schemas\u{2026}",
        "Opens the Data Sources dialog at the Schemas tab of the selected data source.";
    /// Hide the selected schema.
    HideSchema => "db.explorer.hide-schema", "Hide Schemas",
        "Takes the selected schema out of the data source's introspection scope and the tree.";
    /// Remove the data source or drop the object.
    Delete => "db.explorer.delete", "Delete",
        "Asks to remove the selected data source (Remove Data Source…), or shows the DROP statements of the selected database object or group (Drop…) and runs them on OK.";
    /// Empty the selected table.
    Truncate => "db.explorer.truncate", "Truncate\u{2026}",
        "Shows the statement that empties the selected table and runs it on OK.";
}

/// Texts of the dialogs and balloons (our own).
const REMOVE_TITLE: &str = "Remove Data Source";
const DROP_TITLE: &str = "Drop";
const TRUNCATE_TITLE: &str = "Truncate";
const RENAME_TITLE: &str = "Rename";

pub(super) fn init(cx: &mut App) {
    ui::command::register(cx, ACTION_COMMANDS);
    cx.on_action(|_: &ShowContextMenu, cx| {
        with_target(cx, |e, window, cx| e.show_context_menu_below(window, cx))
    });
    cx.on_action(|_: &CopyReference, cx| with_target(cx, |e, _, cx| e.copy_reference(cx)));
    cx.on_action(|_: &Duplicate, cx| with_target(cx, |e, _, cx| e.duplicate(cx)));
    cx.on_action(|_: &Deactivate, cx| with_target(cx, |e, _, cx| e.deactivate(cx)));
    cx.on_action(|_: &Rename, cx| with_target(cx, |e, window, cx| e.rename(window, cx)));
    cx.on_action(|_: &MoveUp, cx| with_target(cx, |e, _, cx| e.move_source(-1, cx)));
    cx.on_action(|_: &MoveDown, cx| with_target(cx, |e, _, cx| e.move_source(1, cx)));
    cx.on_action(|a: &SetColor, cx| {
        let color = a.color.clone();
        with_target(cx, move |e, _, cx| e.set_color(&color, cx))
    });
    cx.on_action(|_: &QuickDocumentation, cx| {
        with_target(cx, |e, window, cx| e.quick_documentation(window, cx))
    });
    cx.on_action(|a: &SetIntrospectionLevel, cx| {
        let level = a.level.clone();
        with_target(cx, move |e, _, cx| e.set_level(&level, cx))
    });
    cx.on_action(|_: &GenerateDdl, cx| with_target(cx, |e, _, cx| e.generate_ddl(cx)));
    cx.on_action(|_: &ForceRefresh, cx| with_target(cx, |e, _, cx| e.force_refresh(cx)));
    cx.on_action(|_: &ForgetCachedSchemas, cx| {
        with_target(cx, |e, _, cx| e.forget_cached_schemas(cx))
    });
    cx.on_action(|_: &ManageShownSchemas, cx| {
        with_target(cx, |e, window, cx| e.manage_shown_schemas(window, cx))
    });
    cx.on_action(|_: &HideSchema, cx| with_target(cx, |e, _, cx| e.hide_schema(cx)));
    cx.on_action(|_: &Delete, cx| with_target(cx, |e, window, cx| e.delete(window, cx)));
    cx.on_action(|_: &Truncate, cx| with_target(cx, |e, window, cx| e.truncate(window, cx)));
}

/// A statement job of the tree's actions on a session of its own.
pub(crate) struct DdlRun {
    source: String,
    session: Session,
    job: JobId,
    after: After,
    error: Option<String>,
}

/// What is read again after statements ran.
#[derive(Debug, Clone, PartialEq)]
enum After {
    /// The data source's introspection scope.
    Scope,
    Schema(String),
    /// Nothing (the statements change no metadata).
    Nothing,
}

impl After {
    fn of(schema: Option<String>) -> Self {
        schema.map_or(After::Scope, After::Schema)
    }
}

/// The documentation popup of a node.
pub(crate) struct DocPopup {
    pub title: String,
    pub lines: Vec<String>,
    pub code: Option<String>,
    pub position: Point<Pixels>,
}

/// The confirmation of Remove Data Source.
pub(crate) struct Confirm {
    pub dialog: gpui::Entity<Dialog>,
    pub source: String,
    _subscription: gpui::Subscription,
}

impl Explorer {
    /// The row the actions work on: the selected row, else the selected
    /// data source.
    pub(super) fn target_key(&self) -> Option<RowKey> {
        self.tree.selected.clone().or_else(|| {
            self.selected_source()
                .map(|s| RowKey::source(&s.name.as_str().into()))
        })
    }

    fn source_named(&self, name: &str) -> Option<&DataSource> {
        self.sources().iter().find(|s| s.name == name)
    }

    fn dialect_of(&self, source: &str) -> sql_syntax::Dialect {
        self.source_named(source)
            .map(|s| ddl::dialect(s.profile.backend))
            .unwrap_or(sql_syntax::Dialect::Postgres)
    }

    /// The model a row's node is in, shared.
    fn model_arc(&self, source: &str) -> Option<Arc<Model>> {
        let live = self.tree.lives.get(source)?;
        live.shown_model().map(|_| live.model.clone())
    }

    /// The nodes a row stands for: a node, or a group's nodes.
    fn row_nodes<'a>(model: &'a Model, key: &RowKey) -> Vec<NodeRef<'a>> {
        match key.node {
            NodeKey::Node(id) => model.node(id).into_iter().collect(),
            NodeKey::Group(parent, group) => model
                .node(parent)
                .map(|p| group.nodes(p))
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// What the menu of `key` needs; `None` for a status row.
    pub(super) fn menu_input(&self, key: &RowKey) -> Option<MenuInput> {
        let source = &*key.source;
        let profile = &self.source_named(source)?.profile;
        let gui = profile.gui_options();
        let dialect = ddl::dialect(profile.backend);
        let model = self.tree.lives.get(source).and_then(|l| l.shown_model());
        let (target, rename, drop, schema_level) = match key.node {
            NodeKey::Status => return None,
            NodeKey::Source => (Target::Source, true, false, None),
            NodeKey::Database => (Target::Database, false, false, None),
            NodeKey::Node(id) => {
                let node = model?.node(id)?;
                let target = match node.kind() {
                    NodeKind::Schema => Target::Schema,
                    kind => Target::Object {
                        table: kind == NodeKind::Table,
                    },
                };
                let level = (node.kind() == NodeKind::Schema)
                    .then(|| gui.schema_levels.get(node.name()).copied())
                    .flatten();
                (
                    target,
                    ddl::rename_statement(dialect, node, "x").is_some(),
                    ddl::drop_statement(dialect, node, DropOptions::default()).is_some(),
                    level,
                )
            }
            NodeKey::Group(..) => {
                let nodes = model.map(|m| Self::row_nodes(m, key)).unwrap_or_default();
                let drop = !nodes.is_empty()
                    && nodes.iter().all(|n| {
                        ddl::drop_statement(dialect, *n, DropOptions::default()).is_some()
                    });
                (Target::Group, false, drop, None)
            }
        };
        let sources = self.sources();
        Some(MenuInput {
            target,
            connected: self
                .tree
                .lives
                .get(source)
                .is_some_and(|l| l.state == DataSourceState::Connected),
            index: sources.iter().position(|s| s.name == source).unwrap_or(0),
            count: sources.len(),
            color: gui.color.clone(),
            level: gui.level,
            schema_level,
            rename,
            drop,
        })
    }

    /// Selects `key` and opens its menu at `position` (window coordinates).
    pub(super) fn open_context_menu(
        &mut self,
        key: RowKey,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tree.doc = None;
        window.focus(&self.focus);
        self.select_row(key.clone(), cx);
        let Some(input) = self.menu_input(&key) else {
            return;
        };
        let menu = ContextMenu::build(context_menu::items(&input), window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            menu,
            position,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Where a popup of the selected row opens: below the row, where its
    /// name starts.
    fn below_selected(&self, cx: &App) -> Option<Point<Pixels>> {
        let key = self.tree.selected.as_ref()?;
        let ix = self.tree.rows.iter().position(|r| &r.key == key)?;
        let row = &self.tree.rows[ix];
        let theme = cx.theme();
        let row_height = theme.metric(Metric::RowHeight);
        let x = super::tree_view::text_x(row.depth, cx);
        let top = -self.scroll.0.borrow().base_handle.offset().y;
        let y = row_height * (ix + 1) as f32 - top;
        Some(self.tree.rows_bounds.get().origin + point(x, y))
    }

    pub(super) fn show_context_menu_below(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.target_key() else {
            return;
        };
        if self.tree.selected.is_none() {
            self.select_row(key.clone(), cx);
        }
        let position = self
            .below_selected(cx)
            .unwrap_or_else(|| self.tree.rows_bounds.get().origin);
        self.open_context_menu(key, position, window, cx);
    }

    // ---- text to the clipboard --------------------------------------------------

    fn copy_reference(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.target_key() else {
            return;
        };
        let text = match key.node {
            NodeKey::Source => Some(key.source.to_string()),
            NodeKey::Database => self
                .source_named(&key.source)
                .and_then(|s| database_name(&s.profile))
                .map(str::to_owned),
            NodeKey::Node(id) => {
                let dialect = self.dialect_of(&key.source);
                self.model_of(&key.source)
                    .and_then(|m| m.node(id))
                    .map(|n| ddl::reference(dialect, n))
            }
            NodeKey::Group(..) | NodeKey::Status => None,
        };
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// The DDL of a row from the model.
    fn ddl_of(&self, key: &RowKey) -> Option<String> {
        let dialect = self.dialect_of(&key.source);
        let model = self.model_of(&key.source)?;
        Some(match key.node {
            NodeKey::Source | NodeKey::Database => ddl::schemas_ddl(dialect, model.schemas()),
            NodeKey::Node(id) => ddl::node_ddl(dialect, model.node(id)?),
            NodeKey::Group(..) => ddl::nodes_ddl(dialect, Self::row_nodes(model, key)),
            NodeKey::Status => return None,
        })
    }

    fn generate_ddl(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.target_key().and_then(|k| self.ddl_of(&k)) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn quick_documentation(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(key) = self.target_key() else {
            return;
        };
        let Some(source) = self.source_named(&key.source).cloned() else {
            return;
        };
        let (title, mut lines) = match key.node {
            NodeKey::Source => (
                format!("data source {}", source.name),
                vec![
                    format!(
                        "{} \u{00b7} {}",
                        crate::backend_title(source.profile.backend),
                        source.profile.target()
                    ),
                    format!("user {}", source.profile.user).trim().to_owned(),
                ],
            ),
            NodeKey::Database => (
                format!(
                    "database {}",
                    database_name(&source.profile).unwrap_or_default()
                ),
                vec![format!("data source {}", source.name)],
            ),
            NodeKey::Node(id) => {
                let Some(node) = self.model_of(&key.source).and_then(|m| m.node(id)) else {
                    return;
                };
                let dialect = ddl::dialect(source.profile.backend);
                (
                    format!(
                        "{} {}",
                        ddl::kind_word(node.kind()),
                        ddl::reference(dialect, node)
                    ),
                    Vec::new(),
                )
            }
            NodeKey::Group(..) | NodeKey::Status => return,
        };
        if let NodeKey::Node(id) = key.node
            && let Some(comment) = self
                .model_of(&key.source)
                .and_then(|m| m.node(id))
                .and_then(|n| n.comment())
        {
            lines.push(comment.to_owned());
        }
        let code = match key.node {
            NodeKey::Node(_) => self.ddl_of(&key).map(|t| t.trim_end().to_owned()),
            _ => None,
        };
        let position = self
            .below_selected(cx)
            .unwrap_or_else(|| self.tree.rows_bounds.get().origin);
        self.tree.doc = Some(DocPopup {
            title,
            lines,
            code,
            position,
        });
        cx.notify();
    }

    // ---- the data source's settings -------------------------------------------

    /// Writes the data sources (`sources` in their new order) and reads
    /// them again; renamed and removed ones as said.
    fn save_sources(&mut self, request: SaveRequest, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let store = crate::store(cx);
        let run_root: PathBuf = root.clone();
        let run = cx.background_spawn(async move { store.save(&run_root, request) });
        self.load_task = cx.spawn(async move |this, cx| {
            let result = run.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = result {
                    this.notify(
                        Notification::new(NotificationKind::Error, "Data sources", e),
                        cx,
                    );
                }
                this.load(cx);
            });
        });
    }

    /// Changes one data source's profile and saves; its open handle keeps
    /// running with the new profile when only display or introspection
    /// options changed.
    fn update_source(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut DataSource),
        cx: &mut Context<Self>,
    ) {
        let mut sources = self.sources().to_vec();
        let Some(source) = sources.iter_mut().find(|s| s.name == name) else {
            return;
        };
        f(source);
        let profile = source.profile.clone();
        if let Some(live) = self.tree.lives.get_mut(name) {
            live.profile = profile;
        }
        self.sources = Some(sources.clone());
        self.save_sources(
            SaveRequest {
                sources,
                ..SaveRequest::default()
            },
            cx,
        );
        cx.notify();
    }

    fn selected_source_name(&self) -> Option<String> {
        self.target_key().map(|k| k.source.to_string())
    }

    fn duplicate(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.target_key().filter(|k| k.node == NodeKey::Source) else {
            return;
        };
        let mut sources = self.sources().to_vec();
        let Some(at) = sources.iter().position(|s| *s.name == *key.source) else {
            return;
        };
        let name = (2..)
            .map(|n| format!("{} ({n})", key.source))
            .find(|n| !sources.iter().any(|s| &s.name == n))
            .expect("a free name");
        let copy = DataSource {
            name: name.clone(),
            profile: sources[at].profile.clone(),
        };
        sources.insert(at + 1, copy);
        self.sources = Some(sources.clone());
        self.selected = Some(name.clone());
        self.tree.selected = Some(RowKey::source(&name.as_str().into()));
        self.sources_changed();
        self.save_sources(
            SaveRequest {
                sources,
                ..SaveRequest::default()
            },
            cx,
        );
        cx.notify();
    }

    fn move_source(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(key) = self.target_key().filter(|k| k.node == NodeKey::Source) else {
            return;
        };
        let mut sources = self.sources().to_vec();
        let Some(at) = sources.iter().position(|s| *s.name == *key.source) else {
            return;
        };
        let Some(to) = at.checked_add_signed(delta).filter(|t| *t < sources.len()) else {
            return;
        };
        sources.swap(at, to);
        self.sources = Some(sources.clone());
        self.sources_changed();
        self.save_sources(
            SaveRequest {
                sources,
                ..SaveRequest::default()
            },
            cx,
        );
        cx.notify();
    }

    fn set_color(&mut self, color: &str, cx: &mut Context<Self>) {
        let Some(name) = self.selected_source_name() else {
            return;
        };
        let known = crate::dialog::COLORS.iter().any(|(c, _)| *c == color);
        if !known {
            return;
        }
        let color = (color != "none").then(|| color.to_owned());
        self.update_source(&name, |s| set_gui(s, |g| g.color = color), cx);
    }

    fn set_level(&mut self, level: &str, cx: &mut Context<Self>) {
        let level = match level {
            "auto" => None,
            "names" => Some(Level::Names),
            "structure" => Some(Level::Structure),
            "full" => Some(Level::Full),
            _ => return,
        };
        let Some(key) = self.target_key() else {
            return;
        };
        let schema = match key.node {
            NodeKey::Source | NodeKey::Database => None,
            NodeKey::Node(id) => match self.model_of(&key.source).and_then(|m| m.node(id)) {
                Some(n) if n.kind() == NodeKind::Schema => Some(n.name().to_owned()),
                _ => return,
            },
            NodeKey::Group(..) | NodeKey::Status => return,
        };
        let name = key.source.to_string();
        let set_schema = schema.clone();
        self.update_source(
            &name,
            move |s| {
                set_gui(s, |g| match set_schema {
                    None => g.level = level,
                    Some(schema) => match level {
                        Some(l) => {
                            g.schema_levels.insert(schema, l);
                        }
                        None => {
                            g.schema_levels.remove(&schema);
                        }
                    },
                })
            },
            cx,
        );
        if let Some(live) = self.tree.lives.get_mut(&name) {
            match &schema {
                Some(schema) => live.refresh_schema(schema),
                None => live.refresh(),
            }
        }
        self.rebuild_rows();
        cx.notify();
    }

    fn manage_shown_schemas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let (Some(root), Some(name)) = (self.root.clone(), self.selected_source_name()) else {
            return;
        };
        let sources = self.sources().to_vec();
        crate::dialog::open_schemas(root, sources, name, window, cx);
    }

    fn hide_schema(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.target_key() else {
            return;
        };
        let NodeKey::Node(id) = key.node else {
            return;
        };
        let Some(model) = self.model_arc(&key.source) else {
            return;
        };
        let Some(schema) = model
            .node(id)
            .filter(|n| n.kind() == NodeKind::Schema)
            .map(|n| n.name().to_owned())
        else {
            return;
        };
        let shown: Vec<String> = model
            .schemas()
            .map(|s| s.name().to_owned())
            .filter(|s| *s != schema)
            .collect();
        let name = key.source.to_string();
        self.update_source(
            &name,
            move |s| {
                set_gui(s, |g| {
                    let system = g.scope.as_ref().is_some_and(|s| s.system);
                    g.scope = Some(Scope {
                        filter: SchemaFilter::List(shown),
                        system,
                    });
                    g.schema_levels.remove(&schema);
                })
            },
            cx,
        );
        if let Some(live) = self.tree.lives.get_mut(&name) {
            let hidden =
                model.without_schemas(&[model.node(id).map(|n| n.name()).unwrap_or_default()]);
            live.model = live.handle.model().set(hidden);
            self.store_model(&name, cx);
        }
        self.tree.selected = None;
        self.rebuild_rows();
        cx.notify();
    }

    /// Writes the shown model of a data source to the cache.
    fn store_model(&self, name: &str, cx: &mut Context<Self>) {
        let (Some(root), Some(live)) = (self.root.clone(), self.tree.lives.get(name)) else {
            return;
        };
        if let Some(cache) = crate::store(cx).model_cache(&root) {
            let model = live.model.clone();
            cx.background_spawn(async move {
                let _ = cache.store(&model);
            })
            .detach();
        }
    }

    // ---- connections and the model ----------------------------------------------

    fn deactivate(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.selected_source_name() else {
            return;
        };
        if let Some(live) = self.tree.lives.get(&name) {
            live.handle.disconnect(TxPolicy::Rollback);
        }
        cx.notify();
    }

    fn force_refresh(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.selected_source_name() else {
            return;
        };
        let Some(live) = self.tree.lives.get_mut(&name) else {
            self.refresh_selected(cx);
            return;
        };
        if let Some(job) = live.refresh.take() {
            live.handle.cancel_job(job);
        }
        live.model = live.handle.model().set(Model::empty(live.id().clone()));
        live.has_model = false;
        live.refresh();
        self.rebuild_rows();
        cx.notify();
    }

    fn forget_cached_schemas(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.selected_source_name() else {
            return;
        };
        if let Some(live) = self.tree.lives.get_mut(&name) {
            if let Some(job) = live.refresh.take() {
                live.handle.cancel_job(job);
            }
            live.queued.clear();
            live.model = live.handle.model().set(Model::empty(live.id().clone()));
            live.has_model = false;
            live.error = None;
        }
        if let Some(root) = self.root.clone()
            && let Some(cache) = crate::store(cx).model_cache(&root)
        {
            let id = db_core::DataSourceId::from(name.as_str());
            cx.background_spawn(async move {
                let _ = cache.remove(&id);
            })
            .detach();
        }
        self.rebuild_rows();
        cx.notify();
    }

    // ---- what changes the database -------------------------------------------

    /// Runs `statements` on a new session of the data source (auto-commit)
    /// and reads `schema` (or the scope) again when they are done.
    fn run_statements(
        &mut self,
        source: &str,
        statements: Vec<String>,
        after: After,
        cx: &mut Context<Self>,
    ) {
        if statements.is_empty() {
            return;
        }
        let Some(live) = self.tree.lives.get(source) else {
            return;
        };
        let session = live.handle.open_session(SessionOptions {
            tx_mode: Some(db_core::TxMode::Auto),
            schema: None,
        });
        let job = session.execute(Job::script(statements.join("\n")));
        self.tree.ddl.push(DdlRun {
            source: source.to_owned(),
            session,
            job,
            after,
            error: None,
        });
        cx.notify();
    }

    /// An event of a statement job of the tree's actions.
    pub(super) fn on_job_event(&mut self, job: JobId, event: JobEvent, cx: &mut Context<Self>) {
        let Some(at) = self.tree.ddl.iter().position(|r| r.job == job) else {
            return;
        };
        match event {
            JobEvent::Failed { error, .. } => {
                self.tree.ddl[at].error = Some(error.error.message.clone());
            }
            JobEvent::Done { outcome } => {
                let run = self.tree.ddl.remove(at);
                run.session.close(TxPolicy::Commit);
                let failed = run.error.is_some() || outcome != JobOutcome::Completed;
                if failed {
                    let text = run
                        .error
                        .unwrap_or_else(|| "The statements were cancelled.".to_owned());
                    self.notify(
                        Notification::new(NotificationKind::Error, run.source.clone(), text),
                        cx,
                    );
                }
                if let Some(live) = self.tree.lives.get_mut(&run.source) {
                    match &run.after {
                        After::Schema(schema) => live.refresh_schema(schema),
                        After::Scope => live.refresh(),
                        After::Nothing => {}
                    }
                }
                self.rebuild_rows();
                cx.notify();
            }
            _ => {}
        }
    }

    /// The schema a row's node is in (what is read again after a change).
    fn schema_of(&self, key: &RowKey) -> Option<String> {
        let id = match key.node {
            NodeKey::Node(id) | NodeKey::Group(id, _) => id,
            _ => return None,
        };
        let node = self.model_of(&key.source)?.node(id)?;
        // A dropped or renamed schema: the whole scope.
        (node.kind() != NodeKind::Schema).then(|| node.schema().name().to_owned())
    }

    fn rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(key) = self.target_key() else {
            return;
        };
        let this = cx.entity().downgrade();
        match key.node {
            NodeKey::Source => {
                let old = key.source.to_string();
                let message = format!("New name of data source \u{201c}{old}\u{201d}:");
                sql_dialog::open(
                    RENAME_TITLE,
                    message,
                    Ask::Rename {
                        old: old.clone(),
                        statements: None,
                    },
                    window,
                    cx,
                    move |answer, cx| {
                        if let Some(this) = this.upgrade() {
                            this.update(cx, |this, cx| this.rename_source(&old, &answer.name, cx));
                        }
                    },
                );
            }
            NodeKey::Node(id) => {
                let Some(model) = self.model_arc(&key.source) else {
                    return;
                };
                let Some(node) = model.node(id) else {
                    return;
                };
                let dialect = self.dialect_of(&key.source);
                if ddl::rename_statement(dialect, node, "x").is_none() {
                    return;
                }
                let message = format!(
                    "New name of {} \u{201c}{}\u{201d}:",
                    ddl::kind_word(node.kind()),
                    node.name()
                );
                let old = node.name().to_owned();
                let make_model = model.clone();
                let statements = move |to: &str| -> Vec<String> {
                    make_model
                        .node(id)
                        .and_then(|n| ddl::rename_statement(dialect, n, to))
                        .into_iter()
                        .collect()
                };
                let schema = self.schema_of(&key);
                let source = key.source.to_string();
                sql_dialog::open(
                    RENAME_TITLE,
                    message,
                    Ask::Rename {
                        old,
                        statements: Some(Box::new(statements)),
                    },
                    window,
                    cx,
                    move |answer, cx| {
                        if let Some(this) = this.upgrade() {
                            this.update(cx, |this, cx| {
                                this.run_statements(
                                    &source,
                                    answer.statements,
                                    After::of(schema),
                                    cx,
                                )
                            });
                        }
                    },
                );
            }
            _ => {}
        }
    }

    fn rename_source(&mut self, old: &str, new: &str, cx: &mut Context<Self>) {
        let new = new.trim();
        if new.is_empty() || new == old || self.sources().iter().any(|s| s.name == new) {
            return;
        }
        let mut sources = self.sources().to_vec();
        let Some(source) = sources.iter_mut().find(|s| s.name == old) else {
            return;
        };
        source.name = new.to_owned();
        // The tree keeps the data source open and selected under its new
        // name.
        let reopen = self.tree.expanded.contains(&RowKey::source(&old.into()));
        self.tree.lives.remove(old);
        self.tree.expanded.retain(|k| *k.source != *old);
        if reopen {
            self.tree.expanded.insert(RowKey::source(&new.into()));
        }
        self.selected = Some(new.to_owned());
        self.tree.selected = Some(RowKey::source(&new.into()));
        self.sources = Some(sources.clone());
        self.sources_changed();
        self.save_sources(
            SaveRequest {
                sources,
                renamed: vec![(old.to_owned(), new.to_owned())],
                ..SaveRequest::default()
            },
            cx,
        );
        cx.notify();
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(key) = self.target_key() else {
            return;
        };
        match key.node {
            NodeKey::Source => self.ask_remove(&key.source, window, cx),
            NodeKey::Node(_) | NodeKey::Group(..) => self.ask_drop(key, window, cx),
            _ => {}
        }
    }

    fn ask_remove(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let message = format!(
            "The data source \u{201c}{name}\u{201d} and its saved password will be removed from the project."
        );
        let dialog = cx.new(|cx| {
            Dialog::new(
                REMOVE_TITLE,
                message,
                vec![
                    DialogButton {
                        id: "cancel",
                        label: "Cancel".into(),
                    },
                    DialogButton {
                        id: "remove",
                        label: "Remove".into(),
                    },
                ],
                cx,
            )
            .with_icon(IconName::QUESTION_DIALOG)
        });
        let source = name.to_owned();
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            let DialogEvent::Closed { button, .. } = event;
            let Some(open) = this.tree.confirm.take() else {
                return;
            };
            window.focus(&this.focus);
            if *button == Some("remove") {
                this.remove_source(&open.source, cx);
            }
            cx.notify();
        });
        window.focus(&gpui::Focusable::focus_handle(dialog.read(cx), cx));
        self.tree.confirm = Some(Confirm {
            dialog,
            source,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn remove_source(&mut self, name: &str, cx: &mut Context<Self>) {
        let sources: Vec<DataSource> = self
            .sources()
            .iter()
            .filter(|s| s.name != name)
            .cloned()
            .collect();
        self.sources = Some(sources.clone());
        if self.selected.as_deref() == Some(name) {
            self.selected = None;
        }
        self.sources_changed();
        self.save_sources(
            SaveRequest {
                sources,
                removed: vec![name.to_owned()],
                ..SaveRequest::default()
            },
            cx,
        );
        cx.notify();
    }

    fn ask_drop(&mut self, key: RowKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some(model) = self.model_arc(&key.source) else {
            return;
        };
        let dialect = self.dialect_of(&key.source);
        let nodes = Self::row_nodes(&model, &key);
        if nodes.is_empty()
            || nodes
                .iter()
                .any(|n| ddl::drop_statement(dialect, *n, DropOptions::default()).is_none())
        {
            return;
        }
        let message = match nodes.as_slice() {
            [one] => format!(
                "Drop {} \u{201c}{}\u{201d}?",
                ddl::kind_word(one.kind()),
                one.name()
            ),
            many => format!("Drop {} objects?", many.len()),
        };
        let ids: Vec<_> = nodes.iter().map(|n| n.id()).collect();
        let make_model = model.clone();
        let statements = move |options: DropOptions| -> Vec<String> {
            ids.iter()
                .filter_map(|id| make_model.node(*id))
                .filter_map(|n| ddl::drop_statement(dialect, n, options))
                .collect()
        };
        let schema = self.schema_of(&key);
        let source = key.source.to_string();
        let this = cx.entity().downgrade();
        sql_dialog::open(
            DROP_TITLE,
            message,
            Ask::Drop(Box::new(statements)),
            window,
            cx,
            move |answer, cx| {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| {
                        this.run_statements(&source, answer.statements, After::of(schema), cx)
                    });
                }
            },
        );
    }

    fn truncate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(key) = self.target_key() else {
            return;
        };
        let NodeKey::Node(id) = key.node else {
            return;
        };
        let dialect = self.dialect_of(&key.source);
        let Some(node) = self
            .model_of(&key.source)
            .and_then(|m| m.node(id))
            .filter(|n| n.kind() == NodeKind::Table)
        else {
            return;
        };
        let statements = vec![ddl::truncate_statement(dialect, node)];
        let message = format!("Delete every row of table \u{201c}{}\u{201d}?", node.name());
        let source = key.source.to_string();
        let this = cx.entity().downgrade();
        sql_dialog::open(
            TRUNCATE_TITLE,
            message,
            Ask::Confirm(statements),
            window,
            cx,
            move |answer, cx| {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| {
                        // Truncating changes no metadata.
                        this.run_statements(&source, answer.statements, After::Nothing, cx)
                    });
                }
            },
        );
    }

    /// The Quick Documentation popup over everything, closed by a click
    /// outside it or Escape.
    pub(super) fn render_doc(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        use gpui::{
            FontWeight, InteractiveElement as _, IntoElement as _, MouseButton, ParentElement as _,
            StatefulInteractiveElement as _, Styled as _, anchored, deferred, div, px,
        };
        let doc = self.tree.doc.as_ref()?;
        let theme = cx.theme();
        let size = window.viewport_size();
        let popup = div()
            .id("database-doc")
            .debug_selector(|| "database-doc".into())
            .occlude()
            .max_w(px(560.0))
            .max_h(px(420.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(12.0))
            .rounded(theme.metric(Metric::CornerRadius) * 2.0)
            .border_1()
            .border_color(theme.color(themes::UiRole::PopupBorder))
            .bg(theme.color(themes::UiRole::SurfaceRaised))
            .text_color(theme.color(themes::UiRole::Text))
            .text_size(theme.ui_size())
            .font_family(theme.ui_family())
            .shadow_lg()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(doc.title.clone()),
            )
            .children(doc.lines.iter().map(|l| {
                div()
                    .text_color(theme.color(themes::UiRole::TextMuted))
                    .child(l.clone())
            }))
            .children(doc.code.clone().map(|code| {
                div()
                    .mt(px(4.0))
                    .font_family(theme.mono_family())
                    .text_size(theme.editor_size())
                    .children(code.lines().map(|l| {
                        div().whitespace_nowrap().child(if l.is_empty() {
                            " ".to_owned()
                        } else {
                            l.to_owned()
                        })
                    }))
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        Some(
            deferred(
                anchored()
                    .position(point(gpui::px(0.0), gpui::px(0.0)))
                    .child(
                        div()
                            .id("database-doc-backdrop")
                            .w(size.width)
                            .h(size.height)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.tree.doc = None;
                                    cx.notify();
                                }),
                            )
                            .child(
                                anchored()
                                    .position(doc.position)
                                    .snap_to_window_with_margin(px(4.0))
                                    .child(popup),
                            ),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// The state of the actions for `db.state`.
    pub(super) fn actions_json(&self, cx: &App) -> Json {
        json!({
            "doc": self.tree.doc.as_ref().map(|d| json!({
                "title": d.title,
                "lines": d.lines,
                "code": d.code,
            })),
            "confirm": self.tree.confirm.as_ref().map(|c| c.dialog.read(cx).state_json()),
            "running": self.tree.ddl.len(),
        })
    }
}

/// Changes a profile's GUI options.
fn set_gui(source: &mut DataSource, f: impl FnOnce(&mut db_core::GuiOptions)) {
    let mut gui = source.profile.gui_options();
    f(&mut gui);
    source.profile.gui = (!gui.is_default()).then_some(gui);
}
