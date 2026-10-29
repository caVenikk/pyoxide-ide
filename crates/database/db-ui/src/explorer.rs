//! The Database tool window of the right stripe: its header, the toolbar
//! (New, Data Source Properties, then the actions of a connected data
//! source, disabled until the object tree exists), and below it the
//! project's data sources, or the empty state that offers to create one or
//! to import a PyCharm project's.
//!
//! Below the toolbar the object tree ([`tree_view`]): data source >
//! database > schema > groups > objects, from each data source's metadata
//! model.
//!
//! The data sources are the project's (`DataSourceStore::load`), read off
//! the UI thread when a project opens and again after the Data Sources
//! dialog saved them.

use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, ClickEvent, Context, DismissEvent,
    Entity, FocusHandle, Focusable, FontWeight, Global, IntoElement, Pixels, Point, Render,
    SharedString, Subscription, Task, UniformListScrollHandle, WeakEntity, Window, canvas, div,
    point, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::components::{DISABLED_ALPHA, Tooltip, icon_button};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::notification::{Notification, NotificationKind};
use ui::theme::ThemeExt;
use workbench::commands::HideToolWindow;
use workbench::tool_windows::{ToolWindowContent, ToolWindowView};
use workbench::{Workbench, WorkbenchEvent};

use crate::dialog::{self, Initial};
use crate::store::DataSource;
use crate::tree;
use crate::{BACKENDS, backend_title, icons};

mod open_console;
mod open_table;
mod tree_view;

pub(crate) use open_table::{open_selected_command, open_table_command, with_table};

/// The key context of the tool window (its bindings in `db.toml`).
pub const KEY_CONTEXT: &str = "DatabaseExplorer";

ui::ui_commands! {
    COMMANDS, scope = "Database";
    /// Show the New menu.
    NewDataSource => "db.explorer.new-data-source", "New",
        "Shows the New menu of the Database tool window (the \"+\" button): a data source of a database, from a URL or a file, or imported from a PyCharm project.";
    /// Create a data source of a backend.
    AddDataSource { backend: String } => "db.explorer.add-data-source", "Data Source",
        "Opens the Data Sources dialog with a new data source of `backend` (`postgres`, `oracle-thin`, `oracle-odpi`, `sqlite`); with `url`, the URL field shows first.",
        r#"{"backend": "postgres"}"#;
    /// Open the Data Sources dialog.
    Properties => "db.explorer.properties", "Data Source Properties",
        "Opens the Data Sources dialog at the data source selected in the Database tool window (the first one when none is).";
    /// Test the selected data source.
    TestConnection => "db.explorer.test-connection", "Test Connection",
        "Connects to the data source selected in the Database tool window with its kept password and reports the server's version or the error in a balloon.";
    /// Import the data sources of a PyCharm project.
    ImportDataSources => "db.explorer.import-data-sources", "Import Data Sources\u{2026}",
        "Imports the data sources of the project's PyCharm configuration (`.idea/dataSources.xml`) as data sources of the project; passwords are not imported.";
    /// The state of the database views.
    State => "db.state", "Database State",
        "Returns the state of the Database tool window, the Data Sources dialog and the password prompt (never a password).";
    /// Select a data source in the tool window.
    SelectDataSource { name: String } => "db.explorer.select", "Select Data Source",
        "Selects the data source `name` in the Database tool window.",
        r#"{"name": "pyoxide-local"}"#;
    /// Refresh the selected data source's metadata.
    Refresh => "db.explorer.refresh", "Refresh",
        "Reads the metadata of the data source selected in the Database tool window again (its introspection scope) and shows it in the tree; the tree keeps the previous model until the new one is complete.";
    /// Open nodes of the tree.
    Expand { path: String } => "db.explorer.expand", "Expand",
        "Opens every node along `path` in the Database tool window's tree and selects the last: the rows' texts from the data source down, joined by `/`. Opening a node reads the local model, never the database; opening a data source the first time shows its cached model, or reads it when there is none.",
        r#"{"path": "pyoxide-local/pyoxide/demo/tables"}"#;
    /// Close a node of the tree.
    Collapse { path: String } => "db.explorer.collapse", "Collapse",
        "Closes the node at `path` in the Database tool window's tree (see db.explorer.expand).",
        r#"{"path": "pyoxide-local/pyoxide/demo"}"#;
    /// Open every node of the tree.
    ExpandAll => "db.explorer.expand-all", "Expand All",
        "Opens every node of the Database tool window's tree that is loaded (the local model is read, never the database).";
    /// Close every node of the tree.
    CollapseAll => "db.explorer.collapse-all", "Collapse All",
        "Closes every node of the Database tool window's tree, leaving the data sources.";
    /// Select the row above.
    SelectPrevious => "db.explorer.up", "Up", "Selects the row above in the Database tool window's tree.";
    /// Select the row below.
    SelectNext => "db.explorer.down", "Down", "Selects the row below in the Database tool window's tree.";
    /// Open the selected node or go into it.
    ExpandOrNext => "db.explorer.expand-or-next", "Expand",
        "Opens the selected node of the Database tool window's tree, or selects its first child when it is open.";
    /// Close the selected node or go to its parent.
    CollapseOrParent => "db.explorer.collapse-or-parent", "Collapse",
        "Closes the selected node of the Database tool window's tree, or selects its parent.";
    /// Open or close the selected node.
    ToggleSelected => "db.explorer.toggle", "Open or Close Node",
        "Opens or closes the selected node of the Database tool window's tree (a double click does the same); nothing on a node without children.";
    /// Scroll to the selected row.
    Locate => "db.explorer.locate", "Select Opened Node",
        "Scrolls the Database tool window's tree to the selected row (the selected data source when no row is).";
}

/// Texts of the empty state and the menu (our own wording where the
/// reference's is not a plain name).
pub const TITLE: &str = "Database";
/// The tool window's width on the reference's main window of 1710 px
/// (the workbench keeps it within the default share of a smaller one).
pub const WIDTH: f32 = 466.0;
const CREATE_GROUP: &str = "Create Data Source";
const IMPORT_GROUP: &str = "Import Data Sources";
const IMPORT_PYCHARM: &str = "From PyCharm Project";
const NO_PROJECT: &str = "Open a project to add data sources";
const MENU_DATA_SOURCE: &str = "Data Source";
const MENU_FROM_URL: &str = "Data Source from URL";
const MENU_FROM_FILE: &str = "Data Source from File";
const MENU_IMPORT: &str = "Import Data Sources\u{2026}";

/// Metrics read off the reference (logical px): the toolbar under the
/// header, the empty state's groups and buttons.
const TOOLBAR_HEIGHT: f32 = 32.0;
const TOOLBAR_LEFT: f32 = 8.0;
const BUTTON: f32 = 24.0;
const ICON: f32 = 16.0;
const SEPARATOR_GAP: f32 = 5.0;
const CONTENT_SIDE: f32 = 14.0;
const GROUP_TOP: f32 = 18.0;
const GROUP_GAP: f32 = 34.0;
const LABEL_GAP: f32 = 10.0;
const TILE_HEIGHT: f32 = 32.0;
const TILE_GAP: f32 = 10.0;
const TILE_PAD: f32 = 14.0;
const TILE_ICON_GAP: f32 = 8.0;

/// The tool windows of the main windows, for the commands.
#[derive(Default)]
struct Explorers(Vec<(AnyWindowHandle, WeakEntity<Explorer>)>);

impl Global for Explorers {}

/// The explorer a command goes to: the active window's, else the newest.
fn target(cx: &App) -> Option<(AnyWindowHandle, Entity<Explorer>)> {
    let live: Vec<_> = cx
        .try_global::<Explorers>()?
        .0
        .iter()
        .filter_map(|(w, e)| e.upgrade().map(|e| (*w, e)))
        .collect();
    let active = cx.active_window();
    live.iter()
        .find(|(w, _)| Some(*w) == active)
        .cloned()
        .or_else(|| live.last().cloned())
}

/// The explorer of a main window.
pub(crate) fn explorer_of(window: AnyWindowHandle, cx: &App) -> Option<Entity<Explorer>> {
    cx.try_global::<Explorers>()?
        .0
        .iter()
        .filter(|(w, _)| *w == window)
        .find_map(|(_, e)| e.upgrade())
}

/// Runs `f` on the target explorer in its window, after the current
/// dispatch (the window may be the one dispatching).
pub(crate) fn with_target(
    cx: &mut App,
    f: impl FnOnce(&mut Explorer, &mut Window, &mut Context<Explorer>) + 'static,
) {
    let Some((window, explorer)) = target(cx) else {
        return;
    };
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| {
            explorer.update(cx, |e, cx| f(e, window, cx));
        });
    });
}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    cx.on_action(|_: &NewDataSource, cx| {
        with_target(cx, |e, window, cx| e.show_new_menu(None, window, cx))
    });
    cx.on_action(|a: &AddDataSource, cx| {
        let backend = a.backend.clone();
        with_target(cx, move |e, window, cx| e.add(&backend, window, cx))
    });
    cx.on_action(|_: &Properties, cx| with_target(cx, |e, window, cx| e.properties(window, cx)));
    cx.on_action(|_: &TestConnection, cx| {
        with_target(cx, |e, window, cx| e.test_selected(window, cx))
    });
    cx.on_action(|_: &ImportDataSources, cx| with_target(cx, |e, _, cx| e.import(cx)));
    cx.on_action(|a: &SelectDataSource, cx| {
        let name = a.name.clone();
        with_target(cx, move |e, _, cx| e.select_name(&name, cx))
    });
    cx.on_action(|_: &Refresh, cx| with_target(cx, |e, _, cx| e.refresh_selected(cx)));
    cx.on_action(|a: &Expand, cx| {
        let path = a.path.clone();
        with_target(cx, move |e, _, cx| e.expand_path(&path, cx))
    });
    cx.on_action(|a: &Collapse, cx| {
        let path = a.path.clone();
        with_target(cx, move |e, _, cx| e.collapse_path(&path, cx))
    });
    cx.on_action(|_: &ExpandAll, cx| with_target(cx, |e, _, cx| e.expand_all(cx)));
    cx.on_action(|_: &CollapseAll, cx| with_target(cx, |e, _, cx| e.collapse_all(cx)));
    cx.on_action(|_: &Locate, cx| with_target(cx, |e, _, cx| e.locate(cx)));
    cx.on_action(|_: &SelectPrevious, cx| with_target(cx, |e, _, cx| e.move_selection(-1, cx)));
    cx.on_action(|_: &SelectNext, cx| with_target(cx, |e, _, cx| e.move_selection(1, cx)));
    cx.on_action(|_: &ExpandOrNext, cx| with_target(cx, |e, _, cx| e.expand_or_next(cx)));
    cx.on_action(|_: &CollapseOrParent, cx| with_target(cx, |e, _, cx| e.collapse_or_parent(cx)));
    cx.on_action(|_: &ToggleSelected, cx| with_target(cx, |e, _, cx| e.toggle_selected(cx)));
    ui::command::register_query(
        cx,
        "db.state",
        std::rc::Rc::new(|_, window, cx| {
            let main = ui::dialog_window::parent_of(window.window_handle(), cx)
                .unwrap_or(window.window_handle());
            let explorer = explorer_of(main, cx).map(|e| e.read(cx).state_json(cx));
            Ok(json!({
                "explorer": explorer.unwrap_or(Json::Null),
                "dialog": dialog::state_json(cx),
                "prompt": crate::prompt::state_json(cx),
            }))
        }),
    );
}

/// Makes the tool window for a main window (the module's hook).
pub(crate) fn create(
    workbench: WeakEntity<Workbench>,
    window: &mut Window,
    cx: &mut App,
) -> ToolWindowContent {
    let handle = window.window_handle();
    let explorer = cx.new(|cx| Explorer::new(workbench.clone(), cx));
    cx.default_global::<Explorers>()
        .0
        .push((handle, explorer.downgrade()));
    if let Some(workbench) = workbench.upgrade() {
        let root = workbench.read(cx).project_root(cx);
        explorer.update(cx, |e, cx| {
            e.subscribe(&workbench, cx);
            if let Some(root) = root {
                e.set_root(root, cx);
            }
        });
    }
    ToolWindowContent::new(explorer)
}

/// Loads the data sources again in every main window of `root`.
pub(crate) fn reload(root: &Path, cx: &mut App) {
    let explorers: Vec<_> = cx
        .try_global::<Explorers>()
        .map(|e| e.0.iter().filter_map(|(_, e)| e.upgrade()).collect())
        .unwrap_or_default();
    for explorer in explorers {
        explorer.update(cx, |e, cx| {
            if e.root.as_deref() == Some(root) {
                e.load(cx);
            }
        });
    }
}

/// An open popup of the tool window.
struct OpenMenu {
    menu: Entity<ContextMenu>,
    position: Point<Pixels>,
    _subscription: Subscription,
}

/// The tool window's view.
pub struct Explorer {
    workbench: WeakEntity<Workbench>,
    root: Option<PathBuf>,
    /// `None` until the first read finished.
    sources: Option<Vec<DataSource>>,
    error: Option<String>,
    selected: Option<String>,
    menu: Option<OpenMenu>,
    /// Where the "+" button is, for its menu.
    add_bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    focus: FocusHandle,
    load_task: Task<()>,
    test_task: Task<()>,
    tree: tree_view::TreeState,
    /// Where the data source handles send their events (made with the
    /// first handle).
    sink: Option<db_session::EventSink>,
    events_task: Task<()>,
    scroll: UniformListScrollHandle,
    /// The pointer's row of the tree and the row cut by its edge that shows
    /// its rest.
    expansion: ui::expandable::ExpandableItems<crate::tree::RowKey>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Explorer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ToolWindowView for Explorer {}

impl Explorer {
    fn new(workbench: WeakEntity<Workbench>, cx: &mut Context<Self>) -> Self {
        Self {
            workbench,
            root: None,
            sources: None,
            error: None,
            selected: None,
            menu: None,
            add_bounds: Default::default(),
            focus: cx.focus_handle(),
            load_task: Task::ready(()),
            test_task: Task::ready(()),
            tree: tree_view::TreeState::default(),
            sink: None,
            events_task: Task::ready(()),
            scroll: UniformListScrollHandle::new(),
            expansion: Default::default(),
            _subscriptions: Vec::new(),
        }
    }

    fn subscribe(&mut self, workbench: &Entity<Workbench>, cx: &mut Context<Self>) {
        self._subscriptions.push(
            cx.subscribe(workbench, |this, _, event: &WorkbenchEvent, cx| {
                if let WorkbenchEvent::ProjectOpened(root) = event {
                    this.set_root(root.clone(), cx);
                }
            }),
        );
    }

    fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.root = Some(root);
        self.sources = None;
        self.load(cx);
    }

    /// The project's folder.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// The data sources as last read.
    pub fn sources(&self) -> &[DataSource] {
        self.sources.as_deref().unwrap_or_default()
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let store = crate::store(cx);
        let read = cx.background_spawn(async move { store.load(&root) });
        self.load_task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(sources) => {
                        if this
                            .selected
                            .as_ref()
                            .is_some_and(|s| !sources.iter().any(|d| &d.name == s))
                        {
                            this.selected = None;
                        }
                        this.sources = Some(sources);
                        this.error = None;
                    }
                    Err(e) => {
                        this.sources = Some(Vec::new());
                        this.error = Some(e);
                    }
                }
                this.sources_changed();
                cx.notify();
            });
        });
    }

    fn selected_source(&self) -> Option<&DataSource> {
        let sources = self.sources.as_ref()?;
        match &self.selected {
            Some(name) => sources.iter().find(|s| &s.name == name),
            None => sources.first(),
        }
    }

    fn select_name(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.sources().iter().any(|s| s.name == name) {
            self.select_row(tree::RowKey::source(&name.into()), cx);
        }
    }

    // ---- actions ----------------------------------------------------------------

    fn new_menu_items(&self) -> Vec<MenuItem> {
        let drivers = BACKENDS
            .iter()
            .map(|b| {
                MenuItem::action(
                    backend_title(*b),
                    AddDataSource {
                        backend: b.id().to_owned(),
                    },
                )
                .icon(icons::backend(*b))
            })
            .collect();
        vec![
            MenuItem::submenu(MENU_DATA_SOURCE, drivers).icon(icons::DATA_SOURCE),
            MenuItem::action(
                MENU_FROM_URL,
                AddDataSource {
                    backend: "url".into(),
                },
            )
            .icon(icons::URL),
            MenuItem::action(
                MENU_FROM_FILE,
                AddDataSource {
                    backend: "sqlite".into(),
                },
            )
            .icon(icons::FILE),
            MenuItem::separator(),
            MenuItem::action(MENU_IMPORT, ImportDataSources).icon(icons::IMPORT),
        ]
    }

    /// The New menu under the "+" button (or at `at`).
    fn show_new_menu(
        &mut self,
        at: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.root.is_none() {
            return;
        }
        let bounds = self.add_bounds.get();
        let position = at.unwrap_or_else(|| point(bounds.left(), bounds.bottom() + px(2.0)));
        let menu = ContextMenu::popup(self.new_menu_items(), PopupOptions::default(), window, cx);
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

    fn add(&mut self, backend: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(root) = self.root.clone() else {
            return;
        };
        let initial = match backend {
            "url" => Initial::Url,
            id => match db_core::Backend::from_id(id) {
                Some(b) => Initial::New(b),
                None => return,
            },
        };
        self.open_dialog(root, initial, window, cx);
    }

    fn properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(root) = self.root.clone() else {
            return;
        };
        let initial = match self.selected_source() {
            Some(s) => Initial::Select(s.name.clone()),
            None => Initial::New(db_core::Backend::Postgres),
        };
        self.open_dialog(root, initial, window, cx);
    }

    fn open_dialog(
        &mut self,
        root: PathBuf,
        initial: Initial,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sources = self.sources().to_vec();
        dialog::open(root, sources, initial, window, cx);
    }

    /// Tests the selected data source; one whose password is not kept
    /// ("Save: Never") asks for it first, every time.
    fn test_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(root), Some(source)) = (self.root.clone(), self.selected_source().cloned())
        else {
            return;
        };
        let asks = source.profile.needs_password()
            && source.profile.gui_options().password_save == db_core::PasswordSave::Never;
        if asks {
            let this = cx.entity().downgrade();
            let (name, user) = (source.name.clone(), source.profile.user.clone());
            crate::prompt::ask(&name, &user, window, cx, move |secret, cx| {
                if let (Some(secret), Some(this)) = (secret, this.upgrade()) {
                    this.update(cx, |this, cx| this.run_test(root, source, Some(secret), cx));
                }
            });
            return;
        }
        self.run_test(root, source, None, cx);
    }

    fn run_test(
        &mut self,
        root: PathBuf,
        source: DataSource,
        password: Option<db_core::Secret>,
        cx: &mut Context<Self>,
    ) {
        let name = source.name.clone();
        let test = crate::store(cx).test(&root, source, password);
        let run = cx.background_spawn(test);
        self.test_task = cx.spawn(async move |this, cx| {
            let result = run.await;
            let _ = this.update(cx, |this, cx| {
                let notification = match result {
                    Ok(info) => Notification::new(
                        NotificationKind::Information,
                        format!("{name}: connected"),
                        format!("{} {}, {}", info.product, info.version, info.driver),
                    ),
                    Err(f) => Notification::new(
                        NotificationKind::Error,
                        format!("{name}: connection failed"),
                        dialog::failure_text(&f),
                    ),
                };
                this.notify(notification, cx);
            });
        });
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        let Some(root) = self.root.clone() else {
            return;
        };
        let store = crate::store(cx);
        let run_root = root.clone();
        let run = cx.background_spawn(async move { store.import_pycharm(&run_root) });
        self.load_task = cx.spawn(async move |this, cx| {
            let result = run.await;
            let _ = this.update(cx, |this, cx| {
                let notification = match result {
                    Ok(summary) => {
                        let mut text = if summary.imported.is_empty() {
                            "No data sources were imported.".to_owned()
                        } else {
                            format!("Imported: {}.", summary.imported.join(", "))
                        };
                        for (name, why) in &summary.skipped {
                            text.push_str(&format!(" Skipped {name}: {why}."));
                        }
                        Notification::new(
                            NotificationKind::Information,
                            "PyCharm data sources",
                            text,
                        )
                    }
                    Err(e) => Notification::new(NotificationKind::Error, "PyCharm data sources", e),
                };
                this.notify(notification, cx);
                this.load(cx);
            });
        });
    }

    fn notify(&self, notification: Notification, cx: &mut Context<Self>) {
        if let Some(workbench) = self.workbench.upgrade() {
            workbench.update(cx, |w, cx| {
                w.notify(notification, cx);
            });
        }
    }

    /// The tool window's state, for `db.state`.
    pub fn state_json(&self, cx: &App) -> Json {
        json!({
            "root": self.root.as_ref().map(|r| r.display().to_string()),
            "loaded": self.sources.is_some(),
            "error": self.error,
            "selected": self.selected_source().map(|s| s.name.clone()),
            "sources": self.sources().iter().map(|s| json!({
                "name": s.name,
                "backend": s.profile.backend.id(),
                "target": s.profile.target(),
            })).collect::<Vec<_>>(),
            "menu": self.menu.as_ref().map(|m| m.menu.read(cx).state_json()),
            "tree": self.tree_json(),
        })
    }

    // ---- drawing ----------------------------------------------------------------

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let focus = self.focus.clone();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(theme.metric(Metric::ToolWindowHeaderHeight))
            .pl(px(12.0))
            .pr(px(8.0))
            .gap(px(2.0))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .child(TITLE),
            )
            .child(div().flex_1())
            .child(
                icon_button(
                    "database-locate",
                    IconName::LOCATE,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("Select Opened Node", None))
                .on_click(cx.listener(|this, _, _, cx| this.locate(cx))),
            )
            .child(
                icon_button(
                    "database-expand-all",
                    IconName::EXPAND_ALL,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("Expand All", None))
                .on_click(cx.listener(|this, _, _, cx| this.expand_all(cx))),
            )
            .child(
                icon_button(
                    "database-collapse-all",
                    IconName::COLLAPSE_ALL,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("Collapse All", None))
                .on_click(cx.listener(|this, _, _, cx| this.collapse_all(cx))),
            )
            .child(
                icon_button(
                    "database-options",
                    IconName::MORE_VERTICAL,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("Options", None)),
            )
            .child(
                icon_button(
                    "database-hide",
                    IconName::HIDE,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("Hide", None))
                .on_click(move |_: &ClickEvent, window, cx| {
                    window.focus(&focus);
                    window.dispatch_action(Box::new(HideToolWindow), cx);
                }),
            )
    }

    fn toolbar_button(
        &self,
        id: &'static str,
        icon: IconName,
        tooltip: &'static str,
        enabled: bool,
        cx: &App,
    ) -> gpui::Stateful<gpui::Div> {
        icon_button(id, icon, px(BUTTON), px(ICON), false, cx)
            .debug_selector(move || id.to_owned())
            .tooltip(Tooltip::text(tooltip, None))
            .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let separator = || {
            div()
                .mx(px(SEPARATOR_GAP))
                .w(px(1.0))
                .h(px(16.0))
                .bg(theme.color(UiRole::Divider))
        };
        let has_project = self.root.is_some();
        let has_source = self.selected_source().is_some();
        let add_bounds = self.add_bounds.clone();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(TOOLBAR_HEIGHT))
            .pl(px(TOOLBAR_LEFT))
            .gap(px(2.0))
            .child(
                self.toolbar_button("database-new", IconName::ADD, "New", has_project, cx)
                    .child(
                        canvas(move |bounds, _, _| add_bounds.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                    )
                    .relative()
                    .when(has_project, |d| {
                        d.on_click(
                            cx.listener(|this, _, window, cx| this.show_new_menu(None, window, cx)),
                        )
                    }),
            )
            .child(
                self.toolbar_button(
                    "database-properties",
                    icons::PROPERTIES,
                    "Data Source Properties",
                    has_project,
                    cx,
                )
                .when(has_project, |d| {
                    d.on_click(cx.listener(|this, _, window, cx| this.properties(window, cx)))
                }),
            )
            .child(separator())
            .child(
                self.toolbar_button(
                    "database-refresh",
                    IconName::REFRESH,
                    "Refresh",
                    has_source,
                    cx,
                )
                .when(has_source, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.refresh_selected(cx)))
                }),
            )
            .child(self.toolbar_button(
                "database-disconnect",
                icons::DISCONNECT,
                "Disconnect",
                false,
                cx,
            ))
            .child(separator())
            .child(
                self.toolbar_button(
                    "database-console",
                    icons::CONSOLE,
                    "Query Console",
                    has_source,
                    cx,
                )
                .when(has_source, |d| {
                    d.on_click(
                        cx.listener(|this, _, window, cx| this.open_console(None, window, cx)),
                    )
                }),
            )
            .child(self.toolbar_button("database-table", icons::TABLE, "Edit Data", false, cx))
            .child(
                div()
                    .id("database-ddl")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .h(px(BUTTON))
                    .px(px(4.0))
                    .text_size(theme.ui_size() - px(2.0))
                    .text_color(theme.color(UiRole::IconGrayStroke))
                    .opacity(DISABLED_ALPHA)
                    .tooltip(Tooltip::text("Go to DDL", None))
                    .child("DDL"),
            )
            .child(self.toolbar_button(
                "database-jump",
                icons::JUMP,
                "Jump to Query Console",
                false,
                cx,
            ))
            .child(separator())
            .child(self.toolbar_button(
                "database-view-options",
                IconName::SHOW,
                "View Options",
                has_source,
                cx,
            ))
    }

    fn tile(
        &self,
        id: impl Into<SharedString>,
        icon: IconName,
        label: &'static str,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let id: SharedString = id.into();
        let selector = id.to_string();
        div()
            .id(gpui::ElementId::Name(id))
            .debug_selector(move || selector.clone())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(TILE_ICON_GAP))
            .h(px(TILE_HEIGHT))
            .px(px(TILE_PAD))
            .rounded(theme.metric(Metric::ButtonRadius))
            .bg(theme.color(UiRole::Hover).opacity(0.6))
            .hover(|s| s.bg(theme.color(UiRole::Hover)))
            .child(Icon::new(icon).size(px(ICON)))
            .child(div().whitespace_nowrap().child(label))
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .into_any_element()
    }

    fn group(&self, title: &'static str, tiles: Vec<AnyElement>, cx: &App) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_none()
            .child(
                div()
                    .text_size(cx.theme().ui_size() - px(1.0))
                    .text_color(cx.theme().color(UiRole::TextMuted))
                    .child(title),
            )
            .child(
                div()
                    .mt(px(LABEL_GAP))
                    .flex()
                    .flex_col()
                    .gap(px(TILE_GAP))
                    .children(tiles),
            )
            .into_any_element()
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.root.is_none() {
            return div()
                .id("database-no-project")
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(cx.theme().color(UiRole::TextMuted))
                .child(NO_PROJECT)
                .into_any_element();
        }
        let create: Vec<AnyElement> = [
            db_core::Backend::Postgres,
            db_core::Backend::OracleThin,
            db_core::Backend::Sqlite,
        ]
        .into_iter()
        .map(|b| {
            self.tile(
                format!("database-create-{}", b.id()),
                icons::backend(b),
                backend_title(b),
                move |this, window, cx| this.add(b.id(), window, cx),
                cx,
            )
        })
        .collect();
        let import = vec![self.tile(
            "database-import-pycharm",
            icons::IMPORT,
            IMPORT_PYCHARM,
            |this, _, cx| this.import(cx),
            cx,
        )];
        div()
            .id("database-empty")
            .debug_selector(|| "database-empty".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .px(px(CONTENT_SIDE))
            .pt(px(GROUP_TOP))
            .gap(px(GROUP_GAP))
            .child(self.group(CREATE_GROUP, create, cx))
            .child(self.group(IMPORT_GROUP, import, cx))
            .into_any_element()
    }
}

impl Render for Explorer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(cx).into_any_element();
        let toolbar = self.render_toolbar(cx).into_any_element();
        let body = if self.sources().is_empty() {
            if self.root.is_some() && self.sources.is_none() {
                div().flex_1().into_any_element()
            } else {
                self.render_empty(cx)
            }
        } else {
            self.render_tree(window, cx)
        };
        let popup = self
            .menu
            .as_ref()
            .map(|m| ui::menu::overlay(&m.menu, m.position, window));
        div()
            .id("database-tool-window-content")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(toolbar)
            .child(body)
            .children(popup)
    }
}
