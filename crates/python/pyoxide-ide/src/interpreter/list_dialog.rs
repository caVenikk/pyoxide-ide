//! The Python Interpreters dialog: "Show All…" of the Interpreter page's
//! combo (the reference IDE's `PythonInterpreterConfigurable` with
//! `PythonInterpreterMasterDetails`, PyCharm 2026.2). At the left the list
//! of interpreters under its toolbar (Add New Interpreter, Remove
//! Interpreter, Rename, "Hide virtual environments associated with other
//! projects", Show Interpreter Paths); at the right the details of the
//! selected one (`PythonLocalInterpreterConfigurable`): its interpreter
//! path and, for a plain virtual environment or a conda one, whether it is
//! associated with the project; Cancel, Apply and OK at the bottom.
//!
//! The list holds the application's configured interpreters and the
//! project's environments that are not configured yet (the reference
//! registers those when the folder opens), sorted by name. Removing one
//! and editing the details wait for Apply or OK; Add and Rename change the
//! list at once, as in the reference. OK hands the selected interpreter to
//! the Interpreter page, which selects it until its own Apply. Rename asks
//! for the name in the "Rename Python Interpreter" dialog; Show Interpreter
//! Paths lists the folders the interpreter imports from (read only:
//! adding and excluding paths comes with the analysis that uses them).

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use editor::Editor;
use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, ClickEvent, Context, Entity,
    FocusHandle, Focusable, Global, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, Pixels,
    Point, Render, SharedString, Subscription, Task, WeakEntity, Window, WindowHandle, canvas, div,
    prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::components::{DISABLED_ALPHA, Tooltip};
use ui::form::{BarButton, BottomBar, ButtonRole, MenuPopup};
use ui::icons::{Icon, IconName, IconSvg};
use ui::menu::MenuItem;
use ui::theme::ThemeExt;

use super::create::Select;
use super::table::{self, Change, Entry, Flavor, Table};
use super::{candidates, configuring, list, runtime};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Open the dialog.
    ShowAllInterpreters => "python.interpreters.show-all", "Show All\u{2026}",
        "Opens the Python Interpreters dialog (\"Show All…\" of the Interpreter page's combo): over the Settings window of an open Interpreter page, with the page's interpreter selected (OK selects the dialog's choice in the page until Apply), else over the active main window.";
    /// Rename a configured interpreter.
    RenameInterpreter { name: String, new_name: String } => "python.interpreters.rename", "Rename Python Interpreter",
        "Renames an interpreter of the application's list of configured interpreters (names are unique and not blank).",
        r#"{"name": "~/work/demo/.venv", "new_name": "demo"}"#;
    /// Remove a configured interpreter.
    RemoveInterpreter { name: String } => "python.interpreters.remove", "Remove Python Interpreter",
        "Removes an interpreter from the application's list of configured interpreters; its files stay.",
        r#"{"name": "demo"}"#;
    /// Change the executable of a configured interpreter.
    SetInterpreterPath { name: String, python: String } => "python.interpreters.set-path", "Set Interpreter Path",
        "Changes the executable of an interpreter of the application's list of configured interpreters (\"Interpreter path\").",
        r#"{"name": "demo", "python": "/work/demo/.venv/bin/python"}"#;
}

ui::ui_commands! {
    DIALOG_COMMANDS, scope = "PythonInterpretersDialog";
    /// Select a row.
    DialogSelect { name: String } => "python.interpreters.dialog.select", "Select Interpreter",
        "Selects an interpreter of the Python Interpreters dialog by its name.",
        r#"{"name": "demo"}"#;
    /// Open the Add popup.
    DialogAdd => "python.interpreters.dialog.add", "Add New Interpreter",
        "Opens the popup of the dialog's Add button (Add Local Interpreter…).";
    /// Open the Add Python Interpreter dialog.
    DialogAddLocal => "python.interpreters.dialog.add-local", "Add Local Interpreter\u{2026}",
        "Opens the Add Python Interpreter dialog over the Python Interpreters dialog; the new interpreter is selected in the list.";
    /// Remove the selected row.
    DialogRemove => "python.interpreters.dialog.remove", "Remove Interpreter",
        "Removes the selected interpreter from the list (the list of configured interpreters changes at Apply or OK).";
    /// Rename the selected row.
    DialogRename => "python.interpreters.dialog.rename", "Rename",
        "Opens the Rename Python Interpreter dialog for the selected interpreter.";
    /// Hide or show other projects' environments.
    DialogToggleFilter => "python.interpreters.dialog.toggle-filter", "Hide virtual environments associated with other projects",
        "Hides or shows the virtual environments associated with other projects (hidden when the dialog opens).";
    /// Show the selected interpreter's paths.
    DialogShowPaths => "python.interpreters.dialog.show-paths", "Show Interpreter Paths",
        "Opens the Interpreter Paths dialog: the folders the selected interpreter imports from.";
    /// Set a field of the details.
    DialogSet { field: String, value: String } => "python.interpreters.dialog.set", "Set Field",
        "Sets a field of the selected interpreter's details: `path` (the interpreter path), `associated` (`true`, `false`: \"Associate this virtual environment with the current project\").",
        r#"{"field": "path", "value": "/work/demo/.venv/bin/python"}"#;
    /// Remove the association with another project.
    DialogRemoveAssociation => "python.interpreters.dialog.remove-association", "Remove Association",
        "The \"Remove association\" link of an environment associated with another project.";
    /// Choose the interpreter path.
    DialogBrowse => "python.interpreters.dialog.browse", "Browse",
        "Chooses the selected interpreter's path in the system's dialog (\"Specify Interpreter Path\").";
    /// Apply.
    DialogApply => "python.interpreters.dialog.apply", "Apply",
        "Applies the removals and the details' changes to the list of configured interpreters.";
    /// OK.
    DialogOk => "python.interpreters.dialog.ok", "OK",
        "Applies the changes, closes the dialog and selects its interpreter in the Interpreter page.";
    /// Cancel.
    DialogCancel => "python.interpreters.dialog.cancel", "Cancel",
        "Closes the dialog; changes not applied are dropped.";
}

ui::ui_commands! {
    RENAME_COMMANDS, scope = "RenamePythonInterpreterDialog";
    /// Type the new name.
    RenameSet { text: String } => "python.interpreters.rename-dialog.set", "Set Name",
        "Replaces the text of the Rename Python Interpreter dialog's field.",
        r#"{"text": "demo"}"#;
    /// OK.
    RenameOk => "python.interpreters.rename-dialog.ok", "OK",
        "Renames the interpreter (only with a name that is not blank and not taken).";
    /// Cancel.
    RenameCancel => "python.interpreters.rename-dialog.cancel", "Cancel",
        "Closes the Rename Python Interpreter dialog.";
}

ui::ui_commands! {
    PATHS_COMMANDS, scope = "InterpreterPathsDialog";
    /// Read the paths again.
    PathsReload => "python.interpreters.paths-dialog.reload", "Reload List of Paths",
        "Asks the interpreter for its paths again.";
    /// OK.
    PathsOk => "python.interpreters.paths-dialog.ok", "OK",
        "Closes the Interpreter Paths dialog.";
    /// Cancel.
    PathsCancel => "python.interpreters.paths-dialog.cancel", "Cancel",
        "Closes the Interpreter Paths dialog.";
}

/// Texts of the reference (PyBundle).
pub const TITLE: &str = "Python Interpreters";
const ADD: &str = "Add New Interpreter";
const REMOVE: &str = "Remove Interpreter";
const RENAME: &str = "Rename";
const HIDE_OTHERS: &str = "Hide virtual environments associated with other projects";
const SHOW_PATHS: &str = "Show Interpreter Paths";
const INTERPRETER_PATH: &str = "Interpreter path:";
const SPECIFY_PATH: &str = "Specify Interpreter Path";
const ASSOCIATE_CURRENT: &str = "Associate this virtual environment with the current project";
const REMOVE_ASSOCIATION: &str = "Remove association";
const INVALID: &str = "invalid";
pub const RENAME_TITLE: &str = "Rename Python Interpreter";
const NEW_NAME: &str = "New name:";
pub const PATHS_TITLE: &str = "Interpreter Paths";
const RELOAD_PATHS: &str = "Reload List of Paths";
const ADD_PATH: &str = "Add";
const REMOVE_PATH: &str = "Remove";

/// `sdk.edit.dialog.associate.virtual.env.with.path`.
fn associate_with(path: &Path) -> String {
    format!("Associate this virtual environment with {}", path.display())
}

/// Metrics measured on the reference (logical px): the window (800 × 676
/// with its title bar), the insets of the settings dialog
/// (`UIUtil.getRegularPanelInsets` without the bottom and right ones), the
/// share of the list (`OnePixelSplitter(.2f)`), its toolbar (22 px buttons
/// on a pitch of 26), its rows, and the details' border (11, 16, 16, 16)
/// with the first row 7 px lower.
const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 648.0;
const MIN_SIZE: (f32, f32) = (520.0, 300.0);
const INSET_TOP: f32 = 8.0;
const INSET_LEFT: f32 = 12.0;
const MASTER_SHARE: f32 = 0.2;
const TOOLBAR_HEIGHT: f32 = 26.0;
const TOOLBAR_PAD: f32 = 1.5;
const TOOL_GAP: f32 = 4.0;
const ROW: f32 = 22.0;
const ROW_PAD: f32 = 3.0;
const ICON_GAP: f32 = 2.0;
const DETAILS_TOP: f32 = 18.0;
const DETAILS_SIDE: f32 = 16.0;
const LABEL_GAP: f32 = 6.0;
const FIELD: f32 = 24.0;
const DETAIL_ROW_GAP: f32 = 10.0;
/// The Rename dialog (`Messages.InputDialog`: the message above a field of
/// 30 columns) and the Interpreter Paths dialog (`PythonPathDialog`: a
/// bordered 600 × 400 panel).
const RENAME_WIDTH: f32 = 380.0;
const RENAME_HEIGHT: f32 = 130.0;
const PAD_TOP: f32 = 12.0;
const PAD_LEFT: f32 = 12.0;
const PAD_RIGHT: f32 = 15.0;
const MESSAGE_GAP: f32 = 5.0;
const PATHS_WIDTH: f32 = 624.0;
const PATHS_HEIGHT: f32 = 464.0;
/// How long the interpreter may take to list its paths.
const PATHS_TIMEOUT: Duration = Duration::from_secs(20);

/// Icons: the pencil of Rename (New UI `actions/edit`).
const EDIT_ICON: IconName = IconName("interpreter/edit");
pub(super) const ICONS: [IconSvg; 1] = [IconSvg {
    name: EDIT_ICON.0,
    light: include_bytes!("../../assets/icons/edit.svg"),
    dark: Some(include_bytes!("../../assets/icons/edit_dark.svg")),
}];
const VENV_ICON: IconName = IconName("python/virtualEnv");

// ---- registration ---------------------------------------------------------------

/// The open dialog, for `python.interpreter.state` and the new
/// interpreter of its Add button.
#[derive(Default)]
struct OpenDialog(Option<WeakEntity<InterpretersDialog>>);

impl Global for OpenDialog {}

fn open_dialog(cx: &App) -> Option<Entity<InterpretersDialog>> {
    cx.try_global::<OpenDialog>()?.0.as_ref()?.upgrade()
}

/// The open dialog's state, `null` without one.
pub(crate) fn state_json(cx: &App) -> Json {
    open_dialog(cx)
        .map(|d| d.read(cx).state_json(cx))
        .unwrap_or(Json::Null)
}

pub(super) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, DIALOG_COMMANDS);
    ui::command::register(cx, RENAME_COMMANDS);
    ui::command::register(cx, PATHS_COMMANDS);
    cx.on_action(|_: &ShowAllInterpreters, cx| {
        // `invokeLater`: after the combo's list closed.
        if let Some((root, window, selected)) = super::page::show_all_target(cx) {
            open_later(window, Some(root), Origin::Page, selected, cx);
        } else if let Some((window, root, selected)) = super::widget::active_project(cx) {
            open_later(window, root, Origin::Window, selected, cx);
        }
    });
    cx.on_action(|a: &RenameInterpreter, cx| {
        let change = Change::Rename {
            name: a.name.clone(),
            new_name: a.new_name.clone(),
        };
        report(
            "python.interpreters.rename",
            list::change(vec![change], cx),
            cx,
        );
    });
    cx.on_action(|a: &RemoveInterpreter, cx| {
        if list::table(cx).find(&a.name).is_none() {
            eprintln!("python.interpreters.remove: no interpreter `{}`", a.name);
            return;
        }
        let change = Change::Remove(a.name.clone());
        report(
            "python.interpreters.remove",
            list::change(vec![change], cx),
            cx,
        );
    });
    cx.on_action(|a: &SetInterpreterPath, cx| {
        let python = PathBuf::from(&a.python);
        let change = Change::SetPython {
            name: a.name.clone(),
            python: python.clone(),
        };
        report(
            "python.interpreters.set-path",
            list::change(vec![change], cx),
            cx,
        );
        refresh_version(a.name.clone(), python, cx);
    });
}

/// Prints the error a change of the list ended with.
fn report(command: &'static str, done: Task<Result<Arc<Table>, String>>, cx: &mut App) {
    cx.spawn(async move |_| {
        if let Err(e) = done.await {
            eprintln!("{command}: {e}");
        }
    })
    .detach();
}

/// The version of an interpreter whose path changed, asked in the
/// background (`PythonSdkUpdater`).
fn refresh_version(name: String, python: PathBuf, cx: &mut App) {
    let version = cx.background_spawn(async move { super::full_version(None, &python) });
    cx.spawn(async move |cx| {
        let version = version.await;
        let _ = cx.update(|cx| {
            if list::table(cx).find(&name).is_some() {
                list::change(vec![Change::SetVersion { name, version }], cx).detach();
            }
        });
    })
    .detach();
}

/// Where the dialog was opened from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// "Show All…" of the Interpreter page: OK selects the choice there.
    Page,
    /// The command, without a page.
    Window,
}

/// Opens the dialog over `window` once the action that asks for it is
/// done (a popup's row runs while its window is updated).
fn open_later(
    window: AnyWindowHandle,
    root: Option<PathBuf>,
    origin: Origin,
    selected: Option<PathBuf>,
    cx: &mut App,
) {
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| open(root, origin, selected, window, cx));
    });
}

fn open(
    root: Option<PathBuf>,
    origin: Origin,
    selected: Option<PathBuf>,
    window: &mut Window,
    cx: &mut App,
) {
    if open_dialog(cx).is_some() {
        return;
    }
    let options = ui::dialog_window::DialogOptions {
        title: TITLE.into(),
        size: gpui::size(
            px(WIDTH),
            px(HEIGHT + ui::dialog_window::title_height(true)),
        ),
        min_size: Some(gpui::size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
        activate: !workbench::env::background(),
        themed_title: true,
        ..ui::dialog_window::DialogOptions::default()
    };
    let mut built = None;
    ui::dialog_window::open(window, options, cx, |window, cx| {
        let dialog = cx.new(|cx| InterpretersDialog::new(root, origin, selected, window, cx));
        built = Some(dialog.clone());
        dialog
    });
    if let Some(dialog) = built {
        cx.set_global(OpenDialog(Some(dialog.downgrade())));
    }
}

/// The Add button's new interpreter (`addSdkNodeAndSelect`): selected in
/// the open dialog of the project at `root`.
pub(crate) fn select_new(root: &Path, name: &str, cx: &mut App) {
    if let Some(dialog) = open_dialog(cx) {
        dialog.update(cx, |d, cx| {
            if d.root.as_deref() == Some(root) {
                d.removed.retain(|n| n != name);
                d.table = list::table(cx);
                d.selected = Some(name.to_owned());
                d.sync_path(cx);
                cx.notify();
            }
        });
    }
}

/// Closes a dialog from its own handlers: the window being updated
/// removes itself.
fn close_window(window: &mut Window, cx: &mut App) {
    ui::dialog_window::close(window.window_handle(), cx);
    window.remove_window();
}

// ---- the list ----------------------------------------------------------------------

/// `StringUtil.naturalCompare`, the order of the list: runs of digits by
/// their value, the rest without case, then by case.
pub(super) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let digits = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut run = String::new();
                    while let Some(c) = it.next_if(char::is_ascii_digit) {
                        run.push(c);
                    }
                    run.trim_start_matches('0').to_owned()
                };
                let (n, m) = (digits(&mut x), digits(&mut y));
                let order = n.len().cmp(&m.len()).then_with(|| n.cmp(&m));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(c), Some(d)) => {
                let order = c.to_lowercase().cmp(d.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// A row of the list.
#[derive(Debug, Clone, PartialEq)]
struct Item {
    entry: Entry,
    /// In the application's list; else an environment of the project that
    /// joins it when it is changed.
    configured: bool,
}

/// The rows: the configured interpreters not removed and the project's
/// environments not configured yet, without the ones associated with other
/// projects while they are hidden, by name.
fn items(
    table: &Table,
    found: &[Entry],
    removed: &[String],
    hide_others: bool,
    root: Option<&Path>,
) -> Vec<Item> {
    let mut out: Vec<Item> = table
        .entries
        .iter()
        .filter(|e| !removed.contains(&e.name))
        .map(|e| Item {
            entry: e.clone(),
            configured: true,
        })
        .collect();
    for entry in found {
        if table.find_python(&entry.python).is_none()
            && table.find(&entry.name).is_none()
            && !removed.contains(&entry.name)
        {
            out.push(Item {
                entry: entry.clone(),
                configured: false,
            });
        }
    }
    if hide_others {
        out.retain(|i| !i.entry.associated_elsewhere(root));
    }
    out.sort_by(|a, b| natural_cmp(&a.entry.name, &b.entry.name));
    out
}

/// The flavour icon of a row.
fn icon_of(flavor: Flavor) -> IconName {
    match flavor {
        Flavor::Uv => candidates::UV_ICON,
        Flavor::Venv => VENV_ICON,
        _ => candidates::PYTHON_ICON,
    }
}

/// A field filled in as Swing's `setText` fills one: the caret at the end,
/// nothing selected.
fn fill(e: &mut Editor, text: &str, cx: &mut Context<Editor>) {
    e.set_text_selected(text, cx);
    let end = e.text().chars().count();
    e.set_selections(text::SelectionSet::caret(end), cx);
}

/// The checks of the Rename dialog (`InputValidatorEx`): the error under
/// the field, and whether OK may close it. Names are unique in the whole
/// list; the name it had is no error but changes nothing.
fn rename_check(initial: &str, text: &str, names: &[String]) -> (Option<&'static str>, bool) {
    let text = text.trim();
    let taken = names.iter().any(|n| n == text);
    let error = if text.is_empty() {
        Some(table::NAME_EMPTY)
    } else if text != initial && taken {
        Some(table::NAME_TAKEN)
    } else {
        None
    };
    (error, !text.is_empty() && !taken)
}

// ---- the dialog --------------------------------------------------------------------

/// Changes of an interpreter's details, until Apply.
#[derive(Debug, Clone, Default, PartialEq)]
struct Edit {
    /// The interpreter path typed.
    python: Option<String>,
    /// The association chosen (`None` inside: none).
    association: Option<Option<PathBuf>>,
    /// "Remove association" was clicked.
    association_removed: bool,
}

impl Edit {
    fn is_empty(&self) -> bool {
        self.python.is_none() && self.association.is_none() && !self.association_removed
    }
}

/// What the association rows show.
#[derive(Debug, Clone, PartialEq)]
struct Association {
    text: String,
    checked: bool,
    enabled: bool,
    /// The "Remove association" link.
    remove_link: bool,
}

/// The Python Interpreters dialog.
pub(crate) struct InterpretersDialog {
    root: Option<PathBuf>,
    origin: Origin,
    focus: FocusHandle,
    list_focus: FocusHandle,
    /// The list as last read.
    table: Arc<Table>,
    /// The project's environments not configured yet.
    found: Vec<Entry>,
    /// Interpreters whose executable is missing ("[invalid]").
    missing: HashSet<PathBuf>,
    /// Names removed, until Apply.
    removed: Vec<String>,
    /// Interpreters removed by Apply while the dialog is open.
    removed_applied: Vec<PathBuf>,
    edits: HashMap<String, Edit>,
    hide_others: bool,
    selected: Option<String>,
    /// The interpreter path of the selected one, and whose it shows.
    path: Entity<Editor>,
    path_for: Option<String>,
    menu: Option<MenuPopup>,
    /// Where the Add button is drawn, for its popup.
    add_bounds: Rc<Cell<Bounds<Pixels>>>,
    rename: Option<WindowHandle<RenameDialog>>,
    paths: Option<WindowHandle<PathsDialog>>,
    load: Task<()>,
    check: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for InterpretersDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl InterpretersDialog {
    fn new(
        root: Option<PathBuf>,
        origin: Origin,
        selected: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        list::ensure_loaded(cx);
        let path = editor::field::new("", None, true, window, cx);
        let subscriptions = vec![
            cx.subscribe(&path, |this, _, e: &editor::EditorEvent, cx| {
                if matches!(e, editor::EditorEvent::Edited(_)) {
                    this.path_edited(cx);
                }
            }),
            // Added, renamed or applied elsewhere: read again.
            cx.observe_global::<list::Interpreters>(|this, cx| {
                this.table = list::table(cx);
                this.check_files(cx);
                this.keep_selection(cx);
                cx.notify();
            }),
            cx.observe_global::<configuring::Configuring>(|_, cx| cx.notify()),
        ];
        let list_focus = cx.focus_handle();
        window.focus(&list_focus);
        let mut this = Self {
            root,
            origin,
            focus: cx.focus_handle(),
            list_focus,
            table: list::table(cx),
            found: Vec::new(),
            missing: HashSet::new(),
            removed: Vec::new(),
            removed_applied: Vec::new(),
            edits: HashMap::new(),
            hide_others: true,
            selected: None,
            path,
            path_for: None,
            menu: None,
            add_bounds: Rc::default(),
            rename: None,
            paths: None,
            load: Task::ready(()),
            check: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.select_python(selected.as_deref(), cx);
        this.load_found(selected, cx);
        this.check_files(cx);
        this
    }

    /// The project's environments that are not configured, read off the
    /// UI thread; then the interpreter to select.
    fn load_found(&mut self, selected: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let chosen = super::chosen(&root, cx);
        let table = self.table.clone();
        let scan = cx.background_spawn(async move {
            let state = super::scan(&root, chosen.as_ref(), &table);
            state
                .found
                .iter()
                .filter(|f| !f.configured)
                .map(|f| f.entry(&root))
                .collect::<Vec<_>>()
        });
        self.load = cx.spawn(async move |this, cx| {
            let found = scan.await;
            let _ = this.update(cx, |this, cx| {
                this.found = found;
                this.check_files(cx);
                if this.selected.is_none()
                    || selected.is_some()
                        && this.selected_item().is_none_or(|i| {
                            selected
                                .as_deref()
                                .is_some_and(|p| !table::same_path(&i.entry.python, p))
                        })
                {
                    this.select_python(selected.as_deref(), cx);
                }
                cx.notify();
            });
        });
    }

    /// Which interpreters have no executable, looked up off the UI thread.
    fn check_files(&mut self, cx: &mut Context<Self>) {
        let pythons: Vec<PathBuf> = self
            .table
            .entries
            .iter()
            .chain(&self.found)
            .map(|e| e.python.clone())
            .collect();
        let missing = cx.background_spawn(async move {
            pythons
                .into_iter()
                .filter(|p| !p.is_file())
                .collect::<HashSet<_>>()
        });
        self.check = cx.spawn(async move |this, cx| {
            let missing = missing.await;
            let _ = this.update(cx, |this, cx| {
                if this.missing != missing {
                    this.missing = missing;
                    cx.notify();
                }
            });
        });
    }

    fn items(&self) -> Vec<Item> {
        items(
            &self.table,
            &self.found,
            &self.removed,
            self.hide_others,
            self.root.as_deref(),
        )
    }

    fn selected_item(&self) -> Option<Item> {
        let name = self.selected.as_ref()?;
        self.items().into_iter().find(|i| &i.entry.name == name)
    }

    /// Selects the row of `python`, else the first one
    /// (`selectNodeInTree`, `promiseSelectFirst`).
    fn select_python(&mut self, python: Option<&Path>, cx: &mut Context<Self>) {
        let items = self.items();
        let row = python
            .and_then(|p| items.iter().find(|i| table::same_path(&i.entry.python, p)))
            .or(items.first());
        self.selected = row.map(|i| i.entry.name.clone());
        self.sync_path(cx);
    }

    fn select(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        if self.selected != name {
            self.selected = name;
            self.sync_path(cx);
            cx.notify();
        }
    }

    /// The selection is gone (removed, hidden, renamed elsewhere): the row
    /// in its place.
    fn keep_selection(&mut self, cx: &mut Context<Self>) {
        let items = self.items();
        if self
            .selected
            .as_ref()
            .is_some_and(|s| items.iter().any(|i| &i.entry.name == s))
        {
            return;
        }
        self.selected = items.first().map(|i| i.entry.name.clone());
        self.sync_path(cx);
    }

    fn move_selection(&mut self, by: isize, cx: &mut Context<Self>) {
        let items = self.items();
        if items.is_empty() {
            return;
        }
        let at = self
            .selected
            .as_ref()
            .and_then(|s| items.iter().position(|i| &i.entry.name == s));
        let next = match at {
            None => 0,
            Some(at) => (at as isize + by).clamp(0, items.len() as isize - 1) as usize,
        };
        self.select(Some(items[next].entry.name.clone()), cx);
    }

    /// The interpreter path the details show: the one typed, else the
    /// interpreter's.
    fn path_text(&self, item: &Item) -> String {
        self.edits
            .get(&item.entry.name)
            .and_then(|e| e.python.clone())
            .unwrap_or_else(|| item.entry.python.display().to_string())
    }

    /// Shows the selected interpreter's path in the field.
    fn sync_path(&mut self, cx: &mut Context<Self>) {
        let item = self.selected_item();
        let text = item.as_ref().map(|i| self.path_text(i)).unwrap_or_default();
        self.path_for = item.map(|i| i.entry.name);
        self.path.update(cx, |e, cx| fill(e, &text, cx));
    }

    fn path_edited(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.path_for.clone() else {
            return;
        };
        let Some(item) = self.items().into_iter().find(|i| i.entry.name == name) else {
            return;
        };
        let text = self.path.read(cx).text();
        let edit = self.edits.entry(name.clone()).or_default();
        edit.python = (text != item.entry.python.display().to_string()).then_some(text);
        if edit.is_empty() {
            self.edits.remove(&name);
        }
        cx.notify();
    }

    /// The association rows of an item: plain virtual environments and
    /// conda ones only.
    fn association(&self, item: &Item) -> Option<Association> {
        let root = self.root.as_deref()?;
        if !matches!(item.entry.flavor, Flavor::Venv | Flavor::Conda) {
            return None;
        }
        let edit = self.edits.get(&item.entry.name);
        let original = item.entry.associated_project.as_deref();
        let removed = edit.is_some_and(|e| e.association_removed);
        if let Some(other) = original.filter(|p| !table::same_path(p, root))
            && !removed
        {
            return Some(Association {
                text: associate_with(other),
                checked: true,
                enabled: false,
                remove_link: true,
            });
        }
        let current = match edit.and_then(|e| e.association.clone()) {
            Some(project) => project,
            None => original.map(Path::to_owned),
        };
        Some(Association {
            text: ASSOCIATE_CURRENT.into(),
            checked: current.is_some_and(|p| table::same_path(&p, root)),
            enabled: true,
            remove_link: false,
        })
    }

    fn set_associated(&mut self, on: bool, cx: &mut Context<Self>) -> Result<(), String> {
        let item = self.selected_item().ok_or("no interpreter is selected")?;
        let association = self
            .association(&item)
            .ok_or("only a virtual environment is associated with a project")?;
        if !association.enabled {
            return Err("the environment is associated with another project".into());
        }
        let root = self.root.clone();
        let name = item.entry.name.clone();
        let edit = self.edits.entry(name.clone()).or_default();
        let project = if on { root } else { None };
        let original = item.entry.associated_project.clone();
        let same = match (&project, &original) {
            (Some(a), Some(b)) => table::same_path(a, b),
            (None, None) => true,
            _ => false,
        };
        edit.association = (!same || edit.association_removed).then_some(project);
        if edit.is_empty() {
            self.edits.remove(&name);
        }
        cx.notify();
        Ok(())
    }

    fn remove_association(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let item = self.selected_item().ok_or("no interpreter is selected")?;
        if !self.association(&item).is_some_and(|a| a.remove_link) {
            return Err("the environment is not associated with another project".into());
        }
        let edit = self.edits.entry(item.entry.name).or_default();
        edit.association_removed = true;
        edit.association = Some(None);
        cx.notify();
        Ok(())
    }

    /// The changes Apply makes: an environment not configured yet joins
    /// the list first.
    fn changes(&self) -> Vec<Change> {
        let all = items(&self.table, &self.found, &[], false, self.root.as_deref());
        let mut changes: Vec<Change> = self
            .removed
            .iter()
            .filter(|n| self.table.find(n).is_some())
            .map(|n| Change::Remove(n.clone()))
            .collect();
        let mut names: Vec<&String> = self.edits.keys().collect();
        names.sort();
        for name in names {
            let edit = &self.edits[name];
            let Some(item) = all.iter().find(|i| &i.entry.name == name) else {
                continue;
            };
            if self.removed.contains(name) {
                continue;
            }
            if !item.configured {
                changes.push(Change::Add(item.entry.clone()));
            }
            if let Some(python) = edit.python.as_deref().map(str::trim)
                && !python.is_empty()
            {
                changes.push(Change::SetPython {
                    name: name.clone(),
                    python: PathBuf::from(python),
                });
            }
            if let Some(project) = &edit.association {
                changes.push(Change::SetAssociation {
                    name: name.clone(),
                    project: project.clone(),
                });
            }
        }
        changes
    }

    fn is_modified(&self) -> bool {
        !self.changes().is_empty()
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let changes = self.changes();
        if changes.is_empty() {
            return;
        }
        for change in &changes {
            match change {
                Change::Remove(name) => {
                    if let Some(e) = self.table.find(name) {
                        self.removed_applied.push(e.python.clone());
                    }
                }
                Change::SetPython { name, python } => {
                    refresh_version(name.clone(), python.clone(), cx);
                }
                _ => {}
            }
        }
        self.removed.clear();
        self.edits.clear();
        report(
            "python.interpreters.dialog.apply",
            list::change(changes, cx),
            cx,
        );
        self.table = list::table(cx);
        self.keep_selection(cx);
        self.sync_path(cx);
        cx.notify();
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        let items = self.items();
        let Some(at) = self
            .selected
            .as_ref()
            .and_then(|s| items.iter().position(|i| &i.entry.name == s))
        else {
            return;
        };
        self.removed.push(items[at].entry.name.clone());
        let rest = self.items();
        let next = rest.get(at).or(rest.last()).map(|i| i.entry.name.clone());
        self.selected = next;
        self.sync_path(cx);
        cx.notify();
    }

    fn toggle_filter(&mut self, cx: &mut Context<Self>) {
        self.hide_others = !self.hide_others;
        self.keep_selection(cx);
        cx.notify();
    }

    /// All names of the list: a new name must be none of them.
    fn names(&self) -> Vec<String> {
        items(&self.table, &self.found, &[], false, self.root.as_deref())
            .into_iter()
            .map(|i| i.entry.name)
            .collect()
    }

    /// Renames an interpreter at once (`renameSdk`); an environment not
    /// configured yet joins the list with the new name.
    fn rename_to(&mut self, name: &str, new_name: &str, cx: &mut Context<Self>) {
        let new_name = new_name.trim().to_owned();
        if new_name == name {
            return;
        }
        let all = items(&self.table, &self.found, &[], false, self.root.as_deref());
        let Some(item) = all.into_iter().find(|i| i.entry.name == name) else {
            return;
        };
        let change = if item.configured {
            Change::Rename {
                name: name.to_owned(),
                new_name: new_name.clone(),
            }
        } else {
            Change::Add(Entry {
                name: new_name.clone(),
                ..item.entry.clone()
            })
        };
        report(
            "python.interpreters.rename",
            list::change(vec![change], cx),
            cx,
        );
        if let Some(edit) = self.edits.remove(name) {
            self.edits.insert(new_name.clone(), edit);
        }
        for removed in &mut self.removed {
            if removed == name {
                *removed = new_name.clone();
            }
        }
        self.table = list::table(cx);
        if self.selected.as_deref() == Some(name) {
            self.selected = Some(new_name.clone());
        }
        if self.path_for.as_deref() == Some(name) {
            self.path_for = Some(new_name);
        }
        cx.notify();
    }

    fn open_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.selected_item() else {
            return;
        };
        self.close_menu(cx);
        let names = self.names();
        let host = cx.entity().downgrade();
        let options = ui::dialog_window::DialogOptions {
            title: RENAME_TITLE.into(),
            size: gpui::size(
                px(RENAME_WIDTH),
                px(RENAME_HEIGHT + ui::dialog_window::title_height(true)),
            ),
            min_size: Some(gpui::size(px(RENAME_WIDTH), px(RENAME_HEIGHT))),
            activate: !workbench::env::background(),
            themed_title: true,
            ..ui::dialog_window::DialogOptions::default()
        };
        let initial = item.entry.name.clone();
        self.rename = ui::dialog_window::open(window, options, cx, move |window, cx| {
            cx.new(|cx| RenameDialog::new(host, initial, names, window, cx))
        });
        cx.notify();
    }

    fn open_paths(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.selected_item() else {
            return;
        };
        self.close_menu(cx);
        let options = ui::dialog_window::DialogOptions {
            title: PATHS_TITLE.into(),
            size: gpui::size(
                px(PATHS_WIDTH),
                px(PATHS_HEIGHT + ui::dialog_window::title_height(true)),
            ),
            min_size: Some(gpui::size(px(360.0), px(240.0))),
            activate: !workbench::env::background(),
            themed_title: true,
            ..ui::dialog_window::DialogOptions::default()
        };
        let python = item.entry.python.clone();
        self.paths = ui::dialog_window::open(window, options, cx, move |window, cx| {
            cx.new(|cx| PathsDialog::new(python, window, cx))
        });
        cx.notify();
    }

    /// The Add button's popup (`collectAddInterpreterActions`): "Add Local
    /// Interpreter…", greyed while an interpreter is being configured.
    fn open_add_menu(
        &mut self,
        at: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu(cx);
        let Some(root) = self.root.clone() else {
            return;
        };
        let busy = configuring::is_configuring(&root, cx);
        let items = vec![
            MenuItem::action(super::page::ADD_LOCAL, DialogAddLocal)
                .icon(super::page::HOME_FOLDER_ICON)
                .enabled(!busy),
        ];
        let at = at.unwrap_or_else(|| self.add_bounds.get().bottom_left());
        let focus = self.list_focus.clone();
        self.menu = MenuPopup::open(
            items,
            None,
            at,
            &focus,
            !workbench::env::background(),
            window,
            cx,
            |this, cx| this.close_menu(cx),
        );
        cx.notify();
    }

    /// The list's context menu (`createActions(fromPopup = true)`).
    fn open_context_menu(
        &mut self,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu(cx);
        let items = vec![
            MenuItem::action(REMOVE, DialogRemove).icon(IconName::REMOVE),
            MenuItem::action(RENAME, DialogRename).icon(EDIT_ICON),
            MenuItem::action(SHOW_PATHS, DialogShowPaths).icon(IconName::SHOW_AS_TREE),
        ];
        let focus = self.list_focus.clone();
        self.menu = MenuPopup::open(
            items,
            None,
            at,
            &focus,
            !workbench::env::background(),
            window,
            cx,
            |this, cx| this.close_menu(cx),
        );
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take() {
            menu.close(cx);
            cx.notify();
        }
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_item().is_none() {
            return;
        }
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SPECIFY_PATH.into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let text = path.display().to_string();
                this.path.update(cx, |e, cx| fill(e, &text, cx));
                this.path_edited(cx);
            });
        })
        .detach();
    }

    /// The interpreter OK hands to the page (`storedSelectedSdk`), with the
    /// path typed for it.
    fn chosen_python(&self) -> Option<PathBuf> {
        let item = self.selected_item()?;
        let text = self.path_text(&item);
        let text = text.trim();
        Some(if text.is_empty() {
            item.entry.python
        } else {
            PathBuf::from(text)
        })
    }

    fn ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = self.chosen_python();
        let removed_now: Vec<PathBuf> = self
            .removed
            .iter()
            .filter_map(|n| self.table.find(n).map(|e| e.python.clone()))
            .collect();
        self.apply(cx);
        let mut removed = std::mem::take(&mut self.removed_applied);
        removed.extend(removed_now);
        self.close(Some(chosen), removed, window, cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let removed = std::mem::take(&mut self.removed_applied);
        self.close(None, removed, window, cx);
    }

    /// Closes the dialog; from the page, the page selects `chosen` (OK) or
    /// keeps its own interpreter unless it was removed.
    fn close(
        &mut self,
        chosen: Option<Option<PathBuf>>,
        removed: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu(cx);
        if open_dialog(cx).is_some_and(|d| d.entity_id() == cx.entity_id()) {
            cx.set_global(OpenDialog(None));
        }
        if let (Origin::Page, Some(root)) = (self.origin, self.root.clone()) {
            let selected = chosen.flatten();
            cx.defer(move |cx| {
                super::page::interpreters_dialog_closed(&root, selected, &removed, cx)
            });
        }
        close_window(window, cx);
    }

    // ---- actions ----------------------------------------------------------------------

    fn on_select(&mut self, a: &DialogSelect, _: &mut Window, cx: &mut Context<Self>) {
        if self.items().iter().any(|i| i.entry.name == a.name) {
            self.select(Some(a.name.clone()), cx);
        } else {
            eprintln!(
                "python.interpreters.dialog.select: no interpreter `{}`",
                a.name
            );
        }
    }

    fn on_add(&mut self, _: &DialogAdd, window: &mut Window, cx: &mut Context<Self>) {
        self.open_add_menu(None, window, cx);
    }

    fn on_add_local(&mut self, _: &DialogAddLocal, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(cx);
        let Some(root) = self.root.clone() else {
            return;
        };
        if configuring::is_configuring(&root, cx) {
            return;
        }
        if let Some(main) = super::widget::main_window_of(&root, cx) {
            super::add_dialog::save_all(main, cx);
        }
        super::add_dialog::open_later(window.window_handle(), root, Select::Dialog, cx);
    }

    fn on_remove(&mut self, _: &DialogRemove, _: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(cx);
        self.remove(cx);
    }

    fn on_rename(&mut self, _: &DialogRename, window: &mut Window, cx: &mut Context<Self>) {
        self.open_rename(window, cx);
    }

    fn on_toggle_filter(&mut self, _: &DialogToggleFilter, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_filter(cx);
    }

    fn on_show_paths(&mut self, _: &DialogShowPaths, window: &mut Window, cx: &mut Context<Self>) {
        self.open_paths(window, cx);
    }

    fn on_set(&mut self, a: &DialogSet, _: &mut Window, cx: &mut Context<Self>) {
        let result = match a.field.as_str() {
            "path" => {
                if self.selected_item().is_none() {
                    Err("no interpreter is selected".to_owned())
                } else {
                    let text = a.value.clone();
                    self.path.update(cx, |e, cx| fill(e, &text, cx));
                    self.path_edited(cx);
                    Ok(())
                }
            }
            "associated" => match a.value.as_str() {
                "true" => self.set_associated(true, cx),
                "false" => self.set_associated(false, cx),
                other => Err(format!("`associated` is `true` or `false`, not `{other}`")),
            },
            other => Err(format!("no field `{other}`")),
        };
        if let Err(e) = result {
            eprintln!("python.interpreters.dialog.set: {e}");
        }
    }

    fn on_remove_association(
        &mut self,
        _: &DialogRemoveAssociation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = self.remove_association(cx) {
            eprintln!("python.interpreters.dialog.remove-association: {e}");
        }
    }

    fn on_browse(&mut self, _: &DialogBrowse, window: &mut Window, cx: &mut Context<Self>) {
        self.browse(window, cx);
    }

    fn on_apply(&mut self, _: &DialogApply, _: &mut Window, cx: &mut Context<Self>) {
        self.apply(cx);
    }

    fn on_ok(&mut self, _: &DialogOk, window: &mut Window, cx: &mut Context<Self>) {
        self.ok(window, cx);
    }

    fn on_cancel(&mut self, _: &DialogCancel, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel(window, cx);
    }

    /// Keys of the list (`CommonShortcuts`): Up and Down move, Delete (and
    /// Backspace on macOS) removes, Shift+F6 renames, Control+N on macOS
    /// and Insert elsewhere opens the Add popup.
    fn on_list_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &e.keystroke.modifiers;
        let key = e.keystroke.key.as_str();
        let plain = !(m.control || m.alt || m.platform || m.shift);
        let add = if cfg!(target_os = "macos") {
            key == "n" && m.control && !(m.alt || m.platform || m.shift)
        } else {
            key == "insert" && plain
        };
        match key {
            "up" if plain => self.move_selection(-1, cx),
            "down" if plain => self.move_selection(1, cx),
            "delete" if plain => self.remove(cx),
            "backspace" if plain && cfg!(target_os = "macos") => self.remove(cx),
            "f6" if m.shift && !(m.control || m.alt || m.platform) => self.open_rename(window, cx),
            _ if add => self.open_add_menu(None, window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    // ---- state ------------------------------------------------------------------------

    fn state_json(&self, cx: &App) -> Json {
        let items = self.items();
        let selected = self.selected_item();
        let has_selection = selected.is_some();
        let busy = self
            .root
            .as_deref()
            .is_some_and(|r| configuring::is_configuring(r, cx));
        let details = selected.as_ref().map(|item| {
            json!({
                "label": INTERPRETER_PATH,
                "path": self.path.read(cx).text(),
                "association": self.association(item).map(|a| json!({
                    "text": a.text,
                    "checked": a.checked,
                    "enabled": a.enabled,
                    "remove-link": a.remove_link.then_some(REMOVE_ASSOCIATION),
                })),
            })
        });
        json!({
            "title": TITLE,
            "project": self.root.as_ref().map(|r| r.display().to_string()),
            "origin": match self.origin {
                Origin::Page => "page",
                Origin::Window => "window",
            },
            "toolbar": [
                {"id": "add", "tooltip": ADD, "enabled": self.root.is_some()},
                {"id": "remove", "tooltip": REMOVE, "enabled": has_selection},
                {"id": "rename", "tooltip": RENAME, "enabled": has_selection},
                {"id": "hide-others", "tooltip": HIDE_OTHERS, "enabled": true, "on": self.hide_others},
                {"id": "show-paths", "tooltip": SHOW_PATHS, "enabled": has_selection},
            ],
            "items": items.iter().map(|i| json!({
                "name": i.entry.name,
                "version": i.entry.version,
                "path": i.entry.python.display().to_string(),
                "kind": i.entry.flavor.id(),
                "icon": icon_of(i.entry.flavor).0,
                "configured": i.configured,
                "associated-project": i.entry.associated_project.as_ref().map(|p| p.display().to_string()),
                "invalid": self.missing.contains(&i.entry.python),
                "selected": self.selected.as_ref() == Some(&i.entry.name),
            })).collect::<Vec<_>>(),
            "selected": self.selected,
            "details": details,
            "removed": self.removed,
            "modified": self.is_modified(),
            "add-enabled": self.root.is_some() && !busy,
            "menu": self.menu.as_ref().map(|m| m.menu.read(cx).state_json()),
            "rename": self.rename.as_ref().and_then(|h| h.read(cx).ok()).map(|r| r.state_json(cx)),
            "paths": self.paths.as_ref().and_then(|h| h.read(cx).ok()).map(PathsDialog::state_json),
        })
    }

    // ---- drawing ----------------------------------------------------------------------

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let has_selection = self.selected.is_some();
        let tool = |id: &'static str,
                    icon: IconName,
                    tooltip: &'static str,
                    enabled: bool,
                    action: Box<dyn gpui::Action>,
                    cx: &App| {
            ui::components::small_icon_button(
                SharedString::from(format!("interpreters-{id}")),
                icon,
                enabled,
                cx,
            )
            .debug_selector(move || format!("interpreters:toolbar:{id}"))
            .tooltip(Tooltip::text(tooltip, None))
            .when(enabled, |d| {
                d.on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            })
        };
        let bounds = self.add_bounds.clone();
        let add = tool(
            "add",
            IconName::ADD,
            ADD,
            self.root.is_some(),
            Box::new(DialogAdd),
            cx,
        )
        .relative()
        .child(
            canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                .absolute()
                .size_full(),
        );
        let filter = ui::components::toggle_button(
            "interpreters-hide-others",
            IconName::FILTER,
            self.hide_others,
            cx,
        )
        .debug_selector(|| "interpreters:toolbar:hide-others".into())
        .tooltip(Tooltip::text(HIDE_OTHERS, None))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(DialogToggleFilter), cx));
        div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h(px(TOOLBAR_HEIGHT))
            .pl(px(TOOLBAR_PAD))
            .gap(px(TOOL_GAP))
            .child(add)
            .child(tool(
                "remove",
                IconName::REMOVE,
                REMOVE,
                has_selection,
                Box::new(DialogRemove),
                cx,
            ))
            .child(tool(
                "rename",
                EDIT_ICON,
                RENAME,
                has_selection,
                Box::new(DialogRename),
                cx,
            ))
            .child(filter)
            .child(tool(
                "show-paths",
                IconName::SHOW_AS_TREE,
                SHOW_PATHS,
                has_selection,
                Box::new(DialogShowPaths),
                cx,
            ))
            .into_any_element()
    }

    fn render_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let focused = self.list_focus.contains_focused(window, cx);
        let radius = theme.metric(themes::Metric::CornerRadius);
        let (selection, selection_text) = if focused {
            (
                theme.color(UiRole::Selection),
                Some(theme.color(UiRole::SelectionText)),
            )
        } else {
            (theme.color(UiRole::SelectionInactive), None)
        };
        let muted = theme.color(UiRole::TextMuted);
        let danger = theme.color(UiRole::Danger);
        let small = theme.ui_size() - px(2.0);
        let rows: Vec<AnyElement> = self
            .items()
            .into_iter()
            .enumerate()
            .map(|(ix, item)| {
                let selected = self.selected.as_ref() == Some(&item.entry.name);
                let invalid = self.missing.contains(&item.entry.python);
                let grey = if selected {
                    selection_text.unwrap_or(muted)
                } else {
                    muted
                };
                let name = if invalid {
                    format!("[{INVALID}] {}", item.entry.name)
                } else {
                    item.entry.name.clone()
                };
                let row_name = item.entry.name.clone();
                let menu_name = item.entry.name.clone();
                div()
                    .id(("interpreters-row", ix))
                    .debug_selector({
                        let n = item.entry.name.clone();
                        move || format!("interpreters:row:{n}")
                    })
                    .flex()
                    .flex_none()
                    .flex_row()
                    .items_center()
                    .h(px(ROW))
                    .pl(px(ROW_PAD))
                    .pr(px(ROW_PAD * 2.0))
                    .gap(px(ICON_GAP))
                    .rounded(radius)
                    .whitespace_nowrap()
                    .when(selected, |d| d.bg(selection))
                    .when_some(selection_text.filter(|_| selected), |d, c| d.text_color(c))
                    .child(Icon::new(icon_of(item.entry.flavor)).into_any_element())
                    .child(
                        div()
                            .flex_none()
                            .when(invalid, |d| d.text_color(danger))
                            .child(name),
                    )
                    .children(item.entry.version.clone().map(|v| {
                        div()
                            .flex_none()
                            .ml(px(4.0))
                            .text_size(small)
                            .text_color(grey)
                            .child(v)
                    }))
                    .child(
                        div()
                            .flex_none()
                            .ml(px(4.0))
                            .text_size(small)
                            .text_color(grey)
                            .child(item.entry.python.display().to_string()),
                    )
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        window.focus(&this.list_focus);
                        this.select(Some(row_name.clone()), cx);
                        if e.click_count() >= 2 {
                            this.open_rename(window, cx);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            window.focus(&this.list_focus);
                            this.select(Some(menu_name.clone()), cx);
                            this.open_context_menu(e.position, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id("interpreters-list")
            .debug_selector(|| "interpreters:list".into())
            .key_context("PythonInterpretersList")
            .track_focus(&self.list_focus)
            .on_key_down(cx.listener(Self::on_list_key))
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_row()
            .overflow_scroll()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_col()
                    .min_w_full()
                    .children(rows),
            )
            .into_any_element()
    }

    fn render_details(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let details = div()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .pt(px(DETAILS_TOP))
            .px(px(DETAILS_SIDE))
            .flex()
            .flex_col()
            .gap(px(DETAIL_ROW_GAP));
        let Some(item) = self.selected_item() else {
            return details.into_any_element();
        };
        let field = editor::field::browse_frame(
            &self.path,
            false,
            "interpreters-path-browse",
            Some(Box::new(DialogBrowse)),
            window,
            cx,
        )
        .debug_selector(|| "interpreters:path".into())
        .flex_1()
        .min_w(px(0.0))
        .h(px(FIELD));
        let path_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(LABEL_GAP))
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .child(INTERPRETER_PATH),
            )
            .child(field);
        let association = self.association(&item).map(|a| {
            let enabled = a.enabled;
            let checked = a.checked;
            let check = ui::form::check_row(
                "interpreters-associate",
                "interpreters-associate-check",
                a.checked,
                div().child(a.text.clone()),
                cx,
            )
            .debug_selector(|| "interpreters:associate".into())
            .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
            .when(enabled, |d| {
                d.on_click(move |_, window, cx| {
                    window.dispatch_action(
                        Box::new(DialogSet {
                            field: "associated".into(),
                            value: (!checked).to_string(),
                        }),
                        cx,
                    )
                })
            });
            let link = a.remove_link.then(|| {
                ui::link::link("interpreters-remove-association", REMOVE_ASSOCIATION, cx)
                    .debug_selector(|| "interpreters:remove-association".into())
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(DialogRemoveAssociation), cx)
                    })
            });
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(DETAIL_ROW_GAP))
                .child(check)
                .children(link)
        });
        details
            .child(path_row)
            .children(association)
            .into_any_element()
    }

    fn render_footer(&self, cx: &App) -> AnyElement {
        BottomBar::new()
            .compact(true)
            .button(BarButton::new(
                "interpreters-ok",
                "OK",
                ButtonRole::Ok,
                Box::new(DialogOk),
            ))
            .button(BarButton::new(
                "interpreters-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(DialogCancel),
            ))
            .button(
                BarButton::new(
                    "interpreters-apply",
                    "Apply",
                    ButtonRole::Apply,
                    Box::new(DialogApply),
                )
                .enabled(self.is_modified()),
            )
            .render(cx)
    }
}

impl Render for InterpretersDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (surface, text, border) = (
            theme.color(UiRole::Surface),
            theme.color(UiRole::Text),
            theme.color(UiRole::Border),
        );
        let family = theme.ui_family();
        let size = theme.ui_size();
        let master =
            ((f32::from(window.viewport_size().width) - INSET_LEFT) * MASTER_SHARE).floor();
        let toolbar = self.render_toolbar(cx);
        let list = self.render_list(window, cx);
        let details = self.render_details(window, cx);
        let footer = self.render_footer(cx);
        div()
            .id("python-interpreters")
            .debug_selector(|| "python-interpreters".into())
            .key_context("PythonInterpretersDialog")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_select))
            .on_action(cx.listener(Self::on_add))
            .on_action(cx.listener(Self::on_add_local))
            .on_action(cx.listener(Self::on_remove))
            .on_action(cx.listener(Self::on_rename))
            .on_action(cx.listener(Self::on_toggle_filter))
            .on_action(cx.listener(Self::on_show_paths))
            .on_action(cx.listener(Self::on_set))
            .on_action(cx.listener(Self::on_remove_association))
            .on_action(cx.listener(Self::on_browse))
            .on_action(cx.listener(Self::on_apply))
            .on_action(cx.listener(Self::on_ok))
            .on_action(cx.listener(Self::on_cancel))
            // Enter and Escape in the path field (the editor's own
            // actions): OK and Cancel, the default buttons.
            .capture_action(
                cx.listener(|this, _: &editor::commands::Newline, window, cx| {
                    this.ok(window, cx);
                    cx.stop_propagation();
                }),
            )
            .capture_action(
                cx.listener(|this, _: &editor::commands::Escape, window, cx| {
                    this.cancel(window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let m = &e.keystroke.modifiers;
                if m.control || m.alt || m.platform || m.shift {
                    return;
                }
                match e.keystroke.key.as_str() {
                    "enter" => this.ok(window, cx),
                    "escape" => this.cancel(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(surface)
            .text_color(text)
            .font_family(family)
            .text_size(size)
            .children(ui::dialog_window::title_bar(TITLE, true, cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_row()
                    .pt(px(INSET_TOP))
                    .pl(px(INSET_LEFT))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_none()
                            .w(px(master))
                            .h_full()
                            .child(toolbar)
                            .child(list),
                    )
                    .child(div().flex_none().w(px(1.0)).h_full().bg(border))
                    .child(details),
            )
            .child(footer)
            .children(ui::dialog_window::blocker(window, cx))
            .child(ui::pointer::frame())
    }
}

// ---- Rename --------------------------------------------------------------------------

/// The "Rename Python Interpreter" dialog (`Messages.showInputDialog` with
/// the reference's validator).
pub(crate) struct RenameDialog {
    host: WeakEntity<InterpretersDialog>,
    initial: String,
    names: Vec<String>,
    field: Entity<Editor>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for RenameDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.field.focus_handle(cx)
    }
}

impl RenameDialog {
    fn new(
        host: WeakEntity<InterpretersDialog>,
        initial: String,
        names: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = editor::field::new(&initial, None, true, window, cx);
        field.update(cx, |e, cx| e.set_text_selected(&initial, cx));
        window.focus(&field.focus_handle(cx));
        let subscriptions = vec![cx.subscribe(&field, |_, _, e: &editor::EditorEvent, cx| {
            if matches!(e, editor::EditorEvent::Edited(_)) {
                cx.notify();
            }
        })];
        Self {
            host,
            initial,
            names,
            field,
            _subscriptions: subscriptions,
        }
    }

    fn check(&self, cx: &App) -> (Option<&'static str>, bool) {
        rename_check(&self.initial, &self.field.read(cx).text(), &self.names)
    }

    fn ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.check(cx).1 {
            return;
        }
        let new_name = self.field.read(cx).text().trim().to_owned();
        let initial = self.initial.clone();
        let _ = self
            .host
            .update(cx, |host, cx| host.rename_to(&initial, &new_name, cx));
        close_window(window, cx);
    }

    fn state_json(&self, cx: &App) -> Json {
        let (error, ok) = self.check(cx);
        json!({
            "title": RENAME_TITLE,
            "label": NEW_NAME,
            "text": self.field.read(cx).text(),
            "error": error,
            "ok-enabled": ok,
        })
    }
}

impl Render for RenameDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (error, ok) = self.check(cx);
        let theme = cx.theme();
        let danger = theme.color(UiRole::Danger);
        let field = editor::field::frame(&self.field, true, error.is_some(), window, cx)
            .debug_selector(|| "rename-interpreter:field".into())
            .h(px(FIELD));
        let footer = BottomBar::new()
            .separator(false)
            .compact(true)
            .button(
                BarButton::new(
                    "rename-interpreter-ok",
                    "OK",
                    ButtonRole::Ok,
                    Box::new(RenameOk),
                )
                .enabled(ok),
            )
            .button(BarButton::new(
                "rename-interpreter-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(RenameCancel),
            ))
            .render(cx);
        div()
            .id("rename-interpreter")
            .debug_selector(|| "rename-interpreter".into())
            .key_context("RenamePythonInterpreterDialog")
            .on_action(cx.listener(|this, a: &RenameSet, _, cx| {
                let text = a.text.clone();
                this.field
                    .update(cx, |e, cx| e.set_text_selected(&text, cx));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RenameOk, window, cx| this.ok(window, cx)))
            .on_action(cx.listener(|_, _: &RenameCancel, window, cx| close_window(window, cx)))
            .capture_action(
                cx.listener(|this, _: &editor::commands::Newline, window, cx| {
                    this.ok(window, cx);
                    cx.stop_propagation();
                }),
            )
            .capture_action(cx.listener(|_, _: &editor::commands::Escape, window, cx| {
                close_window(window, cx);
                cx.stop_propagation();
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.color(UiRole::Surface))
            .text_color(theme.color(UiRole::Text))
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .children(ui::dialog_window::title_bar(RENAME_TITLE, true, cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .pt(px(PAD_TOP))
                    .pl(px(PAD_LEFT))
                    .pr(px(PAD_RIGHT))
                    .flex()
                    .flex_col()
                    .child(div().pb(px(MESSAGE_GAP)).child(NEW_NAME))
                    .child(field)
                    .children(error.map(|e| {
                        div()
                            .debug_selector(|| "rename-interpreter:error".into())
                            .pt(px(4.0))
                            .text_color(danger)
                            .child(e)
                    })),
            )
            .child(footer)
            .child(ui::pointer::frame())
    }
}

// ---- Interpreter Paths ---------------------------------------------------------------

/// The "Interpreter Paths" dialog (`PythonPathDialog` with
/// `PythonPathEditor`): the folders the interpreter imports from, read
/// from it off the UI thread; Reload reads them again.
pub(crate) struct PathsDialog {
    python: PathBuf,
    /// `None` while they are read.
    paths: Option<Result<Vec<PathBuf>, String>>,
    focus: FocusHandle,
    task: Task<()>,
}

impl Focusable for PathsDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PathsDialog {
    fn new(python: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut this = Self {
            python,
            paths: None,
            focus,
            task: Task::ready(()),
        };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.paths = None;
        let python = self.python.clone();
        let work = runtime::spawn(cx, move |token| async move {
            let ctx = py_env::RunContext::default().with_cancel(token);
            py_env::interpreter::sys_path(&python, PATHS_TIMEOUT, &ctx)
                .await
                .map_err(|e| e.to_string())
        });
        self.task = cx.spawn(async move |this, cx| {
            let paths = work.await;
            let _ = this.update(cx, |this, cx| {
                this.paths = Some(paths);
                cx.notify();
            });
        });
        cx.notify();
    }

    fn state_json(&self) -> Json {
        json!({
            "title": PATHS_TITLE,
            "python": self.python.display().to_string(),
            "loading": self.paths.is_none(),
            "paths": match &self.paths {
                Some(Ok(paths)) => paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                _ => Vec::new(),
            },
            "error": match &self.paths {
                Some(Err(e)) => Some(e.clone()),
                _ => None,
            },
        })
    }
}

impl Render for PathsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let border = theme.color(UiRole::Border);
        let muted = theme.color(UiRole::TextMuted);
        let toolbar = div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h(px(TOOLBAR_HEIGHT + 2.0))
            .px(px(3.0))
            .gap(px(TOOL_GAP))
            .border_b_1()
            .border_color(border)
            // Adding and excluding paths come with the analysis that uses
            // them.
            .child(
                ui::components::small_icon_button(
                    "interpreter-paths-add",
                    IconName::ADD,
                    false,
                    cx,
                )
                .tooltip(Tooltip::text(ADD_PATH, None)),
            )
            .child(
                ui::components::small_icon_button(
                    "interpreter-paths-remove",
                    IconName::REMOVE,
                    false,
                    cx,
                )
                .tooltip(Tooltip::text(REMOVE_PATH, None)),
            )
            .child(
                ui::components::small_icon_button(
                    "interpreter-paths-reload",
                    IconName::REFRESH,
                    true,
                    cx,
                )
                .debug_selector(|| "interpreter-paths:reload".into())
                .tooltip(Tooltip::text(RELOAD_PATHS, None))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(PathsReload), cx)),
            );
        let body: AnyElement = match &self.paths {
            None => div()
                .p(px(8.0))
                .child(ui::components::spinner("interpreter-paths-busy", cx))
                .into_any_element(),
            Some(Err(e)) => div()
                .p(px(8.0))
                .text_color(theme.color(UiRole::Danger))
                .child(e.clone())
                .into_any_element(),
            Some(Ok(paths)) => div()
                .id("interpreter-paths-list")
                .flex_1()
                .min_h(px(0.0))
                .overflow_scroll()
                .py(px(2.0))
                .children(paths.iter().map(|p| {
                    div()
                        .h(px(ROW))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .whitespace_nowrap()
                        .child(p.display().to_string())
                }))
                .into_any_element(),
        };
        let footer = BottomBar::new()
            .separator(false)
            .button(BarButton::new(
                "interpreter-paths-ok",
                "OK",
                ButtonRole::Ok,
                Box::new(PathsOk),
            ))
            .button(BarButton::new(
                "interpreter-paths-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(PathsCancel),
            ))
            .render(cx);
        div()
            .id("interpreter-paths")
            .debug_selector(|| "interpreter-paths".into())
            .key_context("InterpreterPathsDialog")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &PathsReload, _, cx| this.reload(cx)))
            .on_action(cx.listener(|_, _: &PathsOk, window, cx| close_window(window, cx)))
            .on_action(cx.listener(|_, _: &PathsCancel, window, cx| close_window(window, cx)))
            .on_key_down(cx.listener(|_, e: &KeyDownEvent, window, cx| {
                let m = &e.keystroke.modifiers;
                if m.control || m.alt || m.platform || m.shift {
                    return;
                }
                if matches!(e.keystroke.key.as_str(), "enter" | "escape") {
                    close_window(window, cx);
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.color(UiRole::Surface))
            .text_color(theme.color(UiRole::Text))
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .children(ui::dialog_window::title_bar(PATHS_TITLE, true, cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .pt(px(PAD_TOP))
                    .px(px(PAD_LEFT))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .flex()
                            .flex_col()
                            .border_1()
                            .border_color(border)
                            .child(toolbar)
                            .child(body)
                            .when(
                                self.paths
                                    .as_ref()
                                    .is_some_and(|p| p.as_ref().is_ok_and(Vec::is_empty)),
                                |d| {
                                    d.child(
                                        div()
                                            .flex_1()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_color(muted)
                                            .child("Nothing to show"),
                                    )
                                },
                            ),
                    ),
            )
            .child(footer)
            .child(ui::pointer::frame())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, flavor: Flavor, project: Option<&str>) -> Entry {
        let mut e = Entry::new(name, format!("/x/{name}/bin/python"), flavor);
        e.associated_project = project.map(PathBuf::from);
        e
    }

    #[test]
    fn names_in_natural_order() {
        let mut names = vec![
            "Python 3.10",
            "python 3.9",
            "Python 3.14",
            "a2",
            "a10",
            "A1",
            "b",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            [
                "A1",
                "a2",
                "a10",
                "b",
                "python 3.9",
                "Python 3.10",
                "Python 3.14"
            ]
        );
        assert_eq!(natural_cmp("x01", "x1"), "x01".cmp("x1"));
    }

    /// The rows: configured ones and the project's environments not
    /// configured, other projects' environments while they are hidden,
    /// removed ones left out.
    #[test]
    fn rows_of_the_list() {
        let table = Table {
            entries: vec![
                entry("Python 3.14", Flavor::System, None),
                entry("other", Flavor::Venv, Some("/b")),
                entry("mine", Flavor::Venv, Some("/a")),
            ],
            ..Table::default()
        };
        let found = vec![
            entry("~/a/.venv", Flavor::Uv, Some("/a")),
            // Configured already (by path).
            entry("mine", Flavor::Venv, Some("/a")),
        ];
        let names = |rows: Vec<Item>| -> Vec<(String, bool)> {
            rows.into_iter()
                .map(|i| (i.entry.name, i.configured))
                .collect()
        };
        let root = Some(Path::new("/a"));
        assert_eq!(
            names(items(&table, &found, &[], true, root)),
            [
                ("mine".to_owned(), true),
                ("Python 3.14".to_owned(), true),
                ("~/a/.venv".to_owned(), false),
            ]
        );
        assert_eq!(
            names(items(&table, &found, &["mine".into()], false, root)),
            [
                ("other".to_owned(), true),
                ("Python 3.14".to_owned(), true),
                ("~/a/.venv".to_owned(), false),
            ]
        );
    }

    /// `InputValidatorEx` of the Rename action.
    #[test]
    fn rename_names_are_not_blank_and_not_taken() {
        let names = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(rename_check("a", "c", &names), (None, true));
        assert_eq!(rename_check("a", "  c ", &names), (None, true));
        assert_eq!(
            rename_check("a", "", &names),
            (Some(table::NAME_EMPTY), false)
        );
        assert_eq!(
            rename_check("a", "   ", &names),
            (Some(table::NAME_EMPTY), false)
        );
        assert_eq!(
            rename_check("a", "b", &names),
            (Some(table::NAME_TAKEN), false)
        );
        // Its own name: no error, but OK changes nothing.
        assert_eq!(rename_check("a", "a", &names), (None, false));
    }
}
