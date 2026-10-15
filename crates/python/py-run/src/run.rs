use std::ffi::OsString;
use std::path::{Path, PathBuf};
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

/// What a run starts: a script file or a module (`-m`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A script, an absolute path.
    Script(PathBuf),
    /// A module by its qualified name (`MODULE_MODE`).
    Module(String),
}

/// A Python run: the inputs of the reference's Python run configuration
/// (`PythonRunConfiguration`, `PythonScriptCommandLineState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonRun {
    /// The interpreter (for a virtual environment its `bin/python` or
    /// `Scripts\python.exe`, not the resolved base interpreter).
    pub python: PathBuf,
    /// Run through `uv run`; `None` runs the interpreter directly.
    pub uv: Option<UvRun>,
    /// The script or module.
    pub target: Target,
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

const PYTHONPATH: &str = "PYTHONPATH";

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
            target: Target::Script(script),
            script_args: Vec::new(),
            interpreter_options: Vec::new(),
            working_dir,
            content_roots: Vec::new(),
            source_roots: Vec::new(),
            env: Vec::new(),
            inherit_env: true,
        }
    }

    /// The process to start, as the reference builds it
    /// (`PythonCommandLineState`, `PythonScripts.buildTargetedCommandLine`):
    ///
    /// - `<python> [options] <script> [args]`, or with uv
    ///   `<uv> run <python> [options] <script> [args]` and `VIRTUAL_ENV`,
    ///   `UV_PROJECT_ENVIRONMENT` set to the environment;
    /// - `PYTHONIOENCODING=UTF-8` (the console's encoding), the
    ///   configuration's variables, then `PYTHONUNBUFFERED=1` and
    ///   `PYCHARM_HOSTED=1` over them (`__PYVENV_LAUNCHER__` too when the IDE
    ///   has it);
    /// - `PYTHONPATH`: a `PYTHONPATH` of the configuration, then the content
    ///   and source roots, then the IDE's `PYTHONPATH` when the system
    ///   environment is included and the configuration sets none, then the
    ///   IDE's `PYTHONPATH` once more (the reference appends it a second time
    ///   when it builds the command line);
    /// - for a virtual environment its activation over everything:
    ///   `VIRTUAL_ENV` and `PATH` with the environment's scripts folder first.
    ///
    /// Reads the IDE's environment (`PYTHONPATH`, `PATH`).
    pub fn command_line(&self) -> CommandLine {
        self.command_line_with(&|name| std::env::var_os(name))
    }

    /// [`PythonRun::command_line`] with the IDE's environment given by
    /// `parent`.
    fn command_line_with(&self, parent: &dyn Fn(&str) -> Option<OsString>) -> CommandLine {
        let mut args: Vec<OsString> = Vec::new();
        let program = match &self.uv {
            Some(uv) => {
                args.push("run".into());
                args.push(self.python.clone().into_os_string());
                uv.uv.clone()
            }
            None => self.python.clone(),
        };
        args.extend(self.interpreter_options.iter().cloned());
        match &self.target {
            Target::Script(script) => args.push(script.clone().into_os_string()),
            Target::Module(module) => {
                args.push("-m".into());
                args.push(module.into());
            }
        }
        args.extend(self.script_args.iter().cloned());

        let mut env = Env::default();
        if let Some(uv) = &self.uv {
            env.set("VIRTUAL_ENV", uv.environment.clone().into_os_string());
            env.set(
                "UV_PROJECT_ENVIRONMENT",
                uv.environment.clone().into_os_string(),
            );
        }

        // The Python execution's own variables.
        let mut exec = Env::default();
        exec.set("PYTHONIOENCODING", "UTF-8".into());
        // prepareEnv: the configuration's variables, then the common ones.
        let mut prepared = Env::default();
        for (name, value) in &self.env {
            prepared.set_os(name.clone(), value.clone());
        }
        prepared.set("PYTHONUNBUFFERED", "1".into());
        if parent("__PYVENV_LAUNCHER__").is_some() {
            prepared.set("__PYVENV_LAUNCHER__", self.python.clone().into_os_string());
        }
        prepared.set("PYCHARM_HOSTED", "1".into());
        // extendEnvs: PYTHONPATH is appended, the rest set.
        for (name, value) in prepared.0 {
            if name == PYTHONPATH {
                exec.append_path(PYTHONPATH, value);
            } else {
                exec.set_os(name, value);
            }
        }
        // buildPythonPath: the roots, and the IDE's PYTHONPATH when the
        // configuration sets none.
        let mut roots: Vec<OsString> = Vec::new();
        for root in self.content_roots.iter().chain(&self.source_roots) {
            let root = root.clone().into_os_string();
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        if self.inherit_env && exec.get(PYTHONPATH).is_none() {
            roots.extend(system_python_path(parent));
        }
        if !roots.is_empty() {
            exec.append_path(PYTHONPATH, join_paths(&roots));
        }
        // buildTargetedCommandLine appends the IDE's PYTHONPATH again,
        // whatever the configuration says.
        let system = system_python_path(parent);
        if !system.is_empty() {
            exec.append_path(PYTHONPATH, join_paths(&system));
        }
        for (name, value) in exec.0 {
            env.set_os(name, value);
        }

        // The virtual environment's activation, over everything.
        let venv = match &self.uv {
            Some(uv) => Some(uv.environment.clone()),
            None => venv_root(&self.python),
        };
        if let Some(venv) = venv {
            let scripts = venv.join(if cfg!(windows) { "Scripts" } else { "bin" });
            let mut path = scripts.into_os_string();
            if let Some(old) = parent("PATH").filter(|p| !p.is_empty()) {
                path.push(path_separator());
                path.push(old);
            }
            env.set("VIRTUAL_ENV", venv.into_os_string());
            env.set("PATH", path);
        }

        CommandLine {
            program,
            args,
            cwd: Some(self.working_dir.clone()),
            env: env.0.into_iter().map(|(k, v)| (k, Some(v))).collect(),
            inherit_env: self.inherit_env,
        }
    }

    /// The first console line exactly as the reference prints it, without
    /// the line break: the quoted command line plus a trailing space (the
    /// reference joins it with a `StringUtil.join` overload that appends the
    /// separator). A view folds it to `<exe> ...` at 1000 characters or more.
    pub fn display(&self) -> String {
        let mut line = self.command_line_with(&|_| None).display();
        line.push(' ');
        line
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

/// Environment variables in order; setting an existing one replaces its
/// value in place.
#[derive(Default)]
struct Env(Vec<(OsString, OsString)>);

impl Env {
    fn set(&mut self, name: &str, value: OsString) {
        self.set_os(name.into(), value);
    }

    fn set_os(&mut self, name: OsString, value: OsString) {
        match self.0.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1 = value,
            None => self.0.push((name, value)),
        }
    }

    fn get(&self, name: &str) -> Option<&OsString> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// `appendToPythonPath`: joined to an existing value with the path
    /// separator.
    fn append_path(&mut self, name: &str, value: OsString) {
        match self.0.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => {
                entry.1.push(path_separator());
                entry.1.push(value);
            }
            None => self.0.push((name.into(), value)),
        }
    }
}

fn path_separator() -> &'static str {
    if cfg!(windows) { ";" } else { ":" }
}

fn join_paths(paths: &[OsString]) -> OsString {
    let mut out = OsString::new();
    for (i, p) in paths.iter().enumerate() {
        if i > 0 {
            out.push(path_separator());
        }
        out.push(p);
    }
    out
}

/// The IDE's `PYTHONPATH` split at the path separator, without trailing
/// empty entries (`appendSystemPythonPath`).
fn system_python_path(parent: &dyn Fn(&str) -> Option<OsString>) -> Vec<OsString> {
    let Some(value) = parent(PYTHONPATH) else {
        return Vec::new();
    };
    let value = value.to_string_lossy().into_owned();
    let mut parts: Vec<OsString> = value.split(path_separator()).map(OsString::from).collect();
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

/// The virtual environment `python` belongs to: `<root>/bin/python` or
/// `<root>\Scripts\python.exe` next to `pyvenv.cfg`.
fn venv_root(python: &Path) -> Option<PathBuf> {
    py_env::paths::venv_root_of(python)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(cmd: &CommandLine) -> Vec<(String, String)> {
        cmd.env
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.as_ref().unwrap().to_string_lossy().into_owned(),
                )
            })
            .collect()
    }

    fn get(cmd: &CommandLine, name: &str) -> Option<String> {
        env_of(cmd)
            .into_iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
    }

    fn no_parent(_: &str) -> Option<OsString> {
        None
    }

    #[cfg(not(windows))]
    #[test]
    fn plain_current_file_run() {
        let mut run = PythonRun::current_file("/usr/bin/python3", "/p/pkg/main.py");
        run.content_roots = vec!["/p".into()];
        let cmd = run.command_line_with(&no_parent);
        assert_eq!(cmd.program, PathBuf::from("/usr/bin/python3"));
        assert_eq!(cmd.args, vec![OsString::from("/p/pkg/main.py")]);
        assert_eq!(cmd.cwd, Some(PathBuf::from("/p/pkg")));
        assert!(cmd.inherit_env);
        assert_eq!(
            env_of(&cmd),
            [
                ("PYTHONIOENCODING", "UTF-8"),
                ("PYTHONUNBUFFERED", "1"),
                ("PYCHARM_HOSTED", "1"),
                ("PYTHONPATH", "/p"),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
        );
        assert_eq!(run.display(), "/usr/bin/python3 /p/pkg/main.py ");
    }

    #[cfg(not(windows))]
    #[test]
    fn python_path_order_follows_the_reference() {
        let mut run = PythonRun::current_file("/usr/bin/python3", "/p/main.py");
        run.content_roots = vec!["/p".into()];
        run.source_roots = vec!["/p/src".into(), "/p".into()];
        let parent = |name: &str| match name {
            "PYTHONPATH" => Some(OsString::from("/a:/b:")),
            _ => None,
        };
        let cmd = run.command_line_with(&parent);
        assert_eq!(get(&cmd, "PYTHONPATH").unwrap(), "/p:/p/src:/a:/b:/a:/b");
        // A PYTHONPATH of the configuration comes first and keeps the IDE's
        // value out of the first round.
        run.env = vec![("PYTHONPATH".into(), "/mine".into())];
        let cmd = run.command_line_with(&parent);
        assert_eq!(get(&cmd, "PYTHONPATH").unwrap(), "/mine:/p:/p/src:/a:/b");
        // Without the system environment the second round still appends it.
        run.env.clear();
        run.inherit_env = false;
        let cmd = run.command_line_with(&parent);
        assert_eq!(get(&cmd, "PYTHONPATH").unwrap(), "/p:/p/src:/a:/b");
        assert!(!cmd.inherit_env);
    }

    #[test]
    fn common_variables_override_the_configuration() {
        let mut run = PythonRun::current_file("python", "main.py");
        run.env = vec![
            ("PYTHONUNBUFFERED".into(), "0".into()),
            ("PYTHONIOENCODING".into(), "latin-1".into()),
            ("MY_VAR".into(), "x".into()),
        ];
        let cmd = run.command_line_with(&no_parent);
        assert_eq!(get(&cmd, "PYTHONUNBUFFERED").unwrap(), "1");
        assert_eq!(get(&cmd, "PYCHARM_HOSTED").unwrap(), "1");
        assert_eq!(get(&cmd, "PYTHONIOENCODING").unwrap(), "latin-1");
        assert_eq!(get(&cmd, "MY_VAR").unwrap(), "x");
        assert_eq!(get(&cmd, "__PYVENV_LAUNCHER__"), None);
        let launcher = |name: &str| (name == "__PYVENV_LAUNCHER__").then(|| OsString::from("x"));
        let cmd = run.command_line_with(&launcher);
        assert_eq!(get(&cmd, "__PYVENV_LAUNCHER__").unwrap(), "python");
    }

    #[test]
    fn a_virtual_environment_is_activated() {
        let dir = tempfile::tempdir().unwrap();
        let venv = dir.path().join(".venv");
        let scripts = venv.join(if cfg!(windows) { "Scripts" } else { "bin" });
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::write(venv.join("pyvenv.cfg"), "home = /usr/bin\n").unwrap();
        let python = scripts.join(if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        });
        let mut run = PythonRun::current_file(&python, dir.path().join("main.py"));
        run.env = vec![("PATH".into(), "/user/path".into())];
        let parent = |name: &str| (name == "PATH").then(|| OsString::from("/usr/bin"));
        let cmd = run.command_line_with(&parent);
        assert_eq!(get(&cmd, "VIRTUAL_ENV").unwrap(), venv.to_string_lossy());
        let expected = format!("{}{}/usr/bin", scripts.to_string_lossy(), path_separator());
        assert_eq!(get(&cmd, "PATH").unwrap(), expected);
        // A system interpreter is not activated.
        let run = PythonRun::current_file(dir.path().join("python"), dir.path().join("main.py"));
        let cmd = run.command_line_with(&parent);
        assert_eq!(get(&cmd, "VIRTUAL_ENV"), None);
        assert_eq!(get(&cmd, "PATH"), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn uv_run() {
        let mut run = PythonRun::current_file("/p/.venv/bin/python", "/p/main.py");
        run.uv = Some(UvRun {
            uv: "/opt/homebrew/bin/uv".into(),
            environment: "/p/.venv".into(),
        });
        run.script_args = vec!["--flag".into()];
        run.interpreter_options = vec!["-X".into(), "dev".into()];
        let cmd = run.command_line_with(&no_parent);
        assert_eq!(cmd.program, PathBuf::from("/opt/homebrew/bin/uv"));
        let args: Vec<_> = cmd
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "run",
                "/p/.venv/bin/python",
                "-X",
                "dev",
                "/p/main.py",
                "--flag"
            ]
        );
        assert_eq!(get(&cmd, "VIRTUAL_ENV").unwrap(), "/p/.venv");
        assert_eq!(get(&cmd, "UV_PROJECT_ENVIRONMENT").unwrap(), "/p/.venv");
        assert_eq!(get(&cmd, "PATH").unwrap(), "/p/.venv/bin");
        assert_eq!(
            run.display(),
            "/opt/homebrew/bin/uv run /p/.venv/bin/python -X dev /p/main.py --flag "
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn display_quotes_only_the_program() {
        let run = PythonRun::current_file("/My Projects/.venv/bin/python", "/My Projects/a b.py");
        assert_eq!(
            run.display(),
            "\"/My Projects/.venv/bin/python\" /My Projects/a b.py "
        );
    }

    #[cfg(windows)]
    #[test]
    fn display_on_windows() {
        let run = PythonRun::current_file(
            r"C:\My Projects\.venv\Scripts\python.exe",
            r"C:\My Projects\main.py",
        );
        assert_eq!(
            run.display(),
            r#""C:\My Projects\.venv\Scripts\python.exe" "C:\My Projects\main.py" "#
        );
    }
}
