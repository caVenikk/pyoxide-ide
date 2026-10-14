//! What a product tells the client about one server.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use code_intel::{Features, ServerId};
use process::CommandLine;
use serde_json::Value;

use crate::customizer::ServerCustomizer;

/// One language server of a project.
#[derive(Clone)]
pub struct ServerSpec {
    /// The product's id of the server (`ruff`, `ty`), unique in a project.
    pub id: ServerId,
    /// The name shown until the server reports its own.
    pub name: String,
    /// How to start it (`ruff server`); it speaks LSP over its standard
    /// streams.
    pub command: CommandLine,
    /// Its workspace folder (the project root).
    pub root: PathBuf,
    /// Language ids of the documents it gets (`python`).
    pub languages: Vec<String>,
    /// The features it serves; a feature it does not offer is not routed
    /// to it even when listed.
    pub features: Features,
    /// `initializationOptions` of `initialize`.
    pub initialization_options: Option<Value>,
    /// The settings `workspace/configuration` answers from (by section,
    /// see [`ServerCustomizer::configuration`]).
    pub settings: Value,
    /// The product's icon for the server, passed on in its status.
    pub icon: Option<String>,
    /// The product's adjustments; none when `None`.
    pub customizer: Option<Arc<dyn ServerCustomizer>>,
    /// Send an empty `workspace/didChangeConfiguration` once the server
    /// runs: servers of the pyright family ask for their settings
    /// (`workspace/configuration`) only after one, as the reference's
    /// `PyrightEmptyConfigurationSender` knows.
    pub empty_configuration_change: bool,
}

impl ServerSpec {
    /// A server without languages or features yet.
    pub fn new(id: &str, command: CommandLine, root: impl Into<PathBuf>) -> Self {
        Self {
            id: ServerId::new(id),
            name: id.to_owned(),
            command,
            root: root.into(),
            languages: Vec::new(),
            features: Features::NONE,
            initialization_options: None,
            settings: Value::Null,
            icon: None,
            customizer: None,
            empty_configuration_change: false,
        }
    }

    /// See [`ServerSpec::empty_configuration_change`].
    pub fn with_empty_configuration_change(mut self) -> Self {
        self.empty_configuration_change = true;
        self
    }

    pub fn with_languages<S: Into<String>>(
        mut self,
        languages: impl IntoIterator<Item = S>,
    ) -> Self {
        self.languages = languages.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_features(mut self, features: Features) -> Self {
        self.features = features;
        self
    }

    pub fn with_settings(mut self, settings: Value) -> Self {
        self.settings = settings;
        self
    }

    pub fn with_customizer(mut self, customizer: Arc<dyn ServerCustomizer>) -> Self {
        self.customizer = Some(customizer);
        self
    }

    /// Documents of this language go to the server.
    pub fn serves(&self, language: &str) -> bool {
        self.languages.iter().any(|l| l == language)
    }
}

impl fmt::Debug for ServerSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerSpec")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("command", &self.command)
            .field("root", &self.root)
            .field("languages", &self.languages)
            .field("features", &self.features)
            .field("customized", &self.customizer.is_some())
            .finish_non_exhaustive()
    }
}
