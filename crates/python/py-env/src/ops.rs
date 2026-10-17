//! The `env.*` operations as typed async functions. The CLI calls them
//! directly (with a progress sink); the command registry wraps the same
//! functions (see [`crate::commands`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::discovery::{self, DiscoveryOptions, DiscoveryReport};
use crate::error::{EnvError, ErrorKind, Result};
use crate::interpreter::{Interpreter, probe};
use crate::packages::{Backend, Outcome, Package};
use crate::paths::{environment_python, venv_python};
use crate::pip::Pip;
use crate::process::{RunContext, Spec, run_ok};
use crate::project::{ProjectInfo, find_project_root, find_project_venv, load_project_async};
use crate::search::IndexProject;
use crate::service::EnvService;
use crate::uv::{
    AddRequest, InitRequest, LockRequest, PipInstallRequest, RemoveRequest, SyncRequest, Tree,
    TreeRequest, Uv, VenvRequest,
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
    let uv = svc.require_uv("creating a virtual environment")?;
    req.path = absolute(&req.path);
    venv_create_with(Some(uv), req, ctx).await
}

/// A virtual environment made with uv when there is one (`uv venv`), else
/// with the `venv` module of the base interpreter that `req.python` names
/// by path (`python -m venv`): the reference IDE's "Virtualenv"
/// environment on a machine without uv. `req.path` is absolute.
pub async fn venv_create_with(
    uv: Option<&Uv>,
    req: VenvRequest,
    ctx: &RunContext,
) -> Result<VenvCreated> {
    let outcome = match uv {
        Some(uv) => uv.venv(&req, ctx).await?,
        None => {
            let base = req.python.as_deref().ok_or_else(|| {
                EnvError::invalid(
                    "creating a virtual environment without uv needs a base interpreter",
                )
            })?;
            let started = std::time::Instant::now();
            let mut spec = Spec::new(base).args(module_venv_args(&req));
            spec.forward_stdout = true;
            if let Some(cwd) = &req.cwd {
                spec = spec.cwd(cwd);
            }
            let command = spec.display();
            let done = run_ok("python", spec, ctx, Duration::from_secs(10 * 60)).await?;
            Outcome::new(command, &done.stdout, &done.stderr, started)
        }
    };
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

/// `-m venv …` for a base interpreter: pip is installed unless `seed` is
/// off (the module's own default).
pub(crate) fn module_venv_args(req: &VenvRequest) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec!["-m".into(), "venv".into()];
    if req.clear {
        a.push("--clear".into());
    }
    if req.system_site_packages {
        a.push("--system-site-packages".into());
    }
    if !req.seed {
        a.push("--without-pip".into());
    }
    if let Some(prompt) = &req.prompt {
        a.push("--prompt".into());
        a.push(prompt.into());
    }
    a.push(req.path.clone().into());
    a
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

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(a: Vec<OsString>) -> Vec<String> {
        a.into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_venv_module_without_uv() {
        let req = VenvRequest {
            path: "/p/.venv".into(),
            python: Some("/usr/bin/python3".into()),
            seed: true,
            system_site_packages: true,
            ..VenvRequest::default()
        };
        assert_eq!(
            strs(module_venv_args(&req)),
            ["-m", "venv", "--system-site-packages", "/p/.venv"]
        );
        let bare = VenvRequest {
            path: "/p/env".into(),
            clear: true,
            prompt: Some("demo".into()),
            ..VenvRequest::default()
        };
        assert_eq!(
            strs(module_venv_args(&bare)),
            [
                "-m",
                "venv",
                "--clear",
                "--without-pip",
                "--prompt",
                "demo",
                "/p/env"
            ]
        );
    }

    /// Without uv and a base interpreter there is nothing to create with.
    #[tokio::test]
    async fn without_uv_a_base_interpreter_is_needed() {
        let e = venv_create_with(
            None,
            VenvRequest {
                path: "/nowhere/.venv".into(),
                ..VenvRequest::default()
            },
            &RunContext::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput, "{e}");
    }

    /// A base interpreter's `venv` module; needs `python3` on `PATH`.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "starts the Python interpreter found on PATH"]
    async fn the_venv_module_creates_an_environment() {
        let python = crate::paths::find_on_path("python3").expect("python3 on PATH");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env");
        let created = venv_create_with(
            None,
            VenvRequest {
                path: path.clone(),
                python: Some(python.to_string_lossy().into_owned()),
                seed: true,
                ..VenvRequest::default()
            },
            &RunContext::default(),
        )
        .await
        .unwrap();
        assert!(created.outcome.command.contains("-m venv"));
        assert!(path.join("pyvenv.cfg").is_file());
        assert!(created.python.starts_with(&path));
        assert!(
            created.interpreter.is_some(),
            "the new environment is probed"
        );
    }
}
