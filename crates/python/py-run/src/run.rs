use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use console::{LinkFilter, UrlFilter};
use process::CommandLine;

use crate::TracebackFilter;

/// Running through `uv run`, which the reference does by default for an
/// interpreter of a uv-managed environment (`UvRunToolProvider`, "Run with
/// `uv run`" on): the process is `uv run <python> <script>` with
/// `VIRTUAL_ENV` and `UV_PROJECT_ENVIRONMENT` set to the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvRun {
    /// The uv executable.
    pub uv: PathBuf,
    /// The virtual environment directory (the interpreter's `sys.prefix`).
    pub environment: PathBuf,
}

impl UvRun {
    /// uv for `environment`, found the way the IDE finds uv elsewhere
    /// (`PYOXIDE_UV`, `PATH`, the installers' locations); `None` when uv is
    /// not installed. Looks at the file system: call it off the UI thread.
    pub fn locate(environment: PathBuf) -> Option<UvRun> {
        let uv = py_env::Uv::locate(py_env::UvConfig::default())?;
        Some(UvRun {
            uv: uv.program().to_path_buf(),
            environment,
        })
    }
}

/// A Python script run: the inputs of the reference's Python run
/// configuration that "Current File" uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonRun {
    /// The interpreter (for a virtual environment its `bin/python` or
    /// `Scripts\python.exe`, not the resolved base interpreter).
    pub python: PathBuf,
    /// Run through `uv run`; `None` runs the interpreter directly.
    pub uv: Option<UvRun>,
    /// The script, an absolute path.
    pub script: PathBuf,
    /// Arguments after the script ("Script parameters"; empty for Current
    /// File).
    pub script_args: Vec<OsString>,
    /// Arguments before the script ("Interpreter options"; empty for Current
    /// File).
    pub interpreter_options: Vec<OsString>,
    /// The working directory; for Current File the script's folder.
    pub working_dir: PathBuf,
    /// Content roots added to `PYTHONPATH` ("Add content roots to
    /// PYTHONPATH"; for a plain project the project folder).
    pub content_roots: Vec<PathBuf>,
    /// Source roots added to `PYTHONPATH` after the content roots ("Add
    /// source roots to PYTHONPATH").
    pub source_roots: Vec<PathBuf>,
    /// The configuration's environment variables, in order.
    pub env: Vec<(OsString, OsString)>,
    /// "Include system environment variables".
    pub inherit_env: bool,
}

impl PythonRun {
    /// The reference's Current File run of `script` with `python`: working
    /// directory = the script's folder, no arguments, the system
    /// environment included, no roots (set
    /// [`PythonRun::content_roots`] and [`PythonRun::source_roots`] from the
    /// project).
    pub fn current_file(python: impl Into<PathBuf>, script: impl Into<PathBuf>) -> Self {
        let script = script.into();
        let working_dir = script
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        PythonRun {
            python: python.into(),
            uv: None,
            script,
            script_args: Vec::new(),
            interpreter_options: Vec::new(),
            working_dir,
            content_roots: Vec::new(),
            source_roots: Vec::new(),
            env: Vec::new(),
            inherit_env: true,
        }
    }

    /// The process to start. Reads the IDE's environment (`PYTHONPATH`,
    /// `PATH`) to merge it as the reference does.
    pub fn command_line(&self) -> CommandLine {
        todo!("PythonRun::command_line")
    }

    /// The first console line exactly as the reference prints it, without
    /// the line break (the reference joins the quoted command line and adds
    /// a trailing space).
    pub fn display(&self) -> String {
        todo!("PythonRun::display")
    }

    /// The console's link filters for this run, as the reference installs
    /// them: the traceback filter (relative paths resolved against the
    /// working directory) and the URL filter.
    pub fn link_filters(&self) -> Vec<Arc<dyn LinkFilter>> {
        vec![
            Arc::new(TracebackFilter::new(Some(self.working_dir.clone()))),
            Arc::new(UrlFilter),
        ]
    }
}
