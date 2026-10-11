//! Keymaps (ADR 0007): TOML files that bind keystrokes to command ids, per
//! key context. Two default keymaps ship, one for macOS and one for Windows
//! and Linux; their keystrokes follow the reference IDE's default keymaps
//! (the IntelliJ Platform Community keymap files, Apache-2.0, see NOTICE).
//!
//! ```toml
//! [[context]]
//! name = "Editor"          # a GPUI key context predicate; omit for global
//! [context.bindings]
//! "cmd-z" = "editor.undo"
//! "cmd-alt-1" = { command = "tabs.select", args = { index = 0 } }
//! ```
//!
//! Keystrokes use GPUI's syntax (`cmd-shift-z`, `alt-left`, `ctrl-f4`).

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::{App, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate};
use serde::Deserialize;
use serde_json::Value as Json;

/// The default keymap of macOS.
pub const MACOS: &str = include_str!("../keymaps/macos.toml");
/// The default keymap of Windows and Linux.
pub const DEFAULT: &str = include_str!("../keymaps/default.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default, rename = "context")]
    contexts: Vec<Section>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Section {
    name: Option<String>,
    #[serde(default)]
    bindings: BTreeMap<String, Target>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Target {
    Command(String),
    WithArgs { command: String, args: toml::Value },
}

/// One keystroke binding.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    /// Keystrokes, space-separated for sequences.
    pub keys: String,
    pub command: String,
    pub args: Option<Json>,
    /// Key context predicate; `None` binds everywhere.
    pub context: Option<String>,
}

/// A parsed keymap.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Keymap {
    pub bindings: Vec<Binding>,
}

impl Keymap {
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut bindings = Vec::new();
        for section in file.contexts {
            for (keys, target) in section.bindings {
                let (command, args) = match target {
                    Target::Command(command) => (command, None),
                    Target::WithArgs { command, args } => {
                        let args = serde_json::to_value(args).map_err(|e| e.to_string())?;
                        (command, Some(args))
                    }
                };
                bindings.push(Binding {
                    keys,
                    command,
                    args,
                    context: section.name.clone(),
                });
            }
        }
        Ok(Self { bindings })
    }

    /// The default keymap of the platform the IDE runs on.
    pub fn platform_default() -> Self {
        let text = if cfg!(target_os = "macos") {
            MACOS
        } else {
            DEFAULT
        };
        Self::parse(text).expect("the default keymap parses")
    }

    /// Commands bound to a keystroke text, for display (first binding).
    pub fn keys_for(&self, command: &str) -> Option<&str> {
        self.bindings
            .iter()
            .find(|b| b.command == command && b.args.is_none())
            .map(|b| b.keys.as_str())
    }
}

/// Installs a keymap. Returns the bindings that could not be installed
/// (unknown command, bad arguments, bad keystroke or context).
pub fn bind(cx: &mut App, keymap: &Keymap) -> Vec<String> {
    let mut errors = Vec::new();
    let mut out = Vec::new();
    for b in &keymap.bindings {
        let action = match cx.build_action(&b.command, b.args.clone()) {
            Ok(action) => action,
            Err(e) => {
                errors.push(format!("{} -> {}: {e}", b.keys, b.command));
                continue;
            }
        };
        let predicate = match &b.context {
            Some(context) => match KeyBindingContextPredicate::parse(context) {
                Ok(p) => Some(Rc::new(p)),
                Err(e) => {
                    errors.push(format!("context `{context}`: {e}"));
                    continue;
                }
            },
            None => None,
        };
        match KeyBinding::load(
            &b.keys,
            action,
            predicate,
            false,
            None,
            &DummyKeyboardMapper,
        ) {
            Ok(binding) => out.push(binding),
            Err(e) => errors.push(format!("{}: {e}", b.keys)),
        }
    }
    cx.bind_keys(out);
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_contexts_commands_and_arguments() {
        let keymap = Keymap::parse(
            r#"
[[context]]
[context.bindings]
"cmd-q" = "app.quit"

[[context]]
name = "Editor"
[context.bindings]
"cmd-z" = "editor.undo"
"cmd-alt-1" = { command = "tabs.select", args = { index = 0 } }
"#,
        )
        .unwrap();
        assert_eq!(keymap.bindings.len(), 3);
        assert_eq!(keymap.bindings[0].context, None);
        let select = keymap
            .bindings
            .iter()
            .find(|b| b.command == "tabs.select")
            .unwrap();
        assert_eq!(select.args, Some(serde_json::json!({"index": 0})));
        assert_eq!(select.context.as_deref(), Some("Editor"));
        assert_eq!(keymap.keys_for("editor.undo"), Some("cmd-z"));
        assert!(Keymap::parse("[[context]]\nnope = 1\n").is_err());
    }

    #[test]
    fn default_keymaps_parse() {
        for text in [MACOS, DEFAULT] {
            let keymap = Keymap::parse(text).unwrap();
            assert!(keymap.bindings.len() > 50);
        }
    }
}
