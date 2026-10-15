//! What the Run/Debug Configurations dialog edits, without a toolkit: the
//! fields of a configuration's type and its "Modify options" menu, as the
//! reference's fragmented editors show them (`SettingsEditorFragment`,
//! `RunConfigurationFragmentedEditor`): a field is shown by default, or
//! when its option is on in "Modify options"; options set to their default
//! are hidden. A type describes its form with a [`TypeEditor`]; the
//! dialog draws it and writes values back through it.

use std::collections::BTreeSet;

use crate::model::{EnvVar, RunConfiguration, Storage};
use crate::paths;

/// How a field is edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    /// A line of text.
    Text,
    /// A path, relative to the project or absolute.
    Path,
    /// One of the values, with their labels.
    Choice(Vec<(String, String)>),
    /// A check box.
    Flag,
    /// The configuration's environment variables (`NAME=value;NAME2=value2`).
    Env,
}

/// A field of the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// What [`TypeEditor::set`] takes.
    pub id: String,
    pub label: String,
    pub kind: FieldKind,
    /// The value as text (`true` or `false` for a flag).
    pub value: String,
    /// Shown while the field is empty.
    pub placeholder: Option<String>,
    /// A note under the field (a path variable Pyoxide does not expand on
    /// another machine).
    pub note: Option<String>,
    /// In the head of the form, without a label: the reference's fragments
    /// above the labelled rows (interpreter, script, parameters), whose
    /// label is a hint shown while Alt is held.
    pub head: bool,
}

impl Field {
    pub fn new(id: &str, label: &str, kind: FieldKind, value: impl Into<String>) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            kind,
            value: value.into(),
            placeholder: None,
            note: None,
            head: false,
        }
    }

    /// In the head of the form (see [`Field::head`]).
    pub fn head(mut self) -> Self {
        self.head = true;
        self
    }

    pub fn placeholder(mut self, text: &str) -> Self {
        self.placeholder = Some(text.to_owned());
        self
    }

    /// A path field notes `$USER_HOME$` and unknown path variables.
    fn noted(mut self) -> Self {
        if let Some(var) = paths::flagged_variable(&self.value) {
            self.note = Some(format!("{var} is kept as written"));
        }
        self
    }
}

/// A row of the "Modify options" menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionItem {
    /// The group title ("Operating System", "Python", "Before Launch").
    pub group: String,
    pub id: String,
    pub label: String,
    pub checked: bool,
    /// A flag shown under the form as a tag with a remove button while it
    /// is on (the reference's tag fragments).
    pub tag: bool,
    /// Pyoxide cannot do it yet: shown, not chosen.
    pub enabled: bool,
}

impl OptionItem {
    pub fn new(group: &str, id: &str, label: &str, checked: bool) -> Self {
        Self {
            group: group.to_owned(),
            id: id.to_owned(),
            label: label.to_owned(),
            checked,
            tag: false,
            enabled: true,
        }
    }

    /// A row of the reference Pyoxide does not support yet.
    pub fn unavailable(group: &str, id: &str, label: &str) -> Self {
        Self {
            enabled: false,
            ..Self::new(group, id, label, false)
        }
    }

    /// Shown as a tag while it is on (see [`OptionItem::tag`]).
    pub fn tag(mut self) -> Self {
        self.tag = true;
        self
    }
}

/// The form of a configuration type.
pub trait TypeEditor: Send + Sync {
    /// The type it edits.
    fn type_id(&self) -> &'static str;

    /// A new configuration of the type with its template's defaults.
    fn create(&self, name: &str, storage: Storage) -> RunConfiguration;

    /// The fields shown for `config`, in order; `shown` holds the options
    /// turned on in this dialog session.
    fn fields(&self, config: &RunConfiguration, shown: &BTreeSet<String>) -> Vec<Field>;

    /// The type's rows of "Modify options".
    fn options(&self, config: &RunConfiguration, shown: &BTreeSet<String>) -> Vec<OptionItem>;

    /// Sets a field; false for a field the type does not have.
    fn set(&self, config: &mut RunConfiguration, field: &str, value: &str) -> bool;

    /// Turns an option on or off: a flag changes the configuration, a
    /// field option shows or hides its field (`shown`).
    fn toggle(&self, config: &mut RunConfiguration, option: &str, shown: &mut BTreeSet<String>);

    /// What keeps the configuration from running, in the reference's words.
    fn validate(&self, config: &RunConfiguration) -> Option<String>;
}

/// Field and option ids every type has, handled by the dialog.
pub mod common {
    pub const ENV: &str = "env";
    pub const ALLOW_PARALLEL: &str = "allow-parallel";
    pub const ACTIVATE_TOOL_WINDOW: &str = "activate-tool-window";
    pub const FOCUS_TOOL_WINDOW: &str = "focus-tool-window";
    pub const EDIT_BEFORE_RUN: &str = "edit-before-run";
    pub const INCLUDE_SYSTEM_ENV: &str = "include-system-env";
}

/// Group titles of "Modify options" (`ExecutionBundle`).
pub mod group {
    pub const OPERATING_SYSTEM: &str = "Operating System";
    pub const LOGS: &str = "Logs";
    pub const BEFORE_LAUNCH: &str = "Before Launch";
}

/// The "Modify options" rows every type has (`CommonTags.parallelRun`,
/// `LogsGroupFragment`, `BeforeRunFragment`): Allow multiple instances
/// first, the Logs and Before Launch groups last; a type puts its own rows
/// between.
pub fn common_options(config: &RunConfiguration, allow_parallel_default: bool) -> Vec<OptionItem> {
    let mut options = common_head_options(config, allow_parallel_default);
    options.extend(common_tail_options(config));
    options
}

/// Allow multiple instances, the first row of Operating System.
pub fn common_head_options(
    config: &RunConfiguration,
    allow_parallel_default: bool,
) -> Vec<OptionItem> {
    vec![
        OptionItem::new(
            group::OPERATING_SYSTEM,
            common::ALLOW_PARALLEL,
            "Allow multiple instances",
            config.allow_parallel.unwrap_or(allow_parallel_default),
        )
        .tag(),
    ]
}

/// The Logs group (not supported yet) and Before Launch.
pub fn common_tail_options(config: &RunConfiguration) -> Vec<OptionItem> {
    vec![
        OptionItem::unavailable(
            group::LOGS,
            "logs",
            "Specify logs to be shown in the console",
        ),
        OptionItem::unavailable(group::LOGS, "save-output", "Save console output to file"),
        OptionItem::unavailable(
            group::LOGS,
            "show-on-stdout",
            "Show console when a message is printed to stdout",
        ),
        OptionItem::unavailable(
            group::LOGS,
            "show-on-stderr",
            "Show console when a message is printed to stderr",
        ),
        OptionItem::unavailable(
            group::BEFORE_LAUNCH,
            "before-launch-task",
            "Add before launch task",
        ),
        OptionItem::new(
            "Before Launch",
            common::ACTIVATE_TOOL_WINDOW,
            "Open run/debug tool window when started",
            config.activate_tool_window,
        )
        .tag(),
        OptionItem::new(
            "Before Launch",
            common::FOCUS_TOOL_WINDOW,
            "Focus run/debug tool window when started",
            config.focus_tool_window,
        )
        .tag(),
        OptionItem::new(
            "Before Launch",
            common::EDIT_BEFORE_RUN,
            "Show the run/debug configuration settings before start",
            config.edit_before_run,
        )
        .tag(),
    ]
}

/// Turns a common option over; false when `option` is not one.
pub fn toggle_common(
    config: &mut RunConfiguration,
    option: &str,
    allow_parallel_default: bool,
) -> bool {
    match option {
        common::INCLUDE_SYSTEM_ENV => config.include_system_env = !config.include_system_env,
        common::ACTIVATE_TOOL_WINDOW => config.activate_tool_window = !config.activate_tool_window,
        common::FOCUS_TOOL_WINDOW => config.focus_tool_window = !config.focus_tool_window,
        common::EDIT_BEFORE_RUN => config.edit_before_run = !config.edit_before_run,
        common::ALLOW_PARALLEL => {
            let on = config.allow_parallel.unwrap_or(allow_parallel_default);
            config.allow_parallel = Some(!on);
        }
        _ => return false,
    }
    true
}

/// The variables as the environment field shows them: `NAME=value`
/// separated by `;`; a variable not set on this machine shows its name
/// only.
pub fn env_text(env: &[EnvVar]) -> String {
    env.iter()
        .map(|v| match &v.value {
            Some(value) => format!("{}={}", v.name, escape_env(value)),
            None => v.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn escape_env(value: &str) -> String {
    value.replace('\\', "\\\\").replace(';', "\\;")
}

/// Reads the environment field back: `;` separates (`\;` is a literal
/// one), the first `=` ends the name. A variable that was there keeps
/// whether its value is shared; a new one is personal in a shared
/// configuration (ADR 0012) and shared with nobody in a personal one.
pub fn parse_env(text: &str, old: &[EnvVar]) -> Vec<EnvVar> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(next) => current.push(next),
                None => current.push('\\'),
            },
            ';' => parts.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    parts.push(current);
    let mut out: Vec<EnvVar> = Vec::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (name, value) = match part.split_once('=') {
            Some((n, v)) => (n.trim(), Some(v.to_owned())),
            None => (part, None),
        };
        if name.is_empty() {
            continue;
        }
        let previous = old.iter().find(|v| v.name == name);
        let shared = previous.is_some_and(|v| v.shared);
        // A name without a value keeps a value it had ("not set" stays so).
        let value = value.or_else(|| previous.and_then(|v| v.value.clone()));
        out.retain(|v| v.name != name);
        out.push(EnvVar {
            name: name.to_owned(),
            value,
            shared,
        });
    }
    out
}

/// The Shell Script type's form (`ShRunConfigurationEditor`).
pub struct ShellScriptEditor;

impl TypeEditor for ShellScriptEditor {
    fn type_id(&self) -> &'static str {
        crate::shell::TYPE_ID
    }

    fn create(&self, name: &str, storage: Storage) -> RunConfiguration {
        use crate::shell::key;
        let mut config = RunConfiguration::new(crate::shell::TYPE_ID, name, storage);
        config.set_bool(key::EXECUTE_SCRIPT_FILE, true);
        config.set_bool(key::EXECUTE_IN_TERMINAL, true);
        config.set_str(key::WORKING_DIRECTORY, paths::PROJECT);
        config
    }

    fn fields(&self, config: &RunConfiguration, _: &BTreeSet<String>) -> Vec<Field> {
        use crate::shell::key;
        let file = config.bool_option(key::EXECUTE_SCRIPT_FILE).unwrap_or(true);
        let text = |k: &str| config.str_option(k).to_owned();
        let mut fields = vec![Field::new(
            key::EXECUTE_SCRIPT_FILE,
            "Execute:",
            FieldKind::Choice(vec![
                ("true".into(), "Script file".into()),
                ("false".into(), "Script text".into()),
            ]),
            file.to_string(),
        )];
        if file {
            fields.push(
                Field::new(
                    key::SCRIPT,
                    "Script path:",
                    FieldKind::Path,
                    text(key::SCRIPT),
                )
                .noted(),
            );
            fields.push(Field::new(
                key::SCRIPT_OPTIONS,
                "Script options:",
                FieldKind::Text,
                text(key::SCRIPT_OPTIONS),
            ));
        } else {
            fields.push(Field::new(
                key::SCRIPT_TEXT,
                "Script text:",
                FieldKind::Text,
                text(key::SCRIPT_TEXT),
            ));
        }
        fields.push(
            Field::new(
                key::WORKING_DIRECTORY,
                "Working directory:",
                FieldKind::Path,
                text(key::WORKING_DIRECTORY),
            )
            .noted(),
        );
        fields.push(Field::new(
            common::ENV,
            "Environment variables:",
            FieldKind::Env,
            env_text(&config.env),
        ));
        if file {
            fields.push(
                Field::new(
                    key::INTERPRETER,
                    "Interpreter path:",
                    FieldKind::Path,
                    text(key::INTERPRETER),
                )
                .noted(),
            );
            fields.push(Field::new(
                key::INTERPRETER_OPTIONS,
                "Interpreter options:",
                FieldKind::Text,
                text(key::INTERPRETER_OPTIONS),
            ));
        }
        fields.push(Field::new(
            key::EXECUTE_IN_TERMINAL,
            "Execute in the terminal",
            FieldKind::Flag,
            config
                .bool_option(key::EXECUTE_IN_TERMINAL)
                .unwrap_or(true)
                .to_string(),
        ));
        fields
    }

    fn options(&self, config: &RunConfiguration, _: &BTreeSet<String>) -> Vec<OptionItem> {
        common_options(config, false)
    }

    fn set(&self, config: &mut RunConfiguration, field: &str, value: &str) -> bool {
        use crate::shell::key;
        match field {
            key::EXECUTE_SCRIPT_FILE | key::EXECUTE_IN_TERMINAL => {
                config.set_bool(field, value == "true")
            }
            key::SCRIPT | key::WORKING_DIRECTORY | key::INTERPRETER => {
                config.set_str(field, &stored_path(value))
            }
            key::SCRIPT_TEXT | key::SCRIPT_OPTIONS | key::INTERPRETER_OPTIONS => {
                config.set_str(field, value)
            }
            _ => return false,
        }
        true
    }

    fn toggle(&self, config: &mut RunConfiguration, option: &str, _: &mut BTreeSet<String>) {
        toggle_common(config, option, false);
    }

    fn validate(&self, config: &RunConfiguration) -> Option<String> {
        use crate::shell::key;
        let file = config.bool_option(key::EXECUTE_SCRIPT_FILE).unwrap_or(true);
        (file && config.str_option(key::SCRIPT).trim().is_empty())
            .then(|| "Shell script not found".to_owned())
    }
}

/// A path typed in a field, in the stored form: `/` separators, relative
/// to the project when it is written so already.
pub fn stored_path(value: &str) -> String {
    value.trim().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_text_round_trips() {
        let old = vec![
            EnvVar::new("SHARED", "1", true),
            EnvVar {
                name: "NOT_HERE".into(),
                value: None,
                shared: false,
            },
        ];
        let text = env_text(&old);
        assert_eq!(text, "SHARED=1;NOT_HERE");
        let parsed = parse_env("SHARED=2; NEW=a\\;b ;NOT_HERE;;=x", &old);
        assert_eq!(
            parsed
                .iter()
                .map(|v| (v.name.as_str(), v.value.as_deref(), v.shared))
                .collect::<Vec<_>>(),
            [
                ("SHARED", Some("2"), true),
                ("NEW", Some("a;b"), false),
                ("NOT_HERE", None, false)
            ]
        );
        assert_eq!(env_text(&parsed), "SHARED=2;NEW=a\\;b;NOT_HERE");
    }

    #[test]
    fn the_shell_script_form() {
        let e = ShellScriptEditor;
        let mut c = e.create("build", Storage::Personal);
        let ids = |c: &RunConfiguration| -> Vec<String> {
            e.fields(c, &BTreeSet::new())
                .into_iter()
                .map(|f| f.id)
                .collect()
        };
        assert_eq!(
            ids(&c),
            [
                "execute-script-file",
                "script",
                "script-options",
                "working-directory",
                "env",
                "interpreter",
                "interpreter-options",
                "execute-in-terminal"
            ]
        );
        assert_eq!(e.validate(&c).as_deref(), Some("Shell script not found"));
        assert!(e.set(&mut c, "script", "scripts\\build.sh"));
        assert_eq!(c.str_option("script"), "scripts/build.sh");
        assert_eq!(e.validate(&c), None);
        assert!(e.set(&mut c, "execute-script-file", "false"));
        assert_eq!(
            ids(&c),
            [
                "execute-script-file",
                "script-text",
                "working-directory",
                "env",
                "execute-in-terminal"
            ]
        );
        assert!(!e.set(&mut c, "nope", "x"));
        let options = e.options(&c, &BTreeSet::new());
        assert!(
            options
                .iter()
                .any(|o| o.id == "allow-parallel" && !o.checked)
        );
        e.toggle(&mut c, "allow-parallel", &mut BTreeSet::new());
        assert_eq!(c.allow_parallel, Some(true));
        e.set(&mut c, "working-directory", "$USER_HOME$/x");
        let wd = e
            .fields(&c, &BTreeSet::new())
            .into_iter()
            .find(|f| f.id == "working-directory")
            .unwrap();
        assert_eq!(wd.note.as_deref(), Some("$USER_HOME$ is kept as written"));
    }
}
