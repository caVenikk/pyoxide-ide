//! Python environments on disk, read without starting Python: the
//! `pyvenv.cfg` of a virtual environment, the Python version of a conda
//! environment, and the environments a project holds. Everything here is
//! blocking file system access and cheap enough to run on a background
//! thread whenever a project opens.

use std::path::{Path, PathBuf};

use crate::paths::{canonical_key, environment_python};
use crate::project::find_project_venv;

/// The `key = value` lines of a virtual environment's `pyvenv.cfg`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PyvenvCfg {
    entries: Vec<(String, String)>,
}

impl PyvenvCfg {
    /// Each line split at its first `=`, both sides trimmed; other lines
    /// are skipped.
    pub fn parse(text: &str) -> Self {
        let entries = text
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                Some((key.trim().to_owned(), value.trim().to_owned()))
            })
            .collect();
        Self { entries }
    }

    /// The `pyvenv.cfg` in an environment folder.
    pub fn read(env_dir: &Path) -> Option<Self> {
        std::fs::read_to_string(env_dir.join("pyvenv.cfg"))
            .ok()
            .map(|text| Self::parse(&text))
    }

    /// The value of `key` (the first one when it repeats).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The Python version the environment was created with: `version_info`
    /// (uv, virtualenv) or `version` (the `venv` module), as Python prints
    /// it (`3.13.1`, `3.14.0rc1`).
    pub fn version(&self) -> Option<String> {
        self.get("version_info")
            .or_else(|| self.get("version"))
            .map(python_version)
            .filter(|v| !v.is_empty())
    }

    /// The environment was created by uv (it writes a `uv` key).
    pub fn is_uv(&self) -> bool {
        self.get("uv").is_some()
    }
}

/// A version as Python prints it: virtualenv writes `sys.version_info` in
/// full (`3.12.1.final.0`, `3.14.0.candidate.1`).
fn python_version(raw: &str) -> String {
    let parts: Vec<&str> = raw.split('.').collect();
    if parts.len() == 5 {
        let release = parts[..3].join(".");
        let serial = parts[4];
        return match parts[3] {
            "final" => release,
            "alpha" => format!("{release}a{serial}"),
            "beta" => format!("{release}b{serial}"),
            "candidate" => format!("{release}rc{serial}"),
            _ => raw.to_owned(),
        };
    }
    raw.to_owned()
}

/// How a folder holding an interpreter is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvLayout {
    /// A virtual environment (`pyvenv.cfg`).
    Venv,
    /// A conda environment (`conda-meta`).
    Conda,
    /// Neither: a Python installation in the folder.
    Installation,
}

/// A folder with a Python interpreter in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvDir {
    /// The folder (the venv or conda prefix).
    pub dir: PathBuf,
    /// Its interpreter (`bin/python`, `Scripts\python.exe`).
    pub python: PathBuf,
    pub layout: EnvLayout,
    /// The `pyvenv.cfg` of a virtual environment.
    pub cfg: Option<PyvenvCfg>,
    /// The Python version, when the files tell it.
    pub version: Option<String>,
}

impl EnvDir {
    /// A virtual environment created by uv.
    pub fn is_uv(&self) -> bool {
        self.cfg.as_ref().is_some_and(PyvenvCfg::is_uv)
    }
}

/// The environment in `dir`, if the folder holds an interpreter.
pub fn inspect(dir: &Path) -> Option<EnvDir> {
    let python = environment_python(dir)?;
    let cfg = PyvenvCfg::read(dir);
    let (layout, version) = if let Some(cfg) = &cfg {
        (EnvLayout::Venv, cfg.version())
    } else if dir.join("conda-meta").is_dir() {
        (EnvLayout::Conda, conda_python_version(dir))
    } else {
        (EnvLayout::Installation, None)
    };
    Some(EnvDir {
        dir: dir.to_path_buf(),
        python,
        layout,
        cfg,
        version,
    })
}

/// The environment an interpreter belongs to: its folder is the parent of
/// `bin` (`Scripts` on Windows); a file elsewhere is not in one.
pub fn inspect_python(python: &Path) -> Option<EnvDir> {
    let bin = python.parent()?;
    let name = bin.file_name()?.to_str()?;
    if !matches!(name, "bin" | "Scripts") && bin.join("conda-meta").is_dir() {
        // A conda prefix on Windows keeps `python.exe` at its top.
        return inspect(bin).filter(|e| e.python == python);
    }
    let env = inspect(bin.parent()?)?;
    (env.python == python).then_some(env)
}

/// What is at the location chosen for a new environment
/// (`FileSystem.validateVenv` of the reference IDE).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// Nothing there yet: the environment can be created.
    Free,
    /// The path is relative.
    NotAbsolute,
    /// A folder with an interpreter in it (an environment already).
    HasPython(EnvDir),
    /// Something else is there: a folder without an interpreter, or a
    /// file.
    Exists,
}

/// Checks the location of a new environment. Blocking file system access
/// (no Python is started).
pub fn check_location(path: &Path) -> Location {
    if !path.is_absolute() {
        return Location::NotAbsolute;
    }
    if std::fs::symlink_metadata(path).is_err() {
        return Location::Free;
    }
    match inspect(path) {
        Some(env) => Location::HasPython(env),
        None => Location::Exists,
    }
}

/// The Python version of a conda environment from the record of its
/// `python` package (`conda-meta/python-3.12.1-h1234_0.json`).
fn conda_python_version(dir: &Path) -> Option<String> {
    std::fs::read_dir(dir.join("conda-meta"))
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| {
            let name = entry.file_name();
            let rest = name.to_str()?.strip_prefix("python-")?;
            let rest = rest.strip_suffix(".json")?;
            let (version, _build) = rest.split_once('-')?;
            version
                .starts_with(|c: char| c.is_ascii_digit())
                .then(|| version.to_owned())
        })
}

/// The environments of a project, as the reference IDE finds them when a
/// folder opens (`VirtualEnvReader.findVenvsInDir`): `UV_PROJECT_ENVIRONMENT`
/// (relative to the project) when it holds one, then every folder directly
/// in the project root that holds an interpreter, by name, then the project
/// environment of a parent folder ([`find_project_venv`]). Each folder is
/// listed once.
pub fn project_environments(root: &Path, uv_project_environment: Option<&Path>) -> Vec<EnvDir> {
    let mut out: Vec<EnvDir> = Vec::new();
    let mut seen = Vec::new();
    let mut add = |env: EnvDir, out: &mut Vec<EnvDir>| {
        let key = canonical_key(&env.dir);
        if !seen.contains(&key) {
            seen.push(key);
            out.push(env);
        }
    };
    if let Some(env) = uv_project_environment {
        let dir = if env.is_absolute() {
            env.to_path_buf()
        } else {
            root.join(env)
        };
        if let Some(env) = inspect(&dir) {
            add(env, &mut out);
        }
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(root)
        .map(|dir| {
            dir.filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    children.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    for child in children {
        if let Some(env) = inspect(&child) {
            add(env, &mut out);
        }
    }
    if let Some(dir) = find_project_venv(root, uv_project_environment)
        && let Some(env) = inspect(&dir)
    {
        add(env, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::venv_python;

    fn venv(dir: &Path, cfg: &str) -> PathBuf {
        let python = venv_python(dir);
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        std::fs::write(dir.join("pyvenv.cfg"), cfg).unwrap();
        python
    }

    #[test]
    fn reads_pyvenv_cfg_as_each_tool_writes_it() {
        // uv
        let cfg = PyvenvCfg::parse(
            "home = /opt/py/bin\nimplementation = CPython\nuv = 0.9.2\nversion_info = 3.13.14\ninclude-system-site-packages = false\nprompt = proj\n",
        );
        assert!(cfg.is_uv());
        assert_eq!(cfg.version().as_deref(), Some("3.13.14"));
        assert_eq!(cfg.get("prompt"), Some("proj"));
        // python -m venv
        let cfg = PyvenvCfg::parse(
            "home = /usr/bin\ninclude-system-site-packages = false\nversion = 3.12.13\nexecutable = /usr/bin/python3.12\ncommand = /usr/bin/python3 -m venv /p/.venv\n",
        );
        assert!(!cfg.is_uv());
        assert_eq!(cfg.version().as_deref(), Some("3.12.13"));
        assert_eq!(
            cfg.get("command"),
            Some("/usr/bin/python3 -m venv /p/.venv")
        );
        // virtualenv
        let cfg = PyvenvCfg::parse("version_info = 3.12.1.final.0\nvirtualenv = 20.25.0\n");
        assert_eq!(cfg.version().as_deref(), Some("3.12.1"));
        let cfg = PyvenvCfg::parse("version_info = 3.14.0.candidate.1\n");
        assert_eq!(cfg.version().as_deref(), Some("3.14.0rc1"));
        assert_eq!(PyvenvCfg::parse("home = /x\n").version(), None);
    }

    #[test]
    fn finds_the_environments_of_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(project_environments(root, None).is_empty());

        let uv = venv(&root.join(".venv"), "uv = 0.9.2\nversion_info = 3.13.14\n");
        let plain = venv(&root.join("venv"), "version = 3.12.13\n");
        // A conda prefix and a folder with an interpreter but no markers.
        let conda = root.join("env");
        std::fs::create_dir_all(conda.join("conda-meta")).unwrap();
        std::fs::write(conda.join("conda-meta/python-3.11.9-h123_0.json"), "{}").unwrap();
        std::fs::write(conda.join("conda-meta/pip-24.0-py_0.json"), "{}").unwrap();
        let conda_python = venv_python(&conda);
        std::fs::create_dir_all(conda_python.parent().unwrap()).unwrap();
        std::fs::write(&conda_python, "").unwrap();
        // Not environments.
        std::fs::create_dir_all(root.join("src/pkg")).unwrap();
        std::fs::write(root.join("main.py"), "").unwrap();

        let found = project_environments(root, None);
        let dirs: Vec<_> = found.iter().map(|e| e.dir.clone()).collect();
        assert_eq!(
            dirs,
            [root.join(".venv"), root.join("env"), root.join("venv")]
        );
        assert_eq!(found[0].python, uv);
        assert!(found[0].is_uv());
        assert_eq!(found[0].layout, EnvLayout::Venv);
        assert_eq!(found[0].version.as_deref(), Some("3.13.14"));
        assert_eq!(found[1].layout, EnvLayout::Conda);
        assert_eq!(found[1].version.as_deref(), Some("3.11.9"));
        assert_eq!(found[2].python, plain);
        assert!(!found[2].is_uv());
        assert_eq!(found[2].version.as_deref(), Some("3.12.13"));

        // UV_PROJECT_ENVIRONMENT comes first; a parent's `.venv` last.
        let custom = venv(&root.join("build/env"), "uv = 1\nversion_info = 3.14.0\n");
        let found = project_environments(root, Some(Path::new("build/env")));
        assert_eq!(found[0].python, custom);
        assert_eq!(found.len(), 4);
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let found = project_environments(&sub, None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].dir, root.join(".venv"));

        assert_eq!(inspect_python(&uv).map(|e| e.dir), Some(root.join(".venv")));
        assert_eq!(inspect_python(&root.join("main.py")), None);
    }

    #[test]
    fn checks_the_location_of_a_new_environment() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(check_location(Path::new(".venv")), Location::NotAbsolute);
        assert_eq!(check_location(&root.join("new/.venv")), Location::Free);
        // An environment with the version its `pyvenv.cfg` tells.
        venv(&root.join(".venv"), "uv = 0.9.2\nversion_info = 3.13.14\n");
        match check_location(&root.join(".venv")) {
            Location::HasPython(env) => {
                assert_eq!(env.version.as_deref(), Some("3.13.14"));
                assert!(env.is_uv());
            }
            other => panic!("{other:?}"),
        }
        // A folder without an interpreter (empty too), and a file.
        std::fs::create_dir_all(root.join("src")).unwrap();
        assert_eq!(check_location(&root.join("src")), Location::Exists);
        std::fs::write(root.join("main.py"), "").unwrap();
        assert_eq!(check_location(&root.join("main.py")), Location::Exists);
    }
}
