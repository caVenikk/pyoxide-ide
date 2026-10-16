//! The project's virtual environment in real shells
//! (tests/scenarios/terminal/open-and-venv.md, steps 1 and 2): a session
//! planned with the Python product's customizer in a project with a uv
//! environment shows the environment's prompt prefix and runs its Python,
//! in a shell with Pyoxide's integration (zsh: the `activate` script is
//! sourced) and in one without (tcsh: the variables are merged).
//!
//! Ignored: they need zsh, tcsh and uv (macOS has the shells); run them with
//! `cargo test -p pyoxide-ide --test terminal_venv -- --ignored`.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use pyoxide_ide::terminal::VirtualEnvCustomizer;
use terminal::{Session, SessionEvent, StartRequest, TerminalSize, plan_session};

const TIMEOUT: Duration = Duration::from_secs(30);

/// A project with a uv environment `.venv`, and a home folder with empty
/// startup files.
struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
}

fn project() -> Project {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let root = base.join("demo");
    let home = base.join("home");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".zshrc"), "").unwrap();
    std::fs::write(home.join(".tcshrc"), "set prompt = 'tcsh> '\n").unwrap();
    let status = Command::new("uv")
        .args(["venv", "--quiet", ".venv"])
        .current_dir(&root)
        .status()
        .expect("uv is installed");
    assert!(status.success());
    Project {
        _dir: dir,
        root,
        home,
    }
}

/// Starts `shell` in the project, types `line` once the screen shows
/// `prompt` and returns the screen with the result line.
fn run(project: &Project, shell: &str, prompt: &str, line: &str) -> String {
    assert!(Path::new(shell).is_file(), "{shell} is not installed");
    let scripts = tempfile::tempdir().unwrap();
    let home = project.home.to_string_lossy().into_owned();
    let request = StartRequest {
        shell_path: Some(shell.to_owned()),
        project_root: Some(project.root.clone()),
        project_env: vec![("HOME".into(), home.clone()), ("ZDOTDIR".into(), home)],
        size: TerminalSize::new(200, 40),
        integration_dir: Some(scripts.path().to_owned()),
        ..StartRequest::default()
    };
    let plan = plan_session(&request, &[&VirtualEnvCustomizer::default()]);
    let (session, rx) = Session::spawn(plan.options).expect("start the shell");
    let deadline = Instant::now() + TIMEOUT;
    let mut typed = false;
    let mut text = String::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(SessionEvent::Exited(_)) => break,
            Ok(_) | Err(flume::RecvTimeoutError::Timeout) => {}
            Err(flume::RecvTimeoutError::Disconnected) => break,
        }
        text = session.snapshot().text();
        if !typed && text.contains(prompt) {
            session.write_text(&format!("{line}\r"));
            typed = true;
        } else if typed && result(&text).is_some() {
            break;
        }
    }
    session.kill();
    text
}

/// The result line `R:<VIRTUAL_ENV>:<python>:E` (not the command line).
fn result(text: &str) -> Option<String> {
    text.lines()
        .filter(|l| l.starts_with("R:") && l.trim_end().ends_with(":E"))
        .map(|l| l.trim_end().to_owned())
        .next_back()
}

fn expected(project: &Project) -> String {
    let venv = project.root.join(".venv");
    format!(
        "R:{}:{}:E",
        venv.display(),
        venv.join("bin").join("python").display()
    )
}

#[test]
#[ignore = "needs zsh and uv"]
fn zsh_sources_the_activation() {
    let project = project();
    let line = "echo \"R:$VIRTUAL_ENV:$(command -v python):E\"";
    let text = run(&project, "/bin/zsh", "(.venv) ", line);
    assert_eq!(result(&text), Some(expected(&project)), "{text}");
}

#[test]
#[ignore = "needs tcsh and uv"]
fn tcsh_gets_the_variables() {
    let project = project();
    // tcsh has no integration: the variables of the activation are merged,
    // the prompt prefix comes in PS1, which tcsh does not use.
    let line = "echo \"R:${VIRTUAL_ENV}:`which python`:E\"";
    let text = run(&project, "/bin/tcsh", "tcsh>", line);
    assert_eq!(result(&text), Some(expected(&project)), "{text}");
}
