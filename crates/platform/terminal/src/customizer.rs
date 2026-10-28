//! The product hook (ADR 0005, 0014): a language product adds environment
//! variables, a script to source before the first prompt and a default
//! working directory to a session, as the reference's
//! `LocalTerminalCustomizer` does (the Python product activates the
//! project's virtual environment). The platform knows no language.

use std::path::{Path, PathBuf};

use crate::integration;
use crate::session::{SessionOptions, TerminalSize};
use crate::start::{
    Os, ShellCommand, ShellKind, default_shell_path, environment, set_variable, shell_command,
    variable, working_directory,
};

/// What a customizer sees of a session about to start.
#[derive(Clone, Copy, Debug)]
pub struct ShellContext<'a> {
    pub command: &'a ShellCommand,
    pub kind: ShellKind,
    pub working_directory: &'a Path,
    pub project_root: Option<&'a Path>,
    /// The "Shell integration" setting is on. The reference activates a
    /// virtual environment only then.
    pub shell_integration: bool,
    /// The shell runs Pyoxide's integration, which sources
    /// [`Customization::source`] before the first prompt; otherwise only
    /// [`Customization::env`] reaches it (cmd.exe and other shells).
    pub sources_scripts: bool,
}

/// A script the shell sources after the user's rc files and before the
/// first prompt (with shell integration on), for example a virtual
/// environment's `activate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceScript {
    pub path: PathBuf,
    pub args: Vec<String>,
}

/// What a customizer adds to a session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Customization {
    /// Variables set in the session (shells without integration get the
    /// activation's variables this way).
    pub env: Vec<(String, String)>,
    pub source: Option<SourceScript>,
}

pub trait ShellCustomizer: Send + Sync {
    /// The folder new sessions start in when neither the request nor the
    /// "Start directory" setting names one (before the project root).
    fn default_working_directory(&self, project_root: Option<&Path>) -> Option<PathBuf> {
        let _ = project_root;
        None
    }

    fn customize(&self, context: &ShellContext<'_>) -> Customization;
}

/// What a new session is asked for.
#[derive(Clone, Debug)]
pub struct StartRequest {
    /// The "Shell path" setting; empty or `None`: the default shell.
    pub shell_path: Option<String>,
    /// A shell command chosen explicitly (New Predefined Session); wins
    /// over `shell_path`.
    pub command: Option<ShellCommand>,
    /// A folder (or file) to start in: Open In > Terminal, a restored tab.
    pub requested_directory: Option<PathBuf>,
    /// The "Start directory" setting.
    pub start_directory: Option<PathBuf>,
    pub project_root: Option<PathBuf>,
    /// The project's "Environment variables" setting.
    pub project_env: Vec<(String, String)>,
    pub size: TerminalSize,
    /// Lines of scrollback; 0: [`crate::DEFAULT_SCROLLBACK`].
    pub scrollback_lines: usize,
    /// The "Shell integration" setting (on by default): the shells Pyoxide
    /// has scripts for ([`integration::supports`]) report the working
    /// directory, the prompt and the commands, and source what the
    /// customizers ask for.
    pub shell_integration: bool,
    /// Where the integration scripts are written (in a folder per
    /// version); `None`: [`integration::default_base_dir`].
    pub integration_dir: Option<PathBuf>,
}

impl Default for StartRequest {
    fn default() -> Self {
        Self {
            shell_path: None,
            command: None,
            requested_directory: None,
            start_directory: None,
            project_root: None,
            project_env: Vec::new(),
            size: TerminalSize::default(),
            scrollback_lines: 0,
            shell_integration: true,
            integration_dir: None,
        }
    }
}

/// A session ready to spawn.
#[derive(Clone, Debug)]
pub struct StartPlan {
    pub options: SessionOptions,
    pub kind: ShellKind,
    /// The scripts the customizers asked to source.
    pub sources: Vec<SourceScript>,
    /// The shell runs Pyoxide's integration: it reports its working
    /// directory (OSC 7) and its prompt and commands (OSC 133).
    pub shell_integration: bool,
}

/// Applies the start rules, the customizers and the shell integration to
/// a request. Touches the file system (folders, the integration scripts,
/// what customizers read): call it off the UI thread.
pub fn plan_session(request: &StartRequest, customizers: &[&dyn ShellCustomizer]) -> StartPlan {
    let mut command = request.command.clone().unwrap_or_else(|| {
        let path = request
            .shell_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_shell_path);
        shell_command(&path)
    });
    let project_root = request.project_root.as_deref();
    let default_directory = customizers
        .iter()
        .find_map(|c| c.default_working_directory(project_root));
    let home = home_dir();
    let cwd = working_directory(
        request.requested_directory.as_deref(),
        request.start_directory.as_deref(),
        default_directory.as_deref(),
        project_root,
        home.as_deref(),
    )
    .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }));
    let kind = command.kind();
    let os = Os::current();
    let scripts = if request.shell_integration && integration::supports(kind, os) {
        let base = request
            .integration_dir
            .clone()
            .or_else(integration::default_base_dir);
        match base.map(|b| integration::prepare(&b)) {
            Some(Ok(dir)) => Some(dir),
            Some(Err(e)) => {
                eprintln!("terminal: shell integration unavailable: {e}");
                None
            }
            None => None,
        }
    } else {
        None
    };
    let context = ShellContext {
        command: &command,
        kind,
        working_directory: &cwd,
        project_root,
        shell_integration: request.shell_integration,
        sources_scripts: scripts.is_some(),
    };
    let mut base = Vec::new();
    let mut sources = Vec::new();
    for customizer in customizers {
        let customization = customizer.customize(&context);
        base.extend(customization.env);
        sources.extend(customization.source);
    }
    let mut env = environment(os, &base, &request.project_env);
    let injected = scripts.as_deref().and_then(|dir| {
        // The session's variables over the inherited ones.
        let session_var = |name: &str| {
            variable(os, &env, name)
                .map(str::to_owned)
                .or_else(|| std::env::var(name).ok())
        };
        integration::inject(os, dir, &command, &sources, &session_var, home.as_deref())
    });
    let shell_integration = injected.is_some();
    if let Some(injected) = injected {
        command = injected.command;
        // The integration's variables come last (ADR 0014).
        for (name, value) in &injected.env {
            set_variable(os, &mut env, name, value);
        }
    }
    StartPlan {
        options: SessionOptions {
            command,
            working_directory: cwd,
            env,
            size: request.size,
            scrollback_lines: request.scrollback_lines,
        },
        kind,
        sources,
        shell_integration,
    }
}

/// The user's home folder from the environment.
pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Activates `/p/.venv` and remembers what it was told.
    #[derive(Default)]
    struct Venv {
        seen: Mutex<Vec<(bool, bool)>>,
    }

    impl ShellCustomizer for Venv {
        fn default_working_directory(&self, project_root: Option<&Path>) -> Option<PathBuf> {
            project_root.map(|r| r.join("missing-sub-folder"))
        }

        fn customize(&self, context: &ShellContext<'_>) -> Customization {
            self.seen
                .lock()
                .unwrap()
                .push((context.shell_integration, context.sources_scripts));
            Customization {
                env: vec![("VIRTUAL_ENV".into(), "/p/.venv".into())],
                source: context.sources_scripts.then(|| SourceScript {
                    path: PathBuf::from("/p/.venv/bin/activate"),
                    args: Vec::new(),
                }),
            }
        }
    }

    fn get<'a>(plan: &'a StartPlan, name: &str) -> Option<&'a str> {
        plan.options
            .env
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn shell(program: &str, args: &[&str]) -> Option<ShellCommand> {
        Some(ShellCommand {
            program: program.into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        })
    }

    #[test]
    fn plan_applies_rules_and_customizers() {
        let root = std::env::temp_dir();
        let scripts = tempfile::tempdir().unwrap();
        let request = StartRequest {
            command: shell("/bin/bash", &["-i"]),
            project_root: Some(root.clone()),
            project_env: vec![("VIRTUAL_ENV".into(), "/override".into())],
            integration_dir: Some(scripts.path().to_owned()),
            ..StartRequest::default()
        };
        let venv = Venv::default();
        let plan = plan_session(&request, &[&venv]);
        assert_eq!(plan.kind, ShellKind::Bash);
        assert_eq!(plan.options.working_directory, root);
        assert_eq!(plan.sources.len(), 1);
        // The project's own variables win.
        assert_eq!(get(&plan, "VIRTUAL_ENV"), Some("/override"));
        assert_eq!(*venv.seen.lock().unwrap(), [(true, true)]);
        // The integration: bash reads Pyoxide's rcfile, which sources the
        // activation.
        assert!(plan.shell_integration);
        let dir = scripts.path().join(integration::version());
        let rcfile = dir.join("bash").join("shell-integration.bash");
        assert_eq!(plan.options.command.args[0], "--rcfile");
        assert_eq!(Path::new(&plan.options.command.args[1]), rcfile);
        assert!(rcfile.is_file());
        assert_eq!(get(&plan, "__IDE_SOURCE_1"), Some("/p/.venv/bin/activate"));
    }

    #[test]
    fn integration_off_and_shells_without_it() {
        let scripts = tempfile::tempdir().unwrap();
        // The setting is off: no scripts, the command as it is.
        let request = StartRequest {
            command: shell("/bin/zsh", &["-i"]),
            shell_integration: false,
            integration_dir: Some(scripts.path().to_owned()),
            ..StartRequest::default()
        };
        let venv = Venv::default();
        let plan = plan_session(&request, &[&venv]);
        assert!(!plan.shell_integration);
        assert_eq!(plan.options.command, request.command.clone().unwrap());
        assert_eq!(get(&plan, "ZDOTDIR"), None);
        assert!(plan.sources.is_empty());
        assert_eq!(*venv.seen.lock().unwrap(), [(false, false)]);
        assert!(std::fs::read_dir(scripts.path()).unwrap().next().is_none());

        // A shell without integration: only the variables.
        let request = StartRequest {
            command: shell("/bin/tcsh", &[]),
            integration_dir: Some(scripts.path().to_owned()),
            ..StartRequest::default()
        };
        let venv = Venv::default();
        let plan = plan_session(&request, &[&venv]);
        assert!(!plan.shell_integration);
        assert_eq!(get(&plan, "VIRTUAL_ENV"), Some("/p/.venv"));
        assert_eq!(get(&plan, "__IDE_SOURCE_1"), None);
        assert_eq!(*venv.seen.lock().unwrap(), [(true, false)]);

        // zsh: ZDOTDIR points to the integration, the project's ZDOTDIR
        // is the one to restore.
        let request = StartRequest {
            command: shell("/bin/zsh", &["-i"]),
            project_env: vec![("ZDOTDIR".into(), "/u/zsh".into())],
            integration_dir: Some(scripts.path().to_owned()),
            ..StartRequest::default()
        };
        let plan = plan_session(&request, &[]);
        assert!(plan.shell_integration);
        let zsh_dir = scripts.path().join(integration::version()).join("zsh");
        assert_eq!(get(&plan, "ZDOTDIR").map(PathBuf::from), Some(zsh_dir));
        assert_eq!(
            get(&plan, integration::ORIGINAL_ZDOTDIR_VAR),
            Some("/u/zsh")
        );
    }

    #[test]
    fn default_shell_when_the_setting_is_empty() {
        let scripts = tempfile::tempdir().unwrap();
        let request = StartRequest {
            shell_path: Some("  ".into()),
            integration_dir: Some(scripts.path().to_owned()),
            ..StartRequest::default()
        };
        let plan = plan_session(&request, &[]);
        assert!(!plan.options.command.program.is_empty());
    }
}
