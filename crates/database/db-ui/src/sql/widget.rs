//! The dialect and scope widget of the status bar (ADR 0022): the SQL
//! dialect of the active `.sql` file and the data source it is attached to,
//! with a popup to change either. A choice is a mapping of the file, its
//! folder or the project, kept in the personal project state (or the
//! shared settings on request), and every open editor of the files it
//! reaches is coloured again at once.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Corner, DismissEvent, Entity, Global,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, StatefulInteractiveElement,
    Styled, Subscription, Task, WeakEntity, Window, canvas, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use sql_syntax::Dialect;
use themes::UiRole;
use ui::command::CommandError;
use ui::components::Tooltip;
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;
use workbench::{StatusWidget, Workbench, WorkbenchEvent};

use super::hub::{Hub, Kind, origin_text};
use super::settings::{self, Resolved};

ui::ui_commands! {
    COMMANDS, scope = "Database";
    /// Open the dialect and scope popup.
    ChooseDialect => "sql.choose", "Change SQL Dialect and Data Source",
        "Opens the SQL popup of the status bar: the dialects, and the data sources the file can be attached to.";
    /// Set the dialect of a file, folder or the project.
    SetDialect { dialect: String, target: Option<String>, shared: Option<bool> } => "sql.set-dialect", "Set SQL Dialect",
        "Maps the active SQL file, its folder (`target`: `folder`) or the whole project (`project`) to a dialect (`generic`, `postgres`, `oracle`, `sqlite`; `default` removes the mapping). The mapping is personal unless `shared` is true, which writes it to the project's `.oxide/project.toml`. Open files are coloured again.",
        r#"{"dialect": "postgres", "target": "folder"}"#;
    /// Attach a data source to a file, folder or the project.
    AttachDataSource { name: Option<String>, schema: Option<String>, target: Option<String>, shared: Option<bool> } => "sql.attach-data-source", "Attach Data Source",
        "Attaches the active SQL file, its folder (`target`: `folder`) or the whole project (`project`) to the data source `name` and the optional `schema`; without `name` the attachment is removed. Names in the file then mean what that data source's metadata says. Personal unless `shared` is true.",
        r#"{"name": "pyoxide-local", "schema": "public"}"#;
    /// Describe the widget.
    SqlState => "sql.state", "SQL State",
        "Returns the SQL widget's state as JSON: whether it shows, its text, the active file's dialect and scope with where each came from, the data sources it can attach, and the open popup.";
}

/// The product-wide settings of the SQL editor glue, in a global so that
/// observers see every change.
pub(crate) struct SqlGlobal(pub Arc<Hub>);

impl Global for SqlGlobal {}

/// The name of a dialect as the widget shows it.
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
    cx.set_global(SqlGlobal(hub));
    ui::command::register(cx, COMMANDS);
    cx.on_action(|_: &ChooseDialect, cx| {
        let Some((window, widget)) = target(cx) else {
            return;
        };
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                widget.update(cx, |w, cx| w.open(window, cx));
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
    bounds: Rc<Cell<Bounds<Pixels>>>,
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
            bounds: Rc::default(),
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

    /// The widget's text: the dialect, then the data source.
    fn text(resolved: &Resolved) -> (String, Option<String>) {
        (
            dialect_title(resolved.dialect).to_owned(),
            resolved
                .data_source
                .as_ref()
                .map(|d| match &resolved.schema {
                    Some(s) => format!("{d}.{s}"),
                    None => d.clone(),
                }),
        )
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

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            return;
        }
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
        let (text, data_source) = resolved
            .as_ref()
            .map(Self::text)
            .map(|(t, d)| (Some(t), d))
            .unwrap_or((None, None));
        json!({
            "visible": resolved.is_some(),
            "file": self.visible_file().map(|f| f.display().to_string()),
            "text": text,
            "data-source-text": data_source,
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

pub(crate) fn status_widget() -> StatusWidget {
    StatusWidget {
        id: "status-sql",
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

impl Render for SqlWidget {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(resolved) = self.resolved() else {
            return div().into_any_element();
        };
        let (dialect, data_source) = Self::text(&resolved);
        let tooltip = Self::tooltip(&resolved);
        let hover = cx.theme().color(UiRole::Hover);
        let muted = cx.theme().color(UiRole::TextMuted);
        let bounds = self.bounds.clone();
        let overlay = self.menu.as_ref().map(|(menu, _)| {
            ui::menu::overlay_at(menu, self.bounds.get().origin, Corner::BottomLeft, window)
        });
        div()
            .id("status-sql")
            .debug_selector(|| "status-sql".into())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .h(px(22.0))
            .px(px(6.0))
            .rounded(px(4.0))
            .whitespace_nowrap()
            .hover(move |s| s.bg(hover))
            .child(dialect)
            .children(data_source.map(|d| div().text_color(muted).child(d)))
            .tooltip(Tooltip::text(tooltip, None))
            .on_click(cx.listener(|this, _, window, cx| this.open(window, cx)))
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .children(overlay)
            .into_any_element()
    }
}
