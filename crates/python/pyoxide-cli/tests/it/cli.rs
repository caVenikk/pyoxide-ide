//! Runs the `pyoxide` binary. Nothing here needs the network: uv runs
//! offline with a private cache and no Python downloads. Tests that need uv
//! and an installed Python skip themselves where those are missing.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn pyoxide(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pyoxide"));
        // The developer's `UV_PYTHON`, `VIRTUAL_ENV` and the like must not
        // decide which Python uv picks for a test project.
        for name in py_env::process::steering_env() {
            cmd.env_remove(name);
        }
        cmd.args(args)
            .current_dir(self.dir.path())
            .env("PYOXIDE_DB_PROFILES", self.path("profiles.toml"))
            .env("UV_OFFLINE", "1")
            .env("UV_NO_CONFIG", "1")
            .env("UV_PYTHON_DOWNLOADS", "never")
            .env("UV_CACHE_DIR", self.path("uv-cache"))
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.pyoxide(args);
        assert!(
            out.status.success(),
            "pyoxide {args:?} failed: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        serde_json::from_str(&self.ok(&all)).unwrap()
    }
}

fn uv_available() -> bool {
    Command::new("uv")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// An interpreter the machine has, found with the CLI itself.
fn some_python(sb: &Sandbox) -> Option<String> {
    let report = sb.json(&["env", "interpreters"]);
    report["interpreters"]
        .as_array()?
        .iter()
        .find(|i| i["is_venv"] == false)
        .and_then(|i| i["path"].as_str())
        .map(str::to_owned)
}

fn path_str(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn commands_lists_both_registries() {
    let sb = Sandbox::new();
    let list = sb.json(&["commands"]);
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    for expected in [
        "db.query",
        "db.profile.list",
        "env.interpreters.list",
        "env.project.info",
        "env.venv.create",
        "env.packages.list",
        "env.packages.add",
        "env.packages.remove",
        "env.sync",
        "env.lock",
        "env.python.install",
        "env.package.search",
    ] {
        assert!(ids.contains(&expected), "missing {expected}");
    }
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "the registry lists commands sorted by id");

    let text = sb.ok(&["commands"]);
    assert!(text.starts_with("COMMAND"));
    assert!(text.contains("env.sync"));
}

#[test]
fn call_reaches_platform_and_python_commands() {
    let sb = Sandbox::new();
    let backends: Value = serde_json::from_str(&sb.ok(&["call", "db.backends", "{}"])).unwrap();
    assert!(backends.as_array().is_some_and(|b| !b.is_empty()));

    let profiles: Value = serde_json::from_str(&sb.ok(&["call", "db.profile.list"])).unwrap();
    assert_eq!(profiles, serde_json::json!([]));

    std::fs::write(
        sb.path("pyproject.toml"),
        "[project]\nname = \"callme\"\nversion = \"2.0\"\ndependencies = [\"httpx\"]\n",
    )
    .unwrap();
    let args = serde_json::json!({"path": path_str(sb.dir.path())}).to_string();
    let info: Value = serde_json::from_str(&sb.ok(&["call", "env.project.info", &args])).unwrap();
    assert_eq!(info["pyproject"]["name"], "callme");
    assert_eq!(info["pyproject"]["dependencies"][0], "httpx");
}

#[test]
fn call_failures_are_json_with_stable_codes() {
    let sb = Sandbox::new();
    let out = sb.pyoxide(&["call", "no.such.command", "{}"]);
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(err["code"], "not_found");

    let out = sb.pyoxide(&["call", "env.package.search", "{}"]);
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(err["code"], "invalid_args");

    let out = sb.pyoxide(&["call", "env.package.search", "not json"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not JSON"));
}

#[test]
fn search_rejects_invalid_names_without_the_network() {
    let sb = Sandbox::new();
    let out = sb.pyoxide(&["env", "search", "a/b"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a valid project name"));
    let out = sb.pyoxide(&["--json", "env", "search", "../x"]);
    let err: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(err["code"], "invalid_args");
}

#[test]
fn info_describes_a_project_directory() {
    let sb = Sandbox::new();
    std::fs::write(
        sb.path("pyproject.toml"),
        "[project]\nname = \"shown\"\nversion = \"1.2.3\"\nrequires-python = \">=3.11\"\ndependencies = [\"rich\"]\n\n[dependency-groups]\ndev = [\"pytest\"]\n",
    )
    .unwrap();
    std::fs::write(sb.path("requirements.txt"), "rich\npytest\n").unwrap();
    let text = sb.ok(&["env", "info", path_str(sb.dir.path())]);
    assert!(
        text.contains("shown 1.2.3, requires-python >=3.11"),
        "{text}"
    );
    assert!(text.contains("dependency-groups.dev (1): pytest"), "{text}");
    assert!(text.contains("requirements.txt: 2 requirements"), "{text}");
    assert!(text.contains("environment: none"), "{text}");

    let json = sb.json(&["env", "info", path_str(sb.dir.path())]);
    assert_eq!(json["pyproject"]["version"], "1.2.3");
    assert_eq!(json["managers"], serde_json::json!(["pip"]));
}

#[test]
fn info_of_a_missing_path_is_an_error() {
    let sb = Sandbox::new();
    let out = sb.pyoxide(&["env", "info", "no-such-dir"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error: "));
}

#[test]
fn interpreters_report_tools_even_when_nothing_is_found() {
    let sb = Sandbox::new();
    let report = sb.json(&["env", "interpreters"]);
    let tools: Vec<&str> = report["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(tools.contains(&"uv") && tools.contains(&"poetry") && tools.contains(&"conda"));
    let text = sb.ok(&["env", "interpreters"]);
    assert!(text.contains("Tools:"));
}

#[test]
fn offline_project_workflow() {
    if !uv_available() {
        eprintln!("skipped: uv is not installed");
        return;
    }
    let sb = Sandbox::new();
    let Some(python) = some_python(&sb) else {
        eprintln!("skipped: no Python interpreter found");
        return;
    };
    let project = sb.path("proj");
    let project_arg = path_str(&project);

    let out = sb.ok(&[
        "env",
        "init",
        "--python",
        &python,
        "--name",
        "cli-demo",
        project_arg,
    ]);
    assert!(out.starts_with("ok: uv init"), "{out}");
    assert!(project.join("pyproject.toml").is_file());

    sb.ok(&[
        "env",
        "add",
        "--project",
        project_arg,
        "--frozen",
        "--group",
        "dev",
        "pytest>=8",
        "ruff",
    ]);
    sb.ok(&[
        "env",
        "add",
        "--project",
        project_arg,
        "--frozen",
        "--optional",
        "fancy",
        "rich",
    ]);
    let info = sb.json(&["env", "info", project_arg]);
    assert_eq!(
        info["pyproject"]["dependency_groups"]["dev"]["requirements"],
        serde_json::json!(["pytest>=8", "ruff"])
    );
    assert_eq!(
        info["pyproject"]["optional_dependencies"]["fancy"],
        serde_json::json!(["rich"])
    );
    sb.ok(&[
        "env",
        "remove",
        "--project",
        project_arg,
        "--frozen",
        "--optional",
        "fancy",
        "rich",
    ]);
    sb.ok(&[
        "env",
        "remove",
        "--project",
        project_arg,
        "--frozen",
        "--group",
        "dev",
        "pytest",
        "ruff",
    ]);

    sb.ok(&["env", "lock", "--project", project_arg]);
    assert!(project.join("uv.lock").is_file());
    sb.ok(&["env", "sync", "--project", project_arg]);

    let info = sb.json(&["env", "info", project_arg]);
    assert!(info["venv"].as_str().is_some_and(|v| v.ends_with(".venv")));
    assert_eq!(info["uv_lock"]["packages"][0]["name"], "cli-demo");

    let list = sb.json(&["env", "packages", "--project", project_arg]);
    assert_eq!(list["backend"], "uv");
    assert_eq!(list["packages"], serde_json::json!([]));
    let text = sb.ok(&["env", "packages", "--project", project_arg]);
    assert!(text.starts_with("0 packages in "), "{text}");

    let tree = sb.ok(&["env", "tree", "--project", project_arg, "--frozen"]);
    assert!(tree.starts_with("cli-demo v0.1.0"), "{tree}");

    // A separate environment, created and listed by path.
    let venv = sb.path("other-venv");
    let created = sb.json(&["env", "venv", "--python", &python, path_str(&venv)]);
    assert_eq!(created["interpreter"]["is_venv"], true);
    let again = sb.pyoxide(&["env", "venv", "--python", &python, path_str(&venv)]);
    assert_eq!(again.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&again.stderr).contains("already exists"));
    sb.ok(&[
        "env",
        "venv",
        "--clear",
        "--python",
        &python,
        path_str(&venv),
    ]);
    let list = sb.json(&["env", "packages", "--venv", path_str(&venv)]);
    assert_eq!(list["packages"], serde_json::json!([]));

    // Progress goes to stderr, results to stdout.
    let out = sb.pyoxide(&["env", "sync", "--project", project_arg]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Resolved 1 package"));
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("ok: uv sync"));
    // --json keeps stderr quiet and puts the log in the result.
    let out = sb.pyoxide(&["--json", "env", "sync", "--project", project_arg]);
    assert!(String::from_utf8_lossy(&out.stderr).is_empty());
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["backend"], "uv");
    assert!(
        result["log"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("Resolved"))
    );
}

#[test]
fn failures_exit_with_a_message_and_a_code() {
    if !uv_available() {
        eprintln!("skipped: uv is not installed");
        return;
    }
    let sb = Sandbox::new();
    let out = sb.pyoxide(&["--json", "env", "python-install", "3.99"]);
    assert_eq!(out.status.code(), Some(1));
    let err: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(err["code"], "env.no_interpreter");
    assert_eq!(err["tool"], "uv");

    let out = sb.pyoxide(&["env", "packages", "--project", path_str(sb.dir.path())]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no virtual environment"));
}
