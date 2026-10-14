//! Language servers as the UI sees them: ids, features and states.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};

use crate::document::DocumentId;

/// A server of a project, by the id its product gave it (`ruff`, `ty`).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServerId(Arc<str>);

impl ServerId {
    pub fn new(id: &str) -> Self {
        Self(Arc::from(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServerId({})", self.0)
    }
}

impl Serialize for ServerId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// A feature a server provides for its documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Feature {
    Diagnostics,
    Completion,
    Hover,
    SignatureHelp,
    Definition,
    TypeDefinition,
    References,
    CodeActions,
    Formatting,
    OrganizeImports,
    Rename,
    DocumentHighlights,
    DocumentSymbols,
    WorkspaceSymbols,
    InlayHints,
    SemanticTokens,
}

impl Feature {
    pub const ALL: [Feature; 16] = [
        Feature::Diagnostics,
        Feature::Completion,
        Feature::Hover,
        Feature::SignatureHelp,
        Feature::Definition,
        Feature::TypeDefinition,
        Feature::References,
        Feature::CodeActions,
        Feature::Formatting,
        Feature::OrganizeImports,
        Feature::Rename,
        Feature::DocumentHighlights,
        Feature::DocumentSymbols,
        Feature::WorkspaceSymbols,
        Feature::InlayHints,
        Feature::SemanticTokens,
    ];

    const fn bit(self) -> u32 {
        1 << self as u32
    }
}

/// A set of [`Feature`]s.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Features(u32);

impl Features {
    pub const NONE: Self = Self(0);

    pub const fn all() -> Self {
        Self::of(&Feature::ALL)
    }

    pub const fn of(features: &[Feature]) -> Self {
        let mut bits = 0;
        let mut i = 0;
        while i < features.len() {
            bits |= features[i].bit();
            i += 1;
        }
        Self(bits)
    }

    pub const fn with(self, feature: Feature) -> Self {
        Self(self.0 | feature.bit())
    }

    pub const fn without(self, feature: Feature) -> Self {
        Self(self.0 & !feature.bit())
    }

    pub const fn contains(self, feature: Feature) -> bool {
        self.0 & feature.bit() != 0
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = Feature> {
        Feature::ALL.into_iter().filter(move |f| self.contains(*f))
    }
}

impl FromIterator<Feature> for Features {
    fn from_iter<I: IntoIterator<Item = Feature>>(iter: I) -> Self {
        iter.into_iter().fold(Self::NONE, Self::with)
    }
}

impl fmt::Debug for Features {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl Serialize for Features {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(None)?;
        for feature in self.iter() {
            seq.serialize_element(&feature)?;
        }
        seq.end()
    }
}

/// The state of a server, as the Language Services widget shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum ServerState {
    /// Started; `initialize` has not been answered yet.
    Initializing,
    Running,
    /// Not running: never started, or stopped. It starts when a document
    /// needs it.
    Stopped,
    /// It could not start or it exited on its own; it is not restarted
    /// automatically (as in the reference).
    Terminated {
        /// The end of its standard error, or why it could not start.
        error_output: String,
    },
}

impl ServerState {
    pub fn is_running(&self) -> bool {
        matches!(self, ServerState::Initializing | ServerState::Running)
    }
}

/// A server of a project and what it does now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerStatus {
    pub id: ServerId,
    /// The server's own name once it reported it (`ruff`), else the name
    /// its product gave it.
    pub name: String,
    /// The version it reported.
    pub version: Option<String>,
    pub state: ServerState,
    /// Its workspace folders.
    pub roots: Vec<PathBuf>,
    /// The product's icon for it (a key the product's UI resolves).
    pub icon: Option<String>,
    /// The features it provides for its documents.
    pub features: Features,
    /// The open documents it serves.
    pub documents: Vec<DocumentId>,
}

impl ServerStatus {
    /// A stopped server without documents.
    pub fn new(id: ServerId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            version: None,
            state: ServerState::Stopped,
            roots: Vec::new(),
            icon: None,
            features: Features::NONE,
            documents: Vec::new(),
        }
    }

    /// Name and version (`ruff 0.15.20`), as a row of the Language
    /// Services popup starts.
    pub fn label(&self) -> String {
        match &self.version {
            Some(version) => format!("{} {version}", self.name),
            None => self.name.clone(),
        }
    }

    pub fn serves(&self, doc: &DocumentId) -> bool {
        self.documents.contains(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_sets() {
        let set = Features::of(&[Feature::Diagnostics, Feature::Formatting]);
        assert!(set.contains(Feature::Formatting));
        assert!(!set.contains(Feature::Completion));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            [Feature::Diagnostics, Feature::Formatting]
        );
        assert_eq!(
            set.without(Feature::Formatting)
                .with(Feature::Hover)
                .iter()
                .count(),
            2
        );
        assert_eq!(Features::all().iter().count(), Feature::ALL.len());
        assert!(Features::NONE.is_empty());
        assert_eq!(
            Features::all().intersection(set),
            set.iter().collect::<Features>()
        );
        assert_eq!(
            serde_json::to_value(set).unwrap(),
            serde_json::json!(["diagnostics", "formatting"])
        );
    }

    #[test]
    fn statuses() {
        let mut status = ServerStatus::new(ServerId::new("ruff"), "ruff");
        assert_eq!(status.label(), "ruff");
        status.version = Some("0.15.20".into());
        assert_eq!(status.label(), "ruff 0.15.20");
        assert!(!status.state.is_running());
        status.state = ServerState::Terminated {
            error_output: "panic".into(),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["id"], "ruff");
        assert_eq!(json["state"]["state"], "terminated");
        assert_eq!(json["state"]["error_output"], "panic");
        assert_eq!(ServerId::new("ty").to_string(), "ty");
    }
}
