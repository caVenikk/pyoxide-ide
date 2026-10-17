//! The interpreters the Add Python Interpreter dialog offers (the
//! reference IDE's `PythonSelectableInterpreter` and its lists in
//! `PythonAddInterpreterModel`): the installations a new environment can
//! be based on (Base Python), every interpreter for Select existing /
//! Python in the reference's order (`sortForExistingEnvironment`), the uv
//! environments of the project (Select existing / uv), how a row is drawn
//! (`customizeForPythonInterpreter`), and the location suggested for a new
//! environment (`PySdkSettings.getPreferredVirtualEnvBasePath`).
//!
//! Finding them reads files and starts interpreters: [`Machine::find`]
//! runs on the interpreter runtime, never on the UI thread.

use std::path::{Path, PathBuf};
use std::time::Duration;

use py_env::venv::EnvLayout;
use py_env::{Interpreter, InterpreterKind, RunContext};

use super::table::{Entry, Flavor, Table};

/// The flavour icons of the rows (the Python file icon, uv's).
pub(crate) const PYTHON_ICON: ui::icons::IconName = ui::icons::IconName("python/file");
pub(crate) const UV_ICON: ui::icons::IconName = ui::icons::IconName("python/uv");
/// How transparent the icon of a found interpreter is drawn
/// (`IconLoader.getTransparentIcon`).
pub(crate) const FOUND_ICON_ALPHA: f32 = 0.5;

/// The download icon of a Python uv can install (`AllIcons.Actions.Download`).
pub(crate) const DOWNLOAD_ICON: ui::icons::IconName = ui::icons::IconName("interpreter/download");

/// Grey texts of a found interpreter (`sdk.rendering.detected.grey.text.*`)
/// and of one to download (`sdk.rendering.installable.grey.text`).
const SYSTEM: &str = "system";
const VIRTUAL_ENVIRONMENT: &str = "virtual environment";
pub(crate) const DOWNLOAD_AND_INSTALL: &str = "download and install";
/// The key of a Python to download among the interpreters' paths.
const DOWNLOAD_PREFIX: &str = "download:";

/// Where an interpreter of the lists comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Origin {
    /// In the application's list of configured interpreters
    /// (`ExistingSelectableInterpreter`).
    Configured {
        name: String,
        associated: Option<PathBuf>,
    },
    /// Found on the machine (`DetectedSelectableInterpreter`).
    Found,
    /// Chosen with the Browse button (`ManuallyAddedSelectableInterpreter`).
    Manual,
    /// A Python uv can download and install (`InstallableSelectableInterpreter`):
    /// its `python` is a key ([`Candidate::installable`]), not a file.
    Installable,
}

/// An interpreter of the dialog's lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub python: PathBuf,
    /// As the interpreter or its files tell it: `3.13.14`, `3.14`.
    pub version: Option<String>,
    pub origin: Origin,
    /// An installation, not an environment: it can be the base of a new
    /// environment (`isBase`).
    pub base: bool,
    /// The tool that manages it, shown after its path (`PyToolUIInfo`).
    pub tool: Option<&'static str>,
}

impl Candidate {
    /// An interpreter found on the machine.
    pub fn found(python: impl Into<PathBuf>, version: Option<String>, base: bool) -> Self {
        Self {
            python: python.into(),
            version,
            origin: Origin::Found,
            base,
            tool: None,
        }
    }

    /// An interpreter `py_env` found and probed: installations are bases,
    /// uv's own interpreters and environments made by uv carry its name.
    pub fn of_interpreter(i: &Interpreter) -> Self {
        let base =
            !i.is_venv && !matches!(i.kind, InterpreterKind::Conda | InterpreterKind::Poetry);
        let uv_env =
            i.is_venv && py_env::venv::inspect_python(&i.path).is_some_and(|env| env.is_uv());
        Self {
            python: i.path.clone(),
            version: Some(i.version.clone()),
            origin: Origin::Found,
            base,
            tool: (i.kind == InterpreterKind::Uv || uv_env).then_some("uv"),
        }
    }

    /// A Python uv can download and install (`3.13.14`), keyed by its
    /// version (`download:3.13.14`).
    pub fn installable(version: &str) -> Self {
        Self {
            python: PathBuf::from(format!("{DOWNLOAD_PREFIX}{version}")),
            version: Some(version.to_owned()),
            origin: Origin::Installable,
            base: true,
            tool: Some("uv"),
        }
    }

    /// The version of a key of a Python to download (`download:3.13.14`).
    pub fn download_key(text: &str) -> Option<&str> {
        text.strip_prefix(DOWNLOAD_PREFIX)
    }

    /// The Python to download and install first, for one to download.
    pub fn install_version(&self) -> Option<&str> {
        match self.origin {
            Origin::Installable => self.version.as_deref(),
            _ => None,
        }
    }

    /// An interpreter of the list of configured ones.
    pub fn of_entry(entry: &Entry) -> Self {
        Self {
            python: entry.python.clone(),
            version: entry.version.clone(),
            origin: Origin::Configured {
                name: entry.name.clone(),
                associated: entry.associated_project.clone(),
            },
            base: entry.flavor == Flavor::System,
            tool: (entry.flavor == Flavor::Uv).then_some("uv"),
        }
    }

    /// `3.13` of the version (the language level).
    pub fn language_level(&self) -> Option<String> {
        let (major, minor) = py_env::interpreter::major_minor(self.version.as_deref()?)?;
        Some(format!("{major}.{minor}"))
    }

    /// Whether a new environment can be based on it (Python 3.8+); an
    /// unknown version is taken as supported.
    pub fn supports_new_environments(&self) -> bool {
        self.version
            .as_deref()
            .and_then(py_env::interpreter::major_minor)
            .is_none_or(|(major, minor)| {
                py_env::interpreter::supports_new_environments(major, minor)
            })
    }

    /// The row's text: the main part and the grey part after it
    /// (`customizeForPythonInterpreter`): `Python 3.14` and
    /// ` (~/path/python) system` for a found one, `Python 3.13.14` and
    /// ` ~/path/python` for a configured one, `Python 3.13.14` and
    /// ` download and install` for one to download.
    pub fn text(&self) -> (String, String) {
        if self.origin == Origin::Installable {
            let version = self.version.as_deref().unwrap_or_default();
            return (
                format!("Python {version}"),
                format!(" {DOWNLOAD_AND_INSTALL}"),
            );
        }
        let path = project::display_path(&self.python);
        match &self.origin {
            Origin::Configured { name, .. } => {
                let title = match &self.version {
                    Some(version) => format!("Python {version}"),
                    None => name.clone(),
                };
                (title, format!(" {path}"))
            }
            Origin::Found | Origin::Manual | Origin::Installable => {
                let level = self.language_level().unwrap_or_default();
                let tool = self.tool.unwrap_or(if self.base {
                    SYSTEM
                } else {
                    VIRTUAL_ENVIRONMENT
                });
                (format!("Python {level:<4}"), format!(" ({path}) {tool}"))
            }
        }
    }

    /// The row's icon and its transparency.
    pub fn icon(&self) -> (ui::icons::IconName, f32) {
        match self.origin {
            Origin::Configured { .. } => (PYTHON_ICON, 1.0),
            Origin::Installable => (DOWNLOAD_ICON, 1.0),
            Origin::Found | Origin::Manual => match self.tool {
                Some("uv") => (UV_ICON, FOUND_ICON_ALPHA),
                _ => (PYTHON_ICON, FOUND_ICON_ALPHA),
            },
        }
    }

    /// Newest version first (`PythonInfoComparator`: the first Python is
    /// the highest), then without a tool before with one (`UiComparator`,
    /// nulls first).
    fn order_key(&self) -> (std::cmp::Reverse<(u32, u32, u32)>, bool) {
        (
            std::cmp::Reverse(version_triple(self.version.as_deref())),
            self.tool.is_some(),
        )
    }
}

fn version_triple(version: Option<&str>) -> (u32, u32, u32) {
    let Some(version) = version else {
        return (0, 0, 0);
    };
    let mut parts = version.trim().split('.').map(|p| {
        p.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u32>()
            .unwrap_or(0)
    });
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

fn sort(list: &mut [Candidate]) {
    list.sort_by_key(Candidate::order_key);
}

/// Keeps the first interpreter of each path.
fn dedup_by_path(list: Vec<Candidate>) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::with_capacity(list.len());
    for c in list {
        if !out
            .iter()
            .any(|o| super::table::same_path(&o.python, &c.python))
        {
            out.push(c);
        }
    }
    out
}

/// The Base Python list (`baseInterpreters`): the ones chosen with Browse
/// that are installations, then the installations found, newest first,
/// then the Pythons uv can download (`installable`, newest first) of the
/// versions none of the installations found has.
pub(crate) fn base_list(
    found: &[Candidate],
    manual: &[Candidate],
    installable: &[Candidate],
) -> Vec<Candidate> {
    let mut detected: Vec<Candidate> = found.iter().filter(|c| c.base).cloned().collect();
    sort(&mut detected);
    let levels: Vec<String> = detected.iter().filter_map(|c| c.language_level()).collect();
    let missing = installable
        .iter()
        .filter(|c| c.language_level().is_none_or(|l| !levels.contains(&l)))
        .cloned();
    let manual = manual.iter().filter(|c| c.base).cloned();
    dedup_by_path(manual.chain(detected).chain(missing).collect())
}

/// The Pythons uv can download for a new environment, newest first
/// (`getInstallableInterpreters`).
pub(crate) fn installable(downloads: &[py_env::uv::UvPython]) -> Vec<Candidate> {
    py_env::uv::downloadable_releases(downloads, py_env::interpreter::OLDEST_FOR_NEW_ENVIRONMENTS)
        .iter()
        .map(|v| Candidate::installable(v))
        .collect()
}

/// The base selected when the dialog opens: the one the last environment
/// was made from when it is in the list (`preferredVirtualEnvBaseSdk`),
/// else the first.
pub(crate) fn default_base(list: &[Candidate], preferred: Option<&Path>) -> Option<usize> {
    preferred
        .and_then(|p| {
            list.iter()
                .position(|c| super::table::same_path(&c.python, p))
        })
        .or((!list.is_empty()).then_some(0))
}

/// The groups of Select existing / Python, in their order (PY-78035).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    /// Associated with the project, or found inside it.
    Project,
    /// Configured ones offered to every project.
    Shared,
    /// Environments in the user's environments folder.
    UserVenvs,
    Other,
}

/// The Select existing list (`allInterpreters`, `sortForExistingEnvironment`):
/// the ones chosen with Browse, the configured ones and the ones found,
/// each path once; the configured ones of other projects are left out.
/// Grouped (the project's, shared configured ones, environments in
/// `venv_root`, the others), newest first within a group.
pub(crate) fn existing_list(
    table: &Table,
    found: &[Candidate],
    manual: &[Candidate],
    root: &Path,
    venv_root: Option<&Path>,
) -> Vec<Candidate> {
    let configured = table.entries.iter().map(Candidate::of_entry);
    let all = dedup_by_path(
        manual
            .iter()
            .cloned()
            .chain(configured)
            .chain(found.iter().cloned())
            .collect(),
    );
    let mut grouped: Vec<(Group, Candidate)> = all
        .into_iter()
        .filter_map(|c| {
            let group = match &c.origin {
                Origin::Configured {
                    associated: Some(project),
                    ..
                } => {
                    if super::table::same_path(project, root) {
                        Group::Project
                    } else {
                        return None;
                    }
                }
                Origin::Configured {
                    associated: None, ..
                } => Group::Shared,
                Origin::Found if c.python.starts_with(root) => Group::Project,
                _ if venv_root.is_some_and(|v| c.python.starts_with(v)) => Group::UserVenvs,
                _ => Group::Other,
            };
            Some((group, c))
        })
        .collect();
    grouped.sort_by(|(ga, a), (gb, b)| ga.cmp(gb).then_with(|| a.order_key().cmp(&b.order_key())));
    grouped.into_iter().map(|(_, c)| c).collect()
}

/// The uv environments of the project for Select existing / uv
/// (`detectEnvironments`): its direct subfolders holding an interpreter
/// whose `pyvenv.cfg` was written by uv. Blocking file system access.
pub(crate) fn uv_environments(root: &Path) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs.into_iter()
        .filter_map(|dir| py_env::venv::inspect(&dir))
        .filter(|env| env.layout == EnvLayout::Venv && env.is_uv())
        .map(|env| Candidate {
            tool: Some("uv"),
            ..Candidate::found(env.python, env.version, false)
        })
        .collect()
}

/// The virtual environments in the user's environments folder
/// (`VirtualEnvReader.getVEnvRootDir`: `WORKON_HOME`, else
/// `~/.virtualenvs`), by their `pyvenv.cfg`. Blocking.
pub(crate) fn user_environments(venv_root: &Path) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(venv_root) else {
        return Vec::new();
    };
    let mut out: Vec<Candidate> = entries
        .filter_map(Result::ok)
        .filter_map(|e| py_env::venv::inspect(&e.path()))
        .filter(|env| env.layout == EnvLayout::Venv)
        .map(|env| {
            let uv = env.is_uv();
            Candidate {
                tool: uv.then_some("uv"),
                ..Candidate::found(env.python, env.version, false)
            }
        })
        .collect();
    out.sort_by(|a, b| a.python.cmp(&b.python));
    out
}

/// The user's environments folder.
pub(crate) fn venv_root(home: Option<&Path>) -> Option<PathBuf> {
    std::env::var_os("WORKON_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".virtualenvs")))
}

/// The location suggested for a new environment of the project at `root`
/// (`getPreferredVirtualEnvBasePath`): `<project>/.venv` until another
/// location was kept; a kept one inside the project as it is, one outside
/// it with the project's folder name added.
pub(crate) fn suggested_location(root: &Path, kept: Option<&Path>) -> PathBuf {
    match kept {
        None => root.join(py_env::project::VENV_DIR_NAMES[0]),
        Some(kept) if kept.is_relative() => root.join(kept),
        Some(kept) if kept.starts_with(root) => kept.to_owned(),
        Some(kept) => match root.file_name() {
            Some(name) => kept.join(name),
            None => kept.to_owned(),
        },
    }
}

/// The project uses uv (a `uv.lock`, or `[tool.uv]` in `pyproject.toml`).
pub(crate) fn is_uv_project(root: &Path) -> bool {
    if root.join("uv.lock").is_file() {
        return true;
    }
    std::fs::read_to_string(root.join("pyproject.toml"))
        .ok()
        .and_then(|text| py_env::project::parse_pyproject(&text).ok())
        .is_some_and(|p| p.uv.is_some())
}

/// The machine the dialog looks at: where interpreters are searched, the
/// tools found, the user's home. Read from the process (blocking: it
/// looks for the tools on disk); tests give their own.
#[derive(Debug, Clone, Default)]
pub(crate) struct Machine {
    pub search: py_env::SearchEnv,
    pub tools: py_env::Tools,
    /// Overrides the user's environments folder (tests).
    pub venv_root: Option<PathBuf>,
}

impl Machine {
    pub fn from_process() -> Self {
        let search = py_env::SearchEnv::from_process();
        let venv_root = venv_root(search.home.as_deref());
        Self {
            search,
            tools: py_env::Tools::locate(),
            venv_root,
        }
    }

    /// The interpreters found for the project at `root`: `py_env`'s
    /// discovery (probed) and the environments of the user's folder.
    pub async fn find(&self, root: &Path, ctx: &RunContext) -> Vec<Candidate> {
        let mut options = py_env::DiscoveryOptions::new(self.search.clone(), self.tools.clone());
        options.project = Some(root.to_owned());
        options.probe_timeout = Duration::from_secs(10);
        let report = py_env::discover(&options, ctx, &|_| {}).await;
        let mut out: Vec<Candidate> = report
            .map(|r| {
                r.interpreters
                    .iter()
                    .map(Candidate::of_interpreter)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(root) = &self.venv_root {
            out.extend(user_environments(root));
        }
        dedup_by_path(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn venv(dir: &Path, cfg: &str) -> PathBuf {
        let python = py_env::paths::venv_python(dir);
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        std::fs::write(dir.join("pyvenv.cfg"), cfg).unwrap();
        python
    }

    fn found(path: &str, version: &str, base: bool) -> Candidate {
        Candidate::found(path, Some(version.into()), base)
    }

    fn paths(list: &[Candidate]) -> Vec<String> {
        list.iter()
            .map(|c| c.python.to_string_lossy().into_owned())
            .collect()
    }

    /// Rows as `customizeForPythonInterpreter` draws them.
    #[test]
    fn rows_are_drawn_as_the_reference_does() {
        let system = found("/usr/bin/python3", "3.9.6", true);
        assert_eq!(
            system.text(),
            (
                "Python 3.9 ".to_owned(),
                " (/usr/bin/python3) system".to_owned()
            )
        );
        assert_eq!(system.icon(), (PYTHON_ICON, FOUND_ICON_ALPHA));
        let uv = Candidate {
            tool: Some("uv"),
            ..found("/opt/uv/cpython-3.14/bin/python3.14", "3.14.0", true)
        };
        assert_eq!(uv.text().0, "Python 3.14");
        assert!(uv.text().1.ends_with(") uv"), "{:?}", uv.text());
        assert_eq!(uv.icon().0, UV_ICON);
        let env = found("/w/.venv/bin/python", "3.13.14", false);
        assert_eq!(env.text().1, " (/w/.venv/bin/python) virtual environment");
        let mut entry = Entry::new("~/w/.venv", "/w/.venv/bin/python", Flavor::Venv);
        entry.version = Some("3.13.14".into());
        let configured = Candidate::of_entry(&entry);
        assert_eq!(
            configured.text(),
            (
                "Python 3.13.14".to_owned(),
                " /w/.venv/bin/python".to_owned()
            )
        );
        assert_eq!(configured.icon(), (PYTHON_ICON, 1.0));
        assert_eq!(configured.language_level().as_deref(), Some("3.13"));
    }

    #[test]
    fn bases_are_installations_newest_first_the_browsed_ones_on_top() {
        let detected = [
            found("/a/python3.12", "3.12.1", true),
            found("/w/.venv/bin/python", "3.13.1", false),
            found("/b/python3.14", "3.14.0", true),
            Candidate {
                tool: Some("uv"),
                ..found("/uv/python3.14", "3.14.0", true)
            },
        ];
        let manual = [
            Candidate {
                origin: Origin::Manual,
                ..found("/m/python3.10", "3.10.4", true)
            },
            Candidate {
                origin: Origin::Manual,
                ..found("/m/env/bin/python", "3.10.4", false)
            },
        ];
        // uv's downloads of the versions not found (3.14 and 3.12 are).
        let downloads: Vec<_> = ["3.14.6", "3.13.14", "3.12.13", "3.10.20"]
            .iter()
            .map(|v| Candidate::installable(v))
            .collect();
        let list = base_list(&detected, &manual, &downloads);
        assert_eq!(
            paths(&list),
            [
                "/m/python3.10",
                "/b/python3.14",
                "/uv/python3.14",
                "/a/python3.12",
                "download:3.13.14",
                "download:3.10.20",
            ]
        );
        // `customizeForPythonInterpreter` of an installable one.
        assert_eq!(
            list[4].text(),
            (
                "Python 3.13.14".to_owned(),
                " download and install".to_owned()
            )
        );
        assert_eq!(list[4].icon(), (DOWNLOAD_ICON, 1.0));
        assert_eq!(list[4].install_version(), Some("3.13.14"));
        assert_eq!(Candidate::download_key("download:3.13.14"), Some("3.13.14"));
        assert_eq!(list[0].install_version(), None);
        assert_eq!(default_base(&list, None), Some(0));
        assert_eq!(
            default_base(&list, Some(Path::new("/a/python3.12"))),
            Some(3)
        );
        assert_eq!(default_base(&list, Some(Path::new("/gone"))), Some(0));
        assert_eq!(default_base(&[], None), None);
        // Python 3.8 is the oldest base of a new environment.
        assert!(found("/p", "3.8.0", true).supports_new_environments());
        assert!(!found("/p", "3.7.9", true).supports_new_environments());
        assert!(Candidate::found("/p", None, true).supports_new_environments());
    }

    /// `sortForExistingEnvironment`: the project's, the shared configured
    /// ones, the user's environments, the others; other projects' configured
    /// ones are hidden; a configured interpreter wins over the same one found.
    #[test]
    fn the_existing_list_is_grouped_as_the_reference_does() {
        let root = Path::new("/w/demo");
        let mut table = Table::default();
        let mut entry = |name: &str, python: &str, flavor, project: Option<&str>| {
            let mut e = Entry::new(name, python, flavor);
            e.version = Some("3.11.0".into());
            e.associated_project = project.map(PathBuf::from);
            table.entries.push(e);
        };
        entry(
            "mine",
            "/envs/mine/bin/python",
            Flavor::Venv,
            Some("/w/demo"),
        );
        entry(
            "other",
            "/w/other/.venv/bin/python",
            Flavor::Venv,
            Some("/w/other"),
        );
        entry(
            "Python 3.11",
            "/usr/local/bin/python3.11",
            Flavor::System,
            None,
        );
        let found = [
            found("/usr/bin/python3", "3.9.6", true),
            found("/w/demo/.venv/bin/python", "3.13.1", false),
            found("/home/u/.virtualenvs/tools/bin/python", "3.12.0", false),
            found("/usr/local/bin/python3.11", "3.11.0", true),
            found("/w/other/.venv/bin/python", "3.11.0", false),
        ];
        let list = existing_list(
            &table,
            &found,
            &[],
            root,
            Some(Path::new("/home/u/.virtualenvs")),
        );
        assert_eq!(
            paths(&list),
            [
                "/w/demo/.venv/bin/python",
                "/envs/mine/bin/python",
                "/usr/local/bin/python3.11",
                "/home/u/.virtualenvs/tools/bin/python",
                "/usr/bin/python3",
            ]
        );
        // The configured one, not the one found at the same path.
        assert!(matches!(list[2].origin, Origin::Configured { .. }));
    }

    #[test]
    fn uv_environments_of_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let uv = venv(&root.join(".venv"), "uv = 0.9.2\nversion_info = 3.13.1\n");
        venv(&root.join("venv"), "version = 3.12.13\n");
        venv(
            &root.join("deep/env"),
            "uv = 0.9.2\nversion_info = 3.12.1\n",
        );
        std::fs::write(root.join("main.py"), "").unwrap();
        let list = uv_environments(root);
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0].python, uv);
        assert_eq!(list[0].tool, Some("uv"));
        assert_eq!(list[0].text().0, "Python 3.13");
        let user = user_environments(root);
        assert_eq!(user.len(), 2, "{user:?}");
        assert!(uv_environments(&root.join("missing")).is_empty());
    }

    #[test]
    fn the_suggested_location() {
        let root = Path::new("/w/demo");
        assert_eq!(suggested_location(root, None), root.join(".venv"));
        assert_eq!(
            suggested_location(root, Some(Path::new("envs/main"))),
            root.join("envs/main")
        );
        assert_eq!(
            suggested_location(root, Some(Path::new("/w/demo/env"))),
            PathBuf::from("/w/demo/env")
        );
        assert_eq!(
            suggested_location(root, Some(Path::new("/home/u/.venvs"))),
            PathBuf::from("/home/u/.venvs/demo")
        );
    }

    #[test]
    fn uv_projects() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(!is_uv_project(root));
        std::fs::write(root.join("pyproject.toml"), "[project]\nname = \"a\"\n").unwrap();
        assert!(!is_uv_project(root));
        std::fs::write(
            root.join("pyproject.toml"),
            "[project]\nname = \"a\"\n[tool.uv]\ndev-dependencies = []\n",
        )
        .unwrap();
        assert!(is_uv_project(root));
        std::fs::write(root.join("pyproject.toml"), "[project]\nname = \"a\"\n").unwrap();
        std::fs::write(root.join("uv.lock"), "version = 1\n").unwrap();
        assert!(is_uv_project(root));
    }
}
