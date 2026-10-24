use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Connection, Result};

/// Stable identity of a configured data source (for profiles, the profile
/// name). The id is a plain string so the way data sources are stored can
/// change without touching the code that holds ids.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DataSourceId(String);

impl DataSourceId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DataSourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for DataSourceId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl From<String> for DataSourceId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

/// What a connection is for. The reason to ask for a connection by purpose is
/// isolation: a console keeps its own session and transaction, table grids and
/// exports read on data connections that never join a user's transaction, and
/// a metadata refresh uses a short connection of its own so it neither sees
/// nor ends a transaction left open in a console (ADR 0021).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// A SQL console: its own session, transaction state and current schema.
    Console,
    /// Table grids, `COUNT(*)` and export.
    Data,
    /// Introspection.
    Meta,
}

impl Purpose {
    pub const ALL: [Purpose; 3] = [Purpose::Console, Purpose::Data, Purpose::Meta];

    /// Stable lower-case name, also used in the session name a server shows
    /// (`pyoxide:<data source>:meta`).
    pub fn id(self) -> &'static str {
        match self {
            Purpose::Console => "console",
            Purpose::Data => "data",
            Purpose::Meta => "meta",
        }
    }
}

impl std::fmt::Display for Purpose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// Opens connections to configured data sources. The engine layer implements
/// it (profiles, secrets and driver dispatch live in `db-service`); the
/// session layer and the tests that need scripted connections use it
/// instead of the drivers.
///
/// Connections are shared handles: [`Connection`] methods take `&self`, so
/// `cancel` can be called from another task while a statement runs
/// (ADR 0002). Every call opens a new session.
#[async_trait]
pub trait ConnectionSource: Send + Sync {
    async fn open(
        &self,
        data_source: &DataSourceId,
        purpose: Purpose,
    ) -> Result<Arc<dyn Connection>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_purposes() {
        let id = DataSourceId::from("reporting");
        assert_eq!(id.as_str(), "reporting");
        assert_eq!(id.to_string(), "reporting");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"reporting\"");
        assert_eq!(
            serde_json::from_str::<DataSourceId>("\"x\"").unwrap(),
            DataSourceId::new("x")
        );
        let names: Vec<_> = Purpose::ALL.iter().map(|p| p.id()).collect();
        assert_eq!(names, ["console", "data", "meta"]);
        assert_eq!(serde_json::to_string(&Purpose::Meta).unwrap(), "\"meta\"");
    }
}
