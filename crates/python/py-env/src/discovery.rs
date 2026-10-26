use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::error::{EnvError, ErrorKind, Result};
use crate::interpreter::{Hint, Interpreter, InterpreterKind, Locations, probe_with};
use crate::paths::{
    canonical_key, conda_python, environment_python, extra_tool_dirs, find_tool, is_executable,
    is_python_name, venv_root_of,
};
use crate::process::{RunContext, Spec, run_ok};
use crate::project::{VENV_DIR_NAMES, find_project_root};
use crate::uv::{Uv, UvConfig};

/// A tool the environment features depend on, and whether it is there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStatus {
    pub name: String,
    pub installed: bool,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// What is lost without it.
    pub note: Option<String>,
}

/// An executable that was found but is not a working interpreter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoveryReport {
    pub interpreters: Vec<Interpreter>,
    pub tools: Vec<ToolStatus>,
    pub skipped: Vec<Skipped>,
}

/// The external tools used for discovery, each possibly absent.
#[derive(Debug, Clone, Default)]
pub struct Tools {
    pub uv: Option<PathBuf>,
    pub poetry: Option<PathBuf>,
    pub conda: Option<PathBuf>,
    /// The Windows `py` launcher.
    pub py_launcher: Option<PathBuf>,
}

impl Tools {
    pub fn locate() -> Tools {
        let conda = std::env::var_os("CONDA_EXE")
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| find_tool("conda"));
        Tools {
            uv: std::env::var_os(brand::env_name!("UV"))
                .map(PathBuf::from)
                .filter(|p| p.is_file())
                .or_else(|| find_tool("uv")),
            poetry: find_tool("poetry"),
            conda,
            py_launcher: if cfg!(windows) { find_tool("py") } else { None },
        }
    }
}

/// The process environment that decides where interpreters are looked for.
#[derive(Debug, Clone, Default)]
pub struct SearchEnv {
    /// Directories searched for `python*` executables, in order.
    pub path_dirs: Vec<PathBuf>,
    pub home: Option<PathBuf>,
    pub virtual_env: Option<PathBuf>,
    pub conda_prefix: Option<PathBuf>,
    pub uv_project_environment: Option<PathBuf>,
    pub pyenv_root: Option<PathBuf>,
    /// Overrides `uv python dir` when uv cannot be asked.
    pub uv_python_install_dir: Option<PathBuf>,
}

impl SearchEnv {
    pub fn from_process() -> SearchEnv {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut path_dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        for d in extra_tool_dirs() {
            if !path_dirs.contains(&d) {
                path_dirs.push(d);
            }
        }
        SearchEnv {
            path_dirs,
            home: dirs::home_dir(),
            virtual_env: var("VIRTUAL_ENV"),
            conda_prefix: var("CONDA_PREFIX"),
            uv_project_environment: var("UV_PROJECT_ENVIRONMENT"),
            pyenv_root: var("PYENV_ROOT"),
            uv_python_install_dir: var("UV_PYTHON_INSTALL_DIR"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveryOptions {
    /// The project directory (or a file in it); enables project environments.
    pub project: Option<PathBuf>,
    pub env: SearchEnv,
    pub tools: Tools,
    /// Limit for one interpreter probe.
    pub probe_timeout: Duration,
    /// Limit for each of uv, poetry and conda.
    pub tool_timeout: Duration,
    /// Interpreters probed at the same time.
    pub concurrency: usize,
}

impl DiscoveryOptions {
    pub fn new(env: SearchEnv, tools: Tools) -> Self {
        Self {
            project: None,
            env,
            tools,
            probe_timeout: Duration::from_secs(10),
            tool_timeout: Duration::from_secs(20),
            concurrency: 8,
        }
    }
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self::new(SearchEnv::from_process(), Tools::locate())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub path: PathBuf,
    pub hint: Hint,
    pub source: &'static str,
}

impl Candidate {
    fn priority(&self) -> u8 {
        match self.hint {
            Hint::ProjectVenv | Hint::Poetry => 0,
            Hint::Conda => 1,
            Hint::Uv => 2,
            Hint::Pyenv => 3,
            Hint::Path => 4,
        }
    }

    /// Two candidates are the same interpreter when their canonical paths
    /// match, except that virtual environments are identified by their
    /// directory (their `python` is a link to the base interpreter).
    fn key(&self) -> PathBuf {
        match venv_root_of(&self.path) {
            Some(root) => canonical_key(&root),
            None => canonical_key(&self.path),
        }
    }
}

/// Find the Python interpreters usable for a project and the tools that
/// manage them. Sources are queried concurrently; every interpreter is
/// probed (`python -I -S -c ...`) with bounded parallelism. Missing tools are
/// reported, not errors. `on_found` is called for each interpreter as soon as
/// it is probed, in completion order; the report is sorted.
pub async fn discover(
    opts: &DiscoveryOptions,
    ctx: &RunContext,
    on_found: &(dyn Fn(&Interpreter) + Send + Sync),
) -> Result<DiscoveryReport> {
    let quiet = RunContext {
        progress: None,
        cancel: ctx.cancel.clone(),
        timeout: None,
    };
    let project_root = opts.project.as_deref().map(|p| {
        let p = if p.is_relative() {
            std::env::current_dir()
                .map(|c| c.join(p))
                .unwrap_or_else(|_| p.to_path_buf())
        } else {
            p.to_path_buf()
        };
        find_project_root(&p)
    });

    let uv = opts
        .tools
        .uv
        .as_ref()
        .map(|p| Uv::new(p, UvConfig::default()));
    let (uv_version, uv_found, uv_dir, path_found, poetry_found, conda_found, py_found) = tokio::join!(
        async {
            match &uv {
                Some(uv) => uv.version(&quiet).await.ok(),
                None => None,
            }
        },
        uv_candidates(uv.as_ref(), opts, &quiet),
        uv_python_dir(uv.as_ref(), opts, &quiet),
        scan_path_dirs(&opts.env.path_dirs),
        poetry_candidates(opts, project_root.as_deref(), &quiet),
        conda_candidates(opts, &quiet),
        py_launcher_candidates(opts, &quiet),
    );

    let mut skipped = Vec::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    candidates.extend(project_env_candidates(&opts.env, project_root.as_deref()));
    candidates.extend(poetry_found);
    candidates.extend(conda_found);
    candidates.extend(uv_found);
    candidates.extend(pyenv_candidates(&opts.env));
    candidates.extend(path_found);
    candidates.extend(py_found);
    drop_apple_stub(&mut candidates, &mut skipped, &quiet).await;

    // Keep the best-ranked candidate of each interpreter; ties keep the first found.
    let mut best: BTreeMap<PathBuf, Candidate> = BTreeMap::new();
    let mut order: Vec<PathBuf> = Vec::new();
    for c in candidates {
        let key = c.key();
        match best.get(&key) {
            Some(existing) if existing.priority() <= c.priority() => {}
            Some(_) => {
                best.insert(key, c);
            }
            None => {
                order.push(key.clone());
                best.insert(key, c);
            }
        }
    }

    let uv_dir = uv_dir.map(|d| canonical_key(&d));
    let locations = Arc::new(Locations { uv_dir });
    let limit = Arc::new(Semaphore::new(opts.concurrency.max(1)));
    let mut tasks: JoinSet<(Candidate, Result<Interpreter>)> = JoinSet::new();
    for key in &order {
        let c = best[key].clone();
        let (loc, limit, quiet) = (locations.clone(), limit.clone(), quiet.clone());
        let timeout = opts.probe_timeout;
        tasks.spawn(async move {
            let _permit = limit.acquire().await;
            let r = probe_with(&c.path, c.hint, c.source, &loc, timeout, &quiet).await;
            (c, r)
        });
    }

    let mut interpreters = Vec::new();
    loop {
        // Cancellation first: a probe cancelled by the same token finishes at
        // the same moment and must not turn the run into a partial result.
        let next = tokio::select! {
            biased;
            () = cancelled(&ctx.cancel) => {
                return Err(EnvError::new(ErrorKind::Cancelled, "interpreter discovery was cancelled"));
            }
            n = tasks.join_next() => n,
        };
        let Some(joined) = next else {
            if ctx.cancel.as_ref().is_some_and(|t| t.is_cancelled()) {
                return Err(EnvError::new(
                    ErrorKind::Cancelled,
                    "interpreter discovery was cancelled",
                ));
            }
            break;
        };
        let Ok((candidate, result)) = joined else {
            continue;
        };
        match result {
            Ok(i) => {
                on_found(&i);
                interpreters.push(i);
            }
            Err(e) => skipped.push(Skipped {
                path: candidate.path,
                reason: e.message,
            }),
        }
    }
    interpreters.sort_by(|a, b| {
        (
            a.kind_rank(),
            std::cmp::Reverse((a.major, a.minor, a.micro)),
            &a.path,
        )
            .cmp(&(
                b.kind_rank(),
                std::cmp::Reverse((b.major, b.minor, b.micro)),
                &b.path,
            ))
    });
    skipped.sort_by(|a, b| a.path.cmp(&b.path));

    let tools = tool_report(opts, uv_version);
    Ok(DiscoveryReport {
        interpreters,
        tools,
        skipped,
    })
}

async fn cancelled(token: &Option<tokio_util::sync::CancellationToken>) {
    match token {
        Some(t) => t.cancelled().await,
        None => std::future::pending().await,
    }
}

impl Interpreter {
    /// Display order: environments of the project first, installations last.
    fn kind_rank(&self) -> u8 {
        match self.kind {
            InterpreterKind::Venv => 0,
            InterpreterKind::Poetry => 1,
            InterpreterKind::Conda => 2,
            InterpreterKind::Uv => 3,
            InterpreterKind::Pyenv => 4,
            InterpreterKind::System => 5,
        }
    }
}

fn tool_report(opts: &DiscoveryOptions, uv_version: Option<String>) -> Vec<ToolStatus> {
    let mut tools = vec![
        ToolStatus {
            name: "uv".into(),
            installed: opts.tools.uv.is_some(),
            path: opts.tools.uv.clone(),
            version: uv_version,
            note: opts.tools.uv.is_none().then(|| {
                "Without uv, packages are managed with pip and projects cannot be locked or synced.".to_owned()
            }),
        },
        ToolStatus {
            name: "poetry".into(),
            installed: opts.tools.poetry.is_some(),
            path: opts.tools.poetry.clone(),
            version: None,
            note: opts
                .tools
                .poetry
                .is_none()
                .then(|| "Poetry environments are not listed.".to_owned()),
        },
        ToolStatus {
            name: "conda".into(),
            installed: opts.tools.conda.is_some(),
            path: opts.tools.conda.clone(),
            version: None,
            note: opts
                .tools
                .conda
                .is_none()
                .then(|| "Conda environments are found from ~/.conda/environments.txt only.".to_owned()),
        },
    ];
    let pyenv = pyenv_versions_dir(&opts.env);
    tools.push(ToolStatus {
        name: "pyenv".into(),
        installed: pyenv.as_ref().is_some_and(|d| d.is_dir()),
        path: pyenv,
        version: None,
        note: None,
    });
    if cfg!(windows) {
        tools.push(ToolStatus {
            name: "py".into(),
            installed: opts.tools.py_launcher.is_some(),
            path: opts.tools.py_launcher.clone(),
            version: None,
            note: None,
        });
    }
    tools
}

// ---------- sources ----------

/// uv's list of installed interpreters (managed and found on the system).
async fn uv_candidates(
    uv: Option<&Uv>,
    opts: &DiscoveryOptions,
    ctx: &RunContext,
) -> Vec<Candidate> {
    let Some(uv) = uv else { return Vec::new() };
    let ctx = RunContext {
        timeout: Some(opts.tool_timeout),
        ..ctx.clone()
    };
    match uv.python_list(true, &ctx).await {
        Ok(list) => list
            .into_iter()
            .filter_map(|p| p.path)
            .map(|path| Candidate {
                path,
                hint: Hint::Uv,
                source: "uv",
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

async fn uv_python_dir(
    uv: Option<&Uv>,
    opts: &DiscoveryOptions,
    ctx: &RunContext,
) -> Option<PathBuf> {
    if let Some(d) = &opts.env.uv_python_install_dir {
        return Some(d.clone());
    }
    let uv = uv?;
    let ctx = RunContext {
        timeout: Some(opts.tool_timeout),
        ..ctx.clone()
    };
    uv.python_dir(&ctx).await.ok()
}

/// `python`, `python3` and `python3.N` in the given directories.
async fn scan_path_dirs(dirs: &[PathBuf]) -> Vec<Candidate> {
    let dirs = dirs.to_vec();
    tokio::task::spawn_blocking(move || scan_path_dirs_blocking(&dirs))
        .await
        .unwrap_or_default()
}

pub(crate) fn scan_path_dirs_blocking(dirs: &[PathBuf]) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut seen_dirs = Vec::new();
    for dir in dirs {
        if dir.as_os_str().is_empty() || seen_dirs.contains(dir) {
            continue;
        }
        seen_dirs.push(dir.clone());
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| is_python_name(&e.file_name()))
            .map(|e| e.path())
            .filter(|p| is_executable(p))
            .collect();
        // python3.13 before python3 before python: the most specific name first.
        found.sort_by_key(|p| std::cmp::Reverse(name_specificity(p)));
        out.extend(found.into_iter().map(|path| Candidate {
            path,
            hint: Hint::Path,
            source: "PATH",
        }));
    }
    out
}

fn name_specificity(p: &Path) -> (u8, u32) {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let name = name.strip_suffix(".exe").unwrap_or(name);
    match name.strip_prefix("python3.") {
        Some(minor) => (2, minor.parse().unwrap_or(0)),
        None if name == "python3" => (1, 0),
        None => (0, 0),
    }
}

/// `.venv` and `venv` in the project and its ancestors, plus the environment
/// variables that name an active environment.
pub(crate) fn project_env_candidates(
    env: &SearchEnv,
    project_root: Option<&Path>,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut push = |dir: &Path, hint: Hint, source: &'static str| {
        if let Some(path) = environment_python(dir) {
            out.push(Candidate { path, hint, source });
        }
    };
    if let Some(root) = project_root {
        if let Some(over) = &env.uv_project_environment {
            let dir = if over.is_absolute() {
                over.clone()
            } else {
                root.join(over)
            };
            push(&dir, Hint::ProjectVenv, "UV_PROJECT_ENVIRONMENT");
        }
        for dir in root.ancestors() {
            for name in VENV_DIR_NAMES {
                let venv = dir.join(name);
                if venv.join("pyvenv.cfg").is_file() {
                    push(
                        &venv,
                        Hint::ProjectVenv,
                        if name == ".venv" { ".venv" } else { "venv" },
                    );
                }
            }
        }
    }
    if let Some(v) = &env.virtual_env {
        push(v, Hint::ProjectVenv, "VIRTUAL_ENV");
    }
    if let Some(c) = &env.conda_prefix {
        push(c, Hint::Conda, "CONDA_PREFIX");
    }
    out
}

fn is_poetry_project(root: &Path) -> bool {
    root.join("poetry.lock").is_file()
        || std::fs::read_to_string(root.join("pyproject.toml"))
            .is_ok_and(|t| t.contains("[tool.poetry]"))
}

/// The environment Poetry keeps for the project (`poetry env info -p`).
async fn poetry_candidates(
    opts: &DiscoveryOptions,
    root: Option<&Path>,
    ctx: &RunContext,
) -> Vec<Candidate> {
    let (Some(poetry), Some(root)) = (&opts.tools.poetry, root) else {
        return Vec::new();
    };
    if !is_poetry_project(root) {
        return Vec::new();
    }
    let spec = Spec::new(poetry)
        .args(["env", "info", "--path"])
        .cwd(root)
        .env("NO_COLOR", "1");
    let Ok(done) = run_ok("poetry", spec, ctx, opts.tool_timeout).await else {
        return Vec::new();
    };
    let dir = PathBuf::from(done.stdout.trim());
    match environment_python(&dir) {
        Some(path) if !done.stdout.trim().is_empty() => vec![Candidate {
            path,
            hint: Hint::Poetry,
            source: "poetry",
        }],
        _ => Vec::new(),
    }
}

/// `conda env list --json`; without conda, the environments conda recorded
/// in `~/.conda/environments.txt`.
async fn conda_candidates(opts: &DiscoveryOptions, ctx: &RunContext) -> Vec<Candidate> {
    let dirs: Vec<PathBuf> = match &opts.tools.conda {
        Some(conda) => {
            let spec = Spec::new(conda)
                .args(["env", "list", "--json"])
                .env("NO_COLOR", "1");
            match run_ok("conda", spec, ctx, opts.tool_timeout).await {
                Ok(done) => parse_conda_env_list(&done.stdout),
                Err(_) => Vec::new(),
            }
        }
        None => opts
            .env
            .home
            .as_ref()
            .and_then(|h| std::fs::read_to_string(h.join(".conda").join("environments.txt")).ok())
            .map(|t| parse_environments_txt(&t))
            .unwrap_or_default(),
    };
    dirs.iter()
        .filter_map(|d| {
            conda_env_python(d).map(|path| Candidate {
                path,
                hint: Hint::Conda,
                source: "conda",
            })
        })
        .collect()
}

fn conda_env_python(dir: &Path) -> Option<PathBuf> {
    let p = conda_python(dir);
    p.is_file().then_some(p)
}

pub(crate) fn parse_conda_env_list(json: &str) -> Vec<PathBuf> {
    #[derive(Deserialize)]
    struct Envs {
        #[serde(default)]
        envs: Vec<PathBuf>,
    }
    serde_json::from_str::<Envs>(json.trim())
        .map(|e| e.envs)
        .unwrap_or_default()
}

pub(crate) fn parse_environments_txt(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .collect()
}

fn pyenv_versions_dir(env: &SearchEnv) -> Option<PathBuf> {
    if let Some(root) = &env.pyenv_root {
        let versions = root.join("versions");
        let win = root.join("pyenv-win").join("versions");
        return Some(if win.is_dir() { win } else { versions });
    }
    let home = env.home.as_ref()?;
    let win = home.join(".pyenv").join("pyenv-win").join("versions");
    Some(if win.is_dir() {
        win
    } else {
        home.join(".pyenv").join("versions")
    })
}

/// `$PYENV_ROOT/versions/*`: one interpreter per installed version.
pub(crate) fn pyenv_candidates(env: &SearchEnv) -> Vec<Candidate> {
    let Some(dir) = pyenv_versions_dir(env) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Candidate> = entries
        .flatten()
        .filter_map(|e| environment_python(&e.path()))
        .map(|path| Candidate {
            path,
            hint: Hint::Pyenv,
            source: "pyenv",
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// `py -0p` on Windows: every installation the launcher knows.
async fn py_launcher_candidates(opts: &DiscoveryOptions, ctx: &RunContext) -> Vec<Candidate> {
    let Some(py) = &opts.tools.py_launcher else {
        return Vec::new();
    };
    let Ok(done) = run_ok("py", Spec::new(py).args(["-0p"]), ctx, opts.tool_timeout).await else {
        return Vec::new();
    };
    parse_py_launcher(&done.stdout)
        .into_iter()
        .map(|path| Candidate {
            path,
            hint: Hint::Path,
            source: "py launcher",
        })
        .collect()
}

/// Lines of `py -0p`: ` -V:3.13 *        C:\Python313\python.exe`, or the older
/// ` -3.9-64        C:\...\python.exe`. The path is whatever starts at a drive
/// letter or a UNC prefix.
pub(crate) fn parse_py_launcher(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let b = line.as_bytes();
            let start = (0..b.len()).find(|&i| {
                (b[i].is_ascii_alphabetic()
                    && b.get(i + 1) == Some(&b':')
                    && matches!(b.get(i + 2), Some(b'\\') | Some(b'/'))
                    && (i == 0 || b[i - 1].is_ascii_whitespace()))
                    || (b[i] == b'\\'
                        && b.get(i + 1) == Some(&b'\\')
                        && (i == 0 || b[i - 1].is_ascii_whitespace()))
            })?;
            let path = line[start..].trim();
            (!path.is_empty()).then(|| PathBuf::from(path))
        })
        .collect()
}

/// On macOS `/usr/bin/python3` is a stub that opens the Command Line Tools
/// installer when they are missing. Do not run it then.
#[cfg(target_os = "macos")]
async fn drop_apple_stub(
    candidates: &mut Vec<Candidate>,
    skipped: &mut Vec<Skipped>,
    ctx: &RunContext,
) {
    const STUBS: [&str; 2] = ["/usr/bin/python3", "/usr/bin/python"];
    if !candidates
        .iter()
        .any(|c| STUBS.iter().any(|s| c.path == Path::new(s)))
    {
        return;
    }
    let spec = Spec::new("/usr/bin/xcode-select").args(["-p"]);
    if run_ok("xcode-select", spec, ctx, Duration::from_secs(5))
        .await
        .is_ok()
    {
        return;
    }
    candidates.retain(|c| {
        let stub = STUBS.iter().any(|s| c.path == Path::new(s));
        if stub {
            skipped.push(Skipped {
                path: c.path.clone(),
                reason: "macOS stub: the Command Line Tools are not installed (run `xcode-select --install`)".into(),
            });
        }
        !stub
    });
}

#[cfg(not(target_os = "macos"))]
async fn drop_apple_stub(_: &mut Vec<Candidate>, _: &mut Vec<Skipped>, _: &RunContext) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_conda_env_list_json() {
        let json = r#"{"envs": ["/opt/conda", "/opt/conda/envs/data", "/home/u/.conda/envs/ml"]}"#;
        assert_eq!(
            parse_conda_env_list(json),
            [
                PathBuf::from("/opt/conda"),
                PathBuf::from("/opt/conda/envs/data"),
                PathBuf::from("/home/u/.conda/envs/ml")
            ]
        );
        assert!(parse_conda_env_list("").is_empty());
        assert!(parse_conda_env_list("{}").is_empty());
        assert!(parse_conda_env_list("conda: command not found").is_empty());
    }

    #[test]
    fn parses_environments_txt() {
        assert_eq!(
            parse_environments_txt("# comment\n/opt/conda\n\n  /home/u/envs/x  \n"),
            [PathBuf::from("/opt/conda"), PathBuf::from("/home/u/envs/x")]
        );
    }

    #[test]
    fn parses_py_launcher_listings() {
        let modern = " -V:3.13 *        C:\\Python313\\python.exe\n -V:3.12          C:\\Users\\me\\AppData\\Local\\Programs\\Python\\Python312\\python.exe\n -V:ContinuumAnalytics/Anaconda39-64 D:\\anaconda3\\python.exe\n";
        assert_eq!(
            parse_py_launcher(modern),
            [
                PathBuf::from("C:\\Python313\\python.exe"),
                PathBuf::from(
                    "C:\\Users\\me\\AppData\\Local\\Programs\\Python\\Python312\\python.exe"
                ),
                PathBuf::from("D:\\anaconda3\\python.exe"),
            ]
        );
        let old = " -3.9-64        C:\\Python39\\python.exe *\n";
        // A trailing marker would end up in the path; the launcher puts `*` before the path.
        assert_eq!(
            parse_py_launcher(old),
            [PathBuf::from("C:\\Python39\\python.exe *")]
        );
        assert!(parse_py_launcher("No installed Pythons found!\n").is_empty());
        assert!(parse_py_launcher("").is_empty());
    }

    #[test]
    fn specificity_orders_python_names() {
        let mut names = vec!["python", "python3", "python3.9", "python3.13", "python3.12"];
        names.sort_by_key(|n| std::cmp::Reverse(name_specificity(Path::new(n))));
        assert_eq!(
            names,
            ["python3.13", "python3.12", "python3.9", "python3", "python"]
        );
    }

    #[cfg(unix)]
    mod unix {
        use super::super::*;
        use crate::paths::venv_python;
        use std::os::unix::fs::PermissionsExt;

        /// An executable that answers the probe the way a real interpreter does.
        fn fake_python(path: &Path, version: [u32; 3], prefix: &Path, base_prefix: &Path) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let json = format!(
                r#"{{"version": [{}, {}, {}, "final", 0], "implementation": "cpython", "prefix": "{}", "base_prefix": "{}", "machine": "arm64", "free_threaded": false}}"#,
                version[0],
                version[1],
                version[2],
                prefix.display(),
                base_prefix.display()
            );
            std::fs::write(path, format!("#!/bin/sh\necho '{json}'\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        fn fake_venv(root: &Path, version: [u32; 3], base: &Path) -> PathBuf {
            std::fs::create_dir_all(root).unwrap();
            std::fs::write(root.join("pyvenv.cfg"), "home = /x\n").unwrap();
            let py = venv_python(root);
            fake_python(&py, version, root, base);
            py
        }

        fn options(root: &Path) -> DiscoveryOptions {
            DiscoveryOptions {
                project: None,
                env: SearchEnv {
                    home: Some(root.join("home")),
                    ..SearchEnv::default()
                },
                tools: Tools::default(),
                probe_timeout: Duration::from_secs(10),
                tool_timeout: Duration::from_secs(10),
                concurrency: 4,
            }
        }

        async fn run(opts: &DiscoveryOptions) -> DiscoveryReport {
            discover(opts, &RunContext::default(), &|_| {})
                .await
                .unwrap()
        }

        #[tokio::test]
        async fn finds_path_interpreters_and_dedupes_aliases() {
            let dir = tempfile::tempdir().unwrap();
            let root = canonical_key(dir.path());
            let bin = root.join("bin");
            let install = root.join("install");
            let real = install.join("bin").join("python3.12");
            fake_python(&real, [3, 12, 4], &install, &install);
            // python3.12, python3 and python all lead to the same file.
            for alias in ["python3.12", "python3", "python"] {
                std::fs::create_dir_all(&bin).unwrap();
                std::os::unix::fs::symlink(&real, bin.join(alias)).unwrap();
            }
            std::fs::write(bin.join("python3-config"), "").unwrap();
            let other = root.join("other").join("python3.9");
            fake_python(&other, [3, 9, 6], &root.join("other"), &root.join("other"));

            let mut o = options(&root);
            o.env.path_dirs = vec![
                bin.clone(),
                root.join("other"),
                bin.clone(),
                root.join("missing"),
            ];
            let report = run(&o).await;
            let versions: Vec<_> = report
                .interpreters
                .iter()
                .map(|i| i.version.as_str())
                .collect();
            assert_eq!(versions, ["3.12.4", "3.9.6"]);
            // The most specific alias represents the interpreter.
            assert_eq!(report.interpreters[0].path, bin.join("python3.12"));
            assert!(
                report
                    .interpreters
                    .iter()
                    .all(|i| i.kind == InterpreterKind::System)
            );
            assert!(report.interpreters.iter().all(|i| i.source == "PATH"));
            assert!(report.skipped.is_empty());
        }

        #[tokio::test]
        async fn project_venvs_are_found_and_not_confused_with_their_base() {
            let dir = tempfile::tempdir().unwrap();
            let root = canonical_key(dir.path());
            let install = root.join("install");
            let base_py = install.join("bin").join("python3.12");
            fake_python(&base_py, [3, 12, 4], &install, &install);

            let project = root.join("proj");
            let venv = project.join(".venv");
            let vpy = fake_venv(&venv, [3, 12, 4], &install);
            // The venv's interpreter links to the base one, as real venvs do.
            std::fs::remove_file(&vpy).unwrap();
            // The link target answers as the venv, because sys.prefix depends on pyvenv.cfg.
            fake_python(
                &install.join("bin").join("venv-python"),
                [3, 12, 4],
                &venv,
                &install,
            );
            std::os::unix::fs::symlink(install.join("bin").join("venv-python"), &vpy).unwrap();
            std::fs::write(project.join("pyproject.toml"), "[project]\nname = \"p\"\n").unwrap();

            let mut o = options(&root);
            o.project = Some(project.join("src"));
            std::fs::create_dir_all(project.join("src")).unwrap();
            o.env.path_dirs = vec![install.join("bin")];
            let report = run(&o).await;
            let kinds: Vec<_> = report
                .interpreters
                .iter()
                .map(|i| (i.kind, i.source.as_str()))
                .collect();
            assert!(
                kinds.contains(&(InterpreterKind::Venv, ".venv")),
                "{kinds:?}"
            );
            let venv_i = report.interpreters.iter().find(|i| i.is_venv).unwrap();
            assert_eq!(venv_i.venv_root.as_deref(), Some(venv.as_path()));
            assert_eq!(venv_i.path, vpy);
            // The venv sorts before the installation it was made from.
            assert!(report.interpreters[0].is_venv);
            assert!(
                report
                    .interpreters
                    .iter()
                    .any(|i| i.path == base_py && i.kind == InterpreterKind::System)
            );
        }

        #[tokio::test]
        async fn virtual_env_variable_and_uv_project_environment_are_used() {
            let dir = tempfile::tempdir().unwrap();
            let root = canonical_key(dir.path());
            let base = root.join("base");
            let active = root.join("active-env");
            fake_venv(&active, [3, 11, 9], &base);
            let project = root.join("proj");
            std::fs::create_dir_all(&project).unwrap();
            std::fs::write(project.join("pyproject.toml"), "[project]\nname = \"p\"\n").unwrap();
            let custom = project.join("tools-env");
            fake_venv(&custom, [3, 13, 1], &base);

            let mut o = options(&root);
            o.project = Some(project);
            o.env.virtual_env = Some(active.clone());
            o.env.uv_project_environment = Some(PathBuf::from("tools-env"));
            let report = run(&o).await;
            let mut sources: Vec<_> = report
                .interpreters
                .iter()
                .map(|i| i.source.as_str())
                .collect();
            sources.sort_unstable();
            assert_eq!(sources, ["UV_PROJECT_ENVIRONMENT", "VIRTUAL_ENV"]);
            assert_eq!(report.interpreters[0].version, "3.13.1");
        }

        #[tokio::test]
        async fn pyenv_versions_are_listed() {
            let dir = tempfile::tempdir().unwrap();
            let root = canonical_key(dir.path());
            let pyenv = root.join("pyenv");
            for v in ["3.10.14", "3.11.9"] {
                let prefix = pyenv.join("versions").join(v);
                let parts: Vec<u32> = v.split('.').map(|p| p.parse().unwrap()).collect();
                fake_python(
                    &prefix.join("bin").join("python"),
                    [parts[0], parts[1], parts[2]],
                    &prefix,
                    &prefix,
                );
            }
            let mut o = options(&root);
            o.env.pyenv_root = Some(pyenv);
            let report = run(&o).await;
            let found: Vec<_> = report
                .interpreters
                .iter()
                .map(|i| (i.version.as_str(), i.kind))
                .collect();
            assert_eq!(
                found,
                [
                    ("3.11.9", InterpreterKind::Pyenv),
                    ("3.10.14", InterpreterKind::Pyenv)
                ]
            );
            let pyenv_tool = report.tools.iter().find(|t| t.name == "pyenv").unwrap();
            assert!(pyenv_tool.installed);
        }

        #[tokio::test]
        async fn conda_envs_come_from_environments_txt_without_conda() {
            let dir = tempfile::tempdir().unwrap();
            let root = canonical_key(dir.path());
            let env_dir = root.join("miniconda").join("envs").join("data");
            std::fs::create_dir_all(env_dir.join("conda-meta")).unwrap();
            fake_python(
                &env_dir.join("bin").join("python"),
                [3, 10, 0],
                &env_dir,
                &env_dir,
            );
            let home = root.join("home");
            std::fs::create_dir_all(home.join(".conda")).unwrap();
            std::fs::write(
                home.join(".conda").join("environments.txt"),
                format!("{}\n{}\n", env_dir.display(), root.join("gone").display()),
            )
            .unwrap();
            let o = options(&root);
            let report = run(&o).await;
            assert_eq!(report.interpreters.len(), 1);
            assert_eq!(report.interpreters[0].kind, InterpreterKind::Conda);
            assert_eq!(report.interpreters[0].source, "conda");
            let conda = report.tools.iter().find(|t| t.name == "conda").unwrap();
            assert!(!conda.installed);
            assert!(conda.note.is_some());
        }

        #[tokio::test]
        async fn conda_command_output_is_used_when_conda_exists() {
            let dir = tempfile::tempdir().unwrap();
            let root = canonical_key(dir.path());
            let env_dir = root.join("envs").join("ml");
            std::fs::create_dir_all(env_dir.join("conda-meta")).unwrap();
            fake_python(
                &env_dir.join("bin").join("python"),
                [3, 12, 1],
                &env_dir,
                &env_dir,
            );
            let conda = root.join("conda");
            std::fs::write(
                &conda,
                format!("#!/bin/sh\n[ \"$1 $2 $3\" = \"env list --json\" ] || exit 2\necho '{{\"envs\": [\"{}\"]}}'\n", env_dir.display()),
            )
            .unwrap();
            std::fs::set_permissions(&conda, std::fs::Permissions::from_mode(0o755)).unwrap();
            let mut o = options(&root);
            o.tools.conda = Some(conda);
            let report = run(&o).await;
            assert_eq!(report.interpreters.len(), 1);
            assert_eq!(report.interpreters[0].kind, InterpreterKind::Conda);
            assert!(
                report
                    .tools
                    .iter()
                    .find(|t| t.name == "conda")
                    .unwrap()
                    .installed
            );
        }

        #[tokio::test]
        async fn poetry_environment_of_a_poetry_project() {
            let dir = tempfile::tempdir().unwrap();
            let root = canonical_key(dir.path());
            let project = root.join("poem");
            std::fs::create_dir_all(&project).unwrap();
            std::fs::write(
                project.join("pyproject.toml"),
                "[tool.poetry]\nname = \"poem\"\n",
            )
            .unwrap();
            let venv = root.join("cache").join("poem-py3.12");
            fake_venv(&venv, [3, 12, 2], &root.join("base"));
            let poetry = root.join("poetry");
            std::fs::write(
                &poetry,
                format!(
                    "#!/bin/sh\n[ \"$1 $2 $3\" = \"env info --path\" ] || exit 2\necho '{}'\n",
                    venv.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&poetry, std::fs::Permissions::from_mode(0o755)).unwrap();
            let mut o = options(&root);
            o.tools.poetry = Some(poetry.clone());
            o.project = Some(project.clone());
            let report = run(&o).await;
            assert_eq!(report.interpreters.len(), 1);
            assert_eq!(report.interpreters[0].kind, InterpreterKind::Poetry);

            // Not a Poetry project: poetry is not asked.
            std::fs::write(
                project.join("pyproject.toml"),
                "[project]\nname = \"poem\"\n",
            )
            .unwrap();
            let report = run(&o).await;
            assert!(report.interpreters.is_empty());
        }

        #[tokio::test]
        async fn broken_candidates_are_reported_not_fatal() {
            let dir = tempfile::tempdir().unwrap();
            let root = canonical_key(dir.path());
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let bad = bin.join("python3.7");
            std::fs::write(&bad, "#!/bin/sh\necho 'cannot run' 1>&2\nexit 1\n").unwrap();
            std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o755)).unwrap();
            fake_python(&bin.join("python3.12"), [3, 12, 0], &bin, &bin);
            let mut o = options(&root);
            o.env.path_dirs = vec![bin];
            let report = run(&o).await;
            assert_eq!(report.interpreters.len(), 1);
            assert_eq!(report.skipped.len(), 1);
            assert!(report.skipped[0].path.ends_with("python3.7"));
            assert_eq!(report.skipped[0].reason, "cannot run");
        }

        #[tokio::test]
        async fn results_stream_while_probing() {
            use std::sync::Mutex;
            let dir = tempfile::tempdir().unwrap();
            let root = canonical_key(dir.path());
            let bin = root.join("bin");
            fake_python(&bin.join("python3.12"), [3, 12, 0], &bin, &bin);
            fake_python(
                &root.join("b").join("python3.11"),
                [3, 11, 0],
                &root.join("b"),
                &root.join("b"),
            );
            let mut o = options(&root);
            o.env.path_dirs = vec![bin, root.join("b")];
            let seen = Mutex::new(Vec::new());
            let report = discover(&o, &RunContext::default(), &|i| {
                seen.lock().unwrap().push(i.version.clone())
            })
            .await
            .unwrap();
            let mut seen = seen.into_inner().unwrap();
            seen.sort();
            assert_eq!(seen, ["3.11.0", "3.12.0"]);
            assert_eq!(report.interpreters.len(), 2);
        }

        #[tokio::test]
        async fn cancellation_returns_an_error() {
            let dir = tempfile::tempdir().unwrap();
            let root = canonical_key(dir.path());
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let slow = bin.join("python3");
            std::fs::write(&slow, "#!/bin/sh\nsleep 30\n").unwrap();
            std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
            let mut o = options(&root);
            o.env.path_dirs = vec![bin];
            let token = tokio_util::sync::CancellationToken::new();
            let ctx = RunContext::default().with_cancel(token.clone());
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                token.cancel();
            });
            let started = std::time::Instant::now();
            let e = discover(&o, &ctx, &|_| {}).await.unwrap_err();
            assert_eq!(e.kind, ErrorKind::Cancelled);
            assert!(started.elapsed() < Duration::from_secs(10));
        }

        #[tokio::test]
        async fn missing_tools_are_reported_with_consequences() {
            let dir = tempfile::tempdir().unwrap();
            let o = options(dir.path());
            let report = run(&o).await;
            let names: Vec<_> = report
                .tools
                .iter()
                .map(|t| (t.name.as_str(), t.installed))
                .collect();
            assert_eq!(
                names,
                [
                    ("uv", false),
                    ("poetry", false),
                    ("conda", false),
                    ("pyenv", false)
                ]
            );
            assert!(report.tools[0].note.as_ref().unwrap().contains("pip"));
        }
    }
}
