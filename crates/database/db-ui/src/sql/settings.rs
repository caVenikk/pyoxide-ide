//! The SQL settings of a project (ADR 0022, "How a document gets its
//! dialect" and "Resolution scope"): a default dialect and scope and
//! mappings of files and folders to a dialect, a data source and a schema.
//! The shared set lives in `.oxide/project.toml`, the personal one in
//! `.oxide/workspace.toml`; both have the same shape:
//!
//! ```toml
//! [sql]
//! dialect = "postgres"
//! data-source = "local"
//! schema = "public"
//!
//! [[sql.mappings]]
//! path = "warehouse/oracle"
//! dialect = "oracle"
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};
use sql_syntax::Dialect;

/// The table of the settings in both files.
pub const TABLE: &str = "sql";

/// A file or folder of the project mapped to a dialect and a scope.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Mapping {
    /// Relative to the project, `/`-separated, no trailing `/`.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dialect: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

impl Mapping {
    fn is_empty(&self) -> bool {
        self.dialect.is_none() && self.data_source.is_none() && self.schema.is_none()
    }

    /// Whether the mapping covers the file at `relative`.
    fn covers(&self, relative: &str) -> bool {
        relative == self.path
            || relative
                .strip_prefix(&self.path)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    fn depth(&self) -> usize {
        self.path.split('/').count()
    }
}

/// One file's worth of SQL settings (shared or personal).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SqlSettings {
    /// The project's default dialect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dialect: Option<String>,
    /// The project's default data source and schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub mappings: Vec<Mapping>,
}

/// The dialect a settings string names.
pub fn dialect_of(id: &str) -> Option<Dialect> {
    Dialect::from_id(id)
}

impl SqlSettings {
    pub fn is_empty(&self) -> bool {
        *self == SqlSettings::default()
    }

    /// Reads the `[sql]` table; what does not fit is left out, so one bad
    /// entry never hides the rest.
    pub fn from_table(table: &toml::Table) -> Self {
        let text = |key: &str| {
            table
                .get(key)
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        };
        let mappings = table
            .get("mappings")
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.clone().try_into::<Mapping>().ok())
                    .filter_map(|mut m| {
                        m.path = normalize(&m.path)?;
                        Some(m)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            dialect: text("dialect"),
            data_source: text("data-source"),
            schema: text("schema"),
            mappings,
        }
    }

    /// The `[sql]` table, `None` when there is nothing to store.
    pub fn to_table(&self) -> Option<toml::Table> {
        if self.is_empty() {
            return None;
        }
        let value = toml::Value::try_from(self).ok()?;
        value.try_into().ok()
    }

    /// The mapping of exactly `path`, created when missing.
    fn mapping_mut(&mut self, path: &str) -> &mut Mapping {
        if let Some(i) = self.mappings.iter().position(|m| m.path == path) {
            return &mut self.mappings[i];
        }
        self.mappings.push(Mapping {
            path: path.to_owned(),
            ..Mapping::default()
        });
        self.mappings.last_mut().expect("just pushed")
    }

    fn tidy(&mut self) {
        self.mappings.retain(|m| !m.is_empty());
    }

    /// Maps `path` (`None`: the project) to a dialect, or removes the
    /// dialect with `None`.
    pub fn set_dialect(&mut self, path: Option<&str>, dialect: Option<Dialect>) {
        let id = dialect.map(|d| d.id().to_owned());
        match path.and_then(normalize) {
            Some(path) => self.mapping_mut(&path).dialect = id,
            None => self.dialect = id,
        }
        self.tidy();
    }

    /// Maps `path` (`None`: the project) to a data source and a schema, or
    /// removes them with `None`.
    pub fn set_scope(&mut self, path: Option<&str>, scope: Option<(&str, Option<&str>)>) {
        let (data_source, schema) = match scope {
            Some((d, s)) => (Some(d.to_owned()), s.map(str::to_owned)),
            None => (None, None),
        };
        match path.and_then(normalize) {
            Some(path) => {
                let m = self.mapping_mut(&path);
                m.data_source = data_source;
                m.schema = schema;
            }
            None => {
                self.data_source = data_source;
                self.schema = schema;
            }
        }
        self.tidy();
    }
}

/// A relative path as stored: no trailing `/`, no `.` parts; `None` for
/// the project folder itself and for a path that leaves it.
pub fn normalize(path: &str) -> Option<String> {
    project::store::normalize(path)
}

/// Where a dialect or a scope came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A mapping of this file or folder (relative path); personal ones are
    /// not shared with the team.
    Mapping { path: String, personal: bool },
    /// The project's default.
    Project,
    /// The data source the file is bound to gave the dialect.
    DataSource(String),
    /// Nothing says: generic SQL, no scope.
    None,
}

/// What names in a file mean, as the settings say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub dialect: Dialect,
    pub dialect_origin: Origin,
    pub data_source: Option<String>,
    pub schema: Option<String>,
    pub scope_origin: Origin,
}

impl Default for Resolved {
    fn default() -> Self {
        Self {
            dialect: Dialect::Generic,
            dialect_origin: Origin::None,
            data_source: None,
            schema: None,
            scope_origin: Origin::None,
        }
    }
}

/// The dialect and scope of the file at `relative` (`None`: outside the
/// project). The closest mapping wins, a personal one over a shared one of
/// the same path; the dialect comes from the mapping's own dialect, else
/// from the data source it names (`source_dialect`), then the project's
/// default.
pub fn resolve(
    shared: &SqlSettings,
    personal: &SqlSettings,
    relative: Option<&str>,
    source_dialect: &dyn Fn(&str) -> Option<Dialect>,
) -> Resolved {
    let mut candidates: Vec<(&Mapping, bool)> = Vec::new();
    if let Some(relative) = relative {
        candidates.extend(
            personal
                .mappings
                .iter()
                .filter(|m| m.covers(relative))
                .map(|m| (m, true)),
        );
        candidates.extend(
            shared
                .mappings
                .iter()
                .filter(|m| m.covers(relative))
                .map(|m| (m, false)),
        );
    }
    // Closest first; a stable sort keeps the personal ones before the
    // shared ones of the same path.
    candidates.sort_by_key(|(m, _)| std::cmp::Reverse(m.depth()));

    let origin = |m: &Mapping, personal: bool| Origin::Mapping {
        path: m.path.clone(),
        personal,
    };
    let mut out = Resolved::default();
    let mut dialect_done = false;
    let mut scope_done = false;
    for &(m, p) in &candidates {
        if !scope_done && let Some(ds) = &m.data_source {
            out.data_source = Some(ds.clone());
            out.schema = m.schema.clone();
            out.scope_origin = origin(m, p);
            scope_done = true;
        }
        if !dialect_done {
            if let Some(d) = m.dialect.as_deref().and_then(dialect_of) {
                out.dialect = d;
                out.dialect_origin = origin(m, p);
                dialect_done = true;
            } else if let Some(d) = m.data_source.as_deref().and_then(source_dialect) {
                out.dialect = d;
                out.dialect_origin = Origin::DataSource(m.data_source.clone().unwrap_or_default());
                dialect_done = true;
            }
        }
    }
    // The project default: the personal one over the shared one.
    if !scope_done {
        let default = [personal, shared].into_iter().find_map(|s| {
            s.data_source
                .as_ref()
                .map(|d| (d.clone(), s.schema.clone()))
        });
        if let Some((ds, schema)) = default {
            out.data_source = Some(ds);
            out.schema = schema;
            out.scope_origin = Origin::Project;
        }
    }
    if !dialect_done {
        let default = [personal, shared]
            .into_iter()
            .find_map(|s| s.dialect.as_deref().and_then(dialect_of));
        if let Some(d) = default {
            out.dialect = d;
            out.dialect_origin = Origin::Project;
        } else if let Some(d) = out.data_source.as_deref().and_then(source_dialect) {
            out.dialect = d;
            out.dialect_origin = Origin::DataSource(out.data_source.clone().unwrap_or_default());
        }
    }
    out
}

/// The path of a file relative to the project, in the stored form.
pub fn relative(root: &Path, file: &Path) -> Option<String> {
    project::store::relative_to(root, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<Dialect> {
        None
    }

    fn pg_source(name: &str) -> Option<Dialect> {
        (name == "local").then_some(Dialect::Postgres)
    }

    fn mapping(path: &str, dialect: Option<&str>, ds: Option<&str>) -> Mapping {
        Mapping {
            path: path.into(),
            dialect: dialect.map(Into::into),
            data_source: ds.map(Into::into),
            schema: None,
        }
    }

    #[test]
    fn nothing_mapped_is_generic_without_a_scope() {
        let r = resolve(
            &SqlSettings::default(),
            &SqlSettings::default(),
            Some("a.sql"),
            &none,
        );
        assert_eq!(r, Resolved::default());
    }

    #[test]
    fn the_project_default_applies_when_no_mapping_does() {
        let shared = SqlSettings {
            dialect: Some("postgres".into()),
            data_source: Some("local".into()),
            schema: Some("public".into()),
            ..SqlSettings::default()
        };
        let r = resolve(&shared, &SqlSettings::default(), Some("a.sql"), &none);
        assert_eq!(r.dialect, Dialect::Postgres);
        assert_eq!(r.dialect_origin, Origin::Project);
        assert_eq!(r.data_source.as_deref(), Some("local"));
        assert_eq!(r.schema.as_deref(), Some("public"));
        assert_eq!(r.scope_origin, Origin::Project);
    }

    #[test]
    fn the_closest_mapping_wins() {
        let shared = SqlSettings {
            mappings: vec![
                mapping("migrations", Some("postgres"), Some("local")),
                mapping("migrations/legacy.sql", Some("oracle"), None),
            ],
            ..SqlSettings::default()
        };
        let none_set = SqlSettings::default();
        let folder = resolve(&shared, &none_set, Some("migrations/001.sql"), &none);
        assert_eq!(folder.dialect, Dialect::Postgres);
        assert_eq!(folder.data_source.as_deref(), Some("local"));
        let file = resolve(&shared, &none_set, Some("migrations/legacy.sql"), &none);
        assert_eq!(file.dialect, Dialect::Oracle);
        // The data source still comes from the folder.
        assert_eq!(file.data_source.as_deref(), Some("local"));
        let other = resolve(&shared, &none_set, Some("migrationsx/a.sql"), &none);
        assert_eq!(other.dialect, Dialect::Generic);
    }

    #[test]
    fn a_personal_mapping_beats_a_shared_one_of_the_same_path() {
        let shared = SqlSettings {
            mappings: vec![mapping("db", Some("postgres"), None)],
            ..SqlSettings::default()
        };
        let personal = SqlSettings {
            mappings: vec![mapping("db", Some("sqlite"), None)],
            ..SqlSettings::default()
        };
        let r = resolve(&shared, &personal, Some("db/x.sql"), &none);
        assert_eq!(r.dialect, Dialect::Sqlite);
        assert_eq!(
            r.dialect_origin,
            Origin::Mapping {
                path: "db".into(),
                personal: true
            }
        );
    }

    #[test]
    fn a_data_source_gives_the_dialect_when_the_mapping_has_none() {
        let shared = SqlSettings {
            mappings: vec![mapping("db", None, Some("local"))],
            ..SqlSettings::default()
        };
        let r = resolve(
            &shared,
            &SqlSettings::default(),
            Some("db/x.sql"),
            &pg_source,
        );
        assert_eq!(r.dialect, Dialect::Postgres);
        assert_eq!(r.dialect_origin, Origin::DataSource("local".into()));
    }

    #[test]
    fn a_file_outside_the_project_has_the_default_only() {
        let shared = SqlSettings {
            dialect: Some("oracle".into()),
            mappings: vec![mapping("db", Some("postgres"), None)],
            ..SqlSettings::default()
        };
        let r = resolve(&shared, &SqlSettings::default(), None, &none);
        assert_eq!(r.dialect, Dialect::Oracle);
    }

    #[test]
    fn settings_round_trip_through_a_table() {
        let mut s = SqlSettings::default();
        s.set_dialect(None, Some(Dialect::Postgres));
        s.set_dialect(Some("warehouse/oracle/"), Some(Dialect::Oracle));
        s.set_scope(Some("warehouse"), Some(("local", Some("public"))));
        let table = s.to_table().unwrap();
        assert_eq!(SqlSettings::from_table(&table), s);
        assert_eq!(s.mappings.len(), 2);
        assert_eq!(s.mappings[0].path, "warehouse/oracle");
    }

    #[test]
    fn clearing_a_mapping_removes_it() {
        let mut s = SqlSettings::default();
        s.set_dialect(Some("a"), Some(Dialect::Sqlite));
        s.set_dialect(Some("a"), None);
        assert!(s.is_empty());
        assert!(s.to_table().is_none());
    }

    #[test]
    fn bad_entries_are_skipped() {
        let table: toml::Table = toml::from_str(
            "dialect = 3\n[[mappings]]\npath = \"../x\"\ndialect = \"oracle\"\n[[mappings]]\npath = \"ok\"\n",
        )
        .unwrap();
        let s = SqlSettings::from_table(&table);
        assert_eq!(s.dialect, None);
        assert_eq!(s.mappings.len(), 1);
        assert_eq!(s.mappings[0].path, "ok");
    }
}
