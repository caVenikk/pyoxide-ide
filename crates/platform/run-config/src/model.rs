//! The language-neutral model of a run configuration.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Prefix of the type of a configuration that Pyoxide keeps from the
/// reference IDE without supporting it: `idea:tests` for the reference's
/// pytest configurations.
pub const UNSUPPORTED_PREFIX: &str = "idea:";

/// What identifies a configuration in a project: its type and its name.
/// Names are unique within a type, as in the reference (whose `uniqueID`
/// is `"<type display name>.<name>"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConfigKey {
    #[serde(rename = "type")]
    pub type_id: String,
    pub name: String,
}

impl ConfigKey {
    pub fn new(type_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            type_id: type_id.into(),
            name: name.into(),
        }
    }
}

impl fmt::Display for ConfigKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} '{}'", self.type_id, self.name)
    }
}

/// Where a configuration is stored (ADR 0012): with the project, meant to
/// be committed ("Store as project file"), or for this person only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Storage {
    Shared,
    Personal,
}

/// An environment variable of a configuration. Its value may be a secret:
/// `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct EnvVar {
    pub name: String,
    /// `None`: the variable is in a shared configuration, its value is
    /// personal and this machine has none ("not set on this machine"); it
    /// is not passed to the process.
    pub value: Option<String>,
    /// The value is stored with a shared configuration. Otherwise it stays
    /// in the personal store, also when the configuration is shared.
    pub shared: bool,
}

impl EnvVar {
    pub fn new(name: impl Into<String>, value: impl Into<String>, shared: bool) -> Self {
        Self {
            name: name.into(),
            value: Some(value.into()),
            shared,
        }
    }
}

impl fmt::Debug for EnvVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvVar")
            .field("name", &self.name)
            .field(
                "value",
                &match self.value {
                    Some(_) => "<redacted>",
                    None => "<not set>",
                },
            )
            .field("shared", &self.shared)
            .finish()
    }
}

/// A task to run before launch, kept as the reference wrote it: Pyoxide
/// stores and shows these but does not run them yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeforeLaunchTask {
    /// The reference's provider id (`Make`, `BuildArtifacts`, `RunConfigurationTask`).
    pub provider: String,
    pub enabled: bool,
    /// The task's element (`<option name="…" …/>`) as written.
    pub xml: String,
}

/// A configuration of a type Pyoxide does not support yet, kept from the
/// reference as it was written.
#[derive(Clone, PartialEq, Eq)]
pub struct Reference {
    /// The reference's type id (`tests`, `docker-deploy`).
    pub type_id: String,
    pub factory: Option<String>,
    /// The type's name in the reference (`Python tests`); `None` for a
    /// type Pyoxide does not know (listed last, as the reference lists
    /// configurations of unknown types).
    pub type_name: Option<String>,
    /// The `<configuration>` element, byte for byte.
    pub xml: String,
}

impl fmt::Debug for Reference {
    // The element may hold environment values.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reference")
            .field("type_id", &self.type_id)
            .field("factory", &self.factory)
            .field("type_name", &self.type_name)
            .field("xml", &format_args!("<{} bytes>", self.xml.len()))
            .finish()
    }
}

/// A run configuration: what every type has, and the options of its type.
#[derive(Debug, Clone, PartialEq)]
pub struct RunConfiguration {
    pub name: String,
    /// The type: a registered [`TypeInfo::id`] (`python`, `shell-script`),
    /// or [`UNSUPPORTED_PREFIX`] and the reference's type id.
    pub type_id: String,
    /// The folder it is grouped in (`folderName`).
    pub folder: Option<String>,
    pub storage: Storage,
    /// A shared configuration's file in the shared folder (`server.toml`);
    /// chosen when it is first written.
    pub file: Option<String>,
    /// Created by a context run; at most a few are kept (personal only).
    pub temporary: bool,
    /// "Allow multiple instances"; `None` keeps the type's default.
    pub allow_parallel: Option<bool>,
    /// "Include system environment variables" (`PARENT_ENVS`).
    pub include_system_env: bool,
    pub env: Vec<EnvVar>,
    /// The type's options, as its type stores them.
    pub options: toml::Table,
    pub before_launch: Vec<BeforeLaunchTask>,
    /// "Open run/debug tool window when started" (default on).
    pub activate_tool_window: bool,
    /// "Focus run/debug tool window when started".
    pub focus_tool_window: bool,
    /// "Show the run/debug configuration settings before start".
    pub edit_before_run: bool,
    /// The name was generated from the context (`nameIsGenerated`).
    pub name_is_generated: bool,
    /// For a type Pyoxide does not support: the reference's configuration.
    pub reference: Option<Reference>,
    /// Keys of the stored form this version does not know, kept as they
    /// are.
    pub extra: toml::Table,
    /// Stored by a newer version of Pyoxide: shown, never written.
    pub newer: bool,
}

impl RunConfiguration {
    /// A configuration of `type_id` with the defaults of a new one.
    pub fn new(type_id: impl Into<String>, name: impl Into<String>, storage: Storage) -> Self {
        Self {
            name: name.into(),
            type_id: type_id.into(),
            folder: None,
            storage,
            file: None,
            temporary: false,
            allow_parallel: None,
            include_system_env: true,
            env: Vec::new(),
            options: toml::Table::new(),
            before_launch: Vec::new(),
            activate_tool_window: true,
            focus_tool_window: false,
            edit_before_run: false,
            name_is_generated: false,
            reference: None,
            extra: toml::Table::new(),
            newer: false,
        }
    }

    pub fn key(&self) -> ConfigKey {
        ConfigKey::new(&self.type_id, &self.name)
    }

    /// The type is not supported: the configuration cannot run and is not
    /// edited.
    pub fn is_unsupported(&self) -> bool {
        self.reference.is_some() || self.type_id.starts_with(UNSUPPORTED_PREFIX)
    }

    pub fn is_shared(&self) -> bool {
        self.storage == Storage::Shared
    }

    /// The environment passed to the process, in order: the variables
    /// with a value on this machine.
    pub fn env_values(&self) -> impl Iterator<Item = (&str, &str)> {
        self.env
            .iter()
            .filter_map(|v| v.value.as_deref().map(|value| (v.name.as_str(), value)))
    }

    /// Option values by key, with a default.
    pub fn str_option(&self, key: &str) -> &str {
        self.options
            .get(key)
            .and_then(toml::Value::as_str)
            .unwrap_or("")
    }

    pub fn bool_option(&self, key: &str) -> Option<bool> {
        self.options.get(key).and_then(toml::Value::as_bool)
    }

    pub fn list_option(&self, key: &str) -> Vec<String> {
        self.options
            .get(key)
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Sets a string option; an empty one is removed.
    pub fn set_str(&mut self, key: &str, value: &str) {
        if value.is_empty() {
            self.options.remove(key);
        } else {
            self.options
                .insert(key.to_owned(), toml::Value::String(value.to_owned()));
        }
    }

    pub fn set_bool(&mut self, key: &str, value: bool) {
        self.options
            .insert(key.to_owned(), toml::Value::Boolean(value));
    }

    /// Sets a list option; an empty one is removed.
    pub fn set_list(&mut self, key: &str, values: &[String]) {
        if values.is_empty() {
            self.options.remove(key);
        } else {
            self.options.insert(
                key.to_owned(),
                toml::Value::Array(
                    values
                        .iter()
                        .map(|v| toml::Value::String(v.clone()))
                        .collect(),
                ),
            );
        }
    }
}

/// A run configuration type a product registers (ADR 0005): the platform
/// registers Shell Script, the Python product registers Python.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeInfo {
    /// Pyoxide's id (`python`), as stored.
    pub id: &'static str,
    /// The type's name in lists and the dialog (`Python`).
    pub name: &'static str,
    /// The type's icon (an icon name of the UI layer).
    pub icon: &'static str,
    /// The reference's type id (`PythonConfigurationType`).
    pub reference_id: &'static str,
    /// Configurations of the type may run in parallel by default.
    pub allow_parallel: bool,
}

/// The types of a product, for names, icons and order.
#[derive(Debug, Clone, Default)]
pub struct Types(Vec<TypeInfo>);

impl Types {
    pub fn new(types: impl IntoIterator<Item = TypeInfo>) -> Self {
        Self(types.into_iter().collect())
    }

    pub fn get(&self, id: &str) -> Option<&TypeInfo> {
        self.0.iter().find(|t| t.id == id)
    }

    pub fn all(&self) -> &[TypeInfo] {
        &self.0
    }

    /// The display name of a configuration's type: the registered name,
    /// the reference's name of an unsupported type, or its id.
    pub fn name_of<'a>(&'a self, config: &'a RunConfiguration) -> &'a str {
        if let Some(info) = self.get(&config.type_id) {
            return info.name;
        }
        match &config.reference {
            Some(r) => r.type_name.as_deref().unwrap_or(&r.type_id),
            None => config
                .type_id
                .strip_prefix(UNSUPPORTED_PREFIX)
                .unwrap_or(&config.type_id),
        }
    }

    /// The type is unknown: neither registered nor known by name from the
    /// reference. Such configurations are listed last.
    pub fn is_unknown(&self, config: &RunConfiguration) -> bool {
        self.get(&config.type_id).is_none()
            && config
                .reference
                .as_ref()
                .is_none_or(|r| r.type_name.is_none())
    }

    /// The icon of a configuration's type; `None` for an unknown type.
    pub fn icon_of(&self, config: &RunConfiguration) -> Option<&'static str> {
        self.get(&config.type_id).map(|t| t.icon)
    }

    /// The configuration may run while it runs already.
    pub fn allows_parallel(&self, config: &RunConfiguration) -> bool {
        config
            .allow_parallel
            .unwrap_or_else(|| self.get(&config.type_id).is_some_and(|t| t.allow_parallel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_values_are_redacted_in_debug_output() {
        let mut config = RunConfiguration::new("python", "server", Storage::Shared);
        config.env = vec![
            EnvVar::new("TOKEN", "s3cr3t-value", false),
            EnvVar {
                name: "OTHER".into(),
                value: None,
                shared: false,
            },
        ];
        config.reference = Some(Reference {
            type_id: "tests".into(),
            factory: None,
            type_name: None,
            xml: "<envs><env name=\"A\" value=\"hidden-value\"/></envs>".into(),
        });
        let text = format!("{config:?}");
        assert!(!text.contains("s3cr3t-value"), "{text}");
        assert!(!text.contains("hidden-value"), "{text}");
        assert!(
            text.contains("TOKEN") && text.contains("<redacted>") && text.contains("<not set>")
        );
        assert_eq!(
            config.env_values().collect::<Vec<_>>(),
            [("TOKEN", "s3cr3t-value")]
        );
    }

    #[test]
    fn type_names_and_unknown_types() {
        let types = Types::new([TypeInfo {
            id: "python",
            name: "Python",
            icon: "python/file",
            reference_id: "PythonConfigurationType",
            allow_parallel: false,
        }]);
        let python = RunConfiguration::new("python", "a", Storage::Personal);
        assert_eq!(types.name_of(&python), "Python");
        assert!(!types.is_unknown(&python));
        let mut tests = RunConfiguration::new("idea:tests", "b", Storage::Personal);
        tests.reference = Some(Reference {
            type_id: "tests".into(),
            factory: Some("py.test".into()),
            type_name: Some("Python tests".into()),
            xml: String::new(),
        });
        assert_eq!(types.name_of(&tests), "Python tests");
        assert!(!types.is_unknown(&tests));
        let mut other = RunConfiguration::new("idea:Weird", "c", Storage::Personal);
        other.reference = Some(Reference {
            type_id: "Weird".into(),
            factory: None,
            type_name: None,
            xml: String::new(),
        });
        assert_eq!(types.name_of(&other), "Weird");
        assert!(types.is_unknown(&other));
        assert!(other.is_unsupported());
        let mut parallel = python.clone();
        assert!(!types.allows_parallel(&parallel));
        parallel.allow_parallel = Some(true);
        assert!(types.allows_parallel(&parallel));
    }
}
