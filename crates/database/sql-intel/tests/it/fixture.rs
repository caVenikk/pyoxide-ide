//! A small shop database per dialect, as the model of a data source, and
//! helpers to look at what the analysis found.
#![allow(dead_code)]

use std::sync::Arc;

use db_core::DataSourceId;
use db_core::schema::{
    ColumnInfo, Level, ObjectInfo, ObjectKind, RelationInfo, SchemaInfo, SchemaSnapshot, Snapshot,
};
use db_model::Model;
use sql_intel::{
    DocumentAnalysis, FixEdit, InspectionProfile, Problem, RefKind, Reference, Resolution, Scope,
    SqlContext, analyze,
};
use sql_syntax::Dialect;

/// A name as the dialect stores an unquoted one: upper case for Oracle.
fn stored(dialect: Dialect, name: &str) -> String {
    if dialect == Dialect::Oracle {
        name.to_uppercase()
    } else {
        name.to_string()
    }
}

fn object(dialect: Dialect, schema: &str, name: &str, kind: ObjectKind) -> ObjectInfo {
    ObjectInfo {
        schema: stored(dialect, schema),
        name: stored(dialect, name),
        kind,
        ..ObjectInfo::default()
    }
}

fn relation(dialect: Dialect, name: &str, columns: &[&str]) -> RelationInfo {
    RelationInfo {
        name: stored(dialect, name),
        columns: columns
            .iter()
            .enumerate()
            .map(|(i, c)| ColumnInfo {
                name: stored(dialect, c),
                position: i as u32 + 1,
                data_type: "integer".into(),
                nullable: true,
                ..ColumnInfo::default()
            })
            .collect(),
        indexes: Vec::new(),
        constraints: Vec::new(),
    }
}

fn schema(
    dialect: Dialect,
    name: &str,
    current: bool,
    level: Level,
    objects: Vec<ObjectInfo>,
    relations: Vec<RelationInfo>,
) -> SchemaSnapshot {
    SchemaSnapshot {
        info: SchemaInfo {
            name: stored(dialect, name),
            is_system: false,
            is_current: current,
            owner: None,
            comment: None,
        },
        level,
        objects,
        relations,
        sources: Vec::new(),
    }
}

/// The name of the main schema: `public`, `SHOP`, `main`.
pub fn main_schema(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Postgres | Dialect::Generic => "public",
        Dialect::Oracle => "shop",
        Dialect::Sqlite => "main",
    }
}

/// The shop: the main schema with `customers (id, name)`, `orders (id,
/// customer_id, total)`, the view `order_totals (customer_id, sum)` and the
/// function `total`; `app` with `users (id, email)` and also `orders (id)`;
/// `audit` loaded to names only, with the table `log`.
pub fn shop(dialect: Dialect) -> Arc<Model> {
    let d = dialect;
    let main = main_schema(d);
    let public = schema(
        d,
        main,
        true,
        Level::Structure,
        vec![
            object(d, main, "customers", ObjectKind::Table),
            object(d, main, "orders", ObjectKind::Table),
            object(d, main, "order_totals", ObjectKind::View),
            object(d, main, "total", ObjectKind::Function),
        ],
        vec![
            relation(d, "customers", &["id", "name"]),
            relation(d, "orders", &["id", "customer_id", "total"]),
            relation(d, "order_totals", &["customer_id", "sum"]),
        ],
    );
    let app = schema(
        d,
        "app",
        false,
        Level::Structure,
        vec![
            object(d, "app", "users", ObjectKind::Table),
            object(d, "app", "invoices", ObjectKind::Table),
        ],
        vec![
            relation(d, "users", &["id", "email"]),
            relation(d, "invoices", &["id"]),
        ],
    );
    let audit = schema(
        d,
        "audit",
        false,
        Level::Names,
        vec![object(d, "audit", "log", ObjectKind::Table)],
        Vec::new(),
    );
    let snapshot = Snapshot {
        level: Level::Structure,
        token: None,
        schemas: vec![public, app, audit],
    };
    Arc::new(Model::from_snapshot(DataSourceId::new("shop"), snapshot))
}

/// A document bound to the shop.
pub fn scoped(dialect: Dialect) -> SqlContext {
    SqlContext::new(dialect, Some(Scope::new(shop(dialect))))
}

pub fn unscoped(dialect: Dialect) -> SqlContext {
    SqlContext::unscoped(dialect)
}

/// The dialects with a model (not Generic).
pub const RESOLVING: [Dialect; 3] = [Dialect::Postgres, Dialect::Oracle, Dialect::Sqlite];

/// What a resolution is, without its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum R {
    Resolved,
    Ambiguous,
    Unresolved,
    Unknown,
}

pub fn class(r: &Resolution) -> R {
    match r {
        Resolution::Resolved(_) => R::Resolved,
        Resolution::Ambiguous(_) => R::Ambiguous,
        Resolution::Unresolved => R::Unresolved,
        Resolution::Unknown => R::Unknown,
    }
}

/// The references of `text`, as (written, kind, class).
pub fn refs(text: &str, ctx: &SqlContext) -> Vec<(String, RefKind, R)> {
    analyze(text, ctx)
        .references()
        .into_iter()
        .map(|r| (r.written.clone(), r.kind, class(&r.resolution)))
        .collect()
}

/// The resolution class of the reference written `name` (the first one).
pub fn res(text: &str, ctx: &SqlContext, name: &str) -> R {
    let all = analyze(text, ctx).references();
    let r = find_ref(&all, name).unwrap_or_else(|| panic!("no reference {name} in {all:#?}"));
    class(&r.resolution)
}

pub fn find_ref<'a>(all: &'a [Reference], name: &str) -> Option<&'a Reference> {
    all.iter().find(|r| r.written == name)
}

/// The problems with the default profile.
pub fn problems(text: &str, ctx: &SqlContext) -> Vec<Problem> {
    problems_of(&analyze(text, ctx))
}

pub fn problems_of(a: &DocumentAnalysis) -> Vec<Problem> {
    a.problems(&InspectionProfile::default())
}

/// The problems as (id, text of the range, message).
pub fn summary(text: &str, ctx: &SqlContext) -> Vec<(&'static str, String, String)> {
    problems(text, ctx)
        .into_iter()
        .map(|p| (p.inspection.id(), text[p.range].to_string(), p.message))
        .collect()
}

/// Applies edits (in the text before any of them).
pub fn apply(text: &str, edits: &[FixEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|e| std::cmp::Reverse(e.range.start));
    let mut out = text.to_string();
    for e in edits {
        out.replace_range(e.range.clone(), &e.text);
    }
    out
}

/// Applies the fix titled `title` of the first problem that has one.
pub fn apply_fix(text: &str, ctx: &SqlContext, title: &str) -> String {
    let ps = problems(text, ctx);
    let fix = ps
        .iter()
        .flat_map(|p| p.fixes.iter())
        .find(|f| f.title == title)
        .unwrap_or_else(|| panic!("no fix {title:?} in {ps:#?}"));
    apply(text, &fix.edits)
}
