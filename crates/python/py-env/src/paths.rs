use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Directories searched for a tool in addition to `PATH`. A GUI application
/// started from the Dock or the Start menu does not inherit the shell's
/// `PATH`, and the installers of uv, pipx and conda put their binaries in a
/// few well-known places.
pub fn extra_tool_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".cargo").join("bin"));
        for conda in [
            "miniconda3",
            "anaconda3",
            "miniforge3",
            "mambaforge",
            "micromamba",
        ] {
            dirs.push(home.join(conda).join("bin"));
            #[cfg(windows)]
            {
                dirs.push(home.join(conda).join("Scripts"));
                dirs.push(home.join(conda).join("condabin"));
            }
        }
        #[cfg(windows)]
        {
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                dirs.push(PathBuf::from(local).join("Programs").join("uv"));
            }
        }
    }
    #[cfg(unix)]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/opt/conda/bin"));
    }
    dirs
}

/// Find an executable by name in `PATH`, then in [`extra_tool_dirs`].
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs = std::env::split_paths(&path).chain(extra_tool_dirs());
    find_in_dirs(name, dirs)
}

/// Find an executable by name in `PATH` only.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    find_in_dirs(name, std::env::split_paths(&path))
}

pub(crate) fn find_in_dirs(name: &str, dirs: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    let candidates = executable_names(name);
    for dir in dirs {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for c in &candidates {
            let p = dir.join(c);
            if is_executable(&p) {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(windows)]
fn executable_names(name: &str) -> Vec<OsString> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let mut out = vec![OsString::from(name)];
    if Path::new(name).extension().is_none() {
        out.extend(
            exts.split(';')
                .filter(|e| !e.is_empty())
                .map(|e| OsString::from(format!("{name}{}", e.to_ascii_lowercase()))),
        );
    }
    out
}

#[cfg(not(windows))]
fn executable_names(name: &str) -> Vec<OsString> {
    vec![OsString::from(name)]
}

/// A file that may be run: on Unix one with an execute bit.
#[cfg(unix)]
pub fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// A file that may be run: on Unix one with an execute bit.
#[cfg(not(unix))]
pub fn is_executable(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

/// A tool installed next to an interpreter: the executable `name` in the
/// interpreter's folder (`<env>/bin/ruff`, `<env>\Scripts\ruff.exe`) or, on
/// Windows, in the `Scripts` folder beside a base or conda interpreter
/// (`C:\Python313\Scripts\ruff.exe`), as the reference's "Sdk" lookup
/// does. `PATHEXT` extensions are tried on Windows. The interpreter's path
/// is used as given: a virtual environment's `python` link is not resolved
/// to the base interpreter.
pub fn env_tool(python: &Path, name: &str) -> Option<PathBuf> {
    let dir = python.parent()?;
    let mut dirs = vec![dir.to_path_buf()];
    if cfg!(windows) && !dir.ends_with("Scripts") {
        dirs.push(dir.join("Scripts"));
    }
    find_in_dirs(name, dirs)
}

/// The interpreter inside a virtual environment directory.
pub fn venv_python(root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        root.join("Scripts").join("python.exe")
    }
    #[cfg(not(windows))]
    {
        root.join("bin").join("python")
    }
}

/// The interpreter inside a conda environment directory.
pub fn conda_python(root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        root.join("python.exe")
    }
    #[cfg(not(windows))]
    {
        root.join("bin").join("python")
    }
}

/// The virtual environment root when `python` is the interpreter of a venv
/// (`<root>/bin/python` or `<root>\Scripts\python.exe` next to `pyvenv.cfg`).
pub fn venv_root_of(python: &Path) -> Option<PathBuf> {
    let root = python.parent()?.parent()?;
    root.join("pyvenv.cfg")
        .is_file()
        .then(|| root.to_path_buf())
}

/// An environment directory (venv or conda prefix) to its interpreter.
pub fn environment_python(dir: &Path) -> Option<PathBuf> {
    let venv = venv_python(dir);
    if venv.is_file() {
        return Some(venv);
    }
    let conda = conda_python(dir);
    if conda.is_file() {
        return Some(conda);
    }
    None
}

/// `name` is a `python`, `python3` or `python3.N` file name (with `.exe` on Windows).
pub(crate) fn is_python_name(file_name: &OsStr) -> bool {
    let Some(name) = file_name.to_str() else {
        return false;
    };
    let name = name.strip_suffix(".exe").unwrap_or(name);
    let Some(rest) = name.strip_prefix("python") else {
        return false;
    };
    if rest.is_empty() || rest == "3" {
        return true;
    }
    match rest.strip_prefix("3.") {
        Some(minor) => !minor.is_empty() && minor.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// Canonical form of a path for comparisons only (on Windows the result may
/// carry a `\\?\` prefix and must not be shown to users).
pub(crate) fn canonical_key(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_names() {
        for ok in [
            "python",
            "python3",
            "python3.12",
            "python3.9",
            "python.exe",
            "python3.13.exe",
        ] {
            assert!(is_python_name(OsStr::new(ok)), "{ok}");
        }
        for bad in [
            "python3-config",
            "python3.",
            "python2",
            "pythonw",
            "pip",
            "python3.12-config",
            "python3.x",
        ] {
            assert!(!is_python_name(OsStr::new(bad)), "{bad}");
        }
    }

    #[test]
    fn venv_layout_helpers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".venv");
        let py = venv_python(&root);
        std::fs::create_dir_all(py.parent().unwrap()).unwrap();
        std::fs::write(&py, "").unwrap();
        assert_eq!(environment_python(&root), Some(py.clone()));
        assert_eq!(venv_root_of(&py), None);
        std::fs::write(root.join("pyvenv.cfg"), "home = /x\n").unwrap();
        assert_eq!(venv_root_of(&py), Some(root));
    }

    #[cfg(unix)]
    #[test]
    fn finds_a_tool_next_to_the_interpreter() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let python = venv_python(&dir.path().join(".venv"));
        let bin = python.parent().unwrap();
        std::fs::create_dir_all(bin).unwrap();
        std::fs::write(&python, "").unwrap();
        assert_eq!(env_tool(&python, "ruff"), None);
        let ruff = bin.join("ruff");
        std::fs::write(&ruff, "#!/bin/sh\n").unwrap();
        // Not executable: not a tool.
        assert_eq!(env_tool(&python, "ruff"), None);
        std::fs::set_permissions(&ruff, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(env_tool(&python, "ruff"), Some(ruff.clone()));
        // The interpreter itself need not exist; its folder counts.
        assert_eq!(env_tool(&bin.join("python3.13"), "ruff"), Some(ruff));
        assert_eq!(env_tool(&python, "ty"), None);
        // A link to the base interpreter is not followed.
        let base = dir.path().join("base").join("bin");
        std::fs::create_dir_all(&base).unwrap();
        let base_python = base.join("python3");
        std::fs::write(&base_python, "").unwrap();
        let linked = dir.path().join("env").join("bin");
        std::fs::create_dir_all(&linked).unwrap();
        std::os::unix::fs::symlink(&base_python, linked.join("python")).unwrap();
        let ty = linked.join("ty");
        std::fs::write(&ty, "").unwrap();
        std::fs::set_permissions(&ty, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(env_tool(&linked.join("python"), "ty"), Some(ty));
        assert_eq!(env_tool(Path::new("python"), "ty"), None);
    }

    #[cfg(windows)]
    #[test]
    fn finds_a_tool_in_scripts_with_pathext() {
        let dir = tempfile::tempdir().unwrap();
        // A virtual environment: the interpreter is in `Scripts`.
        let python = venv_python(&dir.path().join(".venv"));
        let scripts = python.parent().unwrap();
        std::fs::create_dir_all(scripts).unwrap();
        std::fs::write(&python, "").unwrap();
        assert_eq!(env_tool(&python, "ruff"), None);
        let ruff = scripts.join("ruff.exe");
        std::fs::write(&ruff, "").unwrap();
        assert_eq!(env_tool(&python, "ruff"), Some(ruff));
        // A base or conda interpreter: tools are in `Scripts` beside it.
        let base = dir.path().join("Python313");
        std::fs::create_dir_all(base.join("Scripts")).unwrap();
        std::fs::write(base.join("python.exe"), "").unwrap();
        let ty = base.join("Scripts").join("ty.exe");
        std::fs::write(&ty, "").unwrap();
        assert_eq!(env_tool(&base.join("python.exe"), "ty"), Some(ty));
        // `.cmd` shims (npm-installed language servers) through PATHEXT.
        let shim = base.join("Scripts").join("basedpyright-langserver.cmd");
        std::fs::write(&shim, "").unwrap();
        assert_eq!(
            env_tool(&base.join("python.exe"), "basedpyright-langserver"),
            Some(shim)
        );
    }

    #[cfg(unix)]
    #[test]
    fn finds_tools_in_given_dirs_only_when_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("mytool");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        assert_eq!(find_in_dirs("mytool", [dir.path().to_path_buf()]), None);
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            find_in_dirs("mytool", [dir.path().to_path_buf()]),
            Some(tool)
        );
        assert_eq!(find_in_dirs("other", [dir.path().to_path_buf()]), None);
    }
}
