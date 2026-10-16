//! The shell integration with real shells (ADR 0014): each test starts a
//! shell the way the Terminal tool window does (`plan_session` with a
//! customizer that sources an activation script), with a home folder of
//! its own holding startup files, and checks that
//! - the user's startup files run in the shell's order, then the
//!   activation script, before the first prompt;
//! - the working directory is reported (OSC 7) at the start and after `cd`;
//! - the prompt and the commands are marked (OSC 133 A, B, C, D) with the
//!   exit codes, and Enter on an empty line is not a command;
//! - the session has none of Alacritty's variables, and the shell's own
//!   variables (`ZDOTDIR`, `HISTFILE`, `ENV`) are the user's again.
//!
//! They need the shells and are ignored by default; run them with
//! `cargo test -p terminal --test shells -- --ignored --test-threads=1`
//! (one test per shell: `zsh`, `bash`, `sh_on_macos` (`/bin/sh`, bash in
//! POSIX mode), `fish`, `powershell` (`pwsh` on macOS and Linux)). A test
//! fails when its shell is not installed.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use terminal::{
    Customization, MarkKind, Session, SessionEvent, ShellContext, ShellCustomizer, Snapshot,
    SourceScript, StartRequest, TerminalSize, plan_session,
};

const TIMEOUT: Duration = Duration::from_secs(30);

/// Sources `script` where the shell can, else sets a variable.
struct Activate {
    script: PathBuf,
}

impl ShellCustomizer for Activate {
    fn customize(&self, context: &ShellContext<'_>) -> Customization {
        if context.sources_scripts {
            Customization {
                env: Vec::new(),
                source: Some(SourceScript {
                    path: self.script.clone(),
                    args: Vec::new(),
                }),
            }
        } else {
            Customization {
                env: vec![("PYOXIDE_ACTIVATED".into(), "merged".into())],
                source: None,
            }
        }
    }
}

struct Shell {
    session: Option<Session>,
    rx: flume::Receiver<SessionEvent>,
    events: Vec<SessionEvent>,
    /// The folders, removed at the end.
    _dirs: Vec<tempfile::TempDir>,
    /// The home folder (the Unix shells' tests check paths in it).
    #[cfg(unix)]
    home: PathBuf,
    work: PathBuf,
}

/// A home folder with `files` (relative path, text), a working folder with
/// a `sub` folder, and an activation script `activate_name` with
/// `activate_text`.
fn start(program: &str, files: &[(&str, &str)], activate_name: &str, activate_text: &str) -> Shell {
    assert!(
        Path::new(program).is_file() || program == "powershell.exe",
        "{program} is not installed"
    );
    let home_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let home = home_dir.path().canonicalize().unwrap();
    let work = work_dir.path().canonicalize().unwrap();
    for (name, text) in files {
        let path = home.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    std::fs::create_dir(work.join("sub")).unwrap();
    let script = work.join(activate_name);
    std::fs::write(&script, activate_text).unwrap();
    let request = StartRequest {
        shell_path: Some(program.to_owned()),
        requested_directory: Some(work.clone()),
        project_root: Some(work.clone()),
        project_env: vec![
            ("HOME".into(), home.to_string_lossy().into_owned()),
            ("USERPROFILE".into(), home.to_string_lossy().into_owned()),
            ("ZDOTDIR".into(), home.to_string_lossy().into_owned()),
            // macOS bash 3.2 asks to switch to zsh otherwise.
            ("BASH_SILENCE_DEPRECATION_WARNING".into(), "1".into()),
            ("PYOXIDE_ORDER".into(), String::new()),
        ],
        size: TerminalSize::new(200, 60),
        integration_dir: Some(scripts.path().to_owned()),
        ..StartRequest::default()
    };
    let plan = plan_session(&request, &[&Activate { script }]);
    assert!(plan.shell_integration, "{:?}", plan.options.command);
    let (session, rx) = Session::spawn(plan.options).expect("start the shell");
    Shell {
        session: Some(session),
        rx,
        events: Vec::new(),
        _dirs: vec![home_dir, work_dir, scripts],
        #[cfg(unix)]
        home,
        work,
    }
}

impl Shell {
    fn session(&self) -> &Session {
        self.session.as_ref().unwrap()
    }

    fn wait(&mut self, what: &str, done: impl Fn(&Snapshot, &[SessionEvent]) -> bool) -> Snapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let snapshot = self.session().snapshot();
            if done(&snapshot, &self.events) {
                return snapshot;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left.min(Duration::from_millis(200))) {
                Ok(event) => self.events.push(event),
                Err(flume::RecvTimeoutError::Timeout) if !left.is_zero() => {}
                Err(e) => panic!(
                    "waiting for {what}: {e:?}; screen:\n{}\nevents: {:?}",
                    snapshot.text(),
                    self.events
                ),
            }
        }
    }

    fn marks(&self, kind: MarkKind) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, SessionEvent::Mark(m) if m.kind == kind))
            .count()
    }

    fn finished(&self) -> Vec<Option<i32>> {
        self.events
            .iter()
            .filter_map(|e| match e {
                SessionEvent::Mark(m) => match m.kind {
                    MarkKind::CommandFinished { exit_code } => Some(exit_code),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    fn last_directory(&self) -> Option<PathBuf> {
        self.events.iter().rev().find_map(|e| match e {
            SessionEvent::WorkingDirectory(dir) => Some(dir.canonicalize().unwrap_or(dir.clone())),
            _ => None,
        })
    }

    /// Waits for the prompt (its end mark) after `count` of them.
    fn wait_prompt(&mut self, count: usize) {
        self.wait("the prompt", |_, events| {
            events
                .iter()
                .filter(|e| matches!(e, SessionEvent::Mark(m) if m.kind == MarkKind::CommandStart))
                .count()
                > count
        });
    }

    /// Runs a command line and waits for the next prompt.
    fn run(&mut self, line: &str) {
        let prompts = self.marks(MarkKind::CommandStart);
        self.session().write_text(&format!("{line}\r"));
        self.wait_prompt(prompts);
    }

    /// The text between `RESULT:` and `:END` in the last output on the
    /// screen (not the command line, which names variables).
    fn result(&mut self, line: &str) -> String {
        self.run(line);
        let snapshot = self.session().snapshot();
        let text = snapshot.text();
        let found = text
            .lines()
            .filter_map(|l| {
                let start = l.find("RESULT:")? + "RESULT:".len();
                let end = l[start..].find(":END")? + start;
                Some(l[start..end].to_owned())
            })
            .rfind(|r| !r.contains('$'));
        found.unwrap_or_else(|| panic!("no result for {line:?} in\n{text}"))
    }

    /// The checks every shell passes; `false_command` exits with 3.
    fn check_marks_and_directory(&mut self, cd: &str, exit_3: &str) {
        self.wait_prompt(0);
        assert_eq!(self.last_directory().as_deref(), Some(self.work.as_path()));
        assert!(self.marks(MarkKind::PromptStart) >= 1, "{:?}", self.events);
        assert!(self.finished().is_empty(), "{:?}", self.events);

        self.run(exit_3);
        assert_eq!(self.finished(), [Some(3)], "{:?}", self.events);
        assert_eq!(self.marks(MarkKind::CommandExecuted), 1);

        // Enter on an empty line: a prompt again, no command.
        self.run("");
        assert_eq!(self.finished().len(), 1, "{:?}", self.events);
        assert_eq!(self.marks(MarkKind::CommandExecuted), 1);

        self.run(cd);
        assert_eq!(self.finished(), [Some(3), Some(0)], "{:?}", self.events);
        let sub = self.work.join("sub");
        assert_eq!(self.last_directory().as_deref(), Some(sub.as_path()));
    }

    fn exit(&mut self, line: &str) {
        self.session().write_text(&format!("{line}\r"));
        self.wait("the exit", |_, events| {
            events.iter().any(|e| matches!(e, SessionEvent::Exited(_)))
        });
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        if let Some(session) = &self.session
            && session.exit_status().is_none()
        {
            session.kill();
        }
    }
}

#[cfg(unix)]
const ACTIVATE_SH: &str = "PYOXIDE_ORDER=\"$PYOXIDE_ORDER activate\"\n\
export PYOXIDE_ACTIVATED=yes\n\
PS1=\"(t) $PS1\"\n";

#[cfg(unix)]
#[test]
#[ignore = "needs zsh"]
fn zsh() {
    let login = cfg!(target_os = "macos");
    let mut sh = start(
        "/bin/zsh",
        &[
            (
                ".zshenv",
                "export PYOXIDE_ORDER=\"$PYOXIDE_ORDER zshenv\"\n",
            ),
            (".zprofile", "PYOXIDE_ORDER=\"$PYOXIDE_ORDER zprofile\"\n"),
            (
                ".zshrc",
                "PYOXIDE_ORDER=\"$PYOXIDE_ORDER zshrc\"\nPS1='zsh-prompt> '\n",
            ),
            (".zlogin", "PYOXIDE_ORDER=\"$PYOXIDE_ORDER zlogin\"\n"),
        ],
        "activate",
        ACTIVATE_SH,
    );
    sh.check_marks_and_directory("cd sub", "(exit 3)");
    let order = sh.result("echo \"RESULT:${PYOXIDE_ORDER}:END\"");
    let expected = if login {
        " zshenv zprofile zshrc zlogin activate"
    } else {
        " zshenv zshrc activate"
    };
    assert_eq!(order, expected);
    assert_eq!(sh.result("echo \"RESULT:$PYOXIDE_ACTIVATED:END\""), "yes");
    let home = sh.home.to_string_lossy().into_owned();
    assert_eq!(sh.result("echo \"RESULT:${ZDOTDIR-unset}:END\""), home);
    assert_eq!(
        sh.result("echo \"RESULT:${ALACRITTY_WINDOW_ID-unset}${WINDOWID-unset}:END\""),
        "unsetunset"
    );
    assert_eq!(
        sh.result("echo \"RESULT:${__PYOXIDE_ZSH_DIR-unset}${__PYOXIDE_SOURCE_1-unset}:END\""),
        "unsetunset"
    );
    if login {
        // /etc/zshrc of macOS sets it from ZDOTDIR.
        assert_eq!(
            sh.result("echo \"RESULT:$HISTFILE:END\""),
            format!("{home}/.zsh_history")
        );
    }
    let snapshot = sh.session().snapshot();
    assert!(
        snapshot.text().contains("(t) zsh-prompt> "),
        "{}",
        snapshot.text()
    );
    sh.exit("exit");
}

#[cfg(unix)]
#[test]
#[ignore = "needs bash"]
fn bash() {
    let login = cfg!(target_os = "macos");
    let mut sh = start(
        "/bin/bash",
        &[
            (
                ".bash_profile",
                "PYOXIDE_ORDER=\"$PYOXIDE_ORDER profile\"\nPS1='bash-prompt> '\n",
            ),
            (
                ".bashrc",
                "PYOXIDE_ORDER=\"$PYOXIDE_ORDER bashrc\"\nPS1='bash-prompt> '\n",
            ),
        ],
        "activate",
        ACTIVATE_SH,
    );
    // bash runs no DEBUG trap before a subshell `( ... )`, so a command
    // line of one is not marked (nor in the reference): a simple command.
    sh.check_marks_and_directory("cd sub", "/bin/sh -c 'exit 3'");
    let expected = if login {
        " profile activate"
    } else {
        " bashrc activate"
    };
    assert_eq!(sh.result("echo \"RESULT:${PYOXIDE_ORDER}:END\""), expected);
    assert_eq!(sh.result("echo \"RESULT:$PYOXIDE_ACTIVATED:END\""), "yes");
    assert_eq!(
        sh.result("echo \"RESULT:${ALACRITTY_WINDOW_ID-unset}${WINDOWID-unset}:END\""),
        "unsetunset"
    );
    // `$_` survives the DEBUG trap.
    assert_eq!(
        sh.result("true lastarg && echo \"RESULT:$_:END\""),
        "lastarg"
    );
    // `$?` reaches the prompt.
    sh.run("PS1='[$?] bash-prompt> '");
    sh.run("/bin/sh -c 'exit 5'");
    let snapshot = sh.session().snapshot();
    assert!(
        snapshot.text().contains("[5] bash-prompt>"),
        "{}",
        snapshot.text()
    );
    assert_eq!(sh.finished().last(), Some(&Some(5)));
    sh.exit("exit");
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "needs /bin/sh (bash) of macOS"]
fn sh_on_macos() {
    // bash in POSIX mode: it reads $ENV (Pyoxide's script), which reads the
    // profile files and then the user's own ENV (named by .profile here).
    let mut sh = start(
        "/bin/sh",
        &[
            (
                ".profile",
                "PYOXIDE_ORDER=\"$PYOXIDE_ORDER profile\"\nPS1='sh-prompt> '\n\
                 ENV=$HOME/.shrc; export ENV\n",
            ),
            (".shrc", "PYOXIDE_ORDER=\"$PYOXIDE_ORDER shrc\"\n"),
        ],
        "activate",
        ACTIVATE_SH,
    );
    sh.check_marks_and_directory("cd sub", "/bin/sh -c 'exit 3'");
    assert_eq!(
        sh.result("echo \"RESULT:${PYOXIDE_ORDER}:END\""),
        " profile shrc activate"
    );
    assert_eq!(sh.result("echo \"RESULT:$PYOXIDE_ACTIVATED:END\""), "yes");
    let shrc = sh.home.join(".shrc").to_string_lossy().into_owned();
    assert_eq!(sh.result("echo \"RESULT:${ENV-unset}:END\""), shrc);
    assert_eq!(
        sh.result("echo \"RESULT:${ALACRITTY_WINDOW_ID-unset}${__PYOXIDE_SOURCE_1-unset}:END\""),
        "unsetunset"
    );
    let snapshot = sh.session().snapshot();
    assert!(
        snapshot.text().contains("(t) sh-prompt> "),
        "{}",
        snapshot.text()
    );
    sh.exit("exit");
}

#[cfg(unix)]
#[test]
#[ignore = "needs fish"]
fn fish() {
    let program = [
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
    ]
    .into_iter()
    .find(|p| Path::new(p).is_file())
    .unwrap_or("fish");
    let mut sh = start(
        program,
        &[(
            ".config/fish/config.fish",
            "set -gx PYOXIDE_ORDER \"$PYOXIDE_ORDER config\"\n",
        )],
        "activate.fish",
        "set -gx PYOXIDE_ORDER \"$PYOXIDE_ORDER activate\"\nset -gx PYOXIDE_ACTIVATED yes\n",
    );
    sh.check_marks_and_directory("cd sub", "sh -c 'exit 3'");
    assert_eq!(
        sh.result("echo \"RESULT:$PYOXIDE_ORDER:END\""),
        " config activate"
    );
    assert_eq!(sh.result("echo \"RESULT:$PYOXIDE_ACTIVATED:END\""), "yes");
    sh.exit("exit");
}

#[test]
#[ignore = "needs PowerShell"]
fn powershell() {
    let program = if cfg!(windows) {
        "powershell.exe"
    } else {
        [
            "/opt/homebrew/bin/pwsh",
            "/usr/local/bin/pwsh",
            "/usr/bin/pwsh",
        ]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .unwrap_or("pwsh")
    };
    let profile = if cfg!(windows) {
        "Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1"
    } else {
        ".config/powershell/Microsoft.PowerShell_profile.ps1"
    };
    let mut sh = start(
        program,
        &[(
            profile,
            "$env:PYOXIDE_ORDER = \"$env:PYOXIDE_ORDER profile\"\n",
        )],
        "Activate.ps1",
        "$env:PYOXIDE_ORDER = \"$env:PYOXIDE_ORDER activate\"\n$env:PYOXIDE_ACTIVATED = 'yes'\n",
    );
    let exit_3 = if cfg!(windows) {
        "cmd /c exit 3"
    } else {
        "/bin/sh -c 'exit 3'"
    };
    sh.check_marks_and_directory("cd sub", exit_3);
    assert_eq!(
        sh.result("Write-Output \"RESULT:$($env:PYOXIDE_ORDER):END\""),
        " profile activate"
    );
    assert_eq!(
        sh.result("Write-Output \"RESULT:$($env:PYOXIDE_ACTIVATED):END\""),
        "yes"
    );
    sh.exit("exit");
}
