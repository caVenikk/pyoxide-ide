//! A run provider for tests: runs `.py` files as shell scripts (the
//! script comes from [`set_script`], not from the file), with a link filter
//! for `at <path>:<line>` and the URL filter. Its run configuration type,
//! "Script" (the reference's `ScriptType` in test `.idea` folders), runs
//! the configuration's `COMMAND` the same way.

#![allow(dead_code)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use console::{Link, LinkFilter, LinkTarget};
use gpui::{App, Task, TestAppContext};
use process::CommandLine;
use run_config::{IdeaConfiguration, Importer, RunConfiguration, TypeInfo};
use ui::icons::IconName;
use workbench::{RunHook, RunProvider, RunRequest, RunSpec};

thread_local! {
    static SCRIPT: RefCell<String> = const { RefCell::new(String::new()) };
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

    fn prepare_configuration(
        &self,
        request: RunRequest,
        _: &mut App,
    ) -> Task<Result<RunSpec, String>> {
        let config = request.configuration.clone().expect("a configuration");
        let script = config.str_option("command").to_owned();
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
