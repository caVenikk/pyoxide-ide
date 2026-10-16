//! The product hook (ADR 0005, 0014): a language product adds environment
//! variables, a script to source before the first prompt and a default
//! working directory to a session, as the reference's
//! `LocalTerminalCustomizer` does (the Python product activates the
//! project's virtual environment). The platform knows no language.

use std::path::{Path, PathBuf};

use crate::session::{SessionOptions, TerminalSize};
use crate::start::{
    Os, ShellCommand, ShellKind, default_shell_path, environment, shell_command, working_directory,
};

/// What a customizer sees of a session about to start.
#[derive(Clone, Copy, Debug)]
pub struct ShellContext<'a> {
    pub command: &'a ShellCommand,
    pub kind: ShellKind,
    pub working_directory: &'a Path,
    pub project_root: Option<&'a Path>,
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
#[derive(Clone, Debug, Default)]
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
}

/// A session ready to spawn, with the scripts to source for the shell
/// integration.
#[derive(Clone, Debug)]
pub struct StartPlan {
    pub options: SessionOptions,
    pub kind: ShellKind,
    pub sources: Vec<SourceScript>,
}

/// Applies the start rules and the customizers to a request.
pub fn plan_session(request: &StartRequest, customizers: &[&dyn ShellCustomizer]) -> StartPlan {
    let command = request.command.clone().unwrap_or_else(|| {
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
    let context = ShellContext {
        command: &command,
        kind,
        working_directory: &cwd,
        project_root,
    };
    let mut base = Vec::new();
    let mut sources = Vec::new();
    for customizer in customizers {
        let customization = customizer.customize(&context);
        base.extend(customization.env);
        sources.extend(customization.source);
    }
    let env = environment(Os::current(), &base, &request.project_env);
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
    use super::*;

    struct Venv;

    impl ShellCustomizer for Venv {
        fn default_working_directory(&self, project_root: Option<&Path>) -> Option<PathBuf> {
            project_root.map(|r| r.join("missing-sub-folder"))
        }

        fn customize(&self, context: &ShellContext<'_>) -> Customization {
            Customization {
                env: vec![("VIRTUAL_ENV".into(), "/p/.venv".into())],
                source: (context.kind != ShellKind::Cmd).then(|| SourceScript {
                    path: PathBuf::from("/p/.venv/bin/activate"),
                    args: Vec::new(),
                }),
            }
        }
    }

    #[test]
    fn plan_applies_rules_and_customizers() {
        let root = std::env::temp_dir();
        let request = StartRequest {
            command: Some(ShellCommand {
                program: "/bin/bash".into(),
                args: vec!["-i".into()],
            }),
            project_root: Some(root.clone()),
            project_env: vec![("VIRTUAL_ENV".into(), "/override".into())],
            ..StartRequest::default()
        };
        let plan = plan_session(&request, &[&Venv]);
        assert_eq!(plan.kind, ShellKind::Bash);
        assert_eq!(plan.options.working_directory, root);
        assert_eq!(plan.sources.len(), 1);
        let venv = plan
            .options
            .env
            .iter()
            .find(|(n, _)| n == "VIRTUAL_ENV")
            .map(|(_, v)| v.as_str());
        // The project's own variables win.
        assert_eq!(venv, Some("/override"));
    }

    #[test]
    fn default_shell_when_the_setting_is_empty() {
        let request = StartRequest {
            shell_path: Some("  ".into()),
            ..StartRequest::default()
        };
        let plan = plan_session(&request, &[]);
        assert!(!plan.options.command.program.is_empty());
    }
}
