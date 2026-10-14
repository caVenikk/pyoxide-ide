//! The Python services of a project as a whole: the one-time imports of
//! the reference's settings, the settings, every tool's lookup, version
//! and problem, and the server specs.
//!
//! The product runs [`prepare`] off the UI thread when a project opens
//! and again when its settings or its interpreter change, hands
//! [`ProjectState::specs`] to its [`SpecTarget`] (the LSP client), then
//! asks the tools for their versions ([`probe`]) and loads ruff's rule
//! names ([`crate::ruff::load_rules`]) while the servers start.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lsp_client::ServerSpec;
use serde::Serialize;

use crate::idea;
use crate::lookup::{
    Executable, LookupContext, Resolution, SearchPaths, below_minimum, probe_version, resolve,
};
use crate::ruff::RuffCustomizer;
use crate::servers::{self, SpecContext};
use crate::settings::{self, Settings, Tool, ToolSettings};

/// Where the servers of a project go: the LSP client's
/// `LspCodeIntel::set_specs` (the servers whose spec changed or went away
/// stop, new ones start when a document needs them).
pub trait SpecTarget: Send + Sync {
    fn set_specs(&self, specs: Vec<ServerSpec>);
}

/// Why a tool that should run does not, or runs with a warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Problem {
    /// Installed nowhere, and uv is not there to run it.
    NotFound,
    /// Only `uvx` could run it, and the person has not answered whether
    /// it may (a default tool, ADR 0010).
    NeedsUvxConsent,
    /// Only `uvx` could run it, and the person said no.
    UvxDeclined,
    /// Older than the oldest version Pyoxide is tested with (it runs).
    BelowMinimum { version: String, minimum: String },
    /// Its version could not be read (it runs).
    Version { message: String },
}

impl Problem {
    /// The tool does not run because of it.
    pub fn stops(&self) -> bool {
        matches!(
            self,
            Problem::NotFound | Problem::NeedsUvxConsent | Problem::UvxDeclined
        )
    }

    /// What the Language Services widget says about it.
    pub fn message(&self, tool: Tool) -> String {
        let name = tool.name();
        match self {
            Problem::NotFound => format!("{name} is not installed, and uv is not there to run it"),
            Problem::NeedsUvxConsent => format!(
                "{name} is not installed. Run it through uv ({})?",
                uvx_display(tool)
            ),
            Problem::UvxDeclined => {
                format!("{name} is not installed; running it through uv was declined")
            }
            Problem::BelowMinimum { version, minimum } => format!(
                "{name} {version} is older than {minimum}, the oldest version Pyoxide is tested with"
            ),
            Problem::Version { message } => format!("{name}: {message}"),
        }
    }
}

/// `uvx ty`, `uvx --from basedpyright basedpyright-langserver`.
fn uvx_display(tool: Tool) -> String {
    Executable::uvx(tool, PathBuf::from("uvx"))
        .command_line(&[])
        .display()
}

/// One tool of a project.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolState {
    pub tool: Tool,
    pub name: &'static str,
    /// It runs as a server of the project (ruff enabled, or the type
    /// engine's tool).
    pub runs: bool,
    pub settings: ToolSettings,
    /// The person's custom executable.
    pub custom_path: Option<PathBuf>,
    /// The person's answer whether it may run through `uvx`.
    pub uvx_consent: Option<bool>,
    /// The lookup, for a tool that runs.
    pub resolution: Option<Resolution>,
    /// The lookup steps with marks (`Sdk ✗ · Path ✓ · uvx ✓`).
    pub lookup: Option<String>,
    /// The command line of its server.
    pub command: Option<String>,
    pub version: Option<String>,
    pub problem: Option<Problem>,
}

impl ToolState {
    pub fn executable(&self) -> Option<&Executable> {
        self.resolution.as_ref()?.executable.as_ref()
    }
}

/// The Python services of a project.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectState {
    pub root: PathBuf,
    pub interpreter: Option<PathBuf>,
    pub settings: Settings,
    /// Every tool, in [`Tool::ALL`] order.
    pub tools: Vec<ToolState>,
    /// Notes of the import of the reference's settings, when it happened
    /// in this run.
    pub notes: Vec<String>,
    /// Settings that could not be read or written.
    pub errors: Vec<String>,
}

impl ProjectState {
    pub fn tool(&self, tool: Tool) -> &ToolState {
        self.tools
            .iter()
            .find(|t| t.tool == tool)
            .expect("every tool has a state")
    }

    /// The tools waiting for the person's answer whether they may run
    /// through `uvx`.
    pub fn uvx_questions(&self) -> Vec<Tool> {
        self.tools
            .iter()
            .filter(|t| t.problem == Some(Problem::NeedsUvxConsent))
            .map(|t| t.tool)
            .collect()
    }

    /// The server specs: ruff (with `ruff` as its customizer) and the type
    /// server, for the tools that were found.
    pub fn specs(&self, ruff: Option<&Arc<RuffCustomizer>>) -> Vec<ServerSpec> {
        let context = SpecContext {
            root: &self.root,
            interpreter: self.interpreter.as_deref(),
            settings: &self.settings,
            ruff,
        };
        servers::specs(&context, |tool| self.tool(tool).executable().cloned())
    }
}

/// The LSP client of a project takes the specs as they are.
impl SpecTarget for lsp_client::LspCodeIntel {
    fn set_specs(&self, specs: Vec<ServerSpec>) {
        lsp_client::LspCodeIntel::set_specs(self, specs);
    }
}

/// How [`prepare`] runs.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrepareOptions<'a> {
    /// Run the one-time imports of the reference's exclusions and tool
    /// settings first (when the project opens).
    pub import: bool,
    /// Where tools are searched; the application's environment when
    /// `None`.
    pub search: Option<&'a SearchPaths>,
}

/// Reads the settings of the project at `root` and looks every tool that
/// runs up (blocking: the file system; no process is started).
///
/// With [`PrepareOptions::import`] the reference's exclusions are
/// imported first and its tool settings after them: the exclusions are
/// imported only while `project.toml` does not exist, and the tool
/// settings may create it.
pub fn prepare(root: &Path, interpreter: Option<&Path>, options: PrepareOptions) -> ProjectState {
    let mut notes = Vec::new();
    let mut errors = Vec::new();
    if options.import {
        if let Err(e) = project::store::import_idea(root) {
            errors.push(e.to_string());
        }
        match idea::import_tool_settings(root) {
            Ok(Some(imported)) => notes = imported.notes,
            Ok(None) => {}
            Err(e) => errors.push(e.to_string()),
        }
    }
    let settings = settings::load(root).unwrap_or_else(|e| {
        errors.push(e.to_string());
        Settings::default()
    });
    let servers = settings.servers();
    let mut tools = Vec::new();
    for tool in Tool::ALL {
        let custom_path = reported(settings::custom_path(root, tool), &mut errors);
        let uvx_consent = reported(settings::uvx_consent(root, tool), &mut errors);
        let own = settings.tool(tool).clone();
        let runs = servers.contains(&tool);
        let mut state = ToolState {
            tool,
            name: tool.name(),
            runs,
            settings: own.clone(),
            custom_path,
            uvx_consent,
            resolution: None,
            lookup: None,
            command: None,
            version: None,
            problem: None,
        };
        if runs {
            let context = LookupContext {
                interpreter,
                custom_path: state.custom_path.as_deref(),
                allow_uvx: own.uvx_without_asking() || uvx_consent == Some(true),
                search: options.search,
            };
            let resolution = resolve(tool, own.lookup, &context);
            state.problem = match (&resolution.executable, resolution.needs_uvx_consent) {
                (Some(_), _) => None,
                (None, true) if uvx_consent == Some(false) => Some(Problem::UvxDeclined),
                (None, true) => Some(Problem::NeedsUvxConsent),
                (None, false) => Some(Problem::NotFound),
            };
            state.command = resolution
                .executable
                .as_ref()
                .map(|e| e.command_line(tool.server_args()).display());
            state.lookup = Some(resolution.summary());
            state.resolution = Some(resolution);
        }
        tools.push(state);
    }
    ProjectState {
        root: root.to_owned(),
        interpreter: interpreter.map(Path::to_owned),
        settings,
        tools,
        notes,
        errors,
    }
}

/// The value of a personal setting; `None` with the error reported when
/// it cannot be read.
fn reported<T>(
    result: Result<Option<T>, project::StoreError>,
    errors: &mut Vec<String>,
) -> Option<T> {
    result.unwrap_or_else(|e| {
        errors.push(e.to_string());
        None
    })
}

/// How long a tool may take to answer `--version` in [`probe`].
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Asks the tools that run for their versions (blocking: a process per
/// tool). Tools run through `uvx` are not asked: `uvx` runs the newest
/// release, and asking might download it first. A ty older than the
/// minimum gets [`Problem::BelowMinimum`].
pub fn probe(state: &mut ProjectState, timeout: Duration) {
    for tool in &mut state.tools {
        let Some(Executable::Path(path)) = tool.executable() else {
            continue;
        };
        let executable = Executable::Path(path.clone());
        match probe_version(tool.tool, &executable, timeout) {
            Ok(version) => {
                if below_minimum(tool.tool, &version) {
                    tool.problem = Some(Problem::BelowMinimum {
                        version: version.clone(),
                        minimum: tool.tool.minimum_version().unwrap_or_default().to_owned(),
                    });
                }
                tool.version = Some(version);
            }
            Err(e) => {
                tool.problem = Some(Problem::Version {
                    message: e.to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookup::Lookup;
    use crate::settings::TypeEngine;

    /// An executable file (on Unix: mode 755) with `text`.
    fn tool_file(dir: &Path, name: &str, text: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn exe(name: &str) -> String {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        }
    }

    struct Project {
        dir: tempfile::TempDir,
        python: PathBuf,
        tools: PathBuf,
        search: SearchPaths,
    }

    impl Project {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let python = py_env::paths::venv_python(&dir.path().join(".venv"));
            tool_file(
                python.parent().unwrap(),
                python.file_name().unwrap().to_str().unwrap(),
                "",
            );
            let tools = dir.path().join("tools");
            std::fs::create_dir_all(&tools).unwrap();
            let search = SearchPaths {
                path: vec![tools.clone()],
                user_dirs: Vec::new(),
                extensions: if cfg!(windows) {
                    crate::lookup::path_extensions(None)
                } else {
                    Vec::new()
                },
            };
            Self {
                dir,
                python,
                tools,
                search,
            }
        }

        fn root(&self) -> &Path {
            self.dir.path()
        }

        fn prepare(&self, import: bool) -> ProjectState {
            prepare(
                self.root(),
                Some(&self.python),
                PrepareOptions {
                    import,
                    search: Some(&self.search),
                },
            )
        }
    }

    #[test]
    fn default_tools_found_or_asked_for() {
        let p = Project::new();
        let state = p.prepare(true);
        assert!(state.errors.is_empty(), "{:?}", state.errors);
        assert_eq!(state.tool(Tool::Ruff).problem, Some(Problem::NotFound));
        assert!(!state.tool(Tool::Basedpyright).runs);
        assert_eq!(state.tool(Tool::Basedpyright).resolution, None);
        assert!(state.specs(None).is_empty());
        // Nothing was written: no import, no settings.
        assert!(!project::store::settings_file(p.root()).exists());

        let ruff = tool_file(p.python.parent().unwrap(), &exe("ruff"), "");
        let uvx = tool_file(&p.tools, &exe("uvx"), "");
        let state = p.prepare(false);
        let ruff_state = state.tool(Tool::Ruff);
        assert_eq!(ruff_state.problem, None);
        assert_eq!(ruff_state.executable(), Some(&Executable::Path(ruff)));
        assert_eq!(ruff_state.lookup.as_deref(), Some("Sdk ✓ · Path ✗ · uvx ✓"));
        // ty, a default tool, would run through uvx: the person is asked.
        assert_eq!(state.tool(Tool::Ty).problem, Some(Problem::NeedsUvxConsent));
        assert_eq!(state.uvx_questions(), [Tool::Ty]);
        assert_eq!(
            Problem::NeedsUvxConsent.message(Tool::Ty),
            "ty is not installed. Run it through uv (uvx ty)?"
        );
        let specs = state.specs(None);
        assert_eq!(specs.len(), 1);
        // No type server runs: ruff keeps the syntax errors.
        assert_eq!(
            specs[0].initialization_options.as_ref().unwrap()["settings"]["showSyntaxErrors"],
            true
        );

        settings::set_uvx_consent(p.root(), Tool::Ty, Some(false)).unwrap();
        let state = p.prepare(false);
        assert_eq!(state.tool(Tool::Ty).problem, Some(Problem::UvxDeclined));
        assert!(state.uvx_questions().is_empty());

        settings::set_uvx_consent(p.root(), Tool::Ty, Some(true)).unwrap();
        let state = p.prepare(false);
        let ty = state.tool(Tool::Ty);
        assert_eq!(ty.problem, None);
        assert_eq!(ty.executable(), Some(&Executable::uvx(Tool::Ty, uvx)));
        let specs = state.specs(None);
        let ids: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["ruff", "ty"]);
        assert_eq!(specs[1].command.args, ["ty", "server"]);
        let python = p.python.to_string_lossy().into_owned();
        assert_eq!(
            specs[1].settings["ty"]["configuration"]["environment"]["python"],
            python.as_str()
        );
        assert_eq!(
            specs[0].initialization_options.as_ref().unwrap()["settings"]["showSyntaxErrors"],
            false
        );
    }

    #[test]
    fn tools_enabled_explicitly_run_through_uvx() {
        let p = Project::new();
        tool_file(&p.tools, &exe("uvx"), "");
        let mut s = Settings::default();
        s.select_type_engine(TypeEngine::Basedpyright);
        s.ruff.lookup = Lookup::Uvx;
        settings::save(p.root(), &s).unwrap();
        let state = p.prepare(false);
        assert_eq!(state.settings, s);
        assert!(state.tools.iter().all(|t| t.problem.is_none()));
        assert!(!state.tool(Tool::Ty).runs);
        let commands: Vec<String> = state
            .specs(None)
            .iter()
            .map(|s| {
                s.command
                    .args
                    .iter()
                    .map(|a| a.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        assert_eq!(
            commands,
            [
                "ruff server",
                "--from basedpyright basedpyright-langserver --stdio"
            ]
        );
    }

    #[test]
    fn the_reference_settings_are_imported_once() {
        let p = Project::new();
        let idea = p.root().join(".idea");
        std::fs::create_dir_all(&idea).unwrap();
        std::fs::write(
            idea.join("pyTypeEngine.xml"),
            r#"<project version="4"><component name="PyTypeEngineProjectSettings"><option name="typeEngine" value="PYREFLY" /></component></project>"#,
        )
        .unwrap();
        std::fs::write(
            idea.join("pyLspTools.xml"),
            r#"<project version="4"><component name="RuffConfiguration"><option name="enabled" value="true" /><option name="executableDiscoveryMode" value="PATH" /></component></project>"#,
        )
        .unwrap();
        let state = p.prepare(true);
        assert_eq!(state.notes, [idea::NOTE_PYREFLY]);
        assert_eq!(state.settings.ruff.lookup, Lookup::Path);
        assert!(state.settings.ruff.explicit);
        let again = p.prepare(true);
        assert!(again.notes.is_empty());
        assert_eq!(again.settings, state.settings);
    }

    #[test]
    fn unreadable_settings_are_reported() {
        let p = Project::new();
        std::fs::create_dir_all(p.root().join(".oxide")).unwrap();
        std::fs::write(project::store::settings_file(p.root()), "python = 3\n").unwrap();
        let state = p.prepare(false);
        assert_eq!(state.errors.len(), 1, "{:?}", state.errors);
        assert_eq!(state.settings, Settings::default());
    }

    #[cfg(unix)]
    #[test]
    fn versions_and_rule_names() {
        use std::collections::HashMap;

        let p = Project::new();
        let bin = p.python.parent().unwrap().to_owned();
        let ruff = tool_file(
            &bin,
            "ruff",
            "#!/bin/sh\nif [ \"$1\" = rule ]; then echo '[{\"code\": \"F401\", \"name\": \"unused-import\"}]'; else echo 'ruff 0.15.20'; fi\n",
        );
        tool_file(&bin, "ty", "#!/bin/sh\necho 'ty 0.0.80 (abc 2026-09-01)'\n");
        let mut state = p.prepare(false);
        probe(&mut state, PROBE_TIMEOUT);
        assert_eq!(state.tool(Tool::Ruff).version.as_deref(), Some("0.15.20"));
        assert_eq!(state.tool(Tool::Ruff).problem, None);
        let ty = state.tool(Tool::Ty);
        assert_eq!(ty.version.as_deref(), Some("0.0.80"));
        assert_eq!(
            ty.problem,
            Some(Problem::BelowMinimum {
                version: "0.0.80".into(),
                minimum: settings::MINIMUM_TY_VERSION.into()
            })
        );
        assert!(!ty.problem.as_ref().unwrap().stops());
        // It still runs.
        assert_eq!(state.specs(None).len(), 2);
        assert_eq!(
            crate::ruff::load_rules(&Executable::Path(ruff), PROBE_TIMEOUT),
            HashMap::from([("F401".to_owned(), "unused-import".to_owned())])
        );
        let failing = tool_file(&bin, "broken", "#!/bin/sh\nexit 2\n");
        assert!(crate::ruff::load_rules(&Executable::Path(failing), PROBE_TIMEOUT).is_empty());

        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["tools"][1]["problem"]["kind"], "below-minimum");
        assert_eq!(json["tools"][0]["tool"], "ruff");
        assert_eq!(json["settings"]["type_engine"], "ty");
    }
}
