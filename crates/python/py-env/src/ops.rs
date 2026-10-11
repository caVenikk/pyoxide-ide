//! The `env.*` operations as typed async functions. The CLI calls them
//! directly (with a progress sink); the command registry wraps the same
//! functions (see [`crate::commands`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::discovery::{self, DiscoveryOptions, DiscoveryReport};
use crate::error::{EnvError, ErrorKind, Result};
use crate::interpreter::{Interpreter, probe};
use crate::packages::{Backend, Outcome, Package};
use crate::paths::{environment_python, venv_python};
use crate::pip::Pip;
use crate::process::RunContext;
use crate::project::{ProjectInfo, find_project_root, find_project_venv, load_project_async};
use crate::search::IndexProject;
use crate::service::EnvService;
use crate::uv::{
    AddRequest, InitRequest, LockRequest, PipInstallRequest, RemoveRequest, SyncRequest, Tree,
    TreeRequest, VenvRequest,
};

// ---------- arguments and results ----------

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InterpretersArgs {
    /// Project directory; adds its `.venv`, `venv` and Poetry environment.
    #[serde(default)]
    pub project: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PathArgs {
    /// A directory in the project, its `pyproject.toml` or a `.py` script. Default: the current directory.
    #[serde(default)]
    pub path: Option<PathBuf>,
}

/// A change made by uv or pip.
#[derive(Debug, Clone, Serialize)]
pub struct Changed {
    pub backend: Backend,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Serialize)]
pub struct VenvCreated {
    pub path: PathBuf,
    pub python: PathBuf,
    /// The new environment's interpreter, when it could be probed.
    pub interpreter: Option<Interpreter>,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PackagesArgs {
    /// A virtual environment directory or interpreter. Default: the project's environment.
    #[serde(default)]
    pub venv: Option<PathBuf>,
    #[serde(default)]
    pub project: Option<PathBuf>,
    /// Only packages with a newer version on the index (needs the network).
    #[serde(default)]
    pub outdated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageList {
    pub backend: Backend,
    pub python: PathBuf,
    pub packages: Vec<Package>,
}

/// Add packages. With `venv`, install into that environment (`uv pip
/// install`, or `pip` without uv). Without it, add them to the project's
/// `pyproject.toml` (`uv add`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AddArgs {
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub venv: Option<PathBuf>,
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
    /// `venv` mode: install from a requirements file.
    #[serde(default)]
    pub requirements: Option<PathBuf>,
    /// `venv` mode: upgrade to the newest allowed versions.
    #[serde(default)]
    pub upgrade: bool,
}

/// Remove packages, the counterpart of [`AddArgs`].
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RemoveArgs {
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub venv: Option<PathBuf>,
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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PythonInstallArgs {
    pub versions: Vec<String>,
    #[serde(default)]
    pub reinstall: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchArgs {
    pub name: String,
}

// ---------- helpers ----------

/// An absolute path without touching the file system; an empty path is the
/// current directory.
pub(crate) fn absolute(p: &Path) -> PathBuf {
    if p.as_os_str().is_empty() {
        return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    }
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn project_root(p: Option<&Path>) -> PathBuf {
    find_project_root(&absolute(p.unwrap_or(Path::new(""))))
}

/// The interpreter of `venv` (a directory or an executable), or of the
/// project's environment.
pub fn resolve_python(
    svc: &EnvService,
    venv: Option<&Path>,
    project: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(v) = venv {
        let v = absolute(v);
        if v.is_file() {
            return Ok(v);
        }
        return environment_python(&v).ok_or_else(|| {
            EnvError::invalid(format!(
                "{} is not a Python environment (no interpreter in {})",
                v.display(),
                venv_python(&v).parent().unwrap_or(&v).display()
            ))
        });
    }
    let root = project_root(project);
    let env = svc.search_env();
    let dir = find_project_venv(&root, env.uv_project_environment.as_deref())
        .or_else(|| env.virtual_env.clone());
    dir.and_then(|d| environment_python(&d)).ok_or_else(|| {
        EnvError::new(
            ErrorKind::NoInterpreter,
            format!(
                "there is no virtual environment for the project at {}; create one with `env venv` or `env sync`",
                root.display()
            ),
        )
    })
}

// ---------- operations ----------

/// `env.interpreters.list`
pub async fn interpreters_list(
    svc: &EnvService,
    args: InterpretersArgs,
    ctx: &RunContext,
) -> Result<DiscoveryReport> {
    let mut opts = DiscoveryOptions::new(svc.search_env().clone(), svc.tools().clone());
    opts.project = args.project.map(|p| absolute(&p));
    discovery::discover(&opts, ctx, &|_| {}).await
}

/// `env.project.info`
pub async fn project_info(_svc: &EnvService, args: PathArgs) -> Result<ProjectInfo> {
    load_project_async(absolute(args.path.as_deref().unwrap_or(Path::new("")))).await
}

/// `env.project.init`
pub async fn project_init(
    svc: &EnvService,
    mut req: InitRequest,
    ctx: &RunContext,
) -> Result<Changed> {
    req.path = absolute(&req.path);
    let outcome = svc
        .require_uv("creating a project")?
        .init(&req, ctx)
        .await?;
    Ok(Changed {
        backend: Backend::Uv,
        outcome,
    })
}

/// `env.project.tree`
pub async fn project_tree(
    svc: &EnvService,
    mut req: TreeRequest,
    ctx: &RunContext,
) -> Result<Tree> {
    req.project = project_root(Some(&req.project));
    svc.require_uv("showing the dependency tree")?
        .tree(&req, ctx)
        .await
}

/// `env.venv.create`
pub async fn venv_create(
    svc: &EnvService,
    mut req: VenvRequest,
    ctx: &RunContext,
) -> Result<VenvCreated> {
    req.path = absolute(&req.path);
    let uv = svc.require_uv("creating a virtual environment")?;
    let outcome = uv.venv(&req, ctx).await?;
    let python = venv_python(&req.path);
    let interpreter = probe(
        &python,
        ".venv",
        Duration::from_secs(15),
        &RunContext::default(),
    )
    .await
    .ok();
    Ok(VenvCreated {
        path: req.path,
        python,
        interpreter,
        outcome,
    })
}

/// `env.packages.list`
pub async fn packages_list(
    svc: &EnvService,
    args: PackagesArgs,
    ctx: &RunContext,
) -> Result<PackageList> {
    let python = resolve_python(svc, args.venv.as_deref(), args.project.as_deref())?;
    let (backend, packages) = match svc.uv() {
        Some(uv) => (Backend::Uv, uv.pip_list(&python, args.outdated, ctx).await?),
        None => (
            Backend::Pip,
            Pip::new(&python).list(args.outdated, ctx).await?,
        ),
    };
    Ok(PackageList {
        backend,
        python,
        packages,
    })
}

/// `env.packages.add`
pub async fn packages_add(svc: &EnvService, args: AddArgs, ctx: &RunContext) -> Result<Changed> {
    if args.venv.is_some() {
        let project_only = args.group.is_some()
            || args.optional.is_some()
            || args.dev
            || args.script.is_some()
            || args.no_sync
            || args.frozen;
        if project_only {
            return Err(EnvError::invalid(
                "group, optional, dev, script, no_sync and frozen apply to project dependencies, not to a venv",
            ));
        }
        let python = resolve_python(svc, args.venv.as_deref(), None)?;
        return match svc.uv() {
            Some(uv) => {
                let req = PipInstallRequest {
                    python,
                    packages: args.packages,
                    requirements: args.requirements.map(|r| absolute(&r)),
                    upgrade: args.upgrade,
                };
                Ok(Changed {
                    backend: Backend::Uv,
                    outcome: uv.pip_install(&req, ctx).await?,
                })
            }
            None => Ok(Changed {
                backend: Backend::Pip,
                outcome: Pip::new(python)
                    .install(
                        &args.packages,
                        args.requirements.map(|r| absolute(&r)).as_deref(),
                        args.upgrade,
                        ctx,
                    )
                    .await?,
            }),
        };
    }
    if args.requirements.is_some() || args.upgrade {
        return Err(EnvError::invalid(
            "requirements and upgrade apply to installing into a venv; pass `venv`, or use env.sync and env.lock for a project",
        ));
    }
    let uv = svc.require_uv(
        "adding a dependency to pyproject.toml (pass `venv` to install with pip instead)",
    )?;
    let req = AddRequest {
        project: project_root(args.project.as_deref()),
        packages: args.packages,
        group: args.group,
        optional: args.optional,
        dev: args.dev,
        script: args.script.map(|s| absolute(&s)),
        no_sync: args.no_sync,
        frozen: args.frozen,
    };
    Ok(Changed {
        backend: Backend::Uv,
        outcome: uv.add(&req, ctx).await?,
    })
}

/// `env.packages.remove`
pub async fn packages_remove(
    svc: &EnvService,
    args: RemoveArgs,
    ctx: &RunContext,
) -> Result<Changed> {
    if args.venv.is_some() {
        let project_only = args.group.is_some()
            || args.optional.is_some()
            || args.dev
            || args.script.is_some()
            || args.no_sync
            || args.frozen;
        if project_only {
            return Err(EnvError::invalid(
                "group, optional, dev, script, no_sync and frozen apply to project dependencies, not to a venv",
            ));
        }
        let python = resolve_python(svc, args.venv.as_deref(), None)?;
        return match svc.uv() {
            Some(uv) => Ok(Changed {
                backend: Backend::Uv,
                outcome: uv.pip_uninstall(&python, &args.packages, ctx).await?,
            }),
            None => Ok(Changed {
                backend: Backend::Pip,
                outcome: Pip::new(python).uninstall(&args.packages, ctx).await?,
            }),
        };
    }
    let uv = svc.require_uv(
        "removing a dependency from pyproject.toml (pass `venv` to uninstall with pip instead)",
    )?;
    let req = RemoveRequest {
        project: project_root(args.project.as_deref()),
        packages: args.packages,
        group: args.group,
        optional: args.optional,
        dev: args.dev,
        script: args.script.map(|s| absolute(&s)),
        no_sync: args.no_sync,
        frozen: args.frozen,
    };
    Ok(Changed {
        backend: Backend::Uv,
        outcome: uv.remove(&req, ctx).await?,
    })
}

/// `env.sync`
pub async fn sync(svc: &EnvService, mut req: SyncRequest, ctx: &RunContext) -> Result<Changed> {
    req.project = project_root(Some(&req.project));
    let outcome = svc
        .require_uv("syncing the project environment")?
        .sync(&req, ctx)
        .await?;
    Ok(Changed {
        backend: Backend::Uv,
        outcome,
    })
}

/// `env.lock`
pub async fn lock(svc: &EnvService, mut req: LockRequest, ctx: &RunContext) -> Result<Changed> {
    req.project = project_root(Some(&req.project));
    let outcome = svc
        .require_uv("locking the project")?
        .lock(&req, ctx)
        .await?;
    Ok(Changed {
        backend: Backend::Uv,
        outcome,
    })
}

/// `env.python.install`
pub async fn python_install(
    svc: &EnvService,
    args: PythonInstallArgs,
    ctx: &RunContext,
) -> Result<Changed> {
    let outcome = svc
        .require_uv("installing Python versions")?
        .python_install(&args.versions, args.reinstall, ctx)
        .await?;
    Ok(Changed {
        backend: Backend::Uv,
        outcome,
    })
}

/// `env.package.search`
pub async fn package_search(svc: &EnvService, args: SearchArgs) -> Result<IndexProject> {
    svc.index().project(&args.name).await
}
