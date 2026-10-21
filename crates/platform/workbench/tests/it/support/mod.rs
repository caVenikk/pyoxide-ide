//! A run provider for tests: runs `.py` files as shell scripts (the
//! script comes from [`set_script`], not from the file), with a link filter
//! for `at <path>:<line>` and the URL filter. Its run configuration type,
//! "Script" (the reference's `ScriptType` in test `.idea` folders), runs
//! the configuration's `COMMAND` the same way.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use console::{Link, LinkFilter, LinkTarget};
use gpui::{App, Task, TestAppContext};
use process::CommandLine;
use run_config::form::{self, Field, FieldKind, OptionItem, TypeEditor};
use run_config::{IdeaConfiguration, Importer, RunConfiguration, Storage, TypeInfo};
use ui::icons::IconName;
use workbench::{RunHook, RunProvider, RunRequest, RunSpec};

thread_local! {
    static SCRIPT: RefCell<String> = const { RefCell::new(String::new()) };
    static DEBUG_SCRIPTS: RefCell<VecDeque<Vec<serde_json::Value>>> =
        const { RefCell::new(VecDeque::new()) };
}

/// The script of the adapter of a coming debug session (`dap_client::fake`),
/// after the ones pushed before. (The test's executor prepares sessions on
/// its own thread.)
pub fn push_debug_script(script: Vec<serde_json::Value>) {
    DEBUG_SCRIPTS.with(|s| s.borrow_mut().push_back(script));
}

/// Requests a scripted adapter drops unanswered: breakpoints, and what the
/// frames and variables views ask for.
const DEBUG_IGNORED: [&str; 5] = [
    "setBreakpoints",
    "setExceptionBreakpoints",
    "scopes",
    "variables",
    "source",
];

/// A scripted adapter's start: its capabilities, `launch`, the
/// configuration, then the program started with `runInTerminal` in `cwd`
/// (`program`: the command and its arguments; none without).
pub fn debug_start(cwd: &Path, program: Option<Vec<String>>) -> Vec<serde_json::Value> {
    use serde_json::json;
    let mut steps = vec![
        json!({"ignore": DEBUG_IGNORED}),
        json!({"expect": "initialize"}),
        json!({"respond": {"supportsConfigurationDoneRequest": true,
            "supportsTerminateRequest": true}}),
        json!({"expect": "launch"}),
        json!({"event": "initialized"}),
        json!({"expect": "configurationDone"}),
        json!({"respond": null}),
        json!({"respond": null, "to": "launch"}),
    ];
    if let Some(args) = program {
        steps.push(
            json!({"request": "runInTerminal", "arguments": {"kind": "integrated",
            "cwd": cwd, "args": args, "env": {}}}),
        );
        steps.push(json!({"expect_response": "runInTerminal", "success": true}));
    }
    steps
}

/// A stop of thread 1 at a line of a file (from 1): the stop, the threads
/// and its one frame.
pub fn debug_stop(file: &Path, line: u32, reason: &str) -> Vec<serde_json::Value> {
    use serde_json::json;
    vec![
        json!({"event": "stopped", "body": {"reason": reason, "threadId": 1}}),
        json!({"expect": "threads"}),
        json!({"respond": {"threads": [{"id": 1, "name": "MainThread"}]}}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 1}}),
        json!({"respond": {"stackFrames": [{"id": 100 + line, "name": "<module>",
            "line": line, "column": 1, "source": {"path": file}}], "totalFrames": 1}}),
    ]
}

/// A request of thread 1 (`next`, `continue`...) answered.
pub fn debug_answer(command: &str) -> Vec<serde_json::Value> {
    use serde_json::json;
    vec![
        json!({"expect": command, "arguments": {"threadId": 1}}),
        json!({"respond": null}),
    ]
}

/// The program's end as the adapter reports it.
pub fn debug_end(code: i32) -> Vec<serde_json::Value> {
    use serde_json::json;
    vec![
        json!({"event": "exited", "body": {"exitCode": code}}),
        json!({"event": "terminated"}),
    ]
}

/// A program that prints a line and ends at once on every OS (the
/// system's shell: it ends well within the session's wait for the
/// program after the adapter's end, even on a busy machine).
pub fn quick_program() -> Vec<String> {
    if cfg!(windows) {
        ["cmd", "/C", "echo debugged"].map(str::to_owned).to_vec()
    } else {
        ["/bin/sh", "-c", "echo debugged"]
            .map(str::to_owned)
            .to_vec()
    }
}

/// A program that runs until it is stopped.
pub fn long_program() -> Vec<String> {
    if cfg!(windows) {
        ["cmd", "/C", "ping -n 30 127.0.0.1 >NUL"]
            .map(str::to_owned)
            .to_vec()
    } else {
        ["/bin/sh", "-c", "sleep 30"].map(str::to_owned).to_vec()
    }
}

/// The script the next run runs (`sh -c` on Unix, `cmd /V:ON /C` on
/// Windows).
pub fn set_script(script: &str) {
    SCRIPT.with(|s| *s.borrow_mut() = script.to_owned());
}

/// A script per platform.
pub fn script(unix: &str, windows: &str) -> String {
    if cfg!(windows) {
        windows.to_owned()
    } else {
        unix.to_owned()
    }
}

/// A script that runs until it is stopped.
pub fn long_running() -> String {
    script(
        "echo started; sleep 30",
        "echo started & ping -n 30 127.0.0.1 >NUL",
    )
}

/// `at <path>:<line>` is a file link.
struct AtFilter;

impl LinkFilter for AtFilter {
    fn links(&self, line: &str, out: &mut Vec<Link>) {
        let Some(at) = line.find("at ") else {
            return;
        };
        let rest = &line[at + 3..];
        let Some(colon) = rest.rfind(':') else {
            return;
        };
        let Ok(number) = rest[colon + 1..].trim().parse::<u32>() else {
            return;
        };
        out.push(Link {
            range: at + 3..at + 3 + rest.trim_end().len(),
            target: LinkTarget::File {
                path: PathBuf::from(&rest[..colon]),
                line: Some(number),
                column: None,
            },
        });
    }
}

pub struct ShellRunner;

impl RunProvider for ShellRunner {
    fn can_run(&self, file: &Path, _: &App) -> bool {
        file.extension().is_some_and(|e| e == "py")
    }

    fn prepare(&self, request: RunRequest, _: &mut App) -> Task<Result<RunSpec, String>> {
        let script = SCRIPT.with(|s| s.borrow().clone());
        if script.is_empty() {
            return Task::ready(Err("Please select a valid Python interpreter".to_owned()));
        }
        let command = if cfg!(windows) {
            CommandLine::new("cmd").args(["/V:ON", "/C", &script])
        } else {
            CommandLine::new("/bin/sh").args(["-c", &script])
        };
        let mut command = command;
        command.cwd = request.file.parent().map(Path::to_path_buf);
        let title = request
            .file
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let display = format!("{} ", command.display());
        Task::ready(Ok(RunSpec::new(
            title,
            IconName::RUN,
            command,
            display,
            vec![Arc::new(AtFilter), Arc::new(console::UrlFilter)],
        )))
    }

    fn configuration_types(&self) -> Vec<TypeInfo> {
        vec![SCRIPT_TYPE]
    }

    fn importers(&self) -> Vec<Arc<dyn Importer>> {
        vec![Arc::new(ScriptImporter)]
    }

    fn editors(&self) -> Vec<Arc<dyn TypeEditor>> {
        vec![Arc::new(ScriptEditor)]
    }

    /// A `.py` file runs from its context as a "Script" configuration named
    /// after the file, without a command: it runs the script set when it
    /// runs, as the file would.
    fn context_configuration(&self, file: &Path, root: &Path) -> Option<RunConfiguration> {
        if file.extension().is_none_or(|e| e != "py") {
            return None;
        }
        let relative = file.strip_prefix(root).ok()?;
        let mut config =
            ScriptEditor.create(&file.file_stem()?.to_string_lossy(), Storage::Personal);
        config.name_is_generated = true;
        config.set_str("file", &relative.to_string_lossy().replace('\\', "/"));
        Some(config)
    }

    fn same_context(&self, existing: &RunConfiguration, context: &RunConfiguration) -> bool {
        existing.type_id == context.type_id
            && existing.str_option("file") == context.str_option("file")
    }

    fn prepare_configuration(
        &self,
        request: RunRequest,
        _: &mut App,
    ) -> Task<Result<RunSpec, String>> {
        let config = request.configuration.clone().expect("a configuration");
        let script = match config.str_option("command") {
            "" => SCRIPT.with(|s| s.borrow().clone()),
            command => command.to_owned(),
        };
        let command = if cfg!(windows) {
            CommandLine::new("cmd").args(["/V:ON", "/C", &script])
        } else {
            CommandLine::new("/bin/sh").args(["-c", &script])
        };
        let mut command = command;
        command.cwd = request.project_root.clone();
        for (name, value) in config.env_values() {
            command = command.with_env(name, value);
        }
        let display = format!("{} ", command.display());
        Task::ready(Ok(RunSpec::new(
            config.name.clone(),
            IconName::RUN,
            command,
            display,
            vec![Arc::new(console::UrlFilter)],
        )))
    }
}

/// The test product's run configuration type.
pub const SCRIPT_TYPE: TypeInfo = TypeInfo {
    id: "script",
    name: "Script",
    icon: "run/run",
    reference_id: "ScriptType",
    allow_parallel: false,
};

/// The form of the "Script" type: its command and its variables.
pub struct ScriptEditor;

impl TypeEditor for ScriptEditor {
    fn type_id(&self) -> &'static str {
        "script"
    }

    fn create(&self, name: &str, storage: Storage) -> RunConfiguration {
        RunConfiguration::new("script", name, storage)
    }

    fn fields(
        &self,
        config: &RunConfiguration,
        _: &std::collections::BTreeSet<String>,
    ) -> Vec<Field> {
        vec![
            Field::new(
                "command",
                "Command:",
                FieldKind::Text,
                config.str_option("command"),
            ),
            Field::new(
                form::common::ENV,
                "Environment variables:",
                FieldKind::Env,
                form::env_text(&config.env),
            ),
        ]
    }

    fn options(
        &self,
        config: &RunConfiguration,
        _: &std::collections::BTreeSet<String>,
    ) -> Vec<OptionItem> {
        form::common_options(config, false)
    }

    fn set(&self, config: &mut RunConfiguration, field: &str, value: &str) -> bool {
        if field != "command" {
            return false;
        }
        config.set_str("command", value);
        true
    }

    fn toggle(
        &self,
        config: &mut RunConfiguration,
        option: &str,
        _: &mut std::collections::BTreeSet<String>,
    ) {
        form::toggle_common(config, option, false);
    }

    fn validate(&self, config: &RunConfiguration) -> Option<String> {
        config
            .str_option("command")
            .is_empty()
            .then(|| "Please specify a command".to_owned())
    }
}

/// Imports the reference's `ScriptType` configurations: `COMMAND` and the
/// variables.
struct ScriptImporter;

impl Importer for ScriptImporter {
    fn type_name(&self, reference_type: &str) -> Option<&'static str> {
        (reference_type == "ScriptType").then_some("Script")
    }

    fn convert(
        &self,
        idea: &IdeaConfiguration<'_>,
        _: &Path,
        config: &mut RunConfiguration,
    ) -> bool {
        if idea.type_id() != "ScriptType" {
            return false;
        }
        config.type_id = "script".into();
        config.set_str("command", idea.field("COMMAND").unwrap_or(""));
        let shared = config.is_shared();
        config.env = idea
            .envs()
            .into_iter()
            .map(|(n, v)| run_config::EnvVar::new(n, v, shared))
            .collect();
        true
    }
}

pub fn run_hook() -> RunHook {
    RunHook::new(ShellRunner)
}

/// The debugger of tests: the Python breakpoint types' names, line
/// breakpoints on the lines of `.py` files that are neither blank nor
/// comments, and an adapter that runs the next script of
/// [`push_debug_script`] (none without one).
pub struct TestDebug;

impl debugger::DebugAdapterProvider for TestDebug {
    fn prepare(&self, request: &debugger::DebugRequest) -> Result<debugger::AdapterLaunch, String> {
        let script = DEBUG_SCRIPTS
            .with(|s| s.borrow_mut().pop_front())
            .ok_or_else(|| "no debug adapter in tests".to_owned())?;
        Ok(debugger::AdapterLaunch {
            title: request.configuration.name.clone(),
            adapter: debugger::Adapter::Transport(dap_client::fake::adapter(script)),
            adapter_id: "fake".to_owned(),
            kind: debugger::LaunchKind::Launch,
            arguments: serde_json::json!({"program": request.configuration.name}),
            input_file: None,
            evaluation_timeout: None,
        })
    }

    fn default_exception_breakpoints(&self) -> Vec<debugger::ExceptionBreakpoint> {
        let mut any = debugger::ExceptionBreakpoint::new("python-exception", "BaseException");
        any.properties
            .insert("notifyOnTerminate".to_owned(), "true".to_owned());
        vec![any]
    }

    fn line_breakpoint_type(&self) -> Option<debugger::LineBreakpointType> {
        Some(debugger::LineBreakpointType {
            id: "python-line".to_owned(),
            title: "Python Line Breakpoint".to_owned(),
            default_suspend: debugger::SuspendPolicy::Thread,
            suspend_thread: true,
        })
    }

    fn stoppable_lines(&self, file: &Path, text: &str) -> Option<Vec<bool>> {
        (file.extension().is_some_and(|e| e == "py")).then(|| {
            text.split('\n')
                .map(|l| {
                    let t = l.trim();
                    !t.is_empty() && !t.starts_with('#')
                })
                .collect()
        })
    }

    fn exception_breakpoint_type(&self) -> Option<debugger::ExceptionBreakpointType> {
        let option = |key: &str, label: &str| debugger::ExceptionOption {
            key: key.to_owned(),
            label: label.to_owned(),
        };
        Some(debugger::ExceptionBreakpointType {
            id: "python-exception".to_owned(),
            title: "Python Exception Breakpoint".to_owned(),
            any_class: "BaseException".to_owned(),
            any_name: "Any exception".to_owned(),
            options_title: "Activation policy".to_owned(),
            options: vec![
                option("notifyOnTerminate", "On termination"),
                option("notifyOnlyOnFirst", "On raise"),
                option("ignoreLibraries", "Ignore library files"),
            ],
            chooser_title: "Select Exception Class".to_owned(),
        })
    }

    fn exception_classes(&self) -> Vec<String> {
        ["KeyError", "ValueError"].map(str::to_owned).to_vec()
    }

    /// The first `name =` of the file.
    fn value_declaration(&self, text: &str, _line: u32, name: &str) -> Option<(u32, u32)> {
        let assignment = format!("{name} =");
        text.lines().enumerate().find_map(|(i, l)| {
            l.find(&assignment)
                .map(|column| (i as u32 + 1, column as u32 + 1))
        })
    }
}

pub fn debug_hook() -> workbench::DebugHook {
    workbench::DebugHook::new(TestDebug)
}

/// Lets real processes and threads report to the test: the executor may
/// park while they work.
pub fn allow_processes(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
}

/// One step of waiting for a process: the clock moves a frame (the
/// console's flush timer) and real time passes for the process.
pub fn tick(cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_millis(20));
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(5));
}
