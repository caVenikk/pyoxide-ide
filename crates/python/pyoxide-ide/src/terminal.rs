//! The Python product's part in terminal sessions (ADR 0014), as the
//! reference's `PyVirtualEnvTerminalCustomizer` (PyCharm 2026.2): a new
//! session in the project activates the project's interpreter when it is a
//! virtual environment (uv's included) and "Activate virtualenv" is on.
//!
//! - Shells with Pyoxide's integration (zsh, bash, `/bin/sh` of macOS,
//!   fish, PowerShell) source the environment's activation script after
//!   the user's startup files and before the first prompt: `activate`,
//!   `activate.fish`, `Activate.ps1` (`activate.ps1` in uv's), from the
//!   folder of the interpreter (`bin`, `Scripts`).
//! - cmd.exe and other shells get the variables the activation sets, merged
//!   into the session's environment: `VIRTUAL_ENV`, `PATH` with the
//!   scripts folder first, `PYTHONHOME` cleared, and the prompt (`PROMPT`
//!   on Windows, `PS1` elsewhere), with the `_OLD_VIRTUAL_*` variables
//!   `deactivate` restores. The reference runs `activate.bat` (or
//!   `activate` in `/bin/bash`) to read them; Pyoxide sets them as the
//!   script does, without starting a process.
//! - With "Shell integration" off nothing is activated (as in the
//!   reference); conda environments are not activated yet.
//!
//! "Activate virtualenv" is a shared project setting,
//! `[python.terminal] activate-virtualenv` in `.oxide/project.toml`, on by
//! default. While the table does not exist the reference's setting is
//! imported once from `.idea/python-terminal.xml` (component
//! `PyVirtualEnvTerminalCustomizer`, option `virtualEnvActivate`); `.idea`
//! is never written.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ::terminal::start::Os;
use ::terminal::{Customization, ShellContext, ShellCustomizer, ShellKind, SourceScript};
use gpui::App;
use py_env::venv::{EnvDir, EnvLayout};
use workbench::{TerminalHook, TerminalProvider};

use crate::interpreter;

/// The table of the Python terminal settings in `.oxide/project.toml`.
pub const SETTINGS_TABLE: &str = "python.terminal";
/// "Activate virtualenv".
pub const ACTIVATE_KEY: &str = "activate-virtualenv";
/// The reference's file of the setting in `.idea`.
pub const IDEA_FILE: &str = "python-terminal.xml";
const IDEA_COMPONENT: &str = "PyVirtualEnvTerminalCustomizer";
const IDEA_OPTION: &str = "virtualEnvActivate";

/// The product's terminal hook.
pub fn hook() -> TerminalHook {
    TerminalHook::new(Provider)
}

struct Provider;

impl TerminalProvider for Provider {
    fn customizers(&self, project_root: Option<&Path>, cx: &App) -> Vec<Arc<dyn ShellCustomizer>> {
        let choice = project_root
            .map(|root| interpreter::ProjectChoice::of(root, cx))
            .unwrap_or_default();
        vec![Arc::new(VirtualEnvCustomizer { choice })]
    }
}

/// Activates the project's virtual environment in a session that starts
/// in the project.
#[derive(Debug, Clone, Default)]
pub struct VirtualEnvCustomizer {
    /// The interpreter chosen in this session, if any (the interpreter
    /// widget), and the configured interpreters; else the project's as
    /// stored or found.
    choice: interpreter::ProjectChoice,
}

impl ShellCustomizer for VirtualEnvCustomizer {
    fn customize(&self, context: &ShellContext<'_>) -> Customization {
        if !context.shell_integration {
            return Customization::default();
        }
        let Some(root) = context.project_root else {
            return Customization::default();
        };
        // The reference takes the interpreter of the module that owns the
        // working directory: a Pyoxide project is one module.
        if !inside(context.working_directory, root) || !activate_virtualenv(root) {
            return Customization::default();
        }
        let Some(python) = interpreter::project_python(root, &self.choice) else {
            return Customization::default();
        };
        activation(
            Os::current(),
            &python,
            context.kind,
            context.sources_scripts,
            &|name| std::env::var(name).ok(),
        )
    }
}

/// `dir` is `root` or inside it (as given or with links resolved).
fn inside(dir: &Path, root: &Path) -> bool {
    if dir.starts_with(root) {
        return true;
    }
    match (
        native_path::canonicalize(dir),
        native_path::canonicalize(root),
    ) {
        (Ok(dir), Ok(root)) => dir.starts_with(root),
        _ => false,
    }
}

/// What activates the virtual environment of `python` in a shell of
/// `kind`: its activation script when the shell sources scripts and the
/// environment has one for it, else the variables the activation sets
/// (over the inherited variables `var`). Nothing for an interpreter that
/// is not in a virtual environment. Reads files.
pub fn activation(
    os: Os,
    python: &Path,
    kind: ShellKind,
    sources_scripts: bool,
    var: &dyn Fn(&str) -> Option<String>,
) -> Customization {
    let Some(env) = py_env::venv::inspect_python(python).filter(|e| e.layout == EnvLayout::Venv)
    else {
        return Customization::default();
    };
    let Some(scripts) = python.parent() else {
        return Customization::default();
    };
    if sources_scripts && let Some(script) = activate_script(scripts, kind) {
        return Customization {
            env: Vec::new(),
            source: Some(SourceScript {
                path: script,
                args: Vec::new(),
            }),
        };
    }
    Customization {
        env: activation_variables(os, &env, scripts, var),
        source: None,
    }
}

/// The activation script for a shell in the scripts folder of a virtual
/// environment (`PythonEnvironment.Venv.activation`): `Activate.ps1` for
/// PowerShell (`activate.ps1` in environments made by uv or virtualenv),
/// `activate.fish` for fish, `activate` for bash, zsh and sh; none for
/// cmd.exe and other shells.
pub fn activate_script(scripts: &Path, kind: ShellKind) -> Option<PathBuf> {
    let names: &[&str] = match kind {
        ShellKind::PowerShell => &["Activate.ps1", "activate.ps1"],
        ShellKind::Fish => &["activate.fish"],
        ShellKind::Bash | ShellKind::Zsh | ShellKind::Sh => &["activate"],
        ShellKind::Cmd | ShellKind::Other => &[],
    };
    names
        .iter()
        .map(|name| scripts.join(name))
        .find(|path| path.is_file())
}

/// The variables the activation of `env` sets, as its `activate.bat` (and
/// `activate`) set them, given the inherited variables `var`; the names
/// are those the reference keeps of an activation it reads.
fn activation_variables(
    os: Os,
    env: &EnvDir,
    scripts: &Path,
    var: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut set = |name: &str, value: String| out.push((name.to_owned(), value));
    set("VIRTUAL_ENV", env.dir.to_string_lossy().into_owned());
    let scripts = scripts.to_string_lossy().into_owned();
    let old_path = var("PATH").unwrap_or_default();
    let separator = if os == Os::Windows { ';' } else { ':' };
    let path = if old_path.is_empty() {
        scripts
    } else {
        format!("{scripts}{separator}{old_path}")
    };
    set("_OLD_VIRTUAL_PATH", old_path);
    set("PATH", path);
    // Python reads an empty PYTHONHOME as none (a session cannot unset an
    // inherited variable).
    if let Some(home) = var("PYTHONHOME") {
        set("_OLD_VIRTUAL_PYTHONHOME", home);
        set("PYTHONHOME", String::new());
    }
    if var("VIRTUAL_ENV_DISABLE_PROMPT").is_none_or(|v| v.is_empty()) {
        let prompt = prompt_name(env);
        if os == Os::Windows {
            let old = var("PROMPT")
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| "$P$G".to_owned());
            set("PROMPT", format!("({prompt}) {old}"));
            set("_OLD_VIRTUAL_PROMPT", old);
        } else {
            let old = var("PS1").unwrap_or_default();
            set("PS1", format!("({prompt}) {old}"));
        }
    }
    out
}

/// The prompt prefix of an environment: `prompt` of its `pyvenv.cfg` (in
/// quotes when Python's `venv --prompt` wrote it), else its folder's name.
fn prompt_name(env: &EnvDir) -> String {
    let configured = env
        .cfg
        .as_ref()
        .and_then(|cfg| cfg.get("prompt"))
        .map(|p| {
            let quoted = p.len() >= 2
                && ((p.starts_with('\'') && p.ends_with('\''))
                    || (p.starts_with('"') && p.ends_with('"')));
            if quoted { &p[1..p.len() - 1] } else { p }
        })
        .filter(|p| !p.is_empty());
    match configured {
        Some(prompt) => prompt.to_owned(),
        None => env
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

// ---- the setting ------------------------------------------------------------

/// "Activate virtualenv" of the project at `root` (on by default); while
/// `[python.terminal]` does not exist, the reference's setting is imported
/// once ([`import_idea`]). Reads (and may write) files: off the UI thread.
pub fn activate_virtualenv(root: &Path) -> bool {
    match project::store::load_table(root, SETTINGS_TABLE) {
        Ok(Some(table)) => table
            .get(ACTIVATE_KEY)
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        Ok(None) => import_idea(root).unwrap_or(true),
        Err(e) => {
            eprintln!("terminal: {e}");
            true
        }
    }
}

/// Imports "Activate virtualenv" from `.idea/python-terminal.xml` into
/// `[python.terminal]`, which ends the import. Returns the value when the
/// reference's file has it.
pub fn import_idea(root: &Path) -> Option<bool> {
    let value = match project::idea::component_option(root, IDEA_FILE, IDEA_COMPONENT, IDEA_OPTION)?
        .as_str()
    {
        "true" => true,
        "false" => false,
        _ => return None,
    };
    if let Err(e) =
        project::store::save_values(root, SETTINGS_TABLE, &[(ACTIVATE_KEY, Some(value.into()))])
    {
        eprintln!("terminal: {e}");
    }
    Some(value)
}

/// Sets "Activate virtualenv" of the project at `root`. The default (on)
/// is not written, but `[python.terminal]` stays, so the reference's
/// setting is not imported over it. Returns whether the file was written.
pub fn set_activate_virtualenv(root: &Path, on: bool) -> Result<bool, project::StoreError> {
    use project::store::save_values;
    if !on {
        return save_values(root, SETTINGS_TABLE, &[(ACTIVATE_KEY, Some(false.into()))]);
    }
    if project::store::load_table(root, SETTINGS_TABLE)?.is_some() {
        return save_values(root, SETTINGS_TABLE, &[(ACTIVATE_KEY, None)]);
    }
    // The store creates a table only with a value in it: the key is
    // written and removed, which leaves the table.
    save_values(root, SETTINGS_TABLE, &[(ACTIVATE_KEY, Some(true.into()))])?;
    save_values(root, SETTINGS_TABLE, &[(ACTIVATE_KEY, None)])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A virtual environment in `root/.venv` with an interpreter and the
    /// activation scripts `scripts` in its scripts folder.
    fn venv(root: &Path, cfg: &str, scripts: &[&str]) -> PathBuf {
        let dir = root.join(".venv");
        let bin = dir.join(if cfg!(windows) { "Scripts" } else { "bin" });
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(dir.join("pyvenv.cfg"), cfg).unwrap();
        let python = bin.join(if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        });
        std::fs::write(&python, "").unwrap();
        for script in scripts {
            std::fs::write(bin.join(script), "").unwrap();
        }
        python
    }

    fn get<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
        env.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    const CFG: &str = "home = /opt/py/bin\nversion_info = 3.13.1\n";
    const SCRIPTS: [&str; 4] = ["activate", "activate.fish", "activate.ps1", "activate.bat"];

    #[test]
    fn shells_with_integration_source_the_script() {
        let dir = tempfile::tempdir().unwrap();
        let python = venv(dir.path(), CFG, &SCRIPTS);
        let bin = python.parent().unwrap();
        let none = |_: &str| None;
        for (kind, script) in [
            (ShellKind::Zsh, "activate"),
            (ShellKind::Bash, "activate"),
            (ShellKind::Sh, "activate"),
            (ShellKind::Fish, "activate.fish"),
            // uv's name; `Activate.ps1` (Python's venv) is tried first.
            (ShellKind::PowerShell, "ps1"),
        ] {
            let out = activation(Os::current(), &python, kind, true, &none);
            let source = out.source.unwrap_or_else(|| panic!("{kind:?}"));
            assert!(
                source
                    .path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .ends_with(script),
                "{kind:?}: {:?}",
                source.path
            );
            assert_eq!(source.path.parent(), Some(bin));
            assert!(source.args.is_empty());
            assert!(out.env.is_empty(), "{kind:?}");
        }
        // No script for cmd.exe and other shells.
        assert_eq!(activate_script(bin, ShellKind::Cmd), None);
        assert_eq!(activate_script(bin, ShellKind::Other), None);
    }

    #[test]
    fn other_shells_get_the_variables() {
        let dir = tempfile::tempdir().unwrap();
        let python = venv(dir.path(), CFG, &SCRIPTS);
        let venv_dir = dir.path().join(".venv").to_string_lossy().into_owned();
        let bin = python.parent().unwrap().to_string_lossy().into_owned();

        // cmd.exe on Windows: PATH, PROMPT as activate.bat sets them.
        let windows = |name: &str| match name {
            "PATH" => Some("C:\\Windows".to_owned()),
            "PYTHONHOME" => Some("C:\\Py".to_owned()),
            _ => None,
        };
        let out = activation(Os::Windows, &python, ShellKind::Cmd, false, &windows);
        assert_eq!(out.source, None);
        assert_eq!(get(&out.env, "VIRTUAL_ENV"), Some(venv_dir.as_str()));
        let path = format!("{bin};C:\\Windows");
        assert_eq!(get(&out.env, "PATH"), Some(path.as_str()));
        assert_eq!(get(&out.env, "_OLD_VIRTUAL_PATH"), Some("C:\\Windows"));
        assert_eq!(get(&out.env, "PROMPT"), Some("(.venv) $P$G"));
        assert_eq!(get(&out.env, "_OLD_VIRTUAL_PROMPT"), Some("$P$G"));
        assert_eq!(get(&out.env, "PYTHONHOME"), Some(""));
        assert_eq!(get(&out.env, "_OLD_VIRTUAL_PYTHONHOME"), Some("C:\\Py"));
        assert_eq!(get(&out.env, "PS1"), None);

        // An unknown shell elsewhere; the prompt of pyvenv.cfg.
        let dir = tempfile::tempdir().unwrap();
        let python = venv(dir.path(), "prompt = 'demo'\n", &SCRIPTS);
        let unix = |name: &str| match name {
            "PATH" => Some("/usr/bin:/bin".to_owned()),
            "PS1" => Some("$ ".to_owned()),
            _ => None,
        };
        let out = activation(Os::Linux, &python, ShellKind::Other, false, &unix);
        let bin = python.parent().unwrap().to_string_lossy().into_owned();
        let path = format!("{bin}:/usr/bin:/bin");
        assert_eq!(get(&out.env, "PATH"), Some(path.as_str()));
        assert_eq!(get(&out.env, "PS1"), Some("(demo) $ "));
        assert_eq!(get(&out.env, "PROMPT"), None);
        assert_eq!(get(&out.env, "PYTHONHOME"), None);

        // A shell that would source scripts but runs without integration.
        let out = activation(Os::MacOs, &python, ShellKind::Zsh, false, &unix);
        assert_eq!(out.source, None);
        assert!(get(&out.env, "VIRTUAL_ENV").is_some());

        // No prompt change when the person turned it off.
        let disabled = |name: &str| (name == "VIRTUAL_ENV_DISABLE_PROMPT").then(|| "1".to_owned());
        let out = activation(Os::Windows, &python, ShellKind::Cmd, false, &disabled);
        assert_eq!(get(&out.env, "PROMPT"), None);
        assert_eq!(get(&out.env, "PATH"), Some(bin.as_str()));
    }

    #[test]
    fn only_virtual_environments() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let python = bin.join("python");
        std::fs::write(&python, "").unwrap();
        std::fs::write(bin.join("activate"), "").unwrap();
        let none = |_: &str| None;
        let out = activation(Os::Linux, &python, ShellKind::Bash, true, &none);
        assert_eq!(out, Customization::default());
    }

    #[test]
    fn the_setting_is_imported_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // On by default; nothing written.
        assert!(activate_virtualenv(root));
        assert!(!project::store::settings_file(root).exists());

        let idea = root.join(".idea");
        std::fs::create_dir(&idea).unwrap();
        std::fs::write(
            idea.join(IDEA_FILE),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="PyVirtualEnvTerminalCustomizer">
    <option name="virtualEnvActivate" value="false" />
  </component>
</project>
"#,
        )
        .unwrap();
        assert!(!activate_virtualenv(root));
        let stored = project::store::load_value(root, SETTINGS_TABLE, ACTIVATE_KEY).unwrap();
        assert_eq!(stored.and_then(|v| v.as_bool()), Some(false));

        // Turned on in Pyoxide: the default is not written, and the
        // reference's setting is not imported again.
        assert!(set_activate_virtualenv(root, true).unwrap());
        assert!(activate_virtualenv(root));
        assert_eq!(
            project::store::load_value(root, SETTINGS_TABLE, ACTIVATE_KEY).unwrap(),
            None
        );
        assert!(
            project::store::load_table(root, SETTINGS_TABLE)
                .unwrap()
                .is_some()
        );

        // On before any import: the table is kept as well.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".idea")).unwrap();
        std::fs::copy(idea.join(IDEA_FILE), root.join(".idea").join(IDEA_FILE)).unwrap();
        assert!(set_activate_virtualenv(root, true).unwrap());
        assert!(activate_virtualenv(root));
        assert!(set_activate_virtualenv(root, false).unwrap());
        assert!(!activate_virtualenv(root));
    }

    /// A bash session in `cwd` of the project at `root`.
    fn bash_in<'a>(
        command: &'a ::terminal::ShellCommand,
        cwd: &'a Path,
        root: &'a Path,
        integration: bool,
    ) -> ShellContext<'a> {
        ShellContext {
            command,
            kind: ShellKind::Bash,
            working_directory: cwd,
            project_root: Some(root),
            shell_integration: integration,
            sources_scripts: integration,
        }
    }

    #[test]
    fn sessions_in_the_project_activate_its_environment() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let python = venv(root, CFG, &SCRIPTS);
        std::fs::create_dir(root.join("sub")).unwrap();
        let customizer = VirtualEnvCustomizer::default();
        let bash = ::terminal::ShellCommand {
            program: "/bin/bash".into(),
            args: Vec::new(),
        };
        let sub = root.join("sub");
        let out = customizer.customize(&bash_in(&bash, &sub, root, true));
        let source = out.source.expect("the activation");
        assert_eq!(source.path, python.parent().unwrap().join("activate"));

        // Shell integration off, a folder outside the project, the
        // setting off: nothing.
        let nothing = Customization::default();
        assert_eq!(
            customizer.customize(&bash_in(&bash, &sub, root, false)),
            nothing
        );
        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            customizer.customize(&bash_in(&bash, other.path(), root, true)),
            nothing
        );
        set_activate_virtualenv(root, false).unwrap();
        assert_eq!(
            customizer.customize(&bash_in(&bash, root, root, true)),
            nothing
        );
    }
}
