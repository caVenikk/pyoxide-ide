//! Tools > Terminal (`[terminal]`): the options of new terminal sessions
//! (ADR 0014). The start directory and the project's variables are kept
//! per project, in the personal project state.

use serde::{Deserialize, Serialize};

use crate::Section;

/// How new terminal sessions start and how they take keys (the reference's
/// `TerminalOptionsProvider`: `myShellPath`, `myShellIntegration`,
/// `closeSessionOnLogout`, `overrideIdeShortcuts`, `useOptionAsMetaKey`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct TerminalSettings {
    /// "Shell path": a shell, or a command line with its options; empty
    /// for the default shell (`$SHELL` on macOS and Linux, PowerShell on
    /// Windows).
    pub shell_path: String,
    /// "Shell integration": zsh, bash, fish and PowerShell report the
    /// working directory, the prompt and the commands, and the product's
    /// scripts (a virtual environment's `activate`) are sourced before the
    /// first prompt. The reference activates no environment without it.
    pub shell_integration: bool,
    /// "Close session when it ends": the tab of a session whose shell
    /// exited closes.
    pub close_session_on_exit: bool,
    /// "Override IDE shortcuts": a focused terminal takes the keys of IDE
    /// shortcuts, except those of the allowed actions (tool windows, Search
    /// Everywhere, the terminal's own).
    pub override_ide_shortcuts: bool,
    /// "Use Option as Meta key" (macOS): Option with a key sends Escape
    /// and the key instead of the character Option types.
    pub option_as_meta: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell_path: String::new(),
            shell_integration: true,
            close_session_on_exit: true,
            override_ide_shortcuts: true,
            option_as_meta: false,
        }
    }
}

impl Section for TerminalSettings {
    const KEY: &'static str = "terminal";
}

impl TerminalSettings {
    /// The shell path, `None` when empty (the default shell).
    pub fn shell(&self) -> Option<&str> {
        Some(self.shell_path.trim()).filter(|p| !p.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn integration_is_on_by_default() {
        let mut store = Store::new();
        store.register::<TerminalSettings>();
        let settings = store.section::<TerminalSettings>();
        assert!(settings.shell_integration);
        assert_eq!(settings.shell(), None);
        assert_eq!(
            store.get("terminal.shell-integration"),
            Some(serde_json::json!(true))
        );
        let changed = TerminalSettings {
            shell_path: " /bin/zsh -l ".into(),
            shell_integration: false,
        };
        assert!(store.set_section(&changed));
        assert_eq!(store.section::<TerminalSettings>(), changed);
        assert_eq!(changed.shell(), Some("/bin/zsh -l"));
        assert_eq!(
            store.get("terminal.shell-path"),
            Some(serde_json::json!(" /bin/zsh -l "))
        );
    }
}
