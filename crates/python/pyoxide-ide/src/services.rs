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

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Weak};

use code_intel::{CodeIntel, ServerStatus};
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
/// The check mark of the current row of a popup (Pyoxide's own).
pub const CHECKED_ICON: IconName = IconName("python/checked");

const ICONS: &[IconSvg] = &[
    IconSvg {
        name: RUFF_ICON.0,
        light: include_bytes!("../assets/icons/ruff.svg"),
        dark: Some(include_bytes!("../assets/icons/ruff_dark.svg")),
    },
    IconSvg {
        name: BASEDPYRIGHT_ICON.0,
        light: include_bytes!("../assets/icons/basedpyright.svg"),
        dark: Some(include_bytes!("../assets/icons/basedpyright_dark.svg")),
    },
    IconSvg {
        name: TY_ICON.0,
        light: include_bytes!("../assets/icons/typeServer.svg"),
        dark: Some(include_bytes!("../assets/icons/typeServer_dark.svg")),
    },
    IconSvg {
        name: CHECKED_ICON.0,
        light: include_bytes!("../assets/icons/checked.svg"),
        dark: Some(include_bytes!("../assets/icons/checked_dark.svg")),
    },
];

/// The icon of a server by its icon key (the tool id).
pub fn server_icon(key: &str) -> Option<IconName> {
    match Tool::parse(key)? {
        Tool::Ruff => Some(RUFF_ICON),
        Tool::Ty => Some(TY_ICON),
        Tool::Basedpyright => Some(BASEDPYRIGHT_ICON),
    }
}

/// Makes the LSP client of a project: what the workbench talks to, and
/// where its server specs go (alive as long as the client).
pub type ClientFactory = Rc<dyn Fn() -> (Arc<dyn CodeIntel>, Weak<dyn SpecTarget>)>;

/// The product's code intelligence provider.
pub struct PythonCodeIntel {
    factory: ClientFactory,
    search: Option<Arc<SearchPaths>>,
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
        }
    }

    /// The services over another client, with tools searched in `search`
    /// when given.
    pub fn with_client(factory: ClientFactory, search: Option<SearchPaths>) -> Self {
        Self {
            factory,
            search: search.map(Arc::new),
        }
    }
}

impl CodeIntelProvider for PythonCodeIntel {
    fn start(&self, root: &Path, cx: &mut App) -> Arc<dyn CodeIntel> {
        let (client, target) = (self.factory)();
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
    ui::icons::register(cx, ICONS);
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
mod tests {
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
        assert_eq!(server_icon("ruff"), Some(RUFF_ICON));
        assert_eq!(server_icon("basedpyright"), Some(BASEDPYRIGHT_ICON));
        assert_eq!(server_icon("ty"), Some(TY_ICON));
        assert_eq!(server_icon("pyrefly"), None);
        for icon in ICONS {
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
        let root = std::fs::canonicalize(dir.path()).unwrap();
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
        product.code_intel = Some(workbench::CodeIntelHook::new(provider(&recorder, search)));
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

        // Type Engine Settings opens the settings file.
        invoke(cx, "python.type-engine.settings", Json::Null);
        let active = cx
            .read_window(&window, |w, cx| w.read(cx).active_file(cx))
            .unwrap();
        assert_eq!(active, Some(root.join(".oxide/project.toml")));
    }

    /// A default tool installed nowhere: the person is asked, the answer is
    /// kept and the server runs through uvx.
    #[cfg(unix)]
    #[gpui::test]
    fn default_tools_run_through_uvx_once_allowed(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
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
        product.code_intel = Some(workbench::CodeIntelHook::new(provider(&recorder, search)));
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
    }
}
