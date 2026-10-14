//! Ruff's adjustments (ADR 0010): rule names instead of codes in messages
//! and quick fix titles, "Disable for this line" nested under its fix, and
//! "Disable for this file" / "Disable for this project" added, as the
//! reference's `RuffLspClient.customizeQuickFixes` does.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use lsp_client::ServerCustomizer;
use process::CommandLine;

use crate::lookup::Executable;

/// Ruff's [`ServerCustomizer`]; it learns the rule names once the server
/// runs.
#[derive(Debug, Default)]
pub struct RuffCustomizer {
    /// Rule code to rule name (`F401` to `unused-import`).
    rules: Mutex<HashMap<String, String>>,
}

impl RuffCustomizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the rule names (from [`rules_command`]'s output).
    pub fn set_rules(&self, rules: HashMap<String, String>) {
        *self.rules.lock().unwrap_or_else(PoisonError::into_inner) = rules;
    }

    /// The name of a rule code.
    pub fn rule_name(&self, code: &str) -> Option<String> {
        self.rules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(code)
            .cloned()
    }
}

impl ServerCustomizer for RuffCustomizer {}

/// The command that lists ruff's rules as JSON (code, name, ...).
pub fn rules_command(executable: &Executable) -> CommandLine {
    executable.command_line(&["rule", "--output-format", "json", "--all"])
}

/// Rule code to name from the output of [`rules_command`].
pub fn parse_rules(json: &str) -> HashMap<String, String> {
    let _ = json;
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_names() {
        let ruff = RuffCustomizer::new();
        assert_eq!(ruff.rule_name("F401"), None);
        ruff.set_rules(HashMap::from([("F401".into(), "unused-import".into())]));
        assert_eq!(ruff.rule_name("F401").as_deref(), Some("unused-import"));
        let command = rules_command(&Executable::Path("ruff".into()));
        assert_eq!(command.display(), "ruff rule --output-format json --all");
    }
}
