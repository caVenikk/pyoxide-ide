//! The Add Python Interpreter dialog (the reference IDE's
//! `PythonAddLocalInterpreterDialog` with `PythonAddCustomInterpreter`):
//! "Environment:" Generate new or Select existing, "Type:" Virtualenv or uv
//! (new), Python or uv (existing), and the rows of each:
//!
//! - Generate new / Virtualenv (`EnvironmentCreatorVenv`): Base Python,
//!   Location, "Inherit packages from base interpreter", "Make available
//!   to all projects";
//! - Generate new / uv (`EnvironmentCreatorUv`): Python version, Path to
//!   uv, Location with the alert of an environment that is there;
//! - Select existing / Python (`PythonExistingEnvironmentSelector`):
//!   Python path;
//! - Select existing / uv (`UvExistingEnvironmentSelector`): Path to uv,
//!   Environment.
//!
//! Everything it looks up runs on the interpreter runtime: the
//! interpreters of the machine, uv and its Python versions, what is at a
//! location. A path typed into a field is checked on Enter or when the
//! field loses the focus (`ValidatedPathField`); meanwhile OK is disabled.
//! OK hands the choice to `python.interpreter.add` and closes at once; the
//! work goes on while the project is marked as being configured.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use editor::Editor;
use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, ClickEvent, Context, Entity,
    FocusHandle, Focusable, Global, IntoElement, KeyDownEvent, Pixels, Render, SharedString,
    Subscription, Task, WeakEntity, Window, canvas, div, point, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::components::{DISABLED_ALPHA, Tooltip};
use ui::form::{BarButton, BottomBar, ButtonRole, MenuPopup};
use ui::icons::{Icon, IconName, IconSvg};
use ui::menu::{MenuItem, PopupOptions};
use ui::theme::ThemeExt;

use super::candidates::{self, Candidate, Machine, Origin};
use super::create::{self, Kind, LocationCheck, Request, Select};
use super::table::Table;
use super::{configuring, list, runtime};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Open the dialog.
    AddLocalInterpreter => "python.interpreter.add-local", "Add Local Interpreter\u{2026}",
        "Opens the Add Python Interpreter dialog for the project: over the Settings window from the Interpreter page (the new interpreter is selected there until Apply), else over the main window (it becomes the project's interpreter at once).";
}

ui::ui_commands! {
    DIALOG_COMMANDS, scope = "AddPythonInterpreterDialog";
    /// Set a field.
    DialogSet { field: String, value: String } => "python.interpreter.add-dialog.set", "Set Field",
        "Sets a field of the Add Python Interpreter dialog: `environment` (`generate-new`, `select-existing`), `type` (`virtualenv`, `uv`, `python`), `base-python`, `python`, `uv-environment` (an interpreter's path; one not listed is asked and added), `location`, `uv-path` (a path, checked at once), `python-version` (`default`, `3.13`), `inherit-site-packages`, `available-to-all` (`true`, `false`).",
        r#"{"field": "location", "value": "/work/demo/.venv"}"#;
    /// Open a list.
    DialogOpenList { list: String } => "python.interpreter.add-dialog.open-list", "Open List",
        "Opens a list of the Add Python Interpreter dialog: `type`, `base-python`, `python-version`, `python`, `uv-environment`.",
        r#"{"list": "type"}"#;
    /// Check the path in the focused field.
    DialogValidate => "python.interpreter.add-dialog.validate", "Validate",
        "Checks the path typed into the dialog's path fields (Enter in the field).";
    /// Choose a path for a field.
    DialogBrowse { field: String } => "python.interpreter.add-dialog.browse", "Browse",
        "Chooses a path for a field of the dialog in the system's dialog: `base-python`, `python`, `uv-environment`, `location`, `uv-path`.",
        r#"{"field": "location"}"#;
    /// Override the uv environment at the location.
    DialogOverride => "python.interpreter.add-dialog.override", "Override Existing Environment",
        "The link of the alert under Location (Generate new / uv): the environment that is there will be replaced.";
    /// Go to the interpreter at the location.
    DialogSelectExisting => "python.interpreter.add-dialog.select-existing", "Select Existing Interpreter",
        "The link of the alert under Location: switches to Select existing with the interpreter that is there.";
    /// Install uv.
    DialogInstallUv => "python.interpreter.add-dialog.install-uv", "Install uv",
        "The link of \"No uv executable found\": opens uv's installation guide.";
    /// Learn about the supported versions.
    DialogLearnMore => "python.interpreter.add-dialog.learn-more", "Learn More",
        "The link under an unsupported Python: opens the page of the supported Python versions.";
    /// OK.
    DialogOk => "python.interpreter.add-dialog.ok", "OK",
        "Adds the interpreter the dialog describes (`python.interpreter.add`) and closes it.";
    /// Cancel.
    DialogCancel => "python.interpreter.add-dialog.cancel", "Cancel",
        "Closes the dialog without adding an interpreter.";
}

/// Texts of the reference (PyBundle).
pub const TITLE: &str = "Add Python Interpreter";
const ENVIRONMENT: &str = "Environment:";
const GENERATE_NEW: &str = "Generate new";
const SELECT_EXISTING: &str = "Select existing";
const TYPE: &str = "Type:";
const VIRTUALENV: &str = "Virtualenv";
const UV: &str = "uv";
const PYTHON: &str = "Python";
const BASE_PYTHON: &str = "Base Python:";
const LOCATION: &str = "Location:";
const INHERIT: &str = "Inherit packages from base interpreter";
const ALL_PROJECTS: &str = "Make available to all projects";
const PYTHON_VERSION: &str = "Python version:";
const DEFAULT_VERSION: &str = "Default";
const PATH_TO_UV: &str = "Path to uv:";
const PYTHON_PATH: &str = "Python path:";
const UV_ENVIRONMENT: &str = "Environment:";
const SELECT_EXISTING_LINK: &str = "Select existing interpreter";
const OVERRIDE_LINK: &str = "Override existing environment";
const NO_UV: &str = "No uv executable found";
const INSTALL_UV: &str = "Install uv";
const SELECT_PATH: &str = "Select path";
const LEARN_MORE: &str = "Learn more";
const LOADING: &str = "Loading environments\u{2026}";
const NOTHING_FOUND: &str = "No interpreters were found. Select manually using the Browse button.";
const PLEASE_WAIT: &str = "Please wait\u{2026}";
const WAIT_FOR_VALIDATION: &str = "Wait for the executable validation";
const PRESS_ENTER: &str = "Press Enter to validate";
const NOT_DETECTED: &str = "Executable is not detected";
const DOES_NOT_EXIST: &str = "Executable does not exist";
const IS_DIRECTORY: &str = "Path cannot be a directory";
const NOT_EXECUTABLE: &str = "Binary is not executable";
const BROWSE_TOOLTIP: &str = "Browse\u{2026}";
const SELECT_INTERPRETER_TITLE: &str = "Select Python Interpreter";
/// Where "Learn more" and "Install uv" go (`sdk.create.python.versions.
/// help.url`; uv's own guide, as the product decided for now).
const VERSIONS_URL: &str = "https://www.jetbrains.com/help/pycharm/python.html#support";
const UV_INSTALL_URL: &str = "https://docs.astral.sh/uv/";

/// Metrics measured on the reference (logical px): the dialog's content
/// (`setSize(640, 320)` grown to its rows and the compact bottom bar: the
/// window is 350 high with the title), its insets, the label column, the
/// fields (24 high), the gaps between rows, the Type combo's width
/// (`widthGroup`), the notices under a field (`ValidationTooltip`).
const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 322.0;
const MIN_SIZE: (f32, f32) = (480.0, 260.0);
const PAD_TOP: f32 = 12.0;
const PAD_LEFT: f32 = 12.0;
const PAD_RIGHT: f32 = 15.0;
/// The label column: the widest label shown and this gap, but not
/// narrower than `LABEL_WIDTH` ("Environment:" and "Base Python:" give 96.5,
/// "Python version:" 100).
const LABEL_GAP: f32 = 4.5;
const LABEL_WIDTH: f32 = 96.5;
const FIELD: f32 = 24.0;
const FIRST_GAP: f32 = 8.0;
const ROW_GAP: f32 = 12.0;
const NOTICE_GAP: f32 = 10.0;
const CHECK_ROW: f32 = 20.0;
const CHECK_PITCH: f32 = 29.0;
const TYPE_WIDTH: f32 = 135.0;
/// The Python version combo is as wide as its widest value, "Default".
const VERSION_WIDTH: f32 = 84.0;
/// A list popup under a combo: wider than the combo by `LIST_GROW`, starting
/// `LIST_LEFT` before it and `LIST_DROP` below it; its rows are `LIST_ROW`
/// high (the types' `TYPE_ROW`, the versions' `VERSION_ROW`).
const LIST_GROW: f32 = 6.5;
const LIST_LEFT: f32 = 3.0;
const LIST_DROP: f32 = 4.5;
const LIST_ROW: f32 = 25.0;
const TYPE_ROW: f32 = 23.0;
const VERSION_ROW: f32 = 20.0;
const OPTION_GAP: f32 = 16.0;
const ARROW_WIDTH: f32 = 22.0;
const NOTICE_PAD: (f32, f32) = (7.0, 12.0);
const NOTICE_INLINE_PAD: f32 = 8.0;
const NOTICE_ICON_GAP: f32 = 8.0;
const NOTICE_LINK_GAP: f32 = 4.0;
const NOTICE_RADIUS: f32 = 5.0;
const ERROR_RING: f32 = 2.0;

/// Icons of the dialog (New UI icons of the platform).
const CHECKMARK_ICON: IconName = IconName("interpreter/greenCheckmark");
const EXTERNAL_LINK_ICON: IconName = IconName("interpreter/externalLink");
pub(super) const ICONS: [IconSvg; 3] = [
    IconSvg {
        name: candidates::DOWNLOAD_ICON.0,
        light: include_bytes!("../../assets/icons/download.svg"),
        dark: Some(include_bytes!("../../assets/icons/download_dark.svg")),
    },
    IconSvg {
        name: CHECKMARK_ICON.0,
        light: include_bytes!("../../assets/icons/greenCheckmark.svg"),
        dark: Some(include_bytes!("../../assets/icons/greenCheckmark_dark.svg")),
    },
    IconSvg {
        name: EXTERNAL_LINK_ICON.0,
        light: include_bytes!("../../assets/icons/externalLink.svg"),
        dark: Some(include_bytes!("../../assets/icons/externalLink_dark.svg")),
    },
];

/// Field and list ids of the commands.
const F_ENVIRONMENT: &str = "environment";
const F_TYPE: &str = "type";
const F_BASE: &str = "base-python";
const F_LOCATION: &str = "location";
const F_VERSION: &str = "python-version";
const F_UV: &str = "uv-path";
const F_PYTHON: &str = "python";
const F_UV_ENV: &str = "uv-environment";
const F_INHERIT: &str = "inherit-site-packages";
const F_ALL: &str = "available-to-all";

// ---- the machine the dialog looks at ------------------------------------------------

/// The machine of the tests (where interpreters are searched, the tools).
pub(crate) struct MachineOverride(pub Machine);

impl Global for MachineOverride {}

/// The machine, read off the UI thread.
fn machine(cx: &App) -> Task<Machine> {
    match cx.try_global::<MachineOverride>() {
        Some(over) => Task::ready(over.0.clone()),
        None => cx.background_spawn(async { Machine::from_process() }),
    }
}

// ---- the open dialog -------------------------------------------------------------------

/// The open dialog, for `python.interpreter.state`.
#[derive(Default)]
struct OpenDialog(Option<WeakEntity<AddDialog>>);

impl Global for OpenDialog {}

/// The open dialog's state, `null` without one.
pub(crate) fn state_json(cx: &App) -> Json {
    cx.try_global::<OpenDialog>()
        .and_then(|d| d.0.as_ref()?.upgrade())
        .map(|d| d.read(cx).state_json(cx))
        .unwrap_or(Json::Null)
}

/// Opens the dialog for the project at `root` over `window`.
pub(crate) fn open(root: PathBuf, select: Select, window: &mut Window, cx: &mut App) {
    if configuring::is_configuring(&root, cx) {
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
        let dialog = cx.new(|cx| AddDialog::new(root, select, window, cx));
        built = Some(dialog.clone());
        dialog
    });
    if let Some(dialog) = built {
        cx.set_global(OpenDialog(Some(dialog.downgrade())));
    }
}

// ---- the dialog's state ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    New,
    Existing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewType {
    Virtualenv,
    Uv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExistingType {
    Python,
    Uv,
}

/// A check of a path field (`PathValidator`).
#[derive(Debug, Clone, PartialEq)]
enum Check<T> {
    /// Looked up or checked now (the field is disabled meanwhile).
    Running,
    /// Edited and not checked yet ("Press Enter to validate").
    Dirty,
    Done(T),
}

/// What the uv field found (`ValidatedPath.Executable`).
#[derive(Debug, Clone, PartialEq)]
enum UvCheck {
    Found {
        path: PathBuf,
        version: String,
    },
    /// Nothing found, or the path is wrong.
    Problem {
        path: Option<PathBuf>,
        message: String,
    },
}

/// A path field: the text field and its check.
struct PathField<T> {
    editor: Entity<Editor>,
    /// The text as it was set or checked: an edit makes the field dirty.
    checked_text: String,
    check: Check<T>,
    task: Task<()>,
}

impl<T> PathField<T> {
    fn text(&self, cx: &App) -> String {
        self.editor.read(cx).text()
    }

    /// The selected range of the field, in chars (a caret: both ends the
    /// same).
    fn selection(&self, cx: &App) -> [usize; 2] {
        let selection = *self.editor.read(cx).selections().primary();
        [selection.start(), selection.end()]
    }

    /// Fills the field as a text field takes a new text: the caret at its
    /// end, nothing selected.
    fn set_text(&mut self, text: &str, cx: &mut App) {
        self.checked_text = text.to_owned();
        self.editor.update(cx, |e, cx| {
            e.set_text_selected(text, cx);
            let end = e.text().chars().count();
            e.set_selections(text::SelectionSet::caret(end), cx);
        });
    }
}

/// An interpreter combo (`PythonInterpreterComboBox`): its list (`None`
/// while it is read) and the selected one.
#[derive(Default)]
struct InterpreterChoice {
    selected: Option<PathBuf>,
}

/// The lists of the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ListId {
    Type,
    Base,
    Version,
    Python,
    UvEnv,
}

impl ListId {
    fn id(self) -> &'static str {
        match self {
            ListId::Type => F_TYPE,
            ListId::Base => F_BASE,
            ListId::Version => F_VERSION,
            ListId::Python => F_PYTHON,
            ListId::UvEnv => F_UV_ENV,
        }
    }

    fn parse(id: &str) -> Option<ListId> {
        Some(match id {
            F_TYPE => ListId::Type,
            F_BASE => ListId::Base,
            F_VERSION => ListId::Version,
            F_PYTHON => ListId::Python,
            F_UV_ENV => ListId::UvEnv,
            _ => return None,
        })
    }
}

/// The alert under the uv Location (`VenvExistenceValidationState`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum UvAlert {
    /// "Environment at ".venv" already exists" + Override + Select existing.
    Error(String),
    /// "Existing environment at ".venv" will be overridden" + Select existing.
    Warning(String),
}

/// A notice under a field (`validationTooltip`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeKind {
    Error,
    Warning,
}

pub(crate) struct AddDialog {
    root: PathBuf,
    select: Select,
    focus: FocusHandle,
    method: Method,
    new_type: NewType,
    existing_type: ExistingType,
    machine: Option<Machine>,
    table: Arc<Table>,
    /// The interpreters found (`None` while they are looked for).
    found: Option<Vec<Candidate>>,
    /// The ones chosen with Browse or by path.
    manual: Vec<Candidate>,
    /// The project's uv environments (`None` while read).
    uv_envs: Option<Vec<Candidate>>,
    base: InterpreterChoice,
    existing: InterpreterChoice,
    uv_env: InterpreterChoice,
    venv_location: PathField<LocationCheck>,
    uv_location: PathField<LocationCheck>,
    uv: PathField<UvCheck>,
    /// The Pythons uv can download (empty until read, or without uv).
    downloads: Vec<Candidate>,
    /// uv is being asked for them.
    downloads_loading: bool,
    /// uv's Python versions (`None` while read).
    versions: Option<Vec<py_env::uv::MinorVersion>>,
    version: Option<(u32, u32)>,
    inherit: bool,
    all_projects: bool,
    /// Override existing environment was chosen.
    override_existing: bool,
    /// A path chosen by path being asked about (the combo is busy).
    probing: Option<ListId>,
    menu: Option<(ListId, MenuPopup)>,
    anchors: Rc<RefCell<HashMap<ListId, Bounds<Pixels>>>>,
    /// The label column's width for the rows shown (set when drawn).
    label_width: std::cell::Cell<f32>,
    /// Background work; dropping it cancels it.
    found_task: Task<()>,
    downloads_task: Task<()>,
    versions_task: Task<()>,
    probe_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl AddDialog {
    fn new(root: PathBuf, select: Select, window: &mut Window, cx: &mut Context<Self>) -> Self {
        list::ensure_loaded(cx);
        let table = list::table(cx);
        let field = |window: &mut Window, cx: &mut Context<Self>| {
            editor::field::new("", None, true, window, cx)
        };
        let venv_editor = field(window, cx);
        let uv_location_editor = field(window, cx);
        let uv_editor = field(window, cx);
        let mut subscriptions = Vec::new();
        for (editor, which) in [
            (&venv_editor, Which::VenvLocation),
            (&uv_location_editor, Which::UvLocation),
            (&uv_editor, Which::Uv),
        ] {
            subscriptions.push(cx.subscribe(
                editor,
                move |this, _, e: &editor::EditorEvent, cx| {
                    if matches!(e, editor::EditorEvent::Edited(_)) {
                        this.edited(which, cx);
                    }
                },
            ));
            let focus = editor.focus_handle(cx);
            subscriptions.push(cx.on_blur(&focus, window, move |this, _, cx| {
                this.validate(which, cx);
            }));
        }
        let path_field = |editor| PathField {
            editor,
            checked_text: String::new(),
            check: Check::Running,
            task: Task::ready(()),
        };
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut this = Self {
            root,
            select,
            focus,
            method: Method::New,
            new_type: NewType::Virtualenv,
            existing_type: ExistingType::Python,
            machine: None,
            table,
            found: None,
            manual: Vec::new(),
            uv_envs: None,
            base: InterpreterChoice::default(),
            existing: InterpreterChoice::default(),
            uv_env: InterpreterChoice::default(),
            venv_location: path_field(venv_editor),
            uv_location: path_field(uv_location_editor),
            uv: PathField {
                editor: uv_editor,
                checked_text: String::new(),
                check: Check::Running,
                task: Task::ready(()),
            },
            downloads: Vec::new(),
            downloads_loading: false,
            versions: None,
            version: None,
            inherit: false,
            all_projects: false,
            override_existing: false,
            probing: None,
            menu: None,
            anchors: Rc::default(),
            label_width: std::cell::Cell::new(LABEL_WIDTH),
            found_task: Task::ready(()),
            downloads_task: Task::ready(()),
            versions_task: Task::ready(()),
            probe_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.load(cx);
        this
    }

    // ---- looking things up --------------------------------------------------------

    /// On show (`onShown`): the machine's interpreters, the project's uv
    /// environments, uv, the suggested locations, the best guess of what
    /// to make.
    fn load(&mut self, cx: &mut Context<Self>) {
        let machine = machine(cx);
        let root = self.root.clone();
        self.autodetect_location(Which::VenvLocation, cx);
        self.autodetect_location(Which::UvLocation, cx);
        self.found_task = cx.spawn(async move |this, cx| {
            let machine = machine.await;
            let _ = this.update(cx, |this, cx| {
                this.machine = Some(machine.clone());
                this.autodetect_uv(cx);
                this.load_downloads(cx);
            });
            let found = {
                let root = root.clone();
                let machine = machine.clone();
                let Ok(task) = cx.update(|cx| {
                    runtime::spawn(cx, move |token| async move {
                        let ctx = py_env::RunContext::default().with_cancel(token);
                        let found = machine.find(&root, &ctx).await;
                        let uv_envs = candidates::uv_environments(&root);
                        (found, uv_envs)
                    })
                }) else {
                    return;
                };
                task.await
            };
            let guess = {
                let root = root.clone();
                let Ok(table) = cx.update(|cx| list::table(cx)) else {
                    return;
                };
                let chosen = cx.update(|cx| super::chosen(&root, cx)).ok().flatten();
                cx.background_spawn(async move { best_guess(&root, chosen.as_ref(), &table) })
                    .await
            };
            let _ = this.update(cx, |this, cx| {
                let (found, uv_envs) = found;
                this.found = Some(found);
                this.uv_envs = Some(uv_envs);
                this.table = list::table(cx);
                this.select_defaults(cx);
                if let Some((method, new_type, existing_type)) = guess {
                    this.method = method;
                    this.new_type = new_type;
                    this.existing_type = existing_type;
                }
                cx.notify();
            });
        });
    }

    /// The Pythons uv can download (`getInstallableInterpreters`), with the
    /// uv found on the machine.
    fn load_downloads(&mut self, cx: &mut Context<Self>) {
        let Some(uv) = self.machine.as_ref().and_then(|m| m.tools.uv.clone()) else {
            return;
        };
        self.downloads_loading = true;
        let work = runtime::spawn(cx, move |token| async move {
            let ctx = py_env::RunContext::default().with_cancel(token);
            py_env::Uv::new(uv, py_env::UvConfig::default())
                .python_downloads(&ctx)
                .await
                .map(|list| candidates::installable(&list))
                .unwrap_or_default()
        });
        self.downloads_task = cx.spawn(async move |this, cx| {
            let downloads = work.await;
            let _ = this.update(cx, |this, cx| {
                this.downloads = downloads;
                this.downloads_loading = false;
                if this.found.is_some() {
                    this.select_defaults(cx);
                }
                cx.notify();
            });
        });
    }

    /// The defaults of the interpreter combos once their lists are read:
    /// the remembered base (else the first), the first existing one.
    fn select_defaults(&mut self, cx: &App) {
        let preferred = list::table(cx).preferred.base_python.clone();
        if self.base.selected.is_none() {
            let bases = self.base_list();
            self.base.selected = candidates::default_base(&bases, preferred.as_deref())
                .map(|ix| bases[ix].python.clone());
        }
        if self.existing.selected.is_none() {
            self.existing.selected = self.existing_list().first().map(|c| c.python.clone());
        }
        if self.uv_env.selected.is_none() {
            self.uv_env.selected = self.uv_env_list().first().map(|c| c.python.clone());
        }
    }

    fn base_list(&self) -> Vec<Candidate> {
        candidates::base_list(
            self.found.as_deref().unwrap_or_default(),
            &self.manual,
            &self.downloads,
        )
    }

    fn existing_list(&self) -> Vec<Candidate> {
        let venv_root = self.machine.as_ref().and_then(|m| m.venv_root.clone());
        candidates::existing_list(
            &self.table,
            self.found.as_deref().unwrap_or_default(),
            &self.manual,
            &self.root,
            venv_root.as_deref(),
        )
    }

    fn uv_env_list(&self) -> Vec<Candidate> {
        let mut found = self.uv_envs.clone().unwrap_or_default();
        found.extend(self.manual.iter().cloned());
        let mut seen: Vec<PathBuf> = Vec::new();
        found.retain(|c| {
            let new = !seen.contains(&c.python);
            seen.push(c.python.clone());
            new
        });
        found
    }

    fn list_of(&self, list: ListId) -> Option<Vec<Candidate>> {
        match list {
            ListId::Base => self.found.as_ref().map(|_| self.base_list()),
            ListId::Python => self.found.as_ref().map(|_| self.existing_list()),
            ListId::UvEnv => self.uv_envs.as_ref().map(|_| self.uv_env_list()),
            ListId::Type | ListId::Version => None,
        }
    }

    fn choice(&self, list: ListId) -> Option<&InterpreterChoice> {
        match list {
            ListId::Base => Some(&self.base),
            ListId::Python => Some(&self.existing),
            ListId::UvEnv => Some(&self.uv_env),
            ListId::Type | ListId::Version => None,
        }
    }

    fn choice_mut(&mut self, list: ListId) -> Option<&mut InterpreterChoice> {
        match list {
            ListId::Base => Some(&mut self.base),
            ListId::Python => Some(&mut self.existing),
            ListId::UvEnv => Some(&mut self.uv_env),
            ListId::Type | ListId::Version => None,
        }
    }

    /// The selected interpreter of a combo, as its list has it.
    fn selected(&self, list: ListId) -> Option<Candidate> {
        let python = self.choice(list)?.selected.clone()?;
        self.list_of(list)?
            .into_iter()
            .find(|c| c.python == python)
            .or_else(|| self.manual.iter().find(|c| c.python == python).cloned())
    }

    /// The suggested location (`autodetectFolder`), checked.
    fn autodetect_location(&mut self, which: Which, cx: &mut Context<Self>) {
        let kept = list::table(cx).preferred.venv_base_path.clone();
        let path = candidates::suggested_location(&self.root, kept.as_deref());
        let text = path.display().to_string();
        if let Some(field) = self.location_mut(which) {
            field.set_text(&text, cx);
        }
        self.check_location(which, path, cx);
    }

    fn location_mut(&mut self, which: Which) -> Option<&mut PathField<LocationCheck>> {
        match which {
            Which::VenvLocation => Some(&mut self.venv_location),
            Which::UvLocation => Some(&mut self.uv_location),
            Which::Uv => None,
        }
    }

    /// Checks what is at a location (`validateVenv`).
    fn check_location(&mut self, which: Which, path: PathBuf, cx: &mut Context<Self>) {
        if which == Which::UvLocation {
            self.override_existing = false;
        }
        let work = runtime::spawn(cx, move |token| async move {
            let ctx = py_env::RunContext::default().with_cancel(token);
            create::check_location(&path, &ctx).await
        });
        let task = cx.spawn(async move |this, cx| {
            let check = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(field) = this.location_mut(which) {
                    field.check = Check::Done(check);
                }
                cx.notify();
            });
        });
        if let Some(field) = self.location_mut(which) {
            field.check = Check::Running;
            field.task = task;
        }
        cx.notify();
    }

    /// Finds uv (`autodetectExecutable`) and checks it.
    fn autodetect_uv(&mut self, cx: &mut Context<Self>) {
        let found = self.machine.as_ref().and_then(|m| m.tools.uv.clone());
        match found {
            Some(path) => {
                self.uv.set_text(&path.display().to_string(), cx);
                self.check_uv(path, cx);
            }
            None => {
                self.uv.set_text("", cx);
                self.uv.check = Check::Done(UvCheck::Problem {
                    path: None,
                    message: NOT_DETECTED.into(),
                });
                self.uv_changed(cx);
            }
        }
    }

    /// Checks the uv at `path` (`validateExecutable`, then `uv --version`).
    fn check_uv(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let work = runtime::spawn(cx, move |token| async move {
            let ctx = py_env::RunContext::default().with_cancel(token);
            let problem = |message: &str| UvCheck::Problem {
                path: Some(path.clone()),
                message: message.to_owned(),
            };
            match std::fs::metadata(&path) {
                Err(_) => return problem(DOES_NOT_EXIST),
                Ok(m) if m.is_dir() => return problem(IS_DIRECTORY),
                Ok(_) if !py_env::paths::is_executable(&path) => return problem(NOT_EXECUTABLE),
                Ok(_) => {}
            }
            match create::uv_that_runs(Some(path.clone()), &ctx).await {
                Ok((_, version)) => UvCheck::Found { path, version },
                Err(e) => problem(&e),
            }
        });
        self.uv.check = Check::Running;
        self.uv.task = cx.spawn(async move |this, cx| {
            let check = work.await;
            let _ = this.update(cx, |this, cx| {
                this.uv.check = Check::Done(check);
                this.uv_changed(cx);
            });
        });
        self.versions = None;
        cx.notify();
    }

    /// uv was checked: its Python versions (`listSupportedPythonVersions`,
    /// narrowed by the project's `requires-python`).
    fn uv_changed(&mut self, cx: &mut Context<Self>) {
        self.version = None;
        let Check::Done(UvCheck::Found { path, .. }) = &self.uv.check else {
            self.versions = Some(Vec::new());
            self.versions_task = Task::ready(());
            cx.notify();
            return;
        };
        let (uv, root) = (path.clone(), self.root.clone());
        self.versions = None;
        let work = runtime::spawn(cx, move |token| async move {
            let ctx = py_env::RunContext::default().with_cancel(token);
            let request = std::fs::read_to_string(root.join("pyproject.toml"))
                .ok()
                .and_then(|text| py_env::project::parse_pyproject(&text).ok())
                .and_then(|p| p.requires_python);
            py_env::Uv::new(uv, py_env::UvConfig::default())
                .python_versions(request.as_deref(), Some(&root), &ctx)
                .await
                .unwrap_or_default()
        });
        self.versions_task = cx.spawn(async move |this, cx| {
            let versions = work.await;
            let _ = this.update(cx, |this, cx| {
                this.versions = Some(versions);
                cx.notify();
            });
        });
        cx.notify();
    }

    // ---- editing --------------------------------------------------------------------

    /// A path field was edited by the person: dirty until checked.
    fn edited(&mut self, which: Which, cx: &mut Context<Self>) {
        let (text, checked) = match which {
            Which::VenvLocation => (
                self.venv_location.text(cx),
                &self.venv_location.checked_text,
            ),
            Which::UvLocation => (self.uv_location.text(cx), &self.uv_location.checked_text),
            Which::Uv => (self.uv.text(cx), &self.uv.checked_text),
        };
        if &text == checked {
            return;
        }
        match which {
            Which::VenvLocation => {
                self.venv_location.check = Check::Dirty;
                self.venv_location.task = Task::ready(());
            }
            Which::UvLocation => {
                self.uv_location.check = Check::Dirty;
                self.uv_location.task = Task::ready(());
            }
            Which::Uv => {
                self.uv.check = Check::Dirty;
                self.uv.task = Task::ready(());
            }
        }
        cx.notify();
    }

    /// Checks a dirty field (Enter, the focus leaving it): an empty one
    /// goes back to what is found by itself.
    fn validate(&mut self, which: Which, cx: &mut Context<Self>) {
        let dirty = match which {
            Which::VenvLocation => self.venv_location.check == Check::Dirty,
            Which::UvLocation => self.uv_location.check == Check::Dirty,
            Which::Uv => self.uv.check == Check::Dirty,
        };
        if !dirty {
            return;
        }
        match which {
            Which::VenvLocation | Which::UvLocation => {
                let Some(field) = self.location_mut(which) else {
                    return;
                };
                let text = field.editor.read(cx).text().trim().to_owned();
                if text.is_empty() {
                    self.autodetect_location(which, cx);
                } else {
                    field.checked_text = field.editor.read(cx).text();
                    self.check_location(which, PathBuf::from(text), cx);
                }
            }
            Which::Uv => {
                let text = self.uv.text(cx).trim().to_owned();
                if text.is_empty() {
                    self.autodetect_uv(cx);
                } else {
                    self.uv.checked_text = self.uv.text(cx);
                    self.check_uv(PathBuf::from(text), cx);
                }
            }
        }
    }

    /// The path fields shown now.
    fn shown_fields(&self) -> Vec<Which> {
        match (self.method, self.new_type, self.existing_type) {
            (Method::New, NewType::Virtualenv, _) => vec![Which::VenvLocation],
            (Method::New, NewType::Uv, _) => vec![Which::Uv, Which::UvLocation],
            (Method::Existing, _, ExistingType::Python) => Vec::new(),
            (Method::Existing, _, ExistingType::Uv) => vec![Which::Uv],
        }
    }

    fn validate_shown(&mut self, cx: &mut Context<Self>) {
        for which in self.shown_fields() {
            self.validate(which, cx);
        }
    }

    /// An interpreter given by path (Browse, or a path not listed): asked
    /// in the background and added to the list (`onPathSelected`).
    fn choose_path(&mut self, list: ListId, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(found) = self.list_of(list).and_then(|l| {
            l.into_iter()
                .find(|c| super::table::same_path(&c.python, &path))
        }) {
            if let Some(choice) = self.choice_mut(list) {
                choice.selected = Some(found.python);
            }
            cx.notify();
            return;
        }
        let python = match path.is_dir() {
            true => py_env::paths::environment_python(&path).unwrap_or(path),
            false => path,
        };
        let work = runtime::spawn(cx, move |token| async move {
            let ctx = py_env::RunContext::default().with_cancel(token);
            py_env::interpreter::probe(&python, "manual", std::time::Duration::from_secs(15), &ctx)
                .await
                .map(|i| Candidate {
                    origin: Origin::Manual,
                    ..Candidate::of_interpreter(&i)
                })
                .map_err(|e| e.to_string())
        });
        self.probing = Some(list);
        self.probe_task = cx.spawn(async move |this, cx| {
            let probed = work.await;
            let _ = this.update(cx, |this, cx| {
                this.probing = None;
                match probed {
                    // The Base Python list takes installations only
                    // (`requireSystemPython`).
                    Ok(c) if list == ListId::Base && !c.base => {
                        eprintln!("{}: not an installation", c.python.display())
                    }
                    Ok(c) => {
                        let python = c.python.clone();
                        this.manual.retain(|m| m.python != python);
                        this.manual.push(c);
                        if let Some(choice) = this.choice_mut(list) {
                            choice.selected = Some(python);
                        }
                    }
                    Err(e) => eprintln!("python.interpreter.add-dialog: {e}"),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn set_field(
        &mut self,
        field: &str,
        value: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let flag = || match value {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("`{field}` takes `true` or `false`, not `{value}`")),
        };
        match field {
            F_ENVIRONMENT => {
                self.method = match value {
                    "generate-new" => Method::New,
                    "select-existing" => Method::Existing,
                    _ => {
                        return Err(format!(
                            "`environment` takes `generate-new` or `select-existing`, not `{value}`"
                        ));
                    }
                };
            }
            F_TYPE => match (self.method, value) {
                (Method::New, "virtualenv") => self.new_type = NewType::Virtualenv,
                (Method::New, "uv") => self.new_type = NewType::Uv,
                (Method::Existing, "python") => self.existing_type = ExistingType::Python,
                (Method::Existing, "uv") => self.existing_type = ExistingType::Uv,
                _ => return Err(format!("no type `{value}` here")),
            },
            F_BASE => match candidates::Candidate::download_key(value) {
                // A Python to download: chosen as it is listed.
                Some(_) => {
                    let key = PathBuf::from(value);
                    if !self.base_list().iter().any(|c| c.python == key) {
                        return Err(format!("no Python `{value}` to download"));
                    }
                    self.base.selected = Some(key);
                }
                None => self.choose_path(ListId::Base, self.resolve(value), cx),
            },
            F_PYTHON => self.choose_path(ListId::Python, self.resolve(value), cx),
            F_UV_ENV => self.choose_path(ListId::UvEnv, self.resolve(value), cx),
            F_LOCATION => {
                let which = match (self.method, self.new_type) {
                    (Method::New, NewType::Uv) => Which::UvLocation,
                    _ => Which::VenvLocation,
                };
                let path = self.resolve(value);
                if let Some(field) = self.location_mut(which) {
                    field.set_text(&path.display().to_string(), cx);
                }
                self.check_location(which, path, cx);
            }
            F_UV => {
                let path = self.resolve(value);
                self.uv.set_text(&path.display().to_string(), cx);
                self.check_uv(path, cx);
            }
            F_VERSION => {
                self.version = match value {
                    "default" | "" => None,
                    v => Some(
                        py_env::interpreter::major_minor(v)
                            .ok_or_else(|| format!("no Python version `{v}`"))?,
                    ),
                };
            }
            F_INHERIT => self.inherit = flag()?,
            F_ALL => self.all_projects = flag()?,
            _ => return Err(format!("no field `{field}` in the dialog")),
        }
        self.close_menu(cx);
        cx.notify();
        Ok(())
    }

    /// A path as given (absolute) or relative to the project.
    fn resolve(&self, path: &str) -> PathBuf {
        super::resolve(&self.root, path)
    }

    // ---- what is shown ------------------------------------------------------------

    /// The alert under the uv Location.
    fn uv_alert(&self) -> Option<UvAlert> {
        match &self.uv_location.check {
            Check::Done(LocationCheck::HasPython { name, .. }) => Some(if self.override_existing {
                UvAlert::Warning(create::override_warning(name))
            } else {
                UvAlert::Error(create::override_error(name))
            }),
            _ => None,
        }
    }

    /// The problem of a location field (its red outline), `None` when it
    /// may be used.
    fn location_problem(&self, which: Which) -> Option<String> {
        let field = match which {
            Which::UvLocation => &self.uv_location,
            _ => &self.venv_location,
        };
        match &field.check {
            Check::Running => Some(WAIT_FOR_VALIDATION.to_owned()),
            Check::Dirty => Some(PRESS_ENTER.to_owned()),
            Check::Done(LocationCheck::HasPython { .. })
                if which == Which::UvLocation && self.override_existing =>
            {
                None
            }
            Check::Done(check) => check.message(),
        }
    }

    fn uv_problem(&self) -> Option<String> {
        match &self.uv.check {
            Check::Running => Some(WAIT_FOR_VALIDATION.to_owned()),
            Check::Dirty => Some(PRESS_ENTER.to_owned()),
            Check::Done(UvCheck::Found { .. }) => None,
            Check::Done(UvCheck::Problem { message, .. }) => Some(message.clone()),
        }
    }

    fn uv_found(&self) -> bool {
        matches!(self.uv.check, Check::Done(UvCheck::Found { .. }))
    }

    /// "No uv executable found" shows once uv was looked for and nothing
    /// was found.
    fn uv_missing(&self) -> bool {
        matches!(
            self.uv.check,
            Check::Done(UvCheck::Problem { path: None, .. })
        )
    }

    /// The problem of an interpreter combo: loading, nothing selected, an
    /// unsupported base (it gates OK; its text is the notice below).
    fn combo_problem(&self, list: ListId) -> Option<String> {
        if self.list_of(list).is_none() || self.probing == Some(list) {
            return Some(PLEASE_WAIT.to_owned());
        }
        let Some(selected) = self.selected(list) else {
            return Some(String::new());
        };
        (list == ListId::Base && !selected.supports_new_environments())
            .then(create::unsupported_base)
    }

    /// The notice under an interpreter combo: the unsupported base (an
    /// error), an old existing interpreter (a warning).
    fn combo_notice(&self, list: ListId) -> Option<(NoticeKind, String)> {
        let selected = self.selected(list)?;
        if selected.supports_new_environments() {
            return None;
        }
        match list {
            ListId::Base => Some((NoticeKind::Error, create::unsupported_base())),
            ListId::Python => Some((NoticeKind::Warning, create::unmanaged_python_warning())),
            _ => None,
        }
    }

    /// The problems that keep OK disabled, for the panel shown.
    fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        match (self.method, self.new_type, self.existing_type) {
            (Method::New, NewType::Virtualenv, _) => {
                out.extend(self.combo_problem(ListId::Base));
                out.extend(self.location_problem(Which::VenvLocation));
            }
            (Method::New, NewType::Uv, _) => {
                out.extend(self.uv_problem());
                out.extend(self.location_problem(Which::UvLocation));
            }
            (Method::Existing, _, ExistingType::Python) => {
                out.extend(self.combo_problem(ListId::Python));
            }
            (Method::Existing, _, ExistingType::Uv) => {
                out.extend(self.uv_problem());
                if self.uv_found() {
                    out.extend(self.combo_problem(ListId::UvEnv));
                }
            }
        }
        out
    }

    fn ok_enabled(&self) -> bool {
        self.problems().is_empty()
    }

    // ---- links and buttons ------------------------------------------------------------

    /// "Select existing interpreter": Select existing with the interpreter
    /// at the location (Python; uv for a uv location of the project).
    fn select_existing(&mut self, cx: &mut Context<Self>) {
        let uv = self.method == Method::New && self.new_type == NewType::Uv;
        let check = if uv {
            &self.uv_location.check
        } else {
            &self.venv_location.check
        };
        let at = match check {
            Check::Done(LocationCheck::HasPython { python, level, .. }) => {
                Some((python.clone(), level.clone()))
            }
            _ => None,
        };
        self.method = Method::Existing;
        if uv {
            self.existing_type = ExistingType::Uv;
        } else {
            self.existing_type = ExistingType::Python;
            if let Some((python, level)) = at {
                if !self.existing_list().iter().any(|c| c.python == python) {
                    self.manual.push(Candidate {
                        origin: Origin::Manual,
                        ..Candidate::found(python.clone(), Some(level), false)
                    });
                }
                self.existing.selected = Some(python);
            }
        }
        cx.notify();
    }

    /// The request OK sends.
    fn request(&self, cx: &App) -> Option<Request> {
        let path_of = |field: &PathField<LocationCheck>| Some(PathBuf::from(field.text(cx).trim()));
        let uv_path = match &self.uv.check {
            Check::Done(UvCheck::Found { path, .. }) => Some(path.clone()),
            _ => None,
        };
        let mut r = match (self.method, self.new_type, self.existing_type) {
            (Method::New, NewType::Virtualenv, _) => {
                let mut r = Request::new(Kind::Virtualenv, self.root.clone());
                let base = self.selected(ListId::Base)?;
                match base.install_version() {
                    Some(version) => r.install_python = Some(version.to_owned()),
                    None => r.python = Some(base.python),
                }
                r.location = path_of(&self.venv_location);
                r.inherit_site_packages = self.inherit;
                r.available_to_all = self.all_projects;
                r.uv = self.machine.as_ref().and_then(|m| m.tools.uv.clone());
                r.uv_located = self.machine.is_some();
                r
            }
            (Method::New, NewType::Uv, _) => {
                let mut r = Request::new(Kind::Uv, self.root.clone());
                r.location = path_of(&self.uv_location);
                r.python_version = self.version.map(|(a, b)| format!("{a}.{b}"));
                // A version none of whose interpreters is on the machine is
                // downloaded first.
                if self.version_to_download() {
                    r.install_python = r.python_version.clone();
                }
                r.override_existing = self.override_existing;
                r.uv = uv_path;
                r
            }
            (Method::Existing, _, ExistingType::Python) => {
                let mut r = Request::new(Kind::ExistingPython, self.root.clone());
                r.python = Some(self.selected(ListId::Python)?.python);
                r
            }
            (Method::Existing, _, ExistingType::Uv) => {
                let mut r = Request::new(Kind::ExistingUv, self.root.clone());
                r.python = Some(self.selected(ListId::UvEnv)?.python);
                r.uv = uv_path;
                r
            }
        };
        if r.uv.is_none() && r.kind == Kind::ExistingUv {
            r.uv = self.machine.as_ref().and_then(|m| m.tools.uv.clone());
        }
        Some(r)
    }

    fn ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.validate_shown(cx);
        if !self.ok_enabled() {
            return;
        }
        let Some(request) = self.request(cx) else {
            return;
        };
        super::widget::add_and_report(request, self.select, cx);
        self.close(window, cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(cx);
        if let Some(open) = cx.try_global::<OpenDialog>()
            && open
                .0
                .as_ref()
                .is_some_and(|d| d.entity_id() == cx.entity_id())
        {
            cx.set_global(OpenDialog(None));
        }
        // From its own handlers the window is being updated: it removes
        // itself.
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
    }

    fn browse(&mut self, field: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (files, directories) = match field {
            F_LOCATION => (false, true),
            F_UV => (true, false),
            _ => (true, true),
        };
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files,
            directories,
            multiple: false,
            prompt: (field != F_LOCATION && field != F_UV).then(|| SELECT_INTERPRETER_TITLE.into()),
        });
        let field = field.to_owned();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let _ = this.set_field(&field, &path.display().to_string(), cx);
            });
        })
        .detach();
    }

    // ---- lists ------------------------------------------------------------------------

    fn open_list(&mut self, list: ListId, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(cx);
        let set = |value: String| DialogSet {
            field: list.id().to_owned(),
            value,
        };
        let items: Vec<MenuItem> = match list {
            ListId::Type => match self.method {
                Method::New => vec![
                    MenuItem::action(VIRTUALENV, set("virtualenv".into())).icon(VENV_ICON),
                    MenuItem::action(UV, set("uv".into())).icon(candidates::UV_ICON),
                ],
                Method::Existing => vec![
                    MenuItem::action(PYTHON, set("python".into())).icon(candidates::PYTHON_ICON),
                    MenuItem::action(UV, set("uv".into())).icon(candidates::UV_ICON),
                ],
            },
            ListId::Version => {
                if self.versions.is_none() {
                    return;
                }
                std::iter::once(MenuItem::action(DEFAULT_VERSION, set("default".into())))
                    .chain(self.versions.iter().flatten().map(|m| {
                        let v = format!("{}.{}", m.major, m.minor);
                        let item = MenuItem::action(v.clone(), set(v));
                        if m.installed {
                            item
                        } else {
                            item.small_secondary(format!(" {}", candidates::DOWNLOAD_AND_INSTALL))
                        }
                    }))
                    .collect()
            }
            ListId::Base | ListId::Python | ListId::UvEnv => {
                let Some(candidates) = self.list_of(list) else {
                    return;
                };
                if self.probing == Some(list) {
                    return;
                }
                candidates
                    .iter()
                    .map(|c| {
                        let (text, detail) = c.text();
                        let (icon, alpha) = c.icon();
                        MenuItem::action(text, set(c.python.display().to_string()))
                            .icon(icon)
                            .icon_alpha(alpha)
                            .small_secondary(detail)
                    })
                    .collect()
            }
        };
        if items.is_empty() {
            return;
        }
        // The list opens under the combo, as wide as the combo (long paths
        // are cut) and a little beyond it (`ComboBox` popups).
        let combo = self
            .anchors
            .borrow()
            .get(&list)
            .copied()
            .unwrap_or_default();
        let at = point(combo.left() - px(LIST_LEFT), combo.bottom() + px(LIST_DROP));
        let width = f32::from(combo.size.width) + LIST_GROW;
        // The versions to download are wider than their combo.
        let grows = list == ListId::Version && self.versions.iter().flatten().any(|m| !m.installed);
        let options = PopupOptions {
            speed_search: true,
            min_width: Some(width),
            max_width: (!grows).then_some(width),
            row_height: Some(match list {
                ListId::Version => VERSION_ROW,
                ListId::Type => TYPE_ROW,
                _ => LIST_ROW,
            }),
            // The rows of the types (`PythonEnvironmentComboBoxRenderer`)
            // and of the interpreters (`PythonSdkComboBoxListCellRenderer`)
            // are drawn by renderers of their own: the selection runs from
            // edge to edge. The versions are plain text (`I3_2`).
            edge_to_edge: list != ListId::Version,
            ..PopupOptions::default()
        };
        let focus = self.focus.clone();
        let popup = MenuPopup::open_with(
            items,
            options,
            at,
            &focus,
            !workbench::env::background(),
            window,
            cx,
            Self::close_menu,
        );
        if let Some(popup) = popup {
            self.menu = Some((list, popup));
        }
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if let Some((_, popup)) = self.menu.take() {
            popup.close(cx);
            cx.notify();
        }
    }

    // ---- actions ------------------------------------------------------------------------

    fn on_set(&mut self, a: &DialogSet, _: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.set_field(&a.field, &a.value, cx) {
            eprintln!("python.interpreter.add-dialog.set: {e}");
        }
    }

    fn on_open_list(&mut self, a: &DialogOpenList, window: &mut Window, cx: &mut Context<Self>) {
        match ListId::parse(&a.list) {
            Some(list) => self.open_list(list, window, cx),
            None => eprintln!(
                "python.interpreter.add-dialog.open-list: no list `{}`",
                a.list
            ),
        }
    }

    fn on_validate(&mut self, _: &DialogValidate, _: &mut Window, cx: &mut Context<Self>) {
        self.validate_shown(cx);
    }

    fn on_browse(&mut self, a: &DialogBrowse, window: &mut Window, cx: &mut Context<Self>) {
        self.browse(&a.field.clone(), window, cx);
    }

    fn on_override(&mut self, _: &DialogOverride, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.uv_alert(), Some(UvAlert::Error(_))) {
            self.override_existing = true;
            cx.notify();
        }
    }

    fn on_select_existing(
        &mut self,
        _: &DialogSelectExisting,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_existing(cx);
    }

    fn on_install_uv(&mut self, _: &DialogInstallUv, _: &mut Window, cx: &mut Context<Self>) {
        cx.open_url(UV_INSTALL_URL);
    }

    fn on_learn_more(&mut self, _: &DialogLearnMore, _: &mut Window, cx: &mut Context<Self>) {
        cx.open_url(VERSIONS_URL);
    }

    fn on_ok(&mut self, _: &DialogOk, window: &mut Window, cx: &mut Context<Self>) {
        self.ok(window, cx);
    }

    fn on_cancel(&mut self, _: &DialogCancel, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    /// Enter in a path field checks it (`ValidatedPathField`'s Enter);
    /// elsewhere it is OK. Escape is Cancel.
    fn on_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let in_field = self.shown_fields().into_iter().find(|w| {
            let editor = match w {
                Which::VenvLocation => &self.venv_location.editor,
                Which::UvLocation => &self.uv_location.editor,
                Which::Uv => &self.uv.editor,
            };
            editor.focus_handle(cx).is_focused(window)
        });
        match in_field {
            Some(which) => self.validate(which, cx),
            None => self.ok(window, cx),
        }
    }

    // ---- state ----------------------------------------------------------------------------

    fn combo_json(&self, list: ListId) -> Json {
        let items = self.list_of(list);
        let selected = self.selected(list);
        let row = |c: &Candidate| {
            let (text, detail) = c.text();
            json!({
                "text": text,
                "detail": detail,
                "path": c.python.display().to_string(),
                "icon": c.icon().0.0,
                "origin": match c.origin {
                    Origin::Configured { .. } => "configured",
                    Origin::Found => "found",
                    Origin::Manual => "manual",
                    Origin::Installable => "download",
                },
            })
        };
        json!({
            "loading": items.is_none() || self.probing == Some(list),
            "items": items.as_ref().map(|l| l.iter().map(row).collect::<Vec<_>>()),
            "selected": selected.as_ref().map(row),
            "notice": self.combo_notice(list).map(|(kind, text)| json!({
                "kind": notice_id(kind),
                "text": text,
                "links": [LEARN_MORE],
            })),
        })
    }

    fn location_json(&self, which: Which, cx: &App) -> Json {
        let field = match which {
            Which::UvLocation => &self.uv_location,
            _ => &self.venv_location,
        };
        let alert = match which {
            Which::UvLocation => self.uv_alert().map(|a| match a {
                UvAlert::Error(text) => json!({"kind": "error", "text": text, "links": [OVERRIDE_LINK, SELECT_EXISTING_LINK]}),
                UvAlert::Warning(text) => json!({"kind": "warning", "text": text, "links": [SELECT_EXISTING_LINK]}),
            }),
            _ => match &field.check {
                Check::Done(check @ LocationCheck::HasPython { .. }) => Some(json!({
                    "kind": "error",
                    "text": check.message(),
                    "links": [SELECT_EXISTING_LINK],
                })),
                _ => None,
            },
        };
        json!({
            "text": field.text(cx),
            "selection": field.selection(cx),
            "state": check_id(&field.check),
            "error": self.location_problem(which),
            "alert": alert,
        })
    }

    fn uv_json(&self, cx: &App) -> Json {
        json!({
            "text": self.uv.text(cx),
            "selection": self.uv.selection(cx),
            "state": check_id(&self.uv.check),
            "found": self.uv_found(),
            "version": match &self.uv.check {
                Check::Done(UvCheck::Found { version, .. }) => Some(version.clone()),
                _ => None,
            },
            "error": self.uv_problem(),
            "missing": self.uv_missing().then(|| json!({
                "kind": "warning",
                "text": NO_UV,
                "links": [INSTALL_UV, SELECT_PATH],
            })),
        })
    }

    fn state_json(&self, cx: &App) -> Json {
        let rows: Vec<&str> = self.rows().iter().map(|r| r.label()).collect();
        let mut state = json!({
            "title": TITLE,
            "project": self.root.display().to_string(),
            "environment": match self.method {
                Method::New => "generate-new",
                Method::Existing => "select-existing",
            },
            "type": self.type_id(),
            "types": self.type_labels(),
            "rows": rows,
            "ok-enabled": self.ok_enabled(),
            "problems": self.problems(),
            "list": self.menu.as_ref().map(|(l, _)| l.id()),
            "menu": self.menu.as_ref().map(|(_, m)| m.menu.read(cx).state_json()),
            "downloads": {
                "loading": self.downloads_loading,
                "versions": self.downloads.iter().filter_map(|c| c.install_version()).collect::<Vec<_>>(),
            },
        });
        let fields = state.as_object_mut().expect("an object");
        match (self.method, self.new_type, self.existing_type) {
            (Method::New, NewType::Virtualenv, _) => {
                fields.insert(F_BASE.into(), self.combo_json(ListId::Base));
                fields.insert(
                    F_LOCATION.into(),
                    self.location_json(Which::VenvLocation, cx),
                );
                fields.insert(F_INHERIT.into(), json!(self.inherit));
                fields.insert(F_ALL.into(), json!(self.all_projects));
            }
            (Method::New, NewType::Uv, _) => {
                fields.insert(
                    F_VERSION.into(),
                    json!({
                        "loading": self.versions.is_none(),
                        "items": std::iter::once(DEFAULT_VERSION.to_owned())
                            .chain(self.versions.iter().flatten().map(|m| format!("{}.{}", m.major, m.minor)))
                            .collect::<Vec<_>>(),
                        "download": self.versions.iter().flatten().filter(|m| !m.installed).map(|m| format!("{}.{}", m.major, m.minor)).collect::<Vec<_>>(),
                        "selected": self.version_text(),
                    }),
                );
                fields.insert(F_UV.into(), self.uv_json(cx));
                fields.insert(F_LOCATION.into(), self.location_json(Which::UvLocation, cx));
            }
            (Method::Existing, _, ExistingType::Python) => {
                fields.insert(F_PYTHON.into(), self.combo_json(ListId::Python));
            }
            (Method::Existing, _, ExistingType::Uv) => {
                fields.insert(F_UV.into(), self.uv_json(cx));
                fields.insert(F_UV_ENV.into(), self.combo_json(ListId::UvEnv));
            }
        }
        state
    }

    fn type_id(&self) -> &'static str {
        match (self.method, self.new_type, self.existing_type) {
            (Method::New, NewType::Virtualenv, _) => "virtualenv",
            (Method::New, NewType::Uv, _) | (Method::Existing, _, ExistingType::Uv) => "uv",
            (Method::Existing, _, ExistingType::Python) => "python",
        }
    }

    fn type_labels(&self) -> [&'static str; 2] {
        match self.method {
            Method::New => [VIRTUALENV, UV],
            Method::Existing => [PYTHON, UV],
        }
    }

    /// The version chosen has no interpreter on the machine: uv downloads
    /// one.
    fn version_to_download(&self) -> bool {
        let Some((major, minor)) = self.version else {
            return false;
        };
        self.versions
            .iter()
            .flatten()
            .any(|m| (m.major, m.minor) == (major, minor) && !m.installed)
    }

    fn version_text(&self) -> String {
        match self.version {
            Some((a, b)) => format!("{a}.{b}"),
            None => DEFAULT_VERSION.to_owned(),
        }
    }

    /// The rows shown, in order (the reference's `rowsRange` of the
    /// selected manager).
    fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Environment, Row::Type];
        match (self.method, self.new_type, self.existing_type) {
            (Method::New, NewType::Virtualenv, _) => {
                rows.push(Row::Combo(ListId::Base));
                if self.combo_notice(ListId::Base).is_some() {
                    rows.push(Row::ComboNotice(ListId::Base));
                }
                rows.push(Row::Location(Which::VenvLocation));
                if matches!(
                    self.venv_location.check,
                    Check::Done(LocationCheck::HasPython { .. })
                ) {
                    rows.push(Row::LocationNotice(Which::VenvLocation));
                }
                rows.push(Row::Check(F_INHERIT));
                rows.push(Row::Check(F_ALL));
            }
            (Method::New, NewType::Uv, _) => {
                rows.push(Row::Version);
                if self.uv_missing() {
                    rows.push(Row::UvMissing);
                }
                rows.push(Row::Uv);
                rows.push(Row::Location(Which::UvLocation));
                if self.uv_alert().is_some() {
                    rows.push(Row::LocationNotice(Which::UvLocation));
                }
            }
            (Method::Existing, _, ExistingType::Python) => {
                rows.push(Row::Combo(ListId::Python));
                if self.combo_notice(ListId::Python).is_some() {
                    rows.push(Row::ComboNotice(ListId::Python));
                }
            }
            (Method::Existing, _, ExistingType::Uv) => {
                if self.uv_missing() {
                    rows.push(Row::UvMissing);
                }
                rows.push(Row::Uv);
                if self.uv_found() {
                    rows.push(Row::Combo(ListId::UvEnv));
                }
            }
        }
        rows
    }
}

/// The path fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    VenvLocation,
    UvLocation,
    Uv,
}

/// A row of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Environment,
    Type,
    Combo(ListId),
    ComboNotice(ListId),
    Location(Which),
    LocationNotice(Which),
    Check(&'static str),
    Version,
    UvMissing,
    Uv,
}

impl Row {
    /// The row's label, or what it is (state JSON).
    fn label(&self) -> &'static str {
        match self {
            Row::Environment => ENVIRONMENT,
            Row::Type => TYPE,
            Row::Combo(ListId::Base) => BASE_PYTHON,
            Row::Combo(ListId::Python) => PYTHON_PATH,
            Row::Combo(_) => UV_ENVIRONMENT,
            Row::ComboNotice(_) => "notice",
            Row::Location(_) => LOCATION,
            Row::LocationNotice(_) => "alert",
            Row::Check(F_INHERIT) => INHERIT,
            Row::Check(_) => ALL_PROJECTS,
            Row::Version => PYTHON_VERSION,
            Row::UvMissing => NO_UV,
            Row::Uv => PATH_TO_UV,
        }
    }
}

const VENV_ICON: IconName = IconName("python/virtualEnv");

fn check_id<T>(check: &Check<T>) -> &'static str {
    match check {
        Check::Running => "validating",
        Check::Dirty => "dirty",
        Check::Done(_) => "done",
    }
}

fn notice_id(kind: NoticeKind) -> &'static str {
    match kind {
        NoticeKind::Error => "error",
        NoticeKind::Warning => "warning",
    }
}

/// What the dialog opens with (`selectBestTool`): for a project without an
/// interpreter that uses uv, its uv environment (Select existing / uv) or
/// a new one (Generate new / uv); otherwise Generate new / Virtualenv.
fn best_guess(
    root: &Path,
    chosen: Option<&super::Choice>,
    table: &Table,
) -> Option<(Method, NewType, ExistingType)> {
    let has_interpreter = super::scan(root, chosen, table).selected.is_some();
    if has_interpreter || !candidates::is_uv_project(root) {
        return None;
    }
    Some(if candidates::uv_environments(root).is_empty() {
        (Method::New, NewType::Uv, ExistingType::Python)
    } else {
        (Method::Existing, NewType::Virtualenv, ExistingType::Uv)
    })
}

// ---- drawing -----------------------------------------------------------------------------

impl Focusable for AddDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl AddDialog {
    fn anchor(&self, list: ListId) -> impl IntoElement {
        let anchors = self.anchors.clone();
        canvas(
            move |bounds, _, _| {
                anchors.borrow_mut().insert(list, bounds);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    /// A row: the label column, then the value.
    fn row(&self, label: Option<&'static str>, value: impl IntoElement) -> gpui::Div {
        div()
            .flex()
            .flex_row()
            .items_center()
            .min_h(px(FIELD))
            .child(
                div()
                    .flex_none()
                    .w(px(self.label_width.get()))
                    .whitespace_nowrap()
                    .children(label),
            )
            .child(div().flex_1().min_w(px(0.0)).child(value))
    }

    fn render_environment(&self, cx: &mut Context<Self>) -> AnyElement {
        let option =
            |id: &'static str, label: &'static str, method: Method, cx: &mut Context<Self>| {
                let selected = self.method == method;
                ui::form::radio_row(
                    SharedString::from(format!("add-interpreter-{id}")),
                    SharedString::from(format!("add-interpreter-{id}-radio")),
                    selected,
                    label,
                    cx,
                )
                .debug_selector(move || format!("add-interpreter:{id}"))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.method = method;
                    this.close_menu(cx);
                    cx.notify();
                }))
            };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(OPTION_GAP))
            .child(option("generate-new", GENERATE_NEW, Method::New, cx))
            .child(option(
                "select-existing",
                SELECT_EXISTING,
                Method::Existing,
                cx,
            ))
            .into_any_element()
    }

    fn render_type(&self, cx: &mut Context<Self>) -> AnyElement {
        let (label, icon) = match self.type_id() {
            "virtualenv" => (VIRTUALENV, VENV_ICON),
            "uv" => (UV, candidates::UV_ICON),
            _ => (PYTHON, candidates::PYTHON_ICON),
        };
        let open = self.menu.as_ref().is_some_and(|(l, _)| *l == ListId::Type);
        let focus = cx.theme().color(UiRole::Focus);
        ui::form::combo(
            "add-interpreter-type",
            vec![
                Icon::new(icon).into_any_element(),
                div().child(label).into_any_element(),
            ],
            cx,
        )
        .debug_selector(|| "add-interpreter:type".into())
        .w(px(TYPE_WIDTH))
        .h(px(FIELD))
        .when(open, |d| d.border_color(focus))
        .on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| {
                this.open_list(ListId::Type, window, cx)
            }),
        )
        .child(self.anchor(ListId::Type))
        .into_any_element()
    }

    /// An interpreter combo (`PythonInterpreterComboBox`): the selected
    /// one drawn as its row, the Browse button inside, the arrow button.
    fn render_interpreter_combo(&self, list: ListId, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let small = theme.ui_size() - px(1.0);
        let border = theme.color(UiRole::BorderInput);
        let loading = self.list_of(list).is_none() || self.probing == Some(list);
        let error = matches!(self.combo_notice(list), Some((NoticeKind::Error, _)));
        let content: Vec<AnyElement> = if loading {
            vec![
                div()
                    .italic()
                    .text_color(muted)
                    .child(LOADING)
                    .into_any_element(),
            ]
        } else {
            match self.selected(list) {
                None => vec![
                    div()
                        .italic()
                        .text_color(muted)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(NOTHING_FOUND)
                        .into_any_element(),
                ],
                Some(c) => {
                    let (text, detail) = c.text();
                    let (icon, alpha) = c.icon();
                    vec![
                        div()
                            .flex_none()
                            .opacity(alpha)
                            .child(Icon::new(icon))
                            .into_any_element(),
                        div().flex_none().child(text).into_any_element(),
                        div()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(small)
                            .text_color(muted)
                            .child(detail)
                            .into_any_element(),
                    ]
                }
            }
        };
        let open = self.menu.as_ref().is_some_and(|(l, _)| *l == list);
        let id = list.id();
        let browse: AnyElement = if loading {
            ui::components::spinner(SharedString::from(format!("add-interpreter-{id}-busy")), cx)
                .into_any_element()
        } else {
            ui::form::field_button(
                SharedString::from(format!("add-interpreter-{id}-browse")),
                IconName::BROWSE,
                "",
                Some(Box::new(DialogBrowse {
                    field: id.to_owned(),
                })),
            )
        };
        let frame = ui::components::input_frame(open || error, error, cx)
            .id(SharedString::from(format!("add-interpreter-{id}")))
            .debug_selector(move || format!("add-interpreter:{id}"))
            .h(px(FIELD))
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .pl(px(8.0))
                    .pr(px(6.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .overflow_hidden()
                    .children(content),
            )
            .child(
                div()
                    .id(SharedString::from(format!(
                        "add-interpreter-{id}-browse-tip"
                    )))
                    .flex_none()
                    .pr(px(6.0))
                    .tooltip(Tooltip::text(BROWSE_TOOLTIP, None))
                    .child(browse),
            )
            .child(
                div()
                    .id(SharedString::from(format!("add-interpreter-{id}-arrow")))
                    .flex_none()
                    .w(px(ARROW_WIDTH))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_l_1()
                    .border_color(border)
                    .child(Icon::new(IconName::CHEVRON_DOWN)),
            )
            .when(loading, |d| d.opacity(DISABLED_ALPHA))
            .when(!loading, |d| {
                d.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.open_list(list, window, cx)
                }))
            })
            .child(self.anchor(list));
        frame.into_any_element()
    }

    /// The Python version combo with its spinner while uv is asked.
    fn render_version(&self, cx: &mut Context<Self>) -> AnyElement {
        let loading = self.versions.is_none();
        let open = self
            .menu
            .as_ref()
            .is_some_and(|(l, _)| *l == ListId::Version);
        let focus = cx.theme().color(UiRole::Focus);
        let combo = ui::form::combo(
            "add-interpreter-python-version",
            vec![div().child(self.version_text()).into_any_element()],
            cx,
        )
        .debug_selector(|| "add-interpreter:python-version".into())
        .w(px(VERSION_WIDTH))
        .h(px(FIELD))
        .when(open, |d| d.border_color(focus))
        .when(loading, |d| d.opacity(DISABLED_ALPHA))
        .when(!loading, |d| {
            d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.open_list(ListId::Version, window, cx)
            }))
        })
        .child(self.anchor(ListId::Version));
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(combo)
            .when(loading, |d| {
                d.child(ui::components::spinner("add-interpreter-versions-busy", cx))
            })
            .into_any_element()
    }

    /// A path field (`ValidatedPathField`): the text field, a check mark
    /// once uv is found or a spinner while it is checked, the Browse
    /// button; a red outline while it is wrong, its message as a tooltip.
    fn render_path_field(
        &self,
        which: Which,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (editor, field_id, problem, running, found) = match which {
            Which::VenvLocation => (
                &self.venv_location.editor,
                F_LOCATION,
                self.location_problem(which),
                self.venv_location.check == Check::Running,
                false,
            ),
            Which::UvLocation => (
                &self.uv_location.editor,
                F_LOCATION,
                self.location_problem(which),
                self.uv_location.check == Check::Running,
                false,
            ),
            Which::Uv => (
                &self.uv.editor,
                F_UV,
                self.uv_problem(),
                self.uv.check == Check::Running,
                self.uv_found(),
            ),
        };
        let focused = editor.focus_handle(cx).is_focused(window);
        let dirty = problem.as_deref() == Some(PRESS_ENTER);
        let error = problem.is_some() && !running && !dirty;
        let theme = cx.theme();
        let warning = theme.color(UiRole::Warning);
        let mut frame = ui::components::input_frame(focused || error, error, cx)
            .id(SharedString::from(format!(
                "add-interpreter-{field_id}-{which:?}"
            )))
            .debug_selector(move || format!("add-interpreter:{field_id}"))
            .h(px(FIELD))
            .pl(px(8.0))
            .pr(px(7.0))
            .gap(px(6.0))
            .when(dirty, |d| d.border_color(warning))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(editor::field_line_height_for(true, window, cx))
                    .overflow_hidden()
                    .when(running, |d| d.opacity(DISABLED_ALPHA))
                    .child(editor.clone()),
            )
            .when(running, |d| {
                d.child(ui::components::spinner(
                    SharedString::from(format!("add-interpreter-{field_id}-busy")),
                    cx,
                ))
            })
            .when(found && !running, |d| {
                let tip = match &self.uv.check {
                    Check::Done(UvCheck::Found { version, .. }) => version.clone(),
                    _ => String::new(),
                };
                d.child(
                    div()
                        .id("add-interpreter-uv-found")
                        .flex_none()
                        .child(Icon::new(CHECKMARK_ICON))
                        .tooltip(Tooltip::text(tip, None)),
                )
            })
            .child(ui::form::browse_button(
                SharedString::from(format!("add-interpreter-{field_id}-browse-{which:?}")),
                Some(Box::new(DialogBrowse {
                    field: field_id.to_owned(),
                })),
            ));
        if let Some(problem) = problem.filter(|_| error || dirty) {
            frame = frame.tooltip(Tooltip::text(problem, None));
        }
        let ring = if error {
            Some(theme.color(UiRole::Danger))
        } else {
            None
        };
        // The error outline is wider than the focus ring
        // (`Component.errorFocusColor` around the field).
        div()
            .relative()
            .child(frame)
            .when_some(ring, |d, ring| {
                d.child(
                    div()
                        .absolute()
                        .top(px(-ERROR_RING))
                        .left(px(-ERROR_RING))
                        .right(px(-ERROR_RING))
                        .bottom(px(-ERROR_RING))
                        .rounded(ui::form::field_radius(cx) + px(ERROR_RING))
                        .border(px(ERROR_RING))
                        .border_color(ring),
                )
            })
            .into_any_element()
    }

    /// A notice (`validationTooltip`): the icon and the text, the links
    /// below it (or after it, `inline`), in the error or warning colours.
    fn notice(
        id: &'static str,
        kind: NoticeKind,
        text: &str,
        links: Vec<(&'static str, Box<dyn gpui::Action>, bool)>,
        inline: bool,
        cx: &App,
    ) -> AnyElement {
        let theme = cx.theme();
        let (fg, bg, border, icon) = match kind {
            NoticeKind::Error => (
                theme.color(UiRole::Danger),
                theme.color(UiRole::DangerBg),
                theme.color(UiRole::Danger),
                IconName::ERROR,
            ),
            NoticeKind::Warning => (
                theme.color(UiRole::Warning),
                theme.color(UiRole::WarningBg),
                theme.color(UiRole::Warning),
                IconName::WARNING,
            ),
        };
        let links: Vec<AnyElement> = links
            .into_iter()
            .enumerate()
            .map(|(ix, (label, action, external))| {
                ui::link::link(SharedString::from(format!("{id}-link-{ix}")), label, cx)
                    .debug_selector(move || format!("{id}:link:{label}"))
                    .flex_none()
                    .when(external, |d| d.child(Icon::new(EXTERNAL_LINK_ICON)))
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
                    .into_any_element()
            })
            .collect();
        let lines = text
            .lines()
            .map(|l| div().child(l.to_owned()))
            .collect::<Vec<_>>();
        let text = div().flex().flex_col().text_color(fg).children(lines);
        let pad = if inline || links.is_empty() {
            (NOTICE_INLINE_PAD, NOTICE_INLINE_PAD)
        } else {
            NOTICE_PAD
        };
        let body = if inline {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(NOTICE_ICON_GAP))
                .child(Icon::new(icon))
                .child(text)
                .children(links)
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(NOTICE_LINK_GAP))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_start()
                        .gap(px(NOTICE_ICON_GAP))
                        .child(div().flex_none().pt(px(1.0)).child(Icon::new(icon)))
                        .child(text),
                )
                .when(!links.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(NOTICE_PAD.1))
                            .pl(px(16.0 + NOTICE_ICON_GAP))
                            .children(links),
                    )
                })
                .into_any_element()
        };
        div()
            .debug_selector(move || id.to_owned())
            .w_full()
            .pt(px(pad.0))
            .pb(px(pad.0 + 2.0))
            .px(px(pad.1))
            .rounded(px(NOTICE_RADIUS))
            .border_1()
            .border_color(border)
            .bg(bg)
            .child(body)
            .into_any_element()
    }

    fn render_check(&self, field: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let (checked, label) = match field {
            F_INHERIT => (self.inherit, INHERIT),
            _ => (self.all_projects, ALL_PROJECTS),
        };
        ui::form::check_row(
            SharedString::from(format!("add-interpreter-{field}")),
            SharedString::from(format!("add-interpreter-{field}-box")),
            checked,
            label,
            cx,
        )
        .debug_selector(move || format!("add-interpreter:{field}"))
        .h(px(CHECK_ROW))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            match field {
                F_INHERIT => this.inherit = !this.inherit,
                _ => this.all_projects = !this.all_projects,
            }
            cx.notify();
        }))
        .into_any_element()
    }

    fn render_row(&self, row: Row, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match row {
            Row::Environment => self.row(Some(ENVIRONMENT), self.render_environment(cx)),
            Row::Type => self.row(Some(TYPE), self.render_type(cx)),
            Row::Combo(list) => {
                self.row(Some(row.label()), self.render_interpreter_combo(list, cx))
            }
            Row::ComboNotice(list) => {
                let Some((kind, text)) = self.combo_notice(list) else {
                    return div().into_any_element();
                };
                let learn = (
                    LEARN_MORE,
                    Box::new(DialogLearnMore) as Box<dyn gpui::Action>,
                    true,
                );
                self.row(
                    None,
                    Self::notice(
                        "add-interpreter-version-notice",
                        kind,
                        &text,
                        vec![learn],
                        true,
                        cx,
                    ),
                )
            }
            Row::Location(which) => {
                self.row(Some(LOCATION), self.render_path_field(which, window, cx))
            }
            Row::LocationNotice(which) => {
                let select: Box<dyn gpui::Action> = Box::new(DialogSelectExisting);
                let notice = match which {
                    Which::UvLocation => match self.uv_alert() {
                        Some(UvAlert::Error(text)) => Self::notice(
                            "add-interpreter-alert",
                            NoticeKind::Error,
                            &text,
                            vec![
                                (OVERRIDE_LINK, Box::new(DialogOverride), false),
                                (SELECT_EXISTING_LINK, select, false),
                            ],
                            false,
                            cx,
                        ),
                        Some(UvAlert::Warning(text)) => Self::notice(
                            "add-interpreter-alert",
                            NoticeKind::Warning,
                            &text,
                            vec![(SELECT_EXISTING_LINK, select, false)],
                            false,
                            cx,
                        ),
                        None => div().into_any_element(),
                    },
                    _ => match &self.venv_location.check {
                        Check::Done(check) => Self::notice(
                            "add-interpreter-alert",
                            NoticeKind::Error,
                            &check.message().unwrap_or_default(),
                            vec![(SELECT_EXISTING_LINK, select, false)],
                            false,
                            cx,
                        ),
                        _ => div().into_any_element(),
                    },
                };
                self.row(None, notice)
            }
            Row::Check(field) => self
                .row(None, self.render_check(field, cx))
                .min_h(px(CHECK_ROW)),
            Row::Version => self.row(Some(PYTHON_VERSION), self.render_version(cx)),
            Row::UvMissing => self.row(
                None,
                Self::notice(
                    "add-interpreter-uv-missing",
                    NoticeKind::Warning,
                    NO_UV,
                    vec![
                        (INSTALL_UV, Box::new(DialogInstallUv), false),
                        (
                            SELECT_PATH,
                            Box::new(DialogBrowse {
                                field: F_UV.to_owned(),
                            }),
                            false,
                        ),
                    ],
                    true,
                    cx,
                ),
            ),
            Row::Uv => self.row(
                Some(PATH_TO_UV),
                self.render_path_field(Which::Uv, window, cx),
            ),
        }
        .into_any_element()
    }

    /// The space above a row (the reference's row gaps).
    fn gap_before(prev: Option<Row>, row: Row) -> f32 {
        match (prev, row) {
            (None, _) => 0.0,
            (Some(Row::Environment), _) => FIRST_GAP,
            (_, Row::ComboNotice(_) | Row::LocationNotice(_)) => NOTICE_GAP,
            (Some(Row::Check(_)), Row::Check(_)) => CHECK_PITCH - CHECK_ROW,
            (Some(Row::LocationNotice(_) | Row::ComboNotice(_)), Row::Check(_)) => 10.0,
            (_, Row::Check(_)) => 9.0,
            (Some(Row::UvMissing), _) => 8.0,
            _ => ROW_GAP,
        }
    }

    fn render_footer(&self, cx: &App) -> AnyElement {
        BottomBar::new()
            .separator(false)
            .help(ui::form::help_button("add-interpreter-help", None, cx))
            .button(
                BarButton::new(
                    "add-interpreter-ok",
                    "OK",
                    ButtonRole::Ok,
                    Box::new(DialogOk),
                )
                .enabled(self.ok_enabled()),
            )
            .button(BarButton::new(
                "add-interpreter-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(DialogCancel),
            ))
            .render(cx)
    }
}

impl AddDialog {
    /// The widest label of the rows shown, and the gap after it.
    fn measure_labels(&self, window: &Window, cx: &App) -> f32 {
        let theme = cx.theme();
        let font = gpui::font(theme.ui_family());
        let widest = self
            .rows()
            .iter()
            .filter(|r| {
                !matches!(
                    r,
                    Row::ComboNotice(_) | Row::LocationNotice(_) | Row::Check(_) | Row::UvMissing
                )
            })
            .map(|r| {
                let label = r.label();
                let run = gpui::TextRun {
                    len: label.len(),
                    font: font.clone(),
                    color: gpui::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(
                    SharedString::from(label),
                    theme.ui_size(),
                    &[run],
                    None,
                );
                f32::from(line.width)
            })
            .fold(0.0, f32::max);
        (widest + LABEL_GAP).ceil().max(LABEL_WIDTH)
    }
}

impl Render for AddDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.label_width.set(self.measure_labels(window, cx));
        let theme = cx.theme();
        let (surface, text) = (theme.color(UiRole::Surface), theme.color(UiRole::Text));
        let family = theme.ui_family();
        let size = theme.ui_size();
        let mut prev = None;
        let mut rows: Vec<AnyElement> = Vec::new();
        for row in self.rows() {
            let gap = Self::gap_before(prev, row);
            rows.push(
                div()
                    .pt(px(gap))
                    .child(self.render_row(row, window, cx))
                    .into_any_element(),
            );
            prev = Some(row);
        }
        let footer = self.render_footer(cx);
        div()
            .id("add-interpreter")
            .debug_selector(|| "add-interpreter".into())
            .key_context("AddPythonInterpreterDialog")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_set))
            .on_action(cx.listener(Self::on_open_list))
            .on_action(cx.listener(Self::on_validate))
            .on_action(cx.listener(Self::on_browse))
            .on_action(cx.listener(Self::on_override))
            .on_action(cx.listener(Self::on_select_existing))
            .on_action(cx.listener(Self::on_install_uv))
            .on_action(cx.listener(Self::on_learn_more))
            .on_action(cx.listener(Self::on_ok))
            .on_action(cx.listener(Self::on_cancel))
            // Enter and Escape in the text fields (the editor's own
            // actions), before the fields take them.
            .capture_action(
                cx.listener(|this, _: &editor::commands::Newline, window, cx| {
                    this.on_enter(window, cx);
                    cx.stop_propagation();
                }),
            )
            .capture_action(
                cx.listener(|this, _: &editor::commands::Escape, window, cx| {
                    this.close(window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let m = &e.keystroke.modifiers;
                if m.control || m.alt || m.platform || m.shift {
                    return;
                }
                match e.keystroke.key.as_str() {
                    "enter" => this.on_enter(window, cx),
                    "escape" => this.close(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.validate_shown(cx)),
            )
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
                    .id("add-interpreter-form")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .pt(px(PAD_TOP))
                    .pl(px(PAD_LEFT))
                    .pr(px(PAD_RIGHT))
                    .child(div().flex().flex_col().children(rows)),
            )
            .child(footer)
            .child(ui::pointer::frame())
    }
}

/// Opens the dialog over `window` once the action that asks for it is
/// done (a popup's row runs while its window is updated).
pub(super) fn open_later(window: AnyWindowHandle, root: PathBuf, select: Select, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| open(root, select, window, cx));
    });
}

/// Saves the documents before the dialog opens (`saveAllDocuments`).
pub(super) fn save_all(window: AnyWindowHandle, cx: &mut App) {
    let _ = window.update(cx, |_, window, cx| {
        window.dispatch_action(Box::new(workbench::commands::SaveAll), cx)
    });
}
