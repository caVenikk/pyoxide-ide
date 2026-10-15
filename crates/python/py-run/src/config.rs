//! The Python run configuration type (the reference's
//! `PythonConfigurationType`, factory `Python`): its options as Pyoxide
//! stores them, the import of the reference's Python configurations (and
//! of its standalone `uv run` configurations, converted into Python ones
//! with `uv run` on, as the reference itself converts them later), and the
//! run they describe (`AbstractPythonRunConfiguration`,
//! `PythonRunConfiguration`, `PythonCommandLineState`).

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use run_config::form::{self, Field, FieldKind, OptionItem, TypeEditor};
use run_config::{EnvVar, IdeaConfiguration, Importer, RunConfiguration, Storage, TypeInfo};
use run_config::{env_file, params, paths};

use crate::{PythonRun, Target, UvRun};

/// Pyoxide's id of the type.
pub const TYPE_ID: &str = "python";
/// The reference's id and display name of the type.
pub const REFERENCE_ID: &str = "PythonConfigurationType";
pub const REFERENCE_NAME: &str = "Python";
/// The reference's standalone `uv run` type (still in 2026.2).
pub const UV_REFERENCE_ID: &str = "UvRunConfigurationType";
pub const UV_REFERENCE_NAME: &str = "uv run";
/// The reference's pytest configurations (`tests`, factory `py.test`):
/// kept read-only until the test runner.
pub const TESTS_REFERENCE_ID: &str = "tests";
pub const TESTS_REFERENCE_NAME: &str = "Python tests";

/// The type.
pub const TYPE: TypeInfo = TypeInfo {
    id: TYPE_ID,
    name: REFERENCE_NAME,
    icon: "python/file",
    reference_id: REFERENCE_ID,
    allow_parallel: false,
};

/// The variable every new Python configuration starts with
/// (`PythonEnvUtil.setPythonUnbuffered`).
pub const UNBUFFERED: (&str, &str) = ("PYTHONUNBUFFERED", "1");

/// Option keys of a Python configuration and the reference's fields.
pub mod key {
    /// The script path, or the module name in module mode (`SCRIPT_NAME`).
    pub const SCRIPT: &str = "script";
    /// Run a module (`MODULE_MODE`).
    pub const MODULE_MODE: &str = "module-mode";
    /// "Script parameters" (`PARAMETERS`), as written.
    pub const PARAMETERS: &str = "parameters";
    /// "Interpreter options" (`INTERPRETER_OPTIONS`).
    pub const INTERPRETER_OPTIONS: &str = "interpreter-options";
    /// "Working directory" (`WORKING_DIRECTORY`); empty is the project folder.
    pub const WORKING_DIRECTORY: &str = "working-directory";
    /// The project's interpreter (`IS_MODULE_SDK`).
    pub const USE_PROJECT_INTERPRETER: &str = "use-project-interpreter";
    /// An interpreter by its path (`SDK_HOME`).
    pub const INTERPRETER: &str = "interpreter";
    /// An interpreter by its name in the reference (`SDK_NAME`).
    pub const INTERPRETER_NAME: &str = "interpreter-name";
    /// "Paths to .env files" (`ENV_FILES`).
    pub const ENV_FILES: &str = "env-files";
    /// "Add content roots to PYTHONPATH" (`ADD_CONTENT_ROOTS`, default on).
    pub const ADD_CONTENT_ROOTS: &str = "add-content-roots";
    /// "Add source roots to PYTHONPATH" (`ADD_SOURCE_ROOTS`, default on).
    pub const ADD_SOURCE_ROOTS: &str = "add-source-roots";
    /// "Run with `uv run`" (`RUN_TOOL`); missing is the default, on for an
    /// interpreter of a uv-managed environment.
    pub const RUN_WITH_UV: &str = "run-with-uv";
    /// "Emulate terminal in output console" (`EMULATE_TERMINAL`).
    pub const EMULATE_TERMINAL: &str = "emulate-terminal";
    /// "Redirect input from" (`REDIRECT_INPUT`, `INPUT_FILE`).
    pub const REDIRECT_INPUT: &str = "redirect-input";
    pub const INPUT_FILE: &str = "input-file";
    /// "Run with Python Console" (`SHOW_COMMAND_LINE`); stored, no effect yet.
    pub const RUN_WITH_PYTHON_CONSOLE: &str = "run-with-python-console";
    /// "Debug just my code" (`DEBUG_JUST_MY_CODE`); stored, no effect yet.
    pub const DEBUG_JUST_MY_CODE: &str = "debug-just-my-code";
    /// The reference's module of the configuration (`<module name>`).
    pub const IDE_MODULE: &str = "ide-module";
}

/// The options of a Python configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonOptions {
    pub script: String,
    pub module_mode: bool,
    pub parameters: String,
    pub interpreter_options: String,
    pub working_directory: String,
    pub use_project_interpreter: bool,
    pub interpreter: String,
    pub interpreter_name: String,
    pub env_files: Vec<String>,
    pub add_content_roots: bool,
    pub add_source_roots: bool,
    pub run_with_uv: Option<bool>,
    pub emulate_terminal: bool,
    pub redirect_input: bool,
    pub input_file: String,
    pub run_with_python_console: bool,
    pub debug_just_my_code: bool,
}

impl Default for PythonOptions {
    /// A new configuration: the project's interpreter, roots added.
    fn default() -> Self {
        Self {
            script: String::new(),
            module_mode: false,
            parameters: String::new(),
            interpreter_options: String::new(),
            working_directory: String::new(),
            use_project_interpreter: true,
            interpreter: String::new(),
            interpreter_name: String::new(),
            env_files: Vec::new(),
            add_content_roots: true,
            add_source_roots: true,
            run_with_uv: None,
            emulate_terminal: false,
            redirect_input: false,
            input_file: String::new(),
            run_with_python_console: false,
            debug_just_my_code: false,
        }
    }
}

impl PythonOptions {
    pub fn read(config: &RunConfiguration) -> Self {
        let flag = |k: &str, default: bool| config.bool_option(k).unwrap_or(default);
        Self {
            script: config.str_option(key::SCRIPT).to_owned(),
            module_mode: flag(key::MODULE_MODE, false),
            parameters: config.str_option(key::PARAMETERS).to_owned(),
            interpreter_options: config.str_option(key::INTERPRETER_OPTIONS).to_owned(),
            working_directory: config.str_option(key::WORKING_DIRECTORY).to_owned(),
            use_project_interpreter: flag(key::USE_PROJECT_INTERPRETER, false),
            interpreter: config.str_option(key::INTERPRETER).to_owned(),
            interpreter_name: config.str_option(key::INTERPRETER_NAME).to_owned(),
            env_files: config.list_option(key::ENV_FILES),
            add_content_roots: flag(key::ADD_CONTENT_ROOTS, true),
            add_source_roots: flag(key::ADD_SOURCE_ROOTS, true),
            run_with_uv: config.bool_option(key::RUN_WITH_UV),
            emulate_terminal: flag(key::EMULATE_TERMINAL, false),
            redirect_input: flag(key::REDIRECT_INPUT, false),
            input_file: config.str_option(key::INPUT_FILE).to_owned(),
            run_with_python_console: flag(key::RUN_WITH_PYTHON_CONSOLE, false),
            debug_just_my_code: flag(key::DEBUG_JUST_MY_CODE, false),
        }
    }

    /// Writes the options into `config`, keeping keys this version does
    /// not know.
    pub fn write(&self, config: &mut RunConfiguration) {
        config.set_str(key::SCRIPT, &self.script);
        config.set_bool(key::MODULE_MODE, self.module_mode);
        config.set_str(key::PARAMETERS, &self.parameters);
        config.set_str(key::INTERPRETER_OPTIONS, &self.interpreter_options);
        config.set_str(key::WORKING_DIRECTORY, &self.working_directory);
        config.set_bool(key::USE_PROJECT_INTERPRETER, self.use_project_interpreter);
        config.set_str(key::INTERPRETER, &self.interpreter);
        config.set_str(key::INTERPRETER_NAME, &self.interpreter_name);
        config.set_list(key::ENV_FILES, &self.env_files);
        config.set_bool(key::ADD_CONTENT_ROOTS, self.add_content_roots);
        config.set_bool(key::ADD_SOURCE_ROOTS, self.add_source_roots);
        match self.run_with_uv {
            Some(on) => config.set_bool(key::RUN_WITH_UV, on),
            None => {
                config.options.remove(key::RUN_WITH_UV);
            }
        }
        config.set_bool(key::EMULATE_TERMINAL, self.emulate_terminal);
        config.set_bool(key::REDIRECT_INPUT, self.redirect_input);
        config.set_str(key::INPUT_FILE, &self.input_file);
        config.set_bool(key::RUN_WITH_PYTHON_CONSOLE, self.run_with_python_console);
        config.set_bool(key::DEBUG_JUST_MY_CODE, self.debug_just_my_code);
    }
}

/// A new Python configuration: the defaults of the reference's template
/// and `PYTHONUNBUFFERED=1`, marked shared.
pub fn new_configuration(name: &str, storage: Storage) -> RunConfiguration {
    let mut config = RunConfiguration::new(TYPE_ID, name, storage);
    PythonOptions::default().write(&mut config);
    config.env = vec![EnvVar::new(UNBUFFERED.0, UNBUFFERED.1, true)];
    config
}

/// Imports the reference's Python and `uv run` configurations, and names
/// its pytest configurations.
pub struct PythonImporter;

impl Importer for PythonImporter {
    fn type_name(&self, reference_type: &str) -> Option<&'static str> {
        Some(match reference_type {
            REFERENCE_ID => REFERENCE_NAME,
            UV_REFERENCE_ID => UV_REFERENCE_NAME,
            TESTS_REFERENCE_ID => TESTS_REFERENCE_NAME,
            _ => return None,
        })
    }

    fn convert(
        &self,
        idea: &IdeaConfiguration<'_>,
        root: &Path,
        config: &mut RunConfiguration,
    ) -> bool {
        match idea.type_id() {
            REFERENCE_ID => {
                import_python(idea, root, config);
                true
            }
            UV_REFERENCE_ID => {
                import_uv_run(idea, root, config);
                true
            }
            _ => false,
        }
    }
}

/// Variables a configuration has from its type: shared even in a
/// personal configuration.
fn is_default_variable(name: &str, value: &str) -> bool {
    (name, value) == UNBUFFERED
}

fn import_python(idea: &IdeaConfiguration<'_>, root: &Path, config: &mut RunConfiguration) {
    config.type_id = TYPE_ID.to_owned();
    let text = |name: &str| idea.field(name).unwrap_or("").to_owned();
    let path = |value: &str| paths::from_reference(value, root);
    let module_mode = idea.bool_field("MODULE_MODE", false);
    let script = text("SCRIPT_NAME");
    let options = PythonOptions {
        // A module name is no path.
        script: if module_mode { script } else { path(&script) },
        module_mode,
        parameters: text("PARAMETERS"),
        interpreter_options: text("INTERPRETER_OPTIONS"),
        working_directory: path(&text("WORKING_DIRECTORY")),
        use_project_interpreter: idea.bool_field("IS_MODULE_SDK", false),
        interpreter: path(&text("SDK_HOME")),
        interpreter_name: text("SDK_NAME"),
        env_files: env_file::split_reference_list(&text("ENV_FILES"))
            .iter()
            .map(|f| path(f))
            .collect(),
        add_content_roots: idea.bool_field("ADD_CONTENT_ROOTS", true),
        add_source_roots: idea.bool_field("ADD_SOURCE_ROOTS", true),
        run_with_uv: idea
            .field("RUN_TOOL")
            .filter(|v| !v.is_empty())
            .map(|v| v.eq_ignore_ascii_case("true")),
        emulate_terminal: idea.bool_field("EMULATE_TERMINAL", false),
        redirect_input: idea.bool_field("REDIRECT_INPUT", false),
        input_file: path(&text("INPUT_FILE")),
        run_with_python_console: idea.bool_field("SHOW_COMMAND_LINE", false),
        debug_just_my_code: idea.bool_field("DEBUG_JUST_MY_CODE", false),
    };
    options.write(config);
    if let Some(module) = idea
        .element
        .child("module")
        .and_then(|m| m.attr("name"))
        .filter(|m| !m.is_empty())
    {
        config.set_str(key::IDE_MODULE, module);
    }
    // `PARENT_ENVS`: kept when missing (the constructor's default is on).
    config.include_system_env = idea.bool_field("PARENT_ENVS", true);
    let shared = config.is_shared();
    config.env = idea
        .envs()
        .into_iter()
        .map(|(name, value)| {
            let shared = shared || is_default_variable(&name, &value);
            EnvVar::new(name, value, shared)
        })
        .collect();
}

/// The standalone `uv run` configuration (`UvRunConfigurationOptions`,
/// serialized by `XmlSerializer`): `runType` SCRIPT or MODULE,
/// `scriptOrModule`, `args`, `env`, `uvSdkKey`, `debugJustMyCode`;
/// `uvArgs` and `checkSync` have no Python counterpart and are dropped, as
/// the reference's own conversion drops them
/// (`LegacyUvRunConfigurationConverter`). The name gets `uv run ` in front.
fn import_uv_run(idea: &IdeaConfiguration<'_>, root: &Path, config: &mut RunConfiguration) {
    let e = idea.element;
    let value = |name: &str| {
        e.option_element(name)
            .and_then(|o| o.attr("value"))
            .unwrap_or("")
            .to_owned()
    };
    let list = |name: &str| -> Vec<String> {
        e.option_element(name)
            .and_then(|o| o.child("list"))
            .map(|l| {
                l.children_named("option")
                    .filter_map(|o| o.attr("value").map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let module_mode = value("runType").eq_ignore_ascii_case("MODULE");
    let target = value("scriptOrModule");
    let sdk = value("uvSdkKey");
    let options = PythonOptions {
        script: if module_mode {
            target
        } else {
            paths::from_reference(&target, root)
        },
        module_mode,
        parameters: params::join(&list("args")),
        use_project_interpreter: sdk.is_empty(),
        interpreter_name: sdk,
        run_with_uv: Some(true),
        debug_just_my_code: value("debugJustMyCode").eq_ignore_ascii_case("true"),
        ..PythonOptions::default()
    };
    config.type_id = TYPE_ID.to_owned();
    options.write(config);
    // The constructor's variable, then the configuration's (merged).
    let shared = config.is_shared();
    config.env = vec![EnvVar::new(UNBUFFERED.0, UNBUFFERED.1, true)];
    let entries = e
        .option_element("env")
        .and_then(|o| o.child("map"))
        .map(|m| {
            m.children_named("entry")
                .filter_map(|x| Some((x.attr("key")?.to_owned(), x.attr("value")?.to_owned())))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for (name, value) in entries {
        match config.env.iter_mut().find(|v| v.name == name) {
            Some(v) => v.value = Some(value),
            None => config.env.push(EnvVar::new(name, value, shared)),
        }
    }
    if !config.name.is_empty() {
        config.name = format!("uv run {}", config.name);
    }
}

/// The options of the reference's pytest configuration for display: its
/// `_new_<name>` fields hold JSON-encoded values (`PyTestsShared`,
/// `@ConfigField`), decoded here (`"PATH"` is `PATH`).
pub fn pytest_options(xml: &str) -> Vec<(String, String)> {
    let Ok(e) = run_config::xml::parse(xml) else {
        return Vec::new();
    };
    e.children_named("option")
        .filter_map(|o| {
            let name = o.attr("name")?.strip_prefix("_new_")?.to_owned();
            let raw = o.attr("value").unwrap_or("");
            let value = match serde_json::from_str::<serde_json::Value>(raw) {
                Ok(serde_json::Value::String(s)) => s,
                Ok(other) => other.to_string(),
                Err(_) => raw.to_owned(),
            };
            Some((name, value))
        })
        .collect()
}

/// What a Python configuration's run needs from the product besides the
/// configuration: the project, its source roots, the interpreter chosen
/// for the configuration and, when it runs through `uv run`, uv.
#[derive(Debug, Clone)]
pub struct LaunchContext<'a> {
    pub root: &'a Path,
    pub source_roots: &'a [PathBuf],
    pub python: PathBuf,
    pub uv: Option<UvRun>,
}

/// A Python configuration's run, and the file its standard input comes
/// from ("Redirect input from").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub run: PythonRun,
    pub input_file: Option<PathBuf>,
}

/// The run of a Python configuration (`PythonScriptCommandLineState`):
/// the script or module, the parameters with path variables expanded,
/// the working directory (the project folder when empty), the
/// environment (the variables of the env files first, then the
/// configuration's over them), the project's content and source roots
/// on `PYTHONPATH` as the options say. Reads the env files: run it off the
/// UI thread. The error is the reference's message.
pub fn launch(config: &RunConfiguration, cx: &LaunchContext<'_>) -> Result<Launch, String> {
    let options = PythonOptions::read(config);
    let root = cx.root;
    let target = if options.module_mode {
        let module = options.script.trim();
        if module.is_empty() {
            return Err("Please specify a qualified name of the module".to_owned());
        }
        Target::Module(module.to_owned())
    } else {
        match paths::resolve(&options.script, root) {
            Some(script) => Target::Script(script),
            None => return Err("Please specify a script name".to_owned()),
        }
    };
    let working_dir =
        paths::resolve(&options.working_directory, root).unwrap_or_else(|| root.to_owned());
    let args = |text: &str| -> Vec<OsString> {
        params::parse(&paths::expand_in_text(text, root))
            .into_iter()
            .map(OsString::from)
            .collect()
    };
    let mut env: Vec<(OsString, OsString)> = Vec::new();
    let mut put = |name: &str, value: &str| {
        let name = OsString::from(name);
        match env.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1 = value.into(),
            None => env.push((name, value.into())),
        }
    };
    for file in &options.env_files {
        let Some(path) = paths::resolve(file, root) else {
            continue;
        };
        for (name, value) in env_file::read(&path)? {
            put(&name, &value);
        }
    }
    for (name, value) in config.env_values() {
        put(name, value);
    }
    if options.emulate_terminal && !cfg!(windows) && !config.env.iter().any(|v| v.name == "TERM") {
        put("TERM", "xterm-256color");
    }
    let run = PythonRun {
        python: cx.python.clone(),
        uv: cx.uv.clone(),
        target,
        script_args: args(&options.parameters),
        interpreter_options: args(&options.interpreter_options),
        working_dir,
        content_roots: if options.add_content_roots {
            vec![root.to_owned()]
        } else {
            Vec::new()
        },
        source_roots: if options.add_source_roots {
            cx.source_roots.to_vec()
        } else {
            Vec::new()
        },
        env,
        inherit_env: config.include_system_env,
    };
    let input_file = if options.redirect_input {
        paths::resolve(&options.input_file, root)
    } else {
        None
    };
    Ok(Launch { run, input_file })
}

/// A configuration for running a file from its context (the gutter, the
/// project tree, Run Context Configuration), as the reference's
/// `PythonRunConfigurationProducer` makes one: named after the file without
/// `.py`, the script, its folder as the working directory, the project's
/// interpreter. `None` for a file outside the project.
pub fn context_configuration(file: &Path, root: &Path) -> Option<RunConfiguration> {
    let relative = file.strip_prefix(root).ok()?;
    let script = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let folder = relative
        .parent()
        .map(|p| {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| paths::PROJECT.to_owned());
    let name = file.file_stem()?.to_string_lossy().into_owned();
    let mut config = new_configuration(&name, Storage::Personal);
    config.name_is_generated = true;
    let mut o = PythonOptions::read(&config);
    o.script = script;
    o.working_directory = folder;
    o.write(&mut config);
    Some(config)
}

/// An existing configuration runs the same as a context one: the same
/// script (`isConfigurationFromContext`).
pub fn same_context(existing: &RunConfiguration, context: &RunConfiguration) -> bool {
    let (a, b) = (PythonOptions::read(existing), PythonOptions::read(context));
    existing.type_id == TYPE_ID && !a.module_mode && !b.module_mode && a.script == b.script
}

/// The Python type's form (`PythonConfigurationFragmentedEditor`).
pub struct PythonEditor;

/// Ids of the Python form's fields and options.
pub mod field {
    pub const INTERPRETER: &str = "interpreter";
    pub const TARGET: &str = "target";
    pub const SCRIPT: &str = "script";
    pub const PARAMETERS: &str = "parameters";
    pub const INTERPRETER_OPTIONS: &str = "interpreter-options";
    pub const WORKING_DIRECTORY: &str = "working-directory";
    pub const ENV_FILES: &str = "env-files";
    pub const REDIRECT_INPUT: &str = "redirect-input";
    pub const INPUT_FILE: &str = "input-file";
    pub const ADD_CONTENT_ROOTS: &str = "add-content-roots";
    pub const ADD_SOURCE_ROOTS: &str = "add-source-roots";
    pub const RUN_WITH_PYTHON_CONSOLE: &str = "run-with-python-console";
    pub const EMULATE_TERMINAL: &str = "emulate-terminal";
    pub const RUN_WITH_UV: &str = "run-with-uv";
    pub const DEBUG_JUST_MY_CODE: &str = "debug-just-my-code";
}

/// A value typed as an interpreter is a path when it looks like one.
fn looks_like_path(value: &str) -> bool {
    value.contains('/') || value.contains('\\') || value.starts_with('$')
}

impl TypeEditor for PythonEditor {
    fn type_id(&self) -> &'static str {
        TYPE_ID
    }

    fn create(&self, name: &str, storage: Storage) -> RunConfiguration {
        new_configuration(name, storage)
    }

    fn fields(&self, config: &RunConfiguration, shown: &BTreeSet<String>) -> Vec<Field> {
        let o = PythonOptions::read(config);
        let interpreter = if o.use_project_interpreter {
            String::new()
        } else if !o.interpreter_name.is_empty() {
            o.interpreter_name.clone()
        } else {
            o.interpreter.clone()
        };
        let mut fields = vec![
            Field::new(
                field::INTERPRETER,
                "Python interpreter:",
                FieldKind::Text,
                interpreter,
            )
            .placeholder("Project interpreter"),
            Field::new(
                field::TARGET,
                "Run:",
                FieldKind::Choice(vec![
                    ("script".into(), "script".into()),
                    ("module".into(), "module".into()),
                ]),
                if o.module_mode { "module" } else { "script" },
            ),
        ];
        let mut script = if o.module_mode {
            Field::new(
                field::SCRIPT,
                "Module name:",
                FieldKind::Text,
                o.script.clone(),
            )
            .placeholder("Module name")
        } else {
            Field::new(
                field::SCRIPT,
                "Script path:",
                FieldKind::Path,
                o.script.clone(),
            )
            .placeholder("Script path")
        };
        if !o.module_mode
            && let Some(var) = paths::flagged_variable(&o.script)
        {
            script.note = Some(format!("{var} is kept as written"));
        }
        fields.push(script);
        fields.push(Field::new(
            field::PARAMETERS,
            "Parameters:",
            FieldKind::Text,
            o.parameters.clone(),
        ));
        if !o.interpreter_options.is_empty() || shown.contains(field::INTERPRETER_OPTIONS) {
            fields.push(Field::new(
                field::INTERPRETER_OPTIONS,
                "Interpreter options:",
                FieldKind::Text,
                o.interpreter_options.clone(),
            ));
        }
        fields.push(Field::new(
            field::WORKING_DIRECTORY,
            "Working directory:",
            FieldKind::Path,
            o.working_directory.clone(),
        ));
        fields.push(Field::new(
            form::common::ENV,
            "Environment variables:",
            FieldKind::Env,
            form::env_text(&config.env),
        ));
        if !o.env_files.is_empty() || shown.contains(field::ENV_FILES) {
            fields.push(Field::new(
                field::ENV_FILES,
                "Paths to \".env\" files:",
                FieldKind::Text,
                o.env_files.join(";"),
            ));
        }
        if o.redirect_input {
            fields.push(Field::new(
                field::INPUT_FILE,
                "Redirect input from:",
                FieldKind::Path,
                o.input_file.clone(),
            ));
        }
        fields
    }

    fn options(&self, config: &RunConfiguration, shown: &BTreeSet<String>) -> Vec<OptionItem> {
        let o = PythonOptions::read(config);
        let os = "Operating System";
        let py = "Python";
        let mut options = vec![
            OptionItem::new(
                os,
                field::ENV_FILES,
                "Paths to \".env\" files",
                !o.env_files.is_empty() || shown.contains(field::ENV_FILES),
            ),
            OptionItem::new(
                os,
                field::REDIRECT_INPUT,
                "Redirect input",
                o.redirect_input,
            ),
            OptionItem::new(
                py,
                field::INTERPRETER_OPTIONS,
                "Interpreter options",
                !o.interpreter_options.is_empty() || shown.contains(field::INTERPRETER_OPTIONS),
            ),
            OptionItem::new(
                py,
                field::ADD_CONTENT_ROOTS,
                "Add content roots to PYTHONPATH",
                o.add_content_roots,
            ),
            OptionItem::new(
                py,
                field::ADD_SOURCE_ROOTS,
                "Add source roots to PYTHONPATH",
                o.add_source_roots,
            ),
            OptionItem::new(
                py,
                field::RUN_WITH_PYTHON_CONSOLE,
                "Run with Python Console",
                o.run_with_python_console,
            ),
            OptionItem::new(
                py,
                field::EMULATE_TERMINAL,
                "Emulate terminal in output console",
                o.emulate_terminal,
            ),
            OptionItem::new(
                py,
                field::RUN_WITH_UV,
                "Run with `uv run`",
                o.run_with_uv.unwrap_or(true),
            ),
            OptionItem::new(
                "Debug",
                field::DEBUG_JUST_MY_CODE,
                "Debug just my code",
                o.debug_just_my_code,
            ),
        ];
        options.extend(form::common_options(config, false));
        options
    }

    fn set(&self, config: &mut RunConfiguration, id: &str, value: &str) -> bool {
        let mut o = PythonOptions::read(config);
        match id {
            field::INTERPRETER => {
                let value = value.trim();
                o.interpreter.clear();
                o.interpreter_name.clear();
                o.use_project_interpreter = value.is_empty();
                if looks_like_path(value) {
                    o.interpreter = form::stored_path(value);
                } else {
                    o.interpreter_name = value.to_owned();
                }
            }
            field::TARGET => o.module_mode = value == "module",
            field::SCRIPT => {
                o.script = if o.module_mode {
                    value.trim().to_owned()
                } else {
                    form::stored_path(value)
                }
            }
            field::PARAMETERS => o.parameters = value.to_owned(),
            field::INTERPRETER_OPTIONS => o.interpreter_options = value.to_owned(),
            field::WORKING_DIRECTORY => o.working_directory = form::stored_path(value),
            field::ENV_FILES => {
                o.env_files = value
                    .split(';')
                    .map(form::stored_path)
                    .filter(|p| !p.is_empty())
                    .collect()
            }
            field::INPUT_FILE => o.input_file = form::stored_path(value),
            _ => return false,
        }
        o.write(config);
        true
    }

    fn toggle(&self, config: &mut RunConfiguration, option: &str, shown: &mut BTreeSet<String>) {
        let mut o = PythonOptions::read(config);
        // A field option shows its field; turned off it is hidden and back
        // to its default.
        let flip = |on: bool, shown: &mut BTreeSet<String>| {
            if on {
                shown.remove(option);
            } else {
                shown.insert(option.to_owned());
            }
        };
        match option {
            field::ENV_FILES => {
                let on = !o.env_files.is_empty() || shown.contains(option);
                flip(on, shown);
                if on {
                    o.env_files.clear();
                }
            }
            field::INTERPRETER_OPTIONS => {
                let on = !o.interpreter_options.is_empty() || shown.contains(option);
                flip(on, shown);
                if on {
                    o.interpreter_options.clear();
                }
            }
            field::REDIRECT_INPUT => {
                o.redirect_input = !o.redirect_input;
                if !o.redirect_input {
                    o.input_file.clear();
                }
            }
            field::ADD_CONTENT_ROOTS => o.add_content_roots = !o.add_content_roots,
            field::ADD_SOURCE_ROOTS => o.add_source_roots = !o.add_source_roots,
            field::RUN_WITH_PYTHON_CONSOLE => {
                o.run_with_python_console = !o.run_with_python_console
            }
            field::EMULATE_TERMINAL => o.emulate_terminal = !o.emulate_terminal,
            field::RUN_WITH_UV => o.run_with_uv = Some(!o.run_with_uv.unwrap_or(true)),
            field::DEBUG_JUST_MY_CODE => o.debug_just_my_code = !o.debug_just_my_code,
            other => {
                form::toggle_common(config, other, false);
                return;
            }
        }
        o.write(config);
    }

    fn validate(&self, config: &RunConfiguration) -> Option<String> {
        let o = PythonOptions::read(config);
        if o.script.trim().is_empty() {
            return Some(if o.module_mode {
                "Please specify a qualified name of the module".to_owned()
            } else {
                "Please specify a script name".to_owned()
            });
        }
        None
    }
}

/// The options of an unsupported configuration the Python product knows,
/// for the dialog: the decoded options of a pytest configuration.
pub fn unsupported_details(config: &RunConfiguration) -> Vec<(String, String)> {
    match &config.reference {
        Some(r) if r.type_id == TESTS_REFERENCE_ID => pytest_options(&r.xml),
        _ => Vec::new(),
    }
}

/// Whether a configuration runs through `uv run` with an interpreter of a
/// uv-managed environment (`RUN_TOOL`: missing is uv's default, on).
pub fn runs_with_uv(config: &RunConfiguration) -> bool {
    PythonOptions::read(config).run_with_uv.unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use run_config::idea::{read_shared_document, read_workspace};

    const ROOT: &str = "/work/demo";

    fn import(text: &str) -> Vec<RunConfiguration> {
        read_workspace(text, Path::new(ROOT), &[&PythonImporter])
            .unwrap()
            .configurations
            .into_iter()
            .map(|c| c.config)
            .collect()
    }

    /// A configuration with every Python field the reference writes.
    const EVERY_OPTION: &str = r#"<project version="4"><component name="RunManager">
    <configuration name="server" type="PythonConfigurationType" factoryName="Python" folderName="Servers">
      <module name="demo" />
      <option name="ENV_FILES" value="$PROJECT_DIR$/.env:$PROJECT_DIR$/../shared/.env" />
      <option name="INTERPRETER_OPTIONS" value="-X dev" />
      <option name="PARENT_ENVS" value="false" />
      <envs>
        <env name="PYTHONUNBUFFERED" value="1" />
        <env name="API_TOKEN" value="personal-value" />
      </envs>
      <option name="SDK_HOME" value="$PROJECT_DIR$/.venv/bin/python" />
      <option name="SDK_NAME" value="uv (demo)" />
      <option name="WORKING_DIRECTORY" value="$PROJECT_DIR$/app" />
      <option name="IS_MODULE_SDK" value="false" />
      <option name="ADD_CONTENT_ROOTS" value="false" />
      <option name="ADD_SOURCE_ROOTS" value="true" />
      <option name="DEBUG_JUST_MY_CODE" value="true" />
      <EXTENSION ID="PythonCoverageRunConfigurationExtension" runner="coverage.py" />
      <option name="RUN_TOOL" value="false" />
      <option name="SCRIPT_NAME" value="$PROJECT_DIR$/app/server.py" />
      <option name="PARAMETERS" value="--config $PROJECT_DIR$/conf.yml --name &quot;two words&quot;" />
      <option name="SHOW_COMMAND_LINE" value="true" />
      <option name="EMULATE_TERMINAL" value="true" />
      <option name="MODULE_MODE" value="false" />
      <option name="REDIRECT_INPUT" value="true" />
      <option name="INPUT_FILE" value="$USER_HOME$/input.txt" />
      <method v="2" />
    </configuration>
    <configuration name="worker" type="PythonConfigurationType" factoryName="Python">
      <option name="ENV_FILES" value="" />
      <option name="IS_MODULE_SDK" value="true" />
      <option name="RUN_TOOL" value="" />
      <option name="SCRIPT_NAME" value="celery" />
      <option name="PARAMETERS" value="-A app worker" />
      <option name="MODULE_MODE" value="true" />
      <method v="2" />
    </configuration>
    <configuration name="tool" type="PythonConfigurationType" factoryName="Python">
      <option name="RUN_TOOL" value="true" />
      <option name="SCRIPT_NAME" value="$MY_TOOLS$/tool.py" />
      <method v="2" />
    </configuration>
  </component></project>"#;

    #[test]
    fn every_python_option_is_imported() {
        let configs = import(EVERY_OPTION);
        let server = &configs[0];
        assert_eq!(server.type_id, TYPE_ID);
        assert_eq!(server.folder.as_deref(), Some("Servers"));
        let o = PythonOptions::read(server);
        assert_eq!(
            o,
            PythonOptions {
                script: "app/server.py".into(),
                module_mode: false,
                parameters: "--config $PROJECT_DIR$/conf.yml --name \"two words\"".into(),
                interpreter_options: "-X dev".into(),
                working_directory: "app".into(),
                use_project_interpreter: false,
                interpreter: ".venv/bin/python".into(),
                interpreter_name: "uv (demo)".into(),
                env_files: vec![".env".into(), "../shared/.env".into()],
                add_content_roots: false,
                add_source_roots: true,
                run_with_uv: Some(false),
                emulate_terminal: true,
                redirect_input: true,
                input_file: "$USER_HOME$/input.txt".into(),
                run_with_python_console: true,
                debug_just_my_code: true,
            }
        );
        assert_eq!(server.str_option(key::IDE_MODULE), "demo");
        assert!(!server.include_system_env);
        // A personal configuration: its variables are personal, the
        // type's default is shared.
        assert_eq!(
            server
                .env
                .iter()
                .map(|v| (v.name.as_str(), v.value.as_deref(), v.shared))
                .collect::<Vec<_>>(),
            [
                ("PYTHONUNBUFFERED", Some("1"), true),
                ("API_TOKEN", Some("personal-value"), false)
            ]
        );
        let worker = PythonOptions::read(&configs[1]);
        assert!(worker.module_mode && worker.use_project_interpreter);
        assert_eq!(worker.script, "celery");
        assert_eq!(worker.run_with_uv, None);
        assert!(worker.env_files.is_empty());
        assert!(configs[1].include_system_env);
        let tool = PythonOptions::read(&configs[2]);
        assert_eq!(tool.run_with_uv, Some(true));
        assert_eq!(tool.script, "$MY_TOOLS$/tool.py");
        assert_eq!(paths::flagged_variable(&tool.script), Some("$MY_TOOLS$"));
        assert!(runs_with_uv(&configs[1]) && runs_with_uv(&configs[2]));
        assert!(!runs_with_uv(&configs[0]));
    }

    #[test]
    fn env_files_written_on_windows() {
        let text = r#"<component name="ProjectRunConfigurationManager">
  <configuration name="win" type="PythonConfigurationType" factoryName="Python">
    <option name="ENV_FILES" value="$PROJECT_DIR$/.env;C:/config/.env" />
    <envs><env name="A" value="1" /></envs>
  </configuration>
</component>"#;
        let configs = read_shared_document(text, Path::new(ROOT), &[&PythonImporter]);
        let c = &configs[0].config;
        assert_eq!(c.storage, Storage::Shared);
        assert_eq!(
            PythonOptions::read(c).env_files,
            [".env".to_owned(), "C:/config/.env".to_owned()]
        );
        // Imported shared variables are shared.
        assert!(c.env[0].shared);
    }

    #[test]
    fn the_standalone_uv_run_type_becomes_python_with_uv_run() {
        let text = r#"<project><component name="RunManager">
  <configuration name="serve" type="UvRunConfigurationType" factoryName="UvRunConfigurationType">
    <option name="args">
      <list>
        <option value="--port" />
        <option value="8000 1" />
      </list>
    </option>
    <option name="checkSync" value="false" />
    <option name="env">
      <map>
        <entry key="MODE" value="dev" />
      </map>
    </option>
    <option name="runType" value="MODULE" />
    <option name="scriptOrModule" value="app.main" />
    <option name="uvArgs">
      <list>
        <option value="--frozen" />
      </list>
    </option>
    <option name="uvSdkKey" value="uv (demo)" />
    <method v="2" />
  </configuration>
  <configuration name="script" type="UvRunConfigurationType" factoryName="UvRunConfigurationType">
    <option name="scriptOrModule" value="$PROJECT_DIR$/main.py" />
    <method v="2" />
  </configuration>
</component></project>"#;
        let ws = read_workspace(text, Path::new(ROOT), &[&PythonImporter]).unwrap();
        assert_eq!(ws.configurations[0].unique_id, "uv run.serve");
        let serve = &ws.configurations[0].config;
        assert_eq!(serve.type_id, TYPE_ID);
        assert_eq!(serve.name, "uv run serve");
        let o = PythonOptions::read(serve);
        assert!(o.module_mode);
        assert_eq!(o.script, "app.main");
        assert_eq!(o.parameters, "--port \"8000 1\"");
        assert_eq!(o.interpreter_name, "uv (demo)");
        assert!(!o.use_project_interpreter);
        assert_eq!(o.run_with_uv, Some(true));
        assert_eq!(
            serve.env_values().collect::<Vec<_>>(),
            [("PYTHONUNBUFFERED", "1"), ("MODE", "dev")]
        );
        let script = PythonOptions::read(&ws.configurations[1].config);
        assert!(!script.module_mode && script.use_project_interpreter);
        assert_eq!(script.script, "main.py");
    }

    #[test]
    fn pytest_configurations_stay_read_only_with_decoded_options() {
        let text = r#"<project><component name="RunManager">
  <configuration name="tests in api" type="tests" factoryName="py.test">
    <option name="ENV_FILES" value="" />
    <option name="SDK_NAME" value="uv (demo)" />
    <option name="_new_keywords" value="&quot;&quot;" />
    <option name="_new_parameters" value="&quot;-x&quot;" />
    <option name="_new_additionalArguments" value="&quot;--maxfail=1&quot;" />
    <option name="_new_target" value="&quot;$PROJECT_DIR$/tests&quot;" />
    <option name="_new_targetType" value="&quot;PATH&quot;" />
    <method v="2" />
  </configuration>
</component></project>"#;
        let ws = read_workspace(text, Path::new(ROOT), &[&PythonImporter]).unwrap();
        assert_eq!(ws.configurations[0].unique_id, "Python tests.tests in api");
        let c = &ws.configurations[0].config;
        assert!(c.is_unsupported());
        assert_eq!(c.type_id, "idea:tests");
        let reference = c.reference.as_ref().unwrap();
        assert_eq!(reference.type_name.as_deref(), Some(TESTS_REFERENCE_NAME));
        assert_eq!(reference.factory.as_deref(), Some("py.test"));
        assert_eq!(
            pytest_options(&reference.xml),
            [
                ("keywords", ""),
                ("parameters", "-x"),
                ("additionalArguments", "--maxfail=1"),
                ("target", "$PROJECT_DIR$/tests"),
                ("targetType", "PATH"),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn launch_builds_the_reference_run() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".env"), "FROM_FILE=1\nOVERRIDDEN=file\n").unwrap();
        let mut config = new_configuration("server", Storage::Personal);
        let mut o = PythonOptions::read(&config);
        o.script = "app/server.py".into();
        o.parameters = "--config $PROJECT_DIR$/c.yml \"two words\"".into();
        o.interpreter_options = "-X dev".into();
        o.env_files = vec![".env".into()];
        o.redirect_input = true;
        o.input_file = "in.txt".into();
        o.write(&mut config);
        config
            .env
            .push(EnvVar::new("OVERRIDDEN", "explicit", false));
        config.env.push(EnvVar {
            name: "NOT_SET".into(),
            value: None,
            shared: false,
        });
        let sources = [root.join("src")];
        let cx = LaunchContext {
            root,
            source_roots: &sources,
            python: root.join(".venv/bin/python"),
            uv: None,
        };
        let launch = launch(&config, &cx).unwrap();
        let run = &launch.run;
        assert_eq!(run.target, Target::Script(root.join("app/server.py")));
        assert_eq!(run.working_dir, root);
        assert_eq!(
            run.script_args,
            [
                OsString::from("--config"),
                format!("{}/c.yml", root.display()).into(),
                "two words".into()
            ]
        );
        assert_eq!(
            run.interpreter_options,
            [OsString::from("-X"), "dev".into()]
        );
        assert_eq!(
            run.env,
            [
                ("FROM_FILE", "1"),
                ("OVERRIDDEN", "explicit"),
                ("PYTHONUNBUFFERED", "1"),
            ]
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        );
        assert_eq!(run.content_roots, [root.to_path_buf()]);
        assert_eq!(run.source_roots, sources);
        assert!(run.inherit_env);
        assert_eq!(launch.input_file, Some(root.join("in.txt")));

        // Module mode, roots off, no system environment.
        let mut o = PythonOptions::read(&config);
        o.module_mode = true;
        o.script = "pkg.main".into();
        o.add_content_roots = false;
        o.add_source_roots = false;
        o.env_files.clear();
        o.working_directory = "pkg".into();
        o.write(&mut config);
        config.include_system_env = false;
        let launch2 = super::launch(&config, &cx).unwrap();
        assert_eq!(launch2.run.target, Target::Module("pkg.main".into()));
        assert!(launch2.run.content_roots.is_empty() && launch2.run.source_roots.is_empty());
        assert!(!launch2.run.inherit_env);
        assert_eq!(launch2.run.working_dir, root.join("pkg"));
        let cmd = launch2.run.command_line();
        assert!(!cmd.inherit_env);
        let args: Vec<String> = cmd
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[..4], ["-X", "dev", "-m", "pkg.main"]);

        // A missing env file and an empty script are the reference's errors.
        let mut o = PythonOptions::read(&config);
        o.env_files = vec!["missing.env".into()];
        o.write(&mut config);
        assert!(
            super::launch(&config, &cx)
                .unwrap_err()
                .starts_with("File not found")
        );
        let mut empty = new_configuration("x", Storage::Personal);
        assert_eq!(
            super::launch(&empty, &cx).unwrap_err(),
            "Please specify a script name"
        );
        empty.set_bool(key::MODULE_MODE, true);
        assert_eq!(
            super::launch(&empty, &cx).unwrap_err(),
            "Please specify a qualified name of the module"
        );
    }

    #[test]
    fn the_python_form() {
        let e = PythonEditor;
        let mut c = e.create("server", Storage::Personal);
        let ids = |c: &RunConfiguration, shown: &BTreeSet<String>| -> Vec<String> {
            e.fields(c, shown).into_iter().map(|f| f.id).collect()
        };
        let mut shown = BTreeSet::new();
        assert_eq!(
            ids(&c, &shown),
            [
                "interpreter",
                "target",
                "script",
                "parameters",
                "working-directory",
                "env"
            ]
        );
        assert_eq!(
            e.validate(&c).as_deref(),
            Some("Please specify a script name")
        );
        e.set(&mut c, field::SCRIPT, "app\\main.py");
        assert_eq!(PythonOptions::read(&c).script, "app/main.py");
        assert_eq!(e.validate(&c), None);
        // The interpreter: a name, a path, or the project's.
        e.set(&mut c, field::INTERPRETER, "uv (demo)");
        let o = PythonOptions::read(&c);
        assert!(!o.use_project_interpreter);
        assert_eq!(o.interpreter_name, "uv (demo)");
        e.set(&mut c, field::INTERPRETER, ".venv/bin/python");
        let o = PythonOptions::read(&c);
        assert_eq!(
            (o.interpreter.as_str(), o.interpreter_name.as_str()),
            (".venv/bin/python", "")
        );
        e.set(&mut c, field::INTERPRETER, "");
        assert!(PythonOptions::read(&c).use_project_interpreter);
        // Field options show their fields; flags flip.
        e.toggle(&mut c, field::INTERPRETER_OPTIONS, &mut shown);
        assert!(ids(&c, &shown).contains(&"interpreter-options".to_owned()));
        e.toggle(&mut c, field::INTERPRETER_OPTIONS, &mut shown);
        assert!(!ids(&c, &shown).contains(&"interpreter-options".to_owned()));
        e.toggle(&mut c, field::REDIRECT_INPUT, &mut shown);
        assert!(ids(&c, &shown).contains(&"input-file".to_owned()));
        e.toggle(&mut c, field::RUN_WITH_UV, &mut shown);
        assert_eq!(PythonOptions::read(&c).run_with_uv, Some(false));
        let options = e.options(&c, &shown);
        assert!(options.iter().any(|o| o.id == "run-with-uv" && !o.checked));
        assert!(options.iter().any(|o| o.id == "allow-parallel"));
        e.set(&mut c, field::TARGET, "module");
        e.set(&mut c, field::SCRIPT, "");
        assert_eq!(
            e.validate(&c).as_deref(),
            Some("Please specify a qualified name of the module")
        );
    }

    #[test]
    fn a_context_configuration_runs_the_file_from_its_folder() {
        let root = Path::new("/work/demo");
        let c = context_configuration(&root.join("app/main.py"), root).unwrap();
        assert_eq!(c.name, "main");
        assert!(c.name_is_generated);
        let o = PythonOptions::read(&c);
        assert_eq!(o.script, "app/main.py");
        assert_eq!(o.working_directory, "app");
        assert!(o.use_project_interpreter);
        let top = context_configuration(&root.join("main.py"), root).unwrap();
        assert_eq!(PythonOptions::read(&top).working_directory, ".");
        assert!(same_context(&c, &c));
        assert!(!same_context(&c, &top));
        assert!(context_configuration(Path::new("/elsewhere/x.py"), root).is_none());
    }
}
