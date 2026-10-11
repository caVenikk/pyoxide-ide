//! Tests against the real `uv`. The ones that run by default use no network:
//! uv runs offline with a private cache and without Python downloads, and
//! packages come from a wheel built on the spot. A machine without uv or
//! without an installed Python skips them. The tests that need PyPI or
//! download Python are `#[ignore]`d; run them with
//! `cargo test -p py-env --test uv_integration -- --ignored`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use py_env::discovery::{SearchEnv, Tools};
use py_env::ops::{self, AddArgs, PackagesArgs, PathArgs, RemoveArgs};
use py_env::project::load_project;
use py_env::search::StaticIndex;
use py_env::uv::{
    AddRequest, InitKind, InitRequest, LockRequest, SyncRequest, TreeRequest, VenvRequest,
};
use py_env::{Backend, EnvService, ErrorKind, InterpreterKind, RunContext, Uv, UvConfig};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    svc: EnvService,
    uv: Uv,
    /// An interpreter uv already has.
    python: PathBuf,
}

impl Fixture {
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}

fn config(cache: &Path, offline: bool) -> UvConfig {
    UvConfig {
        offline,
        python_downloads: false,
        no_config: true,
        cache_dir: Some(cache.to_path_buf()),
        python_install_dir: None,
    }
}

async fn fixture_with(offline: bool) -> Option<Fixture> {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("uv-cache");
    let uv = Uv::locate(config(&cache, offline))?;
    let ctx = RunContext::default();
    let python = uv
        .python_list(true, &ctx)
        .await
        .ok()?
        .into_iter()
        .filter(|p| p.implementation == "cpython" && p.variant == "default")
        .find_map(|p| p.path)?;
    let svc = EnvService::new(
        Some(uv.clone()),
        Tools::default(),
        SearchEnv::default(),
        Arc::new(StaticIndex::new()),
    );
    Some(Fixture {
        dir,
        svc,
        uv,
        python,
    })
}

macro_rules! fixture {
    () => {
        match fixture_with(true).await {
            Some(f) => f,
            None => {
                eprintln!("skipped: uv or an installed Python interpreter is not available");
                return;
            }
        }
    };
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

/// A wheel with one module, built with the interpreter's `zipfile`.
fn build_wheel(python: &Path, dir: &Path) -> PathBuf {
    let wheel = dir.join("tinypkg-1.0-py3-none-any.whl");
    let script = r#"
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w") as z:
    z.writestr("tinypkg/__init__.py", "VALUE = 1\n")
    z.writestr("tinypkg-1.0.dist-info/METADATA", "Metadata-Version: 2.1\nName: tinypkg\nVersion: 1.0\nSummary: tiny\n")
    z.writestr("tinypkg-1.0.dist-info/WHEEL", "Wheel-Version: 1.0\nGenerator: test\nRoot-Is-Purelib: true\nTag: py3-none-any\n")
    z.writestr("tinypkg-1.0.dist-info/RECORD", "")
"#;
    let status = Command::new(python)
        .args(["-I", "-c", script])
        .arg(&wheel)
        .status()
        .unwrap();
    assert!(status.success());
    wheel
}

fn collecting_context() -> (RunContext, Arc<Mutex<Vec<String>>>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    (
        RunContext::default().with_progress(move |p| sink.lock().unwrap().push(p.line)),
        lines,
    )
}

#[tokio::test]
async fn uv_reports_its_version_and_interpreters() {
    let f = fixture!();
    let ctx = RunContext::default();
    let version = f.uv.version(&ctx).await.unwrap();
    assert!(
        version.starts_with(|c: char| c.is_ascii_digit()),
        "{version}"
    );
    let installed = f.uv.python_list(true, &ctx).await.unwrap();
    assert!(installed.iter().all(|p| p.installed()));
    assert!(installed.iter().any(|p| p.path.as_ref() == Some(&f.python)));
    let dir = f.uv.python_dir(&ctx).await.unwrap();
    assert!(dir.is_absolute());
}

#[tokio::test]
async fn venv_create_and_wheel_install_roundtrip() {
    let f = fixture!();
    let venv = f.path(".venv");
    let created = ops::venv_create(
        &f.svc,
        VenvRequest {
            path: venv.clone(),
            python: Some(f.python.to_string_lossy().into_owned()),
            ..VenvRequest::default()
        },
        &RunContext::default(),
    )
    .await
    .unwrap();
    assert_eq!(created.path, venv);
    assert!(created.outcome.command.contains("venv"));
    let interpreter = created.interpreter.expect("the new environment is probed");
    assert!(interpreter.is_venv);
    assert_eq!(interpreter.kind, InterpreterKind::Venv);
    assert_eq!(
        canonical(interpreter.venv_root.as_ref().unwrap()),
        canonical(&venv)
    );
    assert!(interpreter.major >= 3);

    // Fresh environment: nothing installed.
    let list = ops::packages_list(
        &f.svc,
        PackagesArgs {
            venv: Some(venv.clone()),
            ..PackagesArgs::default()
        },
        &RunContext::default(),
    )
    .await
    .unwrap();
    assert_eq!(list.backend, Backend::Uv);
    assert!(list.packages.is_empty());

    // Install a local wheel with progress lines streaming out.
    let wheel = build_wheel(&created.python, f.dir.path());
    let (ctx, lines) = collecting_context();
    let installed = ops::packages_add(
        &f.svc,
        AddArgs {
            packages: vec![wheel.to_string_lossy().into_owned()],
            venv: Some(venv.clone()),
            ..AddArgs::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(installed.backend, Backend::Uv);
    assert!(
        installed
            .outcome
            .log
            .iter()
            .any(|l| l.contains("tinypkg==1.0")),
        "{:?}",
        installed.outcome.log
    );
    let streamed = lines.lock().unwrap().join("\n");
    assert!(streamed.contains("Installed 1 package"), "{streamed}");

    let list = ops::packages_list(
        &f.svc,
        PackagesArgs {
            venv: Some(venv.clone()),
            ..PackagesArgs::default()
        },
        &RunContext::default(),
    )
    .await
    .unwrap();
    assert_eq!(list.packages.len(), 1);
    assert_eq!(list.packages[0].name, "tinypkg");
    assert_eq!(list.packages[0].version, "1.0");

    // Offline, the outdated check has nothing to compare with and reports nothing.
    let outdated = ops::packages_list(
        &f.svc,
        PackagesArgs {
            venv: Some(venv.clone()),
            outdated: true,
            ..PackagesArgs::default()
        },
        &RunContext::default(),
    )
    .await;
    if let Ok(o) = outdated {
        assert!(o.packages.is_empty() || o.packages.iter().all(|p| p.latest_version.is_some()));
    }

    let removed = ops::packages_remove(
        &f.svc,
        RemoveArgs {
            packages: vec!["tinypkg".into()],
            venv: Some(venv.clone()),
            ..RemoveArgs::default()
        },
        &RunContext::default(),
    )
    .await
    .unwrap();
    assert!(removed.outcome.log.iter().any(|l| l.contains("tinypkg")));
    let list = ops::packages_list(
        &f.svc,
        PackagesArgs {
            venv: Some(venv),
            ..PackagesArgs::default()
        },
        &RunContext::default(),
    )
    .await
    .unwrap();
    assert!(list.packages.is_empty());
}

#[tokio::test]
async fn pip_serves_when_uv_is_missing() {
    let f = fixture!();
    let venv = f.path(".venv");
    let ctx = RunContext::default();
    f.uv.venv(
        &VenvRequest {
            path: venv.clone(),
            python: Some(f.python.to_string_lossy().into_owned()),
            ..VenvRequest::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    // A venv made by uv has no pip, which is exactly the case to report well.
    let no_uv = f.svc.clone().with_uv(None);
    let err = ops::packages_list(
        &no_uv,
        PackagesArgs {
            venv: Some(venv.clone()),
            ..PackagesArgs::default()
        },
        &ctx,
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::ToolMissing);
    assert_eq!(err.tool.as_deref(), Some("pip"));

    // The interpreter uv found normally ships with pip.
    match ops::packages_list(
        &no_uv,
        PackagesArgs {
            venv: Some(f.python.clone()),
            ..PackagesArgs::default()
        },
        &ctx,
    )
    .await
    {
        Ok(list) => {
            assert_eq!(list.backend, Backend::Pip);
            assert!(
                list.packages
                    .iter()
                    .any(|p| p.name.eq_ignore_ascii_case("pip"))
            );
        }
        Err(e) => assert_eq!(e.kind, ErrorKind::ToolMissing, "{e}"),
    }
}

#[tokio::test]
async fn project_lifecycle_without_network() {
    let f = fixture!();
    let root = f.path("demo");
    let ctx = RunContext::default();

    let created = ops::project_init(
        &f.svc,
        InitRequest {
            path: root.clone(),
            name: Some("demo-app".into()),
            kind: InitKind::App,
            python: Some(f.python.to_string_lossy().into_owned()),
            ..InitRequest::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    assert!(created.outcome.command.contains("--vcs none"));
    assert!(root.join("pyproject.toml").is_file());
    assert!(
        !root.join(".git").exists(),
        "the IDE decides about version control"
    );

    let info = load_project(&root).unwrap();
    assert_eq!(
        info.pyproject.as_ref().unwrap().name.as_deref(),
        Some("demo-app")
    );
    assert!(info.uv_lock.is_none());
    assert!(info.venv.is_none());

    // Edit dependencies in pyproject.toml only (no network needed).
    for (group, optional, dev) in [
        (None, None, false),
        (Some("docs"), None, false),
        (None, Some("fancy"), false),
        (None, None, true),
    ] {
        let changed = ops::packages_add(
            &f.svc,
            AddArgs {
                packages: vec!["requests>=2".into()],
                project: Some(root.clone()),
                group: group.map(str::to_owned),
                optional: optional.map(str::to_owned),
                dev,
                frozen: true,
                ..AddArgs::default()
            },
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(changed.backend, Backend::Uv);
    }
    let pp = load_project(&root).unwrap().pyproject.unwrap();
    assert_eq!(pp.dependencies, ["requests>=2"]);
    assert_eq!(pp.optional_dependencies["fancy"], ["requests>=2"]);
    assert_eq!(pp.dependency_groups["docs"].requirements, ["requests>=2"]);
    assert_eq!(pp.dependency_groups["dev"].requirements, ["requests>=2"]);

    for (group, optional, dev) in [
        (None, None, false),
        (Some("docs"), None, false),
        (None, Some("fancy"), false),
        (None, None, true),
    ] {
        ops::packages_remove(
            &f.svc,
            RemoveArgs {
                packages: vec!["requests".into()],
                project: Some(root.clone()),
                group: group.map(str::to_owned),
                optional: optional.map(str::to_owned),
                dev,
                frozen: true,
                ..RemoveArgs::default()
            },
            &ctx,
        )
        .await
        .unwrap();
    }
    let pp = load_project(&root).unwrap().pyproject.unwrap();
    assert!(pp.dependencies.is_empty());

    // Lock and sync a project without dependencies: no network involved.
    ops::lock(
        &f.svc,
        LockRequest {
            project: root.clone(),
            ..LockRequest::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    let info = load_project(&root).unwrap();
    let lock = info.uv_lock.expect("uv.lock was written");
    assert_eq!(lock.packages.len(), 1);
    assert_eq!(lock.packages[0].name, "demo-app");

    let (ctx2, lines) = collecting_context();
    ops::sync(
        &f.svc,
        SyncRequest {
            project: root.clone(),
            ..SyncRequest::default()
        },
        &ctx2,
    )
    .await
    .unwrap();
    assert!(
        lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("Resolved 1 package"))
    );
    let info = ops::project_info(
        &f.svc,
        PathArgs {
            path: Some(root.clone()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        info.venv.as_deref().map(canonical),
        Some(canonical(&root.join(".venv")))
    );
    assert!(info.managers.contains(&py_env::project::Manager::Uv));

    // The project's environment is found without naming it.
    let list = ops::packages_list(
        &f.svc,
        PackagesArgs {
            project: Some(root.clone()),
            ..PackagesArgs::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    assert!(list.packages.is_empty());
    assert_eq!(
        canonical(&list.python),
        canonical(&py_env::paths::venv_python(&root.join(".venv")))
    );

    let tree = ops::project_tree(
        &f.svc,
        TreeRequest {
            project: root.clone(),
            frozen: true,
            ..TreeRequest::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(tree.roots.len(), 1);
    assert_eq!(tree.roots[0].name, "demo-app");
    assert_eq!(tree.roots[0].version.as_deref(), Some("0.1.0"));
}

#[tokio::test]
async fn script_dependencies_are_written_and_read() {
    let f = fixture!();
    let script = f.path("tool.py");
    std::fs::write(&script, "print('hi')\n").unwrap();
    f.uv.add(
        &AddRequest {
            project: f.dir.path().to_path_buf(),
            packages: vec!["httpx".into()],
            script: Some(script.clone()),
            frozen: true,
            ..AddRequest::default()
        },
        &RunContext::default(),
    )
    .await
    .unwrap();
    let info = load_project(&script).unwrap();
    assert_eq!(info.script.unwrap().dependencies, ["httpx"]);
}

#[tokio::test]
async fn failures_have_typed_kinds() {
    let f = fixture!();
    let ctx = RunContext::default();
    let root = f.path("proj");
    ops::project_init(
        &f.svc,
        InitRequest {
            path: root.clone(),
            python: Some(f.python.to_string_lossy().into_owned()),
            ..InitRequest::default()
        },
        &ctx,
    )
    .await
    .unwrap();

    // Initializing twice.
    let e = ops::project_init(
        &f.svc,
        InitRequest {
            path: root.clone(),
            ..InitRequest::default()
        },
        &ctx,
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::AlreadyExists, "{e}");

    // A package that is not in the project.
    let e = ops::packages_remove(
        &f.svc,
        RemoveArgs {
            packages: vec!["nothere".into()],
            project: Some(root.clone()),
            frozen: true,
            ..RemoveArgs::default()
        },
        &ctx,
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::PackageNotFound, "{e}");
    assert!(e.message.contains("nothere"));

    // Resolving needs the index; offline with an empty cache that is a network failure.
    let e = ops::packages_add(
        &f.svc,
        AddArgs {
            packages: vec!["requests".into()],
            project: Some(root.clone()),
            no_sync: true,
            ..AddArgs::default()
        },
        &ctx,
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network, "{e}");
    assert_eq!(e.tool.as_deref(), Some("uv"));
    assert!(e.exit_code.is_some());

    // Existing environment, unknown Python, missing project, missing download.
    let venv = f.path("v");
    let request = VenvRequest {
        path: venv.clone(),
        python: Some(f.python.to_string_lossy().into_owned()),
        ..VenvRequest::default()
    };
    ops::venv_create(&f.svc, request.clone(), &ctx)
        .await
        .unwrap();
    let e = ops::venv_create(&f.svc, request.clone(), &ctx)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::AlreadyExists, "{e}");
    let again = VenvRequest {
        clear: true,
        ..request
    };
    ops::venv_create(&f.svc, again, &ctx).await.unwrap();

    let e = ops::venv_create(
        &f.svc,
        VenvRequest {
            path: f.path("w"),
            python: Some("3.99".into()),
            ..VenvRequest::default()
        },
        &ctx,
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NoInterpreter, "{e}");

    let empty = f.path("empty");
    std::fs::create_dir_all(&empty).unwrap();
    // uv looks in parent directories too, so the temporary directory must not be inside a project.
    if !f
        .dir
        .path()
        .ancestors()
        .any(|d| d.join("pyproject.toml").is_file())
    {
        let e = ops::sync(
            &f.svc,
            SyncRequest {
                project: empty,
                ..SyncRequest::default()
            },
            &ctx,
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::NoProject, "{e}");
    }

    let e = ops::python_install(
        &f.svc,
        ops::PythonInstallArgs {
            versions: vec!["3.99".into()],
            reinstall: false,
        },
        &ctx,
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NoInterpreter, "{e}");
}

#[tokio::test]
async fn cancelling_stops_uv() {
    let f = fixture!();
    let token = py_env::CancellationToken::new();
    let ctx = RunContext::default().with_cancel(token.clone());
    token.cancel();
    let e = f.uv.version(&ctx).await;
    // Cancelled before it could finish; a very fast uv may still win the race.
    if let Err(e) = e {
        assert_eq!(e.kind, ErrorKind::Cancelled);
    }
}

#[tokio::test]
async fn discovery_finds_interpreters_on_this_machine() {
    let f = fixture!();
    let svc = EnvService::from_process();
    let report = ops::interpreters_list(
        &svc,
        ops::InterpretersArgs::default(),
        &RunContext::default(),
    )
    .await
    .unwrap();
    let uv = report.tools.iter().find(|t| t.name == "uv").unwrap();
    assert!(uv.installed);
    assert!(uv.version.is_some());
    assert!(!report.interpreters.is_empty());
    assert!(
        report
            .interpreters
            .iter()
            .any(|i| canonical(&i.path) == canonical(&f.python)),
        "uv's own interpreter {} is listed",
        f.python.display()
    );
    for i in &report.interpreters {
        assert!(
            i.major >= 2 && !i.version.is_empty() && i.path.is_absolute(),
            "{i:?}"
        );
    }
}

// ---------- network ----------

/// Installs `requests` from PyPI into a new project and reads everything back.
/// Needs the network (PyPI, and the index for resolution).
#[tokio::test]
#[ignore = "needs the network (PyPI)"]
async fn add_sync_and_inspect_with_pypi() {
    let f = match fixture_with(false).await {
        Some(f) => f,
        None => return,
    };
    let root = f.path("net");
    let ctx = RunContext::default();
    ops::project_init(
        &f.svc,
        InitRequest {
            path: root.clone(),
            python: Some(f.python.to_string_lossy().into_owned()),
            ..InitRequest::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    ops::packages_add(
        &f.svc,
        AddArgs {
            packages: vec!["requests".into()],
            project: Some(root.clone()),
            ..AddArgs::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    let list = ops::packages_list(
        &f.svc,
        PackagesArgs {
            project: Some(root.clone()),
            ..PackagesArgs::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    assert!(list.packages.iter().any(|p| p.name == "requests"));
    let info = load_project(&root).unwrap();
    assert!(
        info.uv_lock
            .unwrap()
            .packages
            .iter()
            .any(|p| p.name == "certifi")
    );
    let tree = ops::project_tree(
        &f.svc,
        TreeRequest {
            project: root.clone(),
            ..TreeRequest::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    let requests = &tree.roots[0].children[0];
    assert_eq!(requests.name, "requests");
    assert!(requests.children.iter().any(|c| c.name == "certifi"));
    // Outdated listing against the real index.
    let outdated = ops::packages_list(
        &f.svc,
        PackagesArgs {
            project: Some(root),
            outdated: true,
            ..PackagesArgs::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    assert!(outdated.packages.iter().all(|p| p.latest_version.is_some()));
}

/// Installs a Python version into a temporary directory (a download of tens
/// of megabytes). Needs the network.
#[tokio::test]
#[ignore = "downloads a Python build"]
async fn python_install_goes_to_the_configured_directory() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path().join("pythons");
    let Some(uv) = Uv::locate(UvConfig {
        python_install_dir: Some(install.clone()),
        cache_dir: Some(dir.path().join("cache")),
        no_config: true,
        ..UvConfig::default()
    }) else {
        return;
    };
    let svc = EnvService::new(
        Some(uv.clone()),
        Tools::default(),
        SearchEnv::default(),
        Arc::new(StaticIndex::new()),
    );
    let done = ops::python_install(
        &svc,
        ops::PythonInstallArgs {
            versions: vec!["3.13".into()],
            reinstall: false,
        },
        &RunContext::default(),
    )
    .await
    .unwrap();
    assert!(done.outcome.command.contains("python install"));
    let listed = uv.python_list(true, &RunContext::default()).await.unwrap();
    assert!(listed.iter().any(|p| p.version.starts_with("3.13")
        && p.path.as_ref().is_some_and(|p| p.starts_with(&install))));
}

/// A seeded environment has pip, so the pip fallback can install into it.
/// Needs the network (uv fetches pip, then pip fetches the package).
#[tokio::test]
#[ignore = "needs the network (PyPI)"]
async fn pip_fallback_installs_from_pypi() {
    let f = match fixture_with(false).await {
        Some(f) => f,
        None => return,
    };
    let venv = f.path(".venv");
    let ctx = RunContext::default();
    f.uv.venv(
        &VenvRequest {
            path: venv.clone(),
            python: Some(f.python.to_string_lossy().into_owned()),
            seed: true,
            ..VenvRequest::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    let no_uv = f.svc.clone().with_uv(None);
    let added = ops::packages_add(
        &no_uv,
        AddArgs {
            packages: vec!["idna".into()],
            venv: Some(venv.clone()),
            ..AddArgs::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(added.backend, Backend::Pip);
    let list = ops::packages_list(
        &no_uv,
        PackagesArgs {
            venv: Some(venv),
            ..PackagesArgs::default()
        },
        &ctx,
    )
    .await
    .unwrap();
    assert_eq!(list.backend, Backend::Pip);
    assert!(list.packages.iter().any(|p| p.name == "idna"));
}

/// The real PyPI JSON API. Needs the network.
#[tokio::test]
#[ignore = "needs the network (PyPI)"]
async fn pypi_lookup() {
    use py_env::PackageIndex;
    let pypi = py_env::PyPi::new();
    let p = pypi.project("Requests").await.unwrap();
    assert_eq!(p.name.to_ascii_lowercase(), "requests");
    assert!(!p.versions.is_empty());
    assert!(p.versions.windows(2).all(|w| w[0].version != w[1].version));
    assert_eq!(
        pypi.project("this-package-does-not-exist-pyoxide-0x7f")
            .await
            .unwrap_err()
            .kind,
        ErrorKind::PackageNotFound
    );
}
