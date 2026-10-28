//! Change detection shared by the dialects: what a [`ChangeToken`] holds, how
//! two readings of a schema are compared, and the three entry points the
//! introspectors forward to ([`load`], [`changes`], [`load_objects`]).
//!
//! A dialect says how to take a *stamp* of every object of a schema (one
//! cheap query: PostgreSQL the row versions of the catalogs, Oracle
//! `LAST_DDL_TIME`, SQLite the text of `sqlite_schema`) and how to read just
//! some objects ([`Units`]). The token is the stamps of every schema read so
//! far, so it says per schema, and per object, what the model was built from.
//! Stamps are taken before the data is read: a change that lands in between
//! shows as a difference at the next comparison and is read again (never the
//! other way round, so a change is not lost, only possibly read twice).

use std::collections::{BTreeMap, HashMap, HashSet};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{
    ChangeToken, Delta, Dialect, Error, Introspector, Level, ObjectInfo, ObjectKind, ObjectRef,
    Result, SchemaDelta, SchemaInfo, SchemaSnapshot, Scope, Snapshot,
};

/// The version of the token's content. Another version is `Unsupported`.
const FORMAT: u32 = 1;

/// More objects than this are not asked for by name: the schema is read
/// whole instead (a statement with thousands of names is no cheaper).
const MAX_UNITS: usize = 400;

/// One object as the change query saw it: the kind, the name, the overload
/// signature of a routine (empty otherwise), the table of an index or trigger
/// that is stamped on its own, and the stamp (opaque, equal when the object
/// did not change).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stamp(
    pub ObjectKind,
    pub String,
    pub String,
    pub Option<String>,
    pub String,
);

impl Stamp {
    fn key(&self) -> (ObjectKind, &str, &str, Option<&str>) {
        (self.0, &self.1, &self.2, self.3.as_deref())
    }
}

/// The stamps of one schema and what it was read at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SchemaStamps {
    pub level: Level,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub system: bool,
    /// The schema itself: its owner and comment (PostgreSQL), the schema
    /// version (SQLite).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub row: String,
    pub objects: Vec<Stamp>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Body {
    v: u32,
    #[serde(default)]
    schemas: BTreeMap<String, SchemaStamps>,
}

impl Body {
    /// The content of `token`; `None` for another dialect, another version
    /// or text that is not a token.
    fn parse(token: &ChangeToken, dialect: Dialect) -> Option<Self> {
        if token.dialect != dialect {
            return None;
        }
        let body: Body = serde_json::from_str(&token.value).ok()?;
        (body.v == FORMAT).then_some(body)
    }

    fn token(mut self, dialect: Dialect) -> ChangeToken {
        self.v = FORMAT;
        ChangeToken {
            dialect,
            value: serde_json::to_string(&self).unwrap_or_default(),
        }
    }
}

impl ChangeToken {
    /// The token after a pass that read the schemas `replaced` and made
    /// `new`: the stamps of those schemas come from `new` (none when `new`
    /// is `None`, so a pass that left no stamps does not keep stale ones),
    /// the stamps of every other schema stay as they were in `old`. A token
    /// of another dialect or one that cannot be read is replaced by `new`.
    pub fn merged(
        old: Option<&ChangeToken>,
        new: Option<&ChangeToken>,
        replaced: &[&str],
    ) -> Option<ChangeToken> {
        let Some(new_token) = new else {
            let old = old?;
            let mut body = Body::parse(old, old.dialect)?;
            body.schemas
                .retain(|name, _| !replaced.contains(&name.as_str()));
            return Some(body.token(old.dialect));
        };
        let dialect = new_token.dialect;
        let Some(fresh) = Body::parse(new_token, dialect) else {
            return Some(new_token.clone());
        };
        let mut body = match old.and_then(|o| Body::parse(o, dialect)) {
            Some(body) => body,
            None => return Some(fresh.token(dialect)),
        };
        body.schemas
            .retain(|name, _| !replaced.contains(&name.as_str()));
        body.schemas.extend(fresh.schemas);
        Some(body.token(dialect))
    }

    /// The schemas this token holds stamps of, with the level each was read
    /// at.
    pub fn schemas(&self) -> Vec<(String, Level)> {
        Body::parse(self, self.dialect)
            .map(|b| b.schemas.into_iter().map(|(n, s)| (n, s.level)).collect())
            .unwrap_or_default()
    }
}

/// A short fixed-width digest of `parts` (FNV-1a, 64 bits): equal parts give
/// equal digests in every process and version, which a token kept on disk
/// needs.
pub(crate) fn digest(parts: &[&str]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.bytes().chain(std::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

/// The objects a partial read is about, by what they are named in. A table
/// stands for its columns, keys, indexes and triggers, a routine name for
/// every overload.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Units {
    /// Tables, views, materialized views, sequences, synonyms (and the
    /// triggers that have no table).
    pub relations: Vec<String>,
    /// Functions, procedures, packages.
    pub routines: Vec<String>,
    pub types: Vec<String>,
}

impl Units {
    pub fn add(&mut self, kind: ObjectKind, name: &str) {
        let list = match kind {
            ObjectKind::Function | ObjectKind::Procedure | ObjectKind::Package => {
                &mut self.routines
            }
            ObjectKind::Type => &mut self.types,
            _ => &mut self.relations,
        };
        if !list.iter().any(|n| n == name) {
            list.push(name.to_owned());
        }
    }

    pub fn len(&self) -> usize {
        self.relations.len() + self.routines.len() + self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every name, whatever it is the name of (Oracle keeps the names of
    /// all objects of a schema in one namespace).
    pub fn names(&self) -> Vec<String> {
        let mut all: Vec<String> = self
            .relations
            .iter()
            .chain(&self.routines)
            .chain(&self.types)
            .cloned()
            .collect();
        all.sort();
        all.dedup();
        all
    }

    pub fn selects(&self, object: &ObjectInfo) -> bool {
        let has = |list: &[String], name: &str| list.iter().any(|n| n == name);
        match object.kind {
            ObjectKind::Index | ObjectKind::Trigger => {
                object
                    .parent
                    .as_deref()
                    .is_some_and(|p| has(&self.relations, p))
                    || has(&self.relations, &object.name)
            }
            ObjectKind::Function | ObjectKind::Procedure | ObjectKind::Package => {
                has(&self.routines, &object.name)
            }
            ObjectKind::Type => has(&self.types, &object.name),
            _ => has(&self.relations, &object.name),
        }
    }
}

/// `snapshot` reduced to the objects `units` name, their children, and the
/// structure and sources of those: the same result as a read restricted to
/// them, for dialects that read a schema whole.
pub(crate) fn retain_units(mut snapshot: SchemaSnapshot, units: &Units) -> SchemaSnapshot {
    snapshot.objects.retain(|o| units.selects(o));
    let kept: HashSet<(ObjectKind, &str)> = snapshot
        .objects
        .iter()
        .map(|o| (o.kind, o.name.as_str()))
        .collect();
    let names: HashSet<&str> = snapshot.objects.iter().map(|o| o.name.as_str()).collect();
    snapshot
        .relations
        .retain(|r| names.contains(r.name.as_str()));
    snapshot
        .sources
        .retain(|s| kept.contains(&(s.kind, s.name.as_str())));
    snapshot
}

/// How a dialect takes part in change detection.
#[async_trait]
pub(crate) trait Stamper: Sync {
    fn dialect(&self) -> Dialect;

    /// Every schema with the facts a pass keeps (owner, comment).
    async fn schema_infos(&self) -> Result<Vec<SchemaInfo>>;

    /// The change query: one stamp per object of `info`, taken before any
    /// data is read.
    async fn stamps(&self, info: &SchemaInfo, level: Level) -> Result<SchemaStamps>;

    /// The stamps and the whole schema, in that order. A dialect whose
    /// change query already holds what a read needs may share it.
    async fn stamped_read(
        &self,
        info: SchemaInfo,
        level: Level,
    ) -> Result<(SchemaStamps, SchemaSnapshot)> {
        let stamps = self.stamps(&info, level).await?;
        let snapshot = self.read(info, level, None).await?;
        Ok((stamps, snapshot))
    }

    /// The schema at `level`: all of it (`units` `None`), or the objects of
    /// `units` with their children.
    async fn read(
        &self,
        info: SchemaInfo,
        level: Level,
        units: Option<&Units>,
    ) -> Result<SchemaSnapshot>;
}

/// [`Introspector::load`](super::Introspector::load) for a dialect: the
/// schemas of `scope` read whole, with the token of what they were at.
pub(crate) async fn load(stamper: &dyn Stamper, scope: &Scope, level: Level) -> Result<Snapshot> {
    let mut body = Body::default();
    let mut schemas = Vec::new();
    for info in stamper.schema_infos().await? {
        if !scope.selects(&info) {
            continue;
        }
        let (stamps, snapshot) = stamper.stamped_read(info, level).await?;
        body.schemas.insert(snapshot.info.name.clone(), stamps);
        schemas.push(snapshot);
    }
    Ok(Snapshot {
        level,
        token: Some(body.token(stamper.dialect())),
        schemas,
    })
}

/// What a comparison of two readings of a schema found.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub units: Units,
    pub dropped: Vec<ObjectRef>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.units.is_empty() && self.dropped.is_empty()
    }
}

/// The objects that are new or changed (to be read again) and the ones that
/// are gone (to be dropped) between `old` and `new`. A stamp with a table
/// stands for that table: the table is read again, children and all.
pub(crate) fn diff(schema: &str, old: &[Stamp], new: &[Stamp]) -> Plan {
    let before: HashMap<_, _> = old.iter().map(|s| (s.key(), s)).collect();
    let after: HashMap<_, _> = new.iter().map(|s| (s.key(), s)).collect();
    let mut plan = Plan::default();
    for stamp in new {
        if before.get(&stamp.key()).is_some_and(|b| b.4 == stamp.4) {
            continue;
        }
        match &stamp.3 {
            Some(table) => plan.units.add(ObjectKind::Table, table),
            None => plan.units.add(stamp.0, &stamp.1),
        }
    }
    for stamp in old {
        if after.contains_key(&stamp.key()) {
            continue;
        }
        match &stamp.3 {
            Some(table) => plan.units.add(ObjectKind::Table, table),
            None => {
                // Another overload of a routine may still be there.
                if matches!(stamp.0, ObjectKind::Function | ObjectKind::Procedure) {
                    plan.units.add(stamp.0, &stamp.1);
                }
                plan.dropped.push(ObjectRef {
                    schema: schema.to_owned(),
                    name: stamp.1.clone(),
                    kind: stamp.0,
                });
            }
        }
    }
    plan
}

fn delta_of(snapshot: SchemaSnapshot, dropped: Vec<ObjectRef>) -> SchemaDelta {
    SchemaDelta {
        info: snapshot.info,
        objects: snapshot.objects,
        relations: snapshot.relations,
        sources: snapshot.sources,
        dropped,
    }
}

/// [`Introspector::changes`](super::Introspector::changes) for a dialect.
pub(crate) async fn changes(
    stamper: &dyn Stamper,
    since: &ChangeToken,
    scope: &Scope,
) -> Result<Delta> {
    let dialect = stamper.dialect();
    let mut body = Body::parse(since, dialect).ok_or_else(|| {
        Error::unsupported("the change token is not one of this database; reload the schema")
    })?;
    let infos = stamper.schema_infos().await?;

    let present: HashSet<&str> = infos.iter().map(|i| i.name.as_str()).collect();
    let gone: Vec<String> = body
        .schemas
        .iter()
        .filter(|(name, stamps)| {
            !present.contains(name.as_str())
                && scope.selects(&SchemaInfo {
                    name: (*name).clone(),
                    is_system: stamps.system,
                    ..SchemaInfo::default()
                })
        })
        .map(|(name, _)| name.clone())
        .collect();
    for name in &gone {
        body.schemas.remove(name);
    }

    let mut schemas = Vec::new();
    let mut unseen = Vec::new();
    for info in infos.into_iter().filter(|i| scope.selects(i)) {
        let Some(old) = body.schemas.get(&info.name) else {
            unseen.push(info.name);
            continue;
        };
        let level = old.level;
        let new = stamper.stamps(&info, level).await?;
        let plan = diff(&info.name, &old.objects, &new.objects);
        if plan.is_empty() && old.row == new.row {
            continue;
        }
        let snapshot = if plan.units.is_empty() {
            empty_snapshot(info, level)
        } else if plan.units.len() > MAX_UNITS {
            // So much changed that asking for it by name does not pay.
            stamper.read(info, level, None).await?
        } else {
            stamper.read(info, level, Some(&plan.units)).await?
        };
        let name = snapshot.info.name.clone();
        schemas.push(delta_of(snapshot, plan.dropped));
        body.schemas.insert(name, new);
    }
    Ok(Delta {
        token: body.token(dialect),
        schemas,
        dropped_schemas: gone,
        unseen,
    })
}

/// The units `objects` stand for; refs that cannot be read by name are an
/// error.
fn units_of(schema: &str, objects: &[ObjectRef]) -> Result<Units> {
    let mut units = Units::default();
    for object in objects {
        if object.schema != schema {
            return Err(Error::config(format!(
                "`{}` is not in the schema `{schema}`",
                object.name
            )));
        }
        if matches!(object.kind, ObjectKind::Index | ObjectKind::Trigger) {
            return Err(Error::unsupported(
                "an index or a trigger is read again with its table; name the table",
            ));
        }
        units.add(object.kind, &object.name);
    }
    Ok(units)
}

/// The delta of a partial read: what was asked for and is not there any more
/// is dropped.
fn partial_delta(snapshot: SchemaSnapshot, asked: &[ObjectRef]) -> SchemaDelta {
    let gone = asked
        .iter()
        .filter(|r| {
            !snapshot
                .objects
                .iter()
                .any(|o| o.kind == r.kind && o.name == r.name)
        })
        .cloned()
        .collect();
    delta_of(snapshot, gone)
}

fn empty_snapshot(info: SchemaInfo, level: Level) -> SchemaSnapshot {
    SchemaSnapshot {
        info,
        level,
        objects: Vec::new(),
        relations: Vec::new(),
        sources: Vec::new(),
    }
}

/// [`Introspector::load_objects`](super::Introspector::load_objects) for a
/// dialect with a partial read.
pub(crate) async fn load_objects(
    stamper: &dyn Stamper,
    schema: &str,
    objects: &[ObjectRef],
    level: Level,
) -> Result<SchemaDelta> {
    let units = units_of(schema, objects)?;
    let info = stamper
        .schema_infos()
        .await?
        .into_iter()
        .find(|i| i.name == schema)
        .ok_or_else(|| Error::query(format!("the schema `{schema}` does not exist")))?;
    let snapshot = if units.is_empty() {
        empty_snapshot(info, level)
    } else if units.len() > MAX_UNITS {
        retain_units(stamper.read(info, level, None).await?, &units)
    } else {
        stamper.read(info, level, Some(&units)).await?
    };
    Ok(partial_delta(snapshot, objects))
}

/// The default of [`Introspector::load_objects`](super::Introspector::load_objects):
/// the whole schema is read and reduced.
pub(crate) async fn load_objects_by_load<I: Introspector + ?Sized>(
    intro: &I,
    schema: &str,
    objects: &[ObjectRef],
    level: Level,
) -> Result<SchemaDelta> {
    let units = units_of(schema, objects)?;
    let snapshot = intro.load(&Scope::list([schema]), level).await?;
    let whole = snapshot
        .schemas
        .into_iter()
        .find(|s| s.info.name == schema)
        .ok_or_else(|| Error::query(format!("the schema `{schema}` does not exist")))?;
    Ok(partial_delta(retain_units(whole, &units), objects))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(kind: ObjectKind, name: &str, stamp: &str) -> Stamp {
        Stamp(kind, name.into(), String::new(), None, stamp.into())
    }

    fn child(kind: ObjectKind, name: &str, table: &str, stamp: &str) -> Stamp {
        Stamp(
            kind,
            name.into(),
            String::new(),
            Some(table.into()),
            stamp.into(),
        )
    }

    #[test]
    fn digests_are_stable_and_tell_parts_apart() {
        assert_eq!(digest(&["a", "b"]), digest(&["a", "b"]));
        assert_ne!(digest(&["ab", ""]), digest(&["a", "b"]));
        assert_eq!(digest(&[]), "cbf29ce484222325");
        assert_eq!(digest(&["x"]).len(), 16);
    }

    #[test]
    fn nothing_changed_nothing_planned() {
        let a = vec![
            stamp(ObjectKind::Table, "t", "1"),
            stamp(ObjectKind::Function, "f", "2"),
        ];
        assert_eq!(diff("s", &a, &a.clone()), Plan::default());
    }

    #[test]
    fn a_changed_table_is_read_again_and_a_new_one_too() {
        let old = vec![
            stamp(ObjectKind::Table, "t", "1"),
            stamp(ObjectKind::Table, "u", "1"),
        ];
        let new = vec![
            stamp(ObjectKind::Table, "t", "2"),
            stamp(ObjectKind::Table, "u", "1"),
            stamp(ObjectKind::View, "v", "1"),
        ];
        let plan = diff("s", &old, &new);
        assert_eq!(plan.units.relations, ["t", "v"]);
        assert!(plan.dropped.is_empty());
    }

    #[test]
    fn a_dropped_object_is_dropped_and_a_dropped_overload_reads_the_rest() {
        let old = vec![
            stamp(ObjectKind::Table, "gone", "1"),
            Stamp(
                ObjectKind::Function,
                "f".into(),
                "(a integer)".into(),
                None,
                "1".into(),
            ),
            Stamp(
                ObjectKind::Function,
                "f".into(),
                "(a text)".into(),
                None,
                "1".into(),
            ),
        ];
        let new = vec![Stamp(
            ObjectKind::Function,
            "f".into(),
            "(a text)".into(),
            None,
            "1".into(),
        )];
        let plan = diff("s", &old, &new);
        assert_eq!(plan.units.routines, ["f"]);
        assert!(
            plan.units.relations.is_empty(),
            "a dropped table is not read"
        );
        let names: Vec<_> = plan
            .dropped
            .iter()
            .map(|d| (d.kind, d.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [(ObjectKind::Table, "gone"), (ObjectKind::Function, "f")]
        );
    }

    #[test]
    fn a_child_stands_for_its_table() {
        let old = vec![
            stamp(ObjectKind::Table, "t", "1"),
            child(ObjectKind::Index, "i", "t", "1"),
        ];
        let new = vec![stamp(ObjectKind::Table, "t", "1")];
        let plan = diff("s", &old, &new);
        assert_eq!(plan.units.relations, ["t"]);
        assert!(
            plan.dropped.is_empty(),
            "the table read again replaces its children"
        );
        let changed = vec![
            stamp(ObjectKind::Table, "t", "1"),
            child(ObjectKind::Index, "i", "t", "2"),
        ];
        assert_eq!(diff("s", &old, &changed).units.relations, ["t"]);
    }

    #[test]
    fn units_select_children_by_table_and_names_by_family() {
        let mut units = Units::default();
        units.add(ObjectKind::Table, "t");
        units.add(ObjectKind::Procedure, "p");
        units.add(ObjectKind::Type, "ty");
        units.add(ObjectKind::Table, "t");
        assert_eq!(units.len(), 3);
        assert_eq!(units.names(), ["p", "t", "ty"]);
        let object = |kind, name: &str, parent: Option<&str>| ObjectInfo {
            kind,
            name: name.into(),
            parent: parent.map(Into::into),
            ..ObjectInfo::default()
        };
        assert!(units.selects(&object(ObjectKind::Index, "i", Some("t"))));
        assert!(!units.selects(&object(ObjectKind::Index, "i", Some("u"))));
        assert!(units.selects(&object(ObjectKind::Procedure, "p", None)));
        assert!(!units.selects(&object(ObjectKind::Function, "t", None)));
        assert!(units.selects(&object(ObjectKind::Type, "ty", None)));
    }

    #[test]
    fn tokens_merge_by_schema() {
        let make = |schemas: &[(&str, Level)]| {
            let mut body = Body::default();
            for (name, level) in schemas {
                body.schemas.insert(
                    (*name).to_owned(),
                    SchemaStamps {
                        level: *level,
                        system: false,
                        row: String::new(),
                        objects: vec![stamp(ObjectKind::Table, "t", name)],
                    },
                );
            }
            body.token(Dialect::Postgres)
        };
        let old = make(&[("a", Level::Structure), ("b", Level::Structure)]);
        let new = make(&[("b", Level::Names)]);
        let merged = ChangeToken::merged(Some(&old), Some(&new), &["b"]).unwrap();
        assert_eq!(
            merged.schemas(),
            [
                ("a".to_owned(), Level::Structure),
                ("b".to_owned(), Level::Names)
            ]
        );
        // A pass without stamps forgets the stamps of what it read.
        let none = ChangeToken::merged(Some(&old), None, &["a"]).unwrap();
        assert_eq!(none.schemas(), [("b".to_owned(), Level::Structure)]);
        assert!(ChangeToken::merged(None, None, &[]).is_none());
        // Another dialect's token is replaced.
        let other = ChangeToken {
            dialect: Dialect::Oracle,
            value: "{}".into(),
        };
        let merged = ChangeToken::merged(Some(&other), Some(&new), &["b"]).unwrap();
        assert_eq!(merged.schemas().len(), 1);
    }

    #[test]
    fn a_foreign_token_is_not_read() {
        let token = ChangeToken {
            dialect: Dialect::Sqlite,
            value: "{\"v\":1}".into(),
        };
        assert!(Body::parse(&token, Dialect::Sqlite).is_some());
        assert!(Body::parse(&token, Dialect::Postgres).is_none());
        let old_version = ChangeToken {
            dialect: Dialect::Sqlite,
            value: "{\"v\":99}".into(),
        };
        assert!(Body::parse(&old_version, Dialect::Sqlite).is_none());
        let garbage = ChangeToken {
            dialect: Dialect::Sqlite,
            value: "not json".into(),
        };
        assert!(Body::parse(&garbage, Dialect::Sqlite).is_none());
    }
}
