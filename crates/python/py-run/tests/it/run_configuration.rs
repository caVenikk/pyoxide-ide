//! A Python run configuration started for real through `process`, with a
//! stand-in interpreter and uv (shell scripts that print what they got),
//! so no Python is needed: the command line, the working directory and
//! the environment (env files, then the configuration's variables,
//! `PYTHONPATH` roots, the system environment on and off), and `uv run`.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use process::{ProcessEvent, spawn};
use py_run::UvRun;
use py_run::config::{LaunchContext, PythonOptions, launch, new_configuration};
use run_config::{EnvVar, Storage};

/// Prints its arguments, folder and some variables, one per line.
const PYTHON: &str = r#"#!/bin/sh
echo "ARGS:$*"
echo "CWD:$(pwd)"
echo "FROM_FILE:${FROM_FILE-unset}"
echo "BOTH:${BOTH-unset}"
echo "EXPLICIT:${EXPLICIT-unset}"
echo "UNBUFFERED:${PYTHONUNBUFFERED-unset}"
echo "PYTHONPATH:${PYTHONPATH-unset}"
echo "HOME:${HOME-unset}"
echo "VIRTUAL_ENV:${VIRTUAL_ENV-unset}"
"#;

/// Prints what uv got, then runs the interpreter as `uv run` does.
const UV: &str = r#"#!/bin/sh
echo "UV:$*"
echo "UV_PROJECT_ENVIRONMENT:${UV_PROJECT_ENVIRONMENT-unset}"
shift
exec "$@"
"#;

fn executable(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Runs the command line and returns its standard output.
fn output(cmd: &process::CommandLine) -> String {
    let (_, events) = spawn(cmd).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut out = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left).unwrap() {
            ProcessEvent::Stdout(b) => out.extend(b),
            ProcessEvent::Stderr(b) => out.extend(b),
            ProcessEvent::Exited(_) => break,
        }
    }
    String::from_utf8(out).unwrap()
}

fn line<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .unwrap_or_else(|| panic!("no {key} in {text}"))
}

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
    python: PathBuf,
    uv: PathBuf,
}

fn project() -> Project {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let python = root.join(".venv/bin/python");
    executable(&python, PYTHON);
    std::fs::write(root.join(".venv/pyvenv.cfg"), "home = /usr/bin\n").unwrap();
    let uv = root.join("tools/uv");
    executable(&uv, UV);
    std::fs::create_dir_all(root.join("app")).unwrap();
    std::fs::write(root.join("app/main.py"), "").unwrap();
    std::fs::write(root.join(".env"), "FROM_FILE=file\nBOTH=file\n").unwrap();
    Project {
        _dir: dir,
        root,
        python,
        uv,
    }
}

#[test]
fn a_python_configuration_runs_with_its_command_line_and_environment() {
    let p = project();
    let root = &p.root;
    let mut config = new_configuration("main", Storage::Personal);
    let mut o = PythonOptions::read(&config);
    o.script = "app/main.py".into();
    o.parameters = "--root $PROJECT_DIR$ \"two words\"".into();
    o.interpreter_options = "-u".into();
    o.working_directory = "app".into();
    o.env_files = vec![".env".into()];
    o.write(&mut config);
    config.env.push(EnvVar::new("BOTH", "explicit", false));
    config.env.push(EnvVar::new("EXPLICIT", "yes", false));
    let sources = [root.join("src")];
    let cx = LaunchContext {
        root,
        source_roots: &sources,
        python: p.python.clone(),
        uv: None,
    };
    let run = launch(&config, &cx).unwrap().run;
    let display = run.display();
    assert_eq!(
        display,
        format!(
            "{} -u {}/app/main.py --root {} two words ",
            p.python.display(),
            root.display(),
            root.display()
        )
    );
    let out = output(&run.command_line());
    assert_eq!(
        line(&out, "ARGS"),
        format!(
            "-u {}/app/main.py --root {} two words",
            root.display(),
            root.display()
        )
    );
    assert_eq!(line(&out, "CWD"), root.join("app").display().to_string());
    // Files first, then the configuration's variables over them.
    assert_eq!(line(&out, "FROM_FILE"), "file");
    assert_eq!(line(&out, "BOTH"), "explicit");
    assert_eq!(line(&out, "EXPLICIT"), "yes");
    assert_eq!(line(&out, "UNBUFFERED"), "1");
    let python_path = line(&out, "PYTHONPATH");
    assert!(
        python_path.starts_with(&format!(
            "{}:{}",
            root.display(),
            root.join("src").display()
        )),
        "{python_path}"
    );
    assert_eq!(
        line(&out, "VIRTUAL_ENV"),
        root.join(".venv").display().to_string()
    );
    assert_ne!(
        line(&out, "HOME"),
        "unset",
        "the system environment is included"
    );

    // Without the system environment and without the roots.
    config.include_system_env = false;
    let mut o = PythonOptions::read(&config);
    o.add_content_roots = false;
    o.add_source_roots = false;
    o.write(&mut config);
    let run = launch(&config, &cx).unwrap().run;
    let out = output(&run.command_line());
    assert_eq!(line(&out, "HOME"), "unset");
    assert_eq!(line(&out, "PYTHONPATH"), "unset");
    assert_eq!(line(&out, "EXPLICIT"), "yes");
}

#[test]
fn with_uv_run_on_the_process_starts_through_uv() {
    let p = project();
    let root = &p.root;
    let mut config = new_configuration("serve", Storage::Personal);
    let mut o = PythonOptions::read(&config);
    o.script = "app.main".into();
    o.module_mode = true;
    o.write(&mut config);
    let venv = root.join(".venv");
    let cx = LaunchContext {
        root,
        source_roots: &[],
        python: p.python.clone(),
        uv: Some(UvRun {
            uv: p.uv.clone(),
            environment: venv.clone(),
        }),
    };
    let run = launch(&config, &cx).unwrap().run;
    assert_eq!(
        run.display(),
        format!("{} run {} -m app.main ", p.uv.display(), p.python.display())
    );
    let out = output(&run.command_line());
    assert_eq!(
        line(&out, "UV"),
        format!("run {} -m app.main", p.python.display())
    );
    assert_eq!(
        line(&out, "UV_PROJECT_ENVIRONMENT"),
        venv.display().to_string()
    );
    assert_eq!(line(&out, "VIRTUAL_ENV"), venv.display().to_string());
    assert_eq!(line(&out, "ARGS"), "-m app.main");
    assert_eq!(line(&out, "CWD"), root.display().to_string());
}
