use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{EnvError, ErrorKind, Result};
use crate::paths::canonical_key;
use crate::process::{RunContext, Spec, run};

/// Where an interpreter comes from, which decides how the IDE treats it
/// (a venv can be deleted and recreated, a system Python must not be
/// modified, a uv-managed Python can be updated with `uv python install`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterpreterKind {
    System,
    /// Installed by `uv python install` (lives in uv's Python directory).
    Uv,
    /// A virtual environment (venv, virtualenv, `uv venv`).
    Venv,
    /// A virtual environment managed by Poetry.
    Poetry,
    Conda,
    Pyenv,
}

impl InterpreterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InterpreterKind::System => "system",
            InterpreterKind::Uv => "uv",
            InterpreterKind::Venv => "venv",
            InterpreterKind::Poetry => "poetry",
            InterpreterKind::Conda => "conda",
            InterpreterKind::Pyenv => "pyenv",
        }
    }
}

/// The oldest Python that new environments are created with: the
/// reference IDE refuses older base interpreters
/// (`sdk.create.custom.venv.unsupported.base.version`).
pub const OLDEST_FOR_NEW_ENVIRONMENTS: (u32, u32) = (3, 8);

/// Whether an interpreter of this version can be the base of a new
/// environment ([`OLDEST_FOR_NEW_ENVIRONMENTS`]).
pub fn supports_new_environments(major: u32, minor: u32) -> bool {
    (major, minor) >= OLDEST_FOR_NEW_ENVIRONMENTS
}

/// `(3, 13)` of a version as Python prints it (`3.13.14`, `3.14.0rc1`,
/// `3.14`).
pub fn major_minor(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor: String = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((major, minor.parse().ok()?))
}

/// A probed Python interpreter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interpreter {
    /// The executable to run (for a venv, the one inside the venv).
    pub path: PathBuf,
    /// `3.12.13`, `3.15.0b3`.
    pub version: String,
    pub major: u32,
    pub minor: u32,
    pub micro: u32,
    /// `cpython`, `pypy`, `graalpy`.
    pub implementation: String,
    pub kind: InterpreterKind,
    pub is_venv: bool,
    /// `sys.prefix`: the venv or environment directory for those, the
    /// installation for others.
    pub prefix: PathBuf,
    /// `sys.base_prefix`: the installation a venv was created from.
    pub base_prefix: PathBuf,
    /// The virtual environment directory when `is_venv`.
    pub venv_root: Option<PathBuf>,
    /// `arm64`, `x86_64`, `AMD64`.
    pub arch: Option<String>,
    /// A build without the GIL.
    pub free_threaded: bool,
    /// How it was found: `uv`, `PATH`, `.venv`, `VIRTUAL_ENV`, `poetry`, `conda`, `pyenv`, `py launcher`.
    pub source: String,
}

/// What the probe script prints.
#[derive(Debug, Deserialize)]
pub(crate) struct ProbeOutput {
    version: (u32, u32, u32, String, u32),
    implementation: String,
    prefix: PathBuf,
    base_prefix: PathBuf,
    #[serde(default)]
    machine: Option<String>,
    #[serde(default)]
    free_threaded: bool,
}

/// Runs in the interpreter being inspected. Isolated mode (`-I`) and no site
/// import (`-S`) keep user and project files out of the picture and the
/// start fast; `sys.prefix` is correct without `site`.
pub(crate) const PROBE_SCRIPT: &str = "import sys, json, platform, sysconfig\n\
print(json.dumps({\
'version': [sys.version_info[0], sys.version_info[1], sys.version_info[2], sys.version_info[3], sys.version_info[4]],\
'implementation': sys.implementation.name,\
'prefix': sys.prefix,\
'base_prefix': sys.base_prefix,\
'machine': platform.machine(),\
'free_threaded': bool(sysconfig.get_config_var('Py_GIL_DISABLED'))}))";

pub(crate) fn parse_probe(stdout: &str) -> Result<ProbeOutput> {
    // Anything the interpreter printed before our line (sitecustomize noise) is ignored.
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .ok_or_else(|| EnvError::parse("the interpreter printed no description of itself"))?;
    serde_json::from_str(line)
        .map_err(|e| EnvError::parse(format!("unreadable interpreter description: {e}")))
}

fn version_string(p: &ProbeOutput) -> String {
    let (major, minor, micro, level, serial) = (
        &p.version.0,
        &p.version.1,
        &p.version.2,
        &p.version.3,
        p.version.4,
    );
    let suffix = match level.as_str() {
        "alpha" => format!("a{serial}"),
        "beta" => format!("b{serial}"),
        "candidate" => format!("rc{serial}"),
        _ => String::new(),
    };
    format!("{major}.{minor}.{micro}{suffix}")
}

/// Where an interpreter was found, before the probe says what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hint {
    Path,
    Uv,
    ProjectVenv,
    Poetry,
    Conda,
    Pyenv,
}

pub(crate) struct Locations {
    /// uv's Python install directory, canonicalized.
    pub uv_dir: Option<PathBuf>,
}

pub(crate) fn classify(
    hint: Hint,
    is_venv: bool,
    path: &Path,
    prefix: &Path,
    loc: &Locations,
) -> InterpreterKind {
    if is_venv {
        return if hint == Hint::Poetry {
            InterpreterKind::Poetry
        } else {
            InterpreterKind::Venv
        };
    }
    if hint == Hint::Conda || prefix.join("conda-meta").is_dir() {
        return InterpreterKind::Conda;
    }
    let real = canonical_key(path);
    if let Some(uv_dir) = &loc.uv_dir
        && real.starts_with(uv_dir)
    {
        return InterpreterKind::Uv;
    }
    if hint == Hint::Pyenv || real.components().any(|c| c.as_os_str() == ".pyenv") {
        return InterpreterKind::Pyenv;
    }
    InterpreterKind::System
}

/// Run `python` once and describe it. Fails with a short reason when the
/// executable is not a working Python (a dead symlink, a Windows Store stub,
/// Python 2).
pub async fn probe(
    python: &Path,
    hint_source: &str,
    timeout: Duration,
    ctx: &RunContext,
) -> Result<Interpreter> {
    probe_with(
        python,
        Hint::Path,
        hint_source,
        &Locations { uv_dir: None },
        timeout,
        ctx,
    )
    .await
}

pub(crate) async fn probe_with(
    python: &Path,
    hint: Hint,
    source: &str,
    loc: &Locations,
    timeout: Duration,
    ctx: &RunContext,
) -> Result<Interpreter> {
    let spec = Spec::new(python).args(["-I", "-S", "-c", PROBE_SCRIPT]);
    let done = run("python", spec, ctx, timeout).await.map_err(|mut e| {
        if e.kind == ErrorKind::ToolMissing {
            e.message = format!("{} does not exist or is not executable", python.display());
        }
        e
    })?;
    if !done.success() {
        return Err(EnvError::from_failure("python", done.code, &done.stderr));
    }
    let mut out = parse_probe(&done.stdout)?;
    // `-S` leaves out `site`, which points `sys.prefix` at a virtual
    // environment: the environment's `pyvenv.cfg` tells instead.
    if out.prefix == out.base_prefix
        && let Some(env) = python
            .parent()
            .and_then(Path::parent)
            .and_then(crate::venv::inspect)
            .filter(|env| env.layout == crate::venv::EnvLayout::Venv)
    {
        out.prefix = env.dir;
    }
    let is_venv = out.prefix != out.base_prefix;
    let kind = classify(hint, is_venv, python, &out.prefix, loc);
    Ok(Interpreter {
        path: python.to_path_buf(),
        version: version_string(&out),
        major: out.version.0,
        minor: out.version.1,
        micro: out.version.2,
        implementation: out.implementation.clone(),
        kind,
        is_venv,
        venv_root: is_venv.then(|| out.prefix.clone()),
        prefix: out.prefix,
        base_prefix: out.base_prefix,
        arch: out.machine,
        free_threaded: out.free_threaded,
        source: source.to_owned(),
    })
}

/// Prints the folders an interpreter imports from (the reference's
/// `helpers/syspath.py`): `sys.path` without the current folder, then the
/// roots of packages installed in editable mode (PEP 660), which import
/// hooks find instead of `sys.path`.
pub(crate) const SYS_PATH_SCRIPT: &str = r#"import os, sys
cwd = os.getcwd()
for root in sys.path:
    if root and root != os.curdir and root != cwd:
        print(root)
try:
    import json
    from importlib.metadata import distributions
    from urllib.parse import urlsplit
    from urllib.request import url2pathname
except ImportError:
    distributions = list
for dist in distributions():
    url = None
    try:
        if dist.origin.dir_info.editable:
            url = dist.origin.url
    except AttributeError:
        try:
            data = json.loads(dist.read_text('direct_url.json') or '{}')
            if data.get('dir_info', {}).get('editable'):
                url = data.get('url')
        except Exception:
            pass
    if url:
        parts = urlsplit(url)
        if parts.scheme == 'file' and parts.netloc in ('', 'localhost'):
            print(url2pathname(parts.path))
"#;

/// The folders `python` imports from, in its order, each once: the
/// interpreter's paths (`Interpreter Paths` of the reference). Folders and
/// archives that do not exist are left out.
pub async fn sys_path(python: &Path, timeout: Duration, ctx: &RunContext) -> Result<Vec<PathBuf>> {
    let spec = Spec::new(python).args(["-c", SYS_PATH_SCRIPT]);
    let done = run("python", spec, ctx, timeout).await?;
    if !done.success() {
        return Err(EnvError::from_failure("python", done.code, &done.stderr));
    }
    Ok(parse_sys_path(&done.stdout)
        .into_iter()
        .filter(|p| p.exists())
        .collect())
}

/// The lines [`SYS_PATH_SCRIPT`] printed: paths, each once.
pub fn parse_sys_path(stdout: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for line in stdout.lines().map(str::trim_end).filter(|l| !l.is_empty()) {
        let path = PathBuf::from(line);
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sys_path_lines_are_paths_each_once() {
        let out = "/usr/lib/python313.zip\n/usr/lib/python3.13\n\n/work/.venv/lib/python3.13/site-packages\n/work/src\n/work/src\n";
        assert_eq!(
            parse_sys_path(out),
            [
                "/usr/lib/python313.zip",
                "/usr/lib/python3.13",
                "/work/.venv/lib/python3.13/site-packages",
                "/work/src",
            ]
            .map(PathBuf::from)
        );
        assert!(parse_sys_path("").is_empty());
    }

    const SAMPLE: &str = r#"{"version": [3, 12, 13, "final", 0], "implementation": "cpython", "prefix": "/work/.venv", "base_prefix": "/opt/py/3.12", "machine": "arm64", "free_threaded": false}"#;

    #[test]
    fn versions_for_new_environments() {
        assert_eq!(major_minor("3.13.14"), Some((3, 13)));
        assert_eq!(major_minor("3.14rc1"), Some((3, 14)));
        assert_eq!(major_minor("3.14"), Some((3, 14)));
        assert_eq!(major_minor("3"), None);
        assert_eq!(major_minor("x.y"), None);
        assert!(supports_new_environments(3, 8));
        assert!(supports_new_environments(3, 14));
        assert!(!supports_new_environments(3, 7));
        assert!(!supports_new_environments(2, 7));
    }

    #[test]
    fn parses_probe_output() {
        let p = parse_probe(SAMPLE).unwrap();
        assert_eq!(version_string(&p), "3.12.13");
        assert_eq!(p.implementation, "cpython");
        assert_ne!(p.prefix, p.base_prefix);
        assert_eq!(p.machine.as_deref(), Some("arm64"));
    }

    #[test]
    fn ignores_noise_before_the_json_line() {
        let noisy = format!("site customised\nwarning: something\n{SAMPLE}\n");
        assert!(parse_probe(&noisy).is_ok());
    }

    #[test]
    fn version_strings_cover_pre_releases() {
        let p = parse_probe(
            r#"{"version": [3, 15, 0, "beta", 3], "implementation": "cpython", "prefix": "/a", "base_prefix": "/a"}"#,
        )
        .unwrap();
        assert_eq!(version_string(&p), "3.15.0b3");
        let p = parse_probe(
            r#"{"version": [3, 14, 0, "candidate", 1], "implementation": "cpython", "prefix": "/a", "base_prefix": "/a"}"#,
        )
        .unwrap();
        assert_eq!(version_string(&p), "3.14.0rc1");
    }

    #[test]
    fn rejects_non_json_and_python2_output() {
        assert_eq!(parse_probe("").unwrap_err().kind, ErrorKind::Parse);
        assert_eq!(
            parse_probe("Python 2.7.18").unwrap_err().kind,
            ErrorKind::Parse
        );
        assert_eq!(
            parse_probe("{\"version\": 3}").unwrap_err().kind,
            ErrorKind::Parse
        );
    }

    /// A virtual environment's interpreter run with `-S` reports its
    /// base as `sys.prefix`; its `pyvenv.cfg` makes it a virtual
    /// environment again.
    #[cfg(unix)]
    #[tokio::test]
    async fn environments_are_known_by_their_pyvenv_cfg() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join("env");
        let python = env.join("bin/python");
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(
            &python,
            "#!/bin/sh\necho '{\"version\": [3, 13, 5, \"final\", 0], \"implementation\": \"cpython\", \"prefix\": \"/opt/py\", \"base_prefix\": \"/opt/py\"}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
        let ctx = RunContext::default();
        let base = probe(&python, "manual", Duration::from_secs(10), &ctx)
            .await
            .unwrap();
        assert!(!base.is_venv);
        std::fs::write(
            env.join("pyvenv.cfg"),
            "home = /opt/py/bin\nversion = 3.13.5\n",
        )
        .unwrap();
        let venv = probe(&python, "manual", Duration::from_secs(10), &ctx)
            .await
            .unwrap();
        assert!(venv.is_venv);
        assert_eq!(venv.venv_root.as_deref(), Some(env.as_path()));
        assert_eq!(venv.base_prefix, Path::new("/opt/py"));
    }

    #[test]
    fn venvs_are_classified_before_anything_else() {
        let loc = Locations { uv_dir: None };
        let p = Path::new("/nonexistent/.venv/bin/python");
        assert_eq!(
            classify(
                Hint::ProjectVenv,
                true,
                p,
                Path::new("/nonexistent/.venv"),
                &loc
            ),
            InterpreterKind::Venv
        );
        assert_eq!(
            classify(Hint::Poetry, true, p, Path::new("/nonexistent/.venv"), &loc),
            InterpreterKind::Poetry
        );
    }

    #[test]
    fn location_decides_uv_pyenv_and_system() {
        let dir = tempfile::tempdir().unwrap();
        let uv_dir = canonical_key(dir.path());
        let py = uv_dir.join("cpython-3.13").join("bin").join("python3.13");
        std::fs::create_dir_all(py.parent().unwrap()).unwrap();
        std::fs::write(&py, "").unwrap();
        let loc = Locations {
            uv_dir: Some(uv_dir.clone()),
        };
        let prefix = uv_dir.join("cpython-3.13");
        assert_eq!(
            classify(Hint::Uv, false, &py, &prefix, &loc),
            InterpreterKind::Uv
        );
        // The same file found on PATH is still the uv-managed Python.
        assert_eq!(
            classify(Hint::Path, false, &py, &prefix, &loc),
            InterpreterKind::Uv
        );
        // uv also reports system interpreters; those stay system.
        let sys = Path::new("/usr/bin/python3");
        assert_eq!(
            classify(Hint::Uv, false, sys, Path::new("/usr"), &loc),
            InterpreterKind::System
        );
        let pyenv = Path::new("/home/u/.pyenv/versions/3.11.9/bin/python");
        assert_eq!(
            classify(
                Hint::Path,
                false,
                pyenv,
                Path::new("/home/u/.pyenv/versions/3.11.9"),
                &loc
            ),
            InterpreterKind::Pyenv
        );
        // A conda prefix is recognized by conda-meta even when found on PATH.
        let conda = dir.path().join("miniconda");
        std::fs::create_dir_all(conda.join("conda-meta")).unwrap();
        assert_eq!(
            classify(
                Hint::Path,
                false,
                &conda.join("bin").join("python"),
                &conda,
                &loc
            ),
            InterpreterKind::Conda
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probes_a_fake_interpreter() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let py = dir.path().join("python3");
        std::fs::write(&py, format!("#!/bin/sh\necho '{SAMPLE}'\n")).unwrap();
        std::fs::set_permissions(&py, std::fs::Permissions::from_mode(0o755)).unwrap();
        let i = probe(&py, "test", Duration::from_secs(10), &RunContext::default())
            .await
            .unwrap();
        assert_eq!(i.version, "3.12.13");
        assert!(i.is_venv);
        assert_eq!(i.kind, InterpreterKind::Venv);
        assert_eq!(i.venv_root, Some(PathBuf::from("/work/.venv")));
        assert_eq!(i.path, py);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn broken_interpreters_fail_with_a_reason() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let py = dir.path().join("python");
        std::fs::write(&py, "#!/bin/sh\necho 'bad interpreter' 1>&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&py, std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = probe(&py, "test", Duration::from_secs(10), &RunContext::default())
            .await
            .unwrap_err();
        assert_eq!(e.message, "bad interpreter");

        let e = probe(
            &dir.path().join("missing"),
            "test",
            Duration::from_secs(10),
            &RunContext::default(),
        )
        .await
        .unwrap_err();
        assert!(e.message.contains("does not exist"), "{}", e.message);
    }
}
