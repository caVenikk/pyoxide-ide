//! The debugpy adapter and its `launch` arguments for a Python run
//! (ADR 0015; debugpy 1.8.21 `adapter/clients.py` `launch_request`,
//! `adapter/launchers.py`, `launcher/handlers.py`).
//!
//! The adapter is `python <helper>/debugpy/adapter` with the run's
//! interpreter. With `console: integratedTerminal` it asks the IDE to
//! start the launcher (`runInTerminal`), whose environment is the IDE's;
//! the launcher starts the program with `python`, `pythonArgs`, the program
//! or module and `args` in `cwd`, its own environment updated by `env`
//! (`null` removes a variable). So `env` carries the run's environment,
//! and for a run without the system environment it removes every other
//! variable of the IDE.
//!
//! A run through `uv run` is debugged with its environment's interpreter
//! directly, as the environment is already activated for it.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use process::{CommandLine, ProcessEvent};
use py_run::{PythonRun, Target};
use serde_json::{Map, Value, json};

use crate::helper::{DEBUGPY_VERSION, Helper};

/// The oldest Python debugpy 1.8.21 debugs (its `requires_python`).
pub const MIN_PYTHON: (u32, u32) = (3, 8);

/// An evaluation without a frame (run in pydevd's own frame, whatever the
/// program's names are) of `body`, a function of pydevd's debugger `d`.
fn with_pydevd(body: &str) -> (String, Value) {
    (
        "evaluate".to_owned(),
        json!({
            "expression": format!(
                "(lambda d: {body})(__import__('sys').modules['pydevd'].get_global_debugger())"
            ),
            "context": "repl",
        }),
    )
}

/// Where the library filter's value before Step Into My Code is kept in
/// pydevd's debugger.
const KEPT_FILTER: &str = "_pyoxide_libraries_filter";

/// Step Into My Code with debugpy: pydevd's `stepIn` is a step into the
/// program's own code while its library filter is on
/// (`pydevd_process_net_command_json.on_stepin_request`); the filter is on
/// for the one step, then back to what it was ("Debug just my code" keeps
/// it on for the whole session).
pub fn step_into_my_code() -> debugger::StepIntoMyCode {
    debugger::StepIntoMyCode {
        before: with_pydevd(&format!(
            "vars(d).setdefault('{KEPT_FILTER}', d.get_use_libraries_filter()) is None \
             or d.set_use_libraries_filter(True)"
        )),
        after: with_pydevd(&format!(
            "d.set_use_libraries_filter(vars(d).pop('{KEPT_FILTER}', False))"
        )),
    }
}

/// What [`task_steps`] runs in pydevd.
const TASK_STEPS: &str = include_str!("task_steps.py");

/// Steps that keep to their asyncio task, sent to each debugged process
/// before its first step: pydevd keeps a step that starts in a coroutine
/// in it with Python's trace function, but not in its `sys.monitoring`
/// mode (Python 3.12 and later), where a step over an `await` stops in
/// whatever task of the event loop runs next. `task_steps.py` adds that
/// to pydevd's debugger in the process (its answer: `'installed'`, or
/// `'trace'` when pydevd traces and keeps to the coroutine itself).
pub fn task_steps() -> (String, Value) {
    let source = serde_json::to_string(TASK_STEPS).expect("a string is JSON");
    with_pydevd(&format!(
        "(lambda ns: exec({source}, ns) or ns['result'])({{'d': d}})"
    ))
}

/// The Python debugger options that change a launch (the reference's
/// `PyDebuggerSettings` and `PyDebuggerOptionsProvider`), with the
/// reference's defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugOptions {
    /// "Show return values" (`WatchReturnValues`, off).
    pub show_return_values: bool,
    /// "Simplified Variables View" (`SimplifiedView`, on): functions and
    /// classes are hidden.
    pub simplified_view: bool,
    /// "Gevent compatible" (off).
    pub gevent: bool,
    /// "PyQt compatible" (on) and its backend (`auto`, `pyqt5`, `pyside2`,
    /// ...).
    pub qt: Option<String>,
    /// "Enable Debugpy Logging" (off): the folder for debugpy's logs.
    pub log_dir: Option<PathBuf>,
    /// "Attach to subprocess automatically while debugging"
    /// (`myAttachToSubprocess`, on): debugpy debugs the Python processes
    /// the program starts (`subprocess`, `multiprocessing`, `os.exec*`),
    /// each a child session of the debug session.
    pub attach_to_subprocess: bool,
    /// "Debugger evaluation response timeout (ms)"
    /// (`myEvaluationResponseTimeout`, 60000): how long an evaluation
    /// waits for its answer; `None` (0 ms) waits as long as the session
    /// lives.
    pub evaluation_timeout: Option<Duration>,
}

/// The reference's default of the evaluation response timeout.
pub const DEFAULT_EVALUATION_TIMEOUT: Duration = Duration::from_millis(60_000);

impl Default for DebugOptions {
    fn default() -> Self {
        Self {
            show_return_values: false,
            simplified_view: true,
            gevent: false,
            qt: Some("auto".to_owned()),
            log_dir: None,
            attach_to_subprocess: true,
            evaluation_timeout: Some(DEFAULT_EVALUATION_TIMEOUT),
        }
    }
}

/// debugpy's names of the Qt backends.
const QT_BACKENDS: &[&str] = &["auto", "pyside", "pyside2", "pyqt4", "pyqt5"];

/// The adapter's process: `python <helper>/debugpy/adapter`, with
/// `--log-dir` when logging is on.
pub fn adapter_command(python: &Path, helper: &Helper, options: &DebugOptions) -> CommandLine {
    let mut command = CommandLine::new(python).arg(helper.adapter());
    if let Some(dir) = &options.log_dir {
        command = command.arg("--log-dir").arg(dir);
    }
    command
}

/// The run as debugged: through `uv run` it becomes a run of the
/// environment's interpreter.
pub fn debugged_run(run: &PythonRun) -> PythonRun {
    let mut run = run.clone();
    run.uv = None;
    run
}

/// The `launch` arguments of a run (`name` is the session's).
pub fn launch_arguments(
    run: &PythonRun,
    name: &str,
    just_my_code: bool,
    options: &DebugOptions,
) -> Value {
    launch_arguments_with(run, name, just_my_code, options, std::env::vars_os())
}

/// [`launch_arguments`] with the IDE's environment given.
pub fn launch_arguments_with(
    run: &PythonRun,
    name: &str,
    just_my_code: bool,
    options: &DebugOptions,
    ide_env: impl IntoIterator<Item = (OsString, OsString)>,
) -> Value {
    let run = debugged_run(run);
    let command = run.command_line();
    let mut arguments = Map::new();
    arguments.insert("name".into(), json!(name));
    arguments.insert("python".into(), json!([text(run.python.as_os_str())]));
    if !run.interpreter_options.is_empty() {
        arguments.insert("pythonArgs".into(), texts(&run.interpreter_options));
    }
    match &run.target {
        Target::Script(script) => {
            arguments.insert("program".into(), json!(text(script.as_os_str())));
        }
        Target::Module(module) => {
            arguments.insert("module".into(), json!(module));
        }
    }
    arguments.insert("args".into(), texts(&run.script_args));
    arguments.insert("cwd".into(), json!(text(run.working_dir.as_os_str())));
    arguments.insert("env".into(), Value::Object(env(&command, ide_env)));
    arguments.insert("console".into(), json!("integratedTerminal"));
    arguments.insert("redirectOutput".into(), json!(false));
    arguments.insert("justMyCode".into(), json!(just_my_code));
    arguments.insert("showReturnValue".into(), json!(options.show_return_values));
    arguments.insert("subProcess".into(), json!(options.attach_to_subprocess));
    arguments.insert("stopOnEntry".into(), json!(false));
    arguments.insert(
        "variablePresentation".into(),
        variable_presentation(options.simplified_view),
    );
    if options.gevent {
        arguments.insert("gevent".into(), json!(true));
    }
    let qt = options
        .qt
        .as_deref()
        .map(str::to_ascii_lowercase)
        .filter(|b| QT_BACKENDS.contains(&b.as_str()))
        .unwrap_or_else(|| "none".to_owned());
    arguments.insert("qt".into(), json!(qt));
    Value::Object(arguments)
}

/// How long connecting to the adapter for a subprocess may take.
pub const CHILD_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Where the adapter waits for the client of a subprocess: the `connect`
/// of the child's configuration (debugpy 1.8.21 `adapter/clients.py`
/// `notify_of_subprocess`: the address the adapter listens on for
/// clients).
pub fn child_address(configuration: &Value) -> Result<(String, u16), String> {
    let connect = &configuration["connect"];
    let host = connect["host"].as_str().unwrap_or("127.0.0.1");
    let port = connect["port"]
        .as_u64()
        .and_then(|p| u16::try_from(p).ok())
        .ok_or_else(|| "No debugger address for the subprocess".to_owned())?;
    Ok((host.to_owned(), port))
}

/// How pydevd presents variables: special variables (`__x__`) in their
/// group as the reference shows them, the rest in place; the simplified
/// view hides functions and classes.
pub fn variable_presentation(simplified: bool) -> Value {
    if simplified {
        json!({"all": "inline", "special": "group", "function": "hide", "class": "hide"})
    } else {
        json!({"all": "inline", "special": "group"})
    }
}

/// The run's environment as `env`: its variables over the launcher's (the
/// IDE's), with `null` for those it removes and, without the system
/// environment, for every other variable of the IDE. Names differing only
/// in case are one variable on Windows (the launcher refuses two of them).
fn env(
    command: &CommandLine,
    ide_env: impl IntoIterator<Item = (OsString, OsString)>,
) -> Map<String, Value> {
    let key = |name: &OsStr| {
        let name = text(name);
        if cfg!(windows) {
            name.to_ascii_uppercase()
        } else {
            name
        }
    };
    let mut out: BTreeMap<String, (String, Value)> = BTreeMap::new();
    if !command.inherit_env {
        for (name, _) in ide_env {
            out.insert(key(&name), (text(&name), Value::Null));
        }
    }
    for (name, value) in &command.env {
        let value = value.as_deref().map_or(Value::Null, |v| json!(text(v)));
        out.insert(key(name), (text(name), value));
    }
    out.into_values().collect()
}

fn text(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

fn texts(values: &[OsString]) -> Value {
    Value::Array(values.iter().map(|v| json!(text(v))).collect())
}

/// A Python version's major and minor number (`3.13.1`, `3.14`,
/// `3.15.0b3`, `Python 3.12.1`).
pub fn major_minor(version: &str) -> Option<(u32, u32)> {
    let version = version.trim();
    let version = version.strip_prefix("Python ").unwrap_or(version);
    let mut parts = version.split('.');
    let major = parts.next()?.trim().parse().ok()?;
    let minor: String = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((major, minor.parse().ok()?))
}

/// The interpreter can be debugged: Python 3.8 or later. The error names
/// the requirement, the version and the interpreter.
pub fn check_version(python: &Path, version: &str) -> Result<(), String> {
    match major_minor(version) {
        Some(found) if found >= MIN_PYTHON => Ok(()),
        Some(_) => Err(format!(
            "Cannot debug with Python {}: debugpy {DEBUGPY_VERSION} requires Python {}.{} or later ({})",
            version.trim(),
            MIN_PYTHON.0,
            MIN_PYTHON.1,
            python.display()
        )),
        None => Err(format!(
            "Cannot debug: the version of the Python interpreter {} is unknown ({})",
            python.display(),
            version.trim()
        )),
    }
}

/// How long the interpreter may take to tell its version.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Asks the interpreter for its version (`3.13.1`). Blocking: call it off
/// the UI thread.
pub fn probe_version(python: &Path) -> Result<String, String> {
    let command = CommandLine::new(python)
        .args(["-c", "import sys; print('%d.%d.%d' % sys.version_info[:3])"]);
    let (handle, events) = process::spawn(&command).map_err(|e| {
        format!(
            "Cannot run the Python interpreter {}: {e}",
            python.display()
        )
    })?;
    handle.close_stdin();
    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;
    let mut stdout = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match events.recv_timeout(left) {
            Ok(ProcessEvent::Stdout(chunk)) => stdout.extend_from_slice(&chunk),
            Ok(ProcessEvent::Exited(status)) => {
                let text = String::from_utf8_lossy(&stdout).trim().to_owned();
                if status.success() && major_minor(&text).is_some() {
                    return Ok(text);
                }
                return Err(format!(
                    "The Python interpreter {} did not tell its version",
                    python.display()
                ));
            }
            Ok(_) => {}
            Err(_) => {
                let _ = handle.kill();
                return Err(format!(
                    "The Python interpreter {} did not tell its version in time",
                    python.display()
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[cfg(not(windows))]
    use py_run::UvRun;

    use super::*;

    fn helper() -> (tempfile::TempDir, Helper) {
        let dir = tempfile::tempdir().unwrap();
        let adapter = dir.path().join("debugpy").join("adapter");
        std::fs::create_dir_all(&adapter).unwrap();
        std::fs::write(adapter.join("__main__.py"), "").unwrap();
        let info = dir
            .path()
            .join(format!("debugpy-{DEBUGPY_VERSION}.dist-info"));
        std::fs::create_dir_all(&info).unwrap();
        std::fs::write(info.join("RECORD"), "").unwrap();
        let helper = Helper::at(dir.path()).unwrap();
        (dir, helper)
    }

    fn no_env() -> Vec<(OsString, OsString)> {
        Vec::new()
    }

    #[cfg(not(windows))]
    #[test]
    fn a_script_run() {
        let mut run = PythonRun::current_file("/p/.venv/bin/python", "/p/pkg/main.py");
        run.script_args = vec!["--flag".into(), "two words".into()];
        run.interpreter_options = vec!["-X".into(), "dev".into()];
        run.content_roots = vec!["/p".into()];
        run.env = vec![("MY_VAR".into(), "x".into())];
        let args = launch_arguments_with(&run, "main", false, &DebugOptions::default(), no_env());
        assert_eq!(
            args,
            json!({
                "name": "main",
                "python": ["/p/.venv/bin/python"],
                "pythonArgs": ["-X", "dev"],
                "program": "/p/pkg/main.py",
                "args": ["--flag", "two words"],
                "cwd": "/p/pkg",
                "env": {
                    "MY_VAR": "x",
                    "PYCHARM_HOSTED": "1",
                    "PYTHONIOENCODING": "UTF-8",
                    "PYTHONPATH": "/p",
                    "PYTHONUNBUFFERED": "1",
                },
                "console": "integratedTerminal",
                "redirectOutput": false,
                "justMyCode": false,
                "showReturnValue": false,
                "subProcess": true,
                "stopOnEntry": false,
                "variablePresentation": {"all": "inline", "special": "group",
                    "function": "hide", "class": "hide"},
                "qt": "auto",
            })
        );
    }

    #[test]
    fn a_module_run_with_options() {
        let mut run = PythonRun::current_file("python", "main.py");
        run.target = Target::Module("pkg.tool".into());
        let options = DebugOptions {
            show_return_values: true,
            simplified_view: false,
            gevent: true,
            qt: None,
            log_dir: None,
            attach_to_subprocess: false,
            evaluation_timeout: None,
        };
        let args = launch_arguments_with(&run, "tool", true, &options, no_env());
        assert_eq!(args["module"], "pkg.tool");
        assert!(args.get("program").is_none());
        assert!(args.get("pythonArgs").is_none());
        assert_eq!(args["args"], json!([]));
        assert_eq!(args["justMyCode"], true);
        assert_eq!(args["showReturnValue"], true);
        assert_eq!(args["gevent"], true);
        assert_eq!(args["subProcess"], false);
        assert_eq!(args["qt"], "none");
        assert_eq!(
            args["variablePresentation"],
            json!({"all": "inline", "special": "group"})
        );
        let qt = DebugOptions {
            qt: Some("PyQt5".into()),
            ..DebugOptions::default()
        };
        assert_eq!(
            launch_arguments_with(&run, "tool", false, &qt, no_env())["qt"],
            "pyqt5"
        );
    }

    #[test]
    fn without_the_system_environment_the_ides_variables_are_removed() {
        let mut run = PythonRun::current_file("python", "main.py");
        run.inherit_env = false;
        run.env = vec![("KEEP".into(), "1".into())];
        let ide = vec![
            (OsString::from("HOME"), OsString::from("/home/u")),
            (OsString::from("KEEP"), OsString::from("0")),
        ];
        let args = launch_arguments_with(&run, "main", false, &DebugOptions::default(), ide);
        assert_eq!(args["env"]["HOME"], Value::Null);
        assert_eq!(args["env"]["KEEP"], "1");
        assert_eq!(args["env"]["PYTHONUNBUFFERED"], "1");
    }

    #[cfg(windows)]
    #[test]
    fn names_differing_in_case_are_one_variable_on_windows() {
        let mut run = PythonRun::current_file("python", "main.py");
        run.inherit_env = false;
        run.env = vec![("PATH".into(), r"C:\mine".into())];
        let ide = vec![(OsString::from("Path"), OsString::from(r"C:\Windows"))];
        let args = launch_arguments_with(&run, "main", false, &DebugOptions::default(), ide);
        let env = args["env"].as_object().unwrap();
        assert_eq!(
            env.keys()
                .filter(|k| k.eq_ignore_ascii_case("path"))
                .count(),
            1
        );
        assert_eq!(env["PATH"], r"C:\mine");
    }

    #[cfg(not(windows))]
    #[test]
    fn uv_run_debugs_the_environments_interpreter() {
        let dir = tempfile::tempdir().unwrap();
        let venv = dir.path().join(".venv");
        std::fs::create_dir_all(venv.join("bin")).unwrap();
        std::fs::write(venv.join("pyvenv.cfg"), "home = /usr/bin\n").unwrap();
        let python = venv.join("bin").join("python");
        let mut run = PythonRun::current_file(&python, dir.path().join("main.py"));
        run.uv = Some(UvRun {
            uv: "/opt/homebrew/bin/uv".into(),
            environment: venv.clone(),
        });
        let args = launch_arguments_with(&run, "main", false, &DebugOptions::default(), no_env());
        assert_eq!(args["python"], json!([python.to_string_lossy()]));
        assert_eq!(
            args["program"],
            json!(dir.path().join("main.py").to_string_lossy())
        );
        assert_eq!(args["env"]["VIRTUAL_ENV"], json!(venv.to_string_lossy()));
        assert!(args["env"].get("UV_PROJECT_ENVIRONMENT").is_none());
        assert!(
            args["env"]["PATH"]
                .as_str()
                .unwrap()
                .starts_with(&venv.join("bin").to_string_lossy().into_owned())
        );
    }

    #[test]
    fn the_adapter_runs_with_the_interpreter() {
        let (_dir, helper) = helper();
        let command = adapter_command(Path::new("python3"), &helper, &DebugOptions::default());
        assert_eq!(command.program, PathBuf::from("python3"));
        assert_eq!(command.args, vec![helper.adapter().into_os_string()]);
        assert!(command.inherit_env);
        let logging = DebugOptions {
            log_dir: Some("/tmp/logs".into()),
            ..DebugOptions::default()
        };
        let command = adapter_command(Path::new("python3"), &helper, &logging);
        assert_eq!(
            &command.args[1..],
            ["--log-dir", "/tmp/logs"].map(OsString::from)
        );
    }

    #[test]
    fn the_address_of_a_subprocess() {
        let configuration = json!({
            "name": "Subprocess 4242",
            "subProcessId": 4242,
            "connect": {"host": "127.0.0.1", "port": 50123},
        });
        assert_eq!(
            child_address(&configuration),
            Ok(("127.0.0.1".to_owned(), 50123))
        );
        assert_eq!(
            child_address(&json!({"connect": {"port": 7}})),
            Ok(("127.0.0.1".to_owned(), 7))
        );
        assert!(child_address(&json!({"subProcessId": 1})).is_err());
        assert!(child_address(&json!({"connect": {"port": 70000}})).is_err());
    }

    /// The source runs in pydevd's debugger as a Python string literal (a
    /// JSON string is one) and answers with its `result`.
    #[test]
    fn task_steps_run_their_source_in_pydevd() {
        let (command, arguments) = task_steps();
        assert_eq!(command, "evaluate");
        assert_eq!(arguments["context"], "repl");
        assert!(arguments.get("frameId").is_none());
        let expression = arguments["expression"].as_str().unwrap();
        let literal = serde_json::to_string(TASK_STEPS).unwrap();
        assert_eq!(
            expression,
            format!(
                "(lambda d: (lambda ns: exec({literal}, ns) or ns['result'])({{'d': d}}))\
                 (__import__('sys').modules['pydevd'].get_global_debugger())"
            )
        );
        assert!(!literal.contains("\\/"), "Python keeps the backslash");
        for result in ["result = \"trace\"", "result = \"installed\""] {
            assert!(TASK_STEPS.contains(result), "{result}");
        }
    }

    #[test]
    fn versions() {
        assert_eq!(major_minor("3.13.1"), Some((3, 13)));
        assert_eq!(major_minor("3.14"), Some((3, 14)));
        assert_eq!(major_minor("3.15.0b3"), Some((3, 15)));
        assert_eq!(major_minor("3.10rc1"), Some((3, 10)));
        assert_eq!(major_minor("Python 3.8.20\n"), Some((3, 8)));
        assert_eq!(major_minor("3"), None);
        assert_eq!(major_minor(""), None);
        let python = Path::new("/usr/bin/python3");
        assert!(check_version(python, "3.8.0").is_ok());
        assert!(check_version(python, "3.13").is_ok());
        assert!(check_version(python, "4.0").is_ok());
        let old = check_version(python, "3.7.17").unwrap_err();
        assert_eq!(
            old,
            "Cannot debug with Python 3.7.17: debugpy 1.8.21 requires Python 3.8 or later (/usr/bin/python3)"
        );
        assert!(check_version(python, "2.7.18").is_err());
        assert!(
            check_version(python, "junk")
                .unwrap_err()
                .contains("unknown")
        );
    }

    #[test]
    fn a_missing_interpreter_is_reported() {
        let error = probe_version(Path::new("/no/such/python")).unwrap_err();
        assert!(error.contains("/no/such/python"), "{error}");
    }
}
