//! The action table (`keymaps/actions/`, ADR 0018): the reference
//! IDE's action ids with their texts, their places in the Keymap page's
//! tree and the Pyoxide commands they run, each in its key contexts.
//! Keymaps of the reference are converted through it.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value as Json;

/// The action table that ships: the files of `keymaps/actions/`, one per menu
/// of the Keymap page's tree (`header.toml`, with the version, first), joined
/// in this order, which is the order of the actions within a group.
pub const ACTIONS: &str = concat!(
    include_str!("../../keymaps/actions/header.toml"),
    include_str!("../../keymaps/actions/file.toml"),
    include_str!("../../keymaps/actions/edit.toml"),
    include_str!("../../keymaps/actions/view.toml"),
    include_str!("../../keymaps/actions/navigate.toml"),
    include_str!("../../keymaps/actions/code.toml"),
    include_str!("../../keymaps/actions/refactor.toml"),
    include_str!("../../keymaps/actions/run.toml"),
    include_str!("../../keymaps/actions/git.toml"),
    include_str!("../../keymaps/actions/window.toml"),
    include_str!("../../keymaps/actions/help.toml"),
    include_str!("../../keymaps/actions/editor-actions.toml"),
    include_str!("../../keymaps/actions/other.toml"),
);

/// The tree's group of editor actions, sorted by action id as the
/// reference sorts it.
pub const EDITOR_ACTIONS: &str = "Editor Actions";
/// The top-level groups of the tree in the reference's order
/// (`ActionTreeGroupUtil.createMainGroup`).
pub const TOP_GROUPS: &[&str] = &[EDITOR_ACTIONS, "Main Menu", "Other"];
/// The menus of Main Menu in the reference's order.
pub const MAIN_MENUS: &[&str] = &[
    "File", "Edit", "View", "Navigate", "Code", "Refactor", "Run", "Tools", "Git", "Window", "Help",
];

/// A command an action runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub command: String,
    pub args: Option<Json>,
    /// The key contexts it runs in; `[None]` for a global command.
    pub contexts: Vec<Option<String>>,
}

/// An action of the reference.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    /// The reference's action id (`EditorDuplicate`, `$Copy`).
    pub id: String,
    /// The reference's text (`Duplicate Line or Selection`).
    pub text: String,
    /// Its places in the tree, each a path of group names.
    pub groups: Vec<Vec<String>>,
    pub targets: Vec<Target>,
    /// The action whose shortcuts it has unless a keymap lists it itself
    /// (`use-shortcut-of`: `EditorCopy` has `$Copy`'s); not in the tree.
    pub use_shortcut_of: Option<String>,
}

/// The action table.
#[derive(Debug, Clone, Default)]
pub struct ActionTable {
    pub actions: Vec<Action>,
    by_id: HashMap<String, usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    #[serde(default, rename = "action")]
    actions: Vec<RawAction>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAction {
    id: String,
    text: String,
    #[serde(default)]
    groups: Vec<String>,
    #[serde(rename = "use-shortcut-of")]
    use_shortcut_of: Option<String>,
    command: Option<String>,
    args: Option<toml::Value>,
    context: Option<String>,
    contexts: Option<Vec<String>>,
    #[serde(default)]
    commands: Vec<RawTarget>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    command: String,
    args: Option<toml::Value>,
    context: Option<String>,
    contexts: Option<Vec<String>>,
}

fn target(
    command: String,
    args: Option<toml::Value>,
    context: Option<String>,
    contexts: Option<Vec<String>>,
) -> Result<Target, String> {
    let contexts = match (context, contexts) {
        (Some(_), Some(_)) => return Err(format!("{command}: both `context` and `contexts`")),
        (Some(c), None) => vec![Some(c)],
        (None, Some(cs)) if !cs.is_empty() => cs.into_iter().map(Some).collect(),
        (None, Some(_)) => return Err(format!("{command}: empty `contexts`")),
        (None, None) => vec![None],
    };
    let args = args
        .map(|a| serde_json::to_value(a).map_err(|e| e.to_string()))
        .transpose()?;
    Ok(Target {
        command,
        args,
        contexts,
    })
}

impl ActionTable {
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        if file.version != 1 {
            return Err(format!("unknown version {}", file.version));
        }
        let mut table = Self::default();
        for raw in file.actions {
            let mut targets = Vec::new();
            match raw.command {
                Some(command) if raw.commands.is_empty() => {
                    targets.push(target(command, raw.args, raw.context, raw.contexts)?)
                }
                Some(_) => return Err(format!("{}: both `command` and `commands`", raw.id)),
                None => {
                    if raw.args.is_some() || raw.context.is_some() || raw.contexts.is_some() {
                        return Err(format!(
                            "{}: arguments or contexts without `command`",
                            raw.id
                        ));
                    }
                    for t in raw.commands {
                        targets.push(target(t.command, t.args, t.context, t.contexts)?);
                    }
                }
            }
            if targets.is_empty() {
                return Err(format!("{}: no command", raw.id));
            }
            let groups = raw
                .groups
                .iter()
                .map(|g| g.split(" > ").map(str::to_owned).collect())
                .collect();
            if table.by_id.contains_key(&raw.id) {
                return Err(format!("{}: listed twice", raw.id));
            }
            table.by_id.insert(raw.id.clone(), table.actions.len());
            table.actions.push(Action {
                id: raw.id,
                text: raw.text,
                groups,
                targets,
                use_shortcut_of: raw.use_shortcut_of,
            });
        }
        for action in &table.actions {
            if let Some(base) = &action.use_shortcut_of
                && table.get(base).is_none_or(|b| b.use_shortcut_of.is_some())
            {
                return Err(format!(
                    "{}: `use-shortcut-of` an unknown action",
                    action.id
                ));
            }
        }
        Ok(table)
    }

    /// The table that ships.
    pub fn builtin() -> &'static ActionTable {
        static TABLE: OnceLock<ActionTable> = OnceLock::new();
        TABLE.get_or_init(|| ActionTable::parse(ACTIONS).expect("the action table parses"))
    }

    pub fn get(&self, id: &str) -> Option<&Action> {
        self.by_id.get(id).map(|&ix| &self.actions[ix])
    }

    /// The action whose command `command` is (with `args`, when the
    /// target names arguments).
    pub fn for_command(&self, command: &str, args: Option<&Json>) -> Option<&Action> {
        self.actions.iter().find(|a| {
            a.targets
                .iter()
                .any(|t| t.command == command && (t.args.is_none() || t.args.as_ref() == args))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_parses_with_places_and_contexts() {
        let table = ActionTable::builtin();
        let duplicate = table.get("EditorDuplicate").unwrap();
        assert_eq!(duplicate.text, "Duplicate Line or Selection");
        assert_eq!(
            duplicate.groups,
            vec![
                vec!["Editor Actions".to_owned()],
                vec![
                    "Main Menu".to_owned(),
                    "Edit".to_owned(),
                    "Edit Code Actions".to_owned()
                ]
            ]
        );
        assert_eq!(
            duplicate.targets[0].contexts,
            vec![Some("Editor".to_owned())]
        );
        let copy = table.get("$Copy").unwrap();
        assert_eq!(copy.targets[0].command, "project.tree.copy");
        let editor_copy = table.get("EditorCopy").unwrap();
        assert_eq!(editor_copy.use_shortcut_of.as_deref(), Some("$Copy"));
        assert!(editor_copy.groups.is_empty());
        // A global command.
        assert_eq!(
            table.get("SaveAll").unwrap().targets[0].contexts,
            vec![None]
        );
        assert_eq!(
            table
                .for_command("project.tree.copy", None)
                .map(|a| a.id.as_str()),
            Some("$Copy")
        );
        // Every place starts with a top-level group, and Main Menu's with
        // one of its menus.
        for action in &table.actions {
            for path in &action.groups {
                assert!(TOP_GROUPS.contains(&path[0].as_str()), "{}", action.id);
                if path[0] == "Main Menu" {
                    assert!(MAIN_MENUS.contains(&path[1].as_str()), "{}", action.id);
                }
            }
        }
    }

    #[test]
    fn bad_tables_are_refused() {
        let bad = [
            "version = 2\n",
            "version = 1\n[[action]]\nid = \"A\"\ntext = \"A\"\ngroups = []\n",
            "version = 1\n[[action]]\nid = \"A\"\ntext = \"A\"\ngroups = []\ncommand = \"a\"\ncontext = \"X\"\ncontexts = [\"Y\"]\n",
            "version = 1\n[[action]]\nid = \"A\"\ntext = \"A\"\ngroups = []\ncommand = \"a\"\n[[action]]\nid = \"A\"\ntext = \"A\"\ngroups = []\ncommand = \"b\"\n",
            "version = 1\n[[action]]\nid = \"A\"\ntext = \"A\"\nuse-shortcut-of = \"B\"\ncommand = \"a\"\n",
        ];
        for text in bad {
            assert!(ActionTable::parse(text).is_err(), "{text}");
        }
    }
}
