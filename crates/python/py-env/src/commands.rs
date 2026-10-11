//! Registration of the `env.*` commands (ADR 0001). Each command is the typed
//! function of [`crate::ops`]; the registry adds JSON in and out. Commands
//! carry no progress sink: the tool's report comes back in the result's
//! `log`, and dropping the call stops the tool.

use std::sync::Arc;

use commands::{CommandError, CommandMeta, Registry, typed};

use crate::ops::{
    self, AddArgs, InterpretersArgs, PackagesArgs, PathArgs, PythonInstallArgs, RemoveArgs,
    SearchArgs,
};
use crate::process::RunContext;
use crate::service::EnvService;
use crate::uv::{InitRequest, LockRequest, SyncRequest, TreeRequest, VenvRequest};

macro_rules! meta {
    ($id:literal, $title:literal, $summary:literal, $example:literal) => {
        CommandMeta {
            id: $id,
            title: $title,
            summary: $summary,
            args_example: $example,
        }
    };
}

/// Register every `env.*` command.
pub fn register(registry: &mut Registry, svc: Arc<EnvService>) {
    let s = svc.clone();
    registry.register(typed(
        meta!(
            "env.interpreters.list",
            "List Python Interpreters",
            "Finds Python interpreters (uv, PATH, project venvs, Poetry, conda, pyenv) and reports which tools are installed.",
            r#"{"project": "."}"#
        ),
        move |a: InterpretersArgs| {
            let s = s.clone();
            async move {
                ops::interpreters_list(&s, a, &RunContext::default())
                    .await
                    .map_err(CommandError::from)
            }
        },
    ));

    let s = svc.clone();
    registry.register(typed(
        meta!(
            "env.project.info",
            "Show Project Environment Files",
            "Reads pyproject.toml, uv.lock, requirements files and PEP 723 script metadata.",
            r#"{"path": "."}"#
        ),
        move |a: PathArgs| {
            let s = s.clone();
            async move { ops::project_info(&s, a).await.map_err(CommandError::from) }
        },
    ));

    let s = svc.clone();
    registry.register(typed(
        meta!(
            "env.project.init",
            "Initialize Project",
            "Creates a pyproject.toml project with uv init.",
            r#"{"path": "demo", "name": "demo", "kind": "app", "python": "3.12"}"#
        ),
        move |a: InitRequest| {
            let s = s.clone();
            async move {
                ops::project_init(&s, a, &RunContext::default())
                    .await
                    .map_err(CommandError::from)
            }
        },
    ));

    let s = svc.clone();
    registry.register(typed(
        meta!(
            "env.project.tree",
            "Show Dependency Tree",
            "Shows the resolved dependency tree of a project with uv tree.",
            r#"{"project": ".", "depth": 2}"#
        ),
        move |a: TreeRequest| {
            let s = s.clone();
            async move {
                ops::project_tree(&s, a, &RunContext::default())
                    .await
                    .map_err(CommandError::from)
            }
        },
    ));

    let s = svc.clone();
    registry.register(typed(
        meta!(
            "env.venv.create",
            "Create Virtual Environment",
            "Creates a virtual environment with uv venv, optionally for a given Python version.",
            r#"{"path": ".venv", "python": "3.12", "seed": false}"#
        ),
        move |a: VenvRequest| {
            let s = s.clone();
            async move {
                ops::venv_create(&s, a, &RunContext::default())
                    .await
                    .map_err(CommandError::from)
            }
        },
    ));

    let s = svc.clone();
    registry.register(typed(
        meta!(
            "env.packages.list",
            "List Installed Packages",
            "Lists the packages of a virtual environment (uv pip list, or pip without uv), optionally only outdated ones.",
            r#"{"venv": ".venv", "outdated": false}"#
        ),
        move |a: PackagesArgs| {
            let s = s.clone();
            async move {
                ops::packages_list(&s, a, &RunContext::default())
                    .await
                    .map_err(CommandError::from)
            }
        },
    ));

    let s = svc.clone();
    registry.register(typed(
        meta!(
            "env.packages.add",
            "Add Packages",
            "Adds packages to the project (uv add, with group/optional/dev) or installs them into a venv when `venv` is given.",
            r#"{"packages": ["requests"], "group": "dev"}"#
        ),
        move |a: AddArgs| {
            let s = s.clone();
            async move {
                ops::packages_add(&s, a, &RunContext::default())
                    .await
                    .map_err(CommandError::from)
            }
        },
    ));

    let s = svc.clone();
    registry.register(typed(
        meta!(
            "env.packages.remove",
            "Remove Packages",
            "Removes packages from the project (uv remove) or uninstalls them from a venv when `venv` is given.",
            r#"{"packages": ["requests"]}"#
        ),
        move |a: RemoveArgs| {
            let s = s.clone();
            async move {
                ops::packages_remove(&s, a, &RunContext::default())
                    .await
                    .map_err(CommandError::from)
            }
        },
    ));

    let s = svc.clone();
    registry.register(typed(
        meta!(
            "env.sync",
            "Sync Environment",
            "Makes the project environment match uv.lock (uv sync), updating the lock first unless frozen.",
            r#"{"project": ".", "all_groups": true}"#
        ),
        move |a: SyncRequest| {
            let s = s.clone();
            async move {
                ops::sync(&s, a, &RunContext::default())
                    .await
                    .map_err(CommandError::from)
            }
        },
    ));

    let s = svc.clone();
    registry.register(typed(
        meta!(
            "env.lock",
            "Lock Dependencies",
            "Resolves the project's dependencies into uv.lock (uv lock).",
            r#"{"project": ".", "upgrade": false}"#
        ),
        move |a: LockRequest| {
            let s = s.clone();
            async move {
                ops::lock(&s, a, &RunContext::default())
                    .await
                    .map_err(CommandError::from)
            }
        },
    ));

    let s = svc.clone();
    registry.register(typed(
        meta!(
            "env.python.install",
            "Install Python",
            "Downloads and installs Python versions with uv python install (needs the network).",
            r#"{"versions": ["3.13"]}"#
        ),
        move |a: PythonInstallArgs| {
            let s = s.clone();
            async move {
                ops::python_install(&s, a, &RunContext::default())
                    .await
                    .map_err(CommandError::from)
            }
        },
    ));

    let s = svc;
    registry.register(typed(
        meta!(
            "env.package.search",
            "Look Up Package",
            "Looks a package up on PyPI by name: summary, latest version and all versions.",
            r#"{"name": "requests"}"#
        ),
        move |a: SearchArgs| {
            let s = s.clone();
            async move { ops::package_search(&s, a).await.map_err(CommandError::from) }
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{SearchEnv, Tools};
    use crate::search::{IndexProject, IndexVersion, StaticIndex};
    use serde_json::json;

    fn registry(index: StaticIndex) -> Registry {
        let svc = EnvService::new(
            None,
            Tools::default(),
            SearchEnv::default(),
            Arc::new(index),
        );
        let mut r = Registry::new();
        register(&mut r, Arc::new(svc));
        r
    }

    #[test]
    fn registers_the_documented_commands() {
        let r = registry(StaticIndex::new());
        let ids: Vec<_> = r.list().iter().map(|m| m.id).collect();
        assert_eq!(
            ids,
            [
                "env.interpreters.list",
                "env.lock",
                "env.package.search",
                "env.packages.add",
                "env.packages.list",
                "env.packages.remove",
                "env.project.info",
                "env.project.init",
                "env.project.tree",
                "env.python.install",
                "env.sync",
                "env.venv.create",
            ]
        );
    }

    #[test]
    fn every_example_is_valid_json() {
        let r = registry(StaticIndex::new());
        for m in r.list() {
            serde_json::from_str::<serde_json::Value>(m.args_example)
                .unwrap_or_else(|e| panic!("{}: {e}", m.id));
        }
    }

    #[tokio::test]
    async fn search_goes_through_the_index() {
        let index = StaticIndex::new().with(IndexProject {
            name: "requests".into(),
            summary: Some("HTTP for humans".into()),
            latest: "2.32.5".into(),
            versions: vec![IndexVersion {
                version: "2.32.5".into(),
                yanked: false,
                prerelease: false,
                requires_python: Some(">=3.9".into()),
                uploaded: None,
            }],
            requires_python: Some(">=3.9".into()),
            home_page: None,
            project_urls: Default::default(),
            license: None,
        });
        let r = registry(index);
        let out = r
            .invoke("env.package.search", json!({"name": "Requests"}))
            .await
            .unwrap();
        assert_eq!(out["latest"], "2.32.5");
        assert_eq!(out["versions"][0]["requires_python"], ">=3.9");
        let e = r
            .invoke("env.package.search", json!({"name": "nope"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, "env.package_not_found");
        let e = r.invoke("env.package.search", json!({})).await.unwrap_err();
        assert_eq!(e.code, "invalid_args");
    }

    #[tokio::test]
    async fn commands_that_need_uv_say_so() {
        let r = registry(StaticIndex::new());
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().to_string_lossy().into_owned();
        for (id, args) in [
            ("env.sync", json!({"project": p})),
            ("env.lock", json!({"project": p})),
            ("env.venv.create", json!({"path": format!("{p}/.venv")})),
            ("env.python.install", json!({"versions": ["3.13"]})),
            ("env.packages.add", json!({"packages": ["x"], "project": p})),
            (
                "env.packages.remove",
                json!({"packages": ["x"], "project": p}),
            ),
            ("env.project.init", json!({"path": format!("{p}/new")})),
            ("env.project.tree", json!({"project": p})),
        ] {
            let e = r.invoke(id, args).await.unwrap_err();
            assert_eq!(e.code, "env.tool_missing", "{id}: {}", e.message);
            assert!(e.message.contains("uv"), "{id}: {}", e.message);
        }
    }

    #[tokio::test]
    async fn project_info_reads_the_directory() {
        let r = registry(StaticIndex::new());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"demo\"\nrequires-python = \">=3.11\"\ndependencies = [\"httpx\"]\n",
        )
        .unwrap();
        let out = r
            .invoke("env.project.info", json!({"path": dir.path()}))
            .await
            .unwrap();
        assert_eq!(out["pyproject"]["name"], "demo");
        assert_eq!(out["pyproject"]["dependencies"][0], "httpx");
        assert_eq!(out["uv_lock"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn venv_mode_rejects_project_only_options() {
        let r = registry(StaticIndex::new());
        let e = r
            .invoke(
                "env.packages.add",
                json!({"packages": ["x"], "venv": "/nonexistent", "group": "dev"}),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_args");
        let e = r
            .invoke(
                "env.packages.add",
                json!({"packages": ["x"], "upgrade": true}),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_args");
    }

    #[tokio::test]
    async fn listing_without_an_environment_is_a_clear_error() {
        let r = registry(StaticIndex::new());
        let dir = tempfile::tempdir().unwrap();
        let e = r
            .invoke("env.packages.list", json!({"project": dir.path()}))
            .await
            .unwrap_err();
        assert_eq!(e.code, "env.no_interpreter");
        assert!(e.message.contains("env venv"));
        let e = r
            .invoke("env.packages.list", json!({"venv": dir.path()}))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_args");
    }

    #[tokio::test]
    async fn interpreters_list_reports_missing_tools() {
        let r = registry(StaticIndex::new());
        let out = r.invoke("env.interpreters.list", json!({})).await.unwrap();
        let tools = out["tools"].as_array().unwrap();
        assert_eq!(tools[0]["name"], "uv");
        assert_eq!(tools[0]["installed"], false);
        assert_eq!(out["interpreters"], json!([]));
    }
}
