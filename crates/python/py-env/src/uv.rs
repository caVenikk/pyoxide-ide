use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{EnvError, Result};
use crate::packages::{Outcome, Package, parse_package_list};
use crate::paths::find_tool;
use crate::process::{Finished, RunContext, Spec, run_ok};

const QUICK: Duration = Duration::from_secs(20);
const SHORT: Duration = Duration::from_secs(120);
const LONG: Duration = Duration::from_secs(30 * 60);

/// How uv is run. The defaults leave uv's own configuration alone.
#[derive(Debug, Clone)]
pub struct UvConfig {
    /// Never touch the network (`UV_OFFLINE`).
    pub offline: bool,
    /// Let uv download Python interpreters on demand.
    pub python_downloads: bool,
    /// Ignore `uv.toml` and `[tool.uv]` files (`UV_NO_CONFIG`).
    pub no_config: bool,
    pub cache_dir: Option<PathBuf>,
    /// Where uv keeps the interpreters it installs (`UV_PYTHON_INSTALL_DIR`).
    pub python_install_dir: Option<PathBuf>,
    /// Do not pass the variables that steer uv's choices on from this
    /// process's environment (`UV_*`, `VIRTUAL_ENV`, `CONDA_PREFIX` and the
    /// like, see [`crate::process::steering_env`]), so that only this config
    /// and the arguments decide. For tests; the IDE respects the user's
    /// environment.
    pub hermetic: bool,
}

impl Default for UvConfig {
    fn default() -> Self {
        Self {
            offline: false,
            python_downloads: true,
            no_config: false,
            cache_dir: None,
            python_install_dir: None,
            hermetic: false,
        }
    }
}

/// The uv command-line tool. Every operation runs uv as a child process and
/// never reimplements what it does.
#[derive(Debug, Clone)]
pub struct Uv {
    program: PathBuf,
    config: UvConfig,
}

/// An interpreter known to uv: installed, or available for download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UvPython {
    pub key: String,
    pub version: String,
    /// `None` when the interpreter is not installed.
    pub path: Option<PathBuf>,
    /// The managed installation a shim in `path` points to.
    #[serde(default)]
    pub symlink: Option<PathBuf>,
    /// Download location of an interpreter that is not installed.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub implementation: String,
    #[serde(default)]
    pub variant: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub os: String,
}

impl UvPython {
    pub fn installed(&self) -> bool {
        self.path.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitKind {
    /// uv's default: an application.
    #[default]
    App,
    Lib,
    /// An application built as a package.
    Package,
    /// Only a `pyproject.toml`.
    Bare,
    /// A single script with inline metadata (`path` is the file).
    Script,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InitRequest {
    /// The project directory (or the script file for [`InitKind::Script`]).
    pub path: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: InitKind,
    /// Python version or interpreter for `requires-python`.
    #[serde(default)]
    pub python: Option<String>,
    /// `uv`, `hatch`, `flit`, `pdm`, `poetry`, `setuptools`, `maturin`, `scikit`.
    #[serde(default)]
    pub build_backend: Option<String>,
    #[serde(default)]
    pub no_readme: bool,
    /// Skip the `.python-version` file.
    #[serde(default)]
    pub no_pin_python: bool,
    /// Let uv run `git init`. Off by default: version control is the IDE's decision.
    #[serde(default)]
    pub vcs: bool,
    /// Do not look for a project or workspace around the folder
    /// (`--no-project`), as an interpreter set up for a folder does.
    #[serde(default)]
    pub no_project: bool,
    /// Where uv runs; `path` may then be empty (the folder itself).
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VenvRequest {
    pub path: PathBuf,
    /// A version (`3.12`), an implementation (`pypy@3.10`) or an interpreter path.
    #[serde(default)]
    pub python: Option<String>,
    /// Install pip, setuptools and wheel into the new environment.
    #[serde(default)]
    pub seed: bool,
    /// Replace an existing environment.
    #[serde(default)]
    pub clear: bool,
    #[serde(default)]
    pub system_site_packages: bool,
    #[serde(default)]
    pub prompt: Option<String>,
    /// Let `clear` remove a folder that is not a virtual environment
    /// (`--force`).
    #[serde(default)]
    pub force: bool,
    /// Where uv runs: it reads `.python-version` and `requires-python`
    /// of the project there.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncRequest {
    #[serde(default)]
    pub project: PathBuf,
    #[serde(default)]
    pub extras: Vec<String>,
    #[serde(default)]
    pub all_extras: bool,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub all_groups: bool,
    #[serde(default)]
    pub no_dev: bool,
    /// Do not update `uv.lock`, use it as is.
    #[serde(default)]
    pub frozen: bool,
    /// Fail when `uv.lock` is out of date.
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub python: Option<String>,
    /// The environment to sync instead of the project's default one
    /// (`UV_PROJECT_ENVIRONMENT`, and `VIRTUAL_ENV` for it).
    #[serde(default)]
    pub environment: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LockRequest {
    #[serde(default)]
    pub project: PathBuf,
    /// Allow all packages to be upgraded.
    #[serde(default)]
    pub upgrade: bool,
    #[serde(default)]
    pub upgrade_packages: Vec<String>,
    #[serde(default)]
    pub python: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AddRequest {
    #[serde(default)]
    pub project: PathBuf,
    /// Requirements: `requests`, `rich>=13`, `./local`.
    pub packages: Vec<String>,
    /// A `[dependency-groups]` table.
    #[serde(default)]
    pub group: Option<String>,
    /// A `[project.optional-dependencies]` extra.
    #[serde(default)]
    pub optional: Option<String>,
    /// The `dev` group.
    #[serde(default)]
    pub dev: bool,
    /// Edit the inline metadata of this script instead of a project.
    #[serde(default)]
    pub script: Option<PathBuf>,
    /// Edit `pyproject.toml` only: no lock, no install.
    #[serde(default)]
    pub no_sync: bool,
    /// Edit `pyproject.toml` without re-locking (implies no sync).
    #[serde(default)]
    pub frozen: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoveRequest {
    #[serde(default)]
    pub project: PathBuf,
    pub packages: Vec<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub optional: Option<String>,
    #[serde(default)]
    pub dev: bool,
    #[serde(default)]
    pub script: Option<PathBuf>,
    #[serde(default)]
    pub no_sync: bool,
    #[serde(default)]
    pub frozen: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TreeRequest {
    #[serde(default)]
    pub project: PathBuf,
    #[serde(default)]
    pub depth: Option<u32>,
    /// Show the newest available version next to each package (needs the index).
    #[serde(default)]
    pub outdated: bool,
    #[serde(default)]
    pub no_dev: bool,
    /// Use `uv.lock` as is.
    #[serde(default)]
    pub frozen: bool,
    /// Only the dependencies of this package.
    #[serde(default)]
    pub package: Option<String>,
}

/// `uv pip install` into an environment that is not a uv project.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipInstallRequest {
    /// The interpreter (or environment directory) to install into.
    pub python: PathBuf,
    pub packages: Vec<String>,
    /// Install from a requirements file.
    #[serde(default)]
    pub requirements: Option<PathBuf>,
    /// Upgrade the named packages to the newest allowed versions.
    #[serde(default)]
    pub upgrade: bool,
}

/// One package in the output of `uv tree`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeNode {
    pub name: String,
    pub version: Option<String>,
    /// Annotations: `*` (already shown above), `extra: security`, `group: dev`, `latest: v2.0`.
    pub notes: Vec<String>,
    pub children: Vec<TreeNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    pub roots: Vec<TreeNode>,
    /// The tool's own rendering, for a log pane.
    pub text: String,
}

impl Uv {
    pub fn new(program: impl Into<PathBuf>, config: UvConfig) -> Self {
        Self {
            program: program.into(),
            config,
        }
    }

    /// uv from `PYOXIDE_UV`, `PATH` or the places the installers use.
    pub fn locate(config: UvConfig) -> Option<Uv> {
        let program = std::env::var_os(brand::env_name!("UV"))
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| find_tool("uv"))?;
        Some(Uv::new(program, config))
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn config(&self) -> &UvConfig {
        &self.config
    }

    fn spec(&self, args: Vec<OsString>) -> Spec {
        let mut spec = Spec::new(&self.program).args(args);
        if self.config.hermetic {
            spec = spec.without_steering_env();
        }
        spec = spec.env("NO_COLOR", "1").env("UV_NO_PROGRESS", "1");
        if self.config.offline {
            spec = spec.env("UV_OFFLINE", "1");
        }
        if !self.config.python_downloads {
            spec = spec.env("UV_PYTHON_DOWNLOADS", "never");
        }
        if self.config.no_config {
            spec = spec.env("UV_NO_CONFIG", "1");
        }
        if let Some(dir) = &self.config.python_install_dir {
            spec = spec.env("UV_PYTHON_INSTALL_DIR", dir);
        }
        if let Some(dir) = &self.config.cache_dir {
            spec = spec.env("UV_CACHE_DIR", dir);
        }
        spec
    }

    /// Run uv, with its progress lines going to the context's sink.
    async fn exec(
        &self,
        args: Vec<OsString>,
        cwd: Option<&Path>,
        environment: Option<&Path>,
        ctx: &RunContext,
        timeout: Duration,
    ) -> Result<(Finished, String)> {
        let mut spec = self.spec(args);
        spec.forward_stdout = true;
        if let Some(cwd) = cwd {
            spec = spec.cwd(cwd);
        }
        if let Some(env) = environment {
            spec = spec
                .env("UV_PROJECT_ENVIRONMENT", env)
                .env("VIRTUAL_ENV", env);
        }
        let command = spec.display();
        run_ok("uv", spec, ctx, timeout).await.map(|f| (f, command))
    }

    /// Like `exec`, for commands whose stdout is data: only stderr is forwarded.
    async fn query(
        &self,
        args: Vec<OsString>,
        cwd: Option<&Path>,
        ctx: &RunContext,
        timeout: Duration,
    ) -> Result<Finished> {
        let mut spec = self.spec(args);
        if let Some(cwd) = cwd {
            spec = spec.cwd(cwd);
        }
        run_ok("uv", spec, ctx, timeout).await
    }

    async fn change(
        &self,
        args: Vec<OsString>,
        cwd: Option<&Path>,
        ctx: &RunContext,
        timeout: Duration,
    ) -> Result<Outcome> {
        let started = Instant::now();
        let (done, command) = self.exec(args, cwd, None, ctx, timeout).await?;
        Ok(Outcome::new(command, &done.stdout, &done.stderr, started))
    }

    /// `0.11.25`
    pub async fn version(&self, ctx: &RunContext) -> Result<String> {
        let done = self.query(args(["--version"]), None, ctx, QUICK).await?;
        parse_version(&done.stdout).ok_or_else(|| {
            EnvError::parse(format!(
                "unexpected `uv --version` output: {}",
                done.stdout.trim()
            ))
        })
    }

    /// Interpreters uv knows: installed ones and, unless `installed_only`,
    /// those it can download.
    pub async fn python_list(
        &self,
        installed_only: bool,
        ctx: &RunContext,
    ) -> Result<Vec<UvPython>> {
        let mut a = args(["python", "list", "--output-format", "json"]);
        if installed_only {
            a.push("--only-installed".into());
        }
        let done = self.query(a, None, ctx, QUICK).await?;
        parse_python_list(&done.stdout)
    }

    /// The Python versions (`3.14`, `3.13`, …) uv can create an
    /// environment with, installed or downloadable, newest first; `request`
    /// narrows them (`>=3.11`, the project's `requires-python`).
    pub async fn python_versions(
        &self,
        request: Option<&str>,
        cwd: Option<&Path>,
        ctx: &RunContext,
    ) -> Result<Vec<MinorVersion>> {
        let mut a = args(["python", "list", "--output-format", "json"]);
        if let Some(request) = request.filter(|r| !r.trim().is_empty()) {
            a.push(request.into());
        }
        let done = self.query(a, cwd, ctx, QUICK).await?;
        Ok(minor_versions(&parse_python_list(&done.stdout)?))
    }

    /// The interpreters uv can download for this machine, installed or not
    /// (`uv python list --only-downloads`).
    pub async fn python_downloads(&self, ctx: &RunContext) -> Result<Vec<UvPython>> {
        let a = args([
            "python",
            "list",
            "--only-downloads",
            "--output-format",
            "json",
        ]);
        let done = self.query(a, None, ctx, QUICK).await?;
        parse_python_list(&done.stdout)
    }

    /// The executable of an interpreter uv installed that matches
    /// `request` (`3.13.5`): `uv python find --managed-python --no-project`.
    pub async fn python_find(&self, request: &str, ctx: &RunContext) -> Result<PathBuf> {
        let mut a = args(["python", "find", "--managed-python", "--no-project"]);
        a.push(request.into());
        let done = self.query(a, None, ctx, QUICK).await?;
        done.stdout
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| EnvError::parse("`uv python find` printed nothing"))
    }

    /// uv's directory of managed interpreters.
    pub async fn python_dir(&self, ctx: &RunContext) -> Result<PathBuf> {
        let done = self
            .query(args(["python", "dir"]), None, ctx, QUICK)
            .await?;
        let line = done.stdout.trim();
        if line.is_empty() {
            return Err(EnvError::parse("`uv python dir` printed nothing"));
        }
        Ok(PathBuf::from(line))
    }

    /// `uv python install 3.12 3.13`
    pub async fn python_install(
        &self,
        versions: &[String],
        reinstall: bool,
        ctx: &RunContext,
    ) -> Result<Outcome> {
        if versions.is_empty() {
            return Err(EnvError::invalid(
                "name at least one Python version to install",
            ));
        }
        self.change(python_install_args(versions, reinstall), None, ctx, LONG)
            .await
    }

    /// `uv venv`
    pub async fn venv(&self, req: &VenvRequest, ctx: &RunContext) -> Result<Outcome> {
        self.change(
            venv_args(req),
            req.cwd.as_deref(),
            ctx,
            Duration::from_secs(10 * 60),
        )
        .await
    }

    /// `uv init`
    pub async fn init(&self, req: &InitRequest, ctx: &RunContext) -> Result<Outcome> {
        self.change(init_args(req), req.cwd.as_deref(), ctx, SHORT)
            .await
    }

    /// `uv sync`
    pub async fn sync(&self, req: &SyncRequest, ctx: &RunContext) -> Result<Outcome> {
        let started = Instant::now();
        let (done, command) = self
            .exec(
                sync_args(req),
                Some(&req.project),
                req.environment.as_deref(),
                ctx,
                LONG,
            )
            .await?;
        Ok(Outcome::new(command, &done.stdout, &done.stderr, started))
    }

    /// `uv lock`
    pub async fn lock(&self, req: &LockRequest, ctx: &RunContext) -> Result<Outcome> {
        self.change(lock_args(req), Some(&req.project), ctx, LONG)
            .await
    }

    /// `uv add`
    pub async fn add(&self, req: &AddRequest, ctx: &RunContext) -> Result<Outcome> {
        if req.packages.is_empty() {
            return Err(EnvError::invalid("name at least one package to add"));
        }
        self.change(add_args(req)?, Some(&req.project), ctx, LONG)
            .await
    }

    /// `uv remove`
    pub async fn remove(&self, req: &RemoveRequest, ctx: &RunContext) -> Result<Outcome> {
        if req.packages.is_empty() {
            return Err(EnvError::invalid("name at least one package to remove"));
        }
        self.change(remove_args(req)?, Some(&req.project), ctx, LONG)
            .await
    }

    /// `uv tree`
    pub async fn tree(&self, req: &TreeRequest, ctx: &RunContext) -> Result<Tree> {
        let done = self
            .query(tree_args(req), Some(&req.project), ctx, SHORT)
            .await?;
        Ok(Tree {
            roots: parse_tree(&done.stdout),
            text: done.stdout,
        })
    }

    /// `uv pip list --format json`, with `--outdated` when asked.
    pub async fn pip_list(
        &self,
        python: &Path,
        outdated: bool,
        ctx: &RunContext,
    ) -> Result<Vec<Package>> {
        let mut a = args(["pip", "list", "--format", "json", "--python"]);
        a.push(python.into());
        if outdated {
            a.push("--outdated".into());
        }
        let done = self
            .query(a, None, ctx, if outdated { SHORT } else { QUICK })
            .await?;
        parse_package_list(&done.stdout)
    }

    /// `uv pip install`
    pub async fn pip_install(&self, req: &PipInstallRequest, ctx: &RunContext) -> Result<Outcome> {
        if req.packages.is_empty() && req.requirements.is_none() {
            return Err(EnvError::invalid(
                "name a package or a requirements file to install",
            ));
        }
        self.change(pip_install_args(req), None, ctx, LONG).await
    }

    /// `uv pip uninstall`
    pub async fn pip_uninstall(
        &self,
        python: &Path,
        packages: &[String],
        ctx: &RunContext,
    ) -> Result<Outcome> {
        if packages.is_empty() {
            return Err(EnvError::invalid("name at least one package to uninstall"));
        }
        let mut a = args(["pip", "uninstall", "--python"]);
        a.push(python.into());
        a.extend(packages.iter().map(OsString::from));
        self.change(a, None, ctx, LONG).await
    }
}

fn args<const N: usize>(a: [&str; N]) -> Vec<OsString> {
    a.into_iter().map(OsString::from).collect()
}

fn push_opt(a: &mut Vec<OsString>, flag: &str, value: &Option<String>) {
    if let Some(v) = value {
        a.push(flag.into());
        a.push(v.into());
    }
}

pub(crate) fn python_install_args(versions: &[String], reinstall: bool) -> Vec<OsString> {
    let mut a = args(["python", "install"]);
    if reinstall {
        a.push("--reinstall".into());
    }
    a.extend(versions.iter().map(OsString::from));
    a
}

pub(crate) fn venv_args(req: &VenvRequest) -> Vec<OsString> {
    let mut a = args(["venv"]);
    push_opt(&mut a, "--python", &req.python);
    push_opt(&mut a, "--prompt", &req.prompt);
    if req.seed {
        a.push("--seed".into());
    }
    if req.clear {
        a.push("--clear".into());
    }
    if req.system_site_packages {
        a.push("--system-site-packages".into());
    }
    if req.force {
        a.push("--force".into());
    }
    a.push(req.path.clone().into());
    a
}

/// A `major.minor` Python version uv knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinorVersion {
    pub major: u32,
    pub minor: u32,
    /// One of its interpreters is on the machine; else uv downloads one.
    pub installed: bool,
}

/// `3.13.5` as numbers; `None` for anything but a final release.
fn release(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let micro = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, micro))
}

/// The default build (not free-threaded, not a debug one).
fn default_variant(p: &UvPython) -> bool {
    p.variant.is_empty() || p.variant == "default"
}

/// The `major.minor` versions of final releases among `list`, each once,
/// newest first, installed when one of its interpreters is
/// (`UvLowLevel.listSupportedPythonVersions` takes the `X.Y.Z-` keys of
/// `uv python list`: no pre-releases, no free-threaded builds).
pub fn minor_versions(list: &[UvPython]) -> Vec<MinorVersion> {
    let mut out: Vec<MinorVersion> = Vec::new();
    for p in list.iter().filter(|p| default_variant(p)) {
        let Some((major, minor, _)) = release(&p.version) else {
            continue;
        };
        match out
            .iter_mut()
            .find(|v| (v.major, v.minor) == (major, minor))
        {
            Some(v) => v.installed |= p.installed(),
            None => out.push(MinorVersion {
                major,
                minor,
                installed: p.installed(),
            }),
        }
    }
    out.sort_by_key(|v| std::cmp::Reverse((v.major, v.minor)));
    out
}

/// The CPython releases among uv's downloads (`list`) worth offering: the
/// newest final release of each `major.minor` from `oldest` on, newest
/// first (`3.14.6`, `3.13.14`, …).
pub fn downloadable_releases(list: &[UvPython], oldest: (u32, u32)) -> Vec<String> {
    let mut newest: Vec<(u32, u32, u32)> = Vec::new();
    for p in list
        .iter()
        .filter(|p| default_variant(p) && p.implementation.eq_ignore_ascii_case("cpython"))
    {
        let Some(v) = release(&p.version) else {
            continue;
        };
        if (v.0, v.1) < oldest {
            continue;
        }
        match newest.iter_mut().find(|n| (n.0, n.1) == (v.0, v.1)) {
            Some(n) => *n = (*n).max(v),
            None => newest.push(v),
        }
    }
    newest.sort_by(|a, b| b.cmp(a));
    newest
        .into_iter()
        .map(|(a, b, c)| format!("{a}.{b}.{c}"))
        .collect()
}

pub(crate) fn init_args(req: &InitRequest) -> Vec<OsString> {
    let mut a = args(["init"]);
    push_opt(&mut a, "--name", &req.name);
    push_opt(&mut a, "--python", &req.python);
    push_opt(&mut a, "--build-backend", &req.build_backend);
    match req.kind {
        InitKind::App => {}
        InitKind::Lib => a.push("--lib".into()),
        InitKind::Package => a.push("--package".into()),
        InitKind::Bare => a.push("--bare".into()),
        InitKind::Script => a.push("--script".into()),
    }
    if req.no_readme {
        a.push("--no-readme".into());
    }
    if req.no_pin_python {
        a.push("--no-pin-python".into());
    }
    if !req.vcs {
        a.push("--vcs".into());
        a.push("none".into());
    }
    if req.no_project {
        a.push("--no-project".into());
    }
    if !req.path.as_os_str().is_empty() {
        a.push(req.path.clone().into());
    }
    a
}

pub(crate) fn sync_args(req: &SyncRequest) -> Vec<OsString> {
    let mut a = args(["sync"]);
    for e in &req.extras {
        a.push("--extra".into());
        a.push(e.into());
    }
    if req.all_extras {
        a.push("--all-extras".into());
    }
    for g in &req.groups {
        a.push("--group".into());
        a.push(g.into());
    }
    if req.all_groups {
        a.push("--all-groups".into());
    }
    if req.no_dev {
        a.push("--no-dev".into());
    }
    if req.frozen {
        a.push("--frozen".into());
    }
    if req.locked {
        a.push("--locked".into());
    }
    push_opt(&mut a, "--python", &req.python);
    a
}

pub(crate) fn lock_args(req: &LockRequest) -> Vec<OsString> {
    let mut a = args(["lock"]);
    if req.upgrade {
        a.push("--upgrade".into());
    }
    for p in &req.upgrade_packages {
        a.push("--upgrade-package".into());
        a.push(p.into());
    }
    push_opt(&mut a, "--python", &req.python);
    a
}

/// Where a dependency goes. `uv` rejects combinations of these itself, but a
/// clear message beats a usage error.
fn target_flags(
    a: &mut Vec<OsString>,
    group: &Option<String>,
    optional: &Option<String>,
    dev: bool,
    script: &Option<PathBuf>,
) -> Result<()> {
    let chosen = [group.is_some(), optional.is_some(), dev]
        .iter()
        .filter(|b| **b)
        .count();
    if chosen > 1 {
        return Err(EnvError::invalid(
            "choose one place for the dependency: a group, an optional extra or dev",
        ));
    }
    if script.is_some() && chosen > 0 {
        return Err(EnvError::invalid(
            "groups and extras do not apply to script dependencies",
        ));
    }
    push_opt(a, "--group", group);
    push_opt(a, "--optional", optional);
    if dev {
        a.push("--dev".into());
    }
    if let Some(s) = script {
        a.push("--script".into());
        a.push(s.into());
    }
    Ok(())
}

pub(crate) fn add_args(req: &AddRequest) -> Result<Vec<OsString>> {
    let mut a = args(["add"]);
    target_flags(&mut a, &req.group, &req.optional, req.dev, &req.script)?;
    if req.no_sync {
        a.push("--no-sync".into());
    }
    if req.frozen {
        a.push("--frozen".into());
    }
    // A requirement starting with `-` must not be taken for a flag.
    a.push("--".into());
    a.extend(req.packages.iter().map(OsString::from));
    Ok(a)
}

pub(crate) fn remove_args(req: &RemoveRequest) -> Result<Vec<OsString>> {
    let mut a = args(["remove"]);
    target_flags(&mut a, &req.group, &req.optional, req.dev, &req.script)?;
    if req.no_sync {
        a.push("--no-sync".into());
    }
    if req.frozen {
        a.push("--frozen".into());
    }
    a.push("--".into());
    a.extend(req.packages.iter().map(OsString::from));
    Ok(a)
}

pub(crate) fn tree_args(req: &TreeRequest) -> Vec<OsString> {
    let mut a = args(["tree"]);
    if let Some(d) = req.depth {
        a.push("--depth".into());
        a.push(d.to_string().into());
    }
    if req.outdated {
        a.push("--outdated".into());
    }
    if req.no_dev {
        a.push("--no-dev".into());
    }
    if req.frozen {
        a.push("--frozen".into());
    }
    push_opt(&mut a, "--package", &req.package);
    a
}

pub(crate) fn pip_install_args(req: &PipInstallRequest) -> Vec<OsString> {
    let mut a = args(["pip", "install", "--python"]);
    a.push(req.python.clone().into());
    if req.upgrade {
        a.push("--upgrade".into());
    }
    if let Some(r) = &req.requirements {
        a.push("--requirements".into());
        a.push(r.into());
    }
    if !req.packages.is_empty() {
        a.push("--".into());
        a.extend(req.packages.iter().map(OsString::from));
    }
    a
}

/// `uv 0.11.25 (1fc7de7c4 2026-06-26 aarch64-apple-darwin)` to `0.11.25`.
pub(crate) fn parse_version(output: &str) -> Option<String> {
    let line = output.lines().next()?.trim();
    let v = line.strip_prefix("uv ")?.split_whitespace().next()?;
    v.starts_with(|c: char| c.is_ascii_digit())
        .then(|| v.to_owned())
}

pub(crate) fn parse_python_list(json: &str) -> Result<Vec<UvPython>> {
    serde_json::from_str(json.trim())
        .map_err(|e| EnvError::parse(format!("unreadable `uv python list` output: {e}")))
}

/// Parse the box-drawing tree printed by `uv tree`.
pub(crate) fn parse_tree(text: &str) -> Vec<TreeNode> {
    // (depth, node) stack of the current path from a root.
    let mut roots: Vec<TreeNode> = Vec::new();
    let mut path: Vec<usize> = Vec::new(); // child indexes from the last root
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let indent = line
            .chars()
            .take_while(|c| matches!(c, '│' | '├' | '└' | '─' | ' ' | '\u{a0}'))
            .count();
        let depth = indent / 4;
        let node = parse_tree_line(&line.chars().skip(indent).collect::<String>());
        if depth == 0 {
            roots.push(node);
            path.clear();
            continue;
        }
        let Some(root) = roots.last_mut() else {
            continue;
        };
        path.truncate(depth - 1);
        let mut parent = root;
        for &i in &path {
            parent = &mut parent.children[i];
        }
        parent.children.push(node);
        path.push(parent.children.len() - 1);
    }
    roots
}

fn parse_tree_line(content: &str) -> TreeNode {
    let content = content.trim();
    let mut parts = content.splitn(2, ' ');
    let name = parts.next().unwrap_or_default().to_owned();
    let mut rest = parts.next().unwrap_or_default().trim();
    let mut version = None;
    if let Some(r) = rest.strip_prefix('v') {
        let end = r.find(' ').unwrap_or(r.len());
        if r[..end].starts_with(|c: char| c.is_ascii_digit()) {
            version = Some(r[..end].to_owned());
            rest = r[end..].trim();
        }
    }
    let mut notes = Vec::new();
    let mut s = rest;
    while let Some(open) = s.find(['(', '[']) {
        let close_ch = if s.as_bytes()[open] == b'(' { ')' } else { ']' };
        let Some(close) = s[open..].find(close_ch) else {
            break;
        };
        notes.push(s[open + 1..open + close].trim().to_owned());
        s = &s[open + close + 1..];
    }
    TreeNode {
        name,
        version,
        notes,
        children: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: Vec<OsString>) -> Vec<String> {
        v.into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn configuration_becomes_environment_variables() {
        let env_of = |uv: &Uv| -> Vec<(String, String)> {
            uv.spec(vec![])
                .env
                .into_iter()
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.to_string_lossy().into_owned(),
                    )
                })
                .collect()
        };
        let plain = env_of(&Uv::new("uv", UvConfig::default()));
        assert_eq!(
            plain,
            [
                ("NO_COLOR".to_owned(), "1".to_owned()),
                ("UV_NO_PROGRESS".to_owned(), "1".to_owned())
            ]
        );
        let strict = env_of(&Uv::new(
            "uv",
            UvConfig {
                offline: true,
                python_downloads: false,
                no_config: true,
                cache_dir: Some("/c".into()),
                python_install_dir: Some("/p".into()),
                hermetic: false,
            },
        ));
        for (k, v) in [
            ("UV_OFFLINE", "1"),
            ("UV_PYTHON_DOWNLOADS", "never"),
            ("UV_NO_CONFIG", "1"),
            ("UV_CACHE_DIR", "/c"),
            ("UV_PYTHON_INSTALL_DIR", "/p"),
        ] {
            assert!(strict.contains(&(k.to_owned(), v.to_owned())), "{k}");
        }
    }

    #[test]
    fn only_a_hermetic_uv_hides_the_callers_steering_variables() {
        let default = Uv::new("uv", UvConfig::default());
        assert!(default.spec(vec![]).env_remove.is_empty());
        let hermetic = Uv::new(
            "uv",
            UvConfig {
                hermetic: true,
                ..UvConfig::default()
            },
        );
        assert_eq!(
            hermetic.spec(vec![]).env_remove,
            crate::process::steering_env()
        );
    }

    #[test]
    fn parses_uv_version() {
        assert_eq!(
            parse_version("uv 0.11.25 (1fc7de7c4 2026-06-26 aarch64-apple-darwin)\n").as_deref(),
            Some("0.11.25")
        );
        assert_eq!(parse_version("uv 0.4.0\n").as_deref(), Some("0.4.0"));
        assert_eq!(parse_version("pip 24"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn parses_python_list_output() {
        let json = r#"[
          {"key":"cpython-3.13.14-macos-aarch64-none","version":"3.13.14","version_parts":{"major":3,"minor":13,"patch":14},
           "path":"/home/u/.local/bin/python3.13","symlink":"/home/u/.local/share/uv/python/cpython-3.13-macos-aarch64-none/bin/python3.13",
           "url":null,"os":"macos","variant":"default","implementation":"cpython","arch":"aarch64","libc":"none"},
          {"key":"cpython-3.11.15-macos-aarch64-none","version":"3.11.15","version_parts":{"major":3,"minor":11,"patch":15},
           "path":null,"symlink":null,"url":"https://example.org/cpython-3.11.15.tar.gz","os":"macos","variant":"default",
           "implementation":"cpython","arch":"aarch64","libc":"none"}
        ]"#;
        let list = parse_python_list(json).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].installed());
        assert_eq!(list[0].version, "3.13.14");
        assert!(!list[1].installed());
        assert_eq!(
            list[1].url.as_deref(),
            Some("https://example.org/cpython-3.11.15.tar.gz")
        );
        assert!(parse_python_list("not json").is_err());
        assert!(parse_python_list("[]").unwrap().is_empty());
    }

    #[test]
    fn venv_command_line() {
        let req = VenvRequest {
            path: ".venv".into(),
            python: Some("3.12".into()),
            seed: true,
            clear: true,
            ..VenvRequest::default()
        };
        assert_eq!(
            strs(venv_args(&req)),
            ["venv", "--python", "3.12", "--seed", "--clear", ".venv"]
        );
        let plain = VenvRequest {
            path: "/p/v".into(),
            ..VenvRequest::default()
        };
        assert_eq!(strs(venv_args(&plain)), ["venv", "/p/v"]);
        // The reference's retry after a failed `uv venv` of a folder.
        let forced = VenvRequest {
            path: "/p/.venv".into(),
            clear: true,
            force: true,
            system_site_packages: true,
            cwd: Some("/p".into()),
            ..VenvRequest::default()
        };
        assert_eq!(
            strs(venv_args(&forced)),
            [
                "venv",
                "--clear",
                "--system-site-packages",
                "--force",
                "/p/.venv"
            ]
        );
    }

    fn uv_python(implementation: &str, version: &str, variant: &str, path: bool) -> UvPython {
        UvPython {
            key: format!("{implementation}-{version}-macos"),
            version: version.into(),
            path: path.then(|| PathBuf::from(format!("/uv/{version}/bin/python3"))),
            symlink: None,
            url: None,
            implementation: implementation.into(),
            variant: variant.into(),
            arch: "aarch64".into(),
            os: "macos".into(),
        }
    }

    #[test]
    fn minor_versions_of_final_releases_newest_first() {
        let python =
            |version: &str, variant: &str, path: bool| uv_python("cpython", version, variant, path);
        let list = [
            python("3.15.0a1", "default", false),
            python("3.14.0", "default", false),
            python("3.14.0", "freethreaded", true),
            python("3.12.13", "default", false),
            python("3.13.14", "default", false),
            python("3.13.12", "", true),
            python("3.11.13", "default", false),
        ];
        let versions: Vec<_> = minor_versions(&list)
            .iter()
            .map(|v| (v.major, v.minor, v.installed))
            .collect();
        // Installed: one of its default builds is.
        assert_eq!(
            versions,
            [
                (3, 14, false),
                (3, 13, true),
                (3, 12, false),
                (3, 11, false)
            ]
        );
        assert!(minor_versions(&[]).is_empty());
    }

    #[test]
    fn downloadable_releases_are_the_newest_cpython_of_each_minor_version() {
        let list = [
            uv_python("cpython", "3.15.0b3", "default", false),
            uv_python("cpython", "3.14.6", "default", false),
            uv_python("cpython", "3.14.6", "freethreaded", false),
            uv_python("cpython", "3.13.13", "default", false),
            uv_python("cpython", "3.13.14", "default", false),
            uv_python("cpython", "3.12.13", "default", true),
            uv_python("cpython", "3.7.9", "default", false),
            uv_python("pypy", "3.11.15", "default", false),
            uv_python("graalpy", "3.12.0", "default", false),
        ];
        assert_eq!(
            downloadable_releases(&list, (3, 8)),
            ["3.14.6", "3.13.14", "3.12.13"]
        );
        assert!(downloadable_releases(&list, (3, 15)).is_empty());
    }

    #[test]
    fn init_command_line_skips_vcs_by_default() {
        let req = InitRequest {
            path: "proj".into(),
            name: Some("demo".into()),
            kind: InitKind::Lib,
            python: Some("3.13".into()),
            no_readme: true,
            ..InitRequest::default()
        };
        assert_eq!(
            strs(init_args(&req)),
            [
                "init",
                "--name",
                "demo",
                "--python",
                "3.13",
                "--lib",
                "--no-readme",
                "--vcs",
                "none",
                "proj"
            ]
        );
        let with_vcs = InitRequest {
            path: "p".into(),
            vcs: true,
            kind: InitKind::Script,
            ..InitRequest::default()
        };
        assert_eq!(strs(init_args(&with_vcs)), ["init", "--script", "p"]);
        // An interpreter set up for a folder without `pyproject.toml`
        // (`UvLowLevel.initializeEnvironment`): in the folder itself.
        let bare = InitRequest {
            name: Some("demo".into()),
            kind: InitKind::Bare,
            python: Some("3.13".into()),
            no_project: true,
            cwd: Some("/w/demo".into()),
            ..InitRequest::default()
        };
        assert_eq!(
            strs(init_args(&bare)),
            [
                "init",
                "--name",
                "demo",
                "--python",
                "3.13",
                "--bare",
                "--vcs",
                "none",
                "--no-project"
            ]
        );
    }

    #[test]
    fn sync_and_lock_command_lines() {
        let req = SyncRequest {
            project: ".".into(),
            extras: vec!["fancy".into()],
            groups: vec!["dev".into(), "docs".into()],
            all_extras: true,
            no_dev: true,
            frozen: true,
            python: Some("3.12".into()),
            ..SyncRequest::default()
        };
        assert_eq!(
            strs(sync_args(&req)),
            [
                "sync",
                "--extra",
                "fancy",
                "--all-extras",
                "--group",
                "dev",
                "--group",
                "docs",
                "--no-dev",
                "--frozen",
                "--python",
                "3.12"
            ]
        );
        let lock = LockRequest {
            project: ".".into(),
            upgrade_packages: vec!["requests".into()],
            upgrade: true,
            ..LockRequest::default()
        };
        assert_eq!(
            strs(lock_args(&lock)),
            ["lock", "--upgrade", "--upgrade-package", "requests"]
        );
    }

    #[test]
    fn add_and_remove_command_lines() {
        let req = AddRequest {
            project: ".".into(),
            packages: vec!["requests".into(), "rich>=13".into()],
            group: Some("dev".into()),
            no_sync: true,
            ..AddRequest::default()
        };
        assert_eq!(
            strs(add_args(&req).unwrap()),
            [
                "add",
                "--group",
                "dev",
                "--no-sync",
                "--",
                "requests",
                "rich>=13"
            ]
        );
        let rm = RemoveRequest {
            project: ".".into(),
            packages: vec!["rich".into()],
            optional: Some("fancy".into()),
            frozen: true,
            ..RemoveRequest::default()
        };
        assert_eq!(
            strs(remove_args(&rm).unwrap()),
            ["remove", "--optional", "fancy", "--frozen", "--", "rich"]
        );
        let script = AddRequest {
            project: ".".into(),
            packages: vec!["httpx".into()],
            script: Some("tool.py".into()),
            ..AddRequest::default()
        };
        assert_eq!(
            strs(add_args(&script).unwrap()),
            ["add", "--script", "tool.py", "--", "httpx"]
        );
    }

    #[test]
    fn add_rejects_conflicting_targets() {
        let both = AddRequest {
            project: ".".into(),
            packages: vec!["x".into()],
            group: Some("a".into()),
            dev: true,
            ..AddRequest::default()
        };
        assert!(
            add_args(&both)
                .unwrap_err()
                .message
                .contains("choose one place")
        );
        let script_group = AddRequest {
            project: ".".into(),
            packages: vec!["x".into()],
            optional: Some("e".into()),
            script: Some("s.py".into()),
            ..AddRequest::default()
        };
        assert!(add_args(&script_group).is_err());
    }

    #[test]
    fn pip_install_command_line() {
        let req = PipInstallRequest {
            python: "/v/bin/python".into(),
            packages: vec!["requests".into()],
            requirements: Some("requirements.txt".into()),
            upgrade: true,
        };
        assert_eq!(
            strs(pip_install_args(&req)),
            [
                "pip",
                "install",
                "--python",
                "/v/bin/python",
                "--upgrade",
                "--requirements",
                "requirements.txt",
                "--",
                "requests"
            ]
        );
    }

    #[test]
    fn tree_command_line() {
        let req = TreeRequest {
            project: ".".into(),
            depth: Some(2),
            outdated: true,
            frozen: true,
            package: Some("requests".into()),
            ..TreeRequest::default()
        };
        assert_eq!(
            strs(tree_args(&req)),
            [
                "tree",
                "--depth",
                "2",
                "--outdated",
                "--frozen",
                "--package",
                "requests"
            ]
        );
    }

    const TREE: &str = "\
demo v0.1.0
├── requests v2.32.5
│   ├── certifi v2024.8.30
│   ├── charset-normalizer v3.4.0
│   ├── idna v3.10
│   └── urllib3 v2.2.3
└── rich v13.9.4 (extra: fancy)
    ├── markdown-it-py v3.0.0
    │   └── mdurl v0.1.2
    └── pygments v2.18.0 (*)
dev-tool v1.0 (group: dev) (latest: v1.1)
";

    #[test]
    fn parses_uv_tree() {
        let roots = parse_tree(TREE);
        assert_eq!(roots.len(), 2);
        let demo = &roots[0];
        assert_eq!(demo.name, "demo");
        assert_eq!(demo.version.as_deref(), Some("0.1.0"));
        assert_eq!(demo.children.len(), 2);
        let requests = &demo.children[0];
        assert_eq!(requests.name, "requests");
        assert_eq!(
            requests
                .children
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["certifi", "charset-normalizer", "idna", "urllib3"]
        );
        let rich = &demo.children[1];
        assert_eq!(rich.notes, ["extra: fancy"]);
        assert_eq!(rich.children.len(), 2);
        assert_eq!(rich.children[0].children[0].name, "mdurl");
        assert_eq!(rich.children[1].notes, ["*"]);
        let dev = &roots[1];
        assert_eq!(dev.notes, ["group: dev", "latest: v1.1"]);
        assert!(dev.children.is_empty());
    }

    #[test]
    fn tree_parser_tolerates_noise() {
        assert!(parse_tree("").is_empty());
        let roots = parse_tree("\n\nsolo\n");
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].version, None);
        // A child line without a root is ignored rather than panicking.
        assert!(parse_tree("├── orphan v1\n").is_empty());
    }
}
