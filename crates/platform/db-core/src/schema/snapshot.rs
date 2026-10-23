//! Bulk reads: what to load ([`Scope`], [`Level`]), what comes back
//! ([`Snapshot`]) and the contract for finding out what changed since
//! ([`ChangeToken`], [`Delta`]).

use serde::{Deserialize, Serialize};

use super::{
    ColumnInfo, ConstraintInfo, Dialect, IndexInfo, Introspector, ObjectInfo, ObjectKind, Result,
    SchemaInfo,
};

/// Which schemas a pass reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaFilter {
    /// The schema the session is in (PostgreSQL `current_schema()`, Oracle
    /// the current schema, SQLite `main`).
    Current,
    /// Every schema; the system ones only with [`Scope::system`].
    All,
    /// These schemas, spelled as the database stores them (Oracle upper
    /// case). A listed system schema is read even without
    /// [`Scope::system`]. Unknown names are skipped.
    List(Vec<String>),
    /// Schemas whose names match one of the patterns: `*` for any run of
    /// characters, `?` for one character, ignoring case. System schemas
    /// only with [`Scope::system`].
    Patterns(Vec<String>),
}

/// What an introspection pass covers: the schemas and whether the schemas
/// the database owns itself (`pg_catalog`, `information_schema`, `SYS`, ...)
/// are among them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub filter: SchemaFilter,
    /// Include system schemas that `filter` would otherwise take
    /// (`All`, `Patterns`).
    #[serde(default)]
    pub system: bool,
}

impl Scope {
    pub fn current() -> Self {
        Self::new(SchemaFilter::Current)
    }

    pub fn all() -> Self {
        Self::new(SchemaFilter::All)
    }

    pub fn list<I, S>(schemas: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(SchemaFilter::List(
            schemas.into_iter().map(Into::into).collect(),
        ))
    }

    pub fn patterns<I, S>(patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(SchemaFilter::Patterns(
            patterns.into_iter().map(Into::into).collect(),
        ))
    }

    fn new(filter: SchemaFilter) -> Self {
        Self {
            filter,
            system: false,
        }
    }

    /// The same scope with system schemas included (or not).
    pub fn with_system(mut self, system: bool) -> Self {
        self.system = system;
        self
    }

    /// Whether the scope reads `schema`.
    pub fn selects(&self, schema: &SchemaInfo) -> bool {
        let allowed = self.system || !schema.is_system;
        match &self.filter {
            SchemaFilter::Current => schema.is_current,
            SchemaFilter::All => allowed,
            SchemaFilter::List(names) => names.contains(&schema.name),
            SchemaFilter::Patterns(patterns) => {
                allowed && patterns.iter().any(|p| glob_matches(p, &schema.name))
            }
        }
    }
}

/// `*` and `?` wildcards, ignoring case.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) && p[pi] != '*' {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// How much a pass reads per schema. The levels are ordered: each one
/// includes the previous.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// The objects with their names, parents, owners and comments (and,
    /// for PostgreSQL, the partition tree).
    Names,
    /// Plus columns, indexes and constraints of tables and views, and the
    /// kind-specific details of the objects (routine arguments, trigger
    /// events, sequence parameters, type definitions).
    #[default]
    Structure,
    /// Plus the source text of views, routines, packages, types and
    /// triggers (for SQLite also of tables and indexes: the only text the
    /// database keeps; for PostgreSQL also of materialized views, not for
    /// Oracle ones). [`Introspector::source`] stays the lazy way to
    /// read one.
    Full,
}

/// Columns, indexes and constraints of one table, view or materialized view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationInfo {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub constraints: Vec<ConstraintInfo>,
}

/// The source text of one object (one overload of a routine).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInfo {
    pub name: String,
    pub kind: ObjectKind,
    /// The identity arguments of an overloaded routine; empty otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature: String,
    pub text: String,
}

/// One schema as read by a pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaSnapshot {
    pub info: SchemaInfo,
    pub level: Level,
    /// Every object of the schema, indexes and triggers included (their
    /// `parent` is the table).
    pub objects: Vec<ObjectInfo>,
    /// Empty at [`Level::Names`].
    pub relations: Vec<RelationInfo>,
    /// Empty below [`Level::Full`].
    pub sources: Vec<SourceInfo>,
}

impl SchemaSnapshot {
    /// The structure of a table or view.
    pub fn relation(&self, name: &str) -> Option<&RelationInfo> {
        self.relations.iter().find(|r| r.name == name)
    }
}

/// The result of [`Introspector::load`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub level: Level,
    /// Where the database was when the pass ran, for
    /// [`Introspector::changes`]. `None` when the introspector does not
    /// produce tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<ChangeToken>,
    pub schemas: Vec<SchemaSnapshot>,
}

impl Snapshot {
    pub fn schema(&self, name: &str) -> Option<&SchemaSnapshot> {
        self.schemas.iter().find(|s| s.info.name == name)
    }
}

/// An opaque position in the history of a database's metadata. Only the
/// introspector that made it understands it: PostgreSQL the catalog row
/// versions, Oracle the `LAST_DDL_TIME` of the objects, SQLite the schema
/// version. A token of another dialect, or one the database can no longer
/// compare (a restart, a wrap-around), makes
/// [`Introspector::changes`] fail with `Unsupported`, and the caller reloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeToken {
    pub dialect: Dialect,
    pub value: String,
}

/// An object named without its details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub schema: String,
    pub name: String,
    pub kind: ObjectKind,
}

/// What changed in one schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaDelta {
    pub info: SchemaInfo,
    /// Objects that are new or changed, whole.
    pub objects: Vec<ObjectInfo>,
    pub relations: Vec<RelationInfo>,
    pub sources: Vec<SourceInfo>,
    /// Objects that no longer exist.
    pub dropped: Vec<ObjectRef>,
}

/// The answer of [`Introspector::changes`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delta {
    /// Pass this to the next `changes` call.
    pub token: ChangeToken,
    pub schemas: Vec<SchemaDelta>,
    pub dropped_schemas: Vec<String>,
}

/// Kinds whose source text a pass at [`Level::Full`] reads.
pub(super) fn has_source(dialect: Dialect, kind: ObjectKind) -> bool {
    match kind {
        // Oracle keeps no text of a materialized view in the dictionary
        // views a bulk read can use; `DBMS_METADATA` is one call per view.
        ObjectKind::MaterializedView => dialect != Dialect::Oracle,
        ObjectKind::View
        | ObjectKind::Procedure
        | ObjectKind::Function
        | ObjectKind::Package
        | ObjectKind::Type
        | ObjectKind::Trigger => true,
        ObjectKind::Table | ObjectKind::Index => dialect == Dialect::Sqlite,
        ObjectKind::Sequence | ObjectKind::Synonym => false,
    }
}

/// Kinds that have columns.
pub(super) fn has_columns(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView
    )
}

/// The same result as [`Introspector::load`], read the slow way: one call
/// per schema, per table and per kind. It is the reference the bulk loads
/// are checked against, and a way to read a database that has no bulk path.
/// Optional fields stay empty, and the source of an overloaded routine is
/// one text.
pub async fn load_per_object(
    intro: &dyn Introspector,
    dialect: Dialect,
    scope: &Scope,
    level: Level,
) -> Result<Snapshot> {
    let mut schemas = Vec::new();
    for info in intro.schemas().await? {
        if !scope.selects(&info) {
            continue;
        }
        let objects = intro.objects(&info.name, &[]).await?;
        let mut relations = Vec::new();
        if level >= Level::Structure {
            for object in objects.iter().filter(|o| has_columns(o.kind)) {
                relations.push(RelationInfo {
                    name: object.name.clone(),
                    columns: intro.columns(&info.name, &object.name).await?,
                    indexes: intro.indexes(&info.name, &object.name).await?,
                    constraints: intro.constraints(&info.name, &object.name).await?,
                });
            }
        }
        let mut sources: Vec<SourceInfo> = Vec::new();
        if level >= Level::Full {
            for object in objects.iter().filter(|o| has_source(dialect, o.kind)) {
                if sources
                    .iter()
                    .any(|s| s.name == object.name && s.kind == object.kind)
                {
                    continue;
                }
                if let Some(text) = intro.source(&info.name, &object.name, object.kind).await? {
                    sources.push(SourceInfo {
                        name: object.name.clone(),
                        kind: object.kind,
                        signature: String::new(),
                        text,
                    });
                }
            }
        }
        schemas.push(SchemaSnapshot {
            info,
            level,
            objects,
            relations,
            sources,
        });
    }
    Ok(Snapshot {
        level,
        token: None,
        schemas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(name: &str, system: bool, current: bool) -> SchemaInfo {
        SchemaInfo {
            name: name.into(),
            is_system: system,
            is_current: current,
            ..SchemaInfo::default()
        }
    }

    #[test]
    fn globs() {
        assert!(glob_matches("app_*", "APP_orders"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("a?c", "abc"));
        assert!(!glob_matches("a?c", "ac"));
        assert!(glob_matches("*a*b", "xxaxxb"));
        assert!(!glob_matches("*a*b", "xxaxxbx"));
        assert!(glob_matches("pg_*", "PG_CATALOG"));
        assert!(!glob_matches("public", "public2"));
        assert!(glob_matches("**x", "x"));
    }

    #[test]
    fn scopes_select_schemas() {
        let public = schema("public", false, true);
        let app = schema("app", false, false);
        let catalog = schema("pg_catalog", true, false);

        let current = Scope::current();
        assert!(current.selects(&public) && !current.selects(&app));

        let all = Scope::all();
        assert!(all.selects(&public) && all.selects(&app) && !all.selects(&catalog));
        assert!(Scope::all().with_system(true).selects(&catalog));

        let list = Scope::list(["app", "pg_catalog"]);
        assert!(list.selects(&app) && list.selects(&catalog) && !list.selects(&public));
        assert!(!Scope::list(["APP"]).selects(&app), "names are exact");

        let patterns = Scope::patterns(["p*"]);
        assert!(patterns.selects(&public) && !patterns.selects(&app));
        assert!(!patterns.selects(&catalog));
        assert!(patterns.clone().with_system(true).selects(&catalog));
    }

    #[test]
    fn levels_are_ordered() {
        assert!(Level::Names < Level::Structure && Level::Structure < Level::Full);
        assert_eq!(Level::default(), Level::Structure);
    }

    #[test]
    fn scope_json_shape() {
        let json = serde_json::to_string(&Scope::list(["a"])).unwrap();
        assert_eq!(json, r#"{"filter":{"list":["a"]},"system":false}"#);
        let back: Scope = serde_json::from_str(r#"{"filter":"all"}"#).unwrap();
        assert_eq!(back, Scope::all());
    }
}
