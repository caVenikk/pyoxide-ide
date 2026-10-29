//! The toolbar above the editor of a `.sql` file (ADR 0022): the SQL
//! dialect of the file and the data source and schema it is attached to,
//! with a popup to change them, and a banner while the file has no
//! dialect. A choice is a mapping of the file, its folder or the project,
//! kept in the personal project state (or the shared settings on request),
//! and every open editor of the files it reaches is coloured again at
//! once.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Corner, CursorStyle, DismissEvent,
    Entity, Global, InteractiveElement, IntoElement, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, WeakEntity, Window, canvas, div,
    prelude::*, px,
};
use serde_json::{Value as Json, json};
use sql_syntax::Dialect;
use themes::UiRole;
use ui::command::CommandError;
use ui::components::Tooltip;
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;
use workbench::{EditorHeader, Workbench, WorkbenchEvent};

use super::hub::{Hub, Kind, origin_text};
use super::settings::{self, Resolved};
use crate::icons;

ui::ui_commands! {
    COMMANDS, scope = "Database";
    /// Open the dialect and scope popup.
    ChooseDialect => "sql.choose", "Change SQL Dialect and Data Source",
        "Opens the SQL popup of the file's toolbar: the dialects, and the data sources the file can be attached to.";
    /// Set the dialect of a file, folder or the project.
    SetDialect { dialect: String, target: Option<String>, shared: Option<bool> } => "sql.set-dialect", "Set SQL Dialect",
        "Maps the active SQL file, its folder (`target`: `folder`) or the whole project (`project`) to a dialect (`generic`, `postgres`, `oracle`, `sqlite`; `default` removes the mapping). The mapping is personal unless `shared` is true, which writes it to the project's `.oxide/project.toml`. Open files are coloured again.",
        r#"{"dialect": "postgres", "target": "folder"}"#;
    /// Attach a data source to a file, folder or the project.
    AttachDataSource { name: Option<String>, schema: Option<String>, target: Option<String>, shared: Option<bool> } => "sql.attach-data-source", "Attach Data Source",
        "Attaches the active SQL file, its folder (`target`: `folder`) or the whole project (`project`) to the data source `name` and the optional `schema`; without `name` the attachment is removed. Names in the file then mean what that data source's metadata says. Personal unless `shared` is true.",
        r#"{"name": "local", "schema": "public"}"#;
    /// Connect the data source the active file is attached to.
    ConnectDataSource => "sql.connect", "Connect Data Source",
        "Connects the data source the active SQL file is attached to, for its table and column names: the \"Connect\" link of the hint above the editor, which shows while the data source has no metadata and needs a password. Asks for the password first when none is kept; nothing when the toolbar shows no hint.";
    /// Describe the toolbar.
    SqlState => "sql.state", "SQL State",
        "Returns the SQL toolbar's state as JSON: whether it shows, its chips and banner, the active file's dialect and scope with where each came from, the data sources it can attach, and the open popup.";
}

/// The product-wide settings of the SQL editor glue, in a global so that
/// observers see every change.
pub(crate) struct SqlGlobal(pub Arc<Hub>);

impl Global for SqlGlobal {}

/// The name of a dialect as the toolbar shows it.
pub fn dialect_title(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Generic => "Generic SQL",
        other => other.display_name(),
    }
}

/// Whether the file is edited as SQL.
pub fn is_sql_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("sql"))
}

#[derive(Default)]
struct Widgets(Vec<(AnyWindowHandle, WeakEntity<SqlWidget>)>);

impl Global for Widgets {}

fn widget_of(window: AnyWindowHandle, cx: &App) -> Option<Entity<SqlWidget>> {
    cx.try_global::<Widgets>()?
        .0
        .iter()
        .filter(|(w, _)| *w == window)
        .find_map(|(_, v)| v.upgrade())
}

fn target(cx: &App) -> Option<(AnyWindowHandle, Entity<SqlWidget>)> {
    let widgets = cx.try_global::<Widgets>()?;
    let live: Vec<_> = widgets
        .0
        .iter()
        .filter_map(|(w, v)| v.upgrade().map(|v| (*w, v)))
        .collect();
    let active = cx.active_window();
    live.iter()
        .find(|(w, _)| Some(*w) == active)
        .cloned()
        .or_else(|| live.last().cloned())
}

pub(crate) fn init(cx: &mut App, hub: Arc<Hub>) {
    // A data source without a cached model that code intelligence asks
    // for is loaded by the Database tool window of its project.
    let (wanted, mut requests) = futures::channel::mpsc::unbounded::<(PathBuf, String)>();
    hub.on_model_absent(move |root, source| {
        let _ = wanted.unbounded_send((root.to_owned(), source.to_owned()));
    });
    cx.spawn(async move |cx| {
        while let Some((root, source)) = requests.next().await {
            let _ = cx.update(|cx| crate::explorer::request_model(&root, &source, cx));
        }
    })
    .detach();
    cx.set_global(SqlGlobal(hub));
    ui::command::register(cx, COMMANDS);
    cx.on_action(|_: &ConnectDataSource, cx| {
        let Some((window, widget)) = target(cx) else {
            return;
        };
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                widget.update(cx, |w, cx| w.connect(window, cx));
            });
        });
    });
    cx.on_action(|_: &ChooseDialect, cx| {
        let Some((window, widget)) = target(cx) else {
            return;
        };
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                widget.update(cx, |w, cx| w.open(ChipId::Source, window, cx));
            });
        });
    });
    cx.on_action(|a: &SetDialect, cx| {
        let Some((_, widget)) = target(cx) else {
            return;
        };
        let dialect = match a.dialect.as_str() {
            "default" | "none" => None,
            id => match Dialect::from_id(id) {
                Some(d) => Some(d),
                None => {
                    eprintln!("sql.set-dialect: unknown dialect `{id}`");
                    return;
                }
            },
        };
        let (target_of, kind) = (Target::parse(a.target.as_deref()), kind_of(a.shared));
        widget.update(cx, |w, cx| {
            w.apply(target_of, kind, Change::Dialect(dialect), cx)
        });
    });
    cx.on_action(|a: &AttachDataSource, cx| {
        let Some((_, widget)) = target(cx) else {
            return;
        };
        let (target_of, kind) = (Target::parse(a.target.as_deref()), kind_of(a.shared));
        let change = Change::Scope(a.name.clone().map(|n| (n, a.schema.clone())));
        widget.update(cx, |w, cx| w.apply(target_of, kind, change, cx));
    });
    ui::command::register_query(
        cx,
        "sql.state",
        Rc::new(|_, window, cx| {
            let widget = widget_of(window.window_handle(), cx)
                .ok_or_else(|| CommandError::new("not_available", "no SQL widget"))?;
            Ok(widget.read(cx).state_json(cx))
        }),
    );
}

fn kind_of(shared: Option<bool>) -> Kind {
    if shared.unwrap_or(false) {
        Kind::Shared
    } else {
        Kind::Personal
    }
}

/// What a mapping covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    File,
    Folder,
    Project,
}

impl Target {
    fn parse(text: Option<&str>) -> Self {
        match text {
            Some("folder") => Target::Folder,
            Some("project") => Target::Project,
            _ => Target::File,
        }
    }
}

enum Change {
    Dialect(Option<Dialect>),
    Scope(Option<(String, Option<String>)>),
}

/// The widget of one main window.
pub(crate) struct SqlWidget {
    hub: Arc<Hub>,
    workbench: WeakEntity<Workbench>,
    root: Option<PathBuf>,
    active_file: Option<PathBuf>,
    /// The dialects the open SQL editors were last coloured with.
    applied: HashMap<PathBuf, (Dialect, u64)>,
    menu: Option<(Entity<ContextMenu>, Subscription)>,
    /// Where each chip was drawn; the popup opens below the one it was
    /// opened from.
    dialect_bounds: Rc<Cell<Bounds<Pixels>>>,
    schema_bounds: Rc<Cell<Bounds<Pixels>>>,
    source_bounds: Rc<Cell<Bounds<Pixels>>>,
    popup_anchor: Rc<Cell<Bounds<Pixels>>>,
    /// Files whose "dialect is not configured" banner was cancelled.
    dismissed: HashSet<PathBuf>,
    load: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl SqlWidget {
    fn new(workbench: WeakEntity<Workbench>, cx: &mut Context<Self>) -> Self {
        let hub = cx.global::<SqlGlobal>().0.clone();
        let mut subscriptions = vec![cx.observe_global::<SqlGlobal>(|this, cx| this.changed(cx))];
        let (mut root, mut active_file) = (None, None);
        if let Some(wb) = workbench.upgrade() {
            root = wb.read(cx).project_root(cx);
            active_file = wb.read(cx).active_file(cx);
            subscriptions.push(cx.subscribe(
                &wb,
                |this, _, event: &WorkbenchEvent, cx| match event {
                    WorkbenchEvent::ProjectOpened(root) => this.set_root(Some(root.clone()), cx),
                    WorkbenchEvent::ActiveFileChanged(file) => {
                        this.active_file = file.clone();
                        cx.notify();
                    }
                    _ => {}
                },
            ));
        }
        let mut this = Self {
            hub,
            workbench,
            root: None,
            active_file,
            applied: HashMap::new(),
            menu: None,
            dialect_bounds: Rc::default(),
            schema_bounds: Rc::default(),
            source_bounds: Rc::default(),
            popup_anchor: Rc::default(),
            dismissed: HashSet::new(),
            load: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.set_root(root, cx);
        this
    }

    /// Another project: read its settings and data sources off the UI
    /// thread, then colour the open files again.
    fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        self.root = root.clone();
        self.menu = None;
        cx.notify();
        let Some(root) = root else {
            return;
        };
        let hub = self.hub.clone();
        let read = cx.background_spawn({
            let root = root.clone();
            async move { hub.read_project(&root) }
        });
        self.load = cx.spawn(async move |this, cx| {
            let project = read.await;
            let _ = this.update(cx, |this, cx| {
                let hub = this.hub.clone();
                cx.update_global::<SqlGlobal, _>(|_, _| hub.install(&root, project));
            });
        });
    }

    /// The hub changed: the widget shows the new answer and the editors
    /// whose dialect changed are coloured again.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.refresh_editors(cx);
        cx.notify();
        // The data source a file is attached to is in its tab, its row of
        // the project tree and the toolbar title.
        if let Some(workbench) = self.workbench.upgrade() {
            workbench.update(cx, |workbench, cx| {
                workbench.panel().update(cx, |_, cx| cx.notify());
                cx.notify();
            });
        }
    }

    fn refresh_editors(&mut self, cx: &mut Context<Self>) {
        let Some(workbench) = self.workbench.upgrade() else {
            return;
        };
        let epoch = self.hub.epoch();
        let paths: Vec<PathBuf> = workbench
            .read(cx)
            .tab_paths()
            .into_iter()
            .filter(|p| is_sql_file(p))
            .collect();
        self.applied.retain(|p, _| paths.contains(p));
        for path in paths {
            let dialect = self.hub.dialect(&path);
            let known = self.applied.get(&path).map(|(d, _)| *d);
            self.applied.insert(path.clone(), (dialect, epoch));
            if known == Some(dialect) {
                continue;
            }
            if let Some(editor) = workbench.read(cx).editor_for(&path) {
                editor.update(cx, |e, cx| e.reset_highlighting(cx));
            }
        }
    }

    fn visible_file(&self) -> Option<&Path> {
        self.active_file.as_deref().filter(|f| is_sql_file(f))
    }

    fn resolved(&self) -> Option<Resolved> {
        self.visible_file().map(|f| self.hub.resolve(f))
    }

    /// The chips of the toolbar, left to right: a file attached to a data
    /// source shows its schema (as `database.schema`) and the data source;
    /// another file its dialect and the placeholder of the data source.
    fn chips(&self, resolved: &Resolved) -> Vec<Chip> {
        let attached = resolved.data_source.as_ref().map(|name| {
            let source = self
                .visible_file()
                .and_then(|f| self.hub.source_of(f, name));
            let icon = match source.as_ref().map(|s| s.dialect) {
                Some(Dialect::Postgres) => icons::POSTGRES,
                Some(Dialect::Oracle) => icons::ORACLE,
                Some(Dialect::Sqlite) => icons::SQLITE,
                _ => icons::DATA_SOURCE,
            };
            let database = source
                .and_then(|s| s.database)
                .unwrap_or_else(|| name.clone());
            (name.clone(), icon, database)
        });
        match attached {
            Some((name, icon, database)) => {
                let schema = resolved.schema.as_ref().map(|schema| Chip {
                    id: ChipId::Schema,
                    icon: Some(icons::SCHEMA),
                    text: format!("{database}.{schema}"),
                });
                // A console is bound to its data source: no chip for it.
                let source = (!matches!(resolved.scope_origin, settings::Origin::Console))
                    .then_some(Chip {
                        id: ChipId::Source,
                        icon: Some(icon),
                        text: name,
                    });
                schema.into_iter().chain(source).collect()
            }
            None => vec![
                Chip {
                    id: ChipId::Dialect,
                    icon: None,
                    text: dialect_title(resolved.dialect).to_owned(),
                },
                Chip {
                    id: ChipId::Source,
                    icon: Some(icons::DATA_SOURCE),
                    text: "<data source>".to_owned(),
                },
            ],
        }
    }

    /// The banner shows while no mapping, default or data source gives the
    /// file a dialect, until it is cancelled.
    fn banner_shows(&self, resolved: &Resolved) -> bool {
        matches!(resolved.dialect_origin, settings::Origin::None)
            && self
                .visible_file()
                .is_some_and(|f| !self.dismissed.contains(f))
    }

    /// The data source whose metadata the file's names would resolve
    /// through, when only a connection with a typed password can bring it:
    /// the hint offers to connect.
    fn connect_hint(&self, resolved: &Resolved) -> Option<String> {
        let file = self.visible_file()?;
        let source = resolved.data_source.as_ref()?;
        let root = self.hub.root_of(file).or_else(|| self.root.clone())?;
        self.hub.connect_hint(&root, source).then(|| source.clone())
    }

    /// The hint's "Connect" link: connects the data source as the Database
    /// tool window does, asking for the password first.
    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.resolved().and_then(|r| self.connect_hint(&r)) else {
            return;
        };
        if let Some(explorer) = crate::explorer::explorer_of(window.window_handle(), cx) {
            explorer.update(cx, |e, cx| e.connect_for_sql(&name, cx));
        }
    }

    fn tooltip(resolved: &Resolved) -> String {
        let mut text = format!(
            "SQL dialect: {} ({})",
            dialect_title(resolved.dialect),
            origin_text(&resolved.dialect_origin)
        );
        match &resolved.data_source {
            Some(ds) => text.push_str(&format!(
                "\nData source: {ds} ({})",
                origin_text(&resolved.scope_origin)
            )),
            None => text.push_str("\nNo data source: names are not resolved"),
        }
        text
    }

    fn open(&mut self, chip: ChipId, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            return;
        }
        let anchor = match chip {
            ChipId::Dialect => &self.dialect_bounds,
            ChipId::Schema => &self.schema_bounds,
            ChipId::Source => &self.source_bounds,
        };
        self.popup_anchor.set(anchor.get());
        let (Some(root), Some(file)) = (self.root.clone(), self.visible_file()) else {
            return;
        };
        let resolved = self.hub.resolve(file);
        let sources = self.hub.source_names(&root);
        let (items, selected) = popup_items(&resolved, &sources, self.file_in_root());
        let options = PopupOptions {
            title: Some("SQL Dialect and Data Source".into()),
            speed_search: true,
            selected,
            ..PopupOptions::default()
        };
        let menu = ContextMenu::popup(items, options, window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, subscription));
        cx.notify();
    }

    /// The active file lies in a subfolder of the project (a folder
    /// mapping differs from a project one).
    fn file_in_root(&self) -> bool {
        match (&self.root, &self.active_file) {
            (Some(root), Some(file)) => file.parent().is_some_and(|p| p != root),
            _ => false,
        }
    }

    /// Writes a mapping off the UI thread and tells the observers.
    fn apply(&mut self, target: Target, kind: Kind, change: Change, cx: &mut Context<Self>) {
        let (Some(root), Some(file)) = (self.root.clone(), self.active_file.clone()) else {
            return;
        };
        let relative = match target {
            Target::Project => None,
            Target::File => settings::relative(&root, &file),
            Target::Folder => file
                .parent()
                .and_then(|folder| settings::relative(&root, folder)),
        };
        let hub = self.hub.clone();
        let write = cx.background_spawn({
            let root = root.clone();
            async move {
                // Data sources may have changed since the project opened.
                if hub.project(&root).is_none_or(|p| p.sources.is_empty()) {
                    let sources = hub.load_sources(&root);
                    hub.set_sources(&root, sources);
                }
                hub.update(&root, kind, |s| match change {
                    Change::Dialect(d) => s.set_dialect(relative.as_deref(), d),
                    Change::Scope(scope) => s.set_scope(
                        relative.as_deref(),
                        scope
                            .as_ref()
                            .map(|(n, schema)| (n.as_str(), schema.as_deref())),
                    ),
                })
            }
        });
        self.menu = None;
        self.load = cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |_, cx| {
                match result {
                    Ok(()) => {
                        // The hub already holds the change; observers learn of it.
                        cx.update_global::<SqlGlobal, _>(|_, _| {});
                    }
                    Err(e) => eprintln!("sql: could not save the mapping: {e}"),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn state_json(&self, cx: &App) -> Json {
        let resolved = self.resolved();
        let chips = resolved.as_ref().map(|r| self.chips(r)).unwrap_or_default();
        let text_of = |id: ChipId| chips.iter().find(|c| c.id == id).map(|c| c.text.clone());
        json!({
            "visible": resolved.is_some(),
            "file": self.visible_file().map(|f| f.display().to_string()),
            "banner": resolved.as_ref().is_some_and(|r| self.banner_shows(r)),
            "connect-hint": resolved.as_ref().and_then(|r| self.connect_hint(r)),
            "chips": chips.iter().map(|c| c.text.clone()).collect::<Vec<_>>(),
            "text": text_of(ChipId::Dialect),
            "schema-text": text_of(ChipId::Schema),
            "data-source-text": text_of(ChipId::Source),
            "dialect": resolved.as_ref().map(|r| r.dialect.id()),
            "dialect-origin": resolved.as_ref().map(|r| origin_json(&r.dialect_origin)),
            "data-source": resolved.as_ref().and_then(|r| r.data_source.clone()),
            "schema": resolved.as_ref().and_then(|r| r.schema.clone()),
            "scope-origin": resolved.as_ref().map(|r| origin_json(&r.scope_origin)),
            "tooltip": resolved.as_ref().map(Self::tooltip),
            "sources": self
                .root
                .as_deref()
                .map(|r| self.hub.source_names(r))
                .unwrap_or_default(),
            "popup": self.menu.as_ref().map(|(m, _)| m.read(cx).state_json()),
        })
    }
}

fn origin_json(origin: &settings::Origin) -> Json {
    use settings::Origin;
    match origin {
        Origin::Mapping { path, personal } => {
            json!({"kind": "mapping", "path": path, "personal": personal})
        }
        Origin::Project => json!({"kind": "project"}),
        Origin::DataSource(name) => json!({"kind": "data-source", "name": name}),
        Origin::Console => json!({"kind": "console"}),
        Origin::None => json!({"kind": "none"}),
    }
}

/// The popup's rows and the preselected one: the dialects for the file,
/// submenus to give them to the folder or the project, and the data sources.
fn popup_items(
    resolved: &Resolved,
    sources: &[String],
    in_folder: bool,
) -> (Vec<MenuItem>, Option<usize>) {
    let mut items = Vec::new();
    let mut selected = None;
    let action = |dialect: Dialect, target: &str, shared: bool| SetDialect {
        dialect: dialect.id().to_owned(),
        target: Some(target.to_owned()),
        shared: Some(shared),
    };
    for dialect in Dialect::ALL {
        if dialect == resolved.dialect {
            selected = Some(items.len());
        }
        items.push(
            MenuItem::action(dialect_title(dialect), action(dialect, "file", false))
                .bold(dialect == resolved.dialect),
        );
    }
    items.push(MenuItem::separator());
    let group = |target: &str, shared: bool| {
        Dialect::ALL
            .into_iter()
            .map(|d| MenuItem::action(dialect_title(d), action(d, target, shared)))
            .collect::<Vec<_>>()
    };
    if in_folder {
        items.push(MenuItem::submenu(
            "Use for the Folder",
            group("folder", false),
        ));
    }
    items.push(MenuItem::submenu(
        "Use for the Project",
        group("project", false),
    ));
    items.push(MenuItem::submenu(
        "Use for the Project (Shared)",
        group("project", true),
    ));
    items.push(MenuItem::separator());
    let attach = |name: &str| AttachDataSource {
        name: Some(name.to_owned()),
        schema: None,
        target: Some("file".to_owned()),
        shared: Some(false),
    };
    let mut data_sources: Vec<MenuItem> = sources
        .iter()
        .map(|name| {
            MenuItem::action(name.clone(), attach(name))
                .bold(resolved.data_source.as_deref() == Some(name))
        })
        .collect();
    if data_sources.is_empty() {
        data_sources.push(MenuItem::unavailable("No data sources"));
    }
    items.push(MenuItem::submenu("Attach Data Source", data_sources));
    items.push(
        MenuItem::action(
            "Detach Data Source",
            AttachDataSource {
                name: None,
                schema: None,
                target: Some("file".to_owned()),
                shared: Some(false),
            },
        )
        .enabled(resolved.data_source.is_some()),
    );
    (items, selected)
}

pub(crate) fn editor_header() -> EditorHeader {
    EditorHeader {
        id: "sql-toolbar",
        create: Rc::new(|workbench, window, cx| {
            let view = cx.new(|cx| SqlWidget::new(workbench, cx));
            let handle = window.window_handle();
            let widgets = cx.default_global::<Widgets>();
            widgets.0.retain(|(_, v)| v.upgrade().is_some());
            widgets.0.push((handle, view.downgrade()));
            view.into()
        }),
    }
}

/// Which chip of the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChipId {
    Dialect,
    Schema,
    Source,
}

struct Chip {
    id: ChipId,
    icon: Option<IconName>,
    text: String,
}

const TOOLBAR_HEIGHT: f32 = 36.0;
const BANNER_HEIGHT: f32 = 36.0;
const CHIP_TEXT: f32 = 12.0;

impl SqlWidget {
    fn render_chip(
        &self,
        chip: Chip,
        tooltip: SharedString,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.theme();
        let hover = theme.color(UiRole::Hover);
        let bounds = match chip.id {
            ChipId::Dialect => self.dialect_bounds.clone(),
            ChipId::Schema => self.schema_bounds.clone(),
            ChipId::Source => self.source_bounds.clone(),
        };
        let id = chip.id;
        let selector = match id {
            ChipId::Dialect => "sql-chip-dialect",
            ChipId::Schema => "sql-chip-schema",
            ChipId::Source => "sql-chip-source",
        };
        div()
            .id(selector)
            .debug_selector(|| selector.into())
            .relative()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .h(px(24.0))
            .px(px(6.0))
            .rounded(px(4.0))
            .text_size(px(CHIP_TEXT))
            .whitespace_nowrap()
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(hover))
            .children(chip.icon.map(|i| Icon::new(i).size(px(16.0))))
            .child(SharedString::from(chip.text))
            .child(Icon::new(IconName::CHEVRON_DOWN).size(px(12.0)))
            .tooltip(Tooltip::text(tooltip, None))
            .on_click(cx.listener(move |this, _, window, cx| this.open(id, window, cx)))
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .into_any_element()
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let link = theme.color(UiRole::TextLink);
        let link = |id: &'static str, text: &'static str| {
            div()
                .id(id)
                .debug_selector(|| id.into())
                .flex_none()
                .whitespace_nowrap()
                .text_color(link)
                .cursor(CursorStyle::PointingHand)
                .child(text)
        };
        div()
            .id("sql-banner")
            .debug_selector(|| "sql-banner".into())
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .h(px(BANNER_HEIGHT))
            .mx(px(4.0))
            .mt(px(4.0))
            .px(px(12.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.color(UiRole::Warning).opacity(0.35))
            .bg(theme.color(UiRole::WarningBg))
            .child(Icon::new(IconName::WARNING).size(px(16.0)))
            .child(
                div()
                    .flex_1()
                    .whitespace_nowrap()
                    .child("SQL dialect is not configured."),
            )
            .child(link("sql-banner-change", "Change dialect to...").on_click(
                cx.listener(|this, _, window, cx| this.open(ChipId::Dialect, window, cx)),
            ))
            .child(
                link("sql-banner-cancel", "Cancel").on_click(cx.listener(|this, _, _, cx| {
                    if let Some(file) = this.active_file.clone() {
                        this.dismissed.insert(file);
                    }
                    cx.notify();
                })),
            )
            .into_any_element()
    }
}

impl SqlWidget {
    /// The hint to connect a data source that has no metadata yet and
    /// needs a password: the names of its tables and columns are not
    /// known until it is connected.
    fn render_connect_hint(&self, source: &str, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        div()
            .id("sql-connect-hint")
            .debug_selector(|| "sql-connect-hint".into())
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .h(px(BANNER_HEIGHT))
            .mx(px(4.0))
            .mt(px(4.0))
            .px(px(12.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.color(UiRole::Info).opacity(0.35))
            .bg(theme.color(UiRole::InfoBg))
            .child(Icon::new(IconName::INFO).size(px(16.0)))
            .child(div().flex_1().whitespace_nowrap().child(format!(
                "Connect to \u{201c}{source}\u{201d} to complete its table and column names."
            )))
            .child(
                div()
                    .id("sql-connect-link")
                    .debug_selector(|| "sql-connect-link".into())
                    .flex_none()
                    .whitespace_nowrap()
                    .text_color(theme.color(UiRole::TextLink))
                    .cursor(CursorStyle::PointingHand)
                    .child("Connect")
                    .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))),
            )
            .into_any_element()
    }

    /// Execute and Stop at the left of a console's toolbar, and its
    /// transaction mode (the placeholder of the later transaction
    /// controls).
    fn console_buttons(
        &self,
        resolved: &Resolved,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let console = matches!(resolved.scope_origin, settings::Origin::Console);
        // A file attached to a data source has the same controls; a file
        // without one only Execute (it asks for a data source).
        let attached = resolved.data_source.is_some();
        let running = self
            .visible_file()
            .and_then(|f| {
                crate::console::consoles(cx)
                    .into_iter()
                    .find(|c| c.read(cx).path == f)
            })
            .is_some_and(|c| c.read(cx).running.is_some());
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let button = |id: &'static str, icon: IconName, tip: &'static str, enabled: bool| {
            let b = ui::components::icon_button(id, icon, px(24.0), px(16.0), false, cx)
                .debug_selector(move || id.to_owned())
                .tooltip(Tooltip::text(tip, None));
            if enabled {
                b
            } else {
                b.opacity(ui::components::DISABLED_ALPHA)
            }
        };
        let execute = button("sql-execute", IconName::RUN, "Execute", !running)
            .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::console::Execute), cx))
            .into_any_element();
        if !console && !attached {
            return vec![execute];
        }
        vec![
            execute,
            div()
                .flex_none()
                .px(px(6.0))
                .text_size(px(CHIP_TEXT))
                .text_color(muted)
                .child("Tx: Auto")
                .into_any_element(),
            button(
                "sql-stop",
                if running {
                    IconName::STOP
                } else {
                    icons::STOP_DISABLED
                },
                "Cancel Running Statements",
                running,
            )
            .when(running, |d| {
                d.on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::console::Stop), cx)
                })
            })
            .into_any_element(),
        ]
    }
}

impl Render for SqlWidget {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(resolved) = self.resolved() else {
            return div().into_any_element();
        };
        let tooltip = SharedString::from(Self::tooltip(&resolved));
        let banner = self.banner_shows(&resolved).then(|| self.render_banner(cx));
        let hint = self
            .connect_hint(&resolved)
            .map(|source| self.render_connect_hint(&source, cx));
        let chips: Vec<_> = self
            .chips(&resolved)
            .into_iter()
            .map(|c| self.render_chip(c, tooltip.clone(), cx))
            .collect();
        let overlay = self.menu.as_ref().map(|(menu, _)| {
            let anchor = self.popup_anchor.get();
            ui::menu::overlay_at(menu, anchor.bottom_left(), Corner::TopLeft, window)
        });
        let console = self.console_buttons(&resolved, cx);
        let theme = cx.theme();
        let toolbar = div()
            .id("sql-toolbar")
            .debug_selector(|| "sql-toolbar".into())
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(4.0))
            .h(px(TOOLBAR_HEIGHT))
            .px(px(8.0))
            .border_b_1()
            .border_color(theme.color(UiRole::Border))
            .children(console)
            .child(div().flex_1())
            .children(chips);
        div()
            .flex()
            .flex_none()
            .flex_col()
            .children(banner)
            .children(hint)
            .child(toolbar)
            .children(overlay)
            .into_any_element()
    }
}
