//! Derived keymaps (ADR 0018): a parent keymap and the commands whose
//! bindings differ from it.
//!
//! ```toml
//! version = 1
//! name = "VSCode"
//! parent = "Windows"
//!
//! [[override]]
//! command = "editor.duplicate"
//! context = "Editor"
//! keys = ["shift-alt-down"]
//!
//! [[override]]
//! command = "editor.delete-line"
//! keys = []            # removed
//! ```
//!
//! An override replaces the command's bindings of the parent: in its
//! `context` when it names one, else in every context the parent binds the
//! command in (globally when the parent does not bind it). Arguments are
//! matched when the override names them. A keystroke an override binds
//! takes that keystroke in that context from the parent's other commands,
//! so a keystroke is bound once per context.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::{Binding, Keymap};

/// The format's version.
pub const VERSION: u32 = 1;

/// One command whose bindings differ from the parent's.
#[derive(Debug, Clone, PartialEq)]
pub struct Override {
    pub command: String,
    pub args: Option<Json>,
    /// The key context; `None`: wherever the parent binds the command.
    pub context: Option<String>,
    /// The keystrokes (GPUI syntax, double taps, mouse buttons); empty:
    /// the command loses its bindings.
    pub keys: Vec<String>,
}

/// A keymap made of a parent and overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct Derived {
    pub name: String,
    /// The keymap it is based on, by name.
    pub parent: String,
    pub overrides: Vec<Override>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    name: String,
    parent: String,
    #[serde(default, rename = "override", skip_serializing_if = "Vec::is_empty")]
    overrides: Vec<RawOverride>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOverride {
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    args: Option<toml::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<String>,
    keys: Vec<String>,
}

impl Derived {
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        if file.version != VERSION {
            return Err(format!("unknown version {}", file.version));
        }
        if file.name.trim().is_empty() {
            return Err("the keymap has no name".into());
        }
        let overrides = file
            .overrides
            .into_iter()
            .map(|o| {
                let args = o
                    .args
                    .map(|a| serde_json::to_value(a).map_err(|e| e.to_string()))
                    .transpose()?;
                Ok(Override {
                    command: o.command,
                    args,
                    context: o.context,
                    keys: o.keys,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            name: file.name,
            parent: file.parent,
            overrides,
        })
    }

    /// The keymap in its file format.
    pub fn to_toml(&self) -> String {
        let file = File {
            version: VERSION,
            name: self.name.clone(),
            parent: self.parent.clone(),
            overrides: self
                .overrides
                .iter()
                .map(|o| RawOverride {
                    command: o.command.clone(),
                    args: o.args.as_ref().and_then(|a| toml::Value::try_from(a).ok()),
                    context: o.context.clone(),
                    keys: o.keys.clone(),
                })
                .collect(),
        };
        toml::to_string(&file).expect("a keymap serializes")
    }

    /// The bindings of `parent` with the overrides applied.
    pub fn apply(&self, parent: &Keymap) -> Keymap {
        let mut bindings = parent.bindings.clone();
        // The bindings the overrides add, with their keystrokes in the
        // canonical form.
        let mut added: Vec<(Binding, String)> = Vec::new();
        for o in &self.overrides {
            let matches =
                |b: &Binding| b.command == o.command && (o.args.is_none() || b.args == o.args);
            let contexts: Vec<Option<String>> = match &o.context {
                Some(context) => vec![Some(context.clone())],
                None => {
                    let mut contexts: Vec<Option<String>> = Vec::new();
                    for b in bindings.iter().chain(added.iter().map(|(b, _)| b)) {
                        if matches(b) && !contexts.contains(&b.context) {
                            contexts.push(b.context.clone());
                        }
                    }
                    if contexts.is_empty() {
                        contexts.push(None);
                    }
                    contexts
                }
            };
            bindings.retain(|b| !(matches(b) && contexts.contains(&b.context)));
            added.retain(|(b, _)| !(matches(b) && contexts.contains(&b.context)));
            for keys in &o.keys {
                let canonical = canonical_keys(keys);
                for context in &contexts {
                    // A later override of the same keystroke wins.
                    added.retain(|(b, k)| !(b.context == *context && *k == canonical));
                    let binding = Binding {
                        keys: keys.clone(),
                        command: o.command.clone(),
                        args: o.args.clone(),
                        context: context.clone(),
                    };
                    added.push((binding, canonical.clone()));
                }
            }
        }
        let taken: HashSet<(Option<String>, String)> = added
            .iter()
            .map(|(b, k)| (b.context.clone(), k.clone()))
            .collect();
        bindings.retain(|b| !taken.contains(&(b.context.clone(), canonical_keys(&b.keys))));
        bindings.extend(added.into_iter().map(|(b, _)| b));
        Keymap { bindings }
    }
}

/// The keystrokes in a canonical form (`shift-ctrl-d` and `ctrl-shift-d`
/// are one); double taps and mouse buttons as they are.
pub fn canonical_keys(keys: &str) -> String {
    if super::parse_double_tap(keys).is_some() || super::parse_mouse_button(keys).is_some() {
        return keys.to_owned();
    }
    keys.split(' ')
        .map(|stroke| match gpui::Keystroke::parse(stroke) {
            Ok(k) => k.unparse(),
            Err(_) => stroke.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Two keystroke texts press the same keys.
pub fn same_keys(a: &str, b: &str) -> bool {
    a == b || canonical_keys(a) == canonical_keys(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(keys: &str, command: &str, context: Option<&str>) -> Binding {
        Binding {
            keys: keys.into(),
            command: command.into(),
            args: None,
            context: context.map(str::to_owned),
        }
    }

    fn parent() -> Keymap {
        Keymap {
            bindings: vec![
                binding("cmd-d", "editor.duplicate", Some("Editor")),
                binding("cmd-backspace", "editor.delete-line", Some("Editor")),
                binding("cmd-w", "run.close-tab", Some("RunToolWindow")),
                binding("cmd-w", "run.close-tab", Some("RunConsole > Editor")),
                binding("cmd-w", "tabs.close", None),
                binding("alt-down", "editor.shrink-selection", Some("Editor")),
            ],
        }
    }

    #[test]
    fn parses_and_writes_the_format() {
        let text = r#"
version = 1
name = "Mine"
parent = "macOS"

[[override]]
command = "editor.duplicate"
context = "Editor"
keys = ["shift-alt-down"]

[[override]]
command = "tabs.select"
args = { index = 0 }
keys = ["ctrl-1"]

[[override]]
command = "editor.delete-line"
keys = []
"#;
        let derived = Derived::parse(text).unwrap();
        assert_eq!(derived.name, "Mine");
        assert_eq!(derived.parent, "macOS");
        assert_eq!(derived.overrides.len(), 3);
        assert_eq!(
            derived.overrides[1].args,
            Some(serde_json::json!({"index": 0}))
        );
        assert_eq!(Derived::parse(&derived.to_toml()).unwrap(), derived);
        assert!(Derived::parse("version = 2\nname = \"a\"\nparent = \"b\"\n").is_err());
        assert!(Derived::parse("version = 1\nname = \" \"\nparent = \"b\"\n").is_err());
    }

    #[test]
    fn overrides_replace_remove_and_take_keystrokes() {
        let derived = Derived {
            name: "Mine".into(),
            parent: "macOS".into(),
            overrides: vec![
                // Replaced in its context; the keystroke is taken from
                // Shrink Selection.
                Override {
                    command: "editor.duplicate".into(),
                    args: None,
                    context: Some("Editor".into()),
                    keys: vec!["alt-down".into()],
                },
                // Removed.
                Override {
                    command: "editor.delete-line".into(),
                    args: None,
                    context: None,
                    keys: vec![],
                },
                // Every context the parent binds it in.
                Override {
                    command: "run.close-tab".into(),
                    args: None,
                    context: None,
                    keys: vec!["ctrl-f4".into()],
                },
                // Not bound by the parent: global.
                Override {
                    command: "workbench.synchronize".into(),
                    args: None,
                    context: None,
                    keys: vec!["shift-ctrl-y".into()],
                },
            ],
        };
        let resolved = derived.apply(&parent());
        let has = |keys: &str, command: &str, context: Option<&str>| {
            resolved.bindings.contains(&binding(keys, command, context))
        };
        assert!(has("alt-down", "editor.duplicate", Some("Editor")));
        assert!(!has("cmd-d", "editor.duplicate", Some("Editor")));
        assert!(!has("alt-down", "editor.shrink-selection", Some("Editor")));
        assert!(
            !resolved
                .bindings
                .iter()
                .any(|b| b.command == "editor.delete-line")
        );
        assert!(has("ctrl-f4", "run.close-tab", Some("RunToolWindow")));
        assert!(has("ctrl-f4", "run.close-tab", Some("RunConsole > Editor")));
        assert!(!has("cmd-w", "run.close-tab", Some("RunToolWindow")));
        assert!(has("cmd-w", "tabs.close", None));
        assert!(has("shift-ctrl-y", "workbench.synchronize", None));
        assert!(same_keys("shift-ctrl-y", "ctrl-shift-y"));
        assert!(!same_keys("ctrl-y", "ctrl-shift-y"));
    }
}
