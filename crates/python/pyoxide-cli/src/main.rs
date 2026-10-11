//! `pyoxide`: the command-line front-end of the Pyoxide Python IDE.
//!
//! It composes the platform command registry (`db.*`, from `db-service`) with
//! the Python environment commands (`env.*`, from `py-env`). The `env`
//! sub-commands call the same typed functions as the `env.*` commands; the
//! only difference is that they stream the tools' progress to stderr.

mod db;
mod render;

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use commands::{CommandError, Registry};
use py_env::ops::{
    self, AddArgs, InterpretersArgs, PackagesArgs, PathArgs, PythonInstallArgs, RemoveArgs,
    SearchArgs,
};
use py_env::uv::{InitKind, InitRequest, LockRequest, SyncRequest, TreeRequest, VenvRequest};
use py_env::{CancellationToken, EnvError, EnvService, ErrorKind, RunContext};
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "pyoxide",
    version,
    about = "Pyoxide IDE for Python: environments, packages and the database client",
    after_help = "`pyoxide commands` lists every command of the registry; `pyoxide call <id> <json>` runs one."
)]
struct Cli {
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Profiles file of the db.* commands (default: $PYOXIDE_DB_PROFILES or the user config directory).
    #[arg(long, global = true, value_name = "PATH")]
    profiles: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Python interpreters, virtual environments, projects and packages.
    #[command(subcommand)]
    Env(EnvCmd),
    /// List all registered commands (db.* and env.*).
    Commands,
    /// Invoke a registered command with JSON arguments (from the argument or stdin).
    Call { id: String, args: Option<String> },
}

#[derive(Subcommand)]
enum EnvCmd {
    /// Find interpreters: uv, PATH, project venvs, Poetry, conda, pyenv.
    Interpreters {
        /// Project directory whose environments are included.
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
    },
    /// Show what the project at PATH (default: here) declares: pyproject.toml, uv.lock, requirements, script metadata.
    Info { path: Option<PathBuf> },
    /// Create a project with uv init.
    Init(InitArgs),
    /// Create a virtual environment with uv venv.
    Venv {
        /// Where to create it (default: .venv).
        path: Option<PathBuf>,
        /// Python version, implementation or interpreter path.
        #[arg(long)]
        python: Option<String>,
        /// Install pip and setuptools into it.
        #[arg(long)]
        seed: bool,
        /// Replace an existing environment.
        #[arg(long)]
        clear: bool,
        #[arg(long)]
        system_site_packages: bool,
    },
    /// List installed packages of the project's environment or of --venv.
    Packages {
        #[arg(long, value_name = "PATH")]
        venv: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
        /// Only packages with a newer version on the index.
        #[arg(long)]
        outdated: bool,
    },
    /// Add packages to pyproject.toml (uv add), or install into --venv.
    Add {
        #[arg(required = true, num_args = 1.., value_name = "PACKAGE")]
        packages: Vec<String>,
        #[command(flatten)]
        target: Target,
        /// With --venv: install from a requirements file.
        #[arg(short = 'r', long, value_name = "FILE")]
        requirements: Option<PathBuf>,
        /// With --venv: upgrade to the newest allowed versions.
        #[arg(long)]
        upgrade: bool,
    },
    /// Remove packages from pyproject.toml (uv remove), or uninstall from --venv.
    Remove {
        #[arg(required = true, num_args = 1.., value_name = "PACKAGE")]
        packages: Vec<String>,
        #[command(flatten)]
        target: Target,
    },
    /// Make the project environment match uv.lock (uv sync).
    Sync {
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
        #[arg(long = "extra", value_name = "EXTRA")]
        extras: Vec<String>,
        #[arg(long)]
        all_extras: bool,
        #[arg(long = "group", value_name = "GROUP")]
        groups: Vec<String>,
        #[arg(long)]
        all_groups: bool,
        #[arg(long)]
        no_dev: bool,
        /// Use uv.lock as is.
        #[arg(long)]
        frozen: bool,
        /// Fail if uv.lock is out of date.
        #[arg(long)]
        locked: bool,
        #[arg(long)]
        python: Option<String>,
    },
    /// Resolve dependencies into uv.lock (uv lock).
    Lock {
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
        /// Allow all packages to be upgraded.
        #[arg(long)]
        upgrade: bool,
        #[arg(long = "upgrade-package", value_name = "PACKAGE")]
        upgrade_packages: Vec<String>,
        #[arg(long)]
        python: Option<String>,
    },
    /// Show the dependency tree of the project (uv tree).
    Tree {
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
        #[arg(long)]
        depth: Option<u32>,
        /// Show newer versions on the index.
        #[arg(long)]
        outdated: bool,
        #[arg(long)]
        no_dev: bool,
        #[arg(long)]
        frozen: bool,
        #[arg(long)]
        package: Option<String>,
    },
    /// Install Python versions with uv python install.
    PythonInstall {
        #[arg(required = true, num_args = 1.., value_name = "VERSION")]
        versions: Vec<String>,
        #[arg(long)]
        reinstall: bool,
    },
    /// Look a package up on PyPI.
    Search {
        name: String,
        /// How many versions to list.
        #[arg(long, default_value_t = 10, value_name = "N")]
        versions: usize,
    },
}

#[derive(Args)]
struct InitArgs {
    /// Project directory (default: here).
    path: Option<PathBuf>,
    #[arg(long)]
    name: Option<String>,
    /// A library instead of an application.
    #[arg(long, conflicts_with_all = ["package", "bare", "script"])]
    lib: bool,
    /// An application built as a package.
    #[arg(long, conflicts_with_all = ["bare", "script"])]
    package: bool,
    /// Only a pyproject.toml.
    #[arg(long, conflicts_with = "script")]
    bare: bool,
    /// A single script with inline metadata (PATH is the file).
    #[arg(long)]
    script: bool,
    #[arg(long)]
    python: Option<String>,
    #[arg(long, value_name = "BACKEND")]
    build_backend: Option<String>,
    #[arg(long)]
    no_readme: bool,
    #[arg(long)]
    no_pin_python: bool,
    /// Let uv run git init.
    #[arg(long)]
    vcs: bool,
}

/// Where a dependency change goes.
#[derive(Args)]
struct Target {
    #[arg(long, value_name = "DIR")]
    project: Option<PathBuf>,
    /// Install into / uninstall from this environment with uv pip (or pip) instead of editing pyproject.toml.
    #[arg(long, value_name = "PATH")]
    venv: Option<PathBuf>,
    /// A dependency group (PEP 735).
    #[arg(long)]
    group: Option<String>,
    /// An optional-dependencies extra.
    #[arg(long)]
    optional: Option<String>,
    /// The dev group.
    #[arg(long)]
    dev: bool,
    /// Edit the inline metadata of this script (PEP 723).
    #[arg(long, value_name = "FILE")]
    script: Option<PathBuf>,
    /// Edit pyproject.toml only: no lock, no install.
    #[arg(long)]
    no_sync: bool,
    /// Do not re-lock.
    #[arg(long)]
    frozen: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli).await {
        Ok(code) => code,
        Err(e) => fail(&e, json),
    }
}

fn fail(e: &EnvError, json: bool) -> ExitCode {
    if json {
        print_json(&serde_json::json!({
            "code": e.kind.code(),
            "message": e.message,
            "tool": e.tool,
            "exit_code": e.exit_code,
            "detail": e.detail,
        }));
    } else {
        eprintln!("error: {}", e.message);
    }
    if e.kind == ErrorKind::Cancelled {
        ExitCode::from(130)
    } else {
        ExitCode::FAILURE
    }
}

fn print_json<T: Serialize>(v: &T) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

/// Text or JSON, depending on `--json`.
fn emit<T: Serialize>(json: bool, value: &T, text: impl FnOnce() -> String) {
    if json {
        print_json(value);
    } else {
        println!("{}", text());
    }
}

/// Progress lines go to stderr; Ctrl-C cancels the running tool cleanly.
fn context(json: bool) -> RunContext {
    let token = CancellationToken::new();
    let on_signal = token.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            on_signal.cancel();
        }
    });
    let ctx = RunContext::default().with_cancel(token);
    if json {
        ctx
    } else {
        ctx.with_progress(|p| eprintln!("{}", p.line))
    }
}

fn registry(profiles: Option<PathBuf>) -> Registry {
    let mut r = Registry::new();
    db_service::ops::register(&mut r, Arc::new(db::service(profiles)));
    py_env::register(&mut r, Arc::new(EnvService::from_process()));
    r
}

async fn run(cli: Cli) -> Result<ExitCode, EnvError> {
    let json = cli.json;
    match cli.cmd {
        Cmd::Env(cmd) => env(cmd, json).await?,
        Cmd::Commands => {
            let registry = registry(cli.profiles);
            let list = registry.list();
            emit(json, &list, || render::commands(&list));
        }
        Cmd::Call { id, args } => {
            let raw = match args {
                Some(a) => a,
                None => {
                    let mut s = String::new();
                    std::io::stdin()
                        .read_to_string(&mut s)
                        .map_err(|e| EnvError::invalid(format!("reading stdin: {e}")))?;
                    s
                }
            };
            let value: serde_json::Value = if raw.trim().is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::from_str(&raw)
                    .map_err(|e| EnvError::invalid(format!("arguments are not JSON: {e}")))?
            };
            match registry(cli.profiles).invoke(&id, value).await {
                Ok(v) => print_json(&v),
                Err(e) => {
                    print_json::<CommandError>(&e);
                    return Ok(ExitCode::FAILURE);
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn env(cmd: EnvCmd, json: bool) -> Result<(), EnvError> {
    let svc = EnvService::from_process();
    let ctx = context(json);
    match cmd {
        EnvCmd::Interpreters { project } => {
            let report = ops::interpreters_list(&svc, InterpretersArgs { project }, &ctx).await?;
            emit(json, &report, || render::interpreters(&report));
        }
        EnvCmd::Info { path } => {
            let info = ops::project_info(&svc, PathArgs { path }).await?;
            emit(json, &info, || render::project(&info));
        }
        EnvCmd::Init(a) => {
            let kind = if a.script {
                InitKind::Script
            } else if a.bare {
                InitKind::Bare
            } else if a.package {
                InitKind::Package
            } else if a.lib {
                InitKind::Lib
            } else {
                InitKind::App
            };
            let req = InitRequest {
                path: a.path.unwrap_or_else(|| PathBuf::from(".")),
                name: a.name,
                kind,
                python: a.python,
                build_backend: a.build_backend,
                no_readme: a.no_readme,
                no_pin_python: a.no_pin_python,
                vcs: a.vcs,
            };
            let done = ops::project_init(&svc, req, &ctx).await?;
            emit(json, &done, || render::changed(&done));
        }
        EnvCmd::Venv {
            path,
            python,
            seed,
            clear,
            system_site_packages,
        } => {
            let req = VenvRequest {
                path: path.unwrap_or_else(|| PathBuf::from(".venv")),
                python,
                seed,
                clear,
                system_site_packages,
                prompt: None,
            };
            let done = ops::venv_create(&svc, req, &ctx).await?;
            emit(json, &done, || render::venv(&done));
        }
        EnvCmd::Packages {
            venv,
            project,
            outdated,
        } => {
            let list = ops::packages_list(
                &svc,
                PackagesArgs {
                    venv,
                    project,
                    outdated,
                },
                &ctx,
            )
            .await?;
            emit(json, &list, || render::packages(&list));
        }
        EnvCmd::Add {
            packages,
            target,
            requirements,
            upgrade,
        } => {
            let args = AddArgs {
                packages,
                project: target.project,
                venv: target.venv,
                group: target.group,
                optional: target.optional,
                dev: target.dev,
                script: target.script,
                no_sync: target.no_sync,
                frozen: target.frozen,
                requirements,
                upgrade,
            };
            let done = ops::packages_add(&svc, args, &ctx).await?;
            emit(json, &done, || render::changed(&done));
        }
        EnvCmd::Remove { packages, target } => {
            let args = RemoveArgs {
                packages,
                project: target.project,
                venv: target.venv,
                group: target.group,
                optional: target.optional,
                dev: target.dev,
                script: target.script,
                no_sync: target.no_sync,
                frozen: target.frozen,
            };
            let done = ops::packages_remove(&svc, args, &ctx).await?;
            emit(json, &done, || render::changed(&done));
        }
        EnvCmd::Sync {
            project,
            extras,
            all_extras,
            groups,
            all_groups,
            no_dev,
            frozen,
            locked,
            python,
        } => {
            let req = SyncRequest {
                project: project.unwrap_or_default(),
                extras,
                all_extras,
                groups,
                all_groups,
                no_dev,
                frozen,
                locked,
                python,
            };
            let done = ops::sync(&svc, req, &ctx).await?;
            emit(json, &done, || render::changed(&done));
        }
        EnvCmd::Lock {
            project,
            upgrade,
            upgrade_packages,
            python,
        } => {
            let req = LockRequest {
                project: project.unwrap_or_default(),
                upgrade,
                upgrade_packages,
                python,
            };
            let done = ops::lock(&svc, req, &ctx).await?;
            emit(json, &done, || render::changed(&done));
        }
        EnvCmd::Tree {
            project,
            depth,
            outdated,
            no_dev,
            frozen,
            package,
        } => {
            let req = TreeRequest {
                project: project.unwrap_or_default(),
                depth,
                outdated,
                no_dev,
                frozen,
                package,
            };
            let tree = ops::project_tree(&svc, req, &ctx).await?;
            emit(json, &tree, || tree.text.trim_end().to_owned());
        }
        EnvCmd::PythonInstall {
            versions,
            reinstall,
        } => {
            let done = ops::python_install(
                &svc,
                PythonInstallArgs {
                    versions,
                    reinstall,
                },
                &ctx,
            )
            .await?;
            emit(json, &done, || render::changed(&done));
        }
        EnvCmd::Search { name, versions } => {
            let found = ops::package_search(&svc, SearchArgs { name }).await?;
            emit(json, &found, || render::search(&found, versions));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_the_documented_invocations() {
        for args in [
            &["pyoxide", "env", "interpreters"][..],
            &["pyoxide", "env", "info"],
            &["pyoxide", "env", "info", "some/dir"],
            &["pyoxide", "env", "venv"],
            &["pyoxide", "env", "venv", "--python", "3.12", "path/.venv"],
            &["pyoxide", "env", "packages", "--venv", ".venv"],
            &[
                "pyoxide", "env", "add", "requests", "rich>=13", "--group", "dev",
            ],
            &[
                "pyoxide", "env", "add", "requests", "--venv", ".venv", "-r", "req.txt",
            ],
            &["pyoxide", "env", "remove", "requests"],
            &["pyoxide", "env", "sync"],
            &[
                "pyoxide",
                "env",
                "sync",
                "--all-groups",
                "--extra",
                "a",
                "--extra",
                "b",
            ],
            &["pyoxide", "env", "lock", "--upgrade"],
            &["pyoxide", "env", "python-install", "3.13"],
            &["pyoxide", "env", "search", "requests"],
            &["pyoxide", "env", "init", "--lib", "demo"],
            &["pyoxide", "env", "tree", "--depth", "2"],
            &["pyoxide", "commands"],
            &[
                "pyoxide",
                "--json",
                "call",
                "env.package.search",
                "{\"name\": \"x\"}",
            ],
        ] {
            Cli::try_parse_from(args).unwrap_or_else(|e| panic!("{args:?}: {e}"));
        }
    }

    #[test]
    fn rejects_incomplete_invocations() {
        for args in [
            &["pyoxide", "env", "add"][..],
            &["pyoxide", "env", "python-install"],
            &["pyoxide", "env", "search"],
            &["pyoxide", "env", "init", "--lib", "--bare"],
        ] {
            assert!(Cli::try_parse_from(args).is_err(), "{args:?}");
        }
    }
}
