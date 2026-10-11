use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::error::{EnvError, ErrorKind, Result};
use crate::paths::environment_python;
use crate::version::normalize_name;

/// A package manager or workflow a project is set up for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Manager {
    Uv,
    Poetry,
    Pdm,
    Pipenv,
    Conda,
    /// Plain `requirements*.txt` files.
    Pip,
}

/// A requirement string split into the parts an IDE shows and matches on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    pub raw: String,
    /// PEP 503 normalized.
    pub name: String,
    pub extras: Vec<String>,
    /// `>=2.0,<3`, or empty.
    pub specifier: String,
    /// The part after `;`.
    pub marker: Option<String>,
    /// For `name @ url` requirements.
    pub url: Option<String>,
}

/// Split a PEP 508 requirement. Forgiving: a string that does not start with
/// a project name yields `None`.
pub fn parse_requirement(raw: &str) -> Option<Requirement> {
    let raw = raw.trim();
    let (main, marker) = match raw.split_once(';') {
        Some((m, mk)) => (
            m.trim(),
            Some(mk.trim().to_owned()).filter(|s| !s.is_empty()),
        ),
        None => (raw, None),
    };
    let name_end = main
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .unwrap_or(main.len());
    let name = &main[..name_end];
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return None;
    }
    let mut rest = main[name_end..].trim_start();
    let mut extras = Vec::new();
    if let Some(r) = rest.strip_prefix('[') {
        let close = r.find(']')?;
        extras = r[..close]
            .split(',')
            .map(|e| e.trim().to_owned())
            .filter(|e| !e.is_empty())
            .collect();
        rest = r[close + 1..].trim_start();
    }
    let (specifier, url) = match rest.strip_prefix('@') {
        Some(u) => (String::new(), Some(u.trim().to_owned())),
        None => (
            rest.trim_matches(|c| c == '(' || c == ')')
                .split_whitespace()
                .collect::<String>(),
            None,
        ),
    };
    Some(Requirement {
        raw: raw.to_owned(),
        name: normalize_name(name),
        extras,
        specifier,
        marker,
        url,
    })
}

/// A PEP 735 dependency group.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyGroup {
    pub requirements: Vec<String>,
    /// Names of groups included with `{include-group = "..."}`.
    pub includes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolUv {
    /// The legacy `dev-dependencies` list.
    pub dev_dependencies: Vec<String>,
    /// `package = true|false`: whether the project is built and installed.
    pub package: Option<bool>,
    pub managed: Option<bool>,
    pub default_groups: Vec<String>,
    pub workspace_members: Vec<String>,
    pub workspace_exclude: Vec<String>,
    /// Packages with an entry in `[tool.uv.sources]`.
    pub sources: Vec<String>,
}

/// The parts of `pyproject.toml` the IDE reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PyProject {
    /// `[project] name`, or the Poetry name for Poetry-only projects.
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub requires_python: Option<String>,
    pub dependencies: Vec<String>,
    pub optional_dependencies: BTreeMap<String, Vec<String>>,
    pub dependency_groups: BTreeMap<String, DependencyGroup>,
    /// Fields declared `dynamic`.
    pub dynamic: Vec<String>,
    /// `[build-system] build-backend`.
    pub build_backend: Option<String>,
    pub uv: Option<ToolUv>,
    /// Has a `[tool.poetry]` table.
    pub poetry: bool,
    pub pdm: bool,
    /// Parts that were present but of an unexpected shape.
    pub warnings: Vec<String>,
}

impl PyProject {
    /// Every requirement declared for development in the PEP 735 groups,
    /// with `include-group` expanded.
    pub fn group_requirements(&self, group: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        self.collect_group(group, &mut seen, &mut out);
        out
    }

    fn collect_group(&self, name: &str, seen: &mut BTreeSet<String>, out: &mut Vec<String>) {
        let key = normalize_name(name);
        if !seen.insert(key.clone()) {
            return;
        }
        let found = self
            .dependency_groups
            .iter()
            .find(|(k, _)| normalize_name(k) == key);
        if let Some((_, g)) = found {
            out.extend(g.requirements.iter().cloned());
            for inc in &g.includes {
                self.collect_group(inc, seen, out);
            }
        }
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn string(t: &Table, key: &str) -> Option<String> {
    t.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Read `pyproject.toml` text. A syntax error is an error; a field of the
/// wrong type is skipped with a warning so one typo does not hide the rest.
pub fn parse_pyproject(text: &str) -> Result<PyProject> {
    let doc: Table = text.parse().map_err(|e: toml::de::Error| {
        EnvError::parse(format!("pyproject.toml: {}", first_line(&e.to_string())))
    })?;
    let mut out = PyProject::default();

    if let Some(project) = doc.get("project") {
        match project.as_table() {
            Some(p) => {
                out.name = string(p, "name");
                out.version = string(p, "version");
                out.description = string(p, "description");
                out.requires_python = string(p, "requires-python");
                out.dynamic = strings(p.get("dynamic"));
                out.dependencies = strings(p.get("dependencies"));
                match p.get("optional-dependencies") {
                    Some(Value::Table(t)) => {
                        for (k, v) in t {
                            out.optional_dependencies
                                .insert(k.clone(), strings(Some(v)));
                        }
                    }
                    Some(_) => out
                        .warnings
                        .push("[project.optional-dependencies] is not a table".into()),
                    None => {}
                }
            }
            None => out.warnings.push("[project] is not a table".into()),
        }
    }

    match doc.get("dependency-groups") {
        Some(Value::Table(t)) => {
            for (name, items) in t {
                let mut group = DependencyGroup::default();
                match items.as_array() {
                    Some(items) => {
                        for item in items {
                            match item {
                                Value::String(s) => group.requirements.push(s.clone()),
                                Value::Table(inc) => {
                                    match inc.get("include-group").and_then(Value::as_str) {
                                        Some(g) => group.includes.push(g.to_owned()),
                                        None => out.warnings.push(format!(
                                            "[dependency-groups] {name}: unknown table entry"
                                        )),
                                    }
                                }
                                _ => out.warnings.push(format!(
                                    "[dependency-groups] {name}: entry is not a string"
                                )),
                            }
                        }
                    }
                    None => out
                        .warnings
                        .push(format!("[dependency-groups] {name} is not a list")),
                }
                out.dependency_groups.insert(name.clone(), group);
            }
        }
        Some(_) => out
            .warnings
            .push("[dependency-groups] is not a table".into()),
        None => {}
    }

    if let Some(backend) = doc
        .get("build-system")
        .and_then(Value::as_table)
        .and_then(|b| b.get("build-backend"))
        .and_then(Value::as_str)
    {
        out.build_backend = Some(backend.to_owned());
    }

    if let Some(tool) = doc.get("tool").and_then(Value::as_table) {
        if let Some(uv) = tool.get("uv").and_then(Value::as_table) {
            let ws = uv.get("workspace").and_then(Value::as_table);
            out.uv = Some(ToolUv {
                dev_dependencies: strings(uv.get("dev-dependencies")),
                package: uv.get("package").and_then(Value::as_bool),
                managed: uv.get("managed").and_then(Value::as_bool),
                default_groups: strings(uv.get("default-groups")),
                workspace_members: strings(ws.and_then(|w| w.get("members"))),
                workspace_exclude: strings(ws.and_then(|w| w.get("exclude"))),
                sources: uv
                    .get("sources")
                    .and_then(Value::as_table)
                    .map(|s| s.keys().cloned().collect())
                    .unwrap_or_default(),
            });
        }
        if let Some(poetry) = tool.get("poetry").and_then(Value::as_table) {
            out.poetry = true;
            if out.name.is_none() {
                out.name = string(poetry, "name");
            }
            if out.version.is_none() {
                out.version = string(poetry, "version");
            }
            if out.description.is_none() {
                out.description = string(poetry, "description");
            }
        }
        out.pdm = tool.get("pdm").is_some();
    }
    Ok(out)
}

fn first_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('|'))
        .unwrap_or(s)
        .trim()
        .to_owned()
}

/// A package pinned in `uv.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedPackage {
    pub name: String,
    pub version: Option<String>,
    /// `registry`, `virtual`, `editable`, `git`, `directory`, `path`, `url`.
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UvLock {
    pub path: PathBuf,
    pub requires_python: Option<String>,
    pub packages: Vec<LockedPackage>,
}

pub fn parse_uv_lock(path: &Path, text: &str) -> Result<UvLock> {
    let doc: Table = text.parse().map_err(|e: toml::de::Error| {
        EnvError::parse(format!("uv.lock: {}", first_line(&e.to_string())))
    })?;
    let packages = doc
        .get("package")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_table)
                .filter_map(|p| {
                    Some(LockedPackage {
                        name: string(p, "name")?,
                        version: string(p, "version"),
                        source: p
                            .get("source")
                            .and_then(Value::as_table)
                            .and_then(|s| s.keys().next().cloned()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(UvLock {
        path: path.to_path_buf(),
        requires_python: string(&doc, "requires-python"),
        packages,
    })
}

/// A requirements file (`requirements.txt`, `requirements-dev.txt`, `requirements/*.txt`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementsFile {
    pub path: PathBuf,
    pub requirements: Vec<String>,
    /// Files pulled in with `-r` / `-c`.
    pub includes: Vec<String>,
}

pub fn parse_requirements_txt(path: &Path, text: &str) -> RequirementsFile {
    let mut out = RequirementsFile {
        path: path.to_path_buf(),
        ..RequirementsFile::default()
    };
    let mut pending = String::new();
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(cont) = line.strip_suffix('\\') {
            pending.push_str(cont);
            continue;
        }
        pending.push_str(line);
        let full = std::mem::take(&mut pending);
        let full = strip_comment(&full).trim().to_owned();
        if full.is_empty() {
            continue;
        }
        let include = ["-r ", "--requirement ", "-c ", "--constraint "]
            .iter()
            .find_map(|p| full.strip_prefix(p))
            .or_else(|| {
                ["--requirement=", "--constraint="]
                    .iter()
                    .find_map(|p| full.strip_prefix(p))
            });
        if let Some(inc) = include {
            out.includes.push(inc.trim().to_owned());
        } else if full.starts_with("-e ")
            || full.starts_with("--editable ")
            || !full.starts_with('-')
        {
            out.requirements.push(full);
        }
        // Other options (`--index-url`, `--hash`, ...) are not requirements.
    }
    out
}

fn strip_comment(line: &str) -> &str {
    if line.trim_start().starts_with('#') {
        return "";
    }
    match line.find(" #") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// PEP 723 inline script metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptMetadata {
    pub path: Option<PathBuf>,
    pub requires_python: Option<String>,
    pub dependencies: Vec<String>,
    /// The `[tool.uv]` table, if any, as JSON.
    pub tool_uv: Option<serde_json::Value>,
    /// The TOML between the markers, comment prefixes removed.
    pub raw: String,
}

/// Find the `# /// script` block of a Python source file. Returns `Ok(None)`
/// when there is none and an error when there are several or the block is
/// not valid TOML (PEP 723 says tools must not guess then).
pub fn parse_script_metadata(source: &str) -> Result<Option<ScriptMetadata>> {
    let lines: Vec<&str> = source.lines().collect();
    let mut found: Option<String> = None;
    let mut i = 0;
    while i < lines.len() {
        let Some(kind) = lines[i].strip_prefix("# /// ") else {
            i += 1;
            continue;
        };
        let kind = kind.trim_end();
        if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            i += 1;
            continue;
        }
        // The block is the run of comment lines after the opening marker; it
        // ends at the last `# ///` of that run.
        let mut j = i + 1;
        let mut last_close = None;
        while j < lines.len() && (lines[j] == "#" || lines[j].starts_with("# ")) {
            if lines[j].trim_end() == "# ///" {
                last_close = Some(j);
            }
            j += 1;
        }
        let Some(close) = last_close else {
            i += 1;
            continue;
        };
        if kind == "script" {
            if found.is_some() {
                return Err(EnvError::parse(
                    "the script has more than one `# /// script` block",
                ));
            }
            let body: Vec<&str> = lines[i + 1..close]
                .iter()
                .map(|l| {
                    l.strip_prefix("# ")
                        .unwrap_or(l.strip_prefix('#').unwrap_or(l))
                })
                .collect();
            found = Some(body.join("\n") + "\n");
        }
        i = close + 1;
    }
    let Some(raw) = found else {
        return Ok(None);
    };
    let doc: Table = raw.parse().map_err(|e: toml::de::Error| {
        EnvError::parse(format!("script metadata: {}", first_line(&e.to_string())))
    })?;
    Ok(Some(ScriptMetadata {
        path: None,
        requires_python: string(&doc, "requires-python"),
        dependencies: strings(doc.get("dependencies")),
        tool_uv: doc
            .get("tool")
            .and_then(Value::as_table)
            .and_then(|t| t.get("uv"))
            .and_then(|v| serde_json::to_value(v).ok()),
        raw,
    }))
}

/// What the IDE knows about a project directory (or a single script).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub root: PathBuf,
    pub pyproject_path: Option<PathBuf>,
    pub pyproject: Option<PyProject>,
    pub uv_lock: Option<UvLock>,
    /// Lock files of other tools: `poetry.lock`, `pdm.lock`, `Pipfile.lock`.
    pub other_locks: Vec<PathBuf>,
    pub requirements_files: Vec<RequirementsFile>,
    /// First line of `.python-version`.
    pub python_version_file: Option<String>,
    /// The project's virtual environment directory, if one exists.
    pub venv: Option<PathBuf>,
    pub managers: Vec<Manager>,
    /// Set when the path given was a Python script with inline metadata.
    pub script: Option<ScriptMetadata>,
    /// Files that could not be read, with the reason.
    pub problems: Vec<String>,
}

/// The nearest ancestor of `start` (itself included) that has a
/// `pyproject.toml`; `start` when there is none.
pub fn find_project_root(start: &Path) -> PathBuf {
    let start = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };
    start
        .ancestors()
        .find(|d| d.join("pyproject.toml").is_file())
        .unwrap_or(start)
        .to_path_buf()
}

/// Names tried for a project environment, in order.
pub const VENV_DIR_NAMES: [&str; 2] = [".venv", "venv"];

/// The project's virtual environment directory: `UV_PROJECT_ENVIRONMENT`
/// (relative to the project), then `.venv` and `venv` in the project and its
/// ancestors.
pub fn find_project_venv(root: &Path, uv_project_environment: Option<&Path>) -> Option<PathBuf> {
    if let Some(env) = uv_project_environment {
        let dir = if env.is_absolute() {
            env.to_path_buf()
        } else {
            root.join(env)
        };
        if environment_python(&dir).is_some() {
            return Some(dir);
        }
    }
    root.ancestors()
        .flat_map(|d| VENV_DIR_NAMES.iter().map(move |n| d.join(n)))
        .find(|d| d.join("pyvenv.cfg").is_file() && environment_python(d).is_some())
}

/// Read everything the IDE needs to know about the project at `path`: a
/// directory inside the project, its `pyproject.toml`, or a `.py` script.
pub fn load_project(path: &Path) -> Result<ProjectInfo> {
    let meta = std::fs::metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            EnvError::new(
                ErrorKind::NoProject,
                format!("{} does not exist", path.display()),
            )
        } else {
            EnvError::io(path.display(), &e)
        }
    })?;
    let mut script = None;
    let mut problems = Vec::new();
    if meta.is_file() && path.extension().is_some_and(|e| e == "py") {
        match std::fs::read_to_string(path) {
            Ok(src) => match parse_script_metadata(&src) {
                Ok(Some(mut m)) => {
                    m.path = Some(path.to_path_buf());
                    script = Some(m);
                }
                Ok(None) => {}
                Err(e) => problems.push(format!("{}: {}", path.display(), e.message)),
            },
            Err(e) => problems.push(format!("{}: {e}", path.display())),
        }
    }

    let root = find_project_root(path);
    let pyproject_path = root.join("pyproject.toml");
    let (pyproject_path, pyproject) = if pyproject_path.is_file() {
        match std::fs::read_to_string(&pyproject_path)
            .map_err(|e| EnvError::io(pyproject_path.display(), &e))
            .and_then(|t| parse_pyproject(&t))
        {
            Ok(p) => (Some(pyproject_path), Some(p)),
            Err(e) => {
                problems.push(e.message);
                (Some(pyproject_path), None)
            }
        }
    } else {
        (None, None)
    };

    let lock_path = root.join("uv.lock");
    let uv_lock = if lock_path.is_file() {
        match std::fs::read_to_string(&lock_path)
            .map_err(|e| EnvError::io(lock_path.display(), &e))
            .and_then(|t| parse_uv_lock(&lock_path, &t))
        {
            Ok(l) => Some(l),
            Err(e) => {
                problems.push(e.message);
                None
            }
        }
    } else {
        None
    };

    let other_locks: Vec<PathBuf> = ["poetry.lock", "pdm.lock", "Pipfile.lock"]
        .iter()
        .map(|n| root.join(n))
        .filter(|p| p.is_file())
        .collect();

    let requirements_files = find_requirements_files(&root, &mut problems);

    let python_version_file = std::fs::read_to_string(root.join(".python-version"))
        .ok()
        .and_then(|t| {
            t.lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_owned)
        });

    let venv = find_project_venv(
        &root,
        std::env::var_os("UV_PROJECT_ENVIRONMENT")
            .as_deref()
            .map(Path::new),
    );

    let mut managers = BTreeSet::new();
    if uv_lock.is_some() || pyproject.as_ref().is_some_and(|p| p.uv.is_some()) {
        managers.insert(Manager::Uv);
    }
    if pyproject.as_ref().is_some_and(|p| p.poetry) || root.join("poetry.lock").is_file() {
        managers.insert(Manager::Poetry);
    }
    if pyproject.as_ref().is_some_and(|p| p.pdm) || root.join("pdm.lock").is_file() {
        managers.insert(Manager::Pdm);
    }
    if root.join("Pipfile").is_file() {
        managers.insert(Manager::Pipenv);
    }
    if root.join("environment.yml").is_file() || root.join("environment.yaml").is_file() {
        managers.insert(Manager::Conda);
    }
    if !requirements_files.is_empty() {
        managers.insert(Manager::Pip);
    }

    Ok(ProjectInfo {
        root,
        pyproject_path,
        pyproject,
        uv_lock,
        other_locks,
        requirements_files,
        python_version_file,
        venv,
        managers: managers.into_iter().collect(),
        script,
        problems,
    })
}

fn find_requirements_files(root: &Path, problems: &mut Vec<String>) -> Vec<RequirementsFile> {
    let mut paths = Vec::new();
    for dir in [root.to_path_buf(), root.join("requirements")] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let in_root = dir == root;
            let wanted = if in_root {
                name.starts_with("requirements") && name.ends_with(".txt")
            } else {
                name.ends_with(".txt")
            };
            if wanted && p.is_file() {
                paths.push(p);
            }
        }
    }
    paths.sort();
    paths
        .into_iter()
        .filter_map(|p| match std::fs::read_to_string(&p) {
            Ok(t) => Some(parse_requirements_txt(&p, &t)),
            Err(e) => {
                problems.push(format!("{}: {e}", p.display()));
                None
            }
        })
        .collect()
}

/// Async wrapper: file reads run on the blocking pool.
pub async fn load_project_async(path: PathBuf) -> Result<ProjectInfo> {
    tokio::task::spawn_blocking(move || load_project(&path))
        .await
        .map_err(|e| {
            EnvError::new(
                ErrorKind::Failed,
                format!("reading the project failed: {e}"),
            )
        })?
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
[project]
name = "My_Project"
version = "1.2.3"
description = "demo"
requires-python = ">=3.10"
dynamic = ["readme"]
dependencies = ["requests>=2.31", "rich[jupyter]==13.*; python_version >= '3.9'"]

[project.optional-dependencies]
fancy = ["pillow"]
speed = ["orjson", "uvloop; sys_platform != 'win32'"]

[dependency-groups]
test = ["pytest>=8", "coverage"]
dev = [{include-group = "test"}, "ruff"]
lint = ["ruff", { include-group = "dev" }]

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"

[tool.uv]
dev-dependencies = ["black"]
package = false
default-groups = ["dev"]

[tool.uv.sources]
internal = { path = "../internal" }
other = { git = "https://example.org/other.git" }

[tool.uv.workspace]
members = ["packages/*"]
exclude = ["packages/legacy"]
"#;

    #[test]
    fn reads_pep_621_and_uv_tables() {
        let p = parse_pyproject(FULL).unwrap();
        assert_eq!(p.name.as_deref(), Some("My_Project"));
        assert_eq!(p.version.as_deref(), Some("1.2.3"));
        assert_eq!(p.requires_python.as_deref(), Some(">=3.10"));
        assert_eq!(p.dynamic, ["readme"]);
        assert_eq!(p.dependencies.len(), 2);
        assert_eq!(p.optional_dependencies["fancy"], ["pillow"]);
        assert_eq!(p.optional_dependencies["speed"].len(), 2);
        assert_eq!(p.build_backend.as_deref(), Some("hatchling.build"));
        let uv = p.uv.as_ref().unwrap();
        assert_eq!(uv.dev_dependencies, ["black"]);
        assert_eq!(uv.package, Some(false));
        assert_eq!(uv.default_groups, ["dev"]);
        assert_eq!(uv.sources, ["internal", "other"]);
        assert_eq!(uv.workspace_members, ["packages/*"]);
        assert_eq!(uv.workspace_exclude, ["packages/legacy"]);
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        assert!(!p.poetry);
    }

    #[test]
    fn dependency_groups_follow_includes_without_looping() {
        let p = parse_pyproject(FULL).unwrap();
        assert_eq!(p.dependency_groups["dev"].includes, ["test"]);
        assert_eq!(
            p.group_requirements("dev"),
            ["ruff", "pytest>=8", "coverage"]
        );
        // lint -> dev -> test, and ruff appears in both lint and dev.
        let lint = p.group_requirements("lint");
        assert_eq!(lint, ["ruff", "ruff", "pytest>=8", "coverage"]);
        assert!(p.group_requirements("missing").is_empty());

        let cyclic = parse_pyproject(
            "[dependency-groups]\na = [\"x\", {include-group = \"b\"}]\nb = [\"y\", {include-group = \"a\"}]\n",
        )
        .unwrap();
        assert_eq!(cyclic.group_requirements("a"), ["x", "y"]);
    }

    #[test]
    fn reads_poetry_only_projects() {
        let p = parse_pyproject(
            "[tool.poetry]\nname = \"poem\"\nversion = \"0.3.0\"\ndescription = \"d\"\n\n[tool.poetry.dependencies]\npython = \"^3.11\"\n",
        )
        .unwrap();
        assert!(p.poetry);
        assert_eq!(p.name.as_deref(), Some("poem"));
        assert_eq!(p.version.as_deref(), Some("0.3.0"));
        assert!(p.dependencies.is_empty());
    }

    #[test]
    fn minimal_and_odd_files() {
        let p = parse_pyproject("").unwrap();
        assert_eq!(p, PyProject::default());
        let p = parse_pyproject("[project]\nname = \"x\"\ndependencies = \"requests\"\n").unwrap();
        assert!(p.dependencies.is_empty());
        let p = parse_pyproject("[project.optional-dependencies]\nextra = 3\n").unwrap();
        assert_eq!(p.optional_dependencies["extra"], Vec::<String>::new());
        let p = parse_pyproject("[dependency-groups]\ndev = \"nope\"\ntest = [1]\n").unwrap();
        assert_eq!(p.warnings.len(), 2);
        let p = parse_pyproject("project = 3\n").unwrap();
        assert_eq!(p.warnings, ["[project] is not a table"]);
        let p = parse_pyproject("[tool.pdm]\n").unwrap();
        assert!(p.pdm);
    }

    #[test]
    fn syntax_errors_name_the_file() {
        let e = parse_pyproject("[project\nname = 1").unwrap_err();
        assert_eq!(e.kind, ErrorKind::Parse);
        assert!(e.message.starts_with("pyproject.toml: "), "{}", e.message);
    }

    #[test]
    fn splits_requirements() {
        let r = parse_requirement("Requests[security, socks]>=2.31,<3 ; python_version >= '3.8'")
            .unwrap();
        assert_eq!(r.name, "requests");
        assert_eq!(r.extras, ["security", "socks"]);
        assert_eq!(r.specifier, ">=2.31,<3");
        assert_eq!(r.marker.as_deref(), Some("python_version >= '3.8'"));
        let r = parse_requirement("pip @ https://example.org/pip.whl").unwrap();
        assert_eq!(r.name, "pip");
        assert_eq!(r.url.as_deref(), Some("https://example.org/pip.whl"));
        let r = parse_requirement("zope.interface").unwrap();
        assert_eq!(r.name, "zope-interface");
        assert_eq!(r.specifier, "");
        assert_eq!(
            parse_requirement("flask (>=2, <3)").unwrap().specifier,
            ">=2,<3"
        );
        assert!(parse_requirement("").is_none());
        assert!(parse_requirement("-r other.txt").is_none());
        assert!(parse_requirement("name[unclosed").is_none());
    }

    #[test]
    fn reads_uv_lock() {
        let text = r#"
version = 1
revision = 3
requires-python = ">=3.12"

[[package]]
name = "certifi"
version = "2024.8.30"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "demo"
version = "0.1.0"
source = { virtual = "." }
dependencies = [{ name = "certifi" }]

[[package]]
name = "local"
version = "1.0"
source = { editable = "../local" }
"#;
        let lock = parse_uv_lock(Path::new("uv.lock"), text).unwrap();
        assert_eq!(lock.requires_python.as_deref(), Some(">=3.12"));
        assert_eq!(lock.packages.len(), 3);
        assert_eq!(lock.packages[0].name, "certifi");
        assert_eq!(lock.packages[0].source.as_deref(), Some("registry"));
        assert_eq!(lock.packages[1].source.as_deref(), Some("virtual"));
        assert_eq!(lock.packages[2].source.as_deref(), Some("editable"));
        assert!(parse_uv_lock(Path::new("uv.lock"), "[[package").is_err());
    }

    #[test]
    fn reads_requirements_files() {
        let text = "\
# base requirements
-r base.txt
--constraint=constraints.txt
requests==2.32.5  # pinned
flask>=3 \\
    ; python_version >= '3.9'
-e ./local-pkg
--index-url https://example.org/simple
   # indented comment

git+https://example.org/x.git#egg=x
";
        let f = parse_requirements_txt(Path::new("requirements.txt"), text);
        assert_eq!(f.includes, ["base.txt", "constraints.txt"]);
        assert_eq!(
            f.requirements,
            [
                "requests==2.32.5",
                "flask>=3     ; python_version >= '3.9'",
                "-e ./local-pkg",
                "git+https://example.org/x.git#egg=x",
            ]
        );
    }

    #[test]
    fn finds_script_metadata() {
        let src = r#"#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "requests<3",
#   "rich",
# ]
#
# [tool.uv]
# exclude-newer = "2025-01-01T00:00:00Z"
# ///

import requests
print(requests.get("https://example.org"))
"#;
        let m = parse_script_metadata(src).unwrap().unwrap();
        assert_eq!(m.requires_python.as_deref(), Some(">=3.12"));
        assert_eq!(m.dependencies, ["requests<3", "rich"]);
        assert_eq!(m.tool_uv.unwrap()["exclude-newer"], "2025-01-01T00:00:00Z");
        assert!(m.raw.contains("dependencies = ["));
    }

    #[test]
    fn script_metadata_edge_cases() {
        assert!(parse_script_metadata("print('hi')\n").unwrap().is_none());
        // Other block types are ignored.
        assert!(
            parse_script_metadata("# /// example\n# x = 1\n# ///\n")
                .unwrap()
                .is_none()
        );
        // An unterminated block is not a block.
        assert!(
            parse_script_metadata("# /// script\n# dependencies = []\n")
                .unwrap()
                .is_none()
        );
        // Empty dependencies.
        let m = parse_script_metadata("# /// script\n# dependencies = []\n# ///\n")
            .unwrap()
            .unwrap();
        assert!(m.dependencies.is_empty());
        // Two script blocks are an error.
        let two = "# /// script\n# dependencies = []\n# ///\nx = 1\n# /// script\n# dependencies = []\n# ///\n";
        assert!(parse_script_metadata(two).is_err());
        // Invalid TOML inside is an error.
        assert!(parse_script_metadata("# /// script\n# dependencies = [\n# ///\n").is_err());
        // The block ends at the last closing marker of the comment run.
        let tricky = "# /// script\n# dependencies = [\"a\"]\n# ///\n# ///\n";
        assert!(parse_script_metadata(tricky).is_err());
    }

    fn write(dir: &Path, name: &str, text: &str) {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn loads_a_uv_project_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "pyproject.toml", FULL);
        write(
            root,
            "uv.lock",
            "version = 1\nrequires-python = \">=3.10\"\n\n[[package]]\nname = \"my-project\"\nversion = \"1.2.3\"\nsource = { virtual = \".\" }\n",
        );
        write(root, "requirements.txt", "requests\n");
        write(
            root,
            "requirements-dev.txt",
            "-r requirements.txt\npytest\n",
        );
        write(root, "requirements/extra.txt", "rich\n");
        write(root, "notes.txt", "ignored\n");
        write(root, ".python-version", "3.12\n");
        write(root, ".venv/pyvenv.cfg", "home = /x\n");
        let py = crate::paths::venv_python(&root.join(".venv"));
        write(root, py.strip_prefix(root).unwrap().to_str().unwrap(), "");

        let info = load_project(root).unwrap();
        assert_eq!(info.root, root);
        assert_eq!(
            info.pyproject.as_ref().unwrap().name.as_deref(),
            Some("My_Project")
        );
        assert_eq!(info.uv_lock.as_ref().unwrap().packages.len(), 1);
        let names: Vec<String> = info
            .requirements_files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["extra.txt", "requirements-dev.txt", "requirements.txt"]
        );
        assert_eq!(info.python_version_file.as_deref(), Some("3.12"));
        assert_eq!(info.venv, Some(root.join(".venv")));
        assert_eq!(info.managers, [Manager::Uv, Manager::Pip]);
        assert!(info.problems.is_empty(), "{:?}", info.problems);
        assert!(info.script.is_none());
    }

    #[test]
    fn project_root_is_found_from_a_subdirectory_and_from_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "pyproject.toml", "[project]\nname = \"p\"\n");
        write(root, "src/pkg/mod.py", "x = 1\n");
        assert_eq!(find_project_root(&root.join("src/pkg")), root);
        assert_eq!(find_project_root(&root.join("src/pkg/mod.py")), root);
        let info = load_project(&root.join("src/pkg/mod.py")).unwrap();
        assert_eq!(info.root, root);
        assert!(info.script.is_none());
    }

    #[test]
    fn loads_a_script_without_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("tool.py");
        std::fs::write(
            &script,
            "# /// script\n# dependencies = [\"httpx\"]\n# ///\nprint(1)\n",
        )
        .unwrap();
        let info = load_project(&script).unwrap();
        let m = info.script.unwrap();
        assert_eq!(m.dependencies, ["httpx"]);
        assert_eq!(m.path.as_deref(), Some(script.as_path()));
        assert!(info.pyproject.is_none());
        assert!(info.managers.is_empty());
    }

    #[test]
    fn detects_other_managers_and_reports_broken_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "pyproject.toml", "[tool.poetry]\nname = \"p\"\n");
        write(root, "poetry.lock", "# lock\n");
        write(root, "Pipfile", "");
        write(root, "environment.yml", "name: x\n");
        let info = load_project(root).unwrap();
        assert_eq!(
            info.managers,
            [Manager::Poetry, Manager::Pipenv, Manager::Conda]
        );
        assert_eq!(info.other_locks, [root.join("poetry.lock")]);

        write(root, "pyproject.toml", "[project\n");
        let info = load_project(root).unwrap();
        assert!(info.pyproject.is_none());
        assert_eq!(info.pyproject_path, Some(root.join("pyproject.toml")));
        assert_eq!(info.problems.len(), 1);
        assert!(info.problems[0].contains("pyproject.toml"));
    }

    #[test]
    fn missing_paths_are_no_project() {
        let e = load_project(Path::new("/definitely/not/here")).unwrap_err();
        assert_eq!(e.kind, ErrorKind::NoProject);
    }

    #[test]
    fn venv_lookup_walks_up_and_honors_the_override() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let venv = root.join("venv");
        write(&venv, "pyvenv.cfg", "");
        let py = crate::paths::venv_python(&venv);
        std::fs::create_dir_all(py.parent().unwrap()).unwrap();
        std::fs::write(&py, "").unwrap();
        let sub = root.join("packages/app");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(find_project_venv(&sub, None), Some(venv.clone()));
        // A directory without pyvenv.cfg is not a venv.
        assert_eq!(
            find_project_venv(Path::new("/definitely/not/here"), None),
            None
        );
        // UV_PROJECT_ENVIRONMENT wins, relative to the project.
        let custom = sub.join("env-x");
        write(&custom, "pyvenv.cfg", "");
        let cpy = crate::paths::venv_python(&custom);
        std::fs::create_dir_all(cpy.parent().unwrap()).unwrap();
        std::fs::write(&cpy, "").unwrap();
        assert_eq!(
            find_project_venv(&sub, Some(Path::new("env-x"))),
            Some(custom)
        );
    }
}
