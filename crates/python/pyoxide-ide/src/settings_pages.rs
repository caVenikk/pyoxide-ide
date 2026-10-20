//! The Python pages of the Settings window (ADR 0017): the Python group,
//! Python > Type Engine (`PyTypeEngineConfigurable`) and Python > Tools >
//! External Tools (`PyExternalToolsConfigurable`), over the project's tool
//! settings (`py_lsp::settings`: `.oxide/project.toml`, and the custom
//! executables in `.oxide/workspace.toml`). Both pages are project pages:
//! they read the settings off the UI thread when they open, write them off
//! it on Apply, and the project's servers are prepared again.
//!
//! Not drawn yet on External Tools: the Configure dialog behind the
//! Tool column's gear, Install / Upgrade of a tool and "Install uv", the
//! tooltips of the cells and the "All actions on save…" link.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, DismissEvent, Entity,
    FocusHandle, IntoElement, PathPromptOptions, Pixels, Render, SharedString, Subscription, Task,
    Window, canvas, div, prelude::*, px,
};
use py_lsp::lookup::{self, Lookup, LookupContext, SearchPaths, Stage};
use py_lsp::settings::{self, Settings, Tool, TypeEngine};
use serde_json::{Value as Json, json};
use settings_ui::commands::EditField;
use settings_ui::{PageContext, PageSpec, Scope, SettingsPage};
use themes::UiRole;
use ui::components::Tooltip;
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::pointer::PointerExt as _;
use ui::theme::ThemeExt;

use crate::services::{refresh_root, services_for};

/// Ids of the pages.
pub const PYTHON: &str = "python";
pub const TYPE_ENGINE: &str = "python.type-engine";
pub const DEBUGGER: &str = "python.debugger";
pub const TOOLS: &str = "python.tools";
pub const EXTERNAL_TOOLS: &str = "python.tools.external-tools";

/// The reference's texts (PyBundle, PyLspCoreBundle, PyToolsUiBundle).
const PYTHON_DESCRIPTION: &str = "Configure key Python development settings to customize your coding, debugging, data visualization, and documentation workflows.";
const TOOLS_DESCRIPTION: &str = "Configure external Python tool settings.";
const ENGINE_LABEL: &str = "Type engine:";
const TY_DESCRIPTION: &str = "Currently in Beta, ty provides fast and accurate type checking designed to work well with untyped code. Supports advanced type analysis features.";
/// Pyoxide's own: the reference does not offer basedpyright.
const BASEDPYRIGHT_DESCRIPTION: &str = "Basedpyright, a fork of Pyright with stricter checks and features of Pylance, provides type checking, completion and navigation through its language server.";
const TOOLS_TEXT: &str = "Enable and configure tools, choose a lookup method, and optionally override the path. The tool can be detected from sdk and path, or you can run it manually without installation via the 'uvx \u{2039}tool\u{203A}' command.";
const COLUMN_TOOL: &str = "Tool";
const COLUMN_LOOKUP: &str = "Lookup";
const COLUMN_PATH: &str = "Executable Path";
const NOT_FOUND: &str = "Not found in $PATH";
const NO_FEATURES: &str = "No features selected";
const UV_MISSING: &str = "uv is not installed on your system. install uv to use uvx running mode.";
const BROWSE_TOOLTIP: &str = "Browse for executable";
const RESET_TOOLTIP: &str = "Use auto-detection";
const NO_PROJECT: &str = "Open a project to configure its Python tools.";
/// Feature names of the Tool column (PyToolsUiBundle, RuffBundle).
const INSPECTIONS: &str = "Inspections";
const FORMATTING: &str = "Formatting";
const IMPORT_OPTIMIZER: &str = "Import optimizer";

const TYPE_ENGINE_TERMS: &[&str] = &[ENGINE_LABEL, "ty", "Basedpyright"];
const EXTERNAL_TOOLS_TERMS: &[&str] = &["Ruff", "ty", "Basedpyright", "uvx"];

/// The Python group (`python.group.settings`, weight 1000: the first
/// top-level row), Interpreter (weight 200), Type Engine (group weight
/// 190), the Tools group (weight 72) and External Tools under it.
pub fn pages() -> Vec<PageSpec> {
    vec![
        PageSpec {
            weight: 1000,
            description: Some(PYTHON_DESCRIPTION),
            ..PageSpec::group(PYTHON, None, "Python")
        },
        crate::interpreter::page_spec(),
        PageSpec {
            id: TYPE_ENGINE,
            parent: Some(PYTHON),
            title: "Type Engine",
            weight: 190,
            scope: Scope::ProjectShared,
            search_terms: TYPE_ENGINE_TERMS,
            description: None,
            create: Some(settings_ui::page::create(TypeEnginePage::new)),
        },
        // Debugger: weight 80, as `PyCharmProjectConfigurableStartupActivity`
        // puts it in the Python group.
        PageSpec {
            id: DEBUGGER,
            parent: Some(PYTHON),
            title: "Debugger",
            weight: 80,
            scope: Scope::ProjectPersonal,
            search_terms: crate::debugger_settings::SEARCH_TERMS,
            description: None,
            create: Some(settings_ui::page::create(
                crate::debugger_settings::DebuggerPage::new,
            )),
        },
        PageSpec {
            weight: 72,
            description: Some(TOOLS_DESCRIPTION),
            ..PageSpec::group(TOOLS, Some(PYTHON), "Tools")
        },
        PageSpec {
            id: EXTERNAL_TOOLS,
            parent: Some(TOOLS),
            title: "External Tools",
            weight: 0,
            scope: Scope::ProjectShared,
            search_terms: EXTERNAL_TOOLS_TERMS,
            description: None,
            create: Some(settings_ui::page::create(ExternalToolsPage::new)),
        },
    ]
}

// ---- the stored settings ---------------------------------------------------------

/// The project's tool settings and the custom executables.
#[derive(Debug, Clone, PartialEq)]
struct Stored {
    settings: Settings,
    paths: Vec<(Tool, Option<PathBuf>)>,
}

impl Stored {
    fn custom_path(&self, tool: Tool) -> Option<PathBuf> {
        self.paths
            .iter()
            .find(|(t, _)| *t == tool)
            .and_then(|(_, p)| p.clone())
    }
}

/// Reads the settings (blocking: run it off the UI thread).
fn read_stored(root: &Path) -> Result<Stored, String> {
    let settings = settings::load(root).map_err(|e| e.to_string())?;
    let paths = Tool::ALL
        .into_iter()
        .map(|tool| {
            let path = settings::custom_path(root, tool).map_err(|e| e.to_string())?;
            Ok((tool, path))
        })
        .collect::<Result<_, String>>()?;
    Ok(Stored { settings, paths })
}

/// The pages' state while the settings are read.
#[derive(Debug, Clone, PartialEq)]
enum Loaded {
    Reading,
    Read(Stored),
    Failed(String),
}

impl Loaded {
    fn stored(&self) -> Option<&Stored> {
        match self {
            Loaded::Read(s) => Some(s),
            _ => None,
        }
    }
}

/// Reads the settings of `root` in the background and hands them to `done`
/// on the page.
fn spawn_read<V: 'static>(
    root: PathBuf,
    cx: &mut Context<V>,
    done: impl FnOnce(&mut V, Loaded, &mut Context<V>) + 'static,
) -> Task<()> {
    let work = cx.background_spawn(async move { read_stored(&root) });
    cx.spawn(async move |this, cx| {
        let loaded = match work.await {
            Ok(stored) => Loaded::Read(stored),
            Err(e) => Loaded::Failed(e),
        };
        let _ = this.update(cx, |this, cx| {
            done(this, loaded, cx);
            cx.notify();
        });
    })
}

/// Writes in the background, then prepares the project's servers again;
/// `done` hears of an error. The write outlives the page (OK closes the
/// window at once).
fn spawn_write<V: 'static>(
    root: PathBuf,
    write: impl FnOnce(&Path) -> Result<(), String> + Send + 'static,
    cx: &mut Context<V>,
    done: impl FnOnce(&mut V, Option<String>, &mut Context<V>) + 'static,
) {
    let work = cx.background_spawn({
        let root = root.clone();
        async move { write(&root) }
    });
    cx.spawn(async move |this, cx| {
        let error = work.await.err();
        if let Some(e) = &error {
            eprintln!("python settings: {e}");
        }
        let _ = cx.update(|cx| refresh_root(&root, cx));
        let _ = this.update(cx, |this, cx| {
            done(this, error, cx);
            cx.notify();
        });
    })
    .detach();
}

/// A page's note in place of its options (no project, a settings file that
/// cannot be read).
fn note(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .text_color(cx.theme().color(UiRole::TextMuted))
        .child(text.into())
        .into_any_element()
}

/// A comment under a row (Kotlin UI DSL `comment`): muted, one size
/// smaller, wrapped at the reference's comment width.
fn comment(text: &'static str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .max_w(px(COMMENT_WIDTH))
        .text_size(theme.ui_size() - px(1.0))
        .text_color(theme.color(UiRole::TextMuted))
        .child(wrapped(text, cx))
        .into_any_element()
}

/// Text that wraps, lit up while the window's search finds it.
fn wrapped(text: &'static str, cx: &App) -> AnyElement {
    settings_ui::search::spotlight(text, div().child(text), cx)
}

/// The widest a comment is drawn (`DEFAULT_COMMENT_WIDTH`, 70 columns).
const COMMENT_WIDTH: f32 = 520.0;

// ---- Type Engine -----------------------------------------------------------------

/// Python > Type Engine: "Type engine:" with a segmented button of the type
/// servers and the description of the chosen one under it.
pub struct TypeEnginePage {
    root: Option<PathBuf>,
    loaded: Loaded,
    /// The engine chosen on the page, not applied yet.
    draft: Option<TypeEngine>,
    error: Option<String>,
    focus: FocusHandle,
    _work: Task<()>,
}

impl TypeEnginePage {
    pub fn new(context: &PageContext, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            root: context.project.clone(),
            loaded: Loaded::Reading,
            draft: None,
            error: None,
            focus: cx.focus_handle(),
            _work: Task::ready(()),
        };
        this.read(cx);
        this
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        self._work = spawn_read(root, cx, |this, loaded, _| this.loaded = loaded);
    }

    fn stored(&self) -> Option<TypeEngine> {
        Some(self.loaded.stored()?.settings.type_engine)
    }

    /// The engine the page shows.
    fn engine(&self) -> Option<TypeEngine> {
        self.draft.or(self.stored())
    }

    fn choose(&mut self, engine: TypeEngine, cx: &mut Context<Self>) {
        if self.loaded.stored().is_some() {
            self.draft = Some(engine);
            cx.notify();
        }
    }
}

impl SettingsPage for TypeEnginePage {
    fn is_modified(&self, _: &App) -> bool {
        self.draft.is_some_and(|d| Some(d) != self.stored())
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let (Some(root), Some(engine)) = (self.root.clone(), self.draft) else {
            return Ok(());
        };
        if Some(engine) == self.stored() {
            self.draft = None;
            return Ok(());
        }
        if let Loaded::Read(stored) = &mut self.loaded {
            stored.settings.select_type_engine(engine);
        }
        self.draft = None;
        spawn_write(
            root,
            move |root| {
                settings::select_type_engine(root, engine)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            },
            cx,
            |this, error, _| this.error = error,
        );
        cx.notify();
        Ok(())
    }

    fn reset(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.draft = None;
        self.error = None;
        self.read(cx);
        cx.notify();
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, _: &App) -> Json {
        json!({
            "project": self.root.as_ref().map(|r| r.display().to_string()),
            "loaded": self.loaded.stored().is_some(),
            "engine": self.engine().map(TypeEngine::id),
            "engines": TypeEngine::ALL.map(TypeEngine::name),
            "description": self.engine().map(description),
            "error": self.error.clone().or_else(|| match &self.loaded {
                Loaded::Failed(e) => Some(e.clone()),
                _ => None,
            }),
        })
    }

    fn edit(
        &mut self,
        field: &str,
        value: &Json,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        match field {
            "engine" => {
                let engine = value
                    .as_str()
                    .and_then(TypeEngine::parse)
                    .ok_or("`engine` takes `ty` or `basedpyright`")?;
                if self.loaded.stored().is_none() {
                    return Err("the settings are not read yet".into());
                }
                self.choose(engine, cx);
                Ok(())
            }
            _ => Err(format!("no field `{field}` on this page")),
        }
    }
}

/// The description shown under the segmented button.
fn description(engine: TypeEngine) -> &'static str {
    match engine {
        TypeEngine::Ty => TY_DESCRIPTION,
        TypeEngine::Basedpyright => BASEDPYRIGHT_DESCRIPTION,
    }
}

impl Render for TypeEnginePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = div()
            .id("type-engine-page")
            .debug_selector(|| "type-engine-page".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(ROW_GAP));
        if self.root.is_none() {
            return page.child(note(NO_PROJECT, cx));
        }
        if let Loaded::Failed(e) = &self.loaded {
            return page.child(note(e.clone(), cx));
        }
        let choices: Vec<SharedString> = TypeEngine::ALL
            .iter()
            .map(|e| SharedString::from(e.name()))
            .collect();
        let selected = self
            .engine()
            .and_then(|e| TypeEngine::ALL.iter().position(|x| *x == e));
        let page_entity = cx.entity().downgrade();
        let buttons = ui::form::segmented(
            "type-engine",
            &choices,
            selected,
            move |ix, _, cx| {
                let _ = page_entity.update(cx, |this, cx| this.choose(TypeEngine::ALL[ix], cx));
            },
            cx,
        );
        let row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(LABEL_GAP))
            .child(settings_ui::search::label(ENGINE_LABEL, cx))
            .child(buttons);
        page.child(row)
            .children(self.engine().map(|e| comment(description(e), cx)))
            .children(
                self.error
                    .clone()
                    .map(|e| div().text_color(cx.theme().color(UiRole::Danger)).child(e)),
            )
    }
}

/// Kotlin UI DSL: the gap after a row's label and between rows.
const LABEL_GAP: f32 = 6.0;
const ROW_GAP: f32 = 10.0;

// ---- External Tools ----------------------------------------------------------------

/// The table's metrics (`PyExternalToolsTable.configureView`): the switch
/// column 60 wide, Tool 220, Lookup 170, the path the rest; the header
/// strip 22 high, the cells' text 5 from their edges.
const COLUMNS: [f32; 3] = [60.0, 220.0, 170.0];
const HEADER_HEIGHT: f32 = 22.0;
const ROW_HEIGHT: f32 = 24.0;
const CELL_PAD: f32 = 5.0;
const SUMMARY_GAP: f32 = 8.0;
const ICON_GAP: f32 = 4.0;
/// The colours of the reference's table (`PyExternalToolsLabels`): the
/// features of an enabled tool, the "no features" warning, and the marks
/// of the lookup steps; light and dark.
const FEATURES: [u32; 2] = [0x59A869, 0x6A8759];
const NO_FEATURES_COLOR: [u32; 2] = [0xE08855, 0xCB7B57];
const STEP_FOUND: [u32; 2] = [0x59A869, 0x6A8759];
const STEP_NOT_FOUND: [u32; 2] = [0xC75450, 0xCF5B56];

fn themed(colors: [u32; 2], cx: &App) -> gpui::Hsla {
    gpui::rgb(colors[usize::from(cx.theme().is_dark())]).into()
}

/// The tools in the table's order (by name, ignoring case).
fn table_tools() -> Vec<Tool> {
    let mut tools = Tool::ALL.to_vec();
    tools.sort_by_key(|t| t.name().to_lowercase());
    tools
}

/// A row as the page edits it (`RowState`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    tool: Tool,
    enabled: bool,
    lookup: Lookup,
    custom_path: Option<PathBuf>,
}

fn rows_of(stored: &Stored) -> Vec<Row> {
    table_tools()
        .into_iter()
        .map(|tool| {
            let settings = stored.settings.tool(tool);
            Row {
                tool,
                enabled: settings.enabled,
                lookup: settings.lookup,
                custom_path: stored.custom_path(tool),
            }
        })
        .collect()
}

/// The features of a tool the Tool column lists (`summaryFor`).
fn features(tool: Tool, stored: &Stored) -> Vec<&'static str> {
    let settings = stored.settings.tool(tool);
    let mut out = Vec::new();
    if settings.inspections {
        out.push(INSPECTIONS);
    }
    if tool == Tool::Ruff {
        if settings.formatting {
            out.push(FORMATTING);
        }
        if settings.import_optimizer {
            out.push(IMPORT_OPTIMIZER);
        }
    }
    out
}

/// What the Path column shows (`PathFieldValue`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum PathValue {
    Custom(PathBuf),
    Detected(PathBuf),
    NotFound,
}

/// What the lookup of a tool found (the marks of the Lookup column and
/// the Path column).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    /// The custom path it was looked up with.
    custom_path: Option<PathBuf>,
    sdk: bool,
    path: PathValue,
    uvx: bool,
}

/// Looks every tool up (blocking: run it off the UI thread).
fn probe_tools(
    interpreter: Option<&Path>,
    search: Option<&SearchPaths>,
    rows: &[(Tool, Option<PathBuf>)],
) -> Vec<(Tool, Probe)> {
    let from_env;
    let search = match search {
        Some(s) => s,
        None => {
            from_env = SearchPaths::from_env();
            &from_env
        }
    };
    rows.iter()
        .map(|(tool, custom)| {
            let context = LookupContext {
                interpreter,
                custom_path: None,
                allow_uvx: true,
                search: Some(search),
            };
            let resolution = lookup::resolve(*tool, Lookup::Sdk, &context);
            let found = |stage: Stage| {
                resolution
                    .stages
                    .iter()
                    .find(|s| s.stage == stage)
                    .and_then(|s| s.found.clone())
            };
            let path = match custom {
                Some(custom) => PathValue::Custom(custom.clone()),
                None => found(Stage::Path).map_or(PathValue::NotFound, PathValue::Detected),
            };
            let probe = Probe {
                custom_path: custom.clone(),
                sdk: found(Stage::Sdk).is_some(),
                path,
                uvx: found(Stage::Uvx).is_some(),
            };
            (*tool, probe)
        })
        .collect()
}

/// The lookup modes with the steps each goes through (`modeChainLabel`).
fn chain(lookup: Lookup) -> &'static [Lookup] {
    match lookup {
        Lookup::Sdk => &[Lookup::Sdk, Lookup::Path, Lookup::Uvx],
        Lookup::Path => &[Lookup::Path, Lookup::Uvx],
        Lookup::Uvx => &[Lookup::Uvx],
    }
}

/// `Sdk · Path · uvx`.
fn chain_text(lookup: Lookup) -> String {
    chain(lookup)
        .iter()
        .map(|l| l.label())
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// Whether a step finds the tool (`None` while not looked up).
fn step_found(step: Lookup, probe: Option<&Probe>) -> Option<bool> {
    let probe = probe?;
    Some(match step {
        Lookup::Sdk => probe.sdk,
        Lookup::Path => probe.path != PathValue::NotFound,
        Lookup::Uvx => probe.uvx,
    })
}

/// `Sdk ✗ · Path ✓ · uvx ✓` (`?` while not looked up).
fn marked_chain_text(lookup: Lookup, probe: Option<&Probe>) -> String {
    chain(lookup)
        .iter()
        .map(|step| {
            let mark = match step_found(*step, probe) {
                None => '?',
                Some(true) => '\u{2713}',
                Some(false) => '\u{2717}',
            };
            format!("{} {mark}", step.label())
        })
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// Python > Tools > External Tools: the description, the table of the
/// tools (ON/OFF, Tool with its features, Lookup with the steps it goes
/// through and whether each finds the tool, Executable Path) and a note
/// when uv is missing.
pub struct ExternalToolsPage {
    root: Option<PathBuf>,
    loaded: Loaded,
    rows: Vec<Row>,
    probes: Vec<(Tool, Probe)>,
    /// The open Lookup list.
    list: Option<(Tool, Entity<ContextMenu>, Subscription)>,
    /// Where the Lookup cells are drawn, by row.
    anchors: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    error: Option<String>,
    focus: FocusHandle,
    _read: Task<()>,
    _probe: Task<()>,
}

impl ExternalToolsPage {
    pub fn new(context: &PageContext, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            root: context.project.clone(),
            loaded: Loaded::Reading,
            rows: Vec::new(),
            probes: Vec::new(),
            list: None,
            anchors: Rc::default(),
            error: None,
            focus: cx.focus_handle(),
            _read: Task::ready(()),
            _probe: Task::ready(()),
        };
        this.read(cx);
        this
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        self._read = spawn_read(root, cx, |this, loaded, cx| {
            if let Loaded::Read(stored) = &loaded {
                this.rows = rows_of(stored);
            }
            this.loaded = loaded;
            this.probe(cx);
        });
    }

    /// Looks the tools up again with the custom paths of the rows: the
    /// project interpreter first, then the lookup off the UI thread.
    fn probe(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let paths: Vec<(Tool, Option<PathBuf>)> = self
            .rows
            .iter()
            .map(|r| (r.tool, r.custom_path.clone()))
            .collect();
        let search = services_for(&root, cx)
            .first()
            .and_then(|s| s.read(cx).search());
        let interpreter = crate::interpreter::for_project(&root, cx);
        self._probe = cx.spawn(async move |this, cx| {
            let interpreter = interpreter.await.map(|s| s.python);
            let probes = cx
                .background_spawn(async move {
                    probe_tools(interpreter.as_deref(), search.as_deref(), &paths)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.probes = probes;
                cx.notify();
            });
        });
    }

    fn probe_of(&self, tool: Tool) -> Option<&Probe> {
        self.probes
            .iter()
            .find(|(t, _)| *t == tool)
            .map(|(_, p)| p)
            .filter(|p| {
                self.row(tool)
                    .is_some_and(|r| r.custom_path == p.custom_path)
            })
    }

    fn row(&self, tool: Tool) -> Option<&Row> {
        self.rows.iter().find(|r| r.tool == tool)
    }

    fn row_mut(&mut self, tool: Tool) -> Result<&mut Row, String> {
        self.rows
            .iter_mut()
            .find(|r| r.tool == tool)
            .ok_or_else(|| "the settings are not read yet".to_owned())
    }

    /// uv was looked for and not found.
    fn uv_missing(&self) -> bool {
        !self.probes.is_empty() && self.probes.iter().all(|(_, p)| !p.uvx)
    }

    fn toggle(&mut self, tool: Tool, cx: &mut Context<Self>) {
        if let Ok(row) = self.row_mut(tool) {
            row.enabled = !row.enabled;
            cx.notify();
        }
    }

    fn set_custom_path(&mut self, tool: Tool, path: Option<PathBuf>, cx: &mut Context<Self>) {
        let Ok(row) = self.row_mut(tool) else {
            return;
        };
        if row.custom_path != path {
            row.custom_path = path;
            self.probe(cx);
        }
        cx.notify();
    }

    fn browse(&mut self, tool: Tool, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| this.set_custom_path(tool, Some(path), cx));
            }
        })
        .detach();
    }

    fn open_list(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(cx);
        let Some(index) = self.rows.iter().position(|r| r.tool == tool) else {
            return;
        };
        let current = self.rows[index].lookup;
        let modes = [Lookup::Sdk, Lookup::Path, Lookup::Uvx];
        let items = modes
            .iter()
            .map(|mode| {
                MenuItem::action(
                    chain_text(*mode),
                    EditField {
                        field: format!("lookup:{}", tool.id()),
                        value: json!(mode.id()),
                    },
                )
            })
            .collect();
        let bounds = self
            .anchors
            .borrow()
            .get(index)
            .copied()
            .unwrap_or_default();
        // The chosen row runs where the focus is: on the page.
        window.focus(&self.focus);
        let menu = ContextMenu::popup(
            items,
            PopupOptions {
                selected: modes.iter().position(|m| *m == current),
                min_width: Some(f32::from(bounds.size.width)),
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.list = None;
            cx.notify();
        });
        self.list = Some((tool, menu, subscription));
        cx.notify();
    }

    fn close_list(&mut self, cx: &mut Context<Self>) {
        if self.list.take().is_some() {
            cx.notify();
        }
    }

    /// Closes the list and gives the focus back to the page (the list had
    /// it).
    fn close_list_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.list.is_some() {
            self.close_list(cx);
            window.focus(&self.focus);
        }
    }

    /// The settings to write for the rows: a switch turned is explicit
    /// (ADR 0010); the rest as stored.
    fn settings_for(stored: &Stored, rows: &[Row]) -> Settings {
        let mut settings = stored.settings.clone();
        for row in rows {
            let tool = settings.tool_mut(row.tool);
            if tool.enabled != row.enabled {
                tool.enabled = row.enabled;
                tool.explicit = true;
            }
            tool.lookup = row.lookup;
        }
        settings
    }

    // ---- drawing ----

    fn header(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let cell = |width: Option<f32>, text: &'static str| {
            div()
                .when_some(width, |d, w| d.flex_none().w(px(w)))
                .when(width.is_none(), |d| d.flex_1().min_w(px(0.0)))
                .px(px(CELL_PAD))
                .overflow_hidden()
                .whitespace_nowrap()
                .child(text)
        };
        div()
            .flex_none()
            .h(px(HEADER_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .bg(theme.color(UiRole::Surface))
            .border_b_1()
            .border_color(theme.color(UiRole::Border))
            .child(cell(Some(COLUMNS[0]), ""))
            .child(cell(Some(COLUMNS[1]), COLUMN_TOOL))
            .child(cell(Some(COLUMNS[2]), COLUMN_LOOKUP))
            .child(cell(None, COLUMN_PATH))
            .into_any_element()
    }

    fn render_row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let tool = row.tool;
        let id = tool.id();
        let small = theme.ui_size() - px(1.0);
        let stored = self.loaded.stored();
        let probe = self.probe_of(tool);
        let group: SharedString = format!("external-tool-{id}").into();

        let switch = div()
            .id(("external-tool-switch", index))
            .debug_selector(move || format!("external-tools:switch:{id}"))
            .flex_none()
            .w(px(COLUMNS[0]))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .pointing_hand()
            .child(ui::form::on_off(
                ("external-tool-on-off", index),
                row.enabled,
                cx,
            ))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(tool, cx)));

        let summary = stored.map(|s| features(tool, s)).unwrap_or_default();
        let summary = row.enabled.then(|| {
            if summary.is_empty() {
                (NO_FEATURES.to_owned(), themed(NO_FEATURES_COLOR, cx))
            } else {
                (format!("[{}]", summary.join(", ")), themed(FEATURES, cx))
            }
        });
        let name = div()
            .flex_none()
            .w(px(COLUMNS[1]))
            .px(px(CELL_PAD))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(SUMMARY_GAP))
            .overflow_hidden()
            .whitespace_nowrap()
            .child(settings_ui::search::label(tool.name(), cx))
            .children(summary.map(|(text, color)| {
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_size(small)
                    .text_color(color)
                    .child(text)
            }));

        let open = self.list.as_ref().is_some_and(|(t, _, _)| *t == tool);
        let anchors = self.anchors.clone();
        let steps = chain(row.lookup).iter().enumerate().map(|(n, step)| {
            let mark = step_found(*step, probe);
            let (glyph, color) = match mark {
                None => ("?", theme.color(UiRole::TextMuted)),
                Some(true) => ("\u{2713}", themed(STEP_FOUND, cx)),
                Some(false) => ("\u{2717}", themed(STEP_NOT_FOUND, cx)),
            };
            div()
                .flex()
                .flex_row()
                .when(n > 0, |d| d.child(div().px(px(4.0)).child("\u{b7}")))
                .child(step.label())
                .child(div().pl(px(3.0)).text_color(color).child(glyph))
        });
        let lookup = div()
            .id(("external-tool-lookup", index))
            .debug_selector(move || format!("external-tools:lookup:{id}"))
            .relative()
            .flex_none()
            .w(px(COLUMNS[2]))
            .h_full()
            .px(px(CELL_PAD))
            .flex()
            .flex_row()
            .items_center()
            .whitespace_nowrap()
            .child(div().flex_1().flex().flex_row().children(steps))
            .when(row.enabled, |d| {
                d.pointing_hand()
                    .child(Icon::new(IconName::CHEVRON_DOWN))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if this.list.as_ref().is_some_and(|(t, _, _)| *t == tool) {
                            this.close_list(cx);
                        } else {
                            this.open_list(tool, window, cx);
                        }
                    }))
            })
            .when(open, |d| d.bg(theme.color(UiRole::Hover)))
            .child(
                canvas(
                    move |bounds, _, _| {
                        let mut anchors = anchors.borrow_mut();
                        if anchors.len() <= index {
                            anchors.resize(index + 1, Bounds::default());
                        }
                        anchors[index] = bounds;
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );

        let (path_text, path_color, italic) = match probe.map(|p| &p.path) {
            None => (String::new(), theme.color(UiRole::Text), false),
            Some(PathValue::Custom(p)) => {
                (p.display().to_string(), theme.color(UiRole::Text), false)
            }
            Some(PathValue::Detected(p)) => (
                p.display().to_string(),
                theme.color(UiRole::TextMuted),
                true,
            ),
            Some(PathValue::NotFound) => {
                (NOT_FOUND.to_owned(), theme.color(UiRole::TextMuted), false)
            }
        };
        let custom = row.custom_path.is_some();
        let icons = row.enabled.then(|| {
            div()
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(ICON_GAP))
                .invisible()
                .group_hover(group.clone(), |s| s.visible())
                .when(custom, |d| {
                    d.child(
                        div()
                            .id(("external-tool-reset", index))
                            .debug_selector(move || format!("external-tools:reset:{id}"))
                            .pointing_hand()
                            .child(Icon::new(IconName::ROLLBACK))
                            .tooltip(Tooltip::text(RESET_TOOLTIP, None))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.set_custom_path(tool, None, cx)
                            })),
                    )
                })
                .child(
                    div()
                        .id(("external-tool-browse", index))
                        .debug_selector(move || format!("external-tools:browse:{id}"))
                        .pointing_hand()
                        .child(Icon::new(IconName::BROWSE))
                        .tooltip(Tooltip::text(BROWSE_TOOLTIP, None))
                        .on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| this.browse(tool, cx)),
                        ),
                )
        });
        let path = div()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .px(px(CELL_PAD))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ICON_GAP))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(path_color)
                    .when(italic, |d| d.italic())
                    .child(path_text),
            )
            .children(icons);

        div()
            .id(("external-tool-row", index))
            .debug_selector(move || format!("external-tools:row:{id}"))
            .group(group)
            .flex_none()
            .h(px(ROW_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            // Enabled tools on the table's stripe colour
            // (`UIUtil.getDecoratedRowColor`).
            .when(row.enabled, |d| d.bg(theme.color(UiRole::SurfaceAlt)))
            .child(switch)
            .child(name)
            .child(lookup)
            .child(path)
            .into_any_element()
    }

    fn row_json(&self, row: &Row) -> Json {
        let probe = self.probe_of(row.tool);
        let summary = self
            .loaded
            .stored()
            .map(|s| features(row.tool, s))
            .unwrap_or_default();
        json!({
            "tool": row.tool.id(),
            "name": row.tool.name(),
            "enabled": row.enabled,
            "features": if row.enabled { summary } else { Vec::new() },
            "lookup": row.lookup.id(),
            "chain": marked_chain_text(row.lookup, probe),
            "custom-path": row.custom_path.as_ref().map(|p| p.display().to_string()),
            "path": probe.map(|p| match &p.path {
                PathValue::Custom(p) => json!({"custom": p.display().to_string()}),
                PathValue::Detected(p) => json!({"detected": p.display().to_string()}),
                PathValue::NotFound => json!(NOT_FOUND),
            }),
        })
    }
}

impl SettingsPage for ExternalToolsPage {
    fn is_modified(&self, _: &App) -> bool {
        self.loaded
            .stored()
            .is_some_and(|stored| rows_of(stored) != self.rows)
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let (Some(root), Loaded::Read(stored)) = (self.root.clone(), &self.loaded) else {
            return Ok(());
        };
        if rows_of(stored) == self.rows {
            return Ok(());
        }
        let settings = Self::settings_for(stored, &self.rows);
        let paths: Vec<(Tool, Option<PathBuf>)> = self
            .rows
            .iter()
            .filter(|r| r.custom_path != stored.custom_path(r.tool))
            .map(|r| (r.tool, r.custom_path.clone()))
            .collect();
        let mut written = stored.clone();
        written.settings = settings.clone();
        for (tool, path) in &paths {
            if let Some(entry) = written.paths.iter_mut().find(|(t, _)| t == tool) {
                entry.1 = path.clone();
            }
        }
        self.loaded = Loaded::Read(written);
        spawn_write(
            root,
            move |root| {
                settings::save(root, &settings).map_err(|e| e.to_string())?;
                for (tool, path) in &paths {
                    settings::set_custom_path(root, *tool, path.as_deref())
                        .map_err(|e| e.to_string())?;
                }
                Ok(())
            },
            cx,
            |this, error, _| this.error = error,
        );
        cx.notify();
        Ok(())
    }

    fn reset(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        self.close_list(cx);
        if let Some(stored) = self.loaded.stored() {
            self.rows = rows_of(stored);
        }
        self.read(cx);
        cx.notify();
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, cx: &App) -> Json {
        json!({
            "project": self.root.as_ref().map(|r| r.display().to_string()),
            "loaded": self.loaded.stored().is_some(),
            "probed": !self.probes.is_empty(),
            "rows": self.rows.iter().map(|r| self.row_json(r)).collect::<Vec<_>>(),
            "uv-missing": self.uv_missing(),
            "list": self.list.as_ref().map(|(t, _, _)| t.id()),
            "menu": self.list.as_ref().map(|(_, m, _)| m.read(cx).state_json()),
            "error": self.error.clone().or_else(|| match &self.loaded {
                Loaded::Failed(e) => Some(e.clone()),
                _ => None,
            }),
        })
    }

    /// Fields: `enabled:<tool>` (true or false), `lookup:<tool>` (`sdk`,
    /// `path`, `uvx`), `path:<tool>` (a custom executable, or null for
    /// auto-detection), `list` (opens a tool's Lookup list; null closes
    /// it).
    fn edit(
        &mut self,
        field: &str,
        value: &Json,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let tool_of = |id: &str| Tool::parse(id).ok_or_else(|| format!("no tool `{id}`"));
        if field == "list" {
            match value.as_str() {
                Some(id) => {
                    let tool = tool_of(id)?;
                    self.row_mut(tool)?;
                    self.open_list(tool, window, cx);
                }
                None => self.close_list_in(window, cx),
            }
            return Ok(());
        }
        let (name, id) = field
            .split_once(':')
            .ok_or_else(|| format!("no field `{field}` on this page"))?;
        let tool = tool_of(id)?;
        match name {
            "enabled" => {
                let enabled = value.as_bool().ok_or("`enabled` takes true or false")?;
                self.row_mut(tool)?.enabled = enabled;
            }
            "lookup" => {
                let lookup = value
                    .as_str()
                    .and_then(Lookup::parse)
                    .ok_or("`lookup` takes `sdk`, `path` or `uvx`")?;
                self.row_mut(tool)?.lookup = lookup;
                self.close_list_in(window, cx);
            }
            "path" => {
                let path = match value {
                    Json::Null => None,
                    Json::String(s) if s.trim().is_empty() => None,
                    Json::String(s) => Some(PathBuf::from(s)),
                    _ => return Err("`path` takes a path or null".into()),
                };
                self.row_mut(tool)?;
                self.set_custom_path(tool, path, cx);
            }
            _ => return Err(format!("no field `{field}` on this page")),
        }
        cx.notify();
        Ok(())
    }
}

impl Render for ExternalToolsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = div()
            .id("external-tools-page")
            .debug_selector(|| "external-tools-page".into())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(ROW_GAP));
        if self.root.is_none() {
            return page.child(note(NO_PROJECT, cx));
        }
        if let Loaded::Failed(e) = &self.loaded {
            return page.child(note(e.clone(), cx));
        }
        let theme = cx.theme();
        let border = theme.color(UiRole::Border);
        let rows: Vec<AnyElement> = self
            .rows
            .clone()
            .iter()
            .enumerate()
            .map(|(ix, row)| self.render_row(ix, row, cx))
            .collect();
        let table = div()
            .flex_1()
            .min_h(px(HEADER_HEIGHT + ROW_HEIGHT * 3.0 + 2.0))
            .flex()
            .flex_col()
            .border_1()
            .border_color(border)
            .bg(cx.theme().color(UiRole::Surface))
            .child(self.header(cx))
            .children(rows);
        let uv = self.uv_missing().then(|| {
            div()
                .debug_selector(|| "external-tools:uv-missing".into())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(LABEL_GAP))
                .child(Icon::new(IconName::WARNING))
                .child(UV_MISSING)
        });
        let overlay = self.list.as_ref().map(|(tool, menu, _)| {
            let index = self.rows.iter().position(|r| r.tool == *tool).unwrap_or(0);
            let at = self
                .anchors
                .borrow()
                .get(index)
                .map(|b| b.bottom_left())
                .unwrap_or_default();
            ui::menu::overlay(menu, at, window)
        });
        let error = self
            .error
            .clone()
            .map(|e| div().text_color(cx.theme().color(UiRole::Danger)).child(e));
        page.child(div().child(wrapped(TOOLS_TEXT, cx)))
            .child(table)
            .children(uv)
            .children(error)
            .children(overlay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tools_in_the_reference_order_with_their_chains() {
        assert_eq!(table_tools(), [Tool::Basedpyright, Tool::Ruff, Tool::Ty]);
        assert_eq!(chain_text(Lookup::Sdk), "Sdk \u{b7} Path \u{b7} uvx");
        assert_eq!(chain_text(Lookup::Uvx), "uvx");
        let probe = Probe {
            custom_path: None,
            sdk: false,
            path: PathValue::Detected(PathBuf::from("/bin/ruff")),
            uvx: true,
        };
        assert_eq!(
            marked_chain_text(Lookup::Sdk, Some(&probe)),
            "Sdk \u{2717} \u{b7} Path \u{2713} \u{b7} uvx \u{2713}"
        );
        assert_eq!(marked_chain_text(Lookup::Path, None), "Path ? \u{b7} uvx ?");
    }

    #[test]
    fn features_and_explicit_switches() {
        let stored = Stored {
            settings: Settings::default(),
            paths: Tool::ALL.into_iter().map(|t| (t, None)).collect(),
        };
        assert_eq!(
            features(Tool::Ruff, &stored),
            [INSPECTIONS, FORMATTING, IMPORT_OPTIMIZER]
        );
        assert_eq!(features(Tool::Ty, &stored), [INSPECTIONS]);
        let mut rows = rows_of(&stored);
        assert_eq!(
            rows.iter().map(|r| r.enabled).collect::<Vec<_>>(),
            [false, true, true]
        );
        rows[0].enabled = true;
        rows[1].lookup = Lookup::Uvx;
        let settings = ExternalToolsPage::settings_for(&stored, &rows);
        assert!(settings.basedpyright.enabled && settings.basedpyright.explicit);
        assert_eq!(settings.ruff.lookup, Lookup::Uvx);
        assert!(!settings.ruff.explicit, "a lookup change is not a switch");
        assert!(!settings.ty.explicit);
    }

    #[test]
    fn probes_look_in_the_interpreter_path_and_for_uvx() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let tool = |name: &str| {
            let path = bin.join(name);
            std::fs::write(&path, "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        };
        let ruff = tool(if cfg!(windows) { "ruff.exe" } else { "ruff" });
        let search = SearchPaths {
            path: vec![bin.clone()],
            user_dirs: Vec::new(),
            extensions: if cfg!(windows) {
                vec![".exe".into()]
            } else {
                Vec::new()
            },
        };
        let custom = dir.path().join("my-ty");
        let probes = probe_tools(
            None,
            Some(&search),
            &[(Tool::Ruff, None), (Tool::Ty, Some(custom.clone()))],
        );
        let (_, ruff_probe) = &probes[0];
        assert_eq!(ruff_probe.path, PathValue::Detected(ruff));
        assert!(!ruff_probe.sdk && !ruff_probe.uvx);
        let (_, ty_probe) = &probes[1];
        assert_eq!(ty_probe.path, PathValue::Custom(custom));
    }

    /// Type Engine and External Tools over a project with ruff, ty and
    /// basedpyright on the search path and no uv: the pages read the
    /// settings, Apply writes them and the servers follow; driven through
    /// command ids only.
    #[cfg(unix)]
    #[gpui::test]
    fn the_python_pages_edit_the_project_settings(cx: &mut gpui::TestAppContext) {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;

        use crate::services::tests::{Recorder, provider};

        let dir = tempfile::tempdir().unwrap();
        let root = native_path::canonicalize(dir.path()).unwrap();
        let script = |path: &Path, body: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        script(&root.join(".venv/bin/python"), "exit 1");
        std::fs::write(
            root.join(".venv/pyvenv.cfg"),
            "home = /usr/bin\nversion = 3.13.1\n",
        )
        .unwrap();
        let tools = root.join("tools");
        script(&tools.join("ruff"), "echo 'ruff 0.15.20'");
        script(&tools.join("ty"), "echo 'ty 0.0.90'");
        script(&tools.join("basedpyright-langserver"), "exit 1");
        script(&tools.join("basedpyright"), "echo 'basedpyright 1.40.2'");
        let recorder = Arc::new(Recorder::default());
        let search = SearchPaths {
            path: vec![tools.clone()],
            user_dirs: Vec::new(),
            extensions: Vec::new(),
        };
        let mut product = crate::product();
        product.code_intel = Some(workbench::CodeIntelHook::new(provider(&recorder, search)));
        let product = Rc::new(product);
        let window = cx.update(|cx| {
            workbench::init(cx, &product);
            workbench::open_main_window(cx, product.clone())
        });
        let invoke = |cx: &mut gpui::TestAppContext, id: &str, args: Json| -> Json {
            let shown = args.to_string();
            let out = cx
                .update_window(window.into(), |_, window, cx| {
                    ui::command::invoke(id, args, window, cx)
                })
                .unwrap()
                .unwrap_or_else(|e| panic!("{id} {shown}: {e:?}"));
            cx.run_until_parked();
            for handle in cx.update(|cx| cx.windows()) {
                let _ = cx.update_window(handle, |_, window, cx| window.draw(cx).clear());
            }
            cx.run_until_parked();
            out
        };
        let edit = |cx: &mut gpui::TestAppContext, field: &str, value: Json| {
            invoke(cx, "settings.edit", json!({"field": field, "value": value}))
        };
        let fields = |cx: &mut gpui::TestAppContext| invoke(cx, "settings.state", Json::Null);
        invoke(
            cx,
            "project.open-folder",
            json!({"path": root.display().to_string()}),
        );

        // Type Engine: the project's engine and its description.
        invoke(cx, "settings.open", json!({"page": TYPE_ENGINE}));
        let s = fields(cx);
        assert_eq!(s["rows"][0]["id"], json!(PYTHON), "{s}");
        assert_eq!(s["breadcrumbs"], json!(["Python", "Type Engine"]), "{s}");
        let page = &s["fields"];
        assert_eq!(page["loaded"], true, "{page}");
        assert_eq!(page["engine"], "ty", "{page}");
        assert_eq!(page["engines"], json!(["ty", "Basedpyright"]), "{page}");
        assert_eq!(page["description"], TY_DESCRIPTION, "{page}");
        edit(cx, "engine", json!("basedpyright"));
        let s = fields(cx);
        assert_eq!(s["modified"], json!([TYPE_ENGINE]), "{s}");
        assert_eq!(s["fields"]["description"], BASEDPYRIGHT_DESCRIPTION);
        invoke(cx, "settings.apply", Json::Null);
        let text = std::fs::read_to_string(root.join(".oxide/project.toml")).unwrap();
        assert!(text.contains("name = \"basedpyright\""), "{text}");
        let state = invoke(cx, "python.tools.state", Json::Null);
        assert_eq!(state["settings"]["type_engine"], "basedpyright", "{state}");
        assert_eq!(recorder.last()[1].0, "basedpyright");

        // External Tools: the tools by name, the steps of their lookup,
        // where they were found, and the missing uv.
        invoke(cx, "settings.select", json!({"page": EXTERNAL_TOOLS}));
        let page = fields(cx)["fields"].clone();
        let rows = page["rows"].as_array().unwrap();
        let names: Vec<_> = rows.iter().map(|r| r["name"].clone()).collect();
        assert_eq!(names, [json!("Basedpyright"), json!("Ruff"), json!("ty")]);
        let enabled: Vec<_> = rows.iter().map(|r| r["enabled"].clone()).collect();
        assert_eq!(enabled, [json!(true), json!(true), json!(false)], "{page}");
        assert_eq!(
            rows[1]["features"],
            json!([INSPECTIONS, FORMATTING, IMPORT_OPTIMIZER])
        );
        assert_eq!(rows[2]["features"], json!([]));
        assert_eq!(
            rows[1]["chain"],
            "Sdk \u{2717} \u{b7} Path \u{2713} \u{b7} uvx \u{2717}"
        );
        assert_eq!(
            rows[1]["path"]["detected"],
            json!(tools.join("ruff").display().to_string())
        );
        assert_eq!(page["uv-missing"], true, "{page}");

        // The Lookup list offers the chains; choosing one closes it.
        edit(cx, "list", json!("ruff"));
        let page = fields(cx)["fields"].clone();
        assert_eq!(page["list"], "ruff");
        let items = &page["menu"]["levels"][0]["items"];
        assert_eq!(items[0]["label"], "Sdk \u{b7} Path \u{b7} uvx", "{page}");
        assert_eq!(items[2]["label"], "uvx", "{page}");
        edit(cx, "lookup:ruff", json!("path"));
        let page = fields(cx)["fields"].clone();
        assert_eq!(page["list"], Json::Null);
        assert_eq!(
            page["rows"][1]["chain"],
            "Path \u{2713} \u{b7} uvx \u{2717}"
        );

        // Ruff off, ty with an executable of its own: written on Apply.
        edit(cx, "enabled:ruff", json!(false));
        let custom = root.join("bin/ty");
        edit(cx, "path:ty", json!(custom.display().to_string()));
        let page = fields(cx)["fields"].clone();
        assert_eq!(
            page["rows"][2]["path"]["custom"],
            json!(custom.display().to_string()),
            "{page}"
        );
        assert_eq!(fields(cx)["modified"], json!([EXTERNAL_TOOLS]));
        invoke(cx, "settings.ok", Json::Null);
        let text = std::fs::read_to_string(root.join(".oxide/project.toml")).unwrap();
        assert!(text.contains("[python.tools.ruff]"), "{text}");
        assert!(text.contains("enabled = false"), "{text}");
        assert!(text.contains("lookup = \"path\""), "{text}");
        let personal = std::fs::read_to_string(root.join(".oxide/workspace.toml")).unwrap();
        assert!(personal.contains("[python.tools.ty]"), "{personal}");
        let state = invoke(cx, "python.tools.state", Json::Null);
        assert_eq!(state["settings"]["ruff"]["enabled"], false, "{state}");
        assert!(
            recorder.last().iter().all(|(id, _)| id != "ruff"),
            "{:?}",
            recorder.last()
        );
    }
}
