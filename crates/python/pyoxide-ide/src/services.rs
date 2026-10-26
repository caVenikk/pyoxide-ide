//! The Python language services of the product (ADR 0010, 0011): the code
//! intelligence provider of the workbench, the services of each open
//! project, and the `python.tools.*` commands.
//!
//! When a project opens, the provider hands the workbench the project's
//! LSP client at once and prepares its servers in the background
//! (`py_lsp::services::prepare`): the one-time imports of the reference's
//! settings, the settings, the lookup of each tool, then the server specs
//! for the client. The versions of the tools and ruff's rule names follow
//! while the servers start. A change of the interpreter or of the tool
//! settings prepares them again; the client restarts the servers whose
//! spec changed.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Weak};

use code_intel::{CodeIntel, Markup, ServerStatus};
use code_intel_ui::DocLocation;
use code_intel_ui::completion::Parentheses;
use code_intel_ui::file_doc::FileDoc;
use code_intel_ui::syntax::Lexer;
use gpui::{AnyWindowHandle, App, AppContext as _, Context, Entity, Global, Task};
use py_lsp::lookup::SearchPaths;
use py_lsp::ruff::{RuffCustomizer, load_rules};
use py_lsp::services::{PROBE_TIMEOUT, PrepareOptions, ProjectState, SpecTarget, prepare, probe};
use py_lsp::settings::{self, Tool, TypeEngine};
use serde_json::{Value as Json, json};
use ui::command::CommandError;
use ui::icons::{IconName, IconSvg};
use workbench::{CodeIntelProvider, Workbench};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Describe the Python tools of the project.
    ToolsState => "python.tools.state", "Python Tools State",
        "Returns the Python tools of the project as JSON: the settings, the interpreter, each tool's lookup steps, executable, server command, version and problem, and the tools waiting for an answer whether they may run through uv.";
    /// Answer whether a default tool may run through `uvx`.
    AllowUvx { tool: String, allow: bool } => "python.tools.allow-uvx", "Run Python Tool Through uv",
        "Answers whether a default tool that is installed nowhere (ruff, the type server) may run through uvx, which downloads it; the answer is kept in the personal project state and the servers are prepared again.",
        r#"{"tool": "ty", "allow": true}"#;
    /// Prepare the servers again.
    RefreshTools => "python.tools.refresh", "Refresh Python Tools",
        "Reads the Python tool settings and the project interpreter again; the servers whose configuration changed restart.";
}

/// The servers' icons: ruff's and basedpyright's from IntelliJ Community
/// (Apache-2.0, see NOTICE), a neutral placeholder of Pyoxide's for ty.
pub const RUFF_ICON: IconName = IconName("python/ruff");
pub const TY_ICON: IconName = IconName("python/ty");
pub const BASEDPYRIGHT_ICON: IconName = IconName("python/basedpyright");
/// The same in one colour, for the status bar (the reference colourizes a
/// server's icon there); ty's placeholder is one colour already.
pub const RUFF_STATUS_ICON: IconName = IconName("python/ruffStatus");
pub const BASEDPYRIGHT_STATUS_ICON: IconName = IconName("python/basedpyrightStatus");

const RUFF_SVG: &[u8] = include_bytes!("../assets/icons/ruff.svg");
const RUFF_DARK_SVG: &[u8] = include_bytes!("../assets/icons/ruff_dark.svg");
const BASEDPYRIGHT_SVG: &[u8] = include_bytes!("../assets/icons/basedpyright.svg");
const BASEDPYRIGHT_DARK_SVG: &[u8] = include_bytes!("../assets/icons/basedpyright_dark.svg");

/// The colour of one-colour icons (the platform icons' grey) in the light
/// and the dark theme.
const MONOCHROME: [&str; 2] = ["#6C707E", "#CED0D6"];

/// The one-colour copies of ruff's and basedpyright's icons, light and
/// dark.
static MONOCHROME_SVGS: LazyLock<[Vec<u8>; 4]> = LazyLock::new(|| {
    [
        (RUFF_SVG, MONOCHROME[0]),
        (RUFF_DARK_SVG, MONOCHROME[1]),
        (BASEDPYRIGHT_SVG, MONOCHROME[0]),
        (BASEDPYRIGHT_DARK_SVG, MONOCHROME[1]),
    ]
    .map(|(svg, colour)| monochrome(&String::from_utf8_lossy(svg), colour).into_bytes())
});

fn icons() -> [IconSvg; 5] {
    let m = &*MONOCHROME_SVGS;
    [
        IconSvg {
            name: RUFF_ICON.0,
            light: RUFF_SVG,
            dark: Some(RUFF_DARK_SVG),
        },
        IconSvg {
            name: BASEDPYRIGHT_ICON.0,
            light: BASEDPYRIGHT_SVG,
            dark: Some(BASEDPYRIGHT_DARK_SVG),
        },
        IconSvg {
            name: TY_ICON.0,
            light: include_bytes!("../assets/icons/typeServer.svg"),
            dark: Some(include_bytes!("../assets/icons/typeServer_dark.svg")),
        },
        IconSvg {
            name: RUFF_STATUS_ICON.0,
            light: &m[0],
            dark: Some(&m[1]),
        },
        IconSvg {
            name: BASEDPYRIGHT_STATUS_ICON.0,
            light: &m[2],
            dark: Some(&m[3]),
        },
    ]
}

/// Every `#rgb` and `#rrggbb` colour of an SVG replaced by `colour`.
fn monochrome(svg: &str, colour: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(at) = rest.find('#') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let hex = after.chars().take_while(char::is_ascii_hexdigit).count();
        if hex == 3 || hex == 6 {
            out.push_str(colour);
            rest = &after[hex..];
        } else {
            out.push('#');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// The icon of a server by its icon key (the tool id).
pub fn server_icon(key: &str) -> Option<IconName> {
    match Tool::parse(key)? {
        Tool::Ruff => Some(RUFF_ICON),
        Tool::Ty => Some(TY_ICON),
        Tool::Basedpyright => Some(BASEDPYRIGHT_ICON),
    }
}

/// The one-colour icon of a server for the status bar.
pub fn server_status_bar_icon(key: &str) -> Option<IconName> {
    match Tool::parse(key)? {
        Tool::Ruff => Some(RUFF_STATUS_ICON),
        Tool::Ty => Some(TY_ICON),
        Tool::Basedpyright => Some(BASEDPYRIGHT_STATUS_ICON),
    }
}

/// Makes the LSP client of a project: what the workbench talks to, and
/// where its server specs go (alive as long as the client).
pub type ClientFactory = Rc<dyn Fn() -> (Arc<dyn CodeIntel>, Weak<dyn SpecTarget>)>;

/// The product's code intelligence provider.
pub struct PythonCodeIntel {
    factory: ClientFactory,
    search: Option<Arc<SearchPaths>>,
    /// The roots of the projects started, for the modules of their files.
    roots: RefCell<Vec<PathBuf>>,
}

impl Default for PythonCodeIntel {
    fn default() -> Self {
        Self::new()
    }
}

impl PythonCodeIntel {
    /// The services over `lsp-client`, with tools searched in the
    /// application's environment.
    pub fn new() -> Self {
        Self {
            factory: Rc::new(|| {
                let client = lsp_client::LspCodeIntel::new(Vec::new());
                let target: Arc<dyn SpecTarget> = client.clone();
                let target = Arc::downgrade(&target);
                (client as Arc<dyn CodeIntel>, target)
            }),
            search: None,
            roots: RefCell::default(),
        }
    }

    /// The services over `lsp-client`, with tools searched in `search`
    /// only (besides the interpreter's folder): tests give no folders, so
    /// the tools of the machine never start.
    pub fn with_search(search: SearchPaths) -> Self {
        Self {
            search: Some(Arc::new(search)),
            ..Self::new()
        }
    }

    /// The services over another client, with tools searched in `search`
    /// when given.
    pub fn with_client(factory: ClientFactory, search: Option<SearchPaths>) -> Self {
        Self {
            factory,
            search: search.map(Arc::new),
            roots: RefCell::default(),
        }
    }
}

impl CodeIntelProvider for PythonCodeIntel {
    fn start(&self, root: &Path, cx: &mut App) -> Arc<dyn CodeIntel> {
        let (client, target) = (self.factory)();
        let mut roots = self.roots.borrow_mut();
        if !roots.iter().any(|r| r == root) {
            roots.push(root.to_owned());
        }
        drop(roots);
        let search = self.search.clone();
        let services = cx.new(|cx| PythonServices::new(root.to_owned(), target, search, cx));
        let registry = cx.default_global::<Registry>();
        registry.0.retain(|e| e.client.strong_count() > 0);
        registry.0.push(Entry {
            root: root.to_owned(),
            client: Arc::downgrade(&client),
            services,
        });
        client
    }

    fn server_icon(&self, server: &ServerStatus) -> Option<IconName> {
        server_icon(server.icon.as_deref()?)
    }

    fn server_status_bar_icon(&self, server: &ServerStatus) -> Option<IconName> {
        server_status_bar_icon(server.icon.as_deref()?)
    }

    /// The tool's name (`Ruff`, while its server calls itself `ruff`).
    fn server_title(&self, server: &ServerStatus) -> String {
        server
            .icon
            .as_deref()
            .and_then(Tool::parse)
            .map_or_else(|| server.name.clone(), |tool| tool.name().to_owned())
    }

    fn settings_file(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project::store::settings_file(project_root))
    }

    /// Python functions and methods are called with parentheses; `self`
    /// and `cls` are not passed (the reference's `PyFunctionInsertHandler`).
    fn parentheses(&self, language: &str) -> Option<Parentheses> {
        (language == py_lsp::servers::PYTHON).then_some(Parentheses {
            implicit_first: &["self", "cls"],
        })
    }

    /// Code in documentation is coloured as in the editor.
    fn lexer(&self, language: &str) -> Option<Arc<dyn Lexer>> {
        (language == py_lsp::servers::PYTHON).then(|| Arc::new(py_syntax::PythonLexer) as _)
    }

    /// The module a Python file is (`json` for `json/__init__.pyi`), with
    /// the module icon, as the reference heads the documentation of a
    /// symbol; other files as the platform does.
    fn doc_location(&self, path: &Path) -> Option<DocLocation> {
        let name = module_name(path, &self.roots.borrow());
        match name {
            Some(name) => Some(DocLocation {
                name,
                icon: IconName::MODULE,
            }),
            None => code_intel_ui::doc::default_location(path),
        }
    }

    /// A hover about a module: the reference's documentation of a module
    /// (`Module contextlib`, the file's facts, the docs.python.org link of
    /// a standard library module for the project interpreter's version).
    fn file_doc(
        &self,
        hover: &Markup,
        declaration: &Path,
        project_root: Option<&Path>,
        cx: &mut App,
    ) -> Option<Task<Option<FileDoc>>> {
        let hovered = crate::module_doc::hovered_module(hover)?;
        let roots = self.roots.borrow().clone();
        let interpreter = project_root.map(|root| crate::interpreter::for_project(root, cx));
        let path = declaration.to_owned();
        Some(cx.background_spawn(async move {
            let version = match interpreter {
                Some(task) => task.await.map(|selected| Some(selected.version)),
                None => None,
            };
            let name = crate::module_doc::module_name(&path, &roots, hovered);
            Some(crate::module_doc::module_doc(&path, name, version))
        }))
    }
}

/// The dotted module a Python file is: `json` for `.../json/__init__.pyi`,
/// `json.decoder` for `.../json/decoder.pyi`, `pkg.mod` for
/// `<project>/src/pkg/mod.py`. A file of a project (`roots`) is named from
/// the deepest of its source roots and the project root; any other file
/// (the standard library, site-packages, stubs) from the packages around
/// it (folders with an `__init__.py` or `__init__.pyi`). `None` for a file
/// that is not Python.
pub fn module_name(path: &Path, roots: &[PathBuf]) -> Option<String> {
    let extension = path.extension()?.to_str()?;
    if !["py", "pyi", "pyw"].contains(&extension) {
        return None;
    }
    let in_project = roots
        .iter()
        .filter(|root| path.starts_with(root))
        .flat_map(|root| {
            // The roots in effect of the open project; the detected ones
            // before the project tool window has read them.
            let mut bases = match project::roots::published(root) {
                Some(roots) => {
                    roots.folders(root, &[project::roots::SOURCES, project::roots::TESTS])
                }
                None => crate::source_roots(root),
            };
            bases.push(root.clone());
            bases
        })
        .filter(|base| path.starts_with(base))
        .max_by_key(|base| base.components().count())
        .and_then(|base| dotted(path.strip_prefix(&base).ok()?));
    if in_project.is_some() {
        return in_project;
    }
    let is_package =
        |dir: &Path| dir.join("__init__.py").is_file() || dir.join("__init__.pyi").is_file();
    let stem = path.file_stem()?.to_str()?;
    let mut parts = Vec::new();
    let mut dir = path.parent();
    if stem == "__init__" {
        let package = dir?;
        parts.push(package.file_name()?.to_str()?.to_owned());
        dir = package.parent();
    } else {
        parts.push(stem.to_owned());
    }
    while let Some(package) = dir.filter(|d| is_package(d)) {
        parts.push(package.file_name()?.to_str()?.to_owned());
        dir = package.parent();
    }
    parts.reverse();
    Some(parts.join("."))
}

/// `pkg/mod.py` as `pkg.mod` (`pkg/__init__.py` as `pkg`); `None` when a
/// part is not a Python name (`.venv`, `site-packages`).
fn dotted(relative: &Path) -> Option<String> {
    let mut parts: Vec<String> = relative
        .with_extension("")
        .components()
        .map(|c| c.as_os_str().to_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    if parts.last().is_some_and(|p| p == "__init__") {
        parts.pop();
    }
    let identifier = |p: &String| {
        p.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && p.chars().all(|c| c.is_alphanumeric() || c == '_')
    };
    (!parts.is_empty() && parts.iter().all(identifier)).then(|| parts.join("."))
}

// ---- the services of a project ---------------------------------------------------

/// The Python services of one project in one main window.
pub struct PythonServices {
    root: PathBuf,
    target: Weak<dyn SpecTarget>,
    ruff: Arc<RuffCustomizer>,
    search: Option<Arc<SearchPaths>>,
    /// The last preparation; `None` until the first one is done.
    state: Option<ProjectState>,
    /// How often specs were handed to the client.
    preparations: usize,
    /// The one-time imports ran.
    imported: bool,
    /// A setting could not be written.
    error: Option<String>,
    generation: u64,
    work: Task<()>,
    change: Task<()>,
}

impl PythonServices {
    fn new(
        root: PathBuf,
        target: Weak<dyn SpecTarget>,
        search: Option<Arc<SearchPaths>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            ruff: Arc::new(RuffCustomizer::new(&root)),
            root,
            target,
            search,
            state: None,
            preparations: 0,
            imported: false,
            error: None,
            generation: 0,
            work: Task::ready(()),
            change: Task::ready(()),
        };
        this.refresh(cx);
        this
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The last preparation, once there is one.
    pub fn state(&self) -> Option<&ProjectState> {
        self.state.as_ref()
    }

    /// Where its tools are searched besides the interpreter's folder;
    /// `None`: the application's environment.
    pub fn search(&self) -> Option<Arc<SearchPaths>> {
        self.search.clone()
    }

    /// The type engine in effect.
    pub fn type_engine(&self) -> Option<TypeEngine> {
        Some(self.state.as_ref()?.settings.type_engine)
    }

    /// Reads the settings and the interpreter again, looks the tools up
    /// and hands the specs to the client; then reads the versions and
    /// ruff's rule names. A newer call replaces one in progress.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let import = !self.imported;
        self.imported = true;
        let root = self.root.clone();
        let search = self.search.clone();
        let target = self.target.clone();
        let ruff = self.ruff.clone();
        let interpreter = crate::interpreter::for_project(&root, cx);
        self.work = cx.spawn(async move |this, cx| {
            let interpreter = interpreter.await.map(|s| s.python);
            let state = cx
                .background_spawn({
                    let root = root.clone();
                    let search = search.clone();
                    async move {
                        let options = PrepareOptions {
                            import,
                            search: search.as_deref(),
                        };
                        prepare(&root, interpreter.as_deref(), options)
                    }
                })
                .await;
            for note in state.notes.iter().chain(&state.errors) {
                eprintln!("python services: {note}");
            }
            if let Some(target) = target.upgrade() {
                target.set_specs(state.specs(Some(&ruff)));
            }
            let mut probed = state.clone();
            let current = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return false;
                }
                this.preparations += 1;
                this.state = Some(state);
                cx.notify();
                true
            });
            if !matches!(current, Ok(true)) {
                return;
            }
            let rules = cx
                .background_spawn(async move {
                    probe(&mut probed, PROBE_TIMEOUT);
                    let rules = probed
                        .tool(Tool::Ruff)
                        .executable()
                        .map(|ruff| load_rules(ruff, PROBE_TIMEOUT))
                        .unwrap_or_default();
                    (probed, rules)
                })
                .await;
            let (probed, rules) = rules;
            if !rules.is_empty() {
                ruff.set_rules(rules);
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.state = Some(probed);
                    cx.notify();
                }
            });
        });
    }

    /// Makes `engine` the project's type engine (`[python.type-engine]`)
    /// and prepares the servers of every window of the project again.
    pub fn select_type_engine(&mut self, engine: TypeEngine, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.change_settings(cx, move || {
            settings::select_type_engine(&root, engine).map(|_| ())
        });
    }

    /// Keeps the answer whether `tool` may run through `uvx` and prepares
    /// the servers again.
    pub fn allow_uvx(&mut self, tool: Tool, allow: bool, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.change_settings(cx, move || {
            settings::set_uvx_consent(&root, tool, Some(allow)).map(|_| ())
        });
    }

    fn change_settings(
        &mut self,
        cx: &mut Context<Self>,
        write: impl FnOnce() -> Result<(), project::StoreError> + Send + 'static,
    ) {
        let root = self.root.clone();
        let write = cx.background_spawn(async move { write() });
        self.change = cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |this, cx| {
                this.error = result.err().map(|e| e.to_string());
                cx.notify();
            });
            let _ = cx.update(|cx| refresh_root(&root, cx));
        });
    }

    /// The state reported by `python.tools.state`.
    pub fn state_json(&self) -> Json {
        let mut out = match &self.state {
            Some(state) => serde_json::to_value(state).unwrap_or(Json::Null),
            None => json!({ "root": self.root.display().to_string() }),
        };
        let questions: Vec<Json> = self
            .state
            .as_ref()
            .map(|s| s.uvx_questions())
            .unwrap_or_default()
            .into_iter()
            .map(|tool| {
                json!({
                    "tool": tool.id(),
                    "message": py_lsp::services::Problem::NeedsUvxConsent.message(tool),
                })
            })
            .collect();
        if let Json::Object(map) = &mut out {
            map.insert("ready".into(), json!(self.state.is_some()));
            map.insert("preparations".into(), json!(self.preparations));
            map.insert("uvx-questions".into(), Json::Array(questions));
            map.insert("ruff-rule-names".into(), json!(self.ruff.has_rules()));
            map.insert("error".into(), json!(self.error));
        }
        out
    }
}

// ---- registry ---------------------------------------------------------------------

/// The services of the open projects, with their clients.
#[derive(Default)]
struct Registry(Vec<Entry>);

struct Entry {
    root: PathBuf,
    /// The client the workbench holds; the entry goes with it.
    client: Weak<dyn CodeIntel>,
    services: Entity<PythonServices>,
}

impl Global for Registry {}

/// The services of the project at `root` (one per main window showing
/// it).
pub fn services_for(root: &Path, cx: &App) -> Vec<Entity<PythonServices>> {
    cx.try_global::<Registry>()
        .map(|r| {
            r.0.iter()
                .filter(|e| e.root == root && e.client.strong_count() > 0)
                .map(|e| e.services.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Prepares the servers of the project at `root` again, in every window.
pub fn refresh_root(root: &Path, cx: &mut App) {
    for services in services_for(root, cx) {
        services.update(cx, |s, cx| s.refresh(cx));
    }
}

/// The project of the active main window, else of any main window.
fn active_root(cx: &mut App) -> Option<PathBuf> {
    let windows: Vec<AnyWindowHandle> =
        cx.active_window().into_iter().chain(cx.windows()).collect();
    windows.into_iter().find_map(|handle| {
        handle
            .update(cx, |_, window, cx| {
                window
                    .root::<Workbench>()
                    .flatten()
                    .and_then(|w| w.read(cx).project_root(cx))
            })
            .ok()
            .flatten()
    })
}

/// The services of the active main window's project.
fn active_services(cx: &mut App) -> Option<Entity<PythonServices>> {
    let root = active_root(cx)?;
    services_for(&root, cx).into_iter().next()
}

/// Registers the icons and commands, and prepares the servers again when
/// a project's interpreter changes (the product's `init`).
pub fn init(cx: &mut App) {
    ui::icons::register(cx, &icons());
    ui::command::register(cx, COMMANDS);
    crate::interpreter::on_changed(cx, refresh_root);
    cx.on_action(|action: &AllowUvx, cx| {
        let Some(tool) = Tool::parse(&action.tool) else {
            return;
        };
        let allow = action.allow;
        // The window may be the one dispatching: look at it after.
        cx.defer(move |cx| {
            if let Some(services) = active_services(cx) {
                services.update(cx, |s, cx| s.allow_uvx(tool, allow, cx));
            }
        });
    });
    cx.on_action(|_: &RefreshTools, cx| {
        cx.defer(|cx| {
            if let Some(root) = active_root(cx) {
                refresh_root(&root, cx);
            }
        });
    });
    ui::command::register_query(
        cx,
        "python.tools.state",
        Rc::new(|_, window, cx| {
            let root = window
                .root::<Workbench>()
                .flatten()
                .and_then(|w| w.read(cx).project_root(cx))
                .ok_or_else(|| CommandError::new("not_available", "no project"))?;
            let services = services_for(&root, cx)
                .into_iter()
                .next()
                .ok_or_else(|| CommandError::new("not_available", "no Python services"))?;
            Ok(services.read(cx).state_json())
        }),
    );
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use code_intel::fake::FakeCodeIntel;
    use gpui::TestAppContext;
    use lsp_client::ServerSpec;

    use super::*;

    /// Records the specs it gets.
    #[derive(Default)]
    pub(crate) struct Recorder(pub Mutex<Vec<Vec<ServerSpec>>>);

    impl SpecTarget for Recorder {
        fn set_specs(&self, specs: Vec<ServerSpec>) {
            self.0.lock().unwrap().push(specs);
        }
    }

    impl Recorder {
        /// The ids and command lines of the last specs.
        pub(crate) fn last(&self) -> Vec<(String, String)> {
            self.0
                .lock()
                .unwrap()
                .last()
                .map(|specs| {
                    specs
                        .iter()
                        .map(|s| (s.id.to_string(), s.command.display()))
                        .collect()
                })
                .unwrap_or_default()
        }

        pub(crate) fn count(&self) -> usize {
            self.0.lock().unwrap().len()
        }
    }

    /// A provider over a fake client whose specs `recorder` gets.
    pub(crate) fn provider(recorder: &Arc<Recorder>, search: SearchPaths) -> PythonCodeIntel {
        let recorder = recorder.clone();
        PythonCodeIntel::with_client(
            Rc::new(move || {
                let target: Arc<dyn SpecTarget> = recorder.clone();
                (
                    FakeCodeIntel::new() as Arc<dyn CodeIntel>,
                    Arc::downgrade(&target),
                )
            }),
            Some(search),
        )
    }

    #[test]
    fn server_icons() {
        brand::install();
        assert_eq!(server_icon("ruff"), Some(RUFF_ICON));
        assert_eq!(server_icon("basedpyright"), Some(BASEDPYRIGHT_ICON));
        assert_eq!(server_icon("ty"), Some(TY_ICON));
        assert_eq!(server_icon("pyrefly"), None);
        assert_eq!(server_status_bar_icon("ruff"), Some(RUFF_STATUS_ICON));
        assert_eq!(server_status_bar_icon("ty"), Some(TY_ICON));
        assert_eq!(
            monochrome(
                r##"<rect fill="#FFAF0F"/><path stroke="#fff" clip-path="url(#a)"/>"##,
                "#6C707E"
            ),
            r##"<rect fill="#6C707E"/><path stroke="#6C707E" clip-path="url(#a)"/>"##
        );
        let provider = PythonCodeIntel::with_search(SearchPaths::default());
        let mut ruff = ServerStatus::new(code_intel::ServerId::new("ruff"), "ruff");
        assert_eq!(provider.server_title(&ruff), "ruff");
        ruff.icon = Some("ruff".into());
        assert_eq!(provider.server_title(&ruff), "Ruff");
        assert_eq!(
            provider.parentheses("python").map(|p| p.implicit_first),
            Some(&["self", "cls"][..])
        );
        assert_eq!(provider.parentheses("toml"), None);
        assert_eq!(
            provider.settings_file(Path::new("/p")),
            Some(Path::new("/p").join(".oxide").join("project.toml"))
        );
        for icon in icons() {
            assert!(
                ui::icons::rasterize(icon.light, 32).is_some(),
                "{}",
                icon.name
            );
            assert!(
                ui::icons::rasterize(icon.dark.unwrap(), 32).is_some(),
                "{}",
                icon.name
            );
        }
    }

    #[test]
    fn modules_of_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = |path: PathBuf| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
            path
        };
        // Stubs and the standard library: named from the packages around.
        let stdlib = dir.path().join("typeshed").join("stdlib");
        let json = file(stdlib.join("json").join("__init__.pyi"));
        let decoder = file(stdlib.join("json").join("decoder.pyi"));
        let os_path = file(stdlib.join("os").join("path.pyi"));
        file(stdlib.join("os").join("__init__.pyi"));
        let abc = file(stdlib.join("abc.pyi"));
        assert_eq!(module_name(&json, &[]).as_deref(), Some("json"));
        assert_eq!(module_name(&decoder, &[]).as_deref(), Some("json.decoder"));
        assert_eq!(module_name(&os_path, &[]).as_deref(), Some("os.path"));
        assert_eq!(module_name(&abc, &[]).as_deref(), Some("abc"));
        assert_eq!(module_name(&stdlib.join("notes.txt"), &[]), None);

        // A project: from its source roots, packages with or without
        // `__init__.py`; its environment as any other.
        let project = dir.path().join("project");
        let module = file(project.join("src").join("pkg").join("mod.py"));
        let app = file(project.join("app.py"));
        let init = file(project.join("tools").join("__init__.py"));
        let site = project
            .join(".venv")
            .join("lib")
            .join("python3.13")
            .join("site-packages");
        file(site.join("requests").join("__init__.py"));
        let api = file(site.join("requests").join("api.py"));
        let roots = [project.clone()];
        assert_eq!(module_name(&module, &roots).as_deref(), Some("pkg.mod"));
        assert_eq!(module_name(&app, &roots).as_deref(), Some("app"));
        assert_eq!(module_name(&init, &roots).as_deref(), Some("tools"));
        assert_eq!(module_name(&api, &roots).as_deref(), Some("requests.api"));
        assert_eq!(module_name(&module, &[]).as_deref(), Some("mod"));
        // The roots the project tool window published win over the
        // detected ones: here `src/pkg` is a sources root, `src` none.
        let mut marked = project::Roots::default();
        marked.mark("src/pkg", project::roots::SOURCES);
        project::roots::publish(&project, &marked);
        assert_eq!(module_name(&module, &roots).as_deref(), Some("mod"));
        project::roots::publish(&project, &project::Roots::default());
        assert_eq!(module_name(&module, &roots).as_deref(), Some("src.pkg.mod"));

        let provider = PythonCodeIntel::with_search(SearchPaths::default());
        assert_eq!(
            provider.doc_location(&json),
            Some(DocLocation {
                name: "json".into(),
                icon: IconName::MODULE
            })
        );
        assert_eq!(
            provider
                .doc_location(Path::new("/p/notes.txt"))
                .map(|l| l.name),
            Some("notes".into())
        );
    }

    #[test]
    fn python_code_in_documentation_is_coloured() {
        let provider = PythonCodeIntel::with_search(SearchPaths::default());
        assert!(provider.lexer("toml").is_none());
        let lexer = provider.lexer("python").unwrap();
        let spans = code_intel_ui::doc::lex_lines(
            lexer.as_ref(),
            "def dumps(obj: Any,\n          **kwds: Any) -> str",
        );
        assert!(spans.iter().any(|(r, _)| *r == (0..3)), "{spans:?}");
        assert!(spans.iter().any(|(r, _)| r.start > 20), "{spans:?}");
    }

    #[gpui::test]
    fn services_live_as_long_as_their_client(cx: &mut TestAppContext) {
        let recorder = Arc::new(Recorder::default());
        let dir = tempfile::tempdir().unwrap();
        let provider = provider(&recorder, SearchPaths::default());
        let client = cx.update(|cx| provider.start(dir.path(), cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let services = services_for(dir.path(), cx);
            assert_eq!(services.len(), 1);
            let json = services[0].read(cx).state_json();
            assert_eq!(json["ready"], true);
            assert_eq!(json["tools"][0]["problem"]["kind"], "not-found");
        });
        // Nothing found: no servers.
        assert_eq!(recorder.count(), 1);
        assert!(recorder.last().is_empty());
        drop(client);
        cx.update(|cx| assert!(services_for(dir.path(), cx).is_empty()));
    }

    /// A project with two environments, fake ruff, ty and basedpyright on
    /// the search path, a workbench window of the product over a recording
    /// client; driven through command ids only.
    #[cfg(unix)]
    #[gpui::test]
    fn the_product_prepares_and_follows_its_servers(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = native_path::canonicalize(dir.path()).unwrap();
        let script = |path: &Path, body: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        for env in [".venv", "venv"] {
            script(&root.join(env).join("bin").join("python"), "exit 1");
            std::fs::write(
                root.join(env).join("pyvenv.cfg"),
                "home = /usr/bin\nversion = 3.13.1\n",
            )
            .unwrap();
        }
        std::fs::write(root.join("app.py"), "import os\n").unwrap();
        let tools = root.join("tools");
        script(
            &tools.join("ruff"),
            "if [ \"$1\" = rule ]; then echo '[{\"code\": \"F401\", \"name\": \"unused-import\"}]'; else echo 'ruff 0.15.20'; fi",
        );
        script(&tools.join("ty"), "echo 'ty 0.0.90 (abc 2026-10-12)'");
        script(&tools.join("basedpyright-langserver"), "exit 1");
        script(&tools.join("basedpyright"), "echo 'basedpyright 1.40.2'");

        let recorder = Arc::new(Recorder::default());
        let search = SearchPaths {
            path: vec![tools.clone()],
            user_dirs: Vec::new(),
            extensions: Vec::new(),
        };
        let mut product = crate::product();
        product.code_intel = vec![workbench::CodeIntelHook::new(provider(&recorder, search))];
        let product = Rc::new(product);
        let window = cx.update(|cx| {
            workbench::init(cx, &product);
            workbench::open_main_window(cx, product.clone())
        });
        let invoke = |cx: &mut TestAppContext, id: &str, args: Json| -> Json {
            let out = cx
                .update_window(window.into(), |_, window, cx| {
                    ui::command::invoke(id, args, window, cx)
                })
                .unwrap()
                .unwrap_or_else(|e| panic!("{id}: {e:?}"));
            cx.run_until_parked();
            out
        };
        invoke(
            cx,
            "project.open-folder",
            json!({"path": root.display().to_string()}),
        );

        // ruff and ty found on the path; ty gets the project's environment.
        let state = invoke(cx, "python.tools.state", Json::Null);
        assert_eq!(state["ready"], true);
        assert_eq!(state["interpreter"], json!(root.join(".venv/bin/python")));
        assert_eq!(state["tools"][0]["lookup"], "Sdk ✗ · Path ✓ · uvx ✗");
        assert_eq!(state["tools"][0]["version"], "0.15.20");
        assert_eq!(state["tools"][1]["version"], "0.0.90");
        assert_eq!(state["tools"][1]["problem"], Json::Null);
        assert_eq!(state["tools"][2]["runs"], false);
        assert_eq!(state["ruff-rule-names"], true);
        assert_eq!(
            recorder.last(),
            [
                (
                    "ruff".into(),
                    format!("{} server", tools.join("ruff").display())
                ),
                (
                    "ty".into(),
                    format!("{} server", tools.join("ty").display())
                ),
            ]
        );
        let ty_python = |recorder: &Recorder| {
            let specs = recorder.0.lock().unwrap();
            specs.last().unwrap()[1].settings.clone()
        };
        assert_eq!(
            ty_python(&recorder)["ty"]["configuration"]["environment"]["python"],
            json!(root.join(".venv/bin/python"))
        );

        // The widget shows for Python files.
        let widget = invoke(cx, "python.type-engine.state", Json::Null);
        assert_eq!(widget["visible"], false);
        invoke(cx, "project.open-file", json!({"path": "app.py"}));
        let widget = invoke(cx, "python.type-engine.state", Json::Null);
        assert_eq!(widget["visible"], true);
        assert_eq!(widget["text"], "Types: ty");
        assert_eq!(widget["tooltip"], "Selected type provider for python: ty");
        invoke(cx, "python.type-engine.choose", Json::Null);
        let widget = invoke(cx, "python.type-engine.state", Json::Null);
        let rows = &widget["popup"]["levels"][0]["items"];
        assert_eq!(rows[0]["label"], "ty");
        assert_eq!(rows[1]["label"], "Basedpyright");
        assert_eq!(rows[2]["separator"], true);
        assert_eq!(rows[3]["label"], "Type Engine Settings\u{2026}");
        assert_eq!(rows[3]["command"], "python.type-engine.settings");

        // Choosing basedpyright: kept in the settings, its server replaces ty's.
        invoke(
            cx,
            "python.type-engine.select",
            json!({"name": "basedpyright"}),
        );
        let widget = invoke(cx, "python.type-engine.state", Json::Null);
        assert_eq!(widget["text"], "Types: Basedpyright");
        let text = std::fs::read_to_string(root.join(".oxide/project.toml")).unwrap();
        assert!(text.contains("name = \"basedpyright\""), "{text}");
        assert_eq!(
            recorder.last()[1],
            (
                "basedpyright".into(),
                format!(
                    "{} --stdio",
                    tools.join("basedpyright-langserver").display()
                )
            )
        );
        let state = invoke(cx, "python.tools.state", Json::Null);
        assert_eq!(state["tools"][2]["version"], "1.40.2");

        // Another interpreter: the type server gets it.
        let before = recorder.count();
        invoke(cx, "python.interpreter.select", json!({"path": "venv"}));
        assert!(recorder.count() > before);
        assert_eq!(
            ty_python(&recorder)["python"]["pythonPath"],
            json!(root.join("venv/bin/python"))
        );

        // Type Engine Settings opens Settings > Python > Type Engine.
        invoke(cx, "python.type-engine.settings", Json::Null);
        let settings = invoke(cx, "settings.state", Json::Null);
        assert_eq!(settings["page"], "python.type-engine", "{settings}");
    }

    /// A default tool installed nowhere: the person is asked, the answer is
    /// kept and the server runs through uvx.
    #[cfg(unix)]
    #[gpui::test]
    fn default_tools_run_through_uvx_once_allowed(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = native_path::canonicalize(dir.path()).unwrap();
        let tools = root.join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        let uvx = tools.join("uvx");
        std::fs::write(&uvx, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&uvx, std::fs::Permissions::from_mode(0o755)).unwrap();
        let recorder = Arc::new(Recorder::default());
        let search = SearchPaths {
            path: vec![tools.clone()],
            user_dirs: Vec::new(),
            extensions: Vec::new(),
        };
        let mut product = crate::product();
        product.code_intel = vec![workbench::CodeIntelHook::new(provider(&recorder, search))];
        let product = Rc::new(product);
        let window = cx.update(|cx| {
            workbench::init(cx, &product);
            workbench::open_main_window(cx, product.clone())
        });
        let invoke = |cx: &mut TestAppContext, id: &str, args: Json| -> Json {
            let out = cx
                .update_window(window.into(), |_, window, cx| {
                    ui::command::invoke(id, args, window, cx)
                })
                .unwrap()
                .unwrap_or_else(|e| panic!("{id}: {e:?}"));
            cx.run_until_parked();
            out
        };
        invoke(
            cx,
            "project.open-folder",
            json!({"path": root.display().to_string()}),
        );
        let state = invoke(cx, "python.tools.state", Json::Null);
        let questions: Vec<&str> = state["uvx-questions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| q["tool"].as_str().unwrap())
            .collect();
        assert_eq!(questions, ["ruff", "ty"]);
        assert_eq!(
            state["uvx-questions"][1]["message"],
            "ty is not installed. Run it through uv (uvx ty)?"
        );
        assert!(recorder.last().is_empty());

        // The Types widget is marked and asks in its popup.
        std::fs::write(root.join("app.py"), "x = 1\n").unwrap();
        invoke(cx, "project.open-file", json!({"path": "app.py"}));
        let widget = invoke(cx, "python.type-engine.state", Json::Null);
        assert_eq!(widget["warning"], true);
        assert_eq!(widget["uvx-questions"], json!(["ruff", "ty"]));
        assert!(
            widget["tooltip"]
                .as_str()
                .unwrap()
                .ends_with("ty is not installed. Run it through uv (uvx ty)?"),
            "{widget}"
        );
        invoke(cx, "python.type-engine.choose", Json::Null);
        let widget = invoke(cx, "python.type-engine.state", Json::Null);
        let rows = &widget["popup"]["levels"][0]["items"];
        assert_eq!(rows[0]["label"], "Run Ruff through uv (downloads it)");
        assert_eq!(rows[0]["submenu"], true);
        assert_eq!(rows[1]["label"], "Run ty through uv (downloads it)");
        assert_eq!(rows[2]["separator"], true);
        assert_eq!(rows[3]["label"], "ty");

        invoke(
            cx,
            "python.tools.allow-uvx",
            json!({"tool": "ty", "allow": true}),
        );
        invoke(
            cx,
            "python.tools.allow-uvx",
            json!({"tool": "ruff", "allow": false}),
        );
        let state = invoke(cx, "python.tools.state", Json::Null);
        assert_eq!(state["tools"][0]["problem"]["kind"], "uvx-declined");
        assert_eq!(state["uvx-questions"], json!([]));
        assert_eq!(
            recorder.last(),
            [("ty".into(), format!("{} ty server", uvx.display()))]
        );
        let personal = std::fs::read_to_string(root.join(".oxide/workspace.toml")).unwrap();
        assert!(personal.contains("uvx = \"allow\""), "{personal}");
        assert!(personal.contains("uvx = \"deny\""), "{personal}");
        // ruff still does not run: the mark stays, the question is gone.
        let widget = invoke(cx, "python.type-engine.state", Json::Null);
        assert_eq!(widget["uvx-questions"], json!([]));
        assert_eq!(widget["warning"], true);
    }
}
